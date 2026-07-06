use clap::{Args, Parser, Subcommand};
use std::error::Error;
use std::io::Read;
use std::path::{Path, PathBuf};
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
    Migrate(MigrateArgs),
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
    #[command(alias = "remove-account")]
    DeleteAccount(LoginDeleteAccountArgs),
    #[command(alias = "clear-chrome-profile")]
    ClearProfile(LoginClearProfileArgs),
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
struct LoginDeleteAccountArgs {
    #[arg(long, value_name = "NAME")]
    name: String,
    #[arg(long)]
    label: String,
    #[arg(long)]
    ledger: Option<PathBuf>,
}

#[derive(Args)]
struct LoginClearProfileArgs {
    #[arg(long, value_name = "NAME")]
    name: String,
    #[arg(long)]
    ledger: Option<PathBuf>,
}

#[derive(Args)]
struct MigrateArgs {
    #[arg(long)]
    dry_run: bool,
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
    ledger: Option<PathBuf>,
    #[arg(long)]
    profile: Option<PathBuf>,
    #[arg(long)]
    socket: Option<PathBuf>,
    #[arg(long)]
    headless: bool,
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
        help = "Extension directory (loads the manifest-declared driver and manifest secrets).",
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
    #[arg(
        long,
        value_name = "KEY=VALUE",
        action = clap::ArgAction::Append,
        help = "Key/value option for refreshmint.getOptions(). VALUE is parsed as JSON; \
                falls back to string. Repeat for multiple options."
    )]
    option: Vec<String>,
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
    ledger: Option<PathBuf>,
    #[arg(long)]
    profile: Option<PathBuf>,
    #[arg(long)]
    headless: bool,
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
    Unposted(AccountUnpostedArgs),
    Post(AccountPostArgs),
    PostAll(AccountPostAllArgs),
    Unpost(AccountUnpostArgs),
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
struct AccountUnpostedArgs {
    #[arg(long, alias = "account")]
    login: String,
    #[arg(long)]
    label: String,
    #[arg(long)]
    ledger: Option<PathBuf>,
}

#[derive(Args)]
struct AccountPostArgs {
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

/// Post ALL currently-unposted entries for a login (the ETL "post" phase),
/// mirroring the GUI auto-ETL: each entry posts as a transfer when a unique
/// transfer match exists, otherwise to `Expenses:Unknown` when the account has a
/// GL account configured. Entries with neither are left unposted.
#[derive(Args)]
struct AccountPostAllArgs {
    #[arg(long, alias = "account")]
    login: String,
    /// Restrict to one account label. Omit to post every label for the login.
    #[arg(long)]
    label: Option<String>,
    #[arg(long)]
    ledger: Option<PathBuf>,
}

#[derive(Args)]
struct AccountUnpostArgs {
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
        Some(Commands::Migrate(args)) => run_migrate(args, context),
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
        LoginCommand::DeleteAccount(delete_account_args) => {
            run_login_delete_account(delete_account_args, context)
        }
        LoginCommand::ClearProfile(clear_profile_args) => {
            run_login_clear_profile(clear_profile_args, context)
        }
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
    fn infer_secret_role(name: &str) -> &'static str {
        let lower = name.to_ascii_lowercase();
        if lower.contains("username") || lower.contains("_user") || lower.contains("login") {
            "username"
        } else {
            "password"
        }
    }

    match args.command {
        SecretCommand::Add(a) => {
            let login = require_cli_login_name("login", &a.login)?;
            let domain = require_secret_field("domain", &a.domain)?;
            let name = require_secret_field("name", &a.name)?;

            let store = crate::secret::SecretStore::new(format!("login/{login}"));
            match infer_secret_role(&name) {
                "username" => store
                    .set_username(&domain, &a.value)
                    .map_err(std::io::Error::other)?,
                _ => store
                    .set_password(&domain, &a.value)
                    .map_err(std::io::Error::other)?,
            }
            eprintln!("Secret stored.");
            Ok(())
        }
        SecretCommand::Reenter(a) => {
            let login = require_cli_login_name("login", &a.login)?;
            let domain = require_secret_field("domain", &a.domain)?;
            let name = require_secret_field("name", &a.name)?;

            let store = crate::secret::SecretStore::new(format!("login/{login}"));
            match infer_secret_role(&name) {
                "username" => store
                    .set_username(&domain, &a.value)
                    .map_err(std::io::Error::other)?,
                _ => store
                    .set_password(&domain, &a.value)
                    .map_err(std::io::Error::other)?,
            }
            eprintln!("Secret re-entered.");
            Ok(())
        }
        SecretCommand::Remove(a) => {
            let login = require_cli_login_name("login", &a.login)?;
            let domain = require_secret_field("domain", &a.domain)?;
            let _name = require_secret_field("name", &a.name)?;

            let store = crate::secret::SecretStore::new(format!("login/{login}"));
            store
                .delete_domain(&domain)
                .map_err(std::io::Error::other)?;
            eprintln!("Domain credentials removed.");
            Ok(())
        }
        SecretCommand::List(a) => {
            let login = require_cli_login_name("login", &a.login)?;
            let login_name = login.clone();
            let store = crate::secret::SecretStore::new(format!("login/{login}"));
            let entries = store.list_domains().map_err(std::io::Error::other)?;
            if entries.is_empty() {
                println!("No secrets stored for login '{login_name}'.");
            } else {
                for entry in &entries {
                    println!(
                        "{} username={} password={}",
                        entry.domain, entry.has_username, entry.has_password
                    );
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

    let login_name = require_cli_login_name("login", &args.login)?;
    require_cli_existing_login(&ledger_dir, &login_name)?;
    let extension_name = crate::login_config::resolve_login_extension(&ledger_dir, &login_name)
        .map_err(std::io::Error::other)?;

    let socket = match args.socket {
        Some(path) => path,
        None => crate::scrape::debug::default_debug_socket_path(&login_name)?,
    };
    let config = crate::scrape::debug::DebugStartConfig {
        login_name,
        extension_name,
        ledger_dir,
        profile_override: args.profile,
        headless: args.headless,
        socket_path: Some(socket),
        prompt_requires_override: true,
    };
    crate::scrape::debug::run_debug_session(config)
}

fn run_debug_exec(args: DebugExecArgs) -> Result<(), Box<dyn Error>> {
    let prompt_overrides = parse_prompt_overrides(&args.prompt)?;
    let script_options = parse_script_options(&args.option)?;
    if let Some(extension_dir) = args.extension_dir {
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
        let manifest =
            crate::scrape::load_manifest(&extension_dir).map_err(|err| -> Box<dyn Error> {
                std::io::Error::new(std::io::ErrorKind::InvalidData, err.to_string()).into()
            })?;
        let script_path = crate::scrape::resolve_driver_script_path(&extension_dir, &manifest);
        crate::scrape::debug::exec_debug_entry_module_with_options(
            &args.socket,
            &extension_dir,
            &script_path,
            Some(manifest.secrets),
            Some(prompt_overrides),
            Some(true),
            Some(script_options),
        )?;
    } else {
        let script_path = args.script.ok_or_else(|| {
            std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                "either --script or --extension-dir is required",
            )
        })?;
        let script_source = read_text_input(&script_path)?;
        if script_source.trim().is_empty() {
            return Err(
                std::io::Error::new(std::io::ErrorKind::InvalidInput, "script is empty").into(),
            );
        }
        crate::scrape::debug::exec_debug_script_with_options(
            &args.socket,
            &script_source,
            None,
            Some(prompt_overrides),
            Some(true),
            Some(script_options),
        )?;
    }
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
    for login in crate::login_config::list_logins(&ledger_dir)? {
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
    let login_name = require_cli_login_name("name", &args.name)?;

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
    let login_name = require_cli_login_name("name", &args.name)?;
    require_cli_existing_login(&ledger_dir, &login_name)?;
    let extension = args.extension.trim().to_string();

    let _lock = crate::login_config::acquire_login_lock_with_metadata(
        &ledger_dir,
        &login_name,
        "cli",
        "set-login-extension",
    )
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
    let login_name = require_cli_login_name("name", &args.name)?;
    let _lock = crate::login_config::acquire_login_lock_with_metadata(
        &ledger_dir,
        &login_name,
        "cli",
        "delete-login",
    )
    .map_err(std::io::Error::other)?;
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
    let login_name = require_cli_login_name("name", &args.name)?;
    require_cli_existing_login(&ledger_dir, &login_name)?;
    let label = require_cli_label(&args.label)?;

    let gl_account = args
        .gl_account
        .as_deref()
        .map(str::trim)
        .filter(|v| !v.is_empty())
        .map(ToOwned::to_owned);

    let _lock = crate::login_config::acquire_login_lock_with_metadata(
        &ledger_dir,
        &login_name,
        "cli",
        "set-login-account",
    )
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

fn run_login_delete_account(
    args: LoginDeleteAccountArgs,
    context: tauri::Context<tauri::Wry>,
) -> Result<(), Box<dyn Error>> {
    let ledger_dir = resolve_cli_ledger_dir(args.ledger, context)?;
    crate::ledger::require_refreshmint_extension(&ledger_dir)?;
    let login_name = require_cli_login_name("name", &args.name)?;
    require_cli_existing_login(&ledger_dir, &login_name)?;
    let label = require_cli_label(&args.label)?;
    let _lock = crate::login_config::acquire_login_lock_with_metadata(
        &ledger_dir,
        &login_name,
        "cli",
        "delete-login-account",
    )
    .map_err(std::io::Error::other)?;
    crate::login_config::remove_login_account(&ledger_dir, &login_name, &label)
        .map_err(std::io::Error::other)?;
    println!("Removed label '{label}' from login '{login_name}'.");
    Ok(())
}

fn run_login_clear_profile(
    args: LoginClearProfileArgs,
    context: tauri::Context<tauri::Wry>,
) -> Result<(), Box<dyn Error>> {
    let ledger_dir = resolve_cli_ledger_dir(args.ledger, context)?;
    crate::ledger::require_refreshmint_extension(&ledger_dir)?;
    let login_name = require_cli_login_name("name", &args.name)?;
    require_cli_existing_login(&ledger_dir, &login_name)?;

    let lock = crate::login_config::acquire_login_lock_with_metadata(
        &ledger_dir,
        &login_name,
        "cli",
        "clear-login-profile",
    )
    .map_err(std::io::Error::other)?;
    crate::scrape::profile::clear_login_profile(&ledger_dir, &login_name, &lock)
        .map_err(|e| std::io::Error::other(e.to_string()))?;
    println!("Cleared browser profile for login '{login_name}'.");
    Ok(())
}

fn run_migrate(
    args: MigrateArgs,
    context: tauri::Context<tauri::Wry>,
) -> Result<(), Box<dyn Error>> {
    let ledger_dir = resolve_cli_ledger_dir(args.ledger, context)?;
    crate::ledger::require_refreshmint_extension(&ledger_dir)?;
    let outcome = crate::migration::migrate_ledger(&ledger_dir, args.dry_run)
        .map_err(|err| std::io::Error::other(err.to_string()))?;
    println!("{}", serde_json::to_string_pretty(&outcome)?);
    Ok(())
}

fn run_scrape(args: ScrapeArgs, context: tauri::Context<tauri::Wry>) -> Result<(), Box<dyn Error>> {
    let ledger_dir = match args.ledger.as_ref() {
        Some(path) => crate::ledger::ensure_refreshmint_extension(path.clone())?,
        None => default_ledger_dir(context)?,
    };

    let login_name = require_cli_login_name("login", &args.login)?;
    require_cli_existing_login(&ledger_dir, &login_name)?;
    let extension_name = crate::login_config::resolve_login_extension(&ledger_dir, &login_name)
        .map_err(std::io::Error::other)?;

    let prompt_overrides = parse_prompt_overrides(&args.prompt)?;

    let login_name_str = login_name.clone();
    let ledger_dir_clone = ledger_dir.clone();

    let config = crate::scrape::ScrapeConfig {
        login_name,
        extension_name,
        ledger_dir,
        profile_override: args.profile,
        headless: args.headless,
        prompt_overrides,
        prompt_requires_override: true,
        prompt_ui_handler: None,
    };

    let timestamp = crate::operations::now_timestamp();
    let result = crate::scrape::run_scrape(config);
    let entry = crate::operations::ScrapeLogEntry {
        login_name: login_name_str,
        timestamp,
        success: result.is_ok(),
        error: result.as_ref().err().map(|e| e.to_string()),
        source: "manual".to_string(),
    };
    if let Err(e) = crate::operations::append_scrape_log_entry(&ledger_dir_clone, &entry) {
        eprintln!("warning: failed to write scrape log: {e}");
    }
    result
}

#[derive(serde::Serialize)]
struct CliAccountJournalEntry {
    id: String,
    date: String,
    status: String,
    description: String,
    comment: String,
    evidence: Vec<String>,
    posted: Option<String>,
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
        AccountCommand::Unposted(unposted_args) => run_account_unposted(unposted_args, context),
        AccountCommand::Post(post_args) => run_account_post(post_args, context),
        AccountCommand::PostAll(post_all_args) => run_account_post_all(post_all_args, context),
        AccountCommand::Unpost(unpost_args) => run_account_unpost(unpost_args, context),
        AccountCommand::Transfer(transfer_args) => run_account_transfer(transfer_args, context),
    }
}

fn run_account_documents(
    args: AccountDocumentsArgs,
    context: tauri::Context<tauri::Wry>,
) -> Result<(), Box<dyn Error>> {
    let ledger_dir = resolve_cli_ledger_dir(args.ledger, context)?;
    crate::ledger::require_refreshmint_extension(&ledger_dir)?;
    let login_name = require_cli_login_name("login", &args.login)?;
    let label = require_cli_label(&args.label)?;
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
    run_account_extract_with_dir(&ledger_dir, &args.login, &args.label, &args.document)
}

fn run_account_extract_with_dir(
    ledger_dir: &Path,
    login: &str,
    label: &str,
    documents: &[String],
) -> Result<(), Box<dyn Error>> {
    let ledger_dir = ledger_dir.to_path_buf();
    crate::ledger::require_refreshmint_extension(&ledger_dir)?;

    let login_name = require_cli_login_name("login", login)?;
    let label = require_cli_label(label)?;
    // Validate before locking: acquiring the lock does create_dir_all, so a
    // typo'd --login would otherwise leave a phantom logins/<name>/ dir that
    // list_logins includes.
    crate::require_existing_login(&ledger_dir, &login_name).map_err(std::io::Error::other)?;

    // Extraction does an unlocked read→dedup→write of account.journal. Hold the
    // per-login lock for the whole function so this is atomic w.r.t. other lock
    // holders. Keep the owner/purpose convention in sync with lib.rs
    // run_login_account_extraction_blocking.
    let _login_lock = crate::login_config::acquire_login_lock_with_metadata(
        &ledger_dir,
        &login_name,
        "cli",
        "extraction",
    )
    .map_err(|err| std::io::Error::other(err.to_string()))?;

    let extension_name = crate::login_config::resolve_login_extension(&ledger_dir, &login_name)
        .map_err(std::io::Error::other)?;
    let gl_account = resolve_login_account_gl_account_cli(&ledger_dir, &login_name, &label)?;

    let listed_documents = if documents.is_empty() {
        crate::extract::list_documents_for_login_account(&ledger_dir, &login_name, &label)?
            .into_iter()
            .map(|d| d.filename)
            .collect::<Vec<_>>()
    } else {
        Vec::new()
    };
    let document_names = resolve_extraction_document_names(documents, listed_documents)?;

    if document_names.is_empty() {
        println!("No documents found for login '{login_name}' label '{label}'.");
        return Ok(());
    }

    let doc_count = document_names.len();

    // Run extraction + dedup + journal write, capturing any error so we can
    // always flush the extract log (including console logs) even on failure.
    let mut console_logs: Vec<crate::operations::ExtractConsoleLogLine> = Vec::new();
    let mut new_count = 0usize;

    let outcome: Result<(), Box<dyn Error>> = (|| {
        let extraction = crate::extract::run_extraction_for_login_account(
            &ledger_dir,
            &login_name,
            &label,
            &gl_account,
            &extension_name,
            &document_names,
        )
        .map_err(|err| std::io::Error::other(err.to_string()))?;

        console_logs = extraction
            .console_logs
            .into_iter()
            .map(|l| crate::operations::ExtractConsoleLogLine {
                level: l.level,
                message: l.message,
                document_name: l.document_name,
            })
            .collect();

        let journal_path =
            crate::account_journal::login_account_journal_path(&ledger_dir, &login_name, &label);
        let existing_entries = crate::account_journal::read_journal_at_path(&journal_path)?;

        let config = crate::dedup::DedupConfig::default();
        let mut all_updated = existing_entries;

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

            let actions = crate::dedup::run_dedup_for_login_account(
                &ledger_dir,
                &login_name,
                &label,
                &all_updated,
                &doc_txns,
                doc_name,
                &config,
            );
            new_count += actions
                .iter()
                .filter(|a| matches!(a.result, crate::dedup::DedupResult::New))
                .count();

            let default_account = all_updated
                .first()
                .and_then(|e| e.postings.first())
                .map(|p| p.account.clone())
                .unwrap_or_else(|| gl_account.clone());
            if default_account.is_empty() {
                let has_implicit = doc_txns.iter().any(|t| t.tpostings.is_none());
                if has_implicit {
                    return Err(std::io::Error::other(format!(
                        "login '{login_name}' label '{label}': extractor produced a \
                         transaction without explicit tpostings but no glAccount is \
                         configured; set a GL account or fix the extractor"
                    ))
                    .into());
                }
            }
            let staging_account =
                crate::staging::canonical_staging_account(&format!("{login_name}:{label}"));

            all_updated = crate::dedup::apply_dedup_actions_for_login_account(
                &ledger_dir,
                (&login_name, &label),
                all_updated,
                &actions,
                &default_account,
                &staging_account,
                Some(&format!("{extension_name}:latest")),
            )
            .map_err(|err| std::io::Error::other(err.to_string()))?;
            all_updated = crate::dedup::apply_coverage_lifecycle_for_login_account(
                &ledger_dir,
                &login_name,
                &label,
                doc_name,
                &doc_txns,
                all_updated,
            )
            .map_err(|err| std::io::Error::other(err.to_string()))?;
        }

        crate::account_journal::write_journal_at_path(&journal_path, &all_updated)?;
        Ok(())
    })();

    // Write extract log regardless of success/failure so console logs and errors
    // are always persisted for later review.
    let _ = crate::operations::append_extract_log_entry(
        &ledger_dir,
        &crate::operations::ExtractLogEntry {
            login_name: login_name.clone(),
            label: label.clone(),
            timestamp: crate::operations::now_timestamp(),
            success: outcome.is_ok(),
            error: outcome.as_ref().err().map(|e| e.to_string()),
            document_count: doc_count,
            new_entry_count: new_count,
            console_logs,
            failed_documents: Vec::new(),
        },
    );

    outcome?;
    println!("Extraction complete. Added {new_count} new transaction(s).");
    Ok(())
}

fn run_account_journal(
    args: AccountJournalArgs,
    context: tauri::Context<tauri::Wry>,
) -> Result<(), Box<dyn Error>> {
    let ledger_dir = resolve_cli_ledger_dir(args.ledger, context)?;
    crate::ledger::require_refreshmint_extension(&ledger_dir)?;
    let login_name = require_cli_login_name("login", &args.login)?;
    let label = require_cli_label(&args.label)?;
    let journal_path =
        crate::account_journal::login_account_journal_path(&ledger_dir, &login_name, &label);
    let entries = crate::account_journal::read_journal_at_path(&journal_path)?;
    let extra = crate::ledger::read_refreshmint_config(&ledger_dir)
        .map(|c| c.extra_transfer_patterns)
        .unwrap_or_default();
    println!(
        "{}",
        serde_json::to_string_pretty(&map_entries_for_cli(entries, &extra))?
    );
    Ok(())
}

fn run_account_unposted(
    args: AccountUnpostedArgs,
    context: tauri::Context<tauri::Wry>,
) -> Result<(), Box<dyn Error>> {
    let ledger_dir = resolve_cli_ledger_dir(args.ledger, context)?;
    crate::ledger::require_refreshmint_extension(&ledger_dir)?;
    let login_name = require_cli_login_name("login", &args.login)?;
    let label = require_cli_label(&args.label)?;
    let entries = crate::post::get_unposted_login_account(&ledger_dir, &login_name, &label)
        .map_err(|err| std::io::Error::other(err.to_string()))?;
    let extra = crate::ledger::read_refreshmint_config(&ledger_dir)
        .map(|c| c.extra_transfer_patterns)
        .unwrap_or_default();
    println!(
        "{}",
        serde_json::to_string_pretty(&map_entries_for_cli(entries, &extra))?
    );
    Ok(())
}

fn run_account_post(
    args: AccountPostArgs,
    context: tauri::Context<tauri::Wry>,
) -> Result<(), Box<dyn Error>> {
    let ledger_dir = resolve_cli_ledger_dir(args.ledger, context)?;
    crate::ledger::require_refreshmint_extension(&ledger_dir)?;
    let login_name = require_cli_login_name("login", &args.login)?;
    let label = require_cli_label(&args.label)?;
    let entry_id = require_cli_field("entry_id", &args.entry_id)?;
    let counterpart_account = require_cli_field("counterpart_account", &args.counterpart_account)?;
    let _ = resolve_login_account_gl_account_cli(&ledger_dir, &login_name, &label)?;
    let gl_txn_id = crate::post::post_login_account_entry(
        &ledger_dir,
        &login_name,
        &label,
        &entry_id,
        &counterpart_account,
        args.posting_index,
        "cli",
    )
    .map_err(|err| std::io::Error::other(err.to_string()))?;
    println!("{gl_txn_id}");
    Ok(())
}

/// The ETL "post" phase as a CLI: post every unposted entry for the login
/// (optionally one label), choosing a transfer post when a unique transfer match
/// exists and otherwise defaulting to `Expenses:Unknown`. Mirrors the GUI
/// auto-ETL post phase (see `src/App.tsx`). Per-entry failures are collected and
/// reported, and the command exits non-zero if any entry failed to post (so a
/// silent partial failure can't masquerade as success).
fn run_account_post_all(
    args: AccountPostAllArgs,
    context: tauri::Context<tauri::Wry>,
) -> Result<(), Box<dyn Error>> {
    let ledger_dir = resolve_cli_ledger_dir(args.ledger, context)?;
    crate::ledger::require_refreshmint_extension(&ledger_dir)?;
    run_account_post_all_with_dir(&ledger_dir, &args.login, &args.label).map(|_| ())
}

// login is the raw --login value; label is the optional --label (None = all).

/// Counterpart account for a default (non-transfer) post-all posting: the matching
/// active CategoryRule's account when one exists, else `Expenses:Unknown`. Mirrors
/// the GUI direct-post paths (App.tsx auto-ETL, PipelineTab) which use
/// `suggestion.ruleAccount ?? 'Expenses:Unknown'`; both read
/// categorize::CategoryResult::rule_account.
fn post_all_counterpart(suggestion: Option<&crate::categorize::CategoryResult>) -> &str {
    suggestion
        .and_then(|s| s.rule_account.as_deref())
        .unwrap_or("Expenses:Unknown")
}

fn run_account_post_all_with_dir(
    ledger_dir: &Path,
    login: &str,
    label: &Option<String>,
) -> Result<usize, Box<dyn Error>> {
    let login_name = require_cli_login_name("login", login)?;

    // Either the one requested label, or every account label for this login.
    let labels: Vec<String> = match label {
        Some(label) => vec![require_cli_label(label)?],
        None => crate::login_config::read_login_config(ledger_dir, &login_name)
            .accounts
            .into_keys()
            .collect(),
    };

    let mut posted = 0usize;
    let mut transfers = 0usize;
    let mut skipped = 0usize;
    let mut errors: Vec<String> = Vec::new();

    for label in &labels {
        // The configured GL account decides whether an unmatched entry
        // default-posts to Expenses:Unknown (mirrors the GUI). A conflicting GL
        // account makes this error; record it and skip the label.
        let gl_account = match resolve_login_account_gl_account_cli(ledger_dir, &login_name, label)
        {
            Ok(gl) => gl,
            Err(err) => {
                errors.push(format!("{login_name}/{label}: {err}"));
                continue;
            }
        };

        let unposted = crate::post::get_unposted_login_account(ledger_dir, &login_name, label)
            .map_err(|err| std::io::Error::other(err.to_string()))?;
        if unposted.is_empty() {
            continue;
        }
        // Transfer matches + category-rule matches, same data the GUI uses.
        let suggestions = crate::categorize::suggest_categories(ledger_dir, &login_name, label)
            .map_err(|err| std::io::Error::other(err.to_string()))?;

        for entry in unposted {
            let transfer = suggestions
                .get(&entry.id)
                .and_then(|s| s.transfer_match.as_ref());
            let outcome: Result<&str, String> = if let Some(tm) = transfer {
                // account_locator is "logins/<login>/accounts/<label>".
                let parts: Vec<&str> = tm.account_locator.split('/').collect();
                match (parts.get(1), parts.get(3)) {
                    (Some(other_login), Some(other_label)) => {
                        // Record the transfer decision as a durable TransferLink
                        // resolution BEFORE posting (idempotent via fingerprint
                        // dedup). Mirrors the GUI paths (PipelineTab / App.tsx
                        // auto-ETL via createTransferLinkResolution in
                        // src/automation-utils.ts). With the TransferPolicy
                        // filter in the matchers, blocked pairs never reach this
                        // branch.
                        crate::automation::create_transfer_link(
                            ledger_dir,
                            (&login_name, label, &entry.id),
                            (other_login, other_label, &tm.entry_id),
                        )
                        .map_err(|err| err.to_string())
                        .and_then(|_| {
                            crate::post::post_login_account_transfer(
                                ledger_dir,
                                &login_name,
                                label,
                                &entry.id,
                                other_login,
                                other_label,
                                &tm.entry_id,
                                None,
                                "cli",
                            )
                            .map(|_| "transfer")
                            .map_err(|err| err.to_string())
                        })
                    }
                    _ => Err(format!(
                        "malformed transfer locator: {}",
                        tm.account_locator
                    )),
                }
            } else if !gl_account.is_empty() {
                // A matching CategoryRule posts directly to its account (one GL
                // write) instead of Expenses:Unknown.
                let counterpart = post_all_counterpart(suggestions.get(&entry.id));
                crate::post::post_login_account_entry(
                    ledger_dir,
                    &login_name,
                    label,
                    &entry.id,
                    counterpart,
                    None,
                    "cli",
                )
                .map(|_| "default")
                .map_err(|err| err.to_string())
            } else {
                Ok("skipped")
            };

            match outcome {
                Ok("transfer") => {
                    transfers += 1;
                    posted += 1;
                }
                Ok("skipped") => skipped += 1,
                Ok(_) => posted += 1,
                Err(err) => errors.push(format!("{login_name}/{label}/{}: {err}", entry.id)),
            }
        }
    }

    // After posting, run the automation policy loop (rule-backed
    // recategorizations of pre-existing Unknown GL rows + safe anomaly retires,
    // plus entry-bound Auto resolutions like PostCategory / LinkTransfer).
    // Mirrors App.tsx auto-ETL phase 3 / PipelineTab post-all.
    //
    // list_automation_proposals only runs login_account_proposals when BOTH
    // login and label are set (see automation::list_automation_proposals), so a
    // {login: Some, label: None} scope drains no entry-bound proposals. Drive
    // the policy once per resolved (login, label), then once more for GL.
    let mut policy_applied: Vec<String> = Vec::new();
    for label in &labels {
        policy_applied.extend(
            crate::automation::apply_automation_policy(
                ledger_dir,
                crate::automation::AutomationScope {
                    login_name: Some(login_name.clone()),
                    label: Some(label.clone()),
                    include_gl: Some(false),
                },
            )
            .map_err(|err| std::io::Error::other(err.to_string()))?,
        );
    }
    // A final GL pass (no login scope) drains rule-backed RecategorizeGl on
    // pre-existing Unknown rows plus any remaining global Auto proposals.
    policy_applied.extend(
        crate::automation::apply_automation_policy(
            ledger_dir,
            crate::automation::AutomationScope {
                login_name: None,
                label: None,
                include_gl: Some(true),
            },
        )
        .map_err(|err| std::io::Error::other(err.to_string()))?,
    );
    for line in &policy_applied {
        println!("automation: applied {line}");
    }

    println!(
        "Posted {posted} entries ({transfers} as transfers); left {skipped} unposted (no GL account and no transfer match). Automation applied {} proposal(s).",
        policy_applied.len()
    );
    if !errors.is_empty() {
        for err in &errors {
            eprintln!("post error: {err}");
        }
        return Err(std::io::Error::other(format!(
            "{} entr{} failed to post",
            errors.len(),
            if errors.len() == 1 { "y" } else { "ies" }
        ))
        .into());
    }
    Ok(policy_applied.len())
}

fn run_account_unpost(
    args: AccountUnpostArgs,
    context: tauri::Context<tauri::Wry>,
) -> Result<(), Box<dyn Error>> {
    let ledger_dir = resolve_cli_ledger_dir(args.ledger, context)?;
    crate::ledger::require_refreshmint_extension(&ledger_dir)?;
    let login_name = require_cli_login_name("login", &args.login)?;
    let label = require_cli_label(&args.label)?;
    let entry_id = require_cli_field("entry_id", &args.entry_id)?;
    crate::post::unpost_login_account_entry(
        &ledger_dir,
        &login_name,
        &label,
        &entry_id,
        args.posting_index,
        None,
        "cli",
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
    let gl_txn_id =
        crate::post::post_transfer(&ledger_dir, &account1, &entry_id1, &account2, &entry_id2)
            .map_err(|err| std::io::Error::other(err.to_string()))?;
    println!("{gl_txn_id}");
    Ok(())
}

fn map_entries_for_cli(
    entries: Vec<crate::account_journal::AccountEntry>,
    // Configured extraTransferPatterns, so the isTransfer flag also fires for
    // user-configured descriptions (mirrors the GUI listing path).
    extra_transfer_patterns: &[String],
) -> Vec<CliAccountJournalEntry> {
    entries
        .into_iter()
        .map(|entry| {
            let status = match entry.status {
                crate::account_journal::EntryStatus::Cleared => "cleared",
                crate::account_journal::EntryStatus::Pending => "pending",
                crate::account_journal::EntryStatus::Unmarked => "unmarked",
            };
            let is_transfer = crate::transfer_detector::is_probable_transfer_with_extra(
                &entry.description,
                extra_transfer_patterns,
            );
            CliAccountJournalEntry {
                id: entry.id,
                date: entry.date,
                status: status.to_string(),
                description: entry.description,
                comment: entry.comment,
                evidence: entry.evidence,
                posted: entry.posted,
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

fn require_cli_login_name(field_name: &str, value: &str) -> Result<String, Box<dyn Error>> {
    let login_name = require_cli_field(field_name, value)?;
    crate::login_config::validate_label(&login_name).map_err(|err| {
        std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            format!("invalid {field_name}: {err}"),
        )
    })?;
    Ok(login_name)
}

fn require_cli_existing_login(ledger_dir: &Path, login_name: &str) -> Result<(), Box<dyn Error>> {
    let config_path = crate::login_config::login_config_path(ledger_dir, login_name);
    if config_path.exists() {
        Ok(())
    } else {
        Err(std::io::Error::new(
            std::io::ErrorKind::NotFound,
            format!("login '{login_name}' does not exist"),
        )
        .into())
    }
}

fn require_cli_label(value: &str) -> Result<String, Box<dyn Error>> {
    let label = require_cli_field("label", value)?;
    crate::login_config::validate_label(&label).map_err(|err| {
        std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            format!("invalid label: {err}"),
        )
    })?;
    Ok(label)
}

fn resolve_login_account_gl_account_cli(
    ledger_dir: &std::path::Path,
    login_name: &str,
    label: &str,
) -> Result<String, Box<dyn Error>> {
    let config = crate::login_config::read_login_config(ledger_dir, login_name);
    // _default account entries don't require a label key; treat missing as empty gl_account.
    let gl_account = config
        .accounts
        .get(label)
        .and_then(|a| a.gl_account.as_deref())
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string)
        .unwrap_or_default();

    if gl_account.is_empty() {
        return Ok(gl_account);
    }

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

fn parse_script_options(
    entries: &[String],
) -> Result<crate::scrape::js_api::ScriptOptions, Box<dyn Error>> {
    let mut map = crate::scrape::js_api::ScriptOptions::new();
    for entry in entries {
        let Some((key, val_str)) = entry.split_once('=') else {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                format!("invalid --option value '{entry}', expected KEY=VALUE"),
            )
            .into());
        };
        let value = serde_json::from_str(val_str)
            .unwrap_or_else(|_| serde_json::Value::String(val_str.to_string()));
        map.insert(key.to_string(), value);
    }
    Ok(map)
}

fn default_ledger_dir(context: tauri::Context<tauri::Wry>) -> Result<PathBuf, Box<dyn Error>> {
    let app = tauri::Builder::default().build(context)?;
    let documents_dir = app.path().document_dir()?;
    Ok(crate::ledger::default_ledger_dir_from_documents(
        documents_dir,
    ))
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::{
        evidence_ref_matches_document, map_entries_for_cli, parse_prompt_overrides,
        post_all_counterpart, require_cli_existing_login, require_cli_label,
        require_cli_login_name, resolve_extraction_document_names, run_account_extract_with_dir,
        run_account_post_all_with_dir, run_extension_load_with_dir, run_gl_add_with_dir,
        run_new_with_ledger_path, run_secret, AccountCommand, AddArgs, Cli, Commands,
        ExtensionLoadArgs, LoginCommand, SecretAddArgs, SecretArgs, SecretCommand, SecretListArgs,
        SecretRemoveArgs,
    };
    use crate::ledger::ensure_refreshmint_extension;
    use clap::Parser;
    use serde_json::Value;
    use std::fs;
    use std::path::{Path, PathBuf};
    use std::process::Command;
    use std::time::{SystemTime, UNIX_EPOCH};

    #[test]
    fn map_entries_for_cli_honors_extra_transfer_patterns() {
        // The isTransfer flag (which gates the Pipeline Link Transfer button) must
        // fire for a configured extraTransferPattern. "MOVE MONEY" matches no
        // built-in transfer pattern.
        let entry = crate::account_journal::AccountEntry {
            id: "e1".to_string(),
            date: "2024-01-15".to_string(),
            status: crate::account_journal::EntryStatus::Cleared,
            description: "MOVE MONEY 123".to_string(),
            comment: String::new(),
            evidence: vec![],
            postings: vec![],
            tags: vec![],
            extracted_by: None,
            posted: None,
            posted_postings: vec![],
        };
        assert!(
            !map_entries_for_cli(vec![entry.clone()], &[])[0].is_transfer,
            "unconfigured description is not a built-in transfer"
        );
        assert!(
            map_entries_for_cli(vec![entry], &["move money".to_string()])[0].is_transfer,
            "configured pattern should mark the entry as a transfer"
        );
    }

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
    fn account_post_subcommand_parses_posting_index() {
        let cli = Cli::try_parse_from([
            "refreshmint",
            "account",
            "post",
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
                AccountCommand::Post(post_args) => {
                    assert_eq!(post_args.login, "chase-personal");
                    assert_eq!(post_args.label, "checking");
                    assert_eq!(post_args.entry_id, "txn-1");
                    assert_eq!(post_args.counterpart_account, "Expenses:Food");
                    assert_eq!(post_args.posting_index, Some(1));
                }
                _ => panic!("expected account post command"),
            },
            _ => panic!("expected account command"),
        }
    }

    #[test]
    fn account_post_all_subcommand_parses_optional_label() {
        // Explicit label.
        let cli = Cli::try_parse_from([
            "refreshmint",
            "account",
            "post-all",
            "--login",
            "provident-yonran",
            "--label",
            "signature_cash_back_4569",
        ])
        .unwrap_or_else(|err| panic!("Cli parsing failed: {err}"));
        match cli.command {
            Some(Commands::Account(args)) => match args.command {
                AccountCommand::PostAll(post_all) => {
                    assert_eq!(post_all.login, "provident-yonran");
                    assert_eq!(post_all.label.as_deref(), Some("signature_cash_back_4569"));
                }
                _ => panic!("expected account post-all command"),
            },
            _ => panic!("expected account command"),
        }

        // No label => post every label for the login.
        let cli = Cli::try_parse_from([
            "refreshmint",
            "account",
            "post-all",
            "--login",
            "provident-yonran",
        ])
        .unwrap_or_else(|err| panic!("Cli parsing failed: {err}"));
        match cli.command {
            Some(Commands::Account(args)) => match args.command {
                AccountCommand::PostAll(post_all) => {
                    assert_eq!(post_all.login, "provident-yonran");
                    assert_eq!(post_all.label, None);
                }
                _ => panic!("expected account post-all command"),
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
    fn login_delete_account_subcommand_parses_label() {
        let cli = Cli::try_parse_from([
            "refreshmint",
            "login",
            "delete-account",
            "--name",
            "chase-personal",
            "--label",
            "checking",
        ])
        .unwrap_or_else(|err| panic!("Cli parsing failed: {err}"));

        match cli.command {
            Some(Commands::Login(args)) => match args.command {
                LoginCommand::DeleteAccount(delete_account) => {
                    assert_eq!(delete_account.name, "chase-personal");
                    assert_eq!(delete_account.label, "checking");
                }
                _ => panic!("expected login delete-account command"),
            },
            _ => panic!("expected login command"),
        }
    }

    #[test]
    fn login_remove_account_alias_still_parses() {
        let cli = Cli::try_parse_from([
            "refreshmint",
            "login",
            "remove-account",
            "--name",
            "chase-personal",
            "--label",
            "checking",
        ])
        .unwrap_or_else(|err| panic!("Cli parsing failed: {err}"));

        match cli.command {
            Some(Commands::Login(args)) => match args.command {
                LoginCommand::DeleteAccount(delete_account) => {
                    assert_eq!(delete_account.name, "chase-personal");
                    assert_eq!(delete_account.label, "checking");
                }
                _ => panic!("expected login delete-account command"),
            },
            _ => panic!("expected login command"),
        }
    }

    #[test]
    fn scrape_subcommand_parses_login_flag() {
        let cli = Cli::try_parse_from(["refreshmint", "scrape", "--login", "chase-personal"])
            .unwrap_or_else(|err| panic!("Cli parsing failed: {err}"));

        match cli.command {
            Some(Commands::Scrape(args)) => {
                assert_eq!(args.login, "chase-personal");
            }
            _ => panic!("expected scrape command"),
        }
    }

    #[test]
    fn account_extract_fails_when_login_lock_held() {
        // Mirrors the GUI extraction lock test: the CLI extract path must fail fast
        // when another operation holds the per-login lock, not corrupt the journal.
        let base_dir = create_temp_dir();
        let ledger_dir = base_dir.join("ledger.refreshmint");
        fs::create_dir_all(&ledger_dir).unwrap_or_else(|err| panic!("mkdir failed: {err}"));
        crate::login_config::write_login_config(
            &ledger_dir,
            "chase",
            &crate::login_config::LoginConfig::default(),
        )
        .unwrap_or_else(|err| panic!("failed to write login config: {err}"));
        let _lock = crate::login_config::acquire_login_lock_with_metadata(
            &ledger_dir,
            "chase",
            "test",
            "hold",
        )
        .unwrap_or_else(|err| panic!("failed to acquire login lock: {err}"));
        let result = run_account_extract_with_dir(
            &ledger_dir,
            "chase",
            "checking",
            &["2024-01.pdf".to_string()],
        );
        match result {
            Ok(()) => panic!("expected extraction to fail while login lock held"),
            Err(err) => assert!(
                err.to_string().contains("currently in use"),
                "unexpected error: {err}"
            ),
        }
        let _ = fs::remove_dir_all(&base_dir);
    }

    #[test]
    fn post_all_counterpart_prefers_rule_account() {
        let with_rule = crate::categorize::CategoryResult {
            suggested: None,
            amount_changed: false,
            status_changed: false,
            transfer_match: None,
            transfer_candidates: Vec::new(),
            rule_account: Some("Expenses:Groceries".to_string()),
        };
        assert_eq!(post_all_counterpart(Some(&with_rule)), "Expenses:Groceries");
        let without_rule = crate::categorize::CategoryResult {
            suggested: None,
            amount_changed: false,
            status_changed: false,
            transfer_match: None,
            transfer_candidates: Vec::new(),
            rule_account: None,
        };
        assert_eq!(
            post_all_counterpart(Some(&without_rule)),
            "Expenses:Unknown"
        );
        assert_eq!(post_all_counterpart(None), "Expenses:Unknown");
    }

    #[test]
    fn run_account_post_all_posts_to_rule_account() {
        // A CategoryRule matching an entry makes post-all post directly to the
        // rule's account (one GL write) instead of Expenses:Unknown.
        let base_dir = create_temp_dir();
        let ledger_dir = base_dir.join("ledger.refreshmint");
        fs::create_dir_all(&ledger_dir).unwrap();

        let mut cfg = crate::login_config::LoginConfig::default();
        cfg.accounts.insert(
            "checking".to_string(),
            crate::login_config::LoginAccountConfig {
                gl_account: Some("Assets:Checking".to_string()),
            },
        );
        crate::login_config::write_login_config(&ledger_dir, "chase", &cfg).unwrap();

        let jpath =
            crate::account_journal::login_account_journal_path(&ledger_dir, "chase", "checking");
        fs::create_dir_all(jpath.parent().unwrap()).unwrap();
        crate::account_journal::write_journal_at_path(
            &jpath,
            &[crate::account_journal::AccountEntry {
                id: "entry-1".to_string(),
                date: "2026-01-01".to_string(),
                status: crate::account_journal::EntryStatus::Cleared,
                description: "SAFEWAY #123".to_string(),
                comment: String::new(),
                evidence: vec![],
                postings: vec![crate::account_journal::EntryPosting {
                    account: "Assets:Checking".to_string(),
                    amount: Some(crate::account_journal::SimpleAmount {
                        quantity: "-21.32".to_string(),
                        commodity: "USD".to_string(),
                    }),
                }],
                tags: vec![],
                extracted_by: None,
                posted: None,
                posted_postings: vec![],
            }],
        )
        .unwrap();

        crate::automation::create_resolution(
            &ledger_dir,
            crate::automation::NewResolutionInput {
                kind: crate::automation::ResolutionKind::CategoryRule,
                subject_refs: vec![],
                parts: vec![crate::automation::ResolutionPart {
                    amount: None,
                    account: Some("Expenses:Groceries".to_string()),
                    ref_: None,
                    notes: None,
                }],
                notes: None,
                predicate: Some(crate::automation::CategoryRulePredicate {
                    description_regex: None,
                    normalized_payee: Some("SAFEWAY".to_string()),
                    amount_min: None,
                    amount_max: None,
                }),
            },
        )
        .unwrap();

        let policy_applied =
            run_account_post_all_with_dir(&ledger_dir, "chase", &Some("checking".to_string()))
                .unwrap();

        let gl = fs::read_to_string(ledger_dir.join("general.journal")).unwrap();
        assert!(
            gl.contains("Expenses:Groceries"),
            "expected rule account in GL, got: {gl}"
        );
        assert!(
            !gl.contains("Expenses:Unknown"),
            "entry should not fall back to Expenses:Unknown, got: {gl}"
        );
        // Direct-post (not the policy loop) must be what routed the entry to the
        // rule account: with the entry already posted correctly, the policy pass
        // has nothing to recategorize. (Guards against the policy loop masking a
        // direct-post regression — both would otherwise yield the same final GL.)
        assert_eq!(
            policy_applied, 0,
            "entry should post directly to the rule account, not via the policy loop"
        );

        let _ = fs::remove_dir_all(&base_dir);
    }

    #[test]
    fn run_account_post_all_policy_posts_entry_bound_category_resolution() {
        // Post-all's policy pass must drain entry-bound Auto proposals
        // (PostCategory) for this login's accounts. The scope used to be
        // {login: Some, label: None} — a dead combination that
        // list_automation_proposals never runs login_account_proposals for — so
        // entry-bound resolutions were silently never applied by CLI post-all.
        let base_dir = create_temp_dir();
        let ledger_dir = base_dir.join("ledger.refreshmint");
        fs::create_dir_all(&ledger_dir).unwrap();

        // The account has NO gl_account, so phase-1 direct-posting skips the
        // entry and leaves it unposted for the policy phase to handle.
        let mut cfg = crate::login_config::LoginConfig::default();
        cfg.accounts.insert(
            "checking".to_string(),
            crate::login_config::LoginAccountConfig { gl_account: None },
        );
        crate::login_config::write_login_config(&ledger_dir, "chase", &cfg).unwrap();

        let jpath =
            crate::account_journal::login_account_journal_path(&ledger_dir, "chase", "checking");
        fs::create_dir_all(jpath.parent().unwrap()).unwrap();
        crate::account_journal::write_journal_at_path(
            &jpath,
            &[crate::account_journal::AccountEntry {
                id: "entry-1".to_string(),
                date: "2026-01-01".to_string(),
                status: crate::account_journal::EntryStatus::Cleared,
                description: "BLUE BOTTLE".to_string(),
                comment: String::new(),
                evidence: vec![],
                postings: vec![crate::account_journal::EntryPosting {
                    account: "Assets:Checking".to_string(),
                    amount: Some(crate::account_journal::SimpleAmount {
                        quantity: "-4.50".to_string(),
                        commodity: "USD".to_string(),
                    }),
                }],
                tags: vec![],
                extracted_by: None,
                posted: None,
                posted_postings: vec![],
            }],
        )
        .unwrap();
        fs::write(ledger_dir.join("general.journal"), "").unwrap();

        // Entry-bound "always post this entry to Expenses:Coffee" resolution.
        crate::automation::create_resolution(
            &ledger_dir,
            crate::automation::NewResolutionInput {
                kind: crate::automation::ResolutionKind::Category,
                subject_refs: vec![crate::bookkeeping::TypedRef {
                    kind: crate::bookkeeping::TypedRefKind::LoginEntry,
                    id: None,
                    locator: Some("logins/chase/accounts/checking".to_string()),
                    entry_id: Some("entry-1".to_string()),
                    login_name: Some("chase".to_string()),
                    label: Some("checking".to_string()),
                    filename: None,
                }],
                parts: vec![crate::automation::ResolutionPart {
                    amount: None,
                    account: Some("Expenses:Coffee".to_string()),
                    ref_: None,
                    notes: None,
                }],
                notes: None,
                predicate: None,
            },
        )
        .unwrap();

        let policy_applied =
            run_account_post_all_with_dir(&ledger_dir, "chase", &Some("checking".to_string()))
                .unwrap();

        let gl = fs::read_to_string(ledger_dir.join("general.journal")).unwrap();
        assert!(
            gl.contains("Expenses:Coffee"),
            "policy pass should post the entry to the resolution's account, got: {gl}"
        );
        let entries = crate::account_journal::read_journal_at_path(&jpath).unwrap();
        assert!(
            entries[0].posted.is_some(),
            "entry should be marked posted after the policy pass"
        );
        assert!(
            policy_applied >= 1,
            "policy loop should have applied the entry-bound PostCategory proposal"
        );

        let _ = fs::remove_dir_all(&base_dir);
    }

    #[test]
    fn run_account_post_all_transfer_records_transfer_link_resolution() {
        // Auto post-all must record the transfer decision as a durable
        // TransferLink resolution (idempotent via fingerprint dedup), not just
        // post it — otherwise the automation ledger has no record of the link.
        let base_dir = create_temp_dir();
        let ledger_dir = base_dir.join("ledger.refreshmint");
        crate::ledger::new_ledger_at_dir(&ledger_dir).unwrap();

        let mut chase_cfg = crate::login_config::LoginConfig::default();
        chase_cfg.accounts.insert(
            "checking".to_string(),
            crate::login_config::LoginAccountConfig { gl_account: None },
        );
        crate::login_config::write_login_config(&ledger_dir, "chase", &chase_cfg).unwrap();
        let mut boa_cfg = crate::login_config::LoginConfig::default();
        boa_cfg.accounts.insert(
            "savings".to_string(),
            crate::login_config::LoginAccountConfig { gl_account: None },
        );
        crate::login_config::write_login_config(&ledger_dir, "boa", &boa_cfg).unwrap();

        let make = |account: &str, amount: &str, id: &str| crate::account_journal::AccountEntry {
            id: id.to_string(),
            date: "2026-01-01".to_string(),
            status: crate::account_journal::EntryStatus::Cleared,
            description: "Transfer".to_string(),
            comment: String::new(),
            evidence: vec![],
            postings: vec![crate::account_journal::EntryPosting {
                account: account.to_string(),
                amount: Some(crate::account_journal::SimpleAmount {
                    quantity: amount.to_string(),
                    commodity: "USD".to_string(),
                }),
            }],
            tags: vec![("isTransfer".to_string(), "true".to_string())],
            extracted_by: None,
            posted: None,
            posted_postings: vec![],
        };
        crate::account_journal::write_journal_at_path(
            &crate::account_journal::login_account_journal_path(&ledger_dir, "chase", "checking"),
            &[make("Assets:Checking", "-100.00", "out-1")],
        )
        .unwrap();
        crate::account_journal::write_journal_at_path(
            &crate::account_journal::login_account_journal_path(&ledger_dir, "boa", "savings"),
            &[make("Assets:Savings", "100.00", "in-1")],
        )
        .unwrap();

        run_account_post_all_with_dir(&ledger_dir, "chase", &Some("checking".to_string())).unwrap();

        let gl = fs::read_to_string(ledger_dir.join("general.journal")).unwrap();
        assert!(
            gl.contains("source: logins/chase/accounts/checking:out-1")
                && gl.contains("source: logins/boa/accounts/savings:in-1"),
            "post-all should post the transfer, got: {gl}"
        );
        let transfer_links: Vec<_> = crate::automation::list_resolutions(&ledger_dir)
            .unwrap()
            .into_iter()
            .filter(|r| r.kind == crate::automation::ResolutionKind::TransferLink)
            .collect();
        assert_eq!(
            transfer_links.len(),
            1,
            "post-all should record exactly one TransferLink resolution"
        );
        assert_eq!(
            transfer_links[0].status,
            crate::automation::ResolutionStatus::Active
        );

        // Second run: nothing left to post; the resolution set is unchanged
        // (fingerprint dedup makes recording idempotent).
        run_account_post_all_with_dir(&ledger_dir, "chase", &Some("checking".to_string())).unwrap();
        let after: Vec<_> = crate::automation::list_resolutions(&ledger_dir)
            .unwrap()
            .into_iter()
            .filter(|r| r.kind == crate::automation::ResolutionKind::TransferLink)
            .collect();
        assert_eq!(after.len(), 1, "second run must not duplicate resolutions");
        assert_eq!(after[0].id, transfer_links[0].id);

        let _ = fs::remove_dir_all(&base_dir);
    }

    #[test]
    fn unpost_transfer_records_negative_memory_and_kills_repost_loop() {
        // Regression: the unpost -> re-post loop. Unposting a merged transfer must
        // record a NotTransferLink (negative memory) and disable the TransferLink
        // twin so neither the heuristic nor the resolution re-posts it. See
        // post::unpost_login_account_entry and automation::TransferPolicy.
        let base_dir = create_temp_dir();
        let ledger_dir = base_dir.join("ledger.refreshmint");
        crate::ledger::new_ledger_at_dir(&ledger_dir).unwrap();

        // Two accounts under different logins, NO gl_account so post-all skips
        // non-transfer entries (leaving them unposted).
        let mut chase_cfg = crate::login_config::LoginConfig::default();
        chase_cfg.accounts.insert(
            "checking".to_string(),
            crate::login_config::LoginAccountConfig { gl_account: None },
        );
        crate::login_config::write_login_config(&ledger_dir, "chase", &chase_cfg).unwrap();
        let mut boa_cfg = crate::login_config::LoginConfig::default();
        boa_cfg.accounts.insert(
            "savings".to_string(),
            crate::login_config::LoginAccountConfig { gl_account: None },
        );
        crate::login_config::write_login_config(&ledger_dir, "boa", &boa_cfg).unwrap();

        let make = |account: &str, amount: &str, desc: &str, id: &str| {
            crate::account_journal::AccountEntry {
                id: id.to_string(),
                date: "2026-01-01".to_string(),
                status: crate::account_journal::EntryStatus::Cleared,
                description: desc.to_string(),
                comment: String::new(),
                evidence: vec![],
                postings: vec![
                    crate::account_journal::EntryPosting {
                        account: account.to_string(),
                        amount: Some(crate::account_journal::SimpleAmount {
                            quantity: amount.to_string(),
                            commodity: "USD".to_string(),
                        }),
                    },
                    crate::account_journal::EntryPosting {
                        account: "Equity:Staging".to_string(),
                        amount: None,
                    },
                ],
                tags: vec![("isTransfer".to_string(), "true".to_string())],
                extracted_by: None,
                posted: None,
                posted_postings: vec![],
            }
        };

        let chase_j =
            crate::account_journal::login_account_journal_path(&ledger_dir, "chase", "checking");
        crate::account_journal::write_journal_at_path(
            &chase_j,
            &[make(
                "Assets:Checking",
                "-100.00",
                "Transfer to savings",
                "out-1",
            )],
        )
        .unwrap();
        let boa_j =
            crate::account_journal::login_account_journal_path(&ledger_dir, "boa", "savings");
        crate::account_journal::write_journal_at_path(
            &boa_j,
            &[make(
                "Assets:Savings",
                "100.00",
                "Transfer from checking",
                "in-1",
            )],
        )
        .unwrap();

        // A TypedRef shaped exactly like automation::login_entry_ref, so the
        // NotTransferLink twin (recorded by unpost) matches this fingerprint.
        let login_ref = |login: &str, label: &str, entry: &str| crate::bookkeeping::TypedRef {
            kind: crate::bookkeeping::TypedRefKind::LoginEntry,
            id: None,
            locator: Some(format!("logins/{login}/accounts/{label}")),
            entry_id: Some(entry.to_string()),
            login_name: Some(login.to_string()),
            label: Some(label.to_string()),
            filename: None,
        };

        // Recorded when the transfer was first linked.
        let transfer_link = crate::automation::create_resolution(
            &ledger_dir,
            crate::automation::NewResolutionInput {
                kind: crate::automation::ResolutionKind::TransferLink,
                subject_refs: vec![
                    login_ref("chase", "checking", "out-1"),
                    login_ref("boa", "savings", "in-1"),
                ],
                parts: vec![],
                notes: None,
                predicate: None,
            },
        )
        .unwrap();

        // Sanity: the pair is detected as a transfer before any negative memory.
        let pre = crate::categorize::suggest_categories(&ledger_dir, "chase", "checking").unwrap();
        assert!(
            pre.get("out-1")
                .and_then(|s| s.transfer_match.as_ref())
                .is_some(),
            "pair should match as a transfer before unpost"
        );

        // Post as a transfer, then unpost one side.
        crate::post::post_login_account_transfer(
            &ledger_dir,
            "chase",
            "checking",
            "out-1",
            "boa",
            "savings",
            "in-1",
            None,
            "test",
        )
        .unwrap();
        crate::post::unpost_login_account_entry(
            &ledger_dir,
            "chase",
            "checking",
            "out-1",
            None,
            None,
            "test",
        )
        .unwrap();

        // Negative memory recorded; TransferLink twin disabled.
        let resolutions = crate::automation::list_resolutions(&ledger_dir).unwrap();
        let ntl = resolutions
            .iter()
            .find(|r| r.kind == crate::automation::ResolutionKind::NotTransferLink);
        assert!(
            ntl.is_some_and(|r| r.status == crate::automation::ResolutionStatus::Active),
            "unpost should record an Active NotTransferLink"
        );
        let tl = resolutions
            .iter()
            .find(|r| r.id == transfer_link.id)
            .unwrap();
        assert_eq!(
            tl.status,
            crate::automation::ResolutionStatus::Disabled,
            "the TransferLink twin should be disabled"
        );

        // suggest_categories no longer reports a transfer_match for either side.
        let chase =
            crate::categorize::suggest_categories(&ledger_dir, "chase", "checking").unwrap();
        assert!(chase
            .get("out-1")
            .and_then(|s| s.transfer_match.as_ref())
            .is_none());
        let boa = crate::categorize::suggest_categories(&ledger_dir, "boa", "savings").unwrap();
        assert!(boa
            .get("in-1")
            .and_then(|s| s.transfer_match.as_ref())
            .is_none());

        // post-all leaves both entries unposted — the loop is dead.
        run_account_post_all_with_dir(&ledger_dir, "chase", &Some("checking".to_string())).unwrap();
        run_account_post_all_with_dir(&ledger_dir, "boa", &Some("savings".to_string())).unwrap();
        let chase_entries = crate::account_journal::read_journal_at_path(&chase_j).unwrap();
        assert!(
            chase_entries[0].posted.is_none(),
            "out-1 must stay unposted after post-all"
        );
        let boa_entries = crate::account_journal::read_journal_at_path(&boa_j).unwrap();
        assert!(
            boa_entries[0].posted.is_none(),
            "in-1 must stay unposted after post-all"
        );
        let gl = fs::read_to_string(ledger_dir.join("general.journal")).unwrap();
        assert!(
            !gl.contains("source: logins/chase"),
            "no transfer should be re-posted, got GL: {gl}"
        );

        let _ = fs::remove_dir_all(&base_dir);
    }

    #[test]
    fn run_account_post_all_policy_recategorizes_unknown_gl() {
        // Post-all's policy pass recategorizes a pre-existing Unknown GL row that
        // matches a rule, even when there are no unposted account entries.
        let base_dir = create_temp_dir();
        let ledger_dir = base_dir.join("ledger.refreshmint");
        fs::create_dir_all(&ledger_dir).unwrap();

        let mut cfg = crate::login_config::LoginConfig::default();
        cfg.accounts.insert(
            "checking".to_string(),
            crate::login_config::LoginAccountConfig {
                gl_account: Some("Assets:Checking".to_string()),
            },
        );
        crate::login_config::write_login_config(&ledger_dir, "chase", &cfg).unwrap();
        // Empty account journal (no unposted entries).
        let jpath =
            crate::account_journal::login_account_journal_path(&ledger_dir, "chase", "checking");
        fs::create_dir_all(jpath.parent().unwrap()).unwrap();
        crate::account_journal::write_journal_at_path(&jpath, &[]).unwrap();
        // A pre-existing Unknown GL row.
        fs::write(
            ledger_dir.join("general.journal"),
            "2026-01-01 SAFEWAY #7  ; id: txn-1\n    ; generated-by: refreshmint-post\n    \
             ; source: logins/chase/accounts/checking:e1\n    \
             Assets:Checking  -10.00 USD\n    Expenses:Unknown\n",
        )
        .unwrap();

        crate::automation::create_resolution(
            &ledger_dir,
            crate::automation::NewResolutionInput {
                kind: crate::automation::ResolutionKind::CategoryRule,
                subject_refs: vec![],
                parts: vec![crate::automation::ResolutionPart {
                    amount: None,
                    account: Some("Expenses:Groceries".to_string()),
                    ref_: None,
                    notes: None,
                }],
                notes: None,
                predicate: Some(crate::automation::CategoryRulePredicate {
                    description_regex: None,
                    normalized_payee: Some("SAFEWAY".to_string()),
                    amount_min: None,
                    amount_max: None,
                }),
            },
        )
        .unwrap();

        let policy_applied = run_account_post_all_with_dir(&ledger_dir, "chase", &None).unwrap();

        let gl = fs::read_to_string(ledger_dir.join("general.journal")).unwrap();
        assert!(
            gl.contains("Expenses:Groceries"),
            "policy pass should recategorize the Unknown row, got: {gl}"
        );
        assert!(
            !gl.contains("Expenses:Unknown"),
            "Unknown row should be gone, got: {gl}"
        );
        // The pre-existing row was fixed by the policy loop (there were no unposted
        // entries to direct-post), so at least one proposal must have applied.
        assert!(
            policy_applied >= 1,
            "policy loop should have recategorized the pre-existing Unknown row"
        );

        let _ = fs::remove_dir_all(&base_dir);
    }

    #[test]
    fn account_extract_rejects_missing_login_without_creating_dir() {
        // Mirrors the GUI test: validation must run before the lock so a typo'd
        // --login doesn't leave a phantom logins/<typo>/ dir behind.
        let base_dir = create_temp_dir();
        let ledger_dir = base_dir.join("ledger.refreshmint");
        fs::create_dir_all(&ledger_dir).unwrap_or_else(|err| panic!("mkdir failed: {err}"));
        let result = run_account_extract_with_dir(&ledger_dir, "typo", "checking", &[]);
        match result {
            Ok(()) => panic!("expected extraction to fail for a missing login"),
            Err(err) => assert!(
                err.to_string().contains("does not exist"),
                "unexpected error: {err}"
            ),
        }
        assert!(
            !ledger_dir.join("logins").join("typo").exists(),
            "a rejected login must not leave a phantom logins/ dir behind"
        );
        let _ = fs::remove_dir_all(&base_dir);
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
    fn secret_add_rejects_invalid_login_name() {
        let invalid_login = run_secret(SecretArgs {
            command: SecretCommand::Add(SecretAddArgs {
                login: "../bad".to_string(),
                domain: "example.com".to_string(),
                name: "password".to_string(),
                value: "secret".to_string(),
            }),
        });
        assert!(expect_err(invalid_login, "invalid login").contains("invalid login"));
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

    #[test]
    fn secret_list_rejects_invalid_login_name() {
        let invalid_login = run_secret(SecretArgs {
            command: SecretCommand::List(SecretListArgs {
                login: "../bad".to_string(),
            }),
        });
        assert!(expect_err(invalid_login, "invalid login").contains("invalid login"));
    }

    #[test]
    fn require_cli_login_name_rejects_path_like_name() {
        let result = require_cli_login_name("login", "../bad");
        assert!(expect_err(result, "invalid login").contains("invalid login"));
    }

    #[test]
    fn require_cli_label_rejects_path_like_label() {
        let result = require_cli_label("../bad");
        assert!(expect_err(result, "invalid label").contains("invalid label"));
    }

    #[test]
    fn require_cli_existing_login_errors_when_missing() {
        let dir = create_temp_dir();
        let result = require_cli_existing_login(&dir, "missing-login");
        assert!(expect_err(result, "missing login").contains("does not exist"));
        if let Err(err) = fs::remove_dir_all(&dir) {
            panic!("failed to clean up temp dir: {err}");
        }
    }

    #[test]
    fn require_cli_existing_login_accepts_existing_login() {
        let dir = create_temp_dir();
        let config = crate::login_config::LoginConfig {
            extension: Some("chase-driver".to_string()),
            accounts: std::collections::BTreeMap::new(),
        };
        if let Err(err) = crate::login_config::write_login_config(&dir, "chase", &config) {
            panic!("failed to write login config: {err}");
        }

        let result = require_cli_existing_login(&dir, "chase");
        assert!(result.is_ok(), "expected existing login, got: {result:?}");
        if let Err(err) = fs::remove_dir_all(&dir) {
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
