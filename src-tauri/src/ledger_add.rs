use crate::ledger_open::LedgerView;
use serde::Deserialize;
use std::fs::OpenOptions;
use std::io::{self, Write};
use std::path::Path;
use std::process::{Command, Stdio};

#[derive(Debug, Deserialize)]
pub struct NewTransaction {
    pub date: String,
    pub description: String,
    pub comment: Option<String>,
    pub postings: Vec<NewPosting>,
}

#[derive(Debug, Deserialize)]
pub struct NewPosting {
    pub account: String,
    pub amount: Option<String>,
    pub comment: Option<String>,
}

struct NormalizedPosting {
    account: String,
    amount: Option<String>,
    comment: Option<String>,
}

fn prepare_ledger(ledger_dir: &Path) -> Result<std::path::PathBuf, Box<dyn std::error::Error>> {
    crate::ledger::require_refreshmint_extension(ledger_dir)?;
    if !ledger_dir.is_dir() {
        return Err(io::Error::new(io::ErrorKind::NotFound, "ledger directory not found").into());
    }
    let config = crate::ledger::read_refreshmint_config(ledger_dir)?;
    if config.version != crate::version::APP_VERSION {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            format!(
                "ledger version {} does not match app version {}",
                config.version,
                crate::version::APP_VERSION
            ),
        )
        .into());
    }

    let journal_path = ledger_dir.join("general.journal");
    if !journal_path.is_file() {
        return Err(io::Error::new(io::ErrorKind::NotFound, "general.journal not found").into());
    }

    Ok(journal_path)
}

pub fn add_transaction_to_ledger(
    ledger_dir: &Path,
    transaction: NewTransaction,
) -> Result<LedgerView, Box<dyn std::error::Error>> {
    let journal_path = prepare_ledger(ledger_dir)?;

    validate_single_line(&transaction.date, "date")?;
    validate_single_line(&transaction.description, "description")?;
    validate_single_line(transaction.comment.as_deref().unwrap_or(""), "comment")?;
    let postings = normalize_postings(&transaction.postings)?;
    for posting in &postings {
        validate_single_line(&posting.account, "account")?;
        if let Some(amount) = &posting.amount {
            validate_single_line(amount, "amount")?;
        }
        if let Some(comment) = &posting.comment {
            validate_single_line(comment, "posting comment")?;
        }
    }

    let serialized = serialize_transaction(
        &transaction.date,
        &transaction.description,
        transaction.comment.as_deref(),
        &postings,
    );
    let commit_message = transaction_commit_message(&transaction.date, &transaction.description);
    run_hledger_check(&serialized, &[], "transaction-only")?;
    run_hledger_check(&serialized, &[&journal_path], "journal-plus-transaction")?;
    append_transaction(&journal_path, &serialized)?;
    crate::ledger::commit_general_journal(ledger_dir, &commit_message)?;
    crate::ledger_open::open_ledger_dir(ledger_dir)
}

pub fn add_transaction_text(
    ledger_dir: &Path,
    transaction: &str,
) -> Result<LedgerView, Box<dyn std::error::Error>> {
    let journal_path = prepare_ledger(ledger_dir)?;
    let serialized = ensure_trailing_newline(transaction);
    run_hledger_check(&serialized, &[], "transaction-only")?;
    run_hledger_check(&serialized, &[&journal_path], "journal-plus-transaction")?;
    append_transaction(&journal_path, &serialized)?;
    let commit_message = transaction_commit_message_from_text(&serialized);
    crate::ledger::commit_general_journal(ledger_dir, &commit_message)?;
    crate::ledger_open::open_ledger_dir(ledger_dir)
}

pub fn validate_transaction_text(
    ledger_dir: &Path,
    transaction: &str,
) -> Result<(), Box<dyn std::error::Error>> {
    prepare_ledger(ledger_dir)?;
    let serialized = ensure_trailing_newline(transaction);
    run_hledger_check(&serialized, &[], "transaction-only")?;
    Ok(())
}

pub fn validate_transaction_only(
    ledger_dir: &Path,
    transaction: NewTransaction,
) -> Result<(), Box<dyn std::error::Error>> {
    prepare_ledger(ledger_dir)?;

    validate_single_line(&transaction.date, "date")?;
    validate_single_line(&transaction.description, "description")?;
    validate_single_line(transaction.comment.as_deref().unwrap_or(""), "comment")?;
    let postings = normalize_postings(&transaction.postings)?;
    for posting in &postings {
        validate_single_line(&posting.account, "account")?;
        if let Some(amount) = &posting.amount {
            validate_single_line(amount, "amount")?;
        }
        if let Some(comment) = &posting.comment {
            validate_single_line(comment, "posting comment")?;
        }
    }

    let serialized = serialize_transaction(
        &transaction.date,
        &transaction.description,
        transaction.comment.as_deref(),
        &postings,
    );
    run_hledger_check(&serialized, &[], "transaction-only")?;
    Ok(())
}

fn normalize_postings(postings: &[NewPosting]) -> io::Result<Vec<NormalizedPosting>> {
    let mut normalized = Vec::new();
    for posting in postings {
        let account = posting.account.trim();
        let amount = posting.amount.as_deref().unwrap_or("").trim();
        let comment = posting.comment.as_deref().unwrap_or("").trim();
        if account.is_empty() {
            if !amount.is_empty() || !comment.is_empty() {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidInput,
                    "account is required when entering amounts or notes",
                ));
            }
            continue;
        }
        normalized.push(NormalizedPosting {
            account: account.to_string(),
            amount: if amount.is_empty() {
                None
            } else {
                Some(amount.to_string())
            },
            comment: if comment.is_empty() {
                None
            } else {
                Some(comment.to_string())
            },
        });
    }

    if normalized.len() < 2 {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "transaction must include at least two postings",
        ));
    }
    let missing_amounts = normalized
        .iter()
        .filter(|posting| posting.amount.is_none())
        .count();
    if missing_amounts > 1 {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "only one posting may omit an amount",
        ));
    }

    Ok(normalized)
}

fn validate_single_line(value: &str, field: &str) -> io::Result<()> {
    if value.contains('\n') || value.contains('\r') {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            format!("{field} must be a single line"),
        ));
    }
    Ok(())
}

fn serialize_transaction(
    date: &str,
    description: &str,
    comment: Option<&str>,
    postings: &[NormalizedPosting],
) -> String {
    let mut lines = Vec::new();
    let date = date.trim();
    let description = description.trim();
    if description.is_empty() {
        lines.push(date.to_string());
    } else {
        lines.push(format!("{date}  {description}"));
    }
    if let Some(comment) = comment {
        let comment = comment.trim();
        if !comment.is_empty() {
            if let Some(header) = lines.pop() {
                lines.push(format!("{header}  ; {comment}"));
            }
        }
    }
    for posting in postings {
        let comment = posting.comment.as_deref().unwrap_or("").trim();
        if let Some(amount) = &posting.amount {
            if comment.is_empty() {
                lines.push(format!("  {}  {}", posting.account, amount));
            } else {
                lines.push(format!("  {}  {}  ; {}", posting.account, amount, comment));
            }
        } else if comment.is_empty() {
            lines.push(format!("  {}", posting.account));
        } else {
            lines.push(format!("  {}  ; {}", posting.account, comment));
        }
    }

    let mut serialized = lines.join("\n");
    serialized.push('\n');
    serialized
}

fn ensure_trailing_newline(transaction: &str) -> String {
    let mut serialized = transaction.to_string();
    if !serialized.ends_with('\n') {
        serialized.push('\n');
    }
    serialized
}

fn transaction_commit_message(date: &str, description: &str) -> String {
    let date = date.trim();
    let description = description.trim();
    if description.is_empty() {
        format!("Add transaction {date}")
    } else {
        format!("Add transaction {date} {description}")
    }
}

fn transaction_commit_message_from_text(transaction: &str) -> String {
    for line in transaction.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }
        if trimmed.starts_with(';') || trimmed.starts_with('#') {
            continue;
        }
        let header = trimmed.split(';').next().unwrap_or("").trim();
        if header.is_empty() {
            continue;
        }
        let summary = if header.chars().count() > 72 {
            header
                .chars()
                .take(72)
                .collect::<String>()
                .trim_end()
                .to_string()
        } else {
            header.to_string()
        };
        return format!("Add transaction {summary}");
    }
    "Add transaction".to_string()
}

fn run_hledger_check(transaction: &str, extra_files: &[&Path], context: &str) -> io::Result<()> {
    let mut cmd = Command::new(crate::binpath::hledger_path());
    cmd.arg("check");
    cmd.arg("--color=never");
    for path in extra_files {
        cmd.arg("-f");
        cmd.arg(path);
    }
    cmd.arg("-f");
    cmd.arg("-");
    cmd.env("GIT_CONFIG_GLOBAL", crate::ledger::NULL_DEVICE)
        .env("GIT_CONFIG_SYSTEM", crate::ledger::NULL_DEVICE)
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());

    let mut child = cmd.spawn()?;
    if let Some(mut stdin) = child.stdin.take() {
        stdin.write_all(transaction.as_bytes())?;
    }
    let output = child.wait_with_output()?;
    if output.status.success() {
        Ok(())
    } else {
        Err(io::Error::other(format!(
            "hledger check failed ({context}): {}",
            String::from_utf8_lossy(&output.stderr)
        )))
    }
}

fn append_transaction(journal_path: &Path, transaction: &str) -> io::Result<()> {
    let mut file = OpenOptions::new().append(true).open(journal_path)?;
    if file.metadata()?.len() > 0 {
        file.write_all(b"\n")?;
    }
    file.write_all(transaction.as_bytes())?;
    Ok(())
}
