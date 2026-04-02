use serde::{Deserialize, Serialize};
use std::fs::{self, OpenOptions};
use std::io::{self, BufRead, Write};
use std::path::{Path, PathBuf};

/// A tag is a key-value pair, matching hledger's tag model.
pub type Tag = (String, String);

/// An operation in the per-account operations log.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum AccountOperation {
    /// Records the creation of an account journal entry (for ID stability during re-derivation).
    #[serde(rename = "entry-created")]
    EntryCreated {
        #[serde(rename = "entryId")]
        entry_id: String,
        evidence: Vec<String>,
        date: String,
        amount: String,
        tags: Vec<Tag>,
        timestamp: String,
    },

    /// A manually-added transaction.
    #[serde(rename = "manual-add")]
    ManualAdd {
        #[serde(rename = "entryId")]
        entry_id: String,
        date: String,
        description: String,
        amount: String,
        timestamp: String,
    },

    /// Override dedup decision: force-match or prevent-match.
    #[serde(rename = "dedup-override")]
    DedupOverride {
        action: DedupOverrideAction,
        #[serde(rename = "entryId")]
        entry_id: String,
        #[serde(rename = "proposedEvidence")]
        proposed_evidence: Vec<String>,
        timestamp: String,
    },

    /// Records removal of a scrape session's effects.
    #[serde(rename = "remove-scrape")]
    RemoveScrape {
        #[serde(rename = "scrapeSessionId")]
        scrape_session_id: String,
        timestamp: String,
    },
}

/// Dedup override action: force two entries to match, or prevent them from matching.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum DedupOverrideAction {
    ForceMatch,
    PreventMatch,
}

/// An operation in the GL-level operations log.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum GlOperation {
    /// Post an account journal entry to the GL with a counterpart account.
    #[serde(rename = "post")]
    Post {
        account: String,
        #[serde(rename = "entryId")]
        entry_id: String,
        #[serde(rename = "counterpartAccount")]
        counterpart_account: String,
        #[serde(rename = "postingIndex")]
        posting_index: Option<usize>,
        timestamp: String,
    },

    /// Post an account journal entry to the GL split across multiple counterpart accounts.
    #[serde(rename = "post-split")]
    PostSplit {
        account: String,
        #[serde(rename = "entryId")]
        entry_id: String,
        #[serde(rename = "counterpartAccounts")]
        counterpart_accounts: Vec<String>,
        timestamp: String,
    },

    /// Match two entries across accounts as an inter-account transfer.
    #[serde(rename = "transfer-match")]
    TransferMatch {
        entries: Vec<TransferMatchEntry>,
        timestamp: String,
    },

    /// Undo a previous posting.
    #[serde(rename = "undo-post")]
    UndoPost {
        account: String,
        #[serde(rename = "entryId")]
        entry_id: String,
        #[serde(rename = "postingIndex")]
        posting_index: Option<usize>,
        timestamp: String,
    },

    /// Sync an existing GL transaction in-place (amounts/status updated).
    #[serde(rename = "sync-transaction")]
    SyncTransaction {
        account: String,
        #[serde(rename = "entryId")]
        entry_id: String,
        #[serde(rename = "glTxnId")]
        gl_txn_id: String,
        /// Snapshot of all source entries after the sync (for audit/replay).
        sources: Vec<SyncSource>,
        timestamp: String,
    },
}

/// A source-entry snapshot recorded inside a `SyncTransaction` operation.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SyncSource {
    pub account: String,
    #[serde(rename = "entryId")]
    pub entry_id: String,
    /// New amount (quantity + commodity), if available.
    pub amount: Option<String>,
    /// New status marker: `""`, `"!"`, or `"*"`.
    pub status: String,
}

/// An entry in a transfer-match operation.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TransferMatchEntry {
    pub account: String,
    #[serde(rename = "entryId")]
    pub entry_id: String,
}

/// Returns the path to the per-account operations log.
pub fn account_operations_path(ledger_dir: &Path, account_name: &str) -> PathBuf {
    ledger_dir
        .join("accounts")
        .join(account_name)
        .join("operations.jsonl")
}

/// Returns the path to the per-login-account operations log.
pub fn login_account_operations_path(ledger_dir: &Path, login_name: &str, label: &str) -> PathBuf {
    ledger_dir
        .join("logins")
        .join(login_name)
        .join("accounts")
        .join(label)
        .join("operations.jsonl")
}

/// Returns the path to the GL-level operations log.
pub fn gl_operations_path(ledger_dir: &Path) -> PathBuf {
    ledger_dir.join("operations.jsonl")
}

/// Append an account-level operation to the per-account operations log.
pub fn append_account_operation(
    ledger_dir: &Path,
    account_name: &str,
    operation: &AccountOperation,
) -> io::Result<()> {
    let path = account_operations_path(ledger_dir, account_name);
    append_jsonl(&path, operation)
}

/// Append an account-level operation to a login account operations log.
pub fn append_login_account_operation(
    ledger_dir: &Path,
    login_name: &str,
    label: &str,
    operation: &AccountOperation,
) -> io::Result<()> {
    let path = login_account_operations_path(ledger_dir, login_name, label);
    append_jsonl(&path, operation)
}

/// Read all account-level operations from the per-account operations log.
pub fn read_account_operations(
    ledger_dir: &Path,
    account_name: &str,
) -> io::Result<Vec<AccountOperation>> {
    let path = account_operations_path(ledger_dir, account_name);
    read_jsonl(&path)
}

/// Read all account-level operations from a login account operations log.
pub fn read_login_account_operations(
    ledger_dir: &Path,
    login_name: &str,
    label: &str,
) -> io::Result<Vec<AccountOperation>> {
    let path = login_account_operations_path(ledger_dir, login_name, label);
    read_jsonl(&path)
}

/// Append a GL-level operation to the root operations log.
pub fn append_gl_operation(ledger_dir: &Path, operation: &GlOperation) -> io::Result<()> {
    let path = gl_operations_path(ledger_dir);
    append_jsonl(&path, operation)
}

/// Read all GL-level operations from the root operations log.
pub fn read_gl_operations(ledger_dir: &Path) -> io::Result<Vec<GlOperation>> {
    let path = gl_operations_path(ledger_dir);
    read_jsonl(&path)
}

/// A scrape run log entry persisted per-login to `logins/<login>/scrape-log.jsonl`.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ScrapeLogEntry {
    pub login_name: String,
    pub timestamp: String,
    pub success: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    /// `"manual"` for user-triggered runs, `"auto"` for auto-scrape runs.
    pub source: String,
}

/// Returns the path to the per-login scrape log.
pub fn login_scrape_log_path(ledger_dir: &Path, login_name: &str) -> PathBuf {
    ledger_dir
        .join("logins")
        .join(login_name)
        .join("scrape-log.jsonl")
}

/// Append a scrape log entry to the per-login scrape log.
pub fn append_scrape_log_entry(ledger_dir: &Path, entry: &ScrapeLogEntry) -> io::Result<()> {
    append_jsonl(&login_scrape_log_path(ledger_dir, &entry.login_name), entry)
}

/// Read all scrape log entries for a login (oldest-first).
pub fn read_scrape_log(ledger_dir: &Path, login_name: &str) -> io::Result<Vec<ScrapeLogEntry>> {
    read_jsonl(&login_scrape_log_path(ledger_dir, login_name))
}

/// A structured console log line emitted by an extractor script.
// On-disk format: camelCase fields in JSONL.
// Keep the field set aligned with ConsoleLogLine in extract.rs.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ExtractConsoleLogLine {
    /// One of: "log", "info", "warn", "error", "debug"
    pub level: String,
    pub message: String,
    /// The document that was being extracted when this line was emitted.
    pub document_name: String,
}

/// An extract run log entry persisted per-login-account to
/// `logins/<login>/accounts/<label>/extract-log.jsonl`.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ExtractLogEntry {
    pub login_name: String,
    pub label: String,
    pub timestamp: String,
    pub success: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    pub document_count: usize,
    pub new_entry_count: usize,
    pub console_logs: Vec<ExtractConsoleLogLine>,
}

/// Returns the path to the per-login-account extract log.
pub fn login_extract_log_path(ledger_dir: &Path, login_name: &str, label: &str) -> PathBuf {
    ledger_dir
        .join("logins")
        .join(login_name)
        .join("accounts")
        .join(label)
        .join("extract-log.jsonl")
}

/// Append an extract log entry to the per-login-account extract log.
pub fn append_extract_log_entry(ledger_dir: &Path, entry: &ExtractLogEntry) -> io::Result<()> {
    append_jsonl(
        &login_extract_log_path(ledger_dir, &entry.login_name, &entry.label),
        entry,
    )
}

/// Generate an ISO 8601 timestamp for the current time.
pub fn now_timestamp() -> String {
    chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Millis, true)
}

fn append_jsonl<T: Serialize>(path: &Path, value: &T) -> io::Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }

    let mut line = serde_json::to_string(value).map_err(io::Error::other)?;
    line.push('\n');

    // Write to temp file and append atomically is complex for append-only logs.
    // For a single-user desktop app, direct append is safe.
    let mut file = OpenOptions::new().create(true).append(true).open(path)?;
    file.write_all(line.as_bytes())?;
    Ok(())
}

fn read_jsonl<T: for<'de> Deserialize<'de>>(path: &Path) -> io::Result<Vec<T>> {
    if !path.exists() {
        return Ok(Vec::new());
    }

    let file = fs::File::open(path)?;
    let reader = io::BufReader::new(file);
    let mut operations = Vec::new();

    for (line_num, line) in reader.lines().enumerate() {
        let line = line?;
        let trimmed = line.trim();
        if trimmed.is_empty() || trimmed.starts_with('#') {
            continue;
        }
        let op: T = serde_json::from_str(trimmed).map_err(|e| {
            io::Error::other(format!(
                "{}:{}: invalid JSON: {e}",
                path.display(),
                line_num + 1
            ))
        })?;
        operations.push(op);
    }

    Ok(operations)
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;
    use std::time::{SystemTime, UNIX_EPOCH};

    fn temp_dir(prefix: &str) -> PathBuf {
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let dir = std::env::temp_dir().join(format!(
            "refreshmint-ops-{prefix}-{}-{now}",
            std::process::id()
        ));
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn round_trip_account_operations() {
        let root = temp_dir("acct-ops");
        let op = AccountOperation::EntryCreated {
            entry_id: "txn-abc".to_string(),
            evidence: vec!["2024-02-17-transactions.csv:12:1".to_string()],
            date: "2024-02-15".to_string(),
            amount: "-21.32".to_string(),
            tags: vec![("bankId".to_string(), "FIT123".to_string())],
            timestamp: now_timestamp(),
        };

        append_account_operation(&root, "chase-checking", &op).unwrap();
        let ops = read_account_operations(&root, "chase-checking").unwrap();
        assert_eq!(ops.len(), 1);
        if let AccountOperation::EntryCreated { entry_id, .. } = &ops[0] {
            assert_eq!(entry_id, "txn-abc");
        } else {
            panic!("expected EntryCreated");
        }

        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn round_trip_gl_operations() {
        let root = temp_dir("gl-ops");
        let op = GlOperation::Post {
            account: "chase-checking".to_string(),
            entry_id: "txn-abc123".to_string(),
            counterpart_account: "Expenses:Food".to_string(),
            posting_index: None,
            timestamp: now_timestamp(),
        };

        append_gl_operation(&root, &op).unwrap();
        let ops = read_gl_operations(&root).unwrap();
        assert_eq!(ops.len(), 1);
        if let GlOperation::Post { account, .. } = &ops[0] {
            assert_eq!(account, "chase-checking");
        } else {
            panic!("expected Post");
        }

        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn append_multiple_operations() {
        let root = temp_dir("multi-ops");
        let op1 = AccountOperation::EntryCreated {
            entry_id: "txn-1".to_string(),
            evidence: vec!["doc.csv:1:1".to_string()],
            date: "2024-01-01".to_string(),
            amount: "10.00".to_string(),
            tags: vec![],
            timestamp: now_timestamp(),
        };
        let op2 = AccountOperation::RemoveScrape {
            scrape_session_id: "20240219-090000".to_string(),
            timestamp: now_timestamp(),
        };

        append_account_operation(&root, "test-acct", &op1).unwrap();
        append_account_operation(&root, "test-acct", &op2).unwrap();
        let ops = read_account_operations(&root, "test-acct").unwrap();
        assert_eq!(ops.len(), 2);

        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn read_empty_returns_empty_vec() {
        let root = temp_dir("empty-ops");
        let ops = read_account_operations(&root, "nonexistent").unwrap();
        assert!(ops.is_empty());

        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn round_trip_scrape_log() {
        let root = temp_dir("scrape-log");
        // Nonexistent login returns empty vec.
        let entries = read_scrape_log(&root, "bankofamerica").unwrap();
        assert!(entries.is_empty());

        let e1 = ScrapeLogEntry {
            login_name: "bankofamerica".to_string(),
            timestamp: "2026-03-29T18:39:45.123Z".to_string(),
            success: false,
            error: Some("no progress in last 3 steps".to_string()),
            source: "auto".to_string(),
        };
        let e2 = ScrapeLogEntry {
            login_name: "bankofamerica".to_string(),
            timestamp: "2026-03-29T19:00:00.000Z".to_string(),
            success: true,
            error: None,
            source: "manual".to_string(),
        };
        // Create the login dir so append_scrape_log_entry can write.
        fs::create_dir_all(root.join("logins").join("bankofamerica")).unwrap();
        append_scrape_log_entry(&root, &e1).unwrap();
        append_scrape_log_entry(&root, &e2).unwrap();

        let entries = read_scrape_log(&root, "bankofamerica").unwrap();
        assert_eq!(entries.len(), 2);
        assert!(!entries[0].success);
        assert_eq!(
            entries[0].error.as_deref(),
            Some("no progress in last 3 steps")
        );
        assert_eq!(entries[0].source, "auto");
        assert!(entries[1].success);
        assert!(entries[1].error.is_none());
        assert_eq!(entries[1].source, "manual");

        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn round_trip_extract_log() {
        let root = temp_dir("extract-log");

        let entry = ExtractLogEntry {
            login_name: "target-yon".to_string(),
            label: "_default".to_string(),
            timestamp: now_timestamp(),
            success: true,
            error: None,
            document_count: 2,
            new_entry_count: 3,
            console_logs: vec![
                ExtractConsoleLogLine {
                    level: "warn".to_string(),
                    message: "payment sum 15.00 != grandTotal 15.50".to_string(),
                    document_name: "order-123.json".to_string(),
                },
                ExtractConsoleLogLine {
                    level: "log".to_string(),
                    message: "processed 5 payments".to_string(),
                    document_name: "order-456.json".to_string(),
                },
            ],
        };

        // Nonexistent login returns empty vec before any writes.
        let before =
            read_jsonl::<ExtractLogEntry>(&login_extract_log_path(&root, "target-yon", "_default"))
                .unwrap();
        assert!(before.is_empty());

        append_extract_log_entry(&root, &entry).unwrap();

        let entries =
            read_jsonl::<ExtractLogEntry>(&login_extract_log_path(&root, "target-yon", "_default"))
                .unwrap();
        assert_eq!(entries.len(), 1);
        assert!(entries[0].success);
        assert!(entries[0].error.is_none());
        assert_eq!(entries[0].document_count, 2);
        assert_eq!(entries[0].new_entry_count, 3);
        assert_eq!(entries[0].console_logs.len(), 2);
        assert_eq!(entries[0].console_logs[0].level, "warn");
        assert_eq!(entries[0].console_logs[0].document_name, "order-123.json");
        assert_eq!(entries[0].console_logs[1].level, "log");

        let _ = fs::remove_dir_all(&root);
    }
}
