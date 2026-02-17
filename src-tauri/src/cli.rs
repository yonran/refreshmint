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
    Extension(ExtensionArgs),
    Secret(SecretArgs),
    Scrape(ScrapeArgs),
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
struct ExtensionArgs {
    #[command(subcommand)]
    command: ExtensionCommand,
}

#[derive(Subcommand)]
enum ExtensionCommand {
    Load(ExtensionLoadArgs),
}

#[derive(Args)]
struct ExtensionLoadArgs {
    #[arg(value_name = "PATH")]
    source: PathBuf,
    #[arg(long)]
    ledger: Option<PathBuf>,
    #[arg(long, default_value_t = false)]
    replace: bool,
}

#[derive(Args)]
struct SecretArgs {
    #[command(subcommand)]
    command: SecretCommand,
}

#[derive(Subcommand)]
enum SecretCommand {
    Add(SecretAddArgs),
    Remove(SecretRemoveArgs),
    List(SecretListArgs),
}

#[derive(Args)]
struct SecretAddArgs {
    #[arg(long)]
    account: String,
    #[arg(long)]
    domain: String,
    #[arg(long)]
    name: String,
    #[arg(long)]
    value: String,
}

#[derive(Args)]
struct SecretRemoveArgs {
    #[arg(long)]
    account: String,
    #[arg(long)]
    domain: String,
    #[arg(long)]
    name: String,
}

#[derive(Args)]
struct SecretListArgs {
    #[arg(long)]
    account: String,
}

#[derive(Args)]
struct ScrapeArgs {
    #[arg(long)]
    account: String,
    #[arg(long)]
    extension: String,
    #[arg(long)]
    ledger: Option<PathBuf>,
    #[arg(long)]
    profile: Option<PathBuf>,
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
        Some(Commands::Extension(args)) => run_extension(args, context),
        Some(Commands::Secret(args)) => run_secret(args),
        Some(Commands::Scrape(args)) => run_scrape(args, context),
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

fn run_extension(
    args: ExtensionArgs,
    context: tauri::Context<tauri::Wry>,
) -> Result<(), Box<dyn Error>> {
    match args.command {
        ExtensionCommand::Load(load_args) => run_extension_load(load_args, context),
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

fn run_secret(args: SecretArgs) -> Result<(), Box<dyn Error>> {
    match args.command {
        SecretCommand::Add(a) => {
            let store = crate::secret::SecretStore::new(a.account);
            store.set(&a.domain, &a.name, &a.value)?;
            eprintln!("Secret stored.");
            Ok(())
        }
        SecretCommand::Remove(a) => {
            let store = crate::secret::SecretStore::new(a.account);
            store.delete(&a.domain, &a.name)?;
            eprintln!("Secret removed.");
            Ok(())
        }
        SecretCommand::List(a) => {
            let account_name = a.account.clone();
            let store = crate::secret::SecretStore::new(a.account);
            let entries = store.list()?;
            if entries.is_empty() {
                println!("No secrets stored for account '{account_name}'.");
            } else {
                for (domain, name) in &entries {
                    println!("{domain}/{name}");
                }
            }
            Ok(())
        }
    }
}

fn run_extension_load(
    args: ExtensionLoadArgs,
    context: tauri::Context<tauri::Wry>,
) -> Result<(), Box<dyn Error>> {
    let ledger_dir = match args.ledger.as_ref() {
        Some(path) => crate::ledger::ensure_refreshmint_extension(path.clone())?,
        None => default_ledger_dir(context)?,
    };

    run_extension_load_with_dir(args, ledger_dir)?;
    Ok(())
}

fn run_extension_load_with_dir(
    args: ExtensionLoadArgs,
    ledger_dir: PathBuf,
) -> Result<String, Box<dyn Error>> {
    crate::ledger::require_refreshmint_extension(&ledger_dir)?;
    let loaded =
        crate::extension::load_extension_from_source(&ledger_dir, &args.source, args.replace)?;
    println!("Loaded extension '{loaded}'.");
    Ok(loaded)
}

fn run_scrape(args: ScrapeArgs, context: tauri::Context<tauri::Wry>) -> Result<(), Box<dyn Error>> {
    let ledger_dir = match args.ledger.as_ref() {
        Some(path) => crate::ledger::ensure_refreshmint_extension(path.clone())?,
        None => default_ledger_dir(context)?,
    };

    let config = crate::scrape::ScrapeConfig {
        account: args.account,
        extension_name: args.extension,
        ledger_dir,
        profile_override: args.profile,
    };

    crate::scrape::run_scrape(config)
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
    use super::{
        run_extension_load_with_dir, run_gl_add_with_dir, run_new_with_ledger_path, AddArgs,
        ExtensionLoadArgs,
    };
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

        let commit_subject = match latest_commit_subject(&ledger_dir) {
            Ok(output) => output,
            Err(err) => {
                panic!("read latest commit failed: {err}");
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
    fn extension_load_command_copies_manifest_named_directory() {
        let base_dir = create_temp_dir();
        let ledger_path = base_dir.join("ledger.refreshmint");

        if let Err(err) = fs::create_dir_all(&ledger_path) {
            panic!("failed to create ledger directory: {err}");
        }

        let source_dir = base_dir.join("extension-src");
        if let Err(err) = fs::create_dir_all(&source_dir) {
            panic!("failed to create source directory: {err}");
        }
        if let Err(err) = fs::write(source_dir.join("manifest.json"), r#"{"name":"bank-sync"}"#) {
            panic!("failed to write manifest.json: {err}");
        }
        if let Err(err) = fs::write(source_dir.join("driver.mjs"), "// driver") {
            panic!("failed to write driver.mjs: {err}");
        }

        let args = ExtensionLoadArgs {
            source: source_dir.clone(),
            ledger: Some(ledger_path.clone()),
            replace: false,
        };

        let loaded = match run_extension_load_with_dir(args, ledger_path.clone()) {
            Ok(name) => name,
            Err(err) => {
                panic!("run_extension_load_with_dir failed: {err}");
            }
        };
        assert_eq!(loaded, "bank-sync");
        assert!(ledger_path
            .join("extensions")
            .join("bank-sync")
            .join("driver.mjs")
            .is_file());

        if let Err(err) = fs::remove_dir_all(&base_dir) {
            panic!("failed to clean up temp dir: {err}");
        }
    }

    #[test]
    fn gl_add_appends_transaction() {
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

        let commit_subject = match latest_commit_subject(&ledger_path) {
            Ok(output) => output,
            Err(err) => {
                panic!("read latest commit failed: {err}");
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

        let commit_subject = match latest_commit_subject(&ledger_path) {
            Ok(output) => output,
            Err(err) => {
                panic!("read latest commit failed: {err}");
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

    fn latest_commit_subject(dir: &Path) -> Result<String, std::io::Error> {
        let repo = git2::Repository::open(dir).map_err(|e| std::io::Error::other(e.to_string()))?;
        let head = repo
            .head()
            .map_err(|e| std::io::Error::other(e.to_string()))?;
        let commit = head
            .peel_to_commit()
            .map_err(|e| std::io::Error::other(e.to_string()))?;
        Ok(commit.summary().unwrap_or("").to_string())
    }
}
