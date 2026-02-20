use std::fs::{self, OpenOptions};
use std::io::{self, Write};
use std::path::Path;

use crate::account_journal::{self, AccountEntry};
use crate::operations;

/// Reconcile a single account journal entry by assigning a counterpart account.
///
/// For single-posting entries, creates a GL transaction with the real counterpart.
/// For multi-posting entries, reconciles a specific posting by index.
///
/// Returns the GL transaction ID.
pub fn reconcile_entry(
    ledger_dir: &Path,
    account_name: &str,
    entry_id: &str,
    counterpart_account: &str,
    posting_index: Option<usize>,
) -> Result<String, Box<dyn std::error::Error + Send + Sync>> {
    // Read account journal
    let mut entries = account_journal::read_journal(ledger_dir, account_name)?;
    let original_entries = entries.clone();
    let entry_idx = entries
        .iter()
        .position(|e| e.id == entry_id)
        .ok_or_else(|| format!("entry not found: {entry_id}"))?;

    let entry = &entries[entry_idx];

    if let Some(posting_idx) = posting_index {
        if posting_idx >= entry.postings.len() {
            return Err(format!(
                "posting index {posting_idx} is out of bounds for entry {entry_id} ({} postings)",
                entry.postings.len()
            )
            .into());
        }
    } else if entry.postings.is_empty() {
        return Err(format!("entry {entry_id} has no postings to reconcile").into());
    }

    // Check if already reconciled
    if let Some(posting_idx) = posting_index {
        if entry
            .reconciled_postings
            .iter()
            .any(|(idx, _)| *idx == posting_idx)
        {
            return Err(
                format!("posting {posting_idx} of entry {entry_id} is already reconciled").into(),
            );
        }
    } else if entry.reconciled.is_some() {
        return Err(format!("entry {entry_id} is already reconciled").into());
    }

    // Generate GL transaction
    let gl_txn_id = uuid::Uuid::new_v4().to_string();
    let gl_text = format_gl_transaction(
        entry,
        account_name,
        counterpart_account,
        &gl_txn_id,
        posting_index,
    );

    // Update account journal entry with reconciled tag
    let gl_ref = format!("general.journal:{gl_txn_id}");
    if let Some(posting_idx) = posting_index {
        entries[entry_idx]
            .reconciled_postings
            .push((posting_idx, gl_ref));
    } else {
        entries[entry_idx].reconciled = Some(gl_ref);
    }

    // Write updated account journal first. If this fails, nothing else was mutated.
    account_journal::write_journal(ledger_dir, account_name, &entries)?;

    // Append to general.journal; rollback account journal on failure.
    let journal_path = ledger_dir.join("general.journal");
    if let Err(err) = append_to_journal(&journal_path, &gl_text) {
        let _ = account_journal::write_journal(ledger_dir, account_name, &original_entries);
        return Err(err.into());
    }

    // Log GL operation
    let op = operations::GlOperation::Reconcile {
        account: account_name.to_string(),
        entry_id: entry_id.to_string(),
        counterpart_account: counterpart_account.to_string(),
        posting_index,
        timestamp: operations::now_timestamp(),
    };
    if let Err(err) = operations::append_gl_operation(ledger_dir, &op) {
        let _ = remove_gl_transaction(ledger_dir, &gl_txn_id);
        let _ = account_journal::write_journal(ledger_dir, account_name, &original_entries);
        return Err(err.into());
    }

    Ok(gl_txn_id)
}

/// Undo a reconciliation by removing the GL entry and clearing reconciled tags.
pub fn unreconcile_entry(
    ledger_dir: &Path,
    account_name: &str,
    entry_id: &str,
    posting_index: Option<usize>,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    // Read account journal
    let mut entries = account_journal::read_journal(ledger_dir, account_name)?;
    let original_entries = entries.clone();
    let entry_idx = entries
        .iter()
        .position(|e| e.id == entry_id)
        .ok_or_else(|| format!("entry not found: {entry_id}"))?;

    // Get the GL reference to remove
    let gl_ref = if let Some(posting_idx) = posting_index {
        let pos = original_entries[entry_idx]
            .reconciled_postings
            .iter()
            .position(|(idx, _)| *idx == posting_idx)
            .ok_or_else(|| {
                format!("posting {posting_idx} of entry {entry_id} is not reconciled")
            })?;
        let (_, ref_str) = original_entries[entry_idx].reconciled_postings[pos].clone();
        ref_str
    } else {
        original_entries[entry_idx]
            .reconciled
            .clone()
            .ok_or_else(|| format!("entry {entry_id} is not reconciled"))?
    };

    // Remove the GL transaction from general.journal
    let gl_txn_id = gl_ref.strip_prefix("general.journal:").unwrap_or(&gl_ref);
    let removed_gl_txn = remove_gl_transaction(ledger_dir, gl_txn_id)?;

    // Update account journal entry state in memory
    if let Some(posting_idx) = posting_index {
        if let Some(pos) = entries[entry_idx]
            .reconciled_postings
            .iter()
            .position(|(idx, _)| *idx == posting_idx)
        {
            entries[entry_idx].reconciled_postings.remove(pos);
        }
    } else {
        entries[entry_idx].reconciled = None;
    }

    // Write updated account journal
    if let Err(err) = account_journal::write_journal(ledger_dir, account_name, &entries) {
        if let Some(removed) = removed_gl_txn {
            let journal_path = ledger_dir.join("general.journal");
            let _ = append_to_journal(&journal_path, &removed);
        }
        return Err(err.into());
    }

    // Log undo operation
    let op = operations::GlOperation::UndoReconcile {
        account: account_name.to_string(),
        entry_id: entry_id.to_string(),
        posting_index,
        timestamp: operations::now_timestamp(),
    };
    if let Err(err) = operations::append_gl_operation(ledger_dir, &op) {
        let _ = account_journal::write_journal(ledger_dir, account_name, &original_entries);
        if let Some(removed) = removed_gl_txn {
            let journal_path = ledger_dir.join("general.journal");
            let _ = append_to_journal(&journal_path, &removed);
        }
        return Err(err.into());
    }

    Ok(())
}

/// Reconcile two entries across accounts as an inter-account transfer.
pub fn reconcile_transfer(
    ledger_dir: &Path,
    account1: &str,
    entry_id1: &str,
    account2: &str,
    entry_id2: &str,
) -> Result<String, Box<dyn std::error::Error + Send + Sync>> {
    // Read both account journals
    let mut entries1 = account_journal::read_journal(ledger_dir, account1)?;
    let mut entries2 = account_journal::read_journal(ledger_dir, account2)?;
    let original_entries1 = entries1.clone();
    let original_entries2 = entries2.clone();

    let idx1 = entries1
        .iter()
        .position(|e| e.id == entry_id1)
        .ok_or_else(|| format!("entry not found in {account1}: {entry_id1}"))?;
    let idx2 = entries2
        .iter()
        .position(|e| e.id == entry_id2)
        .ok_or_else(|| format!("entry not found in {account2}: {entry_id2}"))?;

    // Check neither is already reconciled
    if entries1[idx1].reconciled.is_some() {
        return Err(format!("entry {entry_id1} in {account1} is already reconciled").into());
    }
    if entries2[idx2].reconciled.is_some() {
        return Err(format!("entry {entry_id2} in {account2} is already reconciled").into());
    }

    // Generate GL transaction for transfer
    let gl_txn_id = uuid::Uuid::new_v4().to_string();
    let gl_text = format_transfer_gl_transaction(
        &entries1[idx1],
        account1,
        &entries2[idx2],
        account2,
        &gl_txn_id,
    );

    // Update both account journal entries
    let gl_ref = format!("general.journal:{gl_txn_id}");
    entries1[idx1].reconciled = Some(gl_ref.clone());
    entries2[idx2].reconciled = Some(gl_ref);

    if let Err(err) = account_journal::write_journal(ledger_dir, account1, &entries1) {
        return Err(err.into());
    }
    if let Err(err) = account_journal::write_journal(ledger_dir, account2, &entries2) {
        let _ = account_journal::write_journal(ledger_dir, account1, &original_entries1);
        return Err(err.into());
    }

    // Append to general.journal
    let journal_path = ledger_dir.join("general.journal");
    if let Err(err) = append_to_journal(&journal_path, &gl_text) {
        let _ = account_journal::write_journal(ledger_dir, account1, &original_entries1);
        let _ = account_journal::write_journal(ledger_dir, account2, &original_entries2);
        return Err(err.into());
    }

    // Log transfer match
    let op = operations::GlOperation::TransferMatch {
        entries: vec![
            operations::TransferMatchEntry {
                account: account1.to_string(),
                entry_id: entry_id1.to_string(),
            },
            operations::TransferMatchEntry {
                account: account2.to_string(),
                entry_id: entry_id2.to_string(),
            },
        ],
        timestamp: operations::now_timestamp(),
    };
    if let Err(err) = operations::append_gl_operation(ledger_dir, &op) {
        let _ = remove_gl_transaction(ledger_dir, &gl_txn_id);
        let _ = account_journal::write_journal(ledger_dir, account1, &original_entries1);
        let _ = account_journal::write_journal(ledger_dir, account2, &original_entries2);
        return Err(err.into());
    }

    Ok(gl_txn_id)
}

/// Get unreconciled entries for an account.
pub fn get_unreconciled(
    ledger_dir: &Path,
    account_name: &str,
) -> Result<Vec<AccountEntry>, Box<dyn std::error::Error + Send + Sync>> {
    let entries = account_journal::read_journal(ledger_dir, account_name)?;
    Ok(entries
        .into_iter()
        .filter(|e| e.reconciled.is_none() && e.reconciled_postings.is_empty())
        .collect())
}

/// Format a GL transaction for reconciliation.
fn format_gl_transaction(
    entry: &AccountEntry,
    account_name: &str,
    counterpart_account: &str,
    gl_txn_id: &str,
    posting_index: Option<usize>,
) -> String {
    let source_tag = if let Some(posting_idx) = posting_index {
        format!(
            "; source: accounts/{}:{}:posting:{}",
            account_name, entry.id, posting_idx
        )
    } else {
        format!("; source: accounts/{}:{}", account_name, entry.id)
    };

    // Get the amount from the entry's postings
    let (real_account, amount_str) = if let Some(posting_idx) = posting_index {
        let posting = &entry.postings[posting_idx];
        let amount = posting
            .amount
            .as_ref()
            .map(|a| format!("{} {}", a.quantity, a.commodity))
            .unwrap_or_default();
        (posting.account.clone(), amount)
    } else {
        let first_posting = &entry.postings[0];
        let amount = first_posting
            .amount
            .as_ref()
            .map(|a| format!("{} {}", a.quantity, a.commodity))
            .unwrap_or_default();
        (first_posting.account.clone(), amount)
    };

    format!(
        "{}  {}  ; id: {}\n    ; generated-by: refreshmint-reconcile\n    {source_tag}\n    {real_account}  {amount_str}\n    {counterpart_account}\n",
        entry.date, entry.description, gl_txn_id,
    )
}

/// Format a GL transaction for a transfer between two accounts.
fn format_transfer_gl_transaction(
    entry1: &AccountEntry,
    account1: &str,
    _entry2: &AccountEntry,
    account2: &str,
    gl_txn_id: &str,
) -> String {
    let amount1 = entry1
        .postings
        .first()
        .and_then(|p| p.amount.as_ref())
        .map(|a| format!("{} {}", a.quantity, a.commodity))
        .unwrap_or_default();

    let real_account1 = entry1
        .postings
        .first()
        .map(|p| p.account.clone())
        .unwrap_or_default();

    let real_account2 = _entry2
        .postings
        .first()
        .map(|p| p.account.clone())
        .unwrap_or_default();

    format!(
        "{}  {}  ; id: {}\n    ; generated-by: refreshmint-reconcile\n    ; source: accounts/{}:{}\n    ; source: accounts/{}:{}\n    {real_account1}  {amount1}\n    {real_account2}\n",
        entry1.date,
        entry1.description,
        gl_txn_id,
        account1,
        entry1.id,
        account2,
        _entry2.id,
    )
}

fn append_to_journal(journal_path: &Path, text: &str) -> io::Result<()> {
    let mut file = OpenOptions::new()
        .create(true)
        .append(true)
        .open(journal_path)?;
    if file.metadata()?.len() > 0 {
        file.write_all(b"\n")?;
    }
    file.write_all(text.as_bytes())?;
    Ok(())
}

/// Remove a GL transaction from general.journal by its ID.
///
/// Finds the transaction with `; id: <gl_txn_id>` and removes it.
fn remove_gl_transaction(
    ledger_dir: &Path,
    gl_txn_id: &str,
) -> Result<Option<String>, Box<dyn std::error::Error + Send + Sync>> {
    let journal_path = ledger_dir.join("general.journal");
    if !journal_path.exists() {
        return Ok(None);
    }

    let content = fs::read_to_string(&journal_path)?;
    let marker = format!("id: {gl_txn_id}");
    let mut kept_blocks = Vec::new();
    let mut removed_block = None;

    for block in split_journal_blocks(&content) {
        if removed_block.is_none() && block.contains(&marker) {
            removed_block = Some(block);
        } else {
            kept_blocks.push(block);
        }
    }

    let mut final_content = kept_blocks.join("\n\n");
    if !final_content.is_empty() {
        final_content.push('\n');
    }
    fs::write(&journal_path, final_content)?;
    Ok(removed_block)
}

fn split_journal_blocks(content: &str) -> Vec<String> {
    let mut blocks = Vec::new();
    let mut current = String::new();

    for line in content.lines() {
        let starts_new_block = !line.trim().is_empty()
            && !line.starts_with(' ')
            && !line.starts_with('\t')
            && !current.trim().is_empty();
        if starts_new_block {
            blocks.push(current.trim_end().to_string());
            current.clear();
        }
        if !current.is_empty() {
            current.push('\n');
        }
        current.push_str(line);
    }

    if !current.trim().is_empty() {
        blocks.push(current.trim_end().to_string());
    }

    blocks
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;
    use crate::account_journal::{EntryPosting, EntryStatus, SimpleAmount};
    use std::path::PathBuf;
    use std::time::{SystemTime, UNIX_EPOCH};

    fn temp_dir(prefix: &str) -> PathBuf {
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let dir = std::env::temp_dir().join(format!(
            "refreshmint-rec-{prefix}-{}-{now}",
            std::process::id()
        ));
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn make_entry(id: &str, date: &str, desc: &str, amount: &str) -> AccountEntry {
        AccountEntry {
            id: id.to_string(),
            date: date.to_string(),
            status: EntryStatus::Cleared,
            description: desc.to_string(),
            comment: String::new(),
            evidence: vec!["doc.csv:1:1".to_string()],
            postings: vec![
                EntryPosting {
                    account: "Assets:Checking".to_string(),
                    amount: Some(SimpleAmount {
                        commodity: "USD".to_string(),
                        quantity: amount.to_string(),
                    }),
                },
                EntryPosting {
                    account: "Equity:Unreconciled:Checking".to_string(),
                    amount: None,
                },
            ],
            tags: vec![],
            extracted_by: None,
            reconciled: None,
            reconciled_postings: Vec::new(),
        }
    }

    #[test]
    fn reconcile_creates_gl_entry_and_tags_account() {
        let root = temp_dir("reconcile");
        // Create general.journal
        fs::write(root.join("general.journal"), "").unwrap();

        let entries = vec![make_entry("txn-1", "2024-01-15", "Shell Oil", "-21.32")];
        account_journal::write_journal(&root, "chase", &entries).unwrap();

        let gl_id = reconcile_entry(&root, "chase", "txn-1", "Expenses:Gas", None).unwrap();

        // Check GL entry was created
        let gl_content = fs::read_to_string(root.join("general.journal")).unwrap();
        assert!(gl_content.contains("Shell Oil"));
        assert!(gl_content.contains("Expenses:Gas"));
        assert!(gl_content.contains(&format!("id: {gl_id}")));
        assert!(gl_content.contains("generated-by: refreshmint-reconcile"));
        assert!(gl_content.contains("source: accounts/chase:txn-1"));

        // Check account journal was updated
        let updated = account_journal::read_journal(&root, "chase").unwrap();
        assert_eq!(
            updated[0].reconciled.as_ref().unwrap(),
            &format!("general.journal:{gl_id}")
        );

        // Check GL operation was logged
        let ops = operations::read_gl_operations(&root).unwrap();
        assert_eq!(ops.len(), 1);

        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn unreconcile_removes_gl_entry() {
        let root = temp_dir("unreconcile");
        fs::write(root.join("general.journal"), "").unwrap();

        let entries = vec![make_entry("txn-1", "2024-01-15", "Shell Oil", "-21.32")];
        account_journal::write_journal(&root, "chase", &entries).unwrap();

        let gl_id = reconcile_entry(&root, "chase", "txn-1", "Expenses:Gas", None).unwrap();

        // Verify GL entry exists
        let gl_before = fs::read_to_string(root.join("general.journal")).unwrap();
        assert!(gl_before.contains(&gl_id));

        // Unreconcile
        unreconcile_entry(&root, "chase", "txn-1", None).unwrap();

        // Check GL entry was removed
        let gl_after = fs::read_to_string(root.join("general.journal")).unwrap();
        assert!(!gl_after.contains(&gl_id));

        // Check account journal was updated
        let updated = account_journal::read_journal(&root, "chase").unwrap();
        assert!(updated[0].reconciled.is_none());

        // Check undo operation was logged
        let ops = operations::read_gl_operations(&root).unwrap();
        assert_eq!(ops.len(), 2); // reconcile + undo-reconcile

        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn get_unreconciled_filters_correctly() {
        let root = temp_dir("unreconciled-filter");

        let mut entries = vec![
            make_entry("txn-1", "2024-01-15", "Shell Oil", "-21.32"),
            make_entry("txn-2", "2024-01-16", "Walmart", "-50.00"),
        ];
        entries[0].reconciled = Some("general.journal:gl-1".to_string());

        account_journal::write_journal(&root, "test-acct", &entries).unwrap();

        let unreconciled = get_unreconciled(&root, "test-acct").unwrap();
        assert_eq!(unreconciled.len(), 1);
        assert_eq!(unreconciled[0].id, "txn-2");

        let _ = fs::remove_dir_all(&root);
    }
}
