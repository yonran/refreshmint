use std::fs;
use std::io;
use std::path::Path;

use serde::Deserialize;

use crate::account_journal::{self, AccountEntry};
use crate::login_config;
use crate::operations;

/// One leg of a split posting supplied by the caller.
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SplitCounterpart {
    pub account: String,
    /// Explicit amount string (e.g. `"100.00 USD"`).  The last leg may omit
    /// this and hledger will infer the remainder, but callers should always
    /// supply amounts so the GL is unambiguous.
    pub amount: Option<String>,
}

/// Post a single login account journal entry to the GL by assigning a counterpart account.
pub fn post_login_account_entry(
    ledger_dir: &Path,
    login_name: &str,
    label: &str,
    entry_id: &str,
    counterpart_account: &str,
    posting_index: Option<usize>,
    lock_owner: &str,
) -> Result<String, Box<dyn std::error::Error + Send + Sync>> {
    let _gl_lock =
        login_config::acquire_gl_lock_with_metadata(ledger_dir, lock_owner, "post-login-entry")?;
    let _login_lock = login_config::acquire_login_lock_with_metadata(
        ledger_dir,
        login_name,
        lock_owner,
        "post-login-entry",
    )?;
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

    // A whole-entry post and per-leg posts are mutually exclusive; allowing
    // both would materialize the same source amount twice.
    if entry.posted.is_some() {
        return Err(format!("entry {entry_id} is already posted").into());
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
    } else if !entry.posted_postings.is_empty() {
        return Err(format!(
            "entry {entry_id} has posted split postings; unpost them before posting the whole entry"
        )
        .into());
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

    // GL-first: the GL block is self-describing (it carries a `; source:`
    // backref), so a kill before the account journal is written leaves a
    // recoverable orphan, not a lossy dangling ref. consistency::recover_ledger
    // restores the account ref from the GL on next open. See [crate::consistency].
    let gl_journal_path = ledger_dir.join("general.journal");
    append_to_journal(&gl_journal_path, &gl_text)?;

    if let Err(err) = account_journal::write_journal_at_path(&journal_path, &entries) {
        let _ = remove_gl_transaction(ledger_dir, &gl_txn_id);
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
    crate::ledger::commit_post_changes(ledger_dir, login_name, label, &commit_msg)?;

    Ok(gl_txn_id)
}

/// Post a single login account journal entry to the GL, splitting the amount
/// across multiple counterpart accounts.
pub fn post_login_account_entry_split(
    ledger_dir: &Path,
    login_name: &str,
    label: &str,
    entry_id: &str,
    counterparts: Vec<SplitCounterpart>,
    lock_owner: &str,
) -> Result<String, Box<dyn std::error::Error + Send + Sync>> {
    if counterparts.len() < 2 {
        return Err("split requires at least 2 counterpart accounts".into());
    }
    if counterparts.iter().any(|c| c.account.trim().is_empty()) {
        return Err("all counterpart accounts must be non-empty".into());
    }

    let _gl_lock =
        login_config::acquire_gl_lock_with_metadata(ledger_dir, lock_owner, "post-login-split")?;
    let _login_lock = login_config::acquire_login_lock_with_metadata(
        ledger_dir,
        login_name,
        lock_owner,
        "post-login-split",
    )?;
    let journal_path = account_journal::login_account_journal_path(ledger_dir, login_name, label);
    let mut entries = account_journal::read_journal_at_path(&journal_path)?;
    let original_entries = entries.clone();
    let entry_idx = entries
        .iter()
        .position(|e| e.id == entry_id)
        .ok_or_else(|| format!("entry not found: {entry_id}"))?;

    let entry = &entries[entry_idx];

    if entry.postings.is_empty() {
        return Err(format!("entry {entry_id} has no postings to post").into());
    }
    if entry.posted.is_some() {
        return Err(format!("entry {entry_id} is already posted").into());
    }
    if !entry.posted_postings.is_empty() {
        return Err(format!(
            "entry {entry_id} has posted split postings; unpost them before posting a split"
        )
        .into());
    }

    let gl_txn_id = uuid::Uuid::new_v4().to_string();
    let source_locator = format!("logins/{login_name}/accounts/{label}");
    let gl_text = format_gl_split_transaction(entry, &source_locator, &counterparts, &gl_txn_id);

    let gl_ref = format!("general.journal:{gl_txn_id}");
    entries[entry_idx].posted = Some(gl_ref);

    // GL-first (see post_login_account_entry): a crash leaves a recoverable
    // orphan rather than a lossy dangling ref.
    let gl_journal_path = ledger_dir.join("general.journal");
    append_to_journal(&gl_journal_path, &gl_text)?;

    if let Err(err) = account_journal::write_journal_at_path(&journal_path, &entries) {
        let _ = remove_gl_transaction(ledger_dir, &gl_txn_id);
        return Err(err.into());
    }

    let counterpart_accounts: Vec<String> =
        counterparts.iter().map(|c| c.account.clone()).collect();
    let op = operations::GlOperation::PostSplit {
        account: source_locator,
        entry_id: entry_id.to_string(),
        counterpart_accounts,
        timestamp: operations::now_timestamp(),
    };
    if let Err(err) = operations::append_gl_operation(ledger_dir, &op) {
        let _ = remove_gl_transaction(ledger_dir, &gl_txn_id);
        let _ = account_journal::write_journal_at_path(&journal_path, &original_entries);
        return Err(err.into());
    }

    let counterpart_summary = counterparts
        .iter()
        .map(|c| c.account.as_str())
        .collect::<Vec<_>>()
        .join(" + ");
    let commit_msg = format!("post: {entry_id} → {counterpart_summary}");
    crate::ledger::commit_post_changes(ledger_dir, login_name, label, &commit_msg)?;

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
    Ok(crate::gl_journal::split_journal_blocks(&content)
        .into_iter()
        .find(|block| block.contains(&marker)))
}

/// Parse `; source: <locator>:<entry_id>` lines from a GL block.
///
/// Skips posting-indexed sources (`; source: ...:posting:<n>`).
/// Returns vec of `(locator, entry_id)`.
pub(crate) fn parse_sources_from_block(block: &str) -> Vec<(String, String)> {
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

/// Like [parse_sources_from_block] but also returns the per-posting index for
/// `; source: <locator>:<entry_id>:posting:<n>` lines (`None` for whole-entry
/// sources). Used by [crate::consistency] so per-leg posts are checked and
/// recovered, not silently skipped.
pub(crate) fn parse_sources_with_posting_from_block(
    block: &str,
) -> Vec<(String, String, Option<usize>)> {
    let mut sources = Vec::new();
    for line in block.lines() {
        let trimmed = line.trim();
        let Some(rest) = trimmed.strip_prefix("; source: ") else {
            continue;
        };
        let (body, posting_index) = match rest.rsplit_once(":posting:") {
            Some((body, idx)) => match idx.parse::<usize>() {
                Ok(index) => (body, Some(index)),
                Err(_) => continue, // malformed posting index
            },
            None => (rest, None),
        };
        // `body` is `<locator>:<entry_id>`; neither part contains a colon
        // (logins/labels are sanitized, entry ids are uuids/hashes).
        if let Some(colon_pos) = body.rfind(':') {
            let locator = body[..colon_pos].to_string();
            let entry_id = body[colon_pos + 1..].to_string();
            if !locator.is_empty() && !entry_id.is_empty() {
                sources.push((locator, entry_id, posting_index));
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
    // When Some, refuse unless the entry's CURRENT posted ref resolves to this GL
    // txn id. Callers that resolve the entry indirectly (e.g. from an orphaned GL
    // block's source tag; see unpost_gl_transaction) pass Some so a re-posted
    // entry is not silently unposted from a different, live block. Read under the
    // GL lock, so this doubles as the TOCTOU check. Entry-level callers pass None.
    expected_gl_txn: Option<&str>,
    lock_owner: &str,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let _gl_lock =
        login_config::acquire_gl_lock_with_metadata(ledger_dir, lock_owner, "unpost-login-entry")?;
    let preview_journal_path =
        account_journal::login_account_journal_path(ledger_dir, login_name, label);
    let preview_entries = account_journal::read_journal_at_path(&preview_journal_path)?;
    let preview_entry = preview_entries
        .iter()
        .find(|e| e.id == entry_id)
        .ok_or_else(|| format!("entry not found: {entry_id}"))?;
    let gl_ref = if let Some(posting_idx) = posting_index {
        let pos = preview_entry
            .posted_postings
            .iter()
            .position(|(idx, _)| *idx == posting_idx)
            .ok_or_else(|| format!("posting {posting_idx} of entry {entry_id} is not posted"))?;
        let (_, ref_str) = preview_entry.posted_postings[pos].clone();
        ref_str
    } else {
        preview_entry
            .posted
            .clone()
            .ok_or_else(|| format!("entry {entry_id} is not posted"))?
    };

    let gl_txn_id = gl_ref.strip_prefix("general.journal:").unwrap_or(&gl_ref);
    if let Some(expected) = expected_gl_txn {
        let expected = expected
            .strip_prefix("general.journal:")
            .unwrap_or(expected);
        if expected != gl_txn_id {
            return Err(
                format!("entry {entry_id} is posted to {gl_txn_id}, not {expected}").into(),
            );
        }
    }
    let source_locator = format!("logins/{login_name}/accounts/{label}");
    let gl_block = find_gl_block(ledger_dir, gl_txn_id)?
        .ok_or_else(|| format!("GL transaction not found: {gl_txn_id}"))?;
    let source_logins = source_login_names_from_block(&gl_block);
    let _login_locks = acquire_login_locks_for_names(
        ledger_dir,
        &source_logins,
        lock_owner,
        "unpost-login-entry",
    )?;
    let journal_path = account_journal::login_account_journal_path(ledger_dir, login_name, label);
    let mut entries = account_journal::read_journal_at_path(&journal_path)?;
    let original_entries = entries.clone();
    let entry_idx = entries
        .iter()
        .position(|e| e.id == entry_id)
        .ok_or_else(|| format!("entry not found: {entry_id}"))?;

    // Pre-load other-side journals before any mutation (fail fast).
    let other_sides = preload_other_sides(ledger_dir, gl_txn_id, &source_locator, entry_id)?;

    // A reconciled, linked, or soft-closed GL transaction must not be silently
    // removed by an unpost; mirror the guard retire_login_account_entry uses.
    let blockers = crate::bookkeeping::gl_txn_removal_blockers(ledger_dir, gl_txn_id)?;
    if !blockers.is_empty() {
        return Err(format!(
            "cannot unpost entry {entry_id}; GL transaction {gl_txn_id} is protected: {}",
            blockers.join(", ")
        )
        .into());
    }

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

    let gl_journal_path = ledger_dir.join("general.journal");
    let mut committed: Vec<&Path> = vec![gl_journal_path.as_path(), journal_path.as_path()];
    for side in &other_sides {
        committed.push(side.path.as_path());
    }
    crate::ledger::commit_files(ledger_dir, &committed, &format!("unpost: {entry_id}"))?;

    // Auto-record negative transfer memory. Unposting a merged transfer (a GL
    // block with exactly two `; source:` legs) records a NotTransferLink for the
    // pair so it does not silently re-post; mutual exclusion also disables the
    // TransferLink twin. Consulted by automation::TransferPolicy (loaded into
    // categorize::find_transfer_matches / find_gl_transfer_matches, and the auto-post
    // paths). Best-effort AFTER the committed unpost: a failure here must NOT roll
    // it back (log + proceed).
    let sources = parse_sources_from_block(&gl_block);
    if sources.len() == 2 {
        let triples: Vec<(String, String, String)> = sources
            .iter()
            .filter_map(|(locator, entry_id)| {
                locator_to_login_label(locator)
                    .map(|(login, label)| (login.to_string(), label.to_string(), entry_id.clone()))
            })
            .collect();
        if let [a, b] = triples.as_slice() {
            if let Err(err) = crate::automation::create_not_transfer_link(
                ledger_dir,
                (&a.0, &a.1, &a.2),
                (&b.0, &b.1, &b.2),
            ) {
                eprintln!(
                    "unpost {entry_id}: failed to record not-transfer-link resolution: {err}"
                );
            }
        }
    }

    Ok(())
}

/// Unpost a generated GL transaction by its GL txn id (server-side identity
/// resolution for the Transactions tab "Unmerge transfer" action): parse the
/// block's FIRST `; source:` tag into (login, label, entry_id) and delegate to
/// [`unpost_login_account_entry`], which clears every side's `posted` ref and,
/// for a 2-source transfer, records the NotTransferLink negative memory.
pub fn unpost_gl_transaction(
    ledger_dir: &Path,
    gl_txn_id: &str,
    lock_owner: &str,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let block = find_gl_block(ledger_dir, gl_txn_id)?
        .ok_or_else(|| format!("GL transaction not found: {gl_txn_id}"))?;
    let (locator, entry_id) = parse_sources_from_block(&block)
        .into_iter()
        .next()
        .ok_or_else(|| {
            format!("GL transaction {gl_txn_id} has no source tag; cannot unpost by GL id")
        })?;
    let (login_name, label) = locator_to_login_label(&locator)
        .ok_or_else(|| format!("GL transaction {gl_txn_id} has a non-login source: {locator}"))?;
    unpost_login_account_entry(
        ledger_dir,
        login_name,
        label,
        &entry_id,
        None,
        Some(gl_txn_id),
        lock_owner,
    )
}

/// Record NotTransferLink negative memory for a pair of generated GL txns
/// (the Transactions tab "Not a transfer" action): parse BOTH txns' first
/// `; source:` tag into source entries and create the resolution on that pair —
/// GL txn ids are not durable across merges/unposts, source entries are. Errors
/// when either txn lacks a source tag (manual GL txns can't carry negative
/// memory; documented limitation).
pub fn create_not_transfer_link_for_gl_pair(
    ledger_dir: &Path,
    txn_id_1: &str,
    txn_id_2: &str,
) -> Result<crate::automation::Resolution, Box<dyn std::error::Error + Send + Sync>> {
    if txn_id_1 == txn_id_2 {
        return Err("cannot record not-a-transfer for a transaction with itself".into());
    }
    let mut sides = Vec::new();
    for txn_id in [txn_id_1, txn_id_2] {
        let block = find_gl_block(ledger_dir, txn_id)?
            .ok_or_else(|| format!("GL transaction not found: {txn_id}"))?;
        let (locator, entry_id) = parse_sources_from_block(&block)
            .into_iter()
            .next()
            .ok_or_else(|| {
                format!(
                    "GL transaction {txn_id} has no source tag; cannot record not-a-transfer for a manual transaction"
                )
            })?;
        let (login_name, label) = locator_to_login_label(&locator)
            .map(|(login, label)| (login.to_string(), label.to_string()))
            .ok_or_else(|| format!("GL transaction {txn_id} has a non-login source: {locator}"))?;
        sides.push((login_name, label, entry_id));
    }
    let [a, b] = sides.as_slice() else {
        unreachable!("two txn ids produce two sides");
    };
    // Two distinct GL txns whose first source tags resolve to the SAME account
    // entry (e.g. an orphaned duplicate block) would block the entry against
    // itself. Refuse.
    if a == b {
        return Err(format!(
            "cannot record not-a-transfer; both transactions resolve to the same source entry {}/{}:{}",
            a.0, a.1, a.2
        )
        .into());
    }
    Ok(crate::automation::create_not_transfer_link(
        ledger_dir,
        (&a.0, &a.1, &a.2),
        (&b.0, &b.1, &b.2),
    )?)
}

/// Repair a dangling `posted:` ref: an account entry claims it is posted to a GL
/// transaction that no longer exists (see [crate::consistency]). Clears only the
/// account-side ref so the entry shows as unposted again and can be re-posted;
/// there is no GL transaction to remove.
///
/// Guards against clobbering a valid post: refuses if the referenced GL
/// transaction actually exists.
pub fn repair_dangling_ref(
    ledger_dir: &Path,
    login_name: &str,
    label: &str,
    entry_id: &str,
    posting_index: Option<usize>,
    lock_owner: &str,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let _gl_lock =
        login_config::acquire_gl_lock_with_metadata(ledger_dir, lock_owner, "repair-dangling-ref")?;
    let _login_lock = login_config::acquire_login_lock_with_metadata(
        ledger_dir,
        login_name,
        lock_owner,
        "repair-dangling-ref",
    )?;

    let journal_path = account_journal::login_account_journal_path(ledger_dir, login_name, label);
    let mut entries = account_journal::read_journal_at_path(&journal_path)?;
    let original_entries = entries.clone();
    let entry_idx = entries
        .iter()
        .position(|e| e.id == entry_id)
        .ok_or_else(|| format!("entry not found: {entry_id}"))?;

    let target_ref = match posting_index {
        Some(idx) => entries[entry_idx]
            .posted_postings
            .iter()
            .find(|(i, _)| *i == idx)
            .map(|(_, r)| r.clone()),
        None => entries[entry_idx].posted.clone(),
    }
    .ok_or_else(|| format!("entry {entry_id} has no matching posted ref to repair"))?;

    let gl_txn_id = target_ref
        .strip_prefix("general.journal:")
        .unwrap_or(&target_ref)
        .to_string();
    if find_gl_block(ledger_dir, &gl_txn_id)?.is_some() {
        return Err(format!(
            "refusing to clear ref for entry {entry_id}: GL transaction {gl_txn_id} exists (not dangling)"
        )
        .into());
    }

    match posting_index {
        Some(idx) => entries[entry_idx]
            .posted_postings
            .retain(|(i, _)| *i != idx),
        None => entries[entry_idx].posted = None,
    }
    account_journal::write_journal_at_path(&journal_path, &entries)?;

    let op = operations::GlOperation::UndoPost {
        account: format!("logins/{login_name}/accounts/{label}"),
        entry_id: entry_id.to_string(),
        posting_index,
        timestamp: operations::now_timestamp(),
    };
    if let Err(err) = operations::append_gl_operation(ledger_dir, &op) {
        let _ = account_journal::write_journal_at_path(&journal_path, &original_entries);
        return Err(err.into());
    }
    Ok(())
}

/// Repair an orphaned GL transaction: a refreshmint-generated GL transaction
/// whose source entry does not reference it back (see [crate::consistency]).
/// Removes the GL transaction; the source entry is already unposted, so it can
/// be re-posted cleanly.
///
/// Refuses if the GL transaction is protected (reconciled / linked / soft-closed),
/// mirroring the guard in [unpost_login_account_entry].
pub fn repair_orphaned_gl_txn(
    ledger_dir: &Path,
    gl_txn_id: &str,
    lock_owner: &str,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let _gl_lock = login_config::acquire_gl_lock_with_metadata(
        ledger_dir,
        lock_owner,
        "repair-orphaned-gl-txn",
    )?;

    let block = find_gl_block(ledger_dir, gl_txn_id)?
        .ok_or_else(|| format!("GL transaction not found: {gl_txn_id}"))?;

    let blockers = crate::bookkeeping::gl_txn_removal_blockers(ledger_dir, gl_txn_id)?;
    if !blockers.is_empty() {
        return Err(format!(
            "cannot remove GL transaction {gl_txn_id}; it is protected: {}",
            blockers.join(", ")
        )
        .into());
    }

    remove_gl_transaction(ledger_dir, gl_txn_id)?;

    // Best-effort audit entry, attributed to the orphan's source if present.
    let (account, entry_id) = parse_sources_from_block(&block)
        .into_iter()
        .next()
        .unwrap_or_else(|| (String::new(), gl_txn_id.to_string()));
    let op = operations::GlOperation::UndoPost {
        account,
        entry_id,
        posting_index: None,
        timestamp: operations::now_timestamp(),
    };
    let _ = operations::append_gl_operation(ledger_dir, &op);
    Ok(())
}

/// Restore an account entry's whole-entry `posted:` ref from an existing GL
/// transaction — the recovery half of GL-first posting (see the post ordering
/// above and [crate::consistency]). After a hard kill between the GL append and
/// the account write, the GL transaction is the source of truth and names its
/// source entry via `; source:`; this re-links the entry so it shows as posted
/// again (no duplicate on a subsequent post).
///
/// Idempotent: a no-op if the entry already references the GL txn. Refuses if
/// the GL txn is absent (nothing to restore from) or the entry already points at
/// a different GL txn (don't clobber).
pub fn restore_posted_ref(
    ledger_dir: &Path,
    login_name: &str,
    label: &str,
    entry_id: &str,
    posting_index: Option<usize>,
    gl_txn_id: &str,
    lock_owner: &str,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let _gl_lock =
        login_config::acquire_gl_lock_with_metadata(ledger_dir, lock_owner, "restore-posted-ref")?;
    let _login_lock = login_config::acquire_login_lock_with_metadata(
        ledger_dir,
        login_name,
        lock_owner,
        "restore-posted-ref",
    )?;

    if find_gl_block(ledger_dir, gl_txn_id)?.is_none() {
        return Err(format!("GL transaction not found: {gl_txn_id}").into());
    }

    let journal_path = account_journal::login_account_journal_path(ledger_dir, login_name, label);
    let mut entries = account_journal::read_journal_at_path(&journal_path)?;
    let entry = entries
        .iter_mut()
        .find(|e| e.id == entry_id)
        .ok_or_else(|| format!("entry not found: {entry_id}"))?;

    let gl_ref = format!("general.journal:{gl_txn_id}");
    let different = || -> Box<dyn std::error::Error + Send + Sync> {
        format!("entry {entry_id} is already posted to a different GL transaction").into()
    };
    match posting_index {
        None => {
            if let Some(existing) = &entry.posted {
                return if existing == &gl_ref {
                    Ok(())
                } else {
                    Err(different())
                };
            }
            entry.posted = Some(gl_ref);
        }
        Some(index) => {
            if let Some((_, existing)) = entry.posted_postings.iter().find(|(i, _)| *i == index) {
                return if existing == &gl_ref {
                    Ok(())
                } else {
                    Err(different())
                };
            }
            entry.posted_postings.push((index, gl_ref));
        }
    }
    account_journal::write_journal_at_path(&journal_path, &entries)?;
    Ok(())
}

/// Post two login-account entries as an inter-account transfer.
///
/// Uses the new `logins/{login_name}/accounts/{label}` journal paths, unlike
/// `post_transfer` which uses the legacy `accounts/{name}` paths.
#[allow(clippy::too_many_arguments)]
pub fn post_login_account_transfer(
    ledger_dir: &Path,
    login_name1: &str,
    label1: &str,
    entry_id1: &str,
    login_name2: &str,
    label2: &str,
    entry_id2: &str,
    fee_account: Option<&str>,
    lock_owner: &str,
) -> Result<String, Box<dyn std::error::Error + Send + Sync>> {
    // Optional fee-tolerant posting: a validated fee account lets non-cancelling
    // legs post as a 3-leg block (see format_transfer_gl_transaction_with_fee).
    let fee_account = fee_account.map(validate_fee_account).transpose()?;
    let _gl_lock =
        login_config::acquire_gl_lock_with_metadata(ledger_dir, lock_owner, "post-login-transfer")?;
    let _login_locks = acquire_login_locks_for_names(
        ledger_dir,
        &[login_name1.to_string(), login_name2.to_string()],
        lock_owner,
        "post-login-transfer",
    )?;
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

    // Same invariants merge_gl_transfer enforces: matching commodities, parseable
    // amounts, and an explicit fee account whenever the legs do not cancel. The
    // Pipeline modal lists unposted entries in any commodity, so without this a
    // cross-currency or non-cancelling pick would be silently misstated.
    validate_transfer_legs(
        "post",
        &entries1[idx1],
        entry_id1,
        &entries2[idx2],
        entry_id2,
        fee_account,
    )?;

    let gl_txn_id = uuid::Uuid::new_v4().to_string();
    let source1 = format!("logins/{login_name1}/accounts/{label1}");
    let source2 = format!("logins/{login_name2}/accounts/{label2}");
    let gl_text = format_transfer_gl_transaction_with_fee(
        &entries1[idx1],
        &source1,
        &entries2[idx2],
        &source2,
        &gl_txn_id,
        fee_account,
    );

    let gl_ref = format!("general.journal:{gl_txn_id}");
    entries1[idx1].posted = Some(gl_ref.clone());
    entries2[idx2].posted = Some(gl_ref);

    // GL-first (see post_login_account_entry): a crash leaves recoverable
    // orphans (each leg's ref is restorable from the GL `; source:` backref)
    // rather than lossy dangling refs.
    let journal_path = ledger_dir.join("general.journal");
    append_to_journal(&journal_path, &gl_text)?;

    if let Err(err) = account_journal::write_journal_at_path(&journal_path1, &entries1) {
        let _ = remove_gl_transaction(ledger_dir, &gl_txn_id);
        return Err(err.into());
    }
    if let Err(err) = account_journal::write_journal_at_path(&journal_path2, &entries2) {
        let _ = account_journal::write_journal_at_path(&journal_path1, &original_entries1);
        let _ = remove_gl_transaction(ledger_dir, &gl_txn_id);
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
    crate::ledger::commit_transfer_changes(
        ledger_dir,
        login_name1,
        label1,
        login_name2,
        label2,
        &commit_msg,
    )?;

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
        // Honor configured extraTransferPatterns so a custom description is not
        // penalized as a non-transfer (mirrors the matchers).
        let extra_patterns = crate::ledger::read_refreshmint_config(ledger_dir)
            .map(|c| c.extra_transfer_patterns)
            .unwrap_or_default();

        result.sort_by(|a, b| {
            let score_a =
                transfer_candidate_score(&a.2, &src_date, &src_desc, src_amount, &extra_patterns);
            let score_b =
                transfer_candidate_score(&b.2, &src_date, &src_desc, src_amount, &extra_patterns);
            score_a.cmp(&score_b)
        });
    } else {
        // Fall back to date descending when source entry not found.
        result.sort_by(|a, b| b.2.date.cmp(&a.2.date));
    }

    Ok(result)
}

/// Compute a ranking score for a transfer candidate (lower = better match).
///
/// `extra_patterns` are the configured `extraTransferPatterns` so a
/// user-configured description also counts as a probable transfer here (mirrors
/// the matchers, which already honor them via TransferSettings).
fn transfer_candidate_score(
    entry: &account_journal::AccountEntry,
    src_date: &str,
    src_desc: &str,
    src_amount: Option<f64>,
    extra_patterns: &[String],
) -> i64 {
    let mut score: i64 = 0;

    // Penalize entries not labelled as transfers.
    if !crate::transfer_detector::is_probable_transfer_with_extra(
        &entry.description,
        extra_patterns,
    ) {
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
        if (sa + ea).abs() < TRANSFER_CANCEL_EPSILON {
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

    // Account-first: this legacy `accounts/<name>` path is NOT covered by
    // consistency::recover_ledger, so GL-first would just leave an unrecoverable
    // orphan. Keep the original ordering (the login post paths use GL-first +
    // recovery; see post_login_account_entry).
    if let Err(err) = account_journal::write_journal(ledger_dir, account1, &entries1) {
        return Err(err.into());
    }
    if let Err(err) = account_journal::write_journal(ledger_dir, account2, &entries2) {
        let _ = account_journal::write_journal(ledger_dir, account1, &original_entries1);
        return Err(err.into());
    }

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

    let mut posted_mask = vec![false; entry.postings.len()];
    for (idx, _) in &entry.posted_postings {
        if *idx < posted_mask.len() {
            posted_mask[*idx] = true;
        }
    }
    posted_mask.iter().any(|is_posted| !is_posted)
}

/// Format a GL transaction produced from a source account-journal entry.
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

/// Format a GL transaction that splits one bank entry across multiple counterpart accounts.
fn format_gl_split_transaction(
    entry: &AccountEntry,
    source_locator: &str,
    counterparts: &[SplitCounterpart],
    gl_txn_id: &str,
) -> String {
    let source_tag = format!("; source: {}:{}", source_locator, entry.id);

    let first_posting = &entry.postings[0];
    let real_account = &first_posting.account;
    let amount_str = first_posting
        .amount
        .as_ref()
        .map(|a| format!("{} {}", a.quantity, a.commodity))
        .unwrap_or_default();

    let status_marker = entry.status.hledger_marker();
    let mut comment_lines = vec![
        "    ; generated-by: refreshmint-post".to_string(),
        format!("    {source_tag}"),
    ];
    for evidence_ref in collect_unique_evidence_refs([entry]) {
        comment_lines.push(format!("    ; evidence: {evidence_ref}"));
    }
    let comment_block = comment_lines.join("\n");

    let mut counterpart_lines = String::new();
    for c in counterparts {
        if let Some(amt) = &c.amount {
            counterpart_lines.push_str(&format!("    {}  {}\n", c.account, amt));
        } else {
            counterpart_lines.push_str(&format!("    {}\n", c.account));
        }
    }

    format!(
        "{}  {}{}  ; id: {}\n{comment_block}\n    {real_account}  {amount_str}\n{counterpart_lines}",
        entry.date, status_marker, entry.description, gl_txn_id,
    )
}

/// Format a GL transaction for a transfer between two accounts (no fee leg).
/// Delegates to [`format_transfer_gl_transaction_with_fee`] with no fee account;
/// output is byte-identical to the historical two-posting format.
fn format_transfer_gl_transaction(
    entry1: &AccountEntry,
    source1: &str,
    entry2: &AccountEntry,
    source2: &str,
    gl_txn_id: &str,
) -> String {
    format_transfer_gl_transaction_with_fee(entry1, source1, entry2, source2, gl_txn_id, None)
}

/// Number of decimal places in a decimal quantity string (e.g. "-100.50" → 2).
fn decimal_places(quantity: &str) -> usize {
    quantity
        .trim()
        .rsplit_once('.')
        .map(|(_, frac)| frac.len())
        .unwrap_or(0)
}

/// The fee residual `-(a1 + a2)` of a transfer's two leg amounts, formatted at
/// the legs' decimal precision — or `None` when the legs cancel (|a1+a2| <
/// `TRANSFER_CANCEL_EPSILON`) or either quantity does not parse. Because the
/// residual is the exact decimal negation of the legs' sum, a 3-posting block
/// `a1 + a2 + r` balances by construction (nothing validates it with hledger).
fn transfer_fee_residual(quantity1: &str, quantity2: &str) -> Option<String> {
    let a1: f64 = quantity1.trim().parse().ok()?;
    let a2: f64 = quantity2.trim().parse().ok()?;
    let residual = -(a1 + a2);
    if residual.abs() < TRANSFER_CANCEL_EPSILON {
        return None;
    }
    let precision = decimal_places(quantity1).max(decimal_places(quantity2));
    Some(format!("{residual:.precision$}"))
}

/// Two leg amounts cancel below this epsilon (cents tolerance). Shared with the
/// merge cancel guard in `merge_gl_transfer` and the transfer matchers in
/// `categorize` (find_transfer_matches / find_gl_transfer_matches); the frontend
/// mirrors it in gl-transfer-utils.ts.
pub(crate) const TRANSFER_CANCEL_EPSILON: f64 = 0.005;

/// Format a GL transaction for a transfer between two accounts.
///
/// With `fee_account: None` (or legs that cancel) this writes the historical
/// two-posting shape: only entry1's amount explicit, leg 2 elided (hledger
/// infers). With `fee_account: Some` and a non-cancelling residual it writes
/// THREE postings with ALL amounts explicit — `real1 a1 C`, `real2 a2 C`,
/// `fee r C` where `r = -(a1+a2)` at the legs' decimal precision — so the block
/// balances by construction. `sync_gl_transaction` re-derives the fee account
/// from the third posting of an existing block; keep the shapes in sync.
fn format_transfer_gl_transaction_with_fee(
    entry1: &AccountEntry,
    source1: &str,
    entry2: &AccountEntry,
    source2: &str,
    gl_txn_id: &str,
    fee_account: Option<&str>,
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

    let simple1 = entry1.postings.first().and_then(|p| p.amount.as_ref());
    let simple2 = entry2.postings.first().and_then(|p| p.amount.as_ref());

    let amount1 = simple1
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

    // Fee leg: only when requested AND the legs do not cancel (same commodity is
    // enforced by the callers' guards).
    let fee_leg = fee_account.and_then(|fee| {
        let (a1, a2) = (simple1?, simple2?);
        let residual = transfer_fee_residual(&a1.quantity, &a2.quantity)?;
        Some((
            fee.to_string(),
            format!("{residual} {}", a1.commodity),
            format!("{} {}", a2.quantity, a2.commodity),
        ))
    });

    match fee_leg {
        Some((fee, fee_amount, amount2)) => format!(
            "{}  {}{}  ; id: {}\n{comment_block}\n    {real_account1}  {amount1}\n    {real_account2}  {amount2}\n    {fee}  {fee_amount}\n",
            entry1.date,
            status_marker,
            entry1.description,
            gl_txn_id,
        ),
        None => format!(
            "{}  {}{}  ; id: {}\n{comment_block}\n    {real_account1}  {amount1}\n    {real_account2}\n",
            entry1.date,
            status_marker,
            entry1.description,
            gl_txn_id,
        ),
    }
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
    // Crash-safe append: read the current contents, append the new block, and
    // rewrite atomically. A plain O_APPEND write could leave a partial trailing
    // block if the process is killed mid-write. The output is byte-for-byte the
    // same as the previous append (one '\n' separator before the new block when
    // the file is non-empty). See crate::fs_atomic.
    let mut content = match fs::read(journal_path) {
        Ok(bytes) => bytes,
        Err(err) if err.kind() == io::ErrorKind::NotFound => Vec::new(),
        Err(err) => return Err(err),
    };
    if !content.is_empty() {
        content.push(b'\n');
    }
    content.extend_from_slice(text.as_bytes());
    crate::fs_atomic::write_atomic(journal_path, &content)
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

fn source_login_names_from_sources(sources: &[(String, String)]) -> Vec<String> {
    let mut names = std::collections::BTreeSet::new();
    for (locator, _) in sources {
        if let Some((login_name, _)) = locator_to_login_label(locator) {
            names.insert(login_name.to_string());
        }
    }
    names.into_iter().collect()
}

fn source_login_names_from_block(block: &str) -> Vec<String> {
    source_login_names_from_sources(&parse_sources_from_block(block))
}

fn acquire_login_locks_for_names(
    ledger_dir: &Path,
    login_names: &[String],
    owner: &str,
    purpose: &str,
) -> Result<Vec<login_config::LoginLock>, Box<dyn std::error::Error + Send + Sync>> {
    let mut sorted = std::collections::BTreeSet::new();
    for login_name in login_names {
        if !login_name.is_empty() {
            sorted.insert(login_name.clone());
        }
    }

    let mut locks = Vec::new();
    for login_name in sorted {
        locks.push(login_config::acquire_login_lock_with_metadata(
            ledger_dir,
            &login_name,
            owner,
            purpose,
        )?);
    }
    Ok(locks)
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

    for block in crate::gl_journal::split_journal_blocks(&content) {
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
    crate::fs_atomic::write_atomic(&journal_path, final_content.as_bytes())?;
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
    let blocks: Vec<String> = crate::gl_journal::split_journal_blocks(&content)
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
    crate::fs_atomic::write_atomic(&journal_path, final_content.as_bytes())
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

/// Extract the fee account from a 3-posting fee-transfer block: the third
/// posting line's account (the fee leg is always written last by
/// `format_transfer_gl_transaction_with_fee`; keep in sync). Accounts may
/// contain single spaces, so split the trailing amount off at the two-space
/// separator. `None` when the block does not have exactly 3 posting lines.
fn extract_fee_account_from_transfer_block(block: &str) -> Option<String> {
    let postings: Vec<&str> = block
        .lines()
        .filter(|line| {
            let is_indented = line.starts_with(' ') || line.starts_with('\t');
            let trimmed = line.trim();
            is_indented && !trimmed.is_empty() && !trimmed.starts_with(';')
        })
        .collect();
    let [_, _, fee_line] = postings.as_slice() else {
        return None;
    };
    let trimmed = fee_line.trim();
    let account = trimmed
        .split_once("  ")
        .map_or(trimmed, |(account, _)| account);
    Some(account.trim().to_string())
}

/// Count posting lines (indented, non-empty, non-comment) in a GL block. A
/// simple posting has two (real account + counterpart); a split has more.
fn count_posting_lines(block: &str) -> usize {
    block
        .lines()
        .filter(|line| {
            let is_indented = line.starts_with(' ') || line.starts_with('\t');
            let trimmed = line.trim();
            is_indented && !trimmed.is_empty() && !trimmed.starts_with(';')
        })
        .count()
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
    lock_owner: &str,
) -> Result<String, Box<dyn std::error::Error + Send + Sync>> {
    let _gl_lock =
        login_config::acquire_gl_lock_with_metadata(ledger_dir, lock_owner, "sync-gl-transaction")?;
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
    let source_logins = source_login_names_from_block(&gl_block);
    let _login_locks = acquire_login_locks_for_names(
        ledger_dir,
        &source_logins,
        lock_owner,
        "sync-gl-transaction",
    )?;

    // A reconciled/linked/soft-closed GL transaction must not be resynced: sync
    // rewrites the amount/status of the GL block, which would invalidate a
    // finalized reconciliation. Mirror unpost's guard (:632-639).
    let blockers = crate::bookkeeping::gl_txn_removal_blockers(ledger_dir, &gl_txn_id)?;
    if !blockers.is_empty() {
        return Err(format!(
            "cannot sync entry {entry_id}; GL transaction {gl_txn_id} is protected: {}",
            blockers.join(", ")
        )
        .into());
    }

    // 3. Parse sources and load their current entries (fail fast before any writes).
    let raw_sources = parse_sources_from_block(&gl_block);
    let loaded = load_source_entries(ledger_dir, &raw_sources)?;

    // 4. Rebuild the GL block.
    let new_block = match loaded.as_slice() {
        [(loc1, _, e1), (loc2, _, e2)] => {
            // Transfer: two sources. A 2-posting block is fee-less; a 3-posting
            // block carries a single fee leg. More than 3 means a manual extra
            // leg that format_transfer_gl_transaction_with_fee cannot reproduce
            // (it only recognizes one trailing fee leg), so rewriting would
            // silently discard it. Refuse, mirroring the 1-source split refusal
            // below.
            if count_posting_lines(&gl_block) > 3 {
                return Err(format!(
                    "GL transaction {gl_txn_id} has more than 3 postings; sync would discard the extra leg. Unpost and re-post it instead."
                )
                .into());
            }
            // A 3-posting block carries a fee leg
            // (format_transfer_gl_transaction_with_fee); carry its account over
            // and let the formatter recompute the residual from the CURRENT
            // entry amounts (it drops the fee leg if the legs now cancel).
            // Without this the reformat would silently discard the fee leg.
            let fee_account = extract_fee_account_from_transfer_block(&gl_block);
            format_transfer_gl_transaction_with_fee(
                e1,
                loc1,
                e2,
                loc2,
                &gl_txn_id,
                fee_account.as_deref(),
            )
        }
        [(loc, _, e)] => {
            // A split posting (one source, multiple counterpart legs) would be
            // collapsed to a single counterpart by format_gl_transaction below,
            // silently destroying the split and rebalancing the money. Refuse
            // rather than clobber.
            if count_posting_lines(&gl_block) > 2 {
                return Err(format!(
                    "GL transaction {gl_txn_id} is a split posting; sync would collapse it. Unpost and re-post it instead."
                )
                .into());
            }
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

    crate::ledger::commit_general_journal(ledger_dir, &format!("sync: {gl_txn_id}"))?;

    Ok(gl_txn_id)
}

pub fn retire_login_account_entry(
    ledger_dir: &Path,
    login_name: &str,
    label: &str,
    entry_id: &str,
    reason: &str,
    lock_owner: &str,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let _gl_lock =
        login_config::acquire_gl_lock_with_metadata(ledger_dir, lock_owner, "retire-login-entry")?;
    let _login_lock = login_config::acquire_login_lock_with_metadata(
        ledger_dir,
        login_name,
        lock_owner,
        "retire-login-entry",
    )?;

    let journal_path = account_journal::login_account_journal_path(ledger_dir, login_name, label);
    let mut entries = account_journal::read_journal_at_path(&journal_path)?;
    let original_entries = entries.clone();
    let entry_idx = entries
        .iter()
        .position(|e| e.id == entry_id)
        .ok_or_else(|| format!("entry not found: {entry_id}"))?;
    let entry = entries[entry_idx].clone();

    if !entry.posted_postings.is_empty() {
        return Err(format!("entry {entry_id} has partially posted split rows").into());
    }

    let mut removed_gl_txn = None;
    let gl_txn_id = entry.posted.as_deref().map(|gl_ref| {
        gl_ref
            .strip_prefix("general.journal:")
            .unwrap_or(gl_ref)
            .to_string()
    });
    if let Some(gl_txn_id) = gl_txn_id.as_deref() {
        let blockers = crate::bookkeeping::gl_txn_removal_blockers(ledger_dir, gl_txn_id)?;
        if !blockers.is_empty() {
            return Err(format!(
                "cannot retire entry {entry_id}; GL transaction {gl_txn_id} is protected: {}",
                blockers.join(", ")
            )
            .into());
        }
        let source_locator = format!("logins/{login_name}/accounts/{label}");
        let other_sides = preload_other_sides(ledger_dir, gl_txn_id, &source_locator, entry_id)?;
        if !other_sides.is_empty() {
            return Err(format!(
                "cannot retire entry {entry_id}; GL transaction {gl_txn_id} has other source entries"
            )
            .into());
        }
        removed_gl_txn = remove_gl_transaction(ledger_dir, gl_txn_id)?;
    }

    entries.remove(entry_idx);
    if let Err(err) = account_journal::write_journal_at_path(&journal_path, &entries) {
        if let Some(removed) = &removed_gl_txn {
            let gl_journal_path = ledger_dir.join("general.journal");
            let _ = append_to_journal(&gl_journal_path, removed);
        }
        return Err(err.into());
    }

    let op = operations::AccountOperation::EntryRetired {
        entry_id: entry_id.to_string(),
        reason: reason.to_string(),
        timestamp: operations::now_timestamp(),
    };
    if let Err(err) = operations::append_login_account_operation(ledger_dir, login_name, label, &op)
    {
        let _ = account_journal::write_journal_at_path(&journal_path, &original_entries);
        if let Some(removed) = removed_gl_txn {
            let gl_journal_path = ledger_dir.join("general.journal");
            let _ = append_to_journal(&gl_journal_path, &removed);
        }
        return Err(err.into());
    }

    crate::ledger::commit_post_changes(
        ledger_dir,
        login_name,
        label,
        &format!("retire: {entry_id}"),
    )?;

    Ok(())
}

/// Byte offsets `(indent_end, account_end)` of a GL posting line's account name:
/// the account is `line[indent_end..account_end]`. `account_end` is the start of
/// the amount separator (a tab, two spaces, or " ;"), or the line end.
fn posting_account_span(line: &str) -> (usize, usize) {
    let indent_end = line
        .char_indices()
        .find(|(_, ch)| !ch.is_whitespace())
        .map(|(idx, _)| idx)
        .unwrap_or(line.len());
    let rest = &line[indent_end..];

    let mut suffix_start = rest.len();
    let mut prev_was_space = false;
    for (idx, ch) in rest.char_indices() {
        if ch == '\t' {
            suffix_start = idx;
            break;
        }
        if ch == ' ' {
            if prev_was_space {
                suffix_start = idx - 1;
                break;
            }
            prev_was_space = true;
            continue;
        }
        if ch == ';' && idx > 0 && rest[..idx].ends_with(' ') {
            suffix_start = idx - 1;
            break;
        }
        prev_was_space = false;
    }

    (indent_end, indent_end + suffix_start)
}

/// The account name of a GL posting line (trimmed).
fn posting_line_account(line: &str) -> &str {
    let (indent_end, account_end) = posting_account_span(line);
    line[indent_end..account_end].trim()
}

fn replace_posting_account(line: &str, new_account: &str) -> String {
    let (indent_end, account_end) = posting_account_span(line);
    format!(
        "{}{}{}",
        &line[..indent_end],
        new_account,
        &line[account_end..]
    )
}

/// Replace the posting at `posting_index` with `new_account` in an existing GL transaction.
///
/// Finds the block by `txn_id`, rewrites only the indexed posting account while
/// preserving the rest of the posting line, writes the updated file, and commits.
/// Index of the single `Expenses:Unknown` posting of a GL transaction, used by
/// the RecategorizeGl automation proposal to target the leg it must rewrite.
///
/// `suggest_gl_categories` (categorize.rs) lists ANY txn containing an
/// `Expenses:Unknown` posting, including *manual* txns where Unknown is not the
/// last posting. Resolving the actual Unknown posting here (rather than blindly
/// assuming the last posting) keeps the rewrite targeted at the Unknown leg. It
/// is also called at apply time, so it re-checks that the leg is still Unknown,
/// closing the staleness window where the policy could clobber a category the
/// user just set manually.
///
/// Returns `Ok(None)` if no transaction with `txn_id` exists. Returns an error
/// if the transaction has zero or more than one `Expenses:Unknown` posting
/// (nothing safe / no unambiguous target to rewrite).
pub fn gl_txn_unknown_posting_index(
    ledger_dir: &Path,
    txn_id: &str,
) -> Result<Option<usize>, Box<dyn std::error::Error + Send + Sync>> {
    let gl_journal_path = ledger_dir.join("general.journal");
    if !gl_journal_path.exists() {
        return Ok(None);
    }
    // Propagate parse failures rather than swallowing them: `unwrap_or_default()`
    // would turn a broken journal into an empty txn list and a misleading
    // "transaction not found" at the call site.
    let txns = crate::ledger_open::run_hledger_print(&gl_journal_path)?;
    let Some(txn) = txns
        .iter()
        .find(|txn| txn.ttags.iter().any(|(k, v)| k == "id" && v == txn_id))
    else {
        return Ok(None);
    };
    let unknown_indices: Vec<usize> = txn
        .tpostings
        .iter()
        .enumerate()
        .filter(|(_, posting)| posting.paccount == "Expenses:Unknown")
        .map(|(index, _)| index)
        .collect();
    match unknown_indices.as_slice() {
        [index] => Ok(Some(*index)),
        [] => Err(format!(
            "recategorize-gl: transaction {txn_id} has no Expenses:Unknown posting to recategorize"
        )
        .into()),
        _ => Err(format!(
            "recategorize-gl: transaction {txn_id} has {} Expenses:Unknown postings; \
             ambiguous recategorization target",
            unknown_indices.len()
        )
        .into()),
    }
}

pub fn recategorize_gl_transaction(
    ledger_dir: &Path,
    txn_id: &str,
    posting_index: usize,
    new_account: &str,
    lock_owner: &str,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    recategorize_gl_transactions(
        ledger_dir,
        &[(txn_id.to_string(), posting_index, new_account.to_string())],
        lock_owner,
    )
}

/// Recategorize one or more GL postings in a single read/write/commit.
///
/// `edits` is a list of `(txn_id, posting_index, new_account)`. Bulk
/// recategorization used to loop the single-edit command, rewriting the whole
/// `general.journal` and making one git commit per row (O(rows × ledger)). This
/// applies every edit in one pass and commits once.
pub fn recategorize_gl_transactions(
    ledger_dir: &Path,
    edits: &[(String, usize, String)],
    lock_owner: &str,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    if edits.is_empty() {
        return Ok(());
    }
    let _gl_lock =
        login_config::acquire_gl_lock_with_metadata(ledger_dir, lock_owner, "recategorize-gl")?;
    let journal_path = ledger_dir.join("general.journal");
    let content = fs::read_to_string(&journal_path)?;
    let final_content = apply_recategorizations(&content, edits)?;
    crate::fs_atomic::write_atomic(&journal_path, final_content.as_bytes())?;

    let commit_msg = if edits.len() == 1 {
        format!("recategorize: {} → {}", edits[0].0, edits[0].2)
    } else {
        format!("recategorize {} GL postings", edits.len())
    };
    crate::ledger::commit_general_journal(ledger_dir, &commit_msg)?;

    Ok(())
}

/// Pure core shared by the single- and batch-recategorize commands: rewrite the
/// GL journal text, applying each `(txn_id, posting_index, new_account)` edit to
/// the first block carrying that id. Returns an error naming the first edit
/// whose transaction or posting index could not be resolved (and leaves the
/// caller to skip writing).
fn apply_recategorizations(
    content: &str,
    edits: &[(String, usize, String)],
) -> Result<String, String> {
    use std::collections::{HashMap, HashSet};

    let mut by_txn: HashMap<&str, Vec<(usize, &str)>> = HashMap::new();
    for (txn_id, posting_index, new_account) in edits {
        by_txn
            .entry(txn_id.as_str())
            .or_default()
            .push((*posting_index, new_account.as_str()));
    }

    let mut consumed: HashSet<&str> = HashSet::new();
    let mut replaced: HashMap<&str, HashSet<usize>> = HashMap::new();
    // Recategorize must only ever rewrite the counterpart (income/expense) leg,
    // never the bank/balance-sheet leg — rewriting the real account would move
    // money off the reconciled account. Mirrors the frontend rule: `isBalanceSheet`
    // in src/tabs/TransactionsTab.tsx and the `isNonBalanceSheet` guard in
    // src/tabs/TransactionsTable.tsx. (Counterpart edits on reconciled txns stay
    // legal, so this is a leg guard only, not a blocker check.)
    let mut guard_error: Option<String> = None;

    let blocks: Vec<String> = crate::gl_journal::split_journal_blocks(content)
        .into_iter()
        .map(|block| {
            let matched = by_txn.keys().copied().find(|txn_id| {
                !consumed.contains(txn_id) && block.contains(&format!("id: {txn_id}"))
            });
            let Some(txn_id) = matched else {
                return block;
            };
            consumed.insert(txn_id);
            let edits_for = &by_txn[txn_id];
            let replaced_for = replaced.entry(txn_id).or_default();
            let mut current_posting_index = 0usize;
            block
                .lines()
                .map(|line| {
                    let is_indented = line.starts_with(' ') || line.starts_with('\t');
                    let trimmed = line.trim();
                    let is_posting_line =
                        is_indented && !trimmed.is_empty() && !trimmed.starts_with(';');
                    if !is_posting_line {
                        return line.to_string();
                    }
                    let idx = current_posting_index;
                    current_posting_index += 1;
                    if let Some((_, new_account)) = edits_for.iter().find(|(i, _)| *i == idx) {
                        let current_account = posting_line_account(line);
                        if current_account.starts_with("Assets:")
                            || current_account.starts_with("Liabilities:")
                        {
                            guard_error.get_or_insert_with(|| {
                                format!(
                                    "cannot recategorize balance-sheet posting {idx} ({current_account}) of {txn_id}"
                                )
                            });
                            return line.to_string();
                        }
                        replaced_for.insert(idx);
                        replace_posting_account(line, new_account)
                    } else {
                        line.to_string()
                    }
                })
                .collect::<Vec<_>>()
                .join("\n")
                .trim_end()
                .to_string()
        })
        .collect();

    // Reject before the index-validation loop: a refused balance-sheet edit
    // leaves its posting unreplaced, which would otherwise surface as a spurious
    // "out of bounds" error.
    if let Some(err) = guard_error {
        return Err(err);
    }

    for (txn_id, posting_index, _) in edits {
        if !consumed.contains(txn_id.as_str()) {
            return Err(format!("GL transaction not found: {txn_id}"));
        }
        if !replaced
            .get(txn_id.as_str())
            .is_some_and(|set| set.contains(posting_index))
        {
            return Err(format!("GL posting index out of bounds: {posting_index}"));
        }
    }

    let mut final_content = blocks.join("\n\n");
    if !final_content.is_empty() {
        final_content.push('\n');
    }
    Ok(final_content)
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
/// Validate a user-supplied fee account for a fee-tolerant transfer: non-empty
/// and non-balance-sheet (mirrors the frontend `isNonBalanceSheet` rule in
/// TransactionsTable.tsx — a fee posted to Assets:/Liabilities: would misstate a
/// real account). Returns the trimmed account.
fn validate_fee_account(fee_account: &str) -> Result<&str, String> {
    let fee = fee_account.trim();
    if fee.is_empty() {
        return Err("fee account must not be empty".to_string());
    }
    if fee.starts_with("Assets:") || fee.starts_with("Liabilities:") {
        return Err(format!(
            "fee account {fee} is a balance-sheet account; use an expense/income account"
        ));
    }
    Ok(fee)
}

/// Validate that two transfer legs are safe to encode with
/// [`format_transfer_gl_transaction_with_fee`]: both must carry a parseable
/// amount, the commodities must match, and non-cancelling legs
/// (|a1+a2| ≥ [`TRANSFER_CANCEL_EPSILON`]) require an explicit fee account
/// (cancelling legs ignore any fee). Shared by [`merge_gl_transfer`] and
/// [`post_login_account_transfer`] so both encode identical invariants — the
/// fee-less format stores only leg 1's amount+commodity and forces leg 2 to its
/// exact negation, silently misstating a mismatched pair otherwise. `verb`
/// selects the wording ("merge"/"post") in the error messages.
fn validate_transfer_legs(
    verb: &str,
    entry1: &account_journal::AccountEntry,
    entry_id1: &str,
    entry2: &account_journal::AccountEntry,
    entry_id2: &str,
    fee_account: Option<&str>,
) -> Result<(), String> {
    let simple1 = entry1
        .postings
        .first()
        .and_then(|p| p.amount.as_ref())
        .ok_or_else(|| format!("cannot {verb}; entry {entry_id1} has no amount"))?;
    let simple2 = entry2
        .postings
        .first()
        .and_then(|p| p.amount.as_ref())
        .ok_or_else(|| format!("cannot {verb}; entry {entry_id2} has no amount"))?;
    if simple1.commodity != simple2.commodity {
        return Err(format!(
            "cannot {verb}; commodities differ ({} vs {})",
            simple1.commodity, simple2.commodity
        ));
    }
    let a: f64 = simple1
        .quantity
        .parse()
        .map_err(|_| format!("cannot {verb}; entry {entry_id1} has no amount"))?;
    let b: f64 = simple2
        .quantity
        .parse()
        .map_err(|_| format!("cannot {verb}; entry {entry_id2} has no amount"))?;
    // Non-cancelling legs are allowed ONLY with an explicit fee account (the
    // residual becomes a third fee posting; see
    // format_transfer_gl_transaction_with_fee). Cancelling legs ignore any fee.
    if (a + b).abs() >= TRANSFER_CANCEL_EPSILON && fee_account.is_none() {
        let residual = transfer_fee_residual(&simple1.quantity, &simple2.quantity)
            .unwrap_or_else(|| format!("{}", -(a + b)));
        return Err(format!(
            "amounts do not cancel ({a} + {b}); residual {residual} {} — supply a fee account to {verb} as a fee transfer",
            simple1.commodity
        ));
    }
    Ok(())
}

pub fn merge_gl_transfer(
    ledger_dir: &Path,
    txn_id_1: &str,
    txn_id_2: &str,
    fee_account: Option<&str>,
    lock_owner: &str,
) -> Result<String, Box<dyn std::error::Error + Send + Sync>> {
    if txn_id_1 == txn_id_2 {
        return Err("cannot merge a transaction with itself".into());
    }
    let fee_account = fee_account.map(validate_fee_account).transpose()?;
    let _gl_lock =
        login_config::acquire_gl_lock_with_metadata(ledger_dir, lock_owner, "merge-gl-transfer")?;

    // 1. Find both GL blocks.
    let block1 = find_gl_block(ledger_dir, txn_id_1)?
        .ok_or_else(|| format!("GL transaction not found: {txn_id_1}"))?;
    let block2 = find_gl_block(ledger_dir, txn_id_2)?
        .ok_or_else(|| format!("GL transaction not found: {txn_id_2}"))?;

    // Guard: refuse to merge a block that already has more than one source
    // (i.e. is itself a transfer). The merge below keeps only the first source
    // of each block (:1999-2008), so merging a transfer would silently drop the
    // other leg's `posted:` ref and leave it dangling. Reject instead.
    for (txn_id, block) in [(txn_id_1, &block1), (txn_id_2, &block2)] {
        let source_count = parse_sources_from_block(block).len();
        if source_count != 1 {
            return Err(format!(
                "GL transaction {txn_id} has {source_count} sources (already a transfer); cannot merge"
            )
            .into());
        }
    }

    // Guard: refuse to merge a split posting. format_transfer_gl_transaction
    // rebuilds each side as a single real-account + counterpart pair, so merging
    // a split (>2 posting lines) would collapse and rebalance it. Mirror sync's
    // split refusal (:1645-1649).
    for (txn_id, block) in [(txn_id_1, &block1), (txn_id_2, &block2)] {
        if count_posting_lines(block) > 2 {
            return Err(format!(
                "GL transaction {txn_id} is a split posting; merge would collapse it. Unpost and re-post it instead."
            )
            .into());
        }
    }

    // Guard: a reconciled/linked/soft-closed GL transaction must not be removed
    // by a merge (it rewrites both blocks into one). Mirror unpost's guard
    // (:632-639).
    for txn_id in [txn_id_1, txn_id_2] {
        let blockers = crate::bookkeeping::gl_txn_removal_blockers(ledger_dir, txn_id)?;
        if !blockers.is_empty() {
            return Err(format!(
                "cannot merge; GL transaction {txn_id} is protected: {}",
                blockers.join(", ")
            )
            .into());
        }
    }

    let source_logins = source_login_names_from_sources(&[
        parse_sources_from_block(&block1)
            .into_iter()
            .next()
            .ok_or("GL transaction 1 has no source tag")?,
        parse_sources_from_block(&block2)
            .into_iter()
            .next()
            .ok_or("GL transaction 2 has no source tag")?,
    ]);
    let _login_locks =
        acquire_login_locks_for_names(ledger_dir, &source_logins, lock_owner, "merge-gl-transfer")?;

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

    validate_transfer_legs(
        "merge",
        &entries1[idx1],
        &entry_id1,
        &entries2[idx2],
        &entry_id2,
        fee_account,
    )?;

    // 5. Build merged transfer GL text using the two account entries.
    let gl_text = format_transfer_gl_transaction_with_fee(
        &entries1[idx1],
        &locator1,
        &entries2[idx2],
        &locator2,
        &new_uuid,
        fee_account,
    );

    // 6. Compute new GL content: remove both old blocks, append merged.
    let gl_journal_path = ledger_dir.join("general.journal");
    let original_gl_content = fs::read_to_string(&gl_journal_path)?;
    let marker1 = format!("id: {txn_id_1}");
    let marker2 = format!("id: {txn_id_2}");
    let kept_blocks: Vec<String> = crate::gl_journal::split_journal_blocks(&original_gl_content)
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
    if let Err(err) = crate::fs_atomic::write_atomic(&gl_journal_path, new_gl_content.as_bytes()) {
        let _ = account_journal::write_journal_at_path(&path1, &original_entries1);
        if !same_file {
            let _ = account_journal::write_journal_at_path(&path2, &original_entries2);
        }
        return Err(err.into());
    }
    if let Err(err) = crate::bookkeeping::repair_gl_txn_refs_after_merge(
        ledger_dir,
        &[txn_id_1, txn_id_2],
        &new_uuid,
    ) {
        let _ = crate::fs_atomic::write_atomic(&gl_journal_path, original_gl_content.as_bytes());
        let _ = account_journal::write_journal_at_path(&path1, &original_entries1);
        if !same_file {
            let _ = account_journal::write_journal_at_path(&path2, &original_entries2);
        }
        return Err(err.into());
    }

    // 9. Commit all changed files.
    let commit_msg = format!("merge transfer: {txn_id_1} + {txn_id_2} → {new_uuid}");
    match (
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
    }?;

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
            "refreshmint-rec-{prefix}-{}-{now}.refreshmint",
            std::process::id()
        ));
        crate::ledger::new_ledger_at_dir(&dir).unwrap();
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
                    account: "Equity:Staging:Checking".to_string(),
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
    fn repair_dangling_ref_clears_account_side_when_gl_txn_absent() {
        let root = temp_dir("repair-dangling");
        fs::write(root.join("general.journal"), "").unwrap();
        let mut entry = make_entry("entry-1", "2024-01-15", "Shell", "-21.32");
        entry.posted = Some("general.journal:ghost".to_string());
        let path = account_journal::login_account_journal_path(&root, "chase", "checking");
        account_journal::write_journal_at_path(&path, std::slice::from_ref(&entry)).unwrap();

        repair_dangling_ref(&root, "chase", "checking", "entry-1", None, "test").unwrap();

        let updated = account_journal::read_journal_at_path(&path).unwrap();
        assert!(
            updated[0].posted.is_none(),
            "dangling ref should be cleared"
        );
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn repair_dangling_ref_refuses_when_gl_txn_exists() {
        let root = temp_dir("repair-dangling-guard");
        fs::write(
            root.join("general.journal"),
            "2026-01-01 X  ; id: real\n    Assets:A  1 USD\n    Income:B\n",
        )
        .unwrap();
        let mut entry = make_entry("entry-1", "2024-01-15", "Shell", "-21.32");
        entry.posted = Some("general.journal:real".to_string());
        let path = account_journal::login_account_journal_path(&root, "chase", "checking");
        account_journal::write_journal_at_path(&path, std::slice::from_ref(&entry)).unwrap();

        let result = repair_dangling_ref(&root, "chase", "checking", "entry-1", None, "test");
        assert!(
            result.is_err(),
            "must refuse to clear a live (non-dangling) ref"
        );
        let updated = account_journal::read_journal_at_path(&path).unwrap();
        assert_eq!(updated[0].posted.as_deref(), Some("general.journal:real"));
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn repair_orphaned_gl_txn_removes_block() {
        let root = temp_dir("repair-orphan");
        crate::bookkeeping::ensure_bookkeeping_layout(&root).unwrap();
        fs::write(
            root.join("general.journal"),
            "2026-01-01 Coffee  ; id: orphan-1\n    ; source: logins/chase/accounts/checking:entry-1\n    Expenses:Unknown  5 USD\n    Assets:Chase\n",
        )
        .unwrap();
        // The source entry exists but does not reference orphan-1 (unposted).
        let entry = make_entry("entry-1", "2026-01-01", "Coffee", "-5");
        let path = account_journal::login_account_journal_path(&root, "chase", "checking");
        account_journal::write_journal_at_path(&path, std::slice::from_ref(&entry)).unwrap();

        repair_orphaned_gl_txn(&root, "orphan-1", "test").unwrap();

        let gl = fs::read_to_string(root.join("general.journal")).unwrap();
        assert!(
            !gl.contains("orphan-1"),
            "orphaned GL txn should be removed"
        );
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn recategorize_updates_only_selected_posting_index() {
        let root = temp_dir("recategorize-posting-index");
        fs::write(
            root.join("general.journal"),
            "2024-01-15 Grocery run  ; id: txn-1\n    Assets:Checking  -10.00 USD\n    Expenses:Food\n    Expenses:Food\n",
        )
        .unwrap();

        recategorize_gl_transaction(&root, "txn-1", 2, "Expenses:Dining", "test").unwrap();

        let gl_content = fs::read_to_string(root.join("general.journal")).unwrap();
        assert!(
            gl_content.contains("    Expenses:Food\n    Expenses:Dining\n"),
            "only the indexed posting should change"
        );
        assert_eq!(
            gl_content.matches("Expenses:Food").count(),
            1,
            "one duplicate posting should remain unchanged"
        );

        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn recategorize_preserves_amounts_and_comments_on_selected_posting() {
        let root = temp_dir("recategorize-preserves-posting-tail");
        fs::write(
            root.join("general.journal"),
            "2024-01-15 Grocery run  ; id: txn-1\n    Assets:Checking  -10.00 USD\n    Expenses:Food  7.00 USD ; note:snack\n    Expenses:Food  3.00 USD\n",
        )
        .unwrap();

        recategorize_gl_transaction(&root, "txn-1", 1, "Expenses:Dining", "test").unwrap();

        let gl_content = fs::read_to_string(root.join("general.journal")).unwrap();
        assert!(
            gl_content.contains("    Expenses:Dining  7.00 USD ; note:snack\n"),
            "the selected posting should keep its amount and comment"
        );
        assert!(
            gl_content.contains("    Expenses:Food  3.00 USD\n"),
            "other postings should remain unchanged"
        );

        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn recategorize_batch_applies_all_edits_in_one_pass() {
        let root = temp_dir("recategorize-batch");
        fs::write(
            root.join("general.journal"),
            "2024-01-15 A  ; id: txn-1\n    Assets:Checking  -10.00 USD\n    Expenses:Unknown\n\n2024-01-16 B  ; id: txn-2\n    Assets:Checking  -20.00 USD\n    Expenses:Unknown\n",
        )
        .unwrap();

        recategorize_gl_transactions(
            &root,
            &[
                ("txn-1".to_string(), 1, "Expenses:Food".to_string()),
                ("txn-2".to_string(), 1, "Expenses:Gas".to_string()),
            ],
            "test",
        )
        .unwrap();

        let gl = fs::read_to_string(root.join("general.journal")).unwrap();
        assert!(gl.contains("    Expenses:Food\n"), "first edit applied");
        assert!(gl.contains("    Expenses:Gas\n"), "second edit applied");
        assert_eq!(
            gl.matches("Expenses:Unknown").count(),
            0,
            "both Unknown counterparts recategorized in one pass"
        );

        // A batch naming a missing txn errors instead of partially applying.
        let err = recategorize_gl_transactions(
            &root,
            &[("txn-missing".to_string(), 1, "Expenses:Food".to_string())],
            "test",
        )
        .unwrap_err();
        assert!(err.to_string().contains("not found"));

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
        let journal_path = account_journal::login_account_journal_path(&root, "chase", "checking");
        account_journal::write_journal_at_path(&journal_path, &entries).unwrap();

        let err = post_login_account_entry(
            &root,
            "chase",
            "checking",
            "txn-1",
            "Expenses:Gas",
            Some(99),
            "test",
        )
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
        let journal_path = account_journal::login_account_journal_path(&root, "chase", "checking");
        account_journal::write_journal_at_path(&journal_path, &[entry]).unwrap();

        let err = post_login_account_entry(
            &root,
            "chase",
            "checking",
            "txn-1",
            "Expenses:Gas",
            None,
            "test",
        )
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
    fn format_transfer_with_fee_emits_balanced_three_leg_block() {
        // Fee-tolerant representation: all three amounts explicit, residual
        // computed at the legs' decimal precision, block balances by construction.
        let mut e1 = make_entry("txn-1", "2024-01-15", "Transfer", "-100.00");
        let mut e2 = make_entry("txn-2", "2024-01-15", "Transfer", "99.75");
        e2.postings[0].account = "Assets:Savings".to_string();
        e1.evidence = vec!["doc-a.csv:1:1".to_string()];
        e2.evidence = vec!["doc-b.csv:2:1".to_string()];
        let text = format_transfer_gl_transaction_with_fee(
            &e1,
            "accounts/chase",
            &e2,
            "accounts/boa",
            "gl-id",
            Some("Expenses:Bank Fees"),
        );
        assert_eq!(
            text,
            "2024-01-15  * Transfer  ; id: gl-id\n\
             \x20   ; generated-by: refreshmint-post\n\
             \x20   ; source: accounts/chase:txn-1\n\
             \x20   ; source: accounts/boa:txn-2\n\
             \x20   ; evidence: doc-a.csv:1:1\n\
             \x20   ; evidence: doc-b.csv:2:1\n\
             \x20   Assets:Checking  -100.00 USD\n\
             \x20   Assets:Savings  99.75 USD\n\
             \x20   Expenses:Bank Fees  0.25 USD\n"
        );
    }

    #[test]
    fn format_transfer_with_fee_ignores_fee_when_legs_cancel() {
        // When the legs cancel, the output must stay byte-identical to the
        // fee-less formatter (fee account ignored).
        let e1 = make_entry("txn-1", "2024-01-15", "Transfer", "-100.00");
        let e2 = make_entry("txn-2", "2024-01-15", "Transfer", "100.00");
        let with_fee = format_transfer_gl_transaction_with_fee(
            &e1,
            "accounts/chase",
            &e2,
            "accounts/boa",
            "gl-id",
            Some("Expenses:Bank Fees"),
        );
        let without_fee =
            format_transfer_gl_transaction(&e1, "accounts/chase", &e2, "accounts/boa", "gl-id");
        assert_eq!(with_fee, without_fee);
        assert!(!with_fee.contains("Expenses:Bank Fees"));
    }

    /// Post two entries with the given amounts as separate Unknown GL txns and
    /// return their GL ids (shared setup for the fee-merge tests).
    fn post_pair_for_merge(root: &Path, amount1: &str, amount2: &str) -> (String, String) {
        fs::write(root.join("general.journal"), "").unwrap();
        let journal_path = account_journal::login_account_journal_path(root, "chase", "checking");
        account_journal::write_journal_at_path(
            &journal_path,
            &[
                make_entry("txn-1", "2024-01-15", "Transfer out", amount1),
                make_entry("txn-2", "2024-01-15", "Transfer in", amount2),
            ],
        )
        .unwrap();
        let gl1 = post_login_account_entry(
            root,
            "chase",
            "checking",
            "txn-1",
            "Expenses:Unknown",
            None,
            "test",
        )
        .unwrap();
        let gl2 = post_login_account_entry(
            root,
            "chase",
            "checking",
            "txn-2",
            "Expenses:Unknown",
            None,
            "test",
        )
        .unwrap();
        (gl1, gl2)
    }

    #[test]
    fn merge_with_fee_creates_three_leg_block() {
        let root = temp_dir("merge-with-fee");
        let (gl1, gl2) = post_pair_for_merge(&root, "-100.00", "99.75");

        let merged =
            merge_gl_transfer(&root, &gl1, &gl2, Some("Expenses:Bank Fees"), "test").unwrap();

        let gl = fs::read_to_string(root.join("general.journal")).unwrap();
        assert!(gl.contains(&format!("id: {merged}")));
        assert!(
            gl.contains("    Expenses:Bank Fees  0.25 USD\n"),
            "fee leg with residual expected, got: {gl}"
        );
        assert!(gl.contains("    Assets:Checking  -100.00 USD\n"));
        assert!(gl.contains("    Assets:Checking  99.75 USD\n"));
        // Both entries point at the merged txn.
        let journal_path = account_journal::login_account_journal_path(&root, "chase", "checking");
        let entries = account_journal::read_journal_at_path(&journal_path).unwrap();
        let gl_ref = format!("general.journal:{merged}");
        assert_eq!(entries[0].posted.as_deref(), Some(gl_ref.as_str()));
        assert_eq!(entries[1].posted.as_deref(), Some(gl_ref.as_str()));
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn merge_with_fee_ignores_fee_when_legs_cancel() {
        let root = temp_dir("merge-fee-cancelling");
        let (gl1, gl2) = post_pair_for_merge(&root, "-100.00", "100.00");

        merge_gl_transfer(&root, &gl1, &gl2, Some("Expenses:Bank Fees"), "test").unwrap();

        let gl = fs::read_to_string(root.join("general.journal")).unwrap();
        assert!(
            !gl.contains("Expenses:Bank Fees"),
            "cancelling legs must merge without a fee leg, got: {gl}"
        );
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn merge_without_fee_error_includes_residual() {
        let root = temp_dir("merge-no-fee-residual");
        let (gl1, gl2) = post_pair_for_merge(&root, "-100.00", "99.75");

        let err = merge_gl_transfer(&root, &gl1, &gl2, None, "test").unwrap_err();
        assert!(
            err.to_string().contains("do not cancel"),
            "expected the cancel refusal, got: {err}"
        );
        assert!(
            err.to_string().contains("0.25"),
            "error should state the residual, got: {err}"
        );
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn merge_rejects_balance_sheet_fee_account() {
        let root = temp_dir("merge-fee-balance-sheet");
        let (gl1, gl2) = post_pair_for_merge(&root, "-100.00", "99.75");

        let err = merge_gl_transfer(&root, &gl1, &gl2, Some("Assets:Slush"), "test").unwrap_err();
        assert!(
            err.to_string().contains("balance-sheet"),
            "expected fee-account validation error, got: {err}"
        );
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn post_login_account_transfer_with_fee_writes_three_legs() {
        let root = temp_dir("post-transfer-fee");
        fs::write(root.join("general.journal"), "").unwrap();
        let path1 = account_journal::login_account_journal_path(&root, "chase", "checking");
        account_journal::write_journal_at_path(
            &path1,
            &[make_entry("txn-a", "2024-01-15", "Transfer out", "-200.00")],
        )
        .unwrap();
        let mut incoming = make_entry("txn-b", "2024-01-15", "Transfer in", "199.50");
        incoming.postings[0].account = "Assets:Savings".to_string();
        let path2 = account_journal::login_account_journal_path(&root, "boa", "savings");
        account_journal::write_journal_at_path(&path2, &[incoming]).unwrap();

        let gl_id = post_login_account_transfer(
            &root,
            "chase",
            "checking",
            "txn-a",
            "boa",
            "savings",
            "txn-b",
            Some("Expenses:Bank Fees"),
            "test",
        )
        .unwrap();

        let gl = fs::read_to_string(root.join("general.journal")).unwrap();
        assert!(gl.contains(&format!("id: {gl_id}")));
        assert!(gl.contains("    Assets:Checking  -200.00 USD\n"));
        assert!(gl.contains("    Assets:Savings  199.50 USD\n"));
        assert!(gl.contains("    Expenses:Bank Fees  0.50 USD\n"));
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn sync_preserves_fee_leg_and_recomputes_residual() {
        let root = temp_dir("sync-fee-leg");
        let (gl1, gl2) = post_pair_for_merge(&root, "-100.00", "99.75");
        let merged =
            merge_gl_transfer(&root, &gl1, &gl2, Some("Expenses:Bank Fees"), "test").unwrap();

        // Drift leg 1: -100.00 → -100.50.
        let journal_path = account_journal::login_account_journal_path(&root, "chase", "checking");
        let mut entries = account_journal::read_journal_at_path(&journal_path).unwrap();
        entries[0].postings[0].amount = Some(SimpleAmount {
            commodity: "USD".to_string(),
            quantity: "-100.50".to_string(),
        });
        account_journal::write_journal_at_path(&journal_path, &entries).unwrap();

        sync_gl_transaction(&root, "chase", "checking", "txn-1", "test").unwrap();

        let gl = fs::read_to_string(root.join("general.journal")).unwrap();
        assert!(gl.contains(&format!("id: {merged}")));
        assert!(gl.contains("    Assets:Checking  -100.50 USD\n"));
        assert!(
            gl.contains("    Expenses:Bank Fees  0.75 USD\n"),
            "fee leg should carry the recomputed residual, got: {gl}"
        );
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn sync_drops_fee_leg_when_legs_now_cancel() {
        let root = temp_dir("sync-fee-drop");
        let (gl1, gl2) = post_pair_for_merge(&root, "-100.00", "99.75");
        merge_gl_transfer(&root, &gl1, &gl2, Some("Expenses:Bank Fees"), "test").unwrap();

        // Drift leg 2 up to a perfect cancel: 99.75 → 100.00.
        let journal_path = account_journal::login_account_journal_path(&root, "chase", "checking");
        let mut entries = account_journal::read_journal_at_path(&journal_path).unwrap();
        entries[1].postings[0].amount = Some(SimpleAmount {
            commodity: "USD".to_string(),
            quantity: "100.00".to_string(),
        });
        account_journal::write_journal_at_path(&journal_path, &entries).unwrap();

        sync_gl_transaction(&root, "chase", "checking", "txn-2", "test").unwrap();

        let gl = fs::read_to_string(root.join("general.journal")).unwrap();
        assert!(
            !gl.contains("Expenses:Bank Fees"),
            "cancelling legs should drop the fee leg, got: {gl}"
        );
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn sync_refuses_transfer_block_with_extra_leg() {
        // A 2-source block with 4+ postings (fee leg + a manual extra leg) cannot
        // be reproduced by format_transfer_gl_transaction_with_fee, which only
        // recognizes a single trailing fee leg. Rewriting it would silently
        // discard the extra leg, so sync must refuse (mirrors the 1-source split
        // refusal).
        let root = temp_dir("sync-extra-leg");
        let (gl1, gl2) = post_pair_for_merge(&root, "-100.00", "99.75");
        let merged =
            merge_gl_transfer(&root, &gl1, &gl2, Some("Expenses:Bank Fees"), "test").unwrap();

        // Inject a 4th posting line into the block, simulating a manual edit.
        let gl_path = root.join("general.journal");
        let gl = fs::read_to_string(&gl_path).unwrap();
        let gl = gl.replace(
            "    Expenses:Bank Fees  0.25 USD\n",
            "    Expenses:Bank Fees  0.25 USD\n    Expenses:Extra  1.00 USD\n",
        );
        fs::write(&gl_path, &gl).unwrap();

        let err = sync_gl_transaction(&root, "chase", "checking", "txn-1", "test").unwrap_err();
        assert!(
            err.to_string().contains("more than 3 postings"),
            "expected extra-leg refusal, got: {err}"
        );
        let after = fs::read_to_string(&gl_path).unwrap();
        assert!(
            after.contains(&format!("id: {merged}"))
                && after.contains("    Expenses:Extra  1.00 USD\n")
                && after.contains("    Expenses:Bank Fees  0.25 USD\n"),
            "block must be unchanged by the refused sync, got: {after}"
        );
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn unpost_fee_merge_clears_both_refs() {
        // A 3-leg fee transfer unposts like any transfer: the source tags are
        // unaffected by the fee leg, so both sides' refs are cleared and the
        // whole block (fee leg included) is removed. Uses two separate journals
        // (the canonical transfer shape).
        let root = temp_dir("unpost-fee-merge");
        fs::write(root.join("general.journal"), "").unwrap();
        let path1 = account_journal::login_account_journal_path(&root, "chase", "checking");
        account_journal::write_journal_at_path(
            &path1,
            &[make_entry("txn-1", "2024-01-15", "Transfer out", "-100.00")],
        )
        .unwrap();
        let path2 = account_journal::login_account_journal_path(&root, "boa", "savings");
        account_journal::write_journal_at_path(
            &path2,
            &[make_entry("txn-2", "2024-01-15", "Transfer in", "99.75")],
        )
        .unwrap();
        let gl1 = post_login_account_entry(
            &root,
            "chase",
            "checking",
            "txn-1",
            "Expenses:Unknown",
            None,
            "test",
        )
        .unwrap();
        let gl2 = post_login_account_entry(
            &root,
            "boa",
            "savings",
            "txn-2",
            "Expenses:Unknown",
            None,
            "test",
        )
        .unwrap();
        let merged =
            merge_gl_transfer(&root, &gl1, &gl2, Some("Expenses:Bank Fees"), "test").unwrap();
        let gl = fs::read_to_string(root.join("general.journal")).unwrap();
        assert!(gl.contains("Expenses:Bank Fees"), "fee leg expected: {gl}");

        unpost_login_account_entry(&root, "chase", "checking", "txn-1", None, None, "test")
            .unwrap();

        let entries1 = account_journal::read_journal_at_path(&path1).unwrap();
        assert!(entries1[0].posted.is_none(), "txn-1 ref should be cleared");
        let entries2 = account_journal::read_journal_at_path(&path2).unwrap();
        assert!(entries2[0].posted.is_none(), "txn-2 ref should be cleared");
        let gl = fs::read_to_string(root.join("general.journal")).unwrap();
        assert!(
            !gl.contains(&format!("id: {merged}")),
            "the merged block should be removed"
        );
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn unpost_gl_transaction_unmerges_transfer_by_gl_id() {
        // GL-side unmerge: resolve the txn's first source tag server-side and
        // route through unpost_login_account_entry (clears all sides and records
        // the NotTransferLink negative memory).
        let root = temp_dir("unpost-gl-txn");
        fs::write(root.join("general.journal"), "").unwrap();
        let path1 = account_journal::login_account_journal_path(&root, "chase", "checking");
        account_journal::write_journal_at_path(
            &path1,
            &[make_entry("txn-1", "2024-01-15", "Transfer out", "-100.00")],
        )
        .unwrap();
        let path2 = account_journal::login_account_journal_path(&root, "boa", "savings");
        account_journal::write_journal_at_path(
            &path2,
            &[make_entry("txn-2", "2024-01-15", "Transfer in", "100.00")],
        )
        .unwrap();
        let gl_id = post_login_account_transfer(
            &root, "chase", "checking", "txn-1", "boa", "savings", "txn-2", None, "test",
        )
        .unwrap();

        unpost_gl_transaction(&root, &gl_id, "test").unwrap();

        let entries1 = account_journal::read_journal_at_path(&path1).unwrap();
        assert!(entries1[0].posted.is_none(), "txn-1 ref should be cleared");
        let entries2 = account_journal::read_journal_at_path(&path2).unwrap();
        assert!(entries2[0].posted.is_none(), "txn-2 ref should be cleared");
        let gl = fs::read_to_string(root.join("general.journal")).unwrap();
        assert!(!gl.contains(&format!("id: {gl_id}")), "block removed");
        // Negative memory recorded by the underlying unpost.
        let resolutions = crate::automation::list_resolutions(&root).unwrap();
        assert!(
            resolutions.iter().any(|r| {
                r.kind == crate::automation::ResolutionKind::NotTransferLink
                    && r.status == crate::automation::ResolutionStatus::Active
            }),
            "unmerge should record a NotTransferLink"
        );
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn unpost_gl_transaction_refuses_when_entry_posted_elsewhere() {
        // Orphaned GL block O (an interrupted op left it behind) still carries a
        // `; source:` tag pointing at txn-1, but txn-1's LIVE posted ref points at
        // the real block N. "Unmerge" on O must refuse rather than silently remove
        // N, and must record no negative memory.
        let root = temp_dir("unpost-gl-wrong-txn");
        fs::write(root.join("general.journal"), "").unwrap();
        let path = account_journal::login_account_journal_path(&root, "chase", "checking");
        account_journal::write_journal_at_path(
            &path,
            &[make_entry("txn-1", "2024-01-15", "Transfer out", "-100.00")],
        )
        .unwrap();
        let gl_n = post_login_account_entry(
            &root,
            "chase",
            "checking",
            "txn-1",
            "Expenses:Unknown",
            None,
            "test",
        )
        .unwrap();
        // Append an orphaned block O whose source tag also resolves to txn-1.
        let orphan = "\n2024-01-15  *Transfer out  ; id: orphan-o\n    ; generated-by: refreshmint-post\n    ; source: logins/chase/accounts/checking:txn-1\n    Assets:Checking  -100.00 USD\n    Expenses:Unknown\n";
        let gl_path = root.join("general.journal");
        let existing = fs::read_to_string(&gl_path).unwrap();
        fs::write(&gl_path, format!("{existing}{orphan}")).unwrap();

        let err = unpost_gl_transaction(&root, "orphan-o", "test").unwrap_err();
        assert!(
            err.to_string().contains("txn-1") && err.to_string().contains("orphan-o"),
            "expected posted-elsewhere refusal, got: {err}"
        );
        let gl = fs::read_to_string(&gl_path).unwrap();
        assert!(
            gl.contains(&format!("id: {gl_n}")),
            "real block N must survive the refused unpost"
        );
        let entries = account_journal::read_journal_at_path(&path).unwrap();
        assert_eq!(
            entries[0].posted.as_deref(),
            Some(format!("general.journal:{gl_n}").as_str()),
            "txn-1 must remain posted to N"
        );
        let resolutions = crate::automation::list_resolutions(&root).unwrap();
        assert!(
            resolutions.is_empty(),
            "no negative memory should be recorded on a refused unpost"
        );
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn unpost_gl_transaction_errors_without_source_tag() {
        let root = temp_dir("unpost-gl-no-source");
        fs::write(
            root.join("general.journal"),
            "2024-01-15 Manual  ; id: manual-1\n    Assets:A  1 USD\n    Income:B\n",
        )
        .unwrap();
        let err = unpost_gl_transaction(&root, "manual-1", "test").unwrap_err();
        assert!(
            err.to_string().contains("source"),
            "expected a no-source-tag error, got: {err}"
        );
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn not_transfer_link_for_gl_pair_creates_resolution() {
        let root = temp_dir("ntl-gl-pair");
        let (gl1, gl2) = post_pair_for_merge(&root, "-100.00", "100.00");

        let resolution = create_not_transfer_link_for_gl_pair(&root, &gl1, &gl2).unwrap();
        assert_eq!(
            resolution.kind,
            crate::automation::ResolutionKind::NotTransferLink
        );
        assert_eq!(
            resolution.status,
            crate::automation::ResolutionStatus::Active
        );
        assert_eq!(resolution.subject_refs.len(), 2);
        // The subjects are the SOURCE entries (survive merges/unposts), not the
        // GL txn ids.
        assert!(resolution
            .subject_refs
            .iter()
            .all(|r| r.entry_id.as_deref() == Some("txn-1")
                || r.entry_id.as_deref() == Some("txn-2")));
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn not_transfer_link_for_gl_pair_errors_without_source_tag() {
        let root = temp_dir("ntl-gl-pair-no-source");
        let (gl1, _) = post_pair_for_merge(&root, "-100.00", "100.00");
        // Append a manual txn with no source tag.
        append_to_journal(
            &root.join("general.journal"),
            "2024-01-15 Manual  ; id: manual-1\n    Assets:A  1 USD\n    Income:B\n",
        )
        .unwrap();

        let err = create_not_transfer_link_for_gl_pair(&root, &gl1, "manual-1").unwrap_err();
        assert!(
            err.to_string().contains("source"),
            "expected a no-source-tag error, got: {err}"
        );
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn not_transfer_link_for_gl_pair_rejects_self_id() {
        let root = temp_dir("ntl-gl-self-id");
        let (gl1, _gl2) = post_pair_for_merge(&root, "-100.00", "100.00");
        let err = create_not_transfer_link_for_gl_pair(&root, &gl1, &gl1).unwrap_err();
        assert!(
            err.to_string().contains("itself"),
            "expected a self-pair refusal, got: {err}"
        );
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn not_transfer_link_for_gl_pair_rejects_same_source_entry() {
        // Two DIFFERENT GL txn ids whose first source tags resolve to the SAME
        // account entry: recording them as not-a-transfer would block the entry
        // against itself. Refuse.
        let root = temp_dir("ntl-gl-same-source");
        let (gl1, _gl2) = post_pair_for_merge(&root, "-100.00", "100.00");
        append_to_journal(
            &root.join("general.journal"),
            "2024-01-15  *Dup  ; id: dup-1\n    ; generated-by: refreshmint-post\n    ; source: logins/chase/accounts/checking:txn-1\n    Assets:Checking  -100.00 USD\n    Expenses:Unknown\n",
        )
        .unwrap();
        let err = create_not_transfer_link_for_gl_pair(&root, &gl1, "dup-1").unwrap_err();
        assert!(
            err.to_string().contains("same source entry"),
            "expected a same-source-entry refusal, got: {err}"
        );
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn post_transfer_posts_both_sides() {
        // post_transfer is the legacy accounts/<name> transfer path, kept only for
        // the CLI `account-transfer` subcommand. Verify it posts both legs and
        // links them to a single GL transaction.
        let root = temp_dir("post-transfer");
        fs::write(root.join("general.journal"), "").unwrap();

        // Set up two accounts with one entry each.
        let entries1 = vec![make_entry("txn-a", "2024-01-15", "Transfer out", "-200.00")];
        let entries2 = vec![make_entry("txn-b", "2024-01-15", "Transfer in", "200.00")];
        account_journal::write_journal(&root, "chase", &entries1).unwrap();
        account_journal::write_journal(&root, "boa", &entries2).unwrap();

        // Post as a transfer.
        let gl_id = post_transfer(&root, "chase", "txn-a", "boa", "txn-b").unwrap();

        // The GL transaction exists.
        let gl_content = fs::read_to_string(root.join("general.journal")).unwrap();
        assert!(gl_content.contains(&format!("id: {gl_id}")));

        // Both sides are posted to the same GL ref.
        let after1 = account_journal::read_journal(&root, "chase").unwrap();
        let after2 = account_journal::read_journal(&root, "boa").unwrap();
        let gl_ref = format!("general.journal:{gl_id}");
        assert_eq!(after1[0].posted.as_deref(), Some(gl_ref.as_str()));
        assert_eq!(after2[0].posted.as_deref(), Some(gl_ref.as_str()));

        let _ = fs::remove_dir_all(&root);
    }

    fn head_commit_count(dir: &std::path::Path) -> usize {
        let repo = git2::Repository::open(dir).unwrap();
        let mut walk = repo.revwalk().unwrap();
        walk.push_head().unwrap();
        walk.count()
    }

    #[test]
    fn unpost_sync_retire_commit_to_git() {
        let root = temp_dir("mutation-commits");
        let journal_path = account_journal::login_account_journal_path(&root, "chase", "checking");

        // unpost commits.
        account_journal::write_journal_at_path(
            &journal_path,
            &[make_entry("txn-1", "2024-01-15", "Shell", "-21.32")],
        )
        .unwrap();
        post_login_account_entry(
            &root,
            "chase",
            "checking",
            "txn-1",
            "Expenses:Gas",
            None,
            "test",
        )
        .unwrap();
        let before = head_commit_count(&root);
        unpost_login_account_entry(&root, "chase", "checking", "txn-1", None, None, "test")
            .unwrap();
        assert!(
            head_commit_count(&root) > before,
            "unpost must create a git commit"
        );

        // sync commits.
        post_login_account_entry(
            &root,
            "chase",
            "checking",
            "txn-1",
            "Expenses:Gas",
            None,
            "test",
        )
        .unwrap();
        let mut entries = account_journal::read_journal_at_path(&journal_path).unwrap();
        entries[0].postings[0].amount = Some(account_journal::SimpleAmount {
            commodity: "USD".to_string(),
            quantity: "-25.00".to_string(),
        });
        account_journal::write_journal_at_path(&journal_path, &entries).unwrap();
        let before = head_commit_count(&root);
        sync_gl_transaction(&root, "chase", "checking", "txn-1", "test").unwrap();
        assert!(
            head_commit_count(&root) > before,
            "sync must create a git commit"
        );

        // retire commits (unpost first so the entry is retirable).
        unpost_login_account_entry(&root, "chase", "checking", "txn-1", None, None, "test")
            .unwrap();
        let before = head_commit_count(&root);
        retire_login_account_entry(&root, "chase", "checking", "txn-1", "test reason", "test")
            .unwrap();
        assert!(
            head_commit_count(&root) > before,
            "retire must create a git commit"
        );

        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn split_post_blocked_when_legs_already_posted() {
        let root = temp_dir("split-double-materialize");
        fs::write(root.join("general.journal"), "").unwrap();
        let entry = make_entry("txn-1", "2024-01-15", "Shop", "-30.00");
        let journal_path = account_journal::login_account_journal_path(&root, "chase", "checking");
        account_journal::write_journal_at_path(&journal_path, &[entry]).unwrap();

        // Post posting 0 as a single leg.
        post_login_account_entry(
            &root,
            "chase",
            "checking",
            "txn-1",
            "Expenses:A",
            Some(0),
            "test",
        )
        .unwrap();

        // A whole-entry split must now be refused (it would materialize the
        // amount a second time).
        let err = post_login_account_entry_split(
            &root,
            "chase",
            "checking",
            "txn-1",
            vec![
                SplitCounterpart {
                    account: "Expenses:B".into(),
                    amount: Some("10.00 USD".into()),
                },
                SplitCounterpart {
                    account: "Expenses:C".into(),
                    amount: None,
                },
            ],
            "test",
        )
        .unwrap_err();
        assert!(
            err.to_string().contains("posted split postings"),
            "split should be blocked, got: {err}"
        );

        // And a whole-entry plain post must also be refused.
        let err2 = post_login_account_entry(
            &root,
            "chase",
            "checking",
            "txn-1",
            "Expenses:D",
            None,
            "test",
        )
        .unwrap_err();
        assert!(
            err2.to_string().contains("posted split postings"),
            "whole-entry post should be blocked, got: {err2}"
        );

        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn sync_refuses_to_collapse_split_gl_txn() {
        let root = temp_dir("sync-split-guard");
        fs::write(root.join("general.journal"), "").unwrap();
        let entry = make_entry("txn-1", "2024-01-15", "Shop", "-30.00");
        let journal_path = account_journal::login_account_journal_path(&root, "chase", "checking");
        account_journal::write_journal_at_path(&journal_path, &[entry]).unwrap();

        post_login_account_entry_split(
            &root,
            "chase",
            "checking",
            "txn-1",
            vec![
                SplitCounterpart {
                    account: "Expenses:Food".into(),
                    amount: Some("20.00 USD".into()),
                },
                SplitCounterpart {
                    account: "Expenses:Travel".into(),
                    amount: Some("10.00 USD".into()),
                },
            ],
            "test",
        )
        .unwrap();

        // Drift the source amount so sync would be invoked.
        let mut entries = account_journal::read_journal_at_path(&journal_path).unwrap();
        entries[0].postings[0].amount = Some(account_journal::SimpleAmount {
            commodity: "USD".to_string(),
            quantity: "-35.00".to_string(),
        });
        account_journal::write_journal_at_path(&journal_path, &entries).unwrap();

        let err = sync_gl_transaction(&root, "chase", "checking", "txn-1", "test").unwrap_err();
        assert!(
            err.to_string().contains("split"),
            "sync should refuse to collapse a split, got: {err}"
        );

        // Both legs must survive the refused sync.
        let gl = fs::read_to_string(root.join("general.journal")).unwrap();
        assert!(
            gl.contains("Expenses:Food") && gl.contains("Expenses:Travel"),
            "split legs must survive"
        );

        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn unpost_blocked_when_gl_txn_reconciled() {
        let root = temp_dir("unpost-reconciled-guard");
        fs::write(root.join("general.journal"), "").unwrap();

        let entry = make_entry("txn-1", "2024-01-15", "Shell Oil", "-21.32");
        let journal_path = account_journal::login_account_journal_path(&root, "chase", "checking");
        account_journal::write_journal_at_path(&journal_path, &[entry]).unwrap();

        let gl_id = post_login_account_entry(
            &root,
            "chase",
            "checking",
            "txn-1",
            "Expenses:Gas",
            None,
            "test",
        )
        .unwrap();

        // A finalized reconciliation session protects the GL transaction.
        let sessions_dir =
            crate::bookkeeping::bookkeeping_dir(&root).join("reconciliation-sessions");
        fs::create_dir_all(&sessions_dir).unwrap();
        fs::write(
            sessions_dir.join("sess-1.json"),
            format!(
                r#"{{"id":"sess-1","glAccount":"Assets:Checking","statementStartDate":null,"statementEndDate":"2024-01-31","statementStartingBalance":null,"statementEndingBalance":"0.00","currency":null,"status":"finalized","reconciledTxnIds":["{gl_id}"],"notes":null,"createdAt":"2024-02-01T00:00:00Z","updatedAt":"2024-02-01T00:00:00Z"}}"#
            ),
        )
        .unwrap();

        let err =
            unpost_login_account_entry(&root, "chase", "checking", "txn-1", None, None, "test")
                .unwrap_err();
        assert!(
            err.to_string().contains("protected"),
            "unpost of a reconciled GL txn must be blocked, got: {err}"
        );

        // The GL block and posted ref must survive the blocked unpost.
        let gl_content = fs::read_to_string(root.join("general.journal")).unwrap();
        assert!(
            gl_content.contains(&format!("id: {gl_id}")),
            "GL txn must remain after a blocked unpost"
        );
        let entries = account_journal::read_journal_at_path(&journal_path).unwrap();
        assert!(
            entries[0].posted.is_some(),
            "entry must remain posted after a blocked unpost"
        );

        let _ = fs::remove_dir_all(&root);
    }

    const RECAT_GL_BLOCK: &str = "2024-01-15  * Shell Oil  ; id: abc-123\n    ; generated-by: refreshmint-post\n    ; source: logins/chase/accounts/checking:txn-1\n    Assets:Checking  -21.32 USD\n    Expenses:Unknown\n";

    #[test]
    fn recategorize_rejects_balance_sheet_leg() {
        // Posting index 0 is the Assets bank leg; rewriting it would move money
        // off the reconciled account.
        let err = apply_recategorizations(
            RECAT_GL_BLOCK,
            &[("abc-123".to_string(), 0, "Expenses:Gas".to_string())],
        )
        .unwrap_err();
        assert!(
            err.contains("balance-sheet") && err.contains("Assets:Checking"),
            "expected balance-sheet refusal, got: {err}"
        );
    }

    #[test]
    fn recategorize_allows_counterpart_leg() {
        // Posting index 1 is the counterpart (Expenses:Unknown); recategorizing it
        // is the intended operation and must succeed with the bank leg untouched.
        let out = apply_recategorizations(
            RECAT_GL_BLOCK,
            &[("abc-123".to_string(), 1, "Expenses:Gas".to_string())],
        )
        .unwrap();
        assert!(
            out.contains("Expenses:Gas"),
            "counterpart must be rewritten"
        );
        assert!(!out.contains("Expenses:Unknown"), "old counterpart gone");
        assert!(
            out.contains("Assets:Checking  -21.32 USD"),
            "bank leg must be preserved verbatim"
        );
    }

    #[test]
    fn sync_blocked_when_gl_txn_reconciled() {
        // sync rewrites the GL block's amount/status; a finalized reconciliation
        // must protect it. Mirror unpost_blocked_when_gl_txn_reconciled.
        let root = temp_dir("sync-reconciled-guard");
        fs::write(root.join("general.journal"), "").unwrap();

        let entry = make_entry("txn-1", "2024-01-15", "Shell Oil", "-21.32");
        let journal_path = account_journal::login_account_journal_path(&root, "chase", "checking");
        account_journal::write_journal_at_path(&journal_path, &[entry]).unwrap();

        let gl_id = post_login_account_entry(
            &root,
            "chase",
            "checking",
            "txn-1",
            "Expenses:Gas",
            None,
            "test",
        )
        .unwrap();

        // Drift the source amount so sync would rewrite the block.
        let mut entries = account_journal::read_journal_at_path(&journal_path).unwrap();
        entries[0].postings[0].amount = Some(account_journal::SimpleAmount {
            commodity: "USD".to_string(),
            quantity: "-25.00".to_string(),
        });
        account_journal::write_journal_at_path(&journal_path, &entries).unwrap();

        // A finalized reconciliation session protects the GL transaction.
        let sessions_dir =
            crate::bookkeeping::bookkeeping_dir(&root).join("reconciliation-sessions");
        fs::create_dir_all(&sessions_dir).unwrap();
        fs::write(
            sessions_dir.join("sess-1.json"),
            format!(
                r#"{{"id":"sess-1","glAccount":"Assets:Checking","statementStartDate":null,"statementEndDate":"2024-01-31","statementStartingBalance":null,"statementEndingBalance":"0.00","currency":null,"status":"finalized","reconciledTxnIds":["{gl_id}"],"notes":null,"createdAt":"2024-02-01T00:00:00Z","updatedAt":"2024-02-01T00:00:00Z"}}"#
            ),
        )
        .unwrap();

        let err = sync_gl_transaction(&root, "chase", "checking", "txn-1", "test").unwrap_err();
        assert!(
            err.to_string().contains("protected"),
            "sync of a reconciled GL txn must be blocked, got: {err}"
        );

        // The GL block must still carry the original -21.32 amount.
        let gl_content = fs::read_to_string(root.join("general.journal")).unwrap();
        assert!(
            gl_content.contains("-21.32") && !gl_content.contains("-25.00"),
            "GL block must be unchanged after a blocked sync"
        );
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn merge_refuses_multi_source_block() {
        // Merging a block that is already a transfer would drop one leg's posted:
        // ref (merge keeps only the first source of each block).
        let root = temp_dir("merge-multi-source");
        fs::write(root.join("general.journal"), "").unwrap();
        let journal_path = account_journal::login_account_journal_path(&root, "chase", "checking");
        account_journal::write_journal_at_path(
            &journal_path,
            &[
                make_entry("txn-1", "2024-01-15", "Transfer out", "-100.00"),
                make_entry("txn-2", "2024-01-15", "Transfer in", "100.00"),
                make_entry("txn-3", "2024-01-16", "Other", "-100.00"),
            ],
        )
        .unwrap();
        let gl1 = post_login_account_entry(
            &root,
            "chase",
            "checking",
            "txn-1",
            "Expenses:Unknown",
            None,
            "test",
        )
        .unwrap();
        let gl2 = post_login_account_entry(
            &root,
            "chase",
            "checking",
            "txn-2",
            "Expenses:Unknown",
            None,
            "test",
        )
        .unwrap();
        let merged = merge_gl_transfer(&root, &gl1, &gl2, None, "test").unwrap();
        let gl3 = post_login_account_entry(
            &root,
            "chase",
            "checking",
            "txn-3",
            "Expenses:Unknown",
            None,
            "test",
        )
        .unwrap();

        let err = merge_gl_transfer(&root, &merged, &gl3, None, "test").unwrap_err();
        assert!(
            err.to_string().contains("sources") && err.to_string().contains("cannot merge"),
            "expected multi-source refusal, got: {err}"
        );
        let gl = fs::read_to_string(root.join("general.journal")).unwrap();
        assert!(
            gl.contains(&format!("id: {merged}")),
            "merged transfer must survive the refused merge"
        );
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn merge_refuses_split_txn() {
        // Merging a split posting would collapse it to a single counterpart.
        let root = temp_dir("merge-split-guard");
        fs::write(root.join("general.journal"), "").unwrap();
        let journal_path = account_journal::login_account_journal_path(&root, "chase", "checking");
        account_journal::write_journal_at_path(
            &journal_path,
            &[
                make_entry("txn-1", "2024-01-15", "Shop", "-30.00"),
                make_entry("txn-2", "2024-01-15", "Transfer in", "30.00"),
            ],
        )
        .unwrap();
        let gl1 = post_login_account_entry_split(
            &root,
            "chase",
            "checking",
            "txn-1",
            vec![
                SplitCounterpart {
                    account: "Expenses:Food".into(),
                    amount: Some("20.00 USD".into()),
                },
                SplitCounterpart {
                    account: "Expenses:Travel".into(),
                    amount: None,
                },
            ],
            "test",
        )
        .unwrap();
        let gl2 = post_login_account_entry(
            &root,
            "chase",
            "checking",
            "txn-2",
            "Expenses:Unknown",
            None,
            "test",
        )
        .unwrap();

        let err = merge_gl_transfer(&root, &gl1, &gl2, None, "test").unwrap_err();
        assert!(
            err.to_string().contains("split"),
            "expected split refusal, got: {err}"
        );
        let gl = fs::read_to_string(root.join("general.journal")).unwrap();
        assert!(
            gl.contains("Expenses:Food") && gl.contains("Expenses:Travel"),
            "split legs must survive the refused merge"
        );
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn merge_blocked_when_reconciled() {
        // A finalized reconciliation session must protect both txns from a merge.
        let root = temp_dir("merge-reconciled-guard");
        fs::write(root.join("general.journal"), "").unwrap();
        let journal_path = account_journal::login_account_journal_path(&root, "chase", "checking");
        account_journal::write_journal_at_path(
            &journal_path,
            &[
                make_entry("txn-1", "2024-01-15", "Transfer out", "-100.00"),
                make_entry("txn-2", "2024-01-15", "Transfer in", "100.00"),
            ],
        )
        .unwrap();
        let gl1 = post_login_account_entry(
            &root,
            "chase",
            "checking",
            "txn-1",
            "Expenses:Unknown",
            None,
            "test",
        )
        .unwrap();
        let gl2 = post_login_account_entry(
            &root,
            "chase",
            "checking",
            "txn-2",
            "Expenses:Unknown",
            None,
            "test",
        )
        .unwrap();

        let sessions_dir =
            crate::bookkeeping::bookkeeping_dir(&root).join("reconciliation-sessions");
        fs::create_dir_all(&sessions_dir).unwrap();
        fs::write(
            sessions_dir.join("sess-1.json"),
            format!(
                r#"{{"id":"sess-1","glAccount":"Assets:Checking","statementStartDate":null,"statementEndDate":"2024-01-31","statementStartingBalance":null,"statementEndingBalance":"0.00","currency":null,"status":"finalized","reconciledTxnIds":["{gl1}"],"notes":null,"createdAt":"2024-02-01T00:00:00Z","updatedAt":"2024-02-01T00:00:00Z"}}"#
            ),
        )
        .unwrap();

        let err = merge_gl_transfer(&root, &gl1, &gl2, None, "test").unwrap_err();
        assert!(
            err.to_string().contains("protected"),
            "merge of a reconciled txn must be blocked, got: {err}"
        );
        let gl = fs::read_to_string(root.join("general.journal")).unwrap();
        assert!(
            gl.contains(&format!("id: {gl1}")) && gl.contains(&format!("id: {gl2}")),
            "both GL txns must survive the blocked merge"
        );
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn merge_refuses_non_opposite_amounts() {
        // Unequal legs would be silently misstated (leg 2 is forced to -leg1).
        let root = temp_dir("merge-non-opposite");
        fs::write(root.join("general.journal"), "").unwrap();
        let journal_path = account_journal::login_account_journal_path(&root, "chase", "checking");
        account_journal::write_journal_at_path(
            &journal_path,
            &[
                make_entry("txn-1", "2024-01-15", "Transfer out", "-100.00"),
                make_entry("txn-2", "2024-01-15", "Transfer in", "50.00"),
            ],
        )
        .unwrap();
        let gl1 = post_login_account_entry(
            &root,
            "chase",
            "checking",
            "txn-1",
            "Expenses:Unknown",
            None,
            "test",
        )
        .unwrap();
        let gl2 = post_login_account_entry(
            &root,
            "chase",
            "checking",
            "txn-2",
            "Expenses:Unknown",
            None,
            "test",
        )
        .unwrap();

        let err = merge_gl_transfer(&root, &gl1, &gl2, None, "test").unwrap_err();
        assert!(
            err.to_string().contains("do not cancel"),
            "expected opposite-amount refusal, got: {err}"
        );
        let gl = fs::read_to_string(root.join("general.journal")).unwrap();
        assert!(
            gl.contains(&format!("id: {gl1}")) && gl.contains(&format!("id: {gl2}")),
            "both GL txns must survive the refused merge"
        );
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn merge_refuses_different_commodities() {
        // Quantities that cancel numerically are still wrong across commodities:
        // the merged block stores only leg 1's amount+commodity and forces leg 2
        // to its negation, so -100 EUR / +100 USD would misstate the USD account.
        let root = temp_dir("merge-diff-commodity");
        fs::write(root.join("general.journal"), "").unwrap();
        let journal_path = account_journal::login_account_journal_path(&root, "chase", "checking");
        let mut eur_entry = make_entry("txn-1", "2024-01-15", "Wire out", "-100.00");
        eur_entry.postings[0].amount.as_mut().unwrap().commodity = "EUR".to_string();
        account_journal::write_journal_at_path(
            &journal_path,
            &[
                eur_entry,
                make_entry("txn-2", "2024-01-15", "Wire in", "100.00"),
            ],
        )
        .unwrap();
        let gl1 = post_login_account_entry(
            &root,
            "chase",
            "checking",
            "txn-1",
            "Expenses:Unknown",
            None,
            "test",
        )
        .unwrap();
        let gl2 = post_login_account_entry(
            &root,
            "chase",
            "checking",
            "txn-2",
            "Expenses:Unknown",
            None,
            "test",
        )
        .unwrap();

        let err = merge_gl_transfer(&root, &gl1, &gl2, None, "test").unwrap_err();
        assert!(
            err.to_string().contains("commodities differ"),
            "expected commodity refusal, got: {err}"
        );
        let gl = fs::read_to_string(root.join("general.journal")).unwrap();
        assert!(
            gl.contains(&format!("id: {gl1}")) && gl.contains(&format!("id: {gl2}")),
            "both GL txns must survive the refused merge"
        );
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn merge_refuses_missing_amount() {
        // An entry whose amount is absent (or unparseable) must refuse the merge
        // rather than silently skipping the cancellation check.
        let root = temp_dir("merge-missing-amount");
        fs::write(root.join("general.journal"), "").unwrap();
        let journal_path = account_journal::login_account_journal_path(&root, "chase", "checking");
        account_journal::write_journal_at_path(
            &journal_path,
            &[
                make_entry("txn-1", "2024-01-15", "Transfer out", "-100.00"),
                make_entry("txn-2", "2024-01-15", "Transfer in", "100.00"),
            ],
        )
        .unwrap();
        let gl1 = post_login_account_entry(
            &root,
            "chase",
            "checking",
            "txn-1",
            "Expenses:Unknown",
            None,
            "test",
        )
        .unwrap();
        let gl2 = post_login_account_entry(
            &root,
            "chase",
            "checking",
            "txn-2",
            "Expenses:Unknown",
            None,
            "test",
        )
        .unwrap();
        // Drop txn-2's amount after posting; merge re-reads the journal.
        let mut entries = account_journal::read_journal_at_path(&journal_path).unwrap();
        entries
            .iter_mut()
            .find(|e| e.id == "txn-2")
            .unwrap()
            .postings[0]
            .amount = None;
        account_journal::write_journal_at_path(&journal_path, &entries).unwrap();

        let err = merge_gl_transfer(&root, &gl1, &gl2, None, "test").unwrap_err();
        assert!(
            err.to_string().contains("no amount"),
            "expected missing-amount refusal, got: {err}"
        );
        let gl = fs::read_to_string(root.join("general.journal")).unwrap();
        assert!(
            gl.contains(&format!("id: {gl1}")) && gl.contains(&format!("id: {gl2}")),
            "both GL txns must survive the refused merge"
        );
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn transfer_candidate_score_honors_extra_patterns() {
        // A configured extraTransferPattern must exempt a candidate from the
        // +1000 non-transfer penalty, mirroring the matchers. "MOVE MONEY"
        // matches no built-in transfer pattern.
        let entry = make_entry("cand", "2024-01-20", "MOVE MONEY 123", "50.00");
        let without = transfer_candidate_score(&entry, "2024-01-15", "unrelated", None, &[]);
        let with = transfer_candidate_score(
            &entry,
            "2024-01-15",
            "unrelated",
            None,
            &["move money".to_string()],
        );
        assert_eq!(
            without - with,
            1000,
            "a configured pattern should drop exactly the non-transfer penalty"
        );
    }

    #[test]
    fn post_transfer_refuses_different_commodities() {
        // The Pipeline modal lists unposted entries in any commodity, so a
        // cross-currency pick must be refused rather than encoded as an implicit
        // FX conversion. Mirrors merge_refuses_different_commodities.
        let root = temp_dir("post-transfer-diff-commodity");
        fs::write(root.join("general.journal"), "").unwrap();
        let path1 = account_journal::login_account_journal_path(&root, "chase", "checking");
        let mut eur = make_entry("txn-1", "2024-01-15", "Wire out", "-100.00");
        eur.postings[0].amount.as_mut().unwrap().commodity = "EUR".to_string();
        account_journal::write_journal_at_path(&path1, &[eur]).unwrap();
        let path2 = account_journal::login_account_journal_path(&root, "boa", "savings");
        account_journal::write_journal_at_path(
            &path2,
            &[make_entry("txn-2", "2024-01-15", "Wire in", "100.00")],
        )
        .unwrap();

        let err = post_login_account_transfer(
            &root, "chase", "checking", "txn-1", "boa", "savings", "txn-2", None, "test",
        )
        .unwrap_err();
        assert!(
            err.to_string().contains("commodities differ"),
            "expected commodity refusal, got: {err}"
        );
        assert!(
            account_journal::read_journal_at_path(&path1).unwrap()[0]
                .posted
                .is_none(),
            "no leg should be posted on a refused transfer"
        );
        let gl = fs::read_to_string(root.join("general.journal")).unwrap();
        assert!(gl.trim().is_empty(), "no GL block on refused post: {gl}");
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn post_transfer_refuses_missing_amount() {
        // An absent/unparseable amount must refuse rather than silently fall back
        // to the fee-less shape (leg 2 forced to -leg1). Mirrors
        // merge_refuses_missing_amount.
        let root = temp_dir("post-transfer-missing-amount");
        fs::write(root.join("general.journal"), "").unwrap();
        let path1 = account_journal::login_account_journal_path(&root, "chase", "checking");
        account_journal::write_journal_at_path(
            &path1,
            &[make_entry("txn-1", "2024-01-15", "Transfer out", "-100.00")],
        )
        .unwrap();
        let path2 = account_journal::login_account_journal_path(&root, "boa", "savings");
        let mut in_entry = make_entry("txn-2", "2024-01-15", "Transfer in", "100.00");
        in_entry.postings[0].amount = None;
        account_journal::write_journal_at_path(&path2, &[in_entry]).unwrap();

        let err = post_login_account_transfer(
            &root, "chase", "checking", "txn-1", "boa", "savings", "txn-2", None, "test",
        )
        .unwrap_err();
        assert!(
            err.to_string().contains("no amount"),
            "expected missing-amount refusal, got: {err}"
        );
        let gl = fs::read_to_string(root.join("general.journal")).unwrap();
        assert!(gl.trim().is_empty(), "no GL block on refused post: {gl}");
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn post_transfer_requires_fee_when_legs_do_not_cancel() {
        // Non-cancelling legs without a fee account would be misstated (leg 2
        // forced to -leg1). Mirrors merge_refuses_non_opposite_amounts.
        let root = temp_dir("post-transfer-no-fee");
        fs::write(root.join("general.journal"), "").unwrap();
        let path1 = account_journal::login_account_journal_path(&root, "chase", "checking");
        account_journal::write_journal_at_path(
            &path1,
            &[make_entry("txn-1", "2024-01-15", "Transfer out", "-100.00")],
        )
        .unwrap();
        let path2 = account_journal::login_account_journal_path(&root, "boa", "savings");
        account_journal::write_journal_at_path(
            &path2,
            &[make_entry("txn-2", "2024-01-15", "Transfer in", "90.00")],
        )
        .unwrap();

        let err = post_login_account_transfer(
            &root, "chase", "checking", "txn-1", "boa", "savings", "txn-2", None, "test",
        )
        .unwrap_err();
        assert!(
            err.to_string().contains("do not cancel"),
            "expected non-cancelling refusal, got: {err}"
        );
        // But a fee account makes the same pick succeed as a 3-leg block.
        post_login_account_transfer(
            &root,
            "chase",
            "checking",
            "txn-1",
            "boa",
            "savings",
            "txn-2",
            Some("Expenses:Bank Fees"),
            "test",
        )
        .unwrap();
        let gl = fs::read_to_string(root.join("general.journal")).unwrap();
        assert!(
            gl.contains("Expenses:Bank Fees"),
            "fee leg expected on the accepted post: {gl}"
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

        let gl_id = post_login_account_entry(
            &root,
            "chase",
            "checking",
            "txn-1",
            "Expenses:Gas",
            None,
            "test",
        )
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
        let returned_id = sync_gl_transaction(&root, "chase", "checking", "txn-1", "test").unwrap();
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

    #[test]
    fn gl_txn_unknown_posting_index_finds_middle_unknown() {
        let root = temp_dir("unknown-idx-mid");
        // Manual txn: Unknown is the MIDDLE posting (index 1), not the last.
        fs::write(
            root.join("general.journal"),
            "2026-01-01 SAFEWAY  ; id: txn-mid\n    Assets:Chase  -10.00 USD\n    \
             Expenses:Unknown  4.00 USD\n    Expenses:Dining\n",
        )
        .unwrap();
        let index = gl_txn_unknown_posting_index(&root, "txn-mid").unwrap();
        assert_eq!(index, Some(1), "must target the Unknown leg, not the last");
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn gl_txn_unknown_posting_index_errors_on_zero_and_multiple() {
        let root = temp_dir("unknown-idx-amb");
        fs::write(
            root.join("general.journal"),
            // txn-zero: no Unknown posting at all.
            "2026-01-01 A  ; id: txn-zero\n    Assets:Chase  -10.00 USD\n    Expenses:Dining\n\n\
             2026-01-02 B  ; id: txn-two\n    Assets:Chase  -10.00 USD\n    \
             Expenses:Unknown  4.00 USD\n    Expenses:Unknown  6.00 USD\n",
        )
        .unwrap();
        assert!(
            gl_txn_unknown_posting_index(&root, "txn-zero").is_err(),
            "zero Unknown postings must error, not silently pick a leg"
        );
        assert!(
            gl_txn_unknown_posting_index(&root, "txn-two").is_err(),
            "multiple Unknown postings are an ambiguous target and must error"
        );
        // A missing txn is a distinct, non-error case (Ok(None)).
        assert_eq!(gl_txn_unknown_posting_index(&root, "nope").unwrap(), None);
        let _ = fs::remove_dir_all(&root);
    }
}
