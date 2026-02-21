use serde::{Deserialize, Serialize};
use std::fmt::Write as FmtWrite;
use std::fs::{self, OpenOptions};
use std::io::{self, Write};
use std::path::{Path, PathBuf};

/// Status of a transaction entry, matching hledger conventions.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum EntryStatus {
    Unmarked,
    Pending,
    Cleared,
}

impl EntryStatus {
    pub fn hledger_marker(&self) -> &str {
        match self {
            Self::Unmarked => "",
            Self::Pending => "! ",
            Self::Cleared => "* ",
        }
    }
}

/// A simple amount for account journal entries.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SimpleAmount {
    pub commodity: String,
    pub quantity: String,
}

/// A posting within an account journal entry.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EntryPosting {
    pub account: String,
    pub amount: Option<SimpleAmount>,
}

/// A single account journal entry with provenance metadata.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AccountEntry {
    pub id: String,
    pub date: String,
    pub status: EntryStatus,
    pub description: String,
    pub comment: String,
    pub evidence: Vec<String>,
    pub postings: Vec<EntryPosting>,
    #[serde(default)]
    pub tags: Vec<(String, String)>,
    #[serde(default)]
    pub extracted_by: Option<String>,
    #[serde(default)]
    pub reconciled: Option<String>,
    #[serde(default)]
    pub reconciled_postings: Vec<(usize, String)>,
}

impl AccountEntry {
    /// Generate a new entry with a random UUID.
    pub fn new(
        date: String,
        status: EntryStatus,
        description: String,
        evidence: Vec<String>,
        postings: Vec<EntryPosting>,
    ) -> Self {
        Self {
            id: uuid::Uuid::new_v4().to_string(),
            date,
            status,
            description,
            comment: String::new(),
            evidence,
            postings,
            tags: Vec::new(),
            extracted_by: None,
            reconciled: None,
            reconciled_postings: Vec::new(),
        }
    }

    /// Get the value of a tag by key name.
    pub fn tag_value(&self, key: &str) -> Option<&str> {
        self.tags
            .iter()
            .find(|(k, _)| k == key)
            .map(|(_, v)| v.as_str())
    }

    /// Get the bankId from tags, if present.
    pub fn bank_id(&self) -> Option<&str> {
        self.tag_value("bankId")
    }

    /// Check if this entry has a specific evidence reference.
    pub fn has_evidence(&self, evidence_ref: &str) -> bool {
        self.evidence.iter().any(|e| e == evidence_ref)
    }

    /// Add an evidence reference if not already present.
    pub fn add_evidence(&mut self, evidence_ref: String) {
        if !self.evidence.iter().any(|e| e == &evidence_ref) {
            self.evidence.push(evidence_ref);
        }
    }
}

/// Returns the path to the account journal file.
pub fn account_journal_path(ledger_dir: &Path, account_name: &str) -> PathBuf {
    ledger_dir
        .join("accounts")
        .join(account_name)
        .join("account.journal")
}

/// Returns the path to the login account journal file.
pub fn login_account_journal_path(ledger_dir: &Path, login_name: &str, label: &str) -> PathBuf {
    crate::login_config::login_account_journal_path(ledger_dir, login_name, label)
}

/// Returns the path to the account documents directory.
pub fn account_documents_dir(ledger_dir: &Path, account_name: &str) -> PathBuf {
    ledger_dir
        .join("accounts")
        .join(account_name)
        .join("documents")
}

/// Returns the path to the login account documents directory.
pub fn login_account_documents_dir(ledger_dir: &Path, login_name: &str, label: &str) -> PathBuf {
    crate::login_config::login_account_documents_dir(ledger_dir, login_name, label)
}

/// Format a single entry as hledger journal text.
pub fn format_entry(entry: &AccountEntry) -> String {
    let mut buf = String::new();

    // Transaction header line: date [status] description
    let _ = write!(
        buf,
        "{}  {}{}",
        entry.date,
        entry.status.hledger_marker(),
        entry.description
    );

    // Transaction comment with tags
    let mut comments = Vec::new();

    // id tag
    comments.push(format!("id: {}", entry.id));

    // evidence tags
    for ev in &entry.evidence {
        comments.push(format!("evidence: {ev}"));
    }

    // extracted-by tag
    if let Some(extracted_by) = &entry.extracted_by {
        comments.push(format!("extracted-by: {extracted_by}"));
    }

    // reconciled tag
    if let Some(reconciled) = &entry.reconciled {
        comments.push(format!("reconciled: {reconciled}"));
    }

    // reconciled-posting-N tags
    for (idx, gl_ref) in &entry.reconciled_postings {
        comments.push(format!("reconciled-posting-{idx}: {gl_ref}"));
    }

    // custom tags
    for (key, value) in &entry.tags {
        if key != "id"
            && key != "evidence"
            && key != "extracted-by"
            && key != "reconciled"
            && !key.starts_with("reconciled-posting-")
        {
            if value.is_empty() {
                comments.push(format!("{key}:"));
            } else {
                comments.push(format!("{key}: {value}"));
            }
        }
    }

    // Inline comment if not empty
    if !entry.comment.is_empty() {
        comments.insert(0, entry.comment.clone());
    }

    // Write comment lines
    for comment in &comments {
        let _ = write!(buf, "\n    ; {comment}");
    }

    buf.push('\n');

    // Postings
    for posting in &entry.postings {
        match &posting.amount {
            Some(amount) => {
                let _ = writeln!(
                    buf,
                    "    {}  {} {}",
                    posting.account, amount.quantity, amount.commodity
                );
            }
            None => {
                let _ = writeln!(buf, "    {}", posting.account);
            }
        }
    }

    buf
}

/// Format all entries as a complete account journal file.
pub fn format_journal(entries: &[AccountEntry]) -> String {
    let mut buf = String::new();
    for (i, entry) in entries.iter().enumerate() {
        if i > 0 {
            buf.push('\n');
        }
        buf.push_str(&format_entry(entry));
    }
    buf
}

/// Write all entries to the account journal file (atomic write via temp file + rename).
pub fn write_journal(
    ledger_dir: &Path,
    account_name: &str,
    entries: &[AccountEntry],
) -> io::Result<()> {
    let path = account_journal_path(ledger_dir, account_name);
    write_journal_at_path(&path, entries)
}

/// Write all entries to a specific journal path (atomic write via temp file + rename).
pub fn write_journal_at_path(path: &Path, entries: &[AccountEntry]) -> io::Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }

    let content = format_journal(entries);
    atomic_write(path, content.as_bytes())
}

/// Append a single entry to the account journal.
pub fn append_entry(ledger_dir: &Path, account_name: &str, entry: &AccountEntry) -> io::Result<()> {
    let path = account_journal_path(ledger_dir, account_name);
    append_entry_at_path(&path, entry)
}

/// Append a single entry to a specific journal path.
pub fn append_entry_at_path(path: &Path, entry: &AccountEntry) -> io::Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }

    let formatted = format_entry(entry);
    let mut file = OpenOptions::new().create(true).append(true).open(path)?;
    if file.metadata()?.len() > 0 {
        file.write_all(b"\n")?;
    }
    file.write_all(formatted.as_bytes())?;
    Ok(())
}

/// Read all entries from the account journal by parsing the file.
///
/// This parser handles the structured format written by `format_entry`.
/// It parses the hledger-style text back into `AccountEntry` structs.
pub fn read_journal(ledger_dir: &Path, account_name: &str) -> io::Result<Vec<AccountEntry>> {
    let path = account_journal_path(ledger_dir, account_name);
    read_journal_at_path(&path)
}

/// Read all entries from a specific journal path.
pub fn read_journal_at_path(path: &Path) -> io::Result<Vec<AccountEntry>> {
    if !path.exists() {
        return Ok(Vec::new());
    }

    let content = fs::read_to_string(path)?;
    parse_journal(&content)
}

/// Parse hledger-formatted account journal text into entries.
pub fn parse_journal(content: &str) -> io::Result<Vec<AccountEntry>> {
    let mut entries = Vec::new();
    let mut lines = content.lines().peekable();

    while lines.peek().is_some() {
        // Skip blank lines
        while let Some(line) = lines.peek() {
            if line.trim().is_empty() {
                lines.next();
            } else {
                break;
            }
        }

        let Some(header_line) = lines.next() else {
            break;
        };

        let header_line = header_line.trim();
        if header_line.is_empty() {
            continue;
        }

        // Parse header: date  [status] description
        let (date, status, description) = parse_header_line(header_line)?;

        // Collect indented lines (comments and postings)
        let mut comment_lines = Vec::new();
        let mut posting_lines = Vec::new();

        while let Some(line) = lines.peek() {
            if line.is_empty() || (!line.starts_with(' ') && !line.starts_with('\t')) {
                break;
            }
            let line = lines.next().unwrap_or_default();
            let trimmed = line.trim();
            if trimmed.starts_with(';') {
                comment_lines.push(
                    trimmed
                        .strip_prefix("; ")
                        .unwrap_or(trimmed.strip_prefix(';').unwrap_or(trimmed)),
                );
            } else if !trimmed.is_empty() {
                posting_lines.push(trimmed);
            }
        }

        // Parse tags from comment lines
        let mut id = String::new();
        let mut evidence = Vec::new();
        let mut extracted_by = None;
        let mut reconciled = None;
        let mut reconciled_postings = Vec::new();
        let mut tags = Vec::new();
        let mut comment = String::new();

        for comment_line in &comment_lines {
            if let Some(rest) = comment_line.strip_prefix("id: ") {
                id = rest.trim().to_string();
            } else if let Some(rest) = comment_line.strip_prefix("evidence: ") {
                evidence.push(rest.trim().to_string());
            } else if let Some(rest) = comment_line.strip_prefix("extracted-by: ") {
                extracted_by = Some(rest.trim().to_string());
            } else if let Some(rest) = comment_line.strip_prefix("reconciled: ") {
                reconciled = Some(rest.trim().to_string());
            } else if let Some(rest) = strip_reconciled_posting_prefix(comment_line) {
                reconciled_postings.push(rest);
            } else if let Some((key, value)) = parse_tag_line(comment_line) {
                tags.push((key, value));
            } else {
                // Plain comment
                if !comment.is_empty() {
                    comment.push('\n');
                }
                comment.push_str(comment_line);
            }
        }

        // Parse postings
        let postings = posting_lines
            .iter()
            .map(|line| parse_posting_line(line))
            .collect::<io::Result<Vec<_>>>()?;

        entries.push(AccountEntry {
            id,
            date,
            status,
            description,
            comment,
            evidence,
            postings,
            tags,
            extracted_by,
            reconciled,
            reconciled_postings,
        });
    }

    Ok(entries)
}

fn parse_header_line(line: &str) -> io::Result<(String, EntryStatus, String)> {
    // Format: YYYY-MM-DD  [!|*] description
    let parts: Vec<&str> = line.splitn(2, "  ").collect();
    let date = parts.first().unwrap_or(&"").trim().to_string();

    let rest = parts.get(1).unwrap_or(&"").trim();

    let (status, description) = if let Some(desc) = rest.strip_prefix("! ") {
        (EntryStatus::Pending, desc.to_string())
    } else if let Some(desc) = rest.strip_prefix("* ") {
        (EntryStatus::Cleared, desc.to_string())
    } else {
        (EntryStatus::Unmarked, rest.to_string())
    };

    Ok((date, status, description))
}

fn parse_posting_line(line: &str) -> io::Result<EntryPosting> {
    // Format: account  amount commodity
    // or just: account
    let parts: Vec<&str> = line.splitn(2, "  ").collect();
    let account = parts.first().unwrap_or(&"").trim().to_string();
    let amount_part = parts.get(1).unwrap_or(&"").trim();

    let amount = if amount_part.is_empty() {
        None
    } else {
        // Parse "quantity commodity" or just "quantity"
        let amount_parts: Vec<&str> = amount_part.splitn(2, ' ').collect();
        let quantity = amount_parts.first().unwrap_or(&"").to_string();
        let commodity = amount_parts.get(1).unwrap_or(&"").to_string();
        Some(SimpleAmount {
            commodity,
            quantity,
        })
    };

    Ok(EntryPosting { account, amount })
}

fn strip_reconciled_posting_prefix(line: &str) -> Option<(usize, String)> {
    let rest = line.strip_prefix("reconciled-posting-")?;
    let colon_pos = rest.find(':')?;
    let idx_str = &rest[..colon_pos];
    let idx = idx_str.trim().parse::<usize>().ok()?;
    let value = rest[colon_pos + 1..].trim().to_string();
    Some((idx, value))
}

fn parse_tag_line(line: &str) -> Option<(String, String)> {
    let colon_pos = line.find(':')?;
    let key = line[..colon_pos].trim();
    if key.is_empty() || key.contains(' ') {
        return None;
    }
    let value = line[colon_pos + 1..].trim();
    Some((key.to_string(), value.to_string()))
}

fn atomic_write(path: &Path, content: &[u8]) -> io::Result<()> {
    let temp_path = path.with_extension("tmp");
    let mut file = OpenOptions::new()
        .create(true)
        .write(true)
        .truncate(true)
        .open(&temp_path)?;
    file.write_all(content)?;
    file.flush()?;
    fs::rename(&temp_path, path)?;
    Ok(())
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
            "refreshmint-aj-{prefix}-{}-{now}",
            std::process::id()
        ));
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn format_and_parse_round_trip() {
        let entry = AccountEntry {
            id: "abc-123".to_string(),
            date: "2024-02-15".to_string(),
            status: EntryStatus::Cleared,
            description: "SHELL OIL 12345".to_string(),
            comment: String::new(),
            evidence: vec!["2024-02-17-transactions.csv:12:1".to_string()],
            postings: vec![
                EntryPosting {
                    account: "Assets:Checking".to_string(),
                    amount: Some(SimpleAmount {
                        commodity: "USD".to_string(),
                        quantity: "-21.32".to_string(),
                    }),
                },
                EntryPosting {
                    account: "Equity:Unreconciled:Checking".to_string(),
                    amount: Some(SimpleAmount {
                        commodity: "USD".to_string(),
                        quantity: "21.32".to_string(),
                    }),
                },
            ],
            tags: vec![("bankId".to_string(), "FIT123".to_string())],
            extracted_by: Some("chase-driver:1.0".to_string()),
            reconciled: None,
            reconciled_postings: Vec::new(),
        };

        let formatted = format_entry(&entry);
        assert!(formatted.contains("2024-02-15  * SHELL OIL 12345"));
        assert!(formatted.contains("; id: abc-123"));
        assert!(formatted.contains("; evidence: 2024-02-17-transactions.csv:12:1"));
        assert!(formatted.contains("; bankId: FIT123"));

        let parsed = parse_journal(&formatted).unwrap();
        assert_eq!(parsed.len(), 1);
        let p = &parsed[0];
        assert_eq!(p.id, "abc-123");
        assert_eq!(p.date, "2024-02-15");
        assert_eq!(p.status, EntryStatus::Cleared);
        assert_eq!(p.description, "SHELL OIL 12345");
        assert_eq!(p.evidence.len(), 1);
        assert_eq!(p.postings.len(), 2);
        assert_eq!(p.tags.len(), 1);
        assert_eq!(p.tags[0], ("bankId".to_string(), "FIT123".to_string()));
    }

    #[test]
    fn write_and_read_journal() {
        let root = temp_dir("write-read");
        let entries = vec![
            AccountEntry::new(
                "2024-01-01".to_string(),
                EntryStatus::Pending,
                "Test pending".to_string(),
                vec!["doc.csv:1:1".to_string()],
                vec![
                    EntryPosting {
                        account: "Assets:Checking".to_string(),
                        amount: Some(SimpleAmount {
                            commodity: "USD".to_string(),
                            quantity: "-10.00".to_string(),
                        }),
                    },
                    EntryPosting {
                        account: "Equity:Unreconciled:Checking".to_string(),
                        amount: None,
                    },
                ],
            ),
            AccountEntry::new(
                "2024-01-02".to_string(),
                EntryStatus::Cleared,
                "Test cleared".to_string(),
                vec!["doc.csv:2:1".to_string()],
                vec![
                    EntryPosting {
                        account: "Assets:Checking".to_string(),
                        amount: Some(SimpleAmount {
                            commodity: "USD".to_string(),
                            quantity: "-20.00".to_string(),
                        }),
                    },
                    EntryPosting {
                        account: "Equity:Unreconciled:Checking".to_string(),
                        amount: None,
                    },
                ],
            ),
        ];

        write_journal(&root, "test-acct", &entries).unwrap();
        let read_back = read_journal(&root, "test-acct").unwrap();
        assert_eq!(read_back.len(), 2);
        assert_eq!(read_back[0].description, "Test pending");
        assert_eq!(read_back[0].status, EntryStatus::Pending);
        assert_eq!(read_back[1].description, "Test cleared");
        assert_eq!(read_back[1].status, EntryStatus::Cleared);

        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn append_entry_creates_file() {
        let root = temp_dir("append");
        let entry = AccountEntry::new(
            "2024-03-01".to_string(),
            EntryStatus::Unmarked,
            "New entry".to_string(),
            vec!["doc.csv:1:1".to_string()],
            vec![
                EntryPosting {
                    account: "Assets:Cash".to_string(),
                    amount: Some(SimpleAmount {
                        commodity: "USD".to_string(),
                        quantity: "50.00".to_string(),
                    }),
                },
                EntryPosting {
                    account: "Equity:Unreconciled:Cash".to_string(),
                    amount: None,
                },
            ],
        );

        append_entry(&root, "new-acct", &entry).unwrap();
        let read_back = read_journal(&root, "new-acct").unwrap();
        assert_eq!(read_back.len(), 1);
        assert_eq!(read_back[0].description, "New entry");

        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn pending_status_uses_exclamation() {
        let entry = AccountEntry::new(
            "2024-01-01".to_string(),
            EntryStatus::Pending,
            "Pending txn".to_string(),
            vec![],
            vec![
                EntryPosting {
                    account: "A".to_string(),
                    amount: Some(SimpleAmount {
                        commodity: "USD".to_string(),
                        quantity: "1".to_string(),
                    }),
                },
                EntryPosting {
                    account: "B".to_string(),
                    amount: None,
                },
            ],
        );
        let formatted = format_entry(&entry);
        assert!(formatted.contains("! Pending txn"));
    }

    #[test]
    fn read_nonexistent_returns_empty() {
        let root = temp_dir("nonexist");
        let entries = read_journal(&root, "no-such-account").unwrap();
        assert!(entries.is_empty());
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn reconciled_posting_round_trip() {
        let mut entry = AccountEntry::new(
            "2024-01-01".to_string(),
            EntryStatus::Cleared,
            "Multi-posting".to_string(),
            vec!["doc.csv:1:1".to_string()],
            vec![
                EntryPosting {
                    account: "Assets:Checking".to_string(),
                    amount: Some(SimpleAmount {
                        commodity: "USD".to_string(),
                        quantity: "-50.00".to_string(),
                    }),
                },
                EntryPosting {
                    account: "Equity:Unreconciled:Venmo".to_string(),
                    amount: Some(SimpleAmount {
                        commodity: "USD".to_string(),
                        quantity: "50.00".to_string(),
                    }),
                },
            ],
        );
        entry
            .reconciled_postings
            .push((0, "general.journal:gl-txn-1".to_string()));

        let formatted = format_entry(&entry);
        assert!(formatted.contains("reconciled-posting-0: general.journal:gl-txn-1"));

        let parsed = parse_journal(&formatted).unwrap();
        assert_eq!(parsed[0].reconciled_postings.len(), 1);
        assert_eq!(parsed[0].reconciled_postings[0].0, 0);
        assert_eq!(
            parsed[0].reconciled_postings[0].1,
            "general.journal:gl-txn-1"
        );
    }
}
