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
    Login(LoginArgs),
    Debug(DebugArgs),
    Secret(SecretArgs),
    Scrape(ScrapeArgs),
    Account(AccountArgs),
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
struct LoginArgs {
    #[command(subcommand)]
    command: LoginCommand,
}

#[derive(Subcommand)]
enum LoginCommand {
    List(LoginListArgs),
    Create(LoginCreateArgs),
    SetExtension(LoginSetExtensionArgs),
    Delete(LoginDeleteArgs),
    SetAccount(LoginSetAccountArgs),
    RemoveAccount(LoginRemoveAccountArgs),
}

#[derive(Args)]
struct LoginListArgs {
    #[arg(long)]
    ledger: Option<PathBuf>,
}

#[derive(Args)]
struct LoginCreateArgs {
    #[arg(long, value_name = "NAME")]
    name: String,
    #[arg(long)]
    extension: Option<String>,
    #[arg(long)]
    ledger: Option<PathBuf>,
}

#[derive(Args)]
struct LoginSetExtensionArgs {
    #[arg(long, value_name = "NAME")]
    name: String,
    #[arg(long)]
    extension: String,
    #[arg(long)]
    ledger: Option<PathBuf>,
}

#[derive(Args)]
struct LoginDeleteArgs {
    #[arg(long, value_name = "NAME")]
    name: String,
    #[arg(long)]
    ledger: Option<PathBuf>,
}

#[derive(Args)]
struct LoginSetAccountArgs {
    #[arg(long, value_name = "NAME")]
    name: String,
    #[arg(long)]
    label: String,
    #[arg(long = "gl-account", value_name = "ACCOUNT")]
    gl_account: Option<String>,
    #[arg(long)]
    ledger: Option<PathBuf>,
}

#[derive(Args)]
struct LoginRemoveAccountArgs {
    #[arg(long, value_name = "NAME")]
    name: String,
    #[arg(long)]
    label: String,
    #[arg(long)]
    ledger: Option<PathBuf>,
}

#[derive(Args)]
struct DebugArgs {
    #[command(subcommand)]
    command: DebugCommand,
}

#[derive(Subcommand)]
enum DebugCommand {
    Start(DebugStartArgs),
    Exec(DebugExecArgs),
    Stop(DebugStopArgs),
}

#[derive(Args)]
struct DebugStartArgs {
    #[arg(long, alias = "account")]
    login: String,
    #[arg(long)]
    extension: String,
    #[arg(long)]
    ledger: Option<PathBuf>,
    #[arg(long)]
    profile: Option<PathBuf>,
    #[arg(long)]
    socket: Option<PathBuf>,
}

#[derive(Args)]
struct DebugExecArgs {
    #[arg(long)]
    socket: PathBuf,
    #[arg(
        long,
        value_name = "PATH",
        help = "Script path ('-' for stdin).",
        required_unless_present = "extension_dir",
        conflicts_with = "extension_dir"
    )]
    script: Option<PathBuf>,
    #[arg(
        long,
        value_name = "DIR",
        help = "Extension directory (loads driver.mjs and manifest secrets).",
        required_unless_present = "script",
        conflicts_with = "script"
    )]
    extension_dir: Option<PathBuf>,
    #[arg(
        long,
        value_name = "MESSAGE=VALUE",
        action = clap::ArgAction::Append,
        help = "Answer override for refreshmint.prompt(message). Repeat for multiple prompts."
    )]
    prompt: Vec<String>,
}

#[derive(Args)]
struct DebugStopArgs {
    #[arg(long)]
    socket: PathBuf,
}

#[derive(Args)]
struct SecretArgs {
    #[command(subcommand)]
    command: SecretCommand,
}

#[derive(Subcommand)]
enum SecretCommand {
    Add(SecretAddArgs),
    Reenter(SecretReenterArgs),
    Remove(SecretRemoveArgs),
    List(SecretListArgs),
}

#[derive(Args)]
struct SecretAddArgs {
    #[arg(long, alias = "account")]
    login: String,
    #[arg(long)]
    domain: String,
    #[arg(long)]
    name: String,
    #[arg(long)]
    value: String,
}

#[derive(Args)]
struct SecretReenterArgs {
    #[arg(long, alias = "account")]
    login: String,
    #[arg(long)]
    domain: String,
    #[arg(long)]
    name: String,
    #[arg(long)]
    value: String,
}

#[derive(Args)]
struct SecretRemoveArgs {
    #[arg(long, alias = "account")]
    login: String,
    #[arg(long)]
    domain: String,
    #[arg(long)]
    name: String,
}

#[derive(Args)]
struct SecretListArgs {
    #[arg(long, alias = "account")]
    login: String,
}

#[derive(Args)]
struct ScrapeArgs {
    #[arg(long, alias = "account")]
    login: String,
    #[arg(long)]
    extension: Option<String>,
    #[arg(long)]
    ledger: Option<PathBuf>,
    #[arg(long)]
    profile: Option<PathBuf>,
    #[arg(
        long,
        value_name = "MESSAGE=VALUE",
        action = clap::ArgAction::Append,
        help = "Answer override for refreshmint.prompt(message). Repeat for multiple prompts."
    )]
    prompt: Vec<String>,
}

#[derive(Args)]
struct AccountArgs {
    #[command(subcommand)]
    command: AccountCommand,
}

#[derive(Subcommand)]
enum AccountCommand {
    Documents(AccountDocumentsArgs),
    Extract(AccountExtractArgs),
    Journal(AccountJournalArgs),
    Unreconciled(AccountUnreconciledArgs),
    Reconcile(AccountReconcileArgs),
    Unreconcile(AccountUnreconcileArgs),
    Transfer(AccountTransferArgs),
}

#[derive(Args)]
struct AccountDocumentsArgs {
    #[arg(long, alias = "account")]
    login: String,
    #[arg(long)]
    label: String,
    #[arg(long)]
    ledger: Option<PathBuf>,
}

#[derive(Args)]
struct AccountExtractArgs {
    #[arg(long, alias = "account")]
    login: String,
    #[arg(long)]
    label: String,
    #[arg(long)]
    extension: Option<String>,
    #[arg(long)]
    ledger: Option<PathBuf>,
    #[arg(
        long = "document",
        value_name = "FILENAME",
        action = clap::ArgAction::Append,
        help = "Document filename to extract. Repeat for multiple files. Defaults to all account documents."
    )]
    document: Vec<String>,
}

#[derive(Args)]
struct AccountJournalArgs {
    #[arg(long, alias = "account")]
    login: String,
    #[arg(long)]
    label: String,
    #[arg(long)]
    ledger: Option<PathBuf>,
}

#[derive(Args)]
struct AccountUnreconciledArgs {
    #[arg(long, alias = "account")]
    login: String,
    #[arg(long)]
    label: String,
    #[arg(long)]
    ledger: Option<PathBuf>,
}

#[derive(Args)]
struct AccountReconcileArgs {
    #[arg(long, alias = "account")]
    login: String,
    #[arg(long)]
    label: String,
    #[arg(long, value_name = "ENTRY_ID")]
    entry_id: String,
    #[arg(long, value_name = "ACCOUNT")]
    counterpart_account: String,
    #[arg(long, value_name = "INDEX")]
    posting_index: Option<usize>,
    #[arg(long)]
    ledger: Option<PathBuf>,
}

#[derive(Args)]
struct AccountUnreconcileArgs {
    #[arg(long, alias = "account")]
    login: String,
    #[arg(long)]
    label: String,
    #[arg(long, value_name = "ENTRY_ID")]
    entry_id: String,
    #[arg(long, value_name = "INDEX")]
    posting_index: Option<usize>,
    #[arg(long)]
    ledger: Option<PathBuf>,
}

#[derive(Args)]
struct AccountTransferArgs {
    #[arg(long)]
    account1: String,
    #[arg(long, value_name = "ENTRY_ID")]
    entry_id1: String,
    #[arg(long)]
    account2: String,
    #[arg(long, value_name = "ENTRY_ID")]
    entry_id2: String,
    #[arg(long)]
    ledger: Option<PathBuf>,
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
        Some(Commands::Login(args)) => run_login(args, context),
        Some(Commands::Debug(args)) => run_debug(args, context),
        Some(Commands::Secret(args)) => run_secret(args),
        Some(Commands::Scrape(args)) => run_scrape(args, context),
        Some(Commands::Account(args)) => run_account(args, context),
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

fn run_login(args: LoginArgs, context: tauri::Context<tauri::Wry>) -> Result<(), Box<dyn Error>> {
    match args.command {
        LoginCommand::List(list_args) => run_login_list(list_args, context),
        LoginCommand::Create(create_args) => run_login_create(create_args, context),
        LoginCommand::SetExtension(set_args) => run_login_set_extension(set_args, context),
        LoginCommand::Delete(delete_args) => run_login_delete(delete_args, context),
        LoginCommand::SetAccount(set_args) => run_login_set_account(set_args, context),
        LoginCommand::RemoveAccount(remove_args) => run_login_remove_account(remove_args, context),
    }
}

fn run_debug(args: DebugArgs, context: tauri::Context<tauri::Wry>) -> Result<(), Box<dyn Error>> {
    match args.command {
        DebugCommand::Start(start_args) => run_debug_start(start_args, context),
        DebugCommand::Exec(exec_args) => run_debug_exec(exec_args),
        DebugCommand::Stop(stop_args) => run_debug_stop(stop_args),
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

fn read_text_input(path: &PathBuf) -> Result<String, Box<dyn Error>> {
    if path.as_os_str() == "-" {
        let mut buffer = String::new();
        std::io::stdin().read_to_string(&mut buffer)?;
        return Ok(buffer);
    }
    Ok(std::fs::read_to_string(path)?)
}

fn read_raw_transaction(path: &PathBuf) -> Result<String, Box<dyn Error>> {
    read_text_input(path)
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
            let login = require_secret_field("login", &a.login)?;
            let domain = require_secret_field("domain", &a.domain)?;
            let name = require_secret_field("name", &a.name)?;

            let store = crate::secret::SecretStore::new(format!("login/{login}"));
            store.set(&domain, &name, &a.value)?;
            eprintln!("Secret stored.");
            Ok(())
        }
        SecretCommand::Reenter(a) => {
            let login = require_secret_field("login", &a.login)?;
            let domain = require_secret_field("domain", &a.domain)?;
            let name = require_secret_field("name", &a.name)?;

            let store = crate::secret::SecretStore::new(format!("login/{login}"));
            store.set(&domain, &name, &a.value)?;
            eprintln!("Secret re-entered.");
            Ok(())
        }
        SecretCommand::Remove(a) => {
            let login = require_secret_field("login", &a.login)?;
            let domain = require_secret_field("domain", &a.domain)?;
            let name = require_secret_field("name", &a.name)?;

            let store = crate::secret::SecretStore::new(format!("login/{login}"));
            store.delete(&domain, &name)?;
            eprintln!("Secret removed.");
            Ok(())
        }
        SecretCommand::List(a) => {
            let login = require_secret_field("login", &a.login)?;
            let login_name = login.clone();
            let store = crate::secret::SecretStore::new(format!("login/{login}"));
            let entries = store.list()?;
            if entries.is_empty() {
                println!("No secrets stored for login '{login_name}'.");
            } else {
                for (domain, name) in &entries {
                    println!("{domain}/{name}");
                }
            }
            Ok(())
        }
    }
}

fn require_secret_field(field_name: &str, value: &str) -> Result<String, Box<dyn Error>> {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            format!("{field_name} is required"),
        )
        .into());
    }
    Ok(trimmed.to_string())
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

fn run_debug_start(
    args: DebugStartArgs,
    context: tauri::Context<tauri::Wry>,
) -> Result<(), Box<dyn Error>> {
    let ledger_dir = match args.ledger.as_ref() {
        Some(path) => crate::ledger::ensure_refreshmint_extension(path.clone())?,
        None => default_ledger_dir(context)?,
    };
    crate::ledger::require_refreshmint_extension(&ledger_dir)?;

    let login_name = args.login.trim().to_string();
    if login_name.is_empty() {
        return Err(
            std::io::Error::new(std::io::ErrorKind::InvalidInput, "login is required").into(),
        );
    }
    let extension = args.extension.trim().to_string();
    if extension.is_empty() {
        return Err(
            std::io::Error::new(std::io::ErrorKind::InvalidInput, "extension is required").into(),
        );
    }

    let socket = match args.socket {
        Some(path) => path,
        None => crate::scrape::debug::default_debug_socket_path(&login_name)?,
    };
    let config = crate::scrape::debug::DebugStartConfig {
        login_name,
        extension_name: extension,
        ledger_dir,
        profile_override: args.profile,
        socket_path: Some(socket),
        prompt_requires_override: true,
    };
    crate::scrape::debug::run_debug_session(config)
}

fn run_debug_exec(args: DebugExecArgs) -> Result<(), Box<dyn Error>> {
    let prompt_overrides = parse_prompt_overrides(&args.prompt)?;
    let (script_source, declared_secrets) = if let Some(extension_dir) = args.extension_dir {
        if !extension_dir.is_dir() {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                format!(
                    "extension directory not found or not a directory: {}",
                    extension_dir.display()
                ),
            )
            .into());
        }
        let script_path = extension_dir.join("driver.mjs");
        let script_source = read_text_input(&script_path)?;
        let declared = crate::scrape::load_manifest_secret_declarations(&extension_dir).map_err(
            |err| -> Box<dyn Error> {
                std::io::Error::new(std::io::ErrorKind::InvalidData, err.to_string()).into()
            },
        )?;
        (script_source, Some(declared))
    } else {
        let script_path = args.script.ok_or_else(|| {
            std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                "either --script or --extension-dir is required",
            )
        })?;
        (read_text_input(&script_path)?, None)
    };

    if script_source.trim().is_empty() {
        return Err(
            std::io::Error::new(std::io::ErrorKind::InvalidInput, "script is empty").into(),
        );
    }
    crate::scrape::debug::exec_debug_script_with_options(
        &args.socket,
        &script_source,
        declared_secrets,
        Some(prompt_overrides),
        Some(true),
    )?;
    println!("Script executed.");
    Ok(())
}

fn run_debug_stop(args: DebugStopArgs) -> Result<(), Box<dyn Error>> {
    crate::scrape::debug::stop_debug_session(&args.socket)?;
    println!("Debug session stopped.");
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

fn run_login_list(
    args: LoginListArgs,
    context: tauri::Context<tauri::Wry>,
) -> Result<(), Box<dyn Error>> {
    let ledger_dir = resolve_cli_ledger_dir(args.ledger, context)?;
    crate::ledger::require_refreshmint_extension(&ledger_dir)?;
    for login in crate::login_config::list_logins(&ledger_dir) {
        println!("{login}");
    }
    Ok(())
}

fn run_login_create(
    args: LoginCreateArgs,
    context: tauri::Context<tauri::Wry>,
) -> Result<(), Box<dyn Error>> {
    let ledger_dir = resolve_cli_ledger_dir(args.ledger, context)?;
    crate::ledger::require_refreshmint_extension(&ledger_dir)?;
    let login_name = require_cli_field("name", &args.name)?;
    crate::login_config::validate_label(&login_name).map_err(std::io::Error::other)?;

    let config_path = crate::login_config::login_config_path(&ledger_dir, &login_name);
    if config_path.exists() {
        return Err(std::io::Error::new(
            std::io::ErrorKind::AlreadyExists,
            format!("login '{login_name}' already exists"),
        )
        .into());
    }

    let extension = args
        .extension
        .as_deref()
        .map(str::trim)
        .filter(|v| !v.is_empty());
    let config = crate::login_config::LoginConfig {
        extension: extension.map(ToOwned::to_owned),
        accounts: std::collections::BTreeMap::new(),
    };
    crate::login_config::write_login_config(&ledger_dir, &login_name, &config)
        .map_err(std::io::Error::other)?;
    println!("Created login '{login_name}'.");
    Ok(())
}

fn run_login_set_extension(
    args: LoginSetExtensionArgs,
    context: tauri::Context<tauri::Wry>,
) -> Result<(), Box<dyn Error>> {
    let ledger_dir = resolve_cli_ledger_dir(args.ledger, context)?;
    crate::ledger::require_refreshmint_extension(&ledger_dir)?;
    let login_name = require_cli_field("name", &args.name)?;
    let extension = args.extension.trim().to_string();

    let _lock = crate::login_config::acquire_login_lock(&ledger_dir, &login_name)
        .map_err(std::io::Error::other)?;
    let mut config = crate::login_config::read_login_config(&ledger_dir, &login_name);
    config.extension = if extension.is_empty() {
        None
    } else {
        Some(extension)
    };
    crate::login_config::write_login_config(&ledger_dir, &login_name, &config)
        .map_err(std::io::Error::other)?;
    println!("Updated extension for login '{login_name}'.");
    Ok(())
}

fn run_login_delete(
    args: LoginDeleteArgs,
    context: tauri::Context<tauri::Wry>,
) -> Result<(), Box<dyn Error>> {
    let ledger_dir = resolve_cli_ledger_dir(args.ledger, context)?;
    crate::ledger::require_refreshmint_extension(&ledger_dir)?;
    let login_name = require_cli_field("name", &args.name)?;
    crate::login_config::delete_login(&ledger_dir, &login_name).map_err(std::io::Error::other)?;
    println!("Deleted login '{login_name}'.");
    Ok(())
}

fn run_login_set_account(
    args: LoginSetAccountArgs,
    context: tauri::Context<tauri::Wry>,
) -> Result<(), Box<dyn Error>> {
    let ledger_dir = resolve_cli_ledger_dir(args.ledger, context)?;
    crate::ledger::require_refreshmint_extension(&ledger_dir)?;
    let login_name = require_cli_field("name", &args.name)?;
    let label = require_cli_field("label", &args.label)?;
    crate::login_config::validate_label(&label).map_err(std::io::Error::other)?;

    let gl_account = args
        .gl_account
        .as_deref()
        .map(str::trim)
        .filter(|v| !v.is_empty())
        .map(ToOwned::to_owned);

    let _lock = crate::login_config::acquire_login_lock(&ledger_dir, &login_name)
        .map_err(std::io::Error::other)?;
    if let Some(ref gl) = gl_account {
        crate::login_config::check_gl_account_uniqueness(&ledger_dir, &login_name, &label, gl)
            .map_err(std::io::Error::other)?;
    }

    let mut config = crate::login_config::read_login_config(&ledger_dir, &login_name);
    config.accounts.insert(
        label.clone(),
        crate::login_config::LoginAccountConfig { gl_account },
    );
    crate::login_config::write_login_config(&ledger_dir, &login_name, &config)
        .map_err(std::io::Error::other)?;
    println!("Updated label '{label}' for login '{login_name}'.");
    Ok(())
}

fn run_login_remove_account(
    args: LoginRemoveAccountArgs,
    context: tauri::Context<tauri::Wry>,
) -> Result<(), Box<dyn Error>> {
    let ledger_dir = resolve_cli_ledger_dir(args.ledger, context)?;
    crate::ledger::require_refreshmint_extension(&ledger_dir)?;
    let login_name = require_cli_field("name", &args.name)?;
    let label = require_cli_field("label", &args.label)?;
    let _lock = crate::login_config::acquire_login_lock(&ledger_dir, &login_name)
        .map_err(std::io::Error::other)?;
    crate::login_config::remove_login_account(&ledger_dir, &login_name, &label)
        .map_err(std::io::Error::other)?;
    println!("Removed label '{label}' from login '{login_name}'.");
    Ok(())
}

fn run_scrape(args: ScrapeArgs, context: tauri::Context<tauri::Wry>) -> Result<(), Box<dyn Error>> {
    let ledger_dir = match args.ledger.as_ref() {
        Some(path) => crate::ledger::ensure_refreshmint_extension(path.clone())?,
        None => default_ledger_dir(context)?,
    };

    let login_name = require_cli_field("login", &args.login)?;
    let extension_name = crate::login_config::resolve_login_extension(
        &ledger_dir,
        &login_name,
        args.extension.as_deref(),
    )
    .map_err(std::io::Error::other)?;

    let prompt_overrides = parse_prompt_overrides(&args.prompt)?;

    let config = crate::scrape::ScrapeConfig {
        login_name,
        extension_name,
        ledger_dir,
        profile_override: args.profile,
        prompt_overrides,
        prompt_requires_override: true,
    };

    crate::scrape::run_scrape(config)
}

#[derive(serde::Serialize)]
struct CliAccountJournalEntry {
    id: String,
    date: String,
    status: String,
    description: String,
    comment: String,
    evidence: Vec<String>,
    reconciled: Option<String>,
    #[serde(rename = "isTransfer")]
    is_transfer: bool,
}

fn run_account(
    args: AccountArgs,
    context: tauri::Context<tauri::Wry>,
) -> Result<(), Box<dyn Error>> {
    match args.command {
        AccountCommand::Documents(doc_args) => run_account_documents(doc_args, context),
        AccountCommand::Extract(extract_args) => run_account_extract(extract_args, context),
        AccountCommand::Journal(journal_args) => run_account_journal(journal_args, context),
        AccountCommand::Unreconciled(unreconciled_args) => {
            run_account_unreconciled(unreconciled_args, context)
        }
        AccountCommand::Reconcile(reconcile_args) => run_account_reconcile(reconcile_args, context),
        AccountCommand::Unreconcile(unreconcile_args) => {
            run_account_unreconcile(unreconcile_args, context)
        }
        AccountCommand::Transfer(transfer_args) => run_account_transfer(transfer_args, context),
    }
}

fn run_account_documents(
    args: AccountDocumentsArgs,
    context: tauri::Context<tauri::Wry>,
) -> Result<(), Box<dyn Error>> {
    let ledger_dir = resolve_cli_ledger_dir(args.ledger, context)?;
    crate::ledger::require_refreshmint_extension(&ledger_dir)?;
    let login_name = require_cli_field("login", &args.login)?;
    let label = require_cli_field("label", &args.label)?;
    crate::login_config::validate_label(&label).map_err(std::io::Error::other)?;
    let documents =
        crate::extract::list_documents_for_login_account(&ledger_dir, &login_name, &label)?;
    println!("{}", serde_json::to_string_pretty(&documents)?);
    Ok(())
}

fn run_account_extract(
    args: AccountExtractArgs,
    context: tauri::Context<tauri::Wry>,
) -> Result<(), Box<dyn Error>> {
    let ledger_dir = resolve_cli_ledger_dir(args.ledger, context)?;
    crate::ledger::require_refreshmint_extension(&ledger_dir)?;

    let login_name = require_cli_field("login", &args.login)?;
    let label = require_cli_field("label", &args.label)?;
    crate::login_config::validate_label(&label).map_err(std::io::Error::other)?;
    let extension_name = crate::login_config::resolve_login_extension(
        &ledger_dir,
        &login_name,
        args.extension.as_deref(),
    )
    .map_err(std::io::Error::other)?;
    let gl_account = resolve_login_account_gl_account_cli(&ledger_dir, &login_name, &label)?;

    let listed_documents = if args.document.is_empty() {
        crate::extract::list_documents_for_login_account(&ledger_dir, &login_name, &label)?
            .into_iter()
            .map(|d| d.filename)
            .collect::<Vec<_>>()
    } else {
        Vec::new()
    };
    let document_names = resolve_extraction_document_names(&args.document, listed_documents)?;

    if document_names.is_empty() {
        println!("No documents found for login '{login_name}' label '{label}'.");
        return Ok(());
    }

    let extraction = crate::extract::run_extraction_for_login_account(
        &ledger_dir,
        &login_name,
        &label,
        &gl_account,
        &extension_name,
        &document_names,
    )
    .map_err(|err| std::io::Error::other(err.to_string()))?;
    let journal_path =
        crate::account_journal::login_account_journal_path(&ledger_dir, &login_name, &label);
    let existing_entries = crate::account_journal::read_journal_at_path(&journal_path)?;

    let config = crate::dedup::DedupConfig::default();
    let mut all_updated = existing_entries;
    let mut new_count = 0usize;

    for doc_name in &extraction.document_names {
        let doc_txns: Vec<_> = extraction
            .proposed_transactions
            .iter()
            .filter(|t| {
                t.evidence_refs()
                    .iter()
                    .any(|e| evidence_ref_matches_document(e, doc_name))
            })
            .cloned()
            .collect();
        if doc_txns.is_empty() {
            continue;
        }

        let actions = crate::dedup::run_dedup(&all_updated, &doc_txns, doc_name, &config);
        new_count += actions
            .iter()
            .filter(|a| matches!(a.result, crate::dedup::DedupResult::New))
            .count();

        let default_account = all_updated
            .first()
            .and_then(|e| e.postings.first())
            .map(|p| p.account.clone())
            .unwrap_or_else(|| gl_account.clone());
        let unreconciled_equity = format!("Equity:Unreconciled:{login_name}:{label}");

        all_updated = crate::dedup::apply_dedup_actions_for_login_account(
            &ledger_dir,
            (&login_name, &label),
            all_updated,
            &actions,
            &default_account,
            &unreconciled_equity,
            Some(&format!("{extension_name}:latest")),
        )
        .map_err(|err| std::io::Error::other(err.to_string()))?;
    }

    crate::account_journal::write_journal_at_path(&journal_path, &all_updated)?;
    println!("Extraction complete. Added {new_count} new transaction(s).");
    Ok(())
}

fn run_account_journal(
    args: AccountJournalArgs,
    context: tauri::Context<tauri::Wry>,
) -> Result<(), Box<dyn Error>> {
    let ledger_dir = resolve_cli_ledger_dir(args.ledger, context)?;
    crate::ledger::require_refreshmint_extension(&ledger_dir)?;
    let login_name = require_cli_field("login", &args.login)?;
    let label = require_cli_field("label", &args.label)?;
    let journal_path =
        crate::account_journal::login_account_journal_path(&ledger_dir, &login_name, &label);
    let entries = crate::account_journal::read_journal_at_path(&journal_path)?;
    println!(
        "{}",
        serde_json::to_string_pretty(&map_entries_for_cli(entries))?
    );
    Ok(())
}

fn run_account_unreconciled(
    args: AccountUnreconciledArgs,
    context: tauri::Context<tauri::Wry>,
) -> Result<(), Box<dyn Error>> {
    let ledger_dir = resolve_cli_ledger_dir(args.ledger, context)?;
    crate::ledger::require_refreshmint_extension(&ledger_dir)?;
    let login_name = require_cli_field("login", &args.login)?;
    let label = require_cli_field("label", &args.label)?;
    let entries =
        crate::reconcile::get_unreconciled_login_account(&ledger_dir, &login_name, &label)
            .map_err(|err| std::io::Error::other(err.to_string()))?;
    println!(
        "{}",
        serde_json::to_string_pretty(&map_entries_for_cli(entries))?
    );
    Ok(())
}

fn run_account_reconcile(
    args: AccountReconcileArgs,
    context: tauri::Context<tauri::Wry>,
) -> Result<(), Box<dyn Error>> {
    let ledger_dir = resolve_cli_ledger_dir(args.ledger, context)?;
    crate::ledger::require_refreshmint_extension(&ledger_dir)?;
    let login_name = require_cli_field("login", &args.login)?;
    let label = require_cli_field("label", &args.label)?;
    let entry_id = require_cli_field("entry_id", &args.entry_id)?;
    let counterpart_account = require_cli_field("counterpart_account", &args.counterpart_account)?;
    let _ = resolve_login_account_gl_account_cli(&ledger_dir, &login_name, &label)?;
    let gl_txn_id = crate::reconcile::reconcile_login_account_entry(
        &ledger_dir,
        &login_name,
        &label,
        &entry_id,
        &counterpart_account,
        args.posting_index,
    )
    .map_err(|err| std::io::Error::other(err.to_string()))?;
    println!("{gl_txn_id}");
    Ok(())
}

fn run_account_unreconcile(
    args: AccountUnreconcileArgs,
    context: tauri::Context<tauri::Wry>,
) -> Result<(), Box<dyn Error>> {
    let ledger_dir = resolve_cli_ledger_dir(args.ledger, context)?;
    crate::ledger::require_refreshmint_extension(&ledger_dir)?;
    let login_name = require_cli_field("login", &args.login)?;
    let label = require_cli_field("label", &args.label)?;
    let entry_id = require_cli_field("entry_id", &args.entry_id)?;
    crate::reconcile::unreconcile_login_account_entry(
        &ledger_dir,
        &login_name,
        &label,
        &entry_id,
        args.posting_index,
    )
    .map_err(|err| std::io::Error::other(err.to_string()))?;
    println!("ok");
    Ok(())
}

fn run_account_transfer(
    args: AccountTransferArgs,
    context: tauri::Context<tauri::Wry>,
) -> Result<(), Box<dyn Error>> {
    let ledger_dir = resolve_cli_ledger_dir(args.ledger, context)?;
    crate::ledger::require_refreshmint_extension(&ledger_dir)?;
    let account1 = require_cli_field("account1", &args.account1)?;
    let entry_id1 = require_cli_field("entry_id1", &args.entry_id1)?;
    let account2 = require_cli_field("account2", &args.account2)?;
    let entry_id2 = require_cli_field("entry_id2", &args.entry_id2)?;
    let gl_txn_id = crate::reconcile::reconcile_transfer(
        &ledger_dir,
        &account1,
        &entry_id1,
        &account2,
        &entry_id2,
    )
    .map_err(|err| std::io::Error::other(err.to_string()))?;
    println!("{gl_txn_id}");
    Ok(())
}

fn map_entries_for_cli(
    entries: Vec<crate::account_journal::AccountEntry>,
) -> Vec<CliAccountJournalEntry> {
    entries
        .into_iter()
        .map(|entry| {
            let status = match entry.status {
                crate::account_journal::EntryStatus::Cleared => "cleared",
                crate::account_journal::EntryStatus::Pending => "pending",
                crate::account_journal::EntryStatus::Unmarked => "unmarked",
            };
            let is_transfer = crate::transfer_detector::is_probable_transfer(&entry.description);
            CliAccountJournalEntry {
                id: entry.id,
                date: entry.date,
                status: status.to_string(),
                description: entry.description,
                comment: entry.comment,
                evidence: entry.evidence,
                reconciled: entry.reconciled,
                is_transfer,
            }
        })
        .collect()
}

fn evidence_ref_matches_document(evidence_ref: &str, document_name: &str) -> bool {
    evidence_ref.starts_with(document_name)
        && evidence_ref
            .get(document_name.len()..)
            .map(|rest| rest.starts_with(':') || rest.starts_with('#'))
            .unwrap_or(false)
}

fn resolve_cli_ledger_dir(
    ledger: Option<PathBuf>,
    context: tauri::Context<tauri::Wry>,
) -> Result<PathBuf, Box<dyn Error>> {
    match ledger {
        Some(path) => Ok(crate::ledger::ensure_refreshmint_extension(path)?),
        None => default_ledger_dir(context),
    }
}

fn require_cli_field(field_name: &str, value: &str) -> Result<String, Box<dyn Error>> {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            format!("{field_name} is required"),
        )
        .into());
    }
    Ok(trimmed.to_string())
}

fn resolve_login_account_gl_account_cli(
    ledger_dir: &std::path::Path,
    login_name: &str,
    label: &str,
) -> Result<String, Box<dyn Error>> {
    let config = crate::login_config::read_login_config(ledger_dir, login_name);
    let account_cfg = config.accounts.get(label).ok_or_else(|| {
        std::io::Error::new(
            std::io::ErrorKind::NotFound,
            format!("label '{label}' not found in login '{login_name}'"),
        )
    })?;

    let gl_account = account_cfg
        .gl_account
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| {
            std::io::Error::other(format!(
                "login '{login_name}' label '{label}' is ignored (gl_account is null); set a GL account first"
            ))
        })?
        .to_string();

    if let Some(conflict) = crate::login_config::find_gl_account_conflicts(ledger_dir)
        .into_iter()
        .find(|conflict| conflict.gl_account == gl_account)
    {
        let entries = conflict
            .entries
            .iter()
            .map(|entry| format!("{}/{}", entry.login_name, entry.label))
            .collect::<Vec<_>>()
            .join(", ");
        return Err(std::io::Error::other(format!(
            "GL account '{}' has conflicting login mappings: {}; resolve conflicts first",
            conflict.gl_account, entries
        ))
        .into());
    }

    Ok(gl_account)
}

fn resolve_extraction_document_names(
    selected: &[String],
    listed: Vec<String>,
) -> Result<Vec<String>, Box<dyn Error>> {
    if selected.is_empty() {
        return Ok(listed);
    }

    let mut names = Vec::new();
    for name in selected {
        let trimmed = name.trim();
        if trimmed.is_empty() {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                "document name cannot be empty",
            )
            .into());
        }
        if !names.iter().any(|existing| existing == trimmed) {
            names.push(trimmed.to_string());
        }
    }
    Ok(names)
}

fn parse_prompt_overrides(
    entries: &[String],
) -> Result<crate::scrape::js_api::PromptOverrides, Box<dyn Error>> {
    let mut overrides = crate::scrape::js_api::PromptOverrides::new();
    for entry in entries {
        let Some((message, value)) = entry.split_once('=') else {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                format!("invalid --prompt value '{entry}', expected MESSAGE=VALUE"),
            )
            .into());
        };

        if message.is_empty() {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                format!("invalid --prompt value '{entry}', MESSAGE cannot be empty"),
            )
            .into());
        }
        if overrides
            .insert(message.to_string(), value.to_string())
            .is_some()
        {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                format!("duplicate --prompt message '{message}'"),
            )
            .into());
        }
    }
    Ok(overrides)
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
        evidence_ref_matches_document, parse_prompt_overrides, resolve_extraction_document_names,
        run_extension_load_with_dir, run_gl_add_with_dir, run_new_with_ledger_path, run_secret,
        AccountCommand, AddArgs, Cli, Commands, ExtensionLoadArgs, LoginCommand, SecretAddArgs,
        SecretArgs, SecretCommand, SecretListArgs, SecretRemoveArgs,
    };
    use crate::ledger::ensure_refreshmint_extension;
    use clap::Parser;
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
    fn parse_prompt_overrides_accepts_message_value_pairs() {
        let entries = vec!["OTP=123456".to_string(), "Security answer=blue".to_string()];
        let overrides = parse_prompt_overrides(&entries)
            .unwrap_or_else(|err| panic!("parse_prompt_overrides failed: {err}"));
        assert_eq!(overrides.get("OTP"), Some(&"123456".to_string()));
        assert_eq!(overrides.get("Security answer"), Some(&"blue".to_string()));
    }

    #[test]
    fn parse_prompt_overrides_rejects_missing_separator() {
        let entries = vec!["OTP123456".to_string()];
        assert!(parse_prompt_overrides(&entries).is_err());
    }

    #[test]
    fn parse_prompt_overrides_rejects_duplicate_messages() {
        let entries = vec!["OTP=111111".to_string(), "OTP=222222".to_string()];
        assert!(parse_prompt_overrides(&entries).is_err());
    }

    #[test]
    fn resolve_extraction_document_names_defaults_to_listed_documents() {
        let listed = vec!["2024-01.csv".to_string(), "2024-02.csv".to_string()];
        let resolved = resolve_extraction_document_names(&[], listed.clone())
            .unwrap_or_else(|err| panic!("resolve_extraction_document_names failed: {err}"));
        assert_eq!(resolved, listed);
    }

    #[test]
    fn resolve_extraction_document_names_trims_and_deduplicates() {
        let selected = vec![
            "  2024-01.csv ".to_string(),
            "2024-01.csv".to_string(),
            "2024-02.csv".to_string(),
        ];
        let resolved = resolve_extraction_document_names(&selected, Vec::new())
            .unwrap_or_else(|err| panic!("resolve_extraction_document_names failed: {err}"));
        assert_eq!(
            resolved,
            vec!["2024-01.csv".to_string(), "2024-02.csv".to_string()]
        );
    }

    #[test]
    fn resolve_extraction_document_names_rejects_empty_values() {
        let selected = vec![" ".to_string()];
        assert!(resolve_extraction_document_names(&selected, Vec::new()).is_err());
    }

    #[test]
    fn evidence_ref_matches_document_requires_delimiter() {
        assert!(evidence_ref_matches_document("foo.csv:1:1", "foo.csv"));
        assert!(evidence_ref_matches_document("foo.csv#page=1", "foo.csv"));
        assert!(!evidence_ref_matches_document("foo.csvx:1:1", "foo.csv"));
        assert!(!evidence_ref_matches_document("foo.csv", "foo.csv"));
    }

    #[test]
    fn account_extract_subcommand_parses_document_flags() {
        let cli = Cli::try_parse_from([
            "refreshmint",
            "account",
            "extract",
            "--login",
            "chase-personal",
            "--label",
            "checking",
            "--extension",
            "chase-driver",
            "--document",
            "2024-01.csv",
            "--document",
            "2024-02.csv",
        ])
        .unwrap_or_else(|err| panic!("Cli parsing failed: {err}"));

        match cli.command {
            Some(Commands::Account(args)) => match args.command {
                AccountCommand::Extract(extract) => {
                    assert_eq!(extract.login, "chase-personal");
                    assert_eq!(extract.label, "checking");
                    assert_eq!(extract.extension, Some("chase-driver".to_string()));
                    assert_eq!(
                        extract.document,
                        vec!["2024-01.csv".to_string(), "2024-02.csv".to_string()]
                    );
                }
                _ => panic!("expected account extract command"),
            },
            _ => panic!("expected account command"),
        }
    }

    #[test]
    fn account_reconcile_subcommand_parses_posting_index() {
        let cli = Cli::try_parse_from([
            "refreshmint",
            "account",
            "reconcile",
            "--login",
            "chase-personal",
            "--label",
            "checking",
            "--entry-id",
            "txn-1",
            "--counterpart-account",
            "Expenses:Food",
            "--posting-index",
            "1",
        ])
        .unwrap_or_else(|err| panic!("Cli parsing failed: {err}"));

        match cli.command {
            Some(Commands::Account(args)) => match args.command {
                AccountCommand::Reconcile(reconcile) => {
                    assert_eq!(reconcile.login, "chase-personal");
                    assert_eq!(reconcile.label, "checking");
                    assert_eq!(reconcile.entry_id, "txn-1");
                    assert_eq!(reconcile.counterpart_account, "Expenses:Food");
                    assert_eq!(reconcile.posting_index, Some(1));
                }
                _ => panic!("expected account reconcile command"),
            },
            _ => panic!("expected account command"),
        }
    }

    #[test]
    fn login_set_account_subcommand_parses_gl_account() {
        let cli = Cli::try_parse_from([
            "refreshmint",
            "login",
            "set-account",
            "--name",
            "chase-personal",
            "--label",
            "checking",
            "--gl-account",
            "Assets:Chase:Checking",
        ])
        .unwrap_or_else(|err| panic!("Cli parsing failed: {err}"));

        match cli.command {
            Some(Commands::Login(args)) => match args.command {
                LoginCommand::SetAccount(set_account) => {
                    assert_eq!(set_account.name, "chase-personal");
                    assert_eq!(set_account.label, "checking");
                    assert_eq!(
                        set_account.gl_account,
                        Some("Assets:Chase:Checking".to_string())
                    );
                }
                _ => panic!("expected login set-account command"),
            },
            _ => panic!("expected login command"),
        }
    }

    #[test]
    fn scrape_subcommand_parses_login_flag() {
        let cli = Cli::try_parse_from([
            "refreshmint",
            "scrape",
            "--login",
            "chase-personal",
            "--extension",
            "chase-driver",
        ])
        .unwrap_or_else(|err| panic!("Cli parsing failed: {err}"));

        match cli.command {
            Some(Commands::Scrape(args)) => {
                assert_eq!(args.login, "chase-personal");
                assert_eq!(args.extension, Some("chase-driver".to_string()));
            }
            _ => panic!("expected scrape command"),
        }
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

    #[test]
    fn secret_add_requires_non_empty_fields() {
        let missing_login = run_secret(SecretArgs {
            command: SecretCommand::Add(SecretAddArgs {
                login: " ".to_string(),
                domain: "example.com".to_string(),
                name: "password".to_string(),
                value: "secret".to_string(),
            }),
        });
        assert!(expect_err(missing_login, "missing login").contains("login is required"));

        let missing_domain = run_secret(SecretArgs {
            command: SecretCommand::Add(SecretAddArgs {
                login: "chase-login".to_string(),
                domain: " ".to_string(),
                name: "password".to_string(),
                value: "secret".to_string(),
            }),
        });
        assert!(expect_err(missing_domain, "missing domain").contains("domain is required"));

        let missing_name = run_secret(SecretArgs {
            command: SecretCommand::Add(SecretAddArgs {
                login: "chase-login".to_string(),
                domain: "example.com".to_string(),
                name: " ".to_string(),
                value: "secret".to_string(),
            }),
        });
        assert!(expect_err(missing_name, "missing name").contains("name is required"));
    }

    #[test]
    fn secret_remove_requires_non_empty_fields() {
        let missing_domain = run_secret(SecretArgs {
            command: SecretCommand::Remove(SecretRemoveArgs {
                login: "chase-login".to_string(),
                domain: " ".to_string(),
                name: "password".to_string(),
            }),
        });
        assert!(expect_err(missing_domain, "missing domain").contains("domain is required"));
    }

    #[test]
    fn secret_list_requires_non_empty_login() {
        let missing_login = run_secret(SecretArgs {
            command: SecretCommand::List(SecretListArgs {
                login: " ".to_string(),
            }),
        });
        assert!(expect_err(missing_login, "missing login").contains("login is required"));
    }

    fn expect_ok<T, E: std::fmt::Display>(result: Result<T, E>, label: &str) -> T {
        match result {
            Ok(value) => value,
            Err(err) => {
                panic!("expected Ok for {label}, got error: {err}");
            }
        }
    }

    fn expect_err<T, E: std::fmt::Display>(result: Result<T, E>, label: &str) -> String {
        match result {
            Ok(_) => panic!("expected Err for {label}, got Ok"),
            Err(err) => err.to_string(),
        }
    }

    fn create_temp_dir() -> PathBuf {
        let base_nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|duration| duration.as_nanos())
            .unwrap_or(0);

        for attempt in 0..64u32 {
            let dir_name = format!(
                "refreshmint-test-{}-{}-{}",
                std::process::id(),
                base_nanos,
                attempt
            );
            let mut dir = std::env::temp_dir();
            dir.push(dir_name);
            match fs::create_dir(&dir) {
                Ok(()) => return dir,
                Err(err) if err.kind() == std::io::ErrorKind::AlreadyExists => continue,
                Err(err) => panic!("failed to create temp dir: {err}"),
            }
        }

        panic!("failed to create unique temp dir after 64 attempts");
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
