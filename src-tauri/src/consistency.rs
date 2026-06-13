//! Referential consistency check and recovery between account journals and
//! `general.journal`.
//!
//! A post appends the GL transaction first, then writes the account journal's
//! `posted:` ref, then commits (see [crate::post]). A hard kill — which neither
//! `Drop` nor the shutdown hook can clean up — can leave two kinds of mismatch:
//!
//! - **Orphaned GL txn**: a refreshmint-generated GL transaction (it has a
//!   `; source:` line) whose source entry does not reference it back. This is
//!   the normal crash-window state of GL-first posting, and it is *recoverable*:
//!   the GL block names its source entry, so [recover_ledger] re-links it.
//! - **Dangling ref**: an account entry claims `posted: general.journal:<id>`
//!   but no GL transaction with that id exists. GL-first ordering makes this
//!   impossible to produce from a post; it would indicate external corruption,
//!   and it is *not* auto-recoverable (the GL data is gone), so it is surfaced
//!   for the user to clear.
//!
//! [recover_ledger] (run on ledger open) auto-completes recoverable orphans;
//! [analyze]/[check_ledger] only report. The GL block format and the
//! `; source:` / `posted:` ref conventions are defined in [crate::post] and
//! [crate::gl_journal]; keep this aligned with them.

use std::collections::HashSet;
use std::path::Path;

use serde::Serialize;

use crate::account_journal::{self, AccountEntry};
use crate::{gl_journal, login_config, post};

/// An account entry that claims to be posted to a GL transaction that is absent
/// from `general.journal`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DanglingRef {
    pub login_name: String,
    pub label: String,
    pub entry_id: String,
    /// `Some(idx)` for a per-posting (split) ref; `None` for a whole-entry ref.
    pub posting_index: Option<usize>,
    pub gl_txn_id: String,
}

/// A refreshmint-generated GL transaction whose source entry does not reference
/// it back.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct OrphanedGlTxn {
    pub gl_txn_id: String,
    pub source_locator: String,
    pub source_entry_id: String,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ConsistencyReport {
    pub dangling_refs: Vec<DanglingRef>,
    pub orphaned_gl_txns: Vec<OrphanedGlTxn>,
}

impl ConsistencyReport {
    pub fn is_clean(&self) -> bool {
        self.dangling_refs.is_empty() && self.orphaned_gl_txns.is_empty()
    }
}

/// An account journal loaded for analysis.
pub struct AccountJournal {
    pub login_name: String,
    pub label: String,
    pub entries: Vec<AccountEntry>,
}

fn gl_id_from_ref(posted_ref: &str) -> &str {
    posted_ref
        .strip_prefix("general.journal:")
        .unwrap_or(posted_ref)
}

/// Pure analysis: cross-check account `posted:` refs against the GL transaction
/// ids present in `gl_content`. Separated from IO so it is deterministically
/// testable.
pub fn analyze(gl_content: &str, accounts: &[AccountJournal]) -> ConsistencyReport {
    let blocks = gl_journal::split_journal_blocks(gl_content);

    // GL transaction ids actually present, and the (id, sources) of every
    // refreshmint-generated block (one that carries a `; source:` line).
    let mut gl_ids: HashSet<String> = HashSet::new();
    let mut sourced_blocks: Vec<(String, Vec<(String, String)>)> = Vec::new();
    for block in &blocks {
        let Some(id) = gl_journal::block_transaction_id(block) else {
            continue;
        };
        gl_ids.insert(id.clone());
        let sources = post::parse_sources_from_block(block);
        if !sources.is_empty() {
            sourced_blocks.push((id, sources));
        }
    }

    let mut report = ConsistencyReport::default();

    // Dangling refs: account entry -> GL id that is not present.
    for journal in accounts {
        for entry in &journal.entries {
            let mut refs: Vec<(Option<usize>, &str)> = Vec::new();
            if let Some(posted) = &entry.posted {
                refs.push((None, posted.as_str()));
            }
            for (idx, posted) in &entry.posted_postings {
                refs.push((Some(*idx), posted.as_str()));
            }
            for (posting_index, posted_ref) in refs {
                let id = gl_id_from_ref(posted_ref);
                if !gl_ids.contains(id) {
                    report.dangling_refs.push(DanglingRef {
                        login_name: journal.login_name.clone(),
                        label: journal.label.clone(),
                        entry_id: entry.id.clone(),
                        posting_index,
                        gl_txn_id: id.to_string(),
                    });
                }
            }
        }
    }

    // Orphaned GL txns: a sourced block whose source entry doesn't reference it.
    for (gl_id, sources) in &sourced_blocks {
        for (locator, entry_id) in sources {
            if !source_entry_references(accounts, locator, entry_id, gl_id) {
                report.orphaned_gl_txns.push(OrphanedGlTxn {
                    gl_txn_id: gl_id.clone(),
                    source_locator: locator.clone(),
                    source_entry_id: entry_id.clone(),
                });
            }
        }
    }

    report
}

/// Whether the entry named by `locator`/`entry_id` exists and references `gl_id`
/// (via its whole-entry or any per-posting `posted:` ref).
fn source_entry_references(
    accounts: &[AccountJournal],
    locator: &str,
    entry_id: &str,
    gl_id: &str,
) -> bool {
    let Some((login, label)) = parse_login_locator(locator) else {
        // Non-login locator (e.g. a manual `accounts/...` source) is outside the
        // set we loaded; don't flag it as orphaned.
        return true;
    };
    let Some(journal) = accounts
        .iter()
        .find(|j| j.login_name == login && j.label == label)
    else {
        return false;
    };
    let Some(entry) = journal.entries.iter().find(|e| e.id == entry_id) else {
        return false;
    };
    if entry
        .posted
        .as_deref()
        .is_some_and(|posted| gl_id_from_ref(posted) == gl_id)
    {
        return true;
    }
    entry
        .posted_postings
        .iter()
        .any(|(_, posted)| gl_id_from_ref(posted) == gl_id)
}

/// Parse a `logins/<login>/accounts/<label>` locator into `(login, label)`.
fn parse_login_locator(locator: &str) -> Option<(String, String)> {
    let rest = locator.strip_prefix("logins/")?;
    let accounts_pos = rest.find("/accounts/")?;
    let login = &rest[..accounts_pos];
    let label = &rest[accounts_pos + "/accounts/".len()..];
    if login.is_empty() || label.is_empty() {
        return None;
    }
    Some((login.to_string(), label.to_string()))
}

/// Load every account journal under `ledger_dir/logins/*/accounts/*`.
pub fn load_account_journals(ledger_dir: &Path) -> std::io::Result<Vec<AccountJournal>> {
    let mut journals = Vec::new();
    for login_name in login_config::list_logins(ledger_dir)? {
        let accounts_dir = ledger_dir.join("logins").join(&login_name).join("accounts");
        let read_dir = match std::fs::read_dir(&accounts_dir) {
            Ok(read_dir) => read_dir,
            Err(_) => continue,
        };
        for entry in read_dir.flatten() {
            if !entry.file_type().map(|t| t.is_dir()).unwrap_or(false) {
                continue;
            }
            let label = entry.file_name().to_string_lossy().into_owned();
            let path = account_journal::login_account_journal_path(ledger_dir, &login_name, &label);
            if !path.exists() {
                continue;
            }
            let entries = account_journal::read_journal_at_path(&path)?;
            journals.push(AccountJournal {
                login_name: login_name.clone(),
                label,
                entries,
            });
        }
    }
    Ok(journals)
}

/// Read the GL and all account journals from disk and analyze them.
pub fn check_ledger(ledger_dir: &Path) -> std::io::Result<ConsistencyReport> {
    let gl_content = match std::fs::read_to_string(ledger_dir.join("general.journal")) {
        Ok(content) => content,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => String::new(),
        Err(err) => return Err(err),
    };
    let accounts = load_account_journals(ledger_dir)?;
    Ok(analyze(&gl_content, &accounts))
}

/// Auto-complete recoverable orphans, then return the residual report.
///
/// This is the redo-log replay that makes GL-first posting effectively atomic
/// (see [crate::post]): a hard kill between the GL append and the account write
/// leaves an orphaned GL transaction whose `; source:` names the source entry,
/// so we restore that entry's `posted:` ref. Run on ledger open. Whole-entry
/// orphans (simple posts, splits, transfers) are completed; anything else
/// (dangling refs, non-login sources) is returned for the user to handle.
pub fn recover_ledger(ledger_dir: &Path, lock_owner: &str) -> std::io::Result<ConsistencyReport> {
    let report = check_ledger(ledger_dir)?;
    let completable: Vec<(String, String, String, String)> = report
        .orphaned_gl_txns
        .iter()
        .filter_map(|orphan| {
            parse_login_locator(&orphan.source_locator).map(|(login, label)| {
                (
                    login,
                    label,
                    orphan.source_entry_id.clone(),
                    orphan.gl_txn_id.clone(),
                )
            })
        })
        .collect();
    if completable.is_empty() {
        return Ok(report);
    }
    for (login, label, entry_id, gl_txn_id) in completable {
        // Best-effort: a failure for one orphan still surfaces in the re-scan.
        if let Err(err) = post::restore_posted_ref(
            ledger_dir, &login, &label, &entry_id, &gl_txn_id, lock_owner,
        ) {
            eprintln!("[recover] could not restore {login}/{label} {entry_id}: {err}");
        }
    }
    check_ledger(ledger_dir)
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;

    fn entry(id: &str, posted: Option<&str>) -> AccountEntry {
        AccountEntry {
            id: id.to_string(),
            date: "2026-01-01".to_string(),
            status: account_journal::EntryStatus::Unmarked,
            description: String::new(),
            comment: String::new(),
            evidence: Vec::new(),
            postings: Vec::new(),
            tags: Vec::new(),
            extracted_by: None,
            posted: posted.map(str::to_string),
            posted_postings: Vec::new(),
        }
    }

    fn journal(login: &str, label: &str, entries: Vec<AccountEntry>) -> AccountJournal {
        AccountJournal {
            login_name: login.to_string(),
            label: label.to_string(),
            entries,
        }
    }

    const GL_WITH_TXN: &str = "2026-01-01 Coffee  ; id: txn-1\n    ; source: logins/chase/accounts/checking:entry-1\n    Expenses:Unknown  5 USD\n    Assets:Chase\n";

    #[test]
    fn clean_ledger_reports_no_problems() {
        let accounts = vec![journal(
            "chase",
            "checking",
            vec![entry("entry-1", Some("general.journal:txn-1"))],
        )];
        let report = analyze(GL_WITH_TXN, &accounts);
        assert!(report.is_clean(), "expected clean, got {report:?}");
    }

    #[test]
    fn detects_dangling_ref_when_gl_txn_missing() {
        // Entry claims posted to txn-1, but the GL is empty (post killed before
        // the GL append).
        let accounts = vec![journal(
            "chase",
            "checking",
            vec![entry("entry-1", Some("general.journal:txn-1"))],
        )];
        let report = analyze("", &accounts);
        assert_eq!(
            report.dangling_refs,
            vec![DanglingRef {
                login_name: "chase".to_string(),
                label: "checking".to_string(),
                entry_id: "entry-1".to_string(),
                posting_index: None,
                gl_txn_id: "txn-1".to_string(),
            }]
        );
        assert!(report.orphaned_gl_txns.is_empty());
    }

    #[test]
    fn detects_orphaned_gl_txn_when_source_entry_does_not_reference_it() {
        // GL has txn-1 sourced from entry-1, but entry-1 is not marked posted
        // (post killed after the GL append, before the account write — or the
        // account write was rolled back).
        let accounts = vec![journal("chase", "checking", vec![entry("entry-1", None)])];
        let report = analyze(GL_WITH_TXN, &accounts);
        assert_eq!(
            report.orphaned_gl_txns,
            vec![OrphanedGlTxn {
                gl_txn_id: "txn-1".to_string(),
                source_locator: "logins/chase/accounts/checking".to_string(),
                source_entry_id: "entry-1".to_string(),
            }]
        );
        assert!(report.dangling_refs.is_empty());
    }

    #[test]
    fn manual_gl_txn_without_source_is_not_orphaned() {
        // A GL transaction with no `; source:` line (e.g. a manual entry) must
        // not be reported as orphaned.
        let gl = "2026-01-01 Manual  ; id: manual-1\n    Expenses:Misc  1 USD\n    Assets:Cash\n";
        let report = analyze(gl, &[]);
        assert!(report.is_clean(), "got {report:?}");
    }

    fn temp_ledger(prefix: &str) -> std::path::PathBuf {
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let dir =
            std::env::temp_dir().join(format!("refreshmint-{prefix}-{}-{now}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn recover_ledger_completes_orphan_from_gl_source() {
        // Simulate a GL-first crash: the GL block (with its `; source:` backref)
        // is written, but the source entry's posted ref was never written.
        let dir = temp_ledger("recover-orphan");
        std::fs::write(dir.join("general.journal"), GL_WITH_TXN).unwrap();
        let path = account_journal::login_account_journal_path(&dir, "chase", "checking");
        account_journal::write_journal_at_path(
            &path,
            std::slice::from_ref(&entry("entry-1", None)),
        )
        .unwrap();

        // Before recovery: the GL txn is orphaned.
        assert_eq!(check_ledger(&dir).unwrap().orphaned_gl_txns.len(), 1);

        let residual = recover_ledger(&dir, "test").unwrap();
        assert!(
            residual.is_clean(),
            "recovery should heal the orphan: {residual:?}"
        );

        // The entry now references the GL txn (re-linked, not re-created).
        let entries = account_journal::read_journal_at_path(&path).unwrap();
        assert_eq!(entries[0].posted.as_deref(), Some("general.journal:txn-1"));

        let _ = std::fs::remove_dir_all(&dir);
    }
}
