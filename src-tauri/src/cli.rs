use clap::{Args, Parser, Subcommand};
use std::error::Error;
use std::io::Read;
use std::path::PathBuf;
use tauri::Manager;

#[derive(Parser)]
#[command(name = "refreshmint", version = crate::version::APP_VERSION)]
pub struct Cli {
    #[command(subcommand)]
    command: Option<Commands>,
}

#[derive(Subcommand)]
enum Commands {
    New(NewArgs),
    Gl(GlArgs),
}

#[derive(Args)]
struct NewArgs {
    #[arg(long)]
    ledger: Option<PathBuf>,
}

#[derive(Args)]
struct GlArgs {
    #[command(subcommand)]
    command: GlCommand,
}

#[derive(Subcommand)]
enum GlCommand {
    Add(AddArgs),
}

#[derive(Args)]
struct AddArgs {
    #[arg(long)]
    ledger: Option<PathBuf>,
    #[arg(
        long,
        value_name = "PATH",
        conflicts_with_all = ["date", "description", "comment", "posting"],
        help = "Read raw transaction text from PATH ('-' for stdin)."
    )]
    raw: Option<PathBuf>,
    #[arg(long, required_unless_present = "raw")]
    date: Option<String>,
    #[arg(long)]
    description: Option<String>,
    #[arg(long)]
    comment: Option<String>,
    #[arg(
        long,
        value_name = "POSTING",
        required_unless_present = "raw",
        help = "Posting as account|amount|comment. Amount and comment are optional."
    )]
    posting: Vec<String>,
}

pub fn run(context: tauri::Context<tauri::Wry>) -> Result<(), Box<dyn Error>> {
    let cli = Cli::parse();
    match cli.command {
        Some(Commands::New(args)) => run_new(args, context),
        Some(Commands::Gl(args)) => run_gl(args, context),
        None => crate::run_with_context(context),
    }
}

fn run_new(args: NewArgs, context: tauri::Context<tauri::Wry>) -> Result<(), Box<dyn Error>> {
    match args.ledger {
        Some(path) => run_new_with_ledger_path(path),
        None => {
            let target_dir = default_ledger_dir(context)?;
            crate::ledger::new_ledger_at_dir(&target_dir)?;
            Ok(())
        }
    }
}

fn run_new_with_ledger_path(path: PathBuf) -> Result<(), Box<dyn Error>> {
    let target_dir = crate::ledger::ensure_refreshmint_extension(path)?;
    crate::ledger::new_ledger_at_dir(&target_dir)?;
    Ok(())
}

fn run_gl(args: GlArgs, context: tauri::Context<tauri::Wry>) -> Result<(), Box<dyn Error>> {
    match args.command {
        GlCommand::Add(add_args) => run_gl_add(add_args, context),
    }
}

fn run_gl_add(args: AddArgs, context: tauri::Context<tauri::Wry>) -> Result<(), Box<dyn Error>> {
    let ledger_dir = match args.ledger.as_ref() {
        Some(path) => crate::ledger::ensure_refreshmint_extension(path.clone())?,
        None => default_ledger_dir(context)?,
    };

    run_gl_add_with_dir(args, ledger_dir)
}

fn run_gl_add_with_dir(args: AddArgs, ledger_dir: PathBuf) -> Result<(), Box<dyn Error>> {
    let AddArgs {
        ledger: _,
        date,
        description,
        comment,
        posting,
        raw,
    } = args;

    if let Some(raw_path) = raw {
        let transaction = read_raw_transaction(&raw_path)?;
        if transaction.trim().is_empty() {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                "raw transaction is empty",
            )
            .into());
        }
        crate::ledger_add::add_transaction_text(&ledger_dir, &transaction)?;
        return Ok(());
    }

    let date = date.ok_or_else(|| {
        std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            "date is required unless --raw is provided",
        )
    })?;
    let description = description.unwrap_or_default();
    if posting.len() < 2 {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            "at least two --posting entries are required",
        )
        .into());
    }

    let postings = posting
        .iter()
        .map(|entry| parse_posting(entry))
        .collect::<Result<Vec<_>, Box<dyn Error>>>()?;

    let transaction = crate::ledger_add::NewTransaction {
        date,
        description,
        comment,
        postings,
    };

    crate::ledger_add::add_transaction_to_ledger(&ledger_dir, transaction)?;
    Ok(())
}

fn read_raw_transaction(path: &PathBuf) -> Result<String, Box<dyn Error>> {
    if path.as_os_str() == "-" {
        let mut buffer = String::new();
        std::io::stdin().read_to_string(&mut buffer)?;
        return Ok(buffer);
    }
    Ok(std::fs::read_to_string(path)?)
}

fn parse_posting(input: &str) -> Result<crate::ledger_add::NewPosting, Box<dyn Error>> {
    let mut parts = input.splitn(3, '|');
    let account = parts.next().unwrap_or("").trim();
    let amount = parts.next().unwrap_or("").trim();
    let comment = parts.next().unwrap_or("").trim();

    if account.is_empty() {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            "posting account cannot be empty",
        )
        .into());
    }

    Ok(crate::ledger_add::NewPosting {
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
    })
}

fn default_ledger_dir(context: tauri::Context<tauri::Wry>) -> Result<PathBuf, Box<dyn Error>> {
    let app = tauri::Builder::default().build(context)?;
    let documents_dir = app.path().document_dir()?;
    Ok(crate::ledger::default_ledger_dir_from_documents(
        documents_dir,
    ))
}

#[cfg(test)]
mod tests {
    use super::{run_gl_add_with_dir, run_new_with_ledger_path, AddArgs};
    use crate::ledger::ensure_refreshmint_extension;
    use serde_json::Value;
    use std::fs;
    use std::path::{Path, PathBuf};
    use std::process::Command;
    use std::time::{SystemTime, UNIX_EPOCH};

    #[test]
    fn ensure_refreshmint_extension_replaces_or_adds() {
        let no_extension = PathBuf::from("ledger");
        assert_eq!(
            expect_ok(ensure_refreshmint_extension(no_extension), "no extension"),
            PathBuf::from("ledger.refreshmint")
        );

        let other_extension = PathBuf::from("ledger.journal");
        assert_eq!(
            expect_ok(
                ensure_refreshmint_extension(other_extension),
                "other extension"
            ),
            PathBuf::from("ledger.refreshmint")
        );

        let already_refreshmint = PathBuf::from("ledger.refreshmint");
        assert_eq!(
            expect_ok(
                ensure_refreshmint_extension(already_refreshmint),
                "refreshmint extension"
            ),
            PathBuf::from("ledger.refreshmint")
        );
    }

    #[test]
    fn ensure_refreshmint_extension_rejects_empty_path() {
        let empty = PathBuf::from("");
        assert!(ensure_refreshmint_extension(empty).is_err());
    }

    #[test]
    fn new_command_creates_ledger_dir_and_git_repo() {
        if Command::new("git").arg("--version").status().is_err() {
            return;
        }

        let base_dir = create_temp_dir();
        let ledger_path = base_dir.join("ledger.journal");

        if let Err(err) = run_new_with_ledger_path(ledger_path) {
            panic!("run_new_with_ledger_path failed: {err}");
        }

        let ledger_dir = base_dir.join("ledger.refreshmint");
        if !ledger_dir.is_dir() {
            panic!("ledger directory was not created");
        }

        let refreshmint_json = ledger_dir.join("refreshmint.json");
        if !refreshmint_json.is_file() {
            panic!("refreshmint.json was not created");
        }

        let journal_path = ledger_dir.join("general.journal");
        if !journal_path.is_file() {
            panic!("general.journal was not created");
        }

        let json_contents = match fs::read_to_string(&refreshmint_json) {
            Ok(contents) => contents,
            Err(err) => {
                panic!("failed to read refreshmint.json: {err}");
            }
        };
        let json: Value = match serde_json::from_str(&json_contents) {
            Ok(json) => json,
            Err(err) => {
                panic!("failed to parse refreshmint.json: {err}");
            }
        };
        let version = match json.get("version").and_then(Value::as_str) {
            Some(version) => version,
            None => {
                panic!("refreshmint.json missing version");
            }
        };
        if version != crate::version::APP_VERSION {
            panic!(
                "refreshmint.json version {version} does not match {}",
                crate::version::APP_VERSION
            );
        }

        if !ledger_dir.join(".git").is_dir() {
            panic!(".git was not created");
        }

        let commit_subject = match git_output(&ledger_dir, &["log", "-1", "--pretty=%s"]) {
            Ok(output) => output,
            Err(err) => {
                panic!("git log failed: {err}");
            }
        };
        if commit_subject.trim() != "Initial commit" {
            panic!("unexpected git commit subject: {commit_subject}");
        }

        if let Err(err) = fs::remove_dir_all(&base_dir) {
            panic!("failed to clean up temp dir: {err}");
        }
    }

    #[test]
    fn gl_add_appends_transaction() {
        if Command::new("git").arg("--version").status().is_err() {
            return;
        }
        if Command::new("hledger").arg("--version").status().is_err() {
            return;
        }

        let base_dir = create_temp_dir();
        let ledger_path = base_dir.join("ledger.refreshmint");

        if let Err(err) = run_new_with_ledger_path(ledger_path.clone()) {
            panic!("run_new_with_ledger_path failed: {err}");
        }

        let args = AddArgs {
            ledger: Some(ledger_path.clone()),
            raw: None,
            date: Some("2025-01-01".to_string()),
            description: Some("Test transaction".to_string()),
            comment: Some("tag:test".to_string()),
            posting: vec![
                "Assets:Checking|10 USD|".to_string(),
                "Expenses:Food||note:snack".to_string(),
            ],
        };

        if let Err(err) = run_gl_add_with_dir(args, ledger_path.clone()) {
            panic!("run_gl_add_with_dir failed: {err}");
        }

        let journal_path = ledger_path.join("general.journal");
        let contents = fs::read_to_string(&journal_path).unwrap_or_else(|err| {
            panic!("failed to read general.journal: {err}");
        });

        if !contents.contains("2025-01-01  Test transaction  ; tag:test") {
            panic!("journal missing transaction header: {contents}");
        }
        if !contents.contains("  Assets:Checking  10 USD") {
            panic!("journal missing first posting: {contents}");
        }
        if !contents.contains("  Expenses:Food  ; note:snack") {
            panic!("journal missing second posting: {contents}");
        }

        let commit_subject = match git_output(&ledger_path, &["log", "-1", "--pretty=%s"]) {
            Ok(output) => output,
            Err(err) => {
                panic!("git log failed: {err}");
            }
        };
        if commit_subject.trim() != "Add transaction 2025-01-01 Test transaction" {
            panic!("unexpected git commit subject: {commit_subject}");
        }

        if let Err(err) = fs::remove_dir_all(&base_dir) {
            panic!("failed to clean up temp dir: {err}");
        }
    }

    #[test]
    fn gl_add_raw_appends_transaction() {
        if Command::new("git").arg("--version").status().is_err() {
            return;
        }
        if Command::new("hledger").arg("--version").status().is_err() {
            return;
        }

        let base_dir = create_temp_dir();
        let ledger_path = base_dir.join("ledger.refreshmint");

        if let Err(err) = run_new_with_ledger_path(ledger_path.clone()) {
            panic!("run_new_with_ledger_path failed: {err}");
        }

        let raw_path = base_dir.join("txn.journal");
        let raw = "; precomment\n2025-02-01=2025-02-03 * (INV-1) Coffee ; tag:food\n    Assets:Cash  -5 USD\n    Expenses:Food  5 USD ; note:snack\n";
        if let Err(err) = fs::write(&raw_path, raw) {
            panic!("failed to write raw transaction: {err}");
        }

        let args = AddArgs {
            ledger: Some(ledger_path.clone()),
            raw: Some(raw_path.clone()),
            date: None,
            description: None,
            comment: None,
            posting: Vec::new(),
        };

        if let Err(err) = run_gl_add_with_dir(args, ledger_path.clone()) {
            panic!("run_gl_add_with_dir failed: {err}");
        }

        let journal_path = ledger_path.join("general.journal");
        let contents = fs::read_to_string(&journal_path).unwrap_or_else(|err| {
            panic!("failed to read general.journal: {err}");
        });

        if !contents.contains("2025-02-01=2025-02-03 * (INV-1) Coffee") {
            panic!("journal missing raw transaction header: {contents}");
        }
        if !contents.contains("    Expenses:Food  5 USD ; note:snack") {
            panic!("journal missing raw posting: {contents}");
        }

        let commit_subject = match git_output(&ledger_path, &["log", "-1", "--pretty=%s"]) {
            Ok(output) => output,
            Err(err) => {
                panic!("git log failed: {err}");
            }
        };
        if commit_subject.trim() != "Add transaction 2025-02-01=2025-02-03 * (INV-1) Coffee" {
            panic!("unexpected git commit subject: {commit_subject}");
        }

        if let Err(err) = fs::remove_dir_all(&base_dir) {
            panic!("failed to clean up temp dir: {err}");
        }
    }

    fn expect_ok<T, E: std::fmt::Display>(result: Result<T, E>, label: &str) -> T {
        match result {
            Ok(value) => value,
            Err(err) => {
                panic!("expected Ok for {label}, got error: {err}");
            }
        }
    }

    fn create_temp_dir() -> PathBuf {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|duration| duration.as_nanos())
            .unwrap_or(0);
        let dir_name = format!("refreshmint-test-{}-{nanos}", std::process::id());
        let mut dir = std::env::temp_dir();
        dir.push(dir_name);
        if let Err(err) = fs::create_dir(&dir) {
            panic!("failed to create temp dir: {err}");
        }
        dir
    }

    fn git_output(dir: &Path, args: &[&str]) -> Result<String, std::io::Error> {
        let output = Command::new("git")
            .args(args)
            .current_dir(dir)
            .env("GIT_CONFIG_GLOBAL", crate::ledger::NULL_DEVICE)
            .env("GIT_CONFIG_SYSTEM", crate::ledger::NULL_DEVICE)
            .env("GIT_CONFIG_NOSYSTEM", "1")
            .env_remove("GIT_DIR")
            .env_remove("GIT_WORK_TREE")
            .env_remove("GIT_INDEX_FILE")
            .env_remove("GIT_COMMON_DIR")
            .env_remove("GIT_OBJECT_DIRECTORY")
            .env_remove("GIT_ALTERNATE_OBJECT_DIRECTORIES")
            .env_remove("GIT_QUARANTINE_PATH")
            .env("GIT_AUTHOR_NAME", crate::ledger::GIT_USER_NAME)
            .env("GIT_AUTHOR_EMAIL", crate::ledger::GIT_USER_EMAIL)
            .env("GIT_COMMITTER_NAME", crate::ledger::GIT_USER_NAME)
            .env("GIT_COMMITTER_EMAIL", crate::ledger::GIT_USER_EMAIL)
            .output()?;

        if output.status.success() {
            Ok(String::from_utf8_lossy(&output.stdout).to_string())
        } else {
            Err(std::io::Error::other(
                String::from_utf8_lossy(&output.stderr).to_string(),
            ))
        }
    }
}
