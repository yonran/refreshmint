use std::fs::{self, OpenOptions};
use std::io::{self, Write};
use std::path::Path;

use crate::account_journal::{self, AccountEntry};
use crate::operations;

/// Post a single account journal entry to the GL by assigning a counterpart account.
///
/// For single-posting entries, creates a GL transaction with the real counterpart.
/// For multi-posting entries, reconciles a specific posting by index.
///
/// Returns the GL transaction ID.
pub fn post_entry(
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
        return Err(format!("entry {entry_id} has no postings to post").into());
    }

    // Check if already reconciled
    if let Some(posting_idx) = posting_index {
        if entry
            .posted_postings
            .iter()
            .any(|(idx, _)| *idx == posting_idx)
        {
            return Err(
                format!("posting {posting_idx} of entry {entry_id} is already posted").into(),
            );
        }
    } else if entry.posted.is_some() {
        return Err(format!("entry {entry_id} is already posted").into());
    }

    // Generate GL transaction
    let gl_txn_id = uuid::Uuid::new_v4().to_string();
    let source_locator = format!("accounts/{account_name}");
    let gl_text = format_gl_transaction(
        entry,
        &source_locator,
        counterpart_account,
        &gl_txn_id,
        posting_index,
    );

    // Update account journal entry with reconciled tag
    let gl_ref = format!("general.journal:{gl_txn_id}");
    if let Some(posting_idx) = posting_index {
        entries[entry_idx]
            .posted_postings
            .push((posting_idx, gl_ref));
    } else {
        entries[entry_idx].posted = Some(gl_ref);
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
    let op = operations::GlOperation::Post {
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

/// Post a single login account journal entry to the GL by assigning a counterpart account.
pub fn post_login_account_entry(
    ledger_dir: &Path,
    login_name: &str,
    label: &str,
    entry_id: &str,
    counterpart_account: &str,
    posting_index: Option<usize>,
) -> Result<String, Box<dyn std::error::Error + Send + Sync>> {
    let journal_path = account_journal::login_account_journal_path(ledger_dir, login_name, label);
    let mut entries = account_journal::read_journal_at_path(&journal_path)?;
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
        return Err(format!("entry {entry_id} has no postings to post").into());
    }

    if let Some(posting_idx) = posting_index {
        if entry
            .posted_postings
            .iter()
            .any(|(idx, _)| *idx == posting_idx)
        {
            return Err(
                format!("posting {posting_idx} of entry {entry_id} is already posted").into(),
            );
        }
    } else if entry.posted.is_some() {
        return Err(format!("entry {entry_id} is already posted").into());
    }

    let gl_txn_id = uuid::Uuid::new_v4().to_string();
    let source_locator = format!("logins/{login_name}/accounts/{label}");
    let gl_text = format_gl_transaction(
        entry,
        &source_locator,
        counterpart_account,
        &gl_txn_id,
        posting_index,
    );

    let gl_ref = format!("general.journal:{gl_txn_id}");
    if let Some(posting_idx) = posting_index {
        entries[entry_idx]
            .posted_postings
            .push((posting_idx, gl_ref));
    } else {
        entries[entry_idx].posted = Some(gl_ref);
    }

    account_journal::write_journal_at_path(&journal_path, &entries)?;

    let gl_journal_path = ledger_dir.join("general.journal");
    if let Err(err) = append_to_journal(&gl_journal_path, &gl_text) {
        let _ = account_journal::write_journal_at_path(&journal_path, &original_entries);
        return Err(err.into());
    }

    let op = operations::GlOperation::Post {
        account: source_locator,
        entry_id: entry_id.to_string(),
        counterpart_account: counterpart_account.to_string(),
        posting_index,
        timestamp: operations::now_timestamp(),
    };
    if let Err(err) = operations::append_gl_operation(ledger_dir, &op) {
        let _ = remove_gl_transaction(ledger_dir, &gl_txn_id);
        let _ = account_journal::write_journal_at_path(&journal_path, &original_entries);
        return Err(err.into());
    }

    Ok(gl_txn_id)
}

// ---------------------------------------------------------------------------
// Transfer-aware unpost helpers
// ---------------------------------------------------------------------------

/// Find a GL block by its id tag without removing it.
fn find_gl_block(ledger_dir: &Path, gl_txn_id: &str) -> io::Result<Option<String>> {
    let journal_path = ledger_dir.join("general.journal");
    if !journal_path.exists() {
        return Ok(None);
    }
    let content = fs::read_to_string(&journal_path)?;
    let marker = format!("id: {gl_txn_id}");
    Ok(split_journal_blocks(&content)
        .into_iter()
        .find(|block| block.contains(&marker)))
}

/// Parse `; source: <locator>:<entry_id>` lines from a GL block.
///
/// Skips posting-indexed sources (`; source: ...:posting:<n>`).
/// Returns vec of `(locator, entry_id)`.
fn parse_sources_from_block(block: &str) -> Vec<(String, String)> {
    let mut sources = Vec::new();
    for line in block.lines() {
        let trimmed = line.trim();
        if let Some(rest) = trimmed.strip_prefix("; source: ") {
            if rest.contains(":posting:") {
                continue; // skip posting-indexed sources
            }
            if let Some(colon_pos) = rest.rfind(':') {
                let locator = rest[..colon_pos].to_string();
                let entry_id = rest[colon_pos + 1..].to_string();
                if !locator.is_empty() && !entry_id.is_empty() {
                    sources.push((locator, entry_id));
                }
            }
        }
    }
    sources
}

/// Resolve a source locator string to its journal file path.
fn journal_path_for_locator(ledger_dir: &Path, locator: &str) -> Option<std::path::PathBuf> {
    if let Some(rest) = locator.strip_prefix("logins/") {
        if let Some(accounts_pos) = rest.find("/accounts/") {
            let login = &rest[..accounts_pos];
            let label = &rest[accounts_pos + "/accounts/".len()..];
            return Some(account_journal::login_account_journal_path(
                ledger_dir, login, label,
            ));
        }
    } else if let Some(acct_name) = locator.strip_prefix("accounts/") {
        return Some(account_journal::account_journal_path(ledger_dir, acct_name));
    }
    None
}

/// Holds a pre-loaded other-side journal with the `posted` tag cleared for
/// the given entry, plus the original snapshot for rollback.
struct OtherSideJournal {
    path: std::path::PathBuf,
    updated: Vec<AccountEntry>,
    original: Vec<AccountEntry>,
}

/// Pre-load all source journals for a GL transaction except the triggering
/// `(triggering_locator, triggering_entry_id)` pair, with `posted` cleared
/// for each matching entry.  Fails fast before any GL mutation.
fn preload_other_sides(
    ledger_dir: &Path,
    gl_txn_id: &str,
    triggering_locator: &str,
    triggering_entry_id: &str,
) -> Result<Vec<OtherSideJournal>, Box<dyn std::error::Error + Send + Sync>> {
    let block = match find_gl_block(ledger_dir, gl_txn_id)? {
        Some(b) => b,
        None => return Ok(vec![]),
    };
    let mut other_sides = Vec::new();
    for (locator, entry_id) in parse_sources_from_block(&block) {
        if locator == triggering_locator && entry_id == triggering_entry_id {
            continue;
        }
        let path = journal_path_for_locator(ledger_dir, &locator)
            .ok_or_else(|| format!("unknown source locator: {locator}"))?;
        let original = account_journal::read_journal_at_path(&path)?;
        let mut updated = original.clone();
        if let Some(idx) = updated.iter().position(|e| e.id == entry_id) {
            updated[idx].posted = None;
        }
        other_sides.push(OtherSideJournal {
            path,
            updated,
            original,
        });
    }
    Ok(other_sides)
}

/// Write pre-loaded other-side journals.  On failure, best-effort restores
/// already-written journals and re-appends the removed GL block.
fn write_other_sides(
    ledger_dir: &Path,
    other_sides: &[OtherSideJournal],
    removed_gl_block: &Option<String>,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    for (i, side) in other_sides.iter().enumerate() {
        if let Err(err) = account_journal::write_journal_at_path(&side.path, &side.updated) {
            // Best-effort rollback
            if let Some(ref removed) = removed_gl_block {
                let _ = append_to_journal(&ledger_dir.join("general.journal"), removed);
            }
            for prev in other_sides.iter().take(i) {
                let _ = account_journal::write_journal_at_path(&prev.path, &prev.original);
            }
            return Err(err.into());
        }
    }
    Ok(())
}

/// Undo a posting by removing the GL entry and clearing posted tags.
///
/// For transfer GL transactions (two `; source:` lines), also clears the
/// `posted` tag on the other-side account journal entry.
pub fn unpost_entry(
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
            .posted_postings
            .iter()
            .position(|(idx, _)| *idx == posting_idx)
            .ok_or_else(|| format!("posting {posting_idx} of entry {entry_id} is not posted"))?;
        let (_, ref_str) = original_entries[entry_idx].posted_postings[pos].clone();
        ref_str
    } else {
        original_entries[entry_idx]
            .posted
            .clone()
            .ok_or_else(|| format!("entry {entry_id} is not posted"))?
    };

    let gl_txn_id = gl_ref.strip_prefix("general.journal:").unwrap_or(&gl_ref);
    let triggering_locator = format!("accounts/{account_name}");

    // Pre-load other-side journals before any mutation (fail fast).
    let other_sides = preload_other_sides(ledger_dir, gl_txn_id, &triggering_locator, entry_id)?;

    // Remove the GL transaction from general.journal (point of no return).
    let removed_gl_txn = remove_gl_transaction(ledger_dir, gl_txn_id)?;

    // Clear posted on other-side entries.
    write_other_sides(ledger_dir, &other_sides, &removed_gl_txn)?;

    // Update triggering account journal entry in memory.
    if let Some(posting_idx) = posting_index {
        if let Some(pos) = entries[entry_idx]
            .posted_postings
            .iter()
            .position(|(idx, _)| *idx == posting_idx)
        {
            entries[entry_idx].posted_postings.remove(pos);
        }
    } else {
        entries[entry_idx].posted = None;
    }

    // Write updated account journal.
    if let Err(err) = account_journal::write_journal(ledger_dir, account_name, &entries) {
        if let Some(removed) = &removed_gl_txn {
            let journal_path = ledger_dir.join("general.journal");
            let _ = append_to_journal(&journal_path, removed);
        }
        for side in &other_sides {
            let _ = account_journal::write_journal_at_path(&side.path, &side.original);
        }
        return Err(err.into());
    }

    // Log undo operation.
    let op = operations::GlOperation::UndoPost {
        account: account_name.to_string(),
        entry_id: entry_id.to_string(),
        posting_index,
        timestamp: operations::now_timestamp(),
    };
    if let Err(err) = operations::append_gl_operation(ledger_dir, &op) {
        let _ = account_journal::write_journal(ledger_dir, account_name, &original_entries);
        for side in &other_sides {
            let _ = account_journal::write_journal_at_path(&side.path, &side.original);
        }
        if let Some(removed) = removed_gl_txn {
            let journal_path = ledger_dir.join("general.journal");
            let _ = append_to_journal(&journal_path, &removed);
        }
        return Err(err.into());
    }

    Ok(())
}

/// Undo posting for a login account entry.
///
/// For transfer GL transactions (two `; source:` lines), also clears the
/// `posted` tag on the other-side account journal entry.
pub fn unpost_login_account_entry(
    ledger_dir: &Path,
    login_name: &str,
    label: &str,
    entry_id: &str,
    posting_index: Option<usize>,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let journal_path = account_journal::login_account_journal_path(ledger_dir, login_name, label);
    let mut entries = account_journal::read_journal_at_path(&journal_path)?;
    let original_entries = entries.clone();
    let entry_idx = entries
        .iter()
        .position(|e| e.id == entry_id)
        .ok_or_else(|| format!("entry not found: {entry_id}"))?;

    let gl_ref = if let Some(posting_idx) = posting_index {
        let pos = original_entries[entry_idx]
            .posted_postings
            .iter()
            .position(|(idx, _)| *idx == posting_idx)
            .ok_or_else(|| format!("posting {posting_idx} of entry {entry_id} is not posted"))?;
        let (_, ref_str) = original_entries[entry_idx].posted_postings[pos].clone();
        ref_str
    } else {
        original_entries[entry_idx]
            .posted
            .clone()
            .ok_or_else(|| format!("entry {entry_id} is not posted"))?
    };

    let gl_txn_id = gl_ref.strip_prefix("general.journal:").unwrap_or(&gl_ref);
    let source_locator = format!("logins/{login_name}/accounts/{label}");

    // Pre-load other-side journals before any mutation (fail fast).
    let other_sides = preload_other_sides(ledger_dir, gl_txn_id, &source_locator, entry_id)?;

    // Remove GL block (point of no return).
    let removed_gl_txn = remove_gl_transaction(ledger_dir, gl_txn_id)?;

    // Clear posted on other-side entries.
    write_other_sides(ledger_dir, &other_sides, &removed_gl_txn)?;

    // Update triggering entry in memory.
    if let Some(posting_idx) = posting_index {
        if let Some(pos) = entries[entry_idx]
            .posted_postings
            .iter()
            .position(|(idx, _)| *idx == posting_idx)
        {
            entries[entry_idx].posted_postings.remove(pos);
        }
    } else {
        entries[entry_idx].posted = None;
    }

    if let Err(err) = account_journal::write_journal_at_path(&journal_path, &entries) {
        if let Some(removed) = &removed_gl_txn {
            let gl_journal_path = ledger_dir.join("general.journal");
            let _ = append_to_journal(&gl_journal_path, removed);
        }
        for side in &other_sides {
            let _ = account_journal::write_journal_at_path(&side.path, &side.original);
        }
        return Err(err.into());
    }

    let op = operations::GlOperation::UndoPost {
        account: source_locator,
        entry_id: entry_id.to_string(),
        posting_index,
        timestamp: operations::now_timestamp(),
    };
    if let Err(err) = operations::append_gl_operation(ledger_dir, &op) {
        let _ = account_journal::write_journal_at_path(&journal_path, &original_entries);
        for side in &other_sides {
            let _ = account_journal::write_journal_at_path(&side.path, &side.original);
        }
        if let Some(removed) = removed_gl_txn {
            let gl_journal_path = ledger_dir.join("general.journal");
            let _ = append_to_journal(&gl_journal_path, &removed);
        }
        return Err(err.into());
    }

    Ok(())
}

/// `(login_name, label, entry)` triple returned by `get_unposted_entries_for_transfer`.
pub type UnpostedTransferEntry = (String, String, AccountEntry);

/// Get all unposted entries across ALL login accounts except the specified
/// `(exclude_login, exclude_label)` pair.  Sorted by date descending.
pub fn get_unposted_entries_for_transfer(
    ledger_dir: &Path,
    exclude_login: &str,
    exclude_label: &str,
) -> Result<Vec<UnpostedTransferEntry>, Box<dyn std::error::Error + Send + Sync>> {
    let logins = crate::login_config::list_logins(ledger_dir)?;
    let mut result: Vec<UnpostedTransferEntry> = Vec::new();

    for login in &logins {
        let config = crate::login_config::read_login_config(ledger_dir, login);
        for label in config.accounts.keys() {
            if login == exclude_login && label == exclude_label {
                continue;
            }
            let journal_path =
                account_journal::login_account_journal_path(ledger_dir, login, label);
            let entries = account_journal::read_journal_at_path(&journal_path)?;
            for entry in entries {
                if entry.posted.is_none() && entry.posted_postings.is_empty() {
                    result.push((login.clone(), label.clone(), entry));
                }
            }
        }
    }

    // Sort by date descending.
    result.sort_by(|a, b| b.2.date.cmp(&a.2.date));
    Ok(result)
}

/// Post two entries across accounts as an inter-account transfer.
pub fn post_transfer(
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

    // Check neither is already posted
    if entries1[idx1].posted.is_some() {
        return Err(format!("entry {entry_id1} in {account1} is already posted").into());
    }
    if entries2[idx2].posted.is_some() {
        return Err(format!("entry {entry_id2} in {account2} is already posted").into());
    }

    // Generate GL transaction for transfer
    let gl_txn_id = uuid::Uuid::new_v4().to_string();
    let source1 = format!("accounts/{account1}");
    let source2 = format!("accounts/{account2}");
    let gl_text = format_transfer_gl_transaction(
        &entries1[idx1],
        &source1,
        &entries2[idx2],
        &source2,
        &gl_txn_id,
    );

    // Update both account journal entries
    let gl_ref = format!("general.journal:{gl_txn_id}");
    entries1[idx1].posted = Some(gl_ref.clone());
    entries2[idx2].posted = Some(gl_ref);

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

/// Get unposted entries for an account.
pub fn get_unposted(
    ledger_dir: &Path,
    account_name: &str,
) -> Result<Vec<AccountEntry>, Box<dyn std::error::Error + Send + Sync>> {
    let entries = account_journal::read_journal(ledger_dir, account_name)?;
    Ok(entries.into_iter().filter(has_unposted_portion).collect())
}

/// Get unposted entries for a login account.
pub fn get_unposted_login_account(
    ledger_dir: &Path,
    login_name: &str,
    label: &str,
) -> Result<Vec<AccountEntry>, Box<dyn std::error::Error + Send + Sync>> {
    let journal_path = account_journal::login_account_journal_path(ledger_dir, login_name, label);
    let entries = account_journal::read_journal_at_path(&journal_path)?;
    Ok(entries.into_iter().filter(has_unposted_portion).collect())
}

fn has_unposted_portion(entry: &AccountEntry) -> bool {
    if entry.posted.is_some() {
        return false;
    }
    if entry.posted_postings.is_empty() {
        return true;
    }
    if entry.postings.is_empty() {
        return false;
    }

    let mut reconciled_mask = vec![false; entry.postings.len()];
    for (idx, _) in &entry.posted_postings {
        if *idx < reconciled_mask.len() {
            reconciled_mask[*idx] = true;
        }
    }
    reconciled_mask.iter().any(|is_reconciled| !is_reconciled)
}

/// Format a GL transaction for reconciliation.
fn format_gl_transaction(
    entry: &AccountEntry,
    source_locator: &str,
    counterpart_account: &str,
    gl_txn_id: &str,
    posting_index: Option<usize>,
) -> String {
    let source_tag = if let Some(posting_idx) = posting_index {
        format!(
            "; source: {}:{}:posting:{}",
            source_locator, entry.id, posting_idx
        )
    } else {
        format!("; source: {}:{}", source_locator, entry.id)
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

    let status_marker = entry.status.hledger_marker();
    format!(
        "{}  {}{}  ; id: {}\n    ; generated-by: refreshmint-post\n    {source_tag}\n    {real_account}  {amount_str}\n    {counterpart_account}\n",
        entry.date, status_marker, entry.description, gl_txn_id,
    )
}

/// Format a GL transaction for a transfer between two accounts.
fn format_transfer_gl_transaction(
    entry1: &AccountEntry,
    source1: &str,
    entry2: &AccountEntry,
    source2: &str,
    gl_txn_id: &str,
) -> String {
    use crate::account_journal::EntryStatus;
    // Both cleared → GL gets * (Cleared); either pending → GL gets ! (Pending); else unmarked.
    let status_marker =
        if entry1.status == EntryStatus::Cleared && entry2.status == EntryStatus::Cleared {
            "* "
        } else if entry1.status == EntryStatus::Pending || entry2.status == EntryStatus::Pending {
            "! "
        } else {
            ""
        };

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

    let real_account2 = entry2
        .postings
        .first()
        .map(|p| p.account.clone())
        .unwrap_or_default();

    format!(
        "{}  {}{}  ; id: {}\n    ; generated-by: refreshmint-post\n    ; source: {}:{}\n    ; source: {}:{}\n    {real_account1}  {amount1}\n    {real_account2}\n",
        entry1.date,
        status_marker,
        entry1.description,
        gl_txn_id,
        source1,
        entry1.id,
        source2,
        entry2.id,
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
            posted: None,
            posted_postings: Vec::new(),
        }
    }

    #[test]
    fn post_creates_gl_entry_and_tags_account() {
        let root = temp_dir("post");
        // Create general.journal
        fs::write(root.join("general.journal"), "").unwrap();

        let entries = vec![make_entry("txn-1", "2024-01-15", "Shell Oil", "-21.32")];
        account_journal::write_journal(&root, "chase", &entries).unwrap();

        let gl_id = post_entry(&root, "chase", "txn-1", "Expenses:Gas", None).unwrap();

        // Check GL entry was created
        let gl_content = fs::read_to_string(root.join("general.journal")).unwrap();
        assert!(gl_content.contains("Shell Oil"));
        assert!(gl_content.contains("Expenses:Gas"));
        assert!(gl_content.contains(&format!("id: {gl_id}")));
        assert!(gl_content.contains("generated-by: refreshmint-post"));
        assert!(gl_content.contains("source: accounts/chase:txn-1"));

        // Check account journal was updated
        let updated = account_journal::read_journal(&root, "chase").unwrap();
        assert_eq!(
            updated[0].posted.as_ref().unwrap(),
            &format!("general.journal:{gl_id}")
        );

        // Check GL operation was logged
        let ops = operations::read_gl_operations(&root).unwrap();
        assert_eq!(ops.len(), 1);

        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn unpost_removes_gl_entry() {
        let root = temp_dir("unpost");
        fs::write(root.join("general.journal"), "").unwrap();

        let entries = vec![make_entry("txn-1", "2024-01-15", "Shell Oil", "-21.32")];
        account_journal::write_journal(&root, "chase", &entries).unwrap();

        let gl_id = post_entry(&root, "chase", "txn-1", "Expenses:Gas", None).unwrap();

        // Verify GL entry exists
        let gl_before = fs::read_to_string(root.join("general.journal")).unwrap();
        assert!(gl_before.contains(&gl_id));

        // Unpost
        unpost_entry(&root, "chase", "txn-1", None).unwrap();

        // Check GL entry was removed
        let gl_after = fs::read_to_string(root.join("general.journal")).unwrap();
        assert!(!gl_after.contains(&gl_id));

        // Check account journal was updated
        let updated = account_journal::read_journal(&root, "chase").unwrap();
        assert!(updated[0].posted.is_none());

        // Check undo operation was logged
        let ops = operations::read_gl_operations(&root).unwrap();
        assert_eq!(ops.len(), 2); // post + undo-post

        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn get_unposted_filters_correctly() {
        let root = temp_dir("unposted-filter");

        let mut entries = vec![
            make_entry("txn-1", "2024-01-15", "Shell Oil", "-21.32"),
            make_entry("txn-2", "2024-01-16", "Walmart", "-50.00"),
        ];
        entries[0].posted = Some("general.journal:gl-1".to_string());

        account_journal::write_journal(&root, "test-acct", &entries).unwrap();

        let unreconciled = get_unposted(&root, "test-acct").unwrap();
        assert_eq!(unreconciled.len(), 1);
        assert_eq!(unreconciled[0].id, "txn-2");

        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn get_unposted_includes_partially_posted_multi_posting_entries() {
        let root = temp_dir("unposted-partial");
        let mut entry = make_entry("txn-1", "2024-01-15", "Venmo pass-through", "-21.32");
        entry.posted_postings = vec![(0, "general.journal:gl-1".to_string())];
        account_journal::write_journal(&root, "test-acct", &[entry]).unwrap();

        let unreconciled = get_unposted(&root, "test-acct").unwrap();
        assert_eq!(unreconciled.len(), 1);
        assert_eq!(unreconciled[0].id, "txn-1");

        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn post_rejects_out_of_bounds_posting_index() {
        let root = temp_dir("posting-index-bounds");
        fs::write(root.join("general.journal"), "").unwrap();
        let entries = vec![make_entry("txn-1", "2024-01-15", "Shell Oil", "-21.32")];
        account_journal::write_journal(&root, "chase", &entries).unwrap();

        let err = post_entry(&root, "chase", "txn-1", "Expenses:Gas", Some(99))
            .expect_err("out-of-bounds index should error");
        assert!(err.to_string().contains("out of bounds"));

        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn post_rejects_entry_without_postings() {
        let root = temp_dir("empty-postings");
        fs::write(root.join("general.journal"), "").unwrap();

        let entry = AccountEntry {
            id: "txn-1".to_string(),
            date: "2024-01-15".to_string(),
            status: EntryStatus::Cleared,
            description: "No postings".to_string(),
            comment: String::new(),
            evidence: vec!["doc.csv:1:1".to_string()],
            postings: Vec::new(),
            tags: vec![],
            extracted_by: None,
            posted: None,
            posted_postings: Vec::new(),
        };
        account_journal::write_journal(&root, "chase", &[entry]).unwrap();

        let err = post_entry(&root, "chase", "txn-1", "Expenses:Gas", None)
            .expect_err("empty postings should error");
        assert!(err.to_string().contains("has no postings"));

        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn format_gl_transaction_cleared_gets_star_marker() {
        let mut entry = make_entry("txn-1", "2024-01-15", "Shell Oil", "-21.32");
        entry.status = EntryStatus::Cleared;
        let text = format_gl_transaction(&entry, "accounts/chase", "Expenses:Gas", "gl-id", None);
        assert!(text.starts_with("2024-01-15  * Shell Oil"));
    }

    #[test]
    fn format_gl_transaction_pending_gets_exclamation_marker() {
        let mut entry = make_entry("txn-1", "2024-01-15", "Shell Oil", "-21.32");
        entry.status = EntryStatus::Pending;
        let text = format_gl_transaction(&entry, "accounts/chase", "Expenses:Gas", "gl-id", None);
        assert!(text.starts_with("2024-01-15  ! Shell Oil"));
    }

    #[test]
    fn format_gl_transaction_unmarked_has_no_status_marker() {
        let mut entry = make_entry("txn-1", "2024-01-15", "Shell Oil", "-21.32");
        entry.status = EntryStatus::Unmarked;
        let text = format_gl_transaction(&entry, "accounts/chase", "Expenses:Gas", "gl-id", None);
        assert!(text.starts_with("2024-01-15  Shell Oil"));
        assert!(!text.contains("* Shell Oil"));
        assert!(!text.contains("! Shell Oil"));
    }

    #[test]
    fn format_transfer_gl_transaction_both_cleared_gets_star() {
        let e1 = make_entry("txn-1", "2024-01-15", "Transfer", "-100.00");
        let e2 = make_entry("txn-2", "2024-01-15", "Transfer", "100.00");
        let text =
            format_transfer_gl_transaction(&e1, "accounts/chase", &e2, "accounts/boa", "gl-id");
        assert!(text.starts_with("2024-01-15  * Transfer"));
    }

    #[test]
    fn format_transfer_gl_transaction_one_pending_gets_exclamation() {
        let e1 = make_entry("txn-1", "2024-01-15", "Transfer", "-100.00");
        let mut e2 = make_entry("txn-2", "2024-01-15", "Transfer", "100.00");
        e2.status = EntryStatus::Pending;
        let text =
            format_transfer_gl_transaction(&e1, "accounts/chase", &e2, "accounts/boa", "gl-id");
        assert!(text.starts_with("2024-01-15  ! Transfer"));
    }

    #[test]
    fn format_transfer_gl_transaction_both_unmarked_has_no_marker() {
        let mut e1 = make_entry("txn-1", "2024-01-15", "Transfer", "-100.00");
        let mut e2 = make_entry("txn-2", "2024-01-15", "Transfer", "100.00");
        e1.status = EntryStatus::Unmarked;
        e2.status = EntryStatus::Unmarked;
        let text =
            format_transfer_gl_transaction(&e1, "accounts/chase", &e2, "accounts/boa", "gl-id");
        assert!(text.starts_with("2024-01-15  Transfer"));
        assert!(!text.contains("* Transfer"));
        assert!(!text.contains("! Transfer"));
    }

    #[test]
    fn unpost_transfer_clears_posted_on_both_sides() {
        let root = temp_dir("unpost-transfer");
        fs::write(root.join("general.journal"), "").unwrap();

        // Set up two accounts with one entry each.
        let entries1 = vec![make_entry("txn-a", "2024-01-15", "Transfer out", "-200.00")];
        let entries2 = vec![make_entry("txn-b", "2024-01-15", "Transfer in", "200.00")];
        account_journal::write_journal(&root, "chase", &entries1).unwrap();
        account_journal::write_journal(&root, "boa", &entries2).unwrap();

        // Post as a transfer.
        let gl_id = post_transfer(&root, "chase", "txn-a", "boa", "txn-b").unwrap();

        // Verify both sides are posted.
        let before1 = account_journal::read_journal(&root, "chase").unwrap();
        let before2 = account_journal::read_journal(&root, "boa").unwrap();
        assert!(before1[0].posted.is_some());
        assert!(before2[0].posted.is_some());

        // Unpost from the first side.
        unpost_entry(&root, "chase", "txn-a", None).unwrap();

        // GL block removed.
        let gl_content = fs::read_to_string(root.join("general.journal")).unwrap();
        assert!(!gl_content.contains(&gl_id));

        // Both sides cleared.
        let after1 = account_journal::read_journal(&root, "chase").unwrap();
        let after2 = account_journal::read_journal(&root, "boa").unwrap();
        assert!(
            after1[0].posted.is_none(),
            "triggering side should be unposted"
        );
        assert!(
            after2[0].posted.is_none(),
            "other side should also be unposted"
        );

        let _ = fs::remove_dir_all(&root);
    }
}
