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

    let commit_msg = format!("post: {entry_id} → {counterpart_account}");
    if let Err(err) = crate::ledger::commit_post_changes(ledger_dir, login_name, label, &commit_msg)
    {
        eprintln!("warning: git commit failed after post: {err}");
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

/// Post two login-account entries as an inter-account transfer.
///
/// Uses the new `logins/{login_name}/accounts/{label}` journal paths, unlike
/// `post_transfer` which uses the legacy `accounts/{name}` paths.
pub fn post_login_account_transfer(
    ledger_dir: &Path,
    login_name1: &str,
    label1: &str,
    entry_id1: &str,
    login_name2: &str,
    label2: &str,
    entry_id2: &str,
) -> Result<String, Box<dyn std::error::Error + Send + Sync>> {
    let journal_path1 =
        account_journal::login_account_journal_path(ledger_dir, login_name1, label1);
    let journal_path2 =
        account_journal::login_account_journal_path(ledger_dir, login_name2, label2);

    let mut entries1 = account_journal::read_journal_at_path(&journal_path1)?;
    let mut entries2 = account_journal::read_journal_at_path(&journal_path2)?;
    let original_entries1 = entries1.clone();
    let original_entries2 = entries2.clone();

    let idx1 = entries1
        .iter()
        .position(|e| e.id == entry_id1)
        .ok_or_else(|| format!("entry not found in {login_name1}/{label1}: {entry_id1}"))?;
    let idx2 = entries2
        .iter()
        .position(|e| e.id == entry_id2)
        .ok_or_else(|| format!("entry not found in {login_name2}/{label2}: {entry_id2}"))?;

    if entries1[idx1].posted.is_some() {
        return Err(
            format!("entry {entry_id1} in {login_name1}/{label1} is already posted").into(),
        );
    }
    if entries2[idx2].posted.is_some() {
        return Err(
            format!("entry {entry_id2} in {login_name2}/{label2} is already posted").into(),
        );
    }

    let gl_txn_id = uuid::Uuid::new_v4().to_string();
    let source1 = format!("logins/{login_name1}/accounts/{label1}");
    let source2 = format!("logins/{login_name2}/accounts/{label2}");
    let gl_text = format_transfer_gl_transaction(
        &entries1[idx1],
        &source1,
        &entries2[idx2],
        &source2,
        &gl_txn_id,
    );

    let gl_ref = format!("general.journal:{gl_txn_id}");
    entries1[idx1].posted = Some(gl_ref.clone());
    entries2[idx2].posted = Some(gl_ref);

    if let Err(err) = account_journal::write_journal_at_path(&journal_path1, &entries1) {
        return Err(err.into());
    }
    if let Err(err) = account_journal::write_journal_at_path(&journal_path2, &entries2) {
        let _ = account_journal::write_journal_at_path(&journal_path1, &original_entries1);
        return Err(err.into());
    }

    let journal_path = ledger_dir.join("general.journal");
    if let Err(err) = append_to_journal(&journal_path, &gl_text) {
        let _ = account_journal::write_journal_at_path(&journal_path1, &original_entries1);
        let _ = account_journal::write_journal_at_path(&journal_path2, &original_entries2);
        return Err(err.into());
    }

    let op = operations::GlOperation::TransferMatch {
        entries: vec![
            operations::TransferMatchEntry {
                account: source1,
                entry_id: entry_id1.to_string(),
            },
            operations::TransferMatchEntry {
                account: source2,
                entry_id: entry_id2.to_string(),
            },
        ],
        timestamp: operations::now_timestamp(),
    };
    if let Err(err) = operations::append_gl_operation(ledger_dir, &op) {
        let _ = remove_gl_transaction(ledger_dir, &gl_txn_id);
        let _ = account_journal::write_journal_at_path(&journal_path1, &original_entries1);
        let _ = account_journal::write_journal_at_path(&journal_path2, &original_entries2);
        return Err(err.into());
    }

    let commit_msg = format!("post transfer: {entry_id1} ↔ {entry_id2}");
    if let Err(err) = crate::ledger::commit_transfer_changes(
        ledger_dir,
        login_name1,
        label1,
        login_name2,
        label2,
        &commit_msg,
    ) {
        eprintln!("warning: git commit failed after transfer post: {err}");
    }

    Ok(gl_txn_id)
}

/// `(login_name, label, entry)` triple returned by `get_unposted_entries_for_transfer`.
pub type UnpostedTransferEntry = (String, String, AccountEntry);

/// Get all unposted entries across ALL login accounts except the specified
/// `(exclude_login, exclude_label)` pair.  Sorted by best-match score for
/// the source entry identified by `source_entry_id`.
pub fn get_unposted_entries_for_transfer(
    ledger_dir: &Path,
    exclude_login: &str,
    exclude_label: &str,
    source_entry_id: &str,
) -> Result<Vec<UnpostedTransferEntry>, Box<dyn std::error::Error + Send + Sync>> {
    // Load source entry for scoring.
    let source_journal_path =
        account_journal::login_account_journal_path(ledger_dir, exclude_login, exclude_label);
    let source_entries = account_journal::read_journal_at_path(&source_journal_path)?;
    let source_entry = source_entries
        .iter()
        .find(|e| e.id == source_entry_id)
        .cloned();

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

    if let Some(src) = source_entry {
        let src_date = src.date.clone();
        let src_desc = src.description.clone();
        let src_amount: Option<f64> = src
            .postings
            .first()
            .and_then(|p| p.amount.as_ref())
            .and_then(|a| a.quantity.parse().ok());

        result.sort_by(|a, b| {
            let score_a = transfer_candidate_score(&a.2, &src_date, &src_desc, src_amount);
            let score_b = transfer_candidate_score(&b.2, &src_date, &src_desc, src_amount);
            score_a.cmp(&score_b)
        });
    } else {
        // Fall back to date descending when source entry not found.
        result.sort_by(|a, b| b.2.date.cmp(&a.2.date));
    }

    Ok(result)
}

/// Compute a ranking score for a transfer candidate (lower = better match).
fn transfer_candidate_score(
    entry: &account_journal::AccountEntry,
    src_date: &str,
    src_desc: &str,
    src_amount: Option<f64>,
) -> i64 {
    let mut score: i64 = 0;

    // Penalize entries not labelled as transfers.
    if !crate::transfer_detector::is_probable_transfer(&entry.description) {
        score += 1000;
    }

    // Date proximity (more days away = higher penalty).
    if let (Ok(a), Ok(b)) = (
        chrono::NaiveDate::parse_from_str(src_date, "%Y-%m-%d"),
        chrono::NaiveDate::parse_from_str(&entry.date, "%Y-%m-%d"),
    ) {
        score += (a - b).num_days().abs() * 10;
    }

    // Reward opposite-sign amounts (characteristic of transfers).
    let entry_amount: Option<f64> = entry
        .postings
        .first()
        .and_then(|p| p.amount.as_ref())
        .and_then(|a| a.quantity.parse().ok());
    if let (Some(sa), Some(ea)) = (src_amount, entry_amount) {
        if (sa + ea).abs() < 0.005 {
            score -= 50;
        }
    }

    // Reward similar descriptions.
    if crate::dedup::descriptions_similar(src_desc, &entry.description) {
        score -= 20;
    }

    score
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
    let mut comment_lines = vec![
        "    ; generated-by: refreshmint-post".to_string(),
        format!("    {source_tag}"),
    ];
    for evidence_ref in collect_unique_evidence_refs([entry]) {
        comment_lines.push(format!("    ; evidence: {evidence_ref}"));
    }
    let comment_block = comment_lines.join("\n");

    format!(
        "{}  {}{}  ; id: {}\n{comment_block}\n    {real_account}  {amount_str}\n    {counterpart_account}\n",
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

    let mut comment_lines = vec![
        "    ; generated-by: refreshmint-post".to_string(),
        format!("    ; source: {source1}:{}", entry1.id),
        format!("    ; source: {source2}:{}", entry2.id),
    ];
    for evidence_ref in collect_unique_evidence_refs([entry1, entry2]) {
        comment_lines.push(format!("    ; evidence: {evidence_ref}"));
    }
    let comment_block = comment_lines.join("\n");

    format!(
        "{}  {}{}  ; id: {}\n{comment_block}\n    {real_account1}  {amount1}\n    {real_account2}\n",
        entry1.date,
        status_marker,
        entry1.description,
        gl_txn_id,
    )
}

fn collect_unique_evidence_refs<'a>(
    entries: impl IntoIterator<Item = &'a AccountEntry>,
) -> Vec<String> {
    let mut refs = std::collections::BTreeSet::new();
    for entry in entries {
        for ev in &entry.evidence {
            let trimmed = ev.trim();
            if !trimmed.is_empty() {
                refs.insert(trimmed.to_string());
            }
        }
    }
    refs.into_iter().collect()
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

/// Parse a `logins/{login}/accounts/{label}` locator into `(login, label)`.
fn locator_to_login_label(locator: &str) -> Option<(&str, &str)> {
    let rest = locator.strip_prefix("logins/")?;
    let pos = rest.find("/accounts/")?;
    let login = &rest[..pos];
    let label = &rest[pos + "/accounts/".len()..];
    if login.is_empty() || label.is_empty() {
        return None;
    }
    Some((login, label))
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

/// Replace a GL block in general.journal in-place.
///
/// Finds the block with `id: <gl_txn_id>` and replaces it with `new_block`.
fn replace_gl_block(ledger_dir: &Path, gl_txn_id: &str, new_block: &str) -> io::Result<()> {
    let journal_path = ledger_dir.join("general.journal");
    let content = fs::read_to_string(&journal_path)?;
    let marker = format!("id: {gl_txn_id}");
    let mut replaced = false;
    let blocks: Vec<String> = split_journal_blocks(&content)
        .into_iter()
        .map(|block| {
            if !replaced && block.contains(&marker) {
                replaced = true;
                new_block.trim_end().to_string()
            } else {
                block
            }
        })
        .collect();
    if !replaced {
        return Err(io::Error::other(format!(
            "GL transaction not found in general.journal: {gl_txn_id}"
        )));
    }
    let mut final_content = blocks.join("\n\n");
    if !final_content.is_empty() {
        final_content.push('\n');
    }
    fs::write(&journal_path, final_content)
}

/// Extract the counterpart account (last indented non-comment posting line) from a GL block.
fn extract_counterpart_from_block(block: &str) -> Option<String> {
    block
        .lines()
        .rfind(|line| {
            let is_indented = line.starts_with(' ') || line.starts_with('\t');
            let trimmed = line.trim();
            is_indented && !trimmed.is_empty() && !trimmed.starts_with(';')
        })
        .map(|line| line.trim().to_string())
}

/// Load account entries for each `(locator, entry_id)` pair.
///
/// Returns a vec of `(locator, entry_id, AccountEntry)` triples (same shape as
/// `UnpostedTransferEntry`).
fn load_source_entries(
    ledger_dir: &Path,
    sources: &[(String, String)],
) -> Result<Vec<UnpostedTransferEntry>, Box<dyn std::error::Error + Send + Sync>> {
    let mut result = Vec::new();
    for (locator, entry_id) in sources {
        let path = journal_path_for_locator(ledger_dir, locator)
            .ok_or_else(|| format!("unknown source locator: {locator}"))?;
        let entries = account_journal::read_journal_at_path(&path)?;
        let entry = entries
            .into_iter()
            .find(|e| &e.id == entry_id)
            .ok_or_else(|| format!("entry {entry_id} not found in {locator}"))?;
        result.push((locator.clone(), entry_id.clone(), entry));
    }
    Ok(result)
}

/// Sync an existing GL transaction in-place to reflect updated amounts/status.
///
/// Rebuilds the GL block from the current state of each source entry without
/// changing `; source:`, `; id:`, or `; generated-by:` tags.  The `posted`
/// ref on the account journal entry is left unchanged.
///
/// Returns the GL transaction UUID.
pub fn sync_gl_transaction(
    ledger_dir: &Path,
    login_name: &str,
    label: &str,
    entry_id: &str,
) -> Result<String, Box<dyn std::error::Error + Send + Sync>> {
    // 1. Load the triggering entry and get its GL ref.
    let journal_path = account_journal::login_account_journal_path(ledger_dir, login_name, label);
    let entries = account_journal::read_journal_at_path(&journal_path)?;
    let entry = entries
        .iter()
        .find(|e| e.id == entry_id)
        .ok_or_else(|| format!("entry not found: {entry_id}"))?;
    let gl_ref = entry
        .posted
        .as_ref()
        .ok_or_else(|| format!("entry {entry_id} is not posted"))?;
    let gl_txn_id = gl_ref
        .strip_prefix("general.journal:")
        .unwrap_or(gl_ref)
        .to_string();

    // 2. Find the existing GL block.
    let gl_block = find_gl_block(ledger_dir, &gl_txn_id)?
        .ok_or_else(|| format!("GL transaction not found: {gl_txn_id}"))?;

    // 3. Parse sources and load their current entries (fail fast before any writes).
    let raw_sources = parse_sources_from_block(&gl_block);
    let loaded = load_source_entries(ledger_dir, &raw_sources)?;

    // 4. Rebuild the GL block.
    let new_block = match loaded.as_slice() {
        [(loc1, _, e1), (loc2, _, e2)] => {
            // Transfer: two sources.
            format_transfer_gl_transaction(e1, loc1, e2, loc2, &gl_txn_id)
        }
        [(loc, _, e)] => {
            // Single posting: extract counterpart from existing block.
            let counterpart = extract_counterpart_from_block(&gl_block)
                .ok_or("could not extract counterpart account from GL block")?;
            format_gl_transaction(e, loc, &counterpart, &gl_txn_id, None)
        }
        _ => {
            return Err(format!(
                "unexpected source count: {} in GL block {gl_txn_id}",
                loaded.len()
            )
            .into());
        }
    };

    // 5. Replace GL block in general.journal (single file write; only point of mutation).
    replace_gl_block(ledger_dir, &gl_txn_id, &new_block)?;

    // 6. Append SyncTransaction to ops log (best-effort; non-fatal on failure).
    let sync_sources: Vec<operations::SyncSource> = loaded
        .iter()
        .map(|(loc, eid, e)| {
            let amount = e
                .postings
                .first()
                .and_then(|p| p.amount.as_ref())
                .map(|a| format!("{} {}", a.quantity, a.commodity));
            operations::SyncSource {
                account: loc.clone(),
                entry_id: eid.clone(),
                amount,
                status: e.status.hledger_marker().trim().to_string(),
            }
        })
        .collect();
    let source_locator = format!("logins/{login_name}/accounts/{label}");
    let op = operations::GlOperation::SyncTransaction {
        account: source_locator,
        entry_id: entry_id.to_string(),
        gl_txn_id: gl_txn_id.clone(),
        sources: sync_sources,
        timestamp: operations::now_timestamp(),
    };
    let _ = operations::append_gl_operation(ledger_dir, &op);

    Ok(gl_txn_id)
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

/// Replace `Expenses:Unknown` with `new_account` in an existing GL transaction.
///
/// Finds the block by `txn_id`, replaces the auto-balanced `Expenses:Unknown`
/// posting line (no explicit amount), writes the updated file, and commits.
pub fn recategorize_gl_transaction(
    ledger_dir: &Path,
    txn_id: &str,
    new_account: &str,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let journal_path = ledger_dir.join("general.journal");
    let content = fs::read_to_string(&journal_path)?;
    let marker = format!("id: {txn_id}");
    let mut found = false;

    let blocks: Vec<String> = split_journal_blocks(&content)
        .into_iter()
        .map(|block| {
            if !found && block.contains(&marker) {
                found = true;
                // Replace the bare `Expenses:Unknown` posting line.
                let new_block: String = block
                    .lines()
                    .map(|line| {
                        let is_indented = line.starts_with(' ') || line.starts_with('\t');
                        if is_indented && line.trim() == "Expenses:Unknown" {
                            let indent: String =
                                line.chars().take_while(|c| c.is_whitespace()).collect();
                            format!("{indent}{new_account}")
                        } else {
                            line.to_string()
                        }
                    })
                    .collect::<Vec<_>>()
                    .join("\n");
                new_block.trim_end().to_string()
            } else {
                block
            }
        })
        .collect();

    if !found {
        return Err(format!("GL transaction not found: {txn_id}").into());
    }

    let mut final_content = blocks.join("\n\n");
    if !final_content.is_empty() {
        final_content.push('\n');
    }
    fs::write(&journal_path, final_content)?;

    let commit_msg = format!("recategorize: {txn_id} → {new_account}");
    if let Err(err) = crate::ledger::commit_general_journal(ledger_dir, &commit_msg) {
        eprintln!("warning: git commit failed after recategorize: {err}");
    }

    Ok(())
}

/// Merge two `Expenses:Unknown` GL transactions into a single transfer transaction.
///
/// Both transactions must each have exactly one `; source:` tag pointing to a
/// login account journal entry.  The function:
/// 1. Removes both old GL blocks
/// 2. Appends a new two-posting transfer transaction
/// 3. Updates each source account entry's `posted:` ref to the new ID
/// 4. Commits all changed files
///
/// Returns the new GL transaction ID.
pub fn merge_gl_transfer(
    ledger_dir: &Path,
    txn_id_1: &str,
    txn_id_2: &str,
) -> Result<String, Box<dyn std::error::Error + Send + Sync>> {
    if txn_id_1 == txn_id_2 {
        return Err("cannot merge a transaction with itself".into());
    }

    // 1. Find both GL blocks.
    let block1 = find_gl_block(ledger_dir, txn_id_1)?
        .ok_or_else(|| format!("GL transaction not found: {txn_id_1}"))?;
    let block2 = find_gl_block(ledger_dir, txn_id_2)?
        .ok_or_else(|| format!("GL transaction not found: {txn_id_2}"))?;

    // 2. Parse sources (expect exactly one each).
    let sources1 = parse_sources_from_block(&block1);
    let sources2 = parse_sources_from_block(&block2);
    let (locator1, entry_id1) = sources1
        .into_iter()
        .next()
        .ok_or("GL transaction 1 has no source tag")?;
    let (locator2, entry_id2) = sources2
        .into_iter()
        .next()
        .ok_or("GL transaction 2 has no source tag")?;

    // 3. Resolve journal paths and load entries.
    let path1 = journal_path_for_locator(ledger_dir, &locator1)
        .ok_or_else(|| format!("unknown source locator: {locator1}"))?;
    let path2 = journal_path_for_locator(ledger_dir, &locator2)
        .ok_or_else(|| format!("unknown source locator: {locator2}"))?;

    let same_file = path1 == path2;

    let mut entries1 = account_journal::read_journal_at_path(&path1)?;
    let original_entries1 = entries1.clone();
    let idx1 = entries1
        .iter()
        .position(|e| e.id == entry_id1)
        .ok_or_else(|| format!("entry {entry_id1} not found in {locator1}"))?;

    let mut entries2;
    let original_entries2;
    let idx2;
    if same_file {
        entries2 = entries1.clone();
        original_entries2 = original_entries1.clone();
        idx2 = entries2
            .iter()
            .position(|e| e.id == entry_id2)
            .ok_or_else(|| format!("entry {entry_id2} not found in {locator2}"))?;
    } else {
        let loaded = account_journal::read_journal_at_path(&path2)?;
        original_entries2 = loaded.clone();
        idx2 = loaded
            .iter()
            .position(|e| e.id == entry_id2)
            .ok_or_else(|| format!("entry {entry_id2} not found in {locator2}"))?;
        entries2 = loaded;
    }

    // 4. Generate new UUID.
    let new_uuid = uuid::Uuid::new_v4().to_string();

    // 5. Build merged transfer GL text using the two account entries.
    let gl_text = format_transfer_gl_transaction(
        &entries1[idx1],
        &locator1,
        &entries2[idx2],
        &locator2,
        &new_uuid,
    );

    // 6. Compute new GL content: remove both old blocks, append merged.
    let gl_journal_path = ledger_dir.join("general.journal");
    let original_gl_content = fs::read_to_string(&gl_journal_path)?;
    let marker1 = format!("id: {txn_id_1}");
    let marker2 = format!("id: {txn_id_2}");
    let kept_blocks: Vec<String> = split_journal_blocks(&original_gl_content)
        .into_iter()
        .filter(|block| !block.contains(&marker1) && !block.contains(&marker2))
        .collect();
    let mut new_gl_content = kept_blocks.join("\n\n");
    if !new_gl_content.is_empty() {
        new_gl_content.push_str("\n\n");
    }
    new_gl_content.push_str(&gl_text);

    // 7. Update posted refs in account entries.
    let new_gl_ref = format!("general.journal:{new_uuid}");
    entries1[idx1].posted = Some(new_gl_ref.clone());
    if same_file {
        entries1[idx2].posted = Some(new_gl_ref);
    } else {
        entries2[idx2].posted = Some(new_gl_ref);
    }

    // 8. Write account journals first, then general.journal.
    account_journal::write_journal_at_path(&path1, &entries1)?;
    if !same_file {
        if let Err(err) = account_journal::write_journal_at_path(&path2, &entries2) {
            let _ = account_journal::write_journal_at_path(&path1, &original_entries1);
            return Err(err.into());
        }
    }
    if let Err(err) = fs::write(&gl_journal_path, &new_gl_content) {
        let _ = account_journal::write_journal_at_path(&path1, &original_entries1);
        if !same_file {
            let _ = account_journal::write_journal_at_path(&path2, &original_entries2);
        }
        return Err(err.into());
    }

    // 9. Commit all changed files.
    let commit_msg = format!("merge transfer: {txn_id_1} + {txn_id_2} → {new_uuid}");
    let commit_result = match (
        locator_to_login_label(&locator1),
        locator_to_login_label(&locator2),
    ) {
        (Some((ln1, lb1)), Some((ln2, lb2))) => {
            crate::ledger::commit_transfer_changes(ledger_dir, ln1, lb1, ln2, lb2, &commit_msg)
        }
        (Some((ln1, lb1)), None) => {
            crate::ledger::commit_post_changes(ledger_dir, ln1, lb1, &commit_msg)
        }
        _ => crate::ledger::commit_general_journal(ledger_dir, &commit_msg),
    };
    if let Err(err) = commit_result {
        eprintln!("warning: git commit failed after merge_gl_transfer: {err}");
    }

    Ok(new_uuid)
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
        assert!(gl_content.contains("evidence: doc.csv:1:1"));

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
    fn format_transfer_gl_transaction_includes_unique_evidence() {
        let mut e1 = make_entry("txn-1", "2024-01-15", "Transfer", "-100.00");
        let mut e2 = make_entry("txn-2", "2024-01-15", "Transfer", "100.00");
        e1.evidence = vec![
            "doc-a.csv:1:1".to_string(),
            "shared.csv:7:1".to_string(),
            "shared.csv:7:1".to_string(),
        ];
        e2.evidence = vec!["doc-b.csv:2:1".to_string(), "shared.csv:7:1".to_string()];
        let text =
            format_transfer_gl_transaction(&e1, "accounts/chase", &e2, "accounts/boa", "gl-id");
        assert!(text.contains("evidence: doc-a.csv:1:1"));
        assert!(text.contains("evidence: doc-b.csv:2:1"));
        assert!(text.contains("evidence: shared.csv:7:1"));
        assert_eq!(text.matches("evidence: shared.csv:7:1").count(), 1);
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

    #[test]
    fn sync_gl_transaction_updates_amount_and_status_in_place() {
        let root = temp_dir("sync-gl");
        fs::write(root.join("general.journal"), "").unwrap();

        // Set up a login account entry and post it.
        let entry = make_entry("txn-1", "2024-01-15", "Shell Oil", "-21.32");
        let journal_path = account_journal::login_account_journal_path(&root, "chase", "checking");
        account_journal::write_journal_at_path(&journal_path, &[entry]).unwrap();

        let gl_id =
            post_login_account_entry(&root, "chase", "checking", "txn-1", "Expenses:Gas", None)
                .unwrap();

        // Mutate the entry: change amount and set status to Pending.
        let mut entries = account_journal::read_journal_at_path(&journal_path).unwrap();
        entries[0].postings[0].amount = Some(account_journal::SimpleAmount {
            commodity: "USD".to_string(),
            quantity: "-25.00".to_string(),
        });
        entries[0].status = EntryStatus::Pending;
        account_journal::write_journal_at_path(&journal_path, &entries).unwrap();

        // Sync the GL transaction.
        let returned_id = sync_gl_transaction(&root, "chase", "checking", "txn-1").unwrap();
        assert_eq!(
            returned_id, gl_id,
            "returned ID must match original GL txn ID"
        );

        // GL block reflects new amount and status.
        let gl_content = fs::read_to_string(root.join("general.journal")).unwrap();
        assert!(gl_content.contains("-25.00"), "amount should be updated");
        assert!(
            gl_content.contains(&format!("id: {gl_id}")),
            "id tag must be preserved"
        );
        assert!(
            gl_content.contains("! Shell Oil"),
            "status marker should be !"
        );
        assert!(
            gl_content.contains("source: logins/chase/accounts/checking:txn-1"),
            "source tag must be preserved"
        );
        assert!(
            gl_content.contains("Expenses:Gas"),
            "counterpart must be preserved"
        );
        // Old amount must be gone.
        assert!(!gl_content.contains("-21.32"), "old amount should be gone");

        // The `posted` ref on the account entry is unchanged.
        let after = account_journal::read_journal_at_path(&journal_path).unwrap();
        assert_eq!(
            after[0].posted.as_deref(),
            Some(&format!("general.journal:{gl_id}")[..]),
            "posted ref must be unchanged"
        );

        // Ops log has post + sync.
        let ops = operations::read_gl_operations(&root).unwrap();
        assert_eq!(ops.len(), 2);
        matches!(&ops[1], operations::GlOperation::SyncTransaction { .. });

        let _ = fs::remove_dir_all(&root);
    }
}
