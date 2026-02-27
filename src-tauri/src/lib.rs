pub mod cli;
pub mod hledger;
pub mod scrape;
pub mod secret;

pub mod account_config;
pub mod account_journal;
pub mod categorize;
pub mod dedup;
pub mod extract;
pub mod login_config;
pub mod migration;
pub mod operations;
pub mod post;
pub mod transfer_detector;

mod binpath;
mod extension;
mod ledger;
mod ledger_add;
mod ledger_open;
mod version;

use tauri::Manager;

struct UiDebugSession {
    socket_path: std::path::PathBuf,
    join_handle: std::thread::JoinHandle<()>,
}

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, serde::Serialize)]
struct SecretEntry {
    domain: String,
    name: String,
}

#[derive(Clone, Debug, serde::Serialize)]
struct AccountSecretEntry {
    domain: String,
    name: String,
    #[serde(rename = "hasValue")]
    has_value: bool,
}

#[derive(serde::Serialize)]
struct SecretSyncResult {
    required: Vec<SecretEntry>,
    added: Vec<SecretEntry>,
    #[serde(rename = "existingRequired")]
    existing_required: Vec<SecretEntry>,
    extras: Vec<SecretEntry>,
}

static UI_DEBUG_SESSION: std::sync::OnceLock<std::sync::Mutex<Option<UiDebugSession>>> =
    std::sync::OnceLock::new();

fn ui_debug_session_state() -> &'static std::sync::Mutex<Option<UiDebugSession>> {
    UI_DEBUG_SESSION.get_or_init(|| std::sync::Mutex::new(None))
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() -> Result<(), Box<dyn std::error::Error>> {
    let context: tauri::Context<tauri::Wry> = tauri::generate_context!();
    run_with_context(context)
}

pub fn run_with_context(
    context: tauri::Context<tauri::Wry>,
) -> Result<(), Box<dyn std::error::Error>> {
    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_store::Builder::default().build())
        .invoke_handler(tauri::generate_handler![
            new_ledger,
            open_ledger,
            add_transaction,
            validate_transaction,
            add_transaction_text,
            validate_transaction_text,
            list_scrape_extensions,
            load_scrape_extension,
            list_account_secrets,
            sync_account_secrets_for_extension,
            add_account_secret,
            reenter_account_secret,
            remove_account_secret,
            start_scrape_debug_session_for_login,
            start_scrape_debug_session,
            stop_scrape_debug_session,
            get_scrape_debug_session_socket,
            run_scrape_for_login,
            run_scrape,
            list_documents,
            list_login_account_documents,
            read_login_account_document_rows,
            read_attachment_data_url,
            run_extraction,
            run_login_account_extraction,
            get_account_journal,
            get_login_account_journal,
            get_unposted,
            get_login_account_unposted,
            post_entry,
            post_login_account_entry,
            unpost_entry,
            unpost_login_account_entry,
            post_transfer,
            post_login_account_transfer,
            get_unposted_entries_for_transfer,
            sync_gl_transaction,
            suggest_categories,
            suggest_gl_categories,
            recategorize_gl_transaction,
            merge_gl_transfer,
            get_account_config,
            set_account_extension,
            list_logins,
            get_login_config,
            create_login,
            set_login_extension,
            delete_login,
            set_login_account,
            remove_login_account,
            delete_login_account,
            list_login_secrets,
            sync_login_secrets_for_extension,
            add_login_secret,
            reenter_login_secret,
            remove_login_secret,
            clear_login_profile,
            migrate_ledger,
            query_transactions,
        ])
        .setup(|app| {
            binpath::init_from_app(app.handle());
            if cfg!(debug_assertions) {
                app.handle().plugin(
                    tauri_plugin_log::Builder::default()
                        .level(log::LevelFilter::Info)
                        .build(),
                )?;
            }
            Ok(())
        })
        .run(context)
        .map_err(|e| e.into())
}

#[tauri::command]
fn new_ledger(app: tauri::AppHandle, ledger: Option<String>) -> Result<(), String> {
    let target_dir = match ledger {
        Some(path) => crate::ledger::ensure_refreshmint_extension(path.into())
            .map_err(|err| err.to_string())?,
        None => {
            let documents_dir = app.path().document_dir().map_err(|err| err.to_string())?;
            crate::ledger::default_ledger_dir_from_documents(documents_dir)
        }
    };

    crate::ledger::new_ledger_at_dir(&target_dir).map_err(|err| err.to_string())
}

#[tauri::command]
fn open_ledger(ledger: String) -> Result<ledger_open::LedgerView, String> {
    let target_dir = std::path::PathBuf::from(ledger);
    ledger_open::open_ledger_dir(&target_dir).map_err(|err| err.to_string())
}

#[tauri::command]
fn add_transaction(
    ledger: String,
    transaction: ledger_add::NewTransaction,
) -> Result<ledger_open::LedgerView, String> {
    let target_dir = std::path::PathBuf::from(ledger);
    ledger_add::add_transaction_to_ledger(&target_dir, transaction).map_err(|err| err.to_string())
}

#[tauri::command]
fn validate_transaction(
    ledger: String,
    transaction: ledger_add::NewTransaction,
) -> Result<(), String> {
    let target_dir = std::path::PathBuf::from(ledger);
    ledger_add::validate_transaction_only(&target_dir, transaction).map_err(|err| err.to_string())
}

#[tauri::command]
fn add_transaction_text(
    ledger: String,
    transaction: String,
) -> Result<ledger_open::LedgerView, String> {
    let target_dir = std::path::PathBuf::from(ledger);
    ledger_add::add_transaction_text(&target_dir, &transaction).map_err(|err| err.to_string())
}

#[tauri::command]
fn validate_transaction_text(ledger: String, transaction: String) -> Result<(), String> {
    let target_dir = std::path::PathBuf::from(ledger);
    ledger_add::validate_transaction_text(&target_dir, &transaction).map_err(|err| err.to_string())
}

#[tauri::command]
fn list_scrape_extensions(ledger: String) -> Result<Vec<String>, String> {
    let target_dir = std::path::PathBuf::from(ledger);
    crate::ledger::require_refreshmint_extension(&target_dir).map_err(|err| err.to_string())?;
    scrape::list_runnable_extensions(&target_dir).map_err(|err| err.to_string())
}

#[tauri::command]
fn load_scrape_extension(ledger: String, source: String, replace: bool) -> Result<String, String> {
    let target_dir = std::path::PathBuf::from(ledger);
    crate::ledger::require_refreshmint_extension(&target_dir).map_err(|err| err.to_string())?;

    let source_path = std::path::PathBuf::from(source);
    crate::extension::load_extension_from_source(&target_dir, &source_path, replace)
        .map_err(|err| err.to_string())
}

#[tauri::command]
fn list_account_secrets(account: String) -> Result<Vec<AccountSecretEntry>, String> {
    let account = require_non_empty_input("account", account)?;
    let store = crate::secret::SecretStore::new(account);
    let mut entries = store
        .list_with_value_state()
        .map_err(|err| err.to_string())?
        .into_iter()
        .map(|(domain, name, has_value)| AccountSecretEntry {
            domain,
            name,
            has_value,
        })
        .collect::<Vec<_>>();
    entries.sort_by(|a, b| a.domain.cmp(&b.domain).then_with(|| a.name.cmp(&b.name)));
    Ok(entries)
}

fn flatten_declared_secret_entries(
    declared: &scrape::js_api::SecretDeclarations,
) -> Vec<SecretEntry> {
    let mut entries = declared
        .iter()
        .flat_map(|(domain, names)| {
            names.iter().map(|name| SecretEntry {
                domain: domain.clone(),
                name: name.clone(),
            })
        })
        .collect::<Vec<_>>();
    entries.sort_by(|a, b| a.domain.cmp(&b.domain).then_with(|| a.name.cmp(&b.name)));
    entries
}

fn classify_secret_entries(
    required: &[SecretEntry],
    existing: &[SecretEntry],
) -> (Vec<SecretEntry>, Vec<SecretEntry>, Vec<SecretEntry>) {
    let required_keys = required
        .iter()
        .map(|entry| format!("{}/{}", entry.domain, entry.name))
        .collect::<std::collections::BTreeSet<_>>();
    let existing_keys = existing
        .iter()
        .map(|entry| format!("{}/{}", entry.domain, entry.name))
        .collect::<std::collections::BTreeSet<_>>();

    let mut added = Vec::new();
    let mut existing_required = Vec::new();
    for entry in required {
        let key = format!("{}/{}", entry.domain, entry.name);
        if existing_keys.contains(&key) {
            existing_required.push(entry.clone());
        } else {
            added.push(entry.clone());
        }
    }

    let mut extras = Vec::new();
    for entry in existing {
        let key = format!("{}/{}", entry.domain, entry.name);
        if !required_keys.contains(&key) {
            extras.push(entry.clone());
        }
    }

    (added, existing_required, extras)
}

#[tauri::command]
fn sync_account_secrets_for_extension(
    ledger: String,
    account: String,
    extension: String,
) -> Result<SecretSyncResult, String> {
    let target_dir = std::path::PathBuf::from(ledger);
    crate::ledger::require_refreshmint_extension(&target_dir).map_err(|err| err.to_string())?;
    let account = require_non_empty_input("account", account)?;
    let extension = account_config::resolve_extension(&target_dir, &account, Some(&extension))
        .map_err(|err| err.to_string())?;

    let extension_dir = account_config::resolve_extension_dir(&target_dir, &extension);
    let declared =
        scrape::load_manifest_secret_declarations(&extension_dir).map_err(|err| err.to_string())?;
    let required = flatten_declared_secret_entries(&declared);

    let store = crate::secret::SecretStore::new(account);
    let mut existing = store
        .list()
        .map_err(|err| err.to_string())?
        .into_iter()
        .map(|(domain, name)| SecretEntry { domain, name })
        .collect::<Vec<_>>();
    existing.sort_by(|a, b| a.domain.cmp(&b.domain).then_with(|| a.name.cmp(&b.name)));

    let (added, existing_required, extras) = classify_secret_entries(&required, &existing);
    for entry in &added {
        store
            .ensure_indexed(&entry.domain, &entry.name)
            .map_err(|err| err.to_string())?;
    }

    Ok(SecretSyncResult {
        required,
        added,
        existing_required,
        extras,
    })
}

#[tauri::command]
fn add_account_secret(
    account: String,
    domain: String,
    name: String,
    value: String,
) -> Result<(), String> {
    let account = require_non_empty_input("account", account)?;
    let domain = require_non_empty_input("domain", domain)?;
    let name = require_non_empty_input("name", name)?;
    let store = crate::secret::SecretStore::new(account);
    store
        .set(&domain, &name, &value)
        .map_err(|err| err.to_string())
}

#[tauri::command]
fn reenter_account_secret(
    account: String,
    domain: String,
    name: String,
    value: String,
) -> Result<(), String> {
    let account = require_non_empty_input("account", account)?;
    let domain = require_non_empty_input("domain", domain)?;
    let name = require_non_empty_input("name", name)?;
    let store = crate::secret::SecretStore::new(account);
    store
        .set(&domain, &name, &value)
        .map_err(|err| err.to_string())
}

#[tauri::command]
fn remove_account_secret(account: String, domain: String, name: String) -> Result<(), String> {
    let account = require_non_empty_input("account", account)?;
    let domain = require_non_empty_input("domain", domain)?;
    let name = require_non_empty_input("name", name)?;
    let store = crate::secret::SecretStore::new(account);
    store.delete(&domain, &name).map_err(|err| err.to_string())
}

fn require_non_empty_input(field_name: &str, value: String) -> Result<String, String> {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        return Err(format!("{field_name} is required"));
    }
    Ok(trimmed.to_string())
}

fn require_login_name_input(value: String) -> Result<String, String> {
    let login_name = require_non_empty_input("login_name", value)?;
    login_config::validate_label(&login_name)
        .map_err(|err| format!("invalid login_name: {err}"))?;
    Ok(login_name)
}

fn require_label_input(value: String) -> Result<String, String> {
    let label = require_non_empty_input("label", value)?;
    login_config::validate_label(&label).map_err(|err| format!("invalid label: {err}"))?;
    Ok(label)
}

fn require_existing_login(ledger_dir: &std::path::Path, login_name: &str) -> Result<(), String> {
    let config_path = login_config::login_config_path(ledger_dir, login_name);
    if config_path.exists() {
        Ok(())
    } else {
        Err(format!("login '{login_name}' does not exist"))
    }
}

#[tauri::command]
fn start_scrape_debug_session_for_login(
    ledger: String,
    login_name: String,
) -> Result<String, String> {
    let login_name = require_login_name_input(login_name)?;

    let target_dir = std::path::PathBuf::from(ledger);
    crate::ledger::require_refreshmint_extension(&target_dir).map_err(|err| err.to_string())?;
    require_existing_login(&target_dir, &login_name)?;
    let extension = login_config::resolve_login_extension(&target_dir, &login_name)?;
    let socket_path = crate::scrape::debug::default_debug_socket_path(&login_name)
        .map_err(|err| err.to_string())?;

    let state = ui_debug_session_state();
    let mut guard = state
        .lock()
        .map_err(|_| "failed to acquire debug session lock".to_string())?;

    if let Some(session) = guard.as_ref() {
        if !session.join_handle.is_finished() {
            return Err(format!(
                "a debug session is already running at {}",
                session.socket_path.display()
            ));
        }
    }

    if let Some(finished) = guard.take() {
        let _ = finished.join_handle.join();
    }

    let config = crate::scrape::debug::DebugStartConfig {
        login_name,
        extension_name: extension,
        ledger_dir: target_dir,
        profile_override: None,
        socket_path: Some(socket_path.clone()),
        prompt_requires_override: false,
    };
    let socket_for_thread = socket_path.clone();
    let join_handle = std::thread::spawn(move || {
        if let Err(err) = crate::scrape::debug::run_debug_session(config) {
            eprintln!("debug session exited with error: {err}");
        }
    });

    *guard = Some(UiDebugSession {
        socket_path: socket_for_thread,
        join_handle,
    });

    Ok(socket_path.to_string_lossy().to_string())
}

#[tauri::command]
fn start_scrape_debug_session(ledger: String, account: String) -> Result<String, String> {
    // Compatibility alias for legacy account-keyed callers.
    let login_name = require_non_empty_input("account", account)?;
    start_scrape_debug_session_for_login(ledger, login_name)
}

#[tauri::command]
fn stop_scrape_debug_session() -> Result<(), String> {
    let state = ui_debug_session_state();
    let session = {
        let mut guard = state
            .lock()
            .map_err(|_| "failed to acquire debug session lock".to_string())?;
        guard.take()
    };
    let Some(session) = session else {
        return Ok(());
    };

    let stop_result = crate::scrape::debug::stop_debug_session(&session.socket_path);
    let _ = session.join_handle.join();

    if let Err(err) = stop_result {
        return Err(err.to_string());
    }
    Ok(())
}

#[tauri::command]
fn get_scrape_debug_session_socket() -> Result<Option<String>, String> {
    let state = ui_debug_session_state();
    let finished = {
        let mut guard = state
            .lock()
            .map_err(|_| "failed to acquire debug session lock".to_string())?;
        let Some(session) = guard.as_ref() else {
            return Ok(None);
        };
        if session.join_handle.is_finished() {
            guard.take()
        } else {
            return Ok(Some(session.socket_path.to_string_lossy().to_string()));
        }
    };
    if let Some(session) = finished {
        let _ = session.join_handle.join();
    }
    Ok(None)
}

#[tauri::command]
async fn run_scrape_for_login(ledger: String, login_name: String) -> Result<(), String> {
    let login_name = require_login_name_input(login_name)?;

    let target_dir = std::path::PathBuf::from(ledger);
    crate::ledger::require_refreshmint_extension(&target_dir).map_err(|err| err.to_string())?;
    require_existing_login(&target_dir, &login_name)?;

    let extension = login_config::resolve_login_extension(&target_dir, &login_name)
        .map_err(|err| err.to_string())?;

    let config = scrape::ScrapeConfig {
        login_name,
        extension_name: extension,
        ledger_dir: target_dir,
        profile_override: None,
        prompt_overrides: scrape::js_api::PromptOverrides::new(),
        prompt_requires_override: false,
    };

    tokio::task::spawn_blocking(move || scrape::run_scrape(config).map_err(|err| err.to_string()))
        .await
        .map_err(|err| err.to_string())?
        .map_err(|err| err.to_string())
}

#[tauri::command]
async fn run_scrape(ledger: String, account: String) -> Result<(), String> {
    let login_name = require_non_empty_input("account", account)?;
    run_scrape_for_login(ledger, login_name).await
}

#[tauri::command]
fn list_documents(
    ledger: String,
    account_name: String,
) -> Result<Vec<extract::DocumentWithInfo>, String> {
    let target_dir = std::path::PathBuf::from(ledger);
    let account_name = require_non_empty_input("account_name", account_name)?;
    extract::list_documents(&target_dir, &account_name).map_err(|err| err.to_string())
}

#[tauri::command]
fn list_login_account_documents(
    ledger: String,
    login_name: String,
    label: String,
) -> Result<Vec<extract::DocumentWithInfo>, String> {
    let target_dir = std::path::PathBuf::from(ledger);
    let login_name = require_login_name_input(login_name)?;
    let label = require_label_input(label)?;
    extract::list_documents_for_login_account(&target_dir, &login_name, &label)
        .map_err(|err| err.to_string())
}

#[tauri::command]
fn read_login_account_document_rows(
    ledger: String,
    login_name: String,
    label: String,
    document_name: String,
) -> Result<Vec<Vec<String>>, String> {
    let target_dir = std::path::PathBuf::from(ledger);
    let login_name = require_login_name_input(login_name)?;
    let label = require_label_input(label)?;
    extract::read_login_account_document_csv_rows(&target_dir, &login_name, &label, &document_name)
        .map_err(|err| err.to_string())
}

#[tauri::command]
fn read_attachment_data_url(ledger: String, filename: String) -> Result<String, String> {
    let ledger_dir = std::path::Path::new(&ledger);
    extract::read_attachment_data_url(ledger_dir, &filename).map_err(|e| e.to_string())
}

#[tauri::command]
fn run_extraction(
    ledger: String,
    account_name: String,
    document_names: Vec<String>,
) -> Result<usize, String> {
    let target_dir = std::path::PathBuf::from(ledger);
    let account_name = require_non_empty_input("account_name", account_name)?;
    let extension_name = account_config::resolve_extension(&target_dir, &account_name, None)
        .map_err(|err| err.to_string())?;

    let result =
        extract::run_extraction(&target_dir, &account_name, &extension_name, &document_names)
            .map_err(|err| err.to_string())?;

    // Run dedup on extracted transactions
    let existing_entries =
        account_journal::read_journal(&target_dir, &account_name).map_err(|err| err.to_string())?;

    let config = dedup::DedupConfig::default();
    let mut all_updated = existing_entries;
    let mut new_count = 0;

    // Process each document's transactions through dedup
    for doc_name in &result.document_names {
        let doc_txns: Vec<_> = result
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

        let actions = dedup::run_dedup(&all_updated, &doc_txns, doc_name, &config);
        new_count += actions
            .iter()
            .filter(|a| matches!(a.result, dedup::DedupResult::New))
            .count();

        // Determine default account and unreconciled equity from existing entries or manifest
        let default_account = all_updated
            .first()
            .and_then(|e| e.postings.first())
            .map(|p| p.account.clone())
            .unwrap_or_else(|| format!("Assets:{account_name}"));
        let unreconciled_equity = format!("Equity:Unreconciled:{account_name}");

        all_updated = dedup::apply_dedup_actions(
            &target_dir,
            &account_name,
            all_updated,
            &actions,
            &default_account,
            &unreconciled_equity,
            Some(&format!("{extension_name}:latest")),
        )
        .map_err(|err| err.to_string())?;
    }

    // Write updated journal
    account_journal::write_journal(&target_dir, &account_name, &all_updated)
        .map_err(|err| err.to_string())?;

    Ok(new_count)
}

#[tauri::command]
fn run_login_account_extraction(
    ledger: String,
    login_name: String,
    label: String,
    document_names: Vec<String>,
) -> Result<usize, String> {
    let target_dir = std::path::PathBuf::from(ledger);
    let login_name = require_login_name_input(login_name)?;
    let label = require_label_input(label)?;

    let extension_name = login_config::resolve_login_extension(&target_dir, &login_name)
        .map_err(|err| err.to_string())?;
    let gl_account = resolve_login_account_gl_account(&target_dir, &login_name, &label)?;

    let result = extract::run_extraction_for_login_account(
        &target_dir,
        &login_name,
        &label,
        &gl_account,
        &extension_name,
        &document_names,
    )
    .map_err(|err| err.to_string())?;

    let journal_path =
        account_journal::login_account_journal_path(&target_dir, &login_name, &label);
    let existing_entries =
        account_journal::read_journal_at_path(&journal_path).map_err(|err| err.to_string())?;

    let config = dedup::DedupConfig::default();
    let mut all_updated = existing_entries;
    let mut new_count = 0usize;

    for doc_name in &result.document_names {
        let doc_txns: Vec<_> = result
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

        let actions = dedup::run_dedup(&all_updated, &doc_txns, doc_name, &config);
        new_count += actions
            .iter()
            .filter(|a| matches!(a.result, dedup::DedupResult::New))
            .count();

        let default_account = all_updated
            .first()
            .and_then(|e| e.postings.first())
            .map(|p| p.account.clone())
            .unwrap_or_else(|| gl_account.clone());
        let unreconciled_equity = format!("Equity:Unreconciled:{login_name}:{label}");

        all_updated = dedup::apply_dedup_actions_for_login_account(
            &target_dir,
            (&login_name, &label),
            all_updated,
            &actions,
            &default_account,
            &unreconciled_equity,
            Some(&format!("{extension_name}:latest")),
        )
        .map_err(|err| err.to_string())?;
    }

    account_journal::write_journal_at_path(&journal_path, &all_updated)
        .map_err(|err| err.to_string())?;

    Ok(new_count)
}

#[tauri::command]
fn get_account_config(
    ledger: String,
    account_name: String,
) -> Result<account_config::AccountConfig, String> {
    let target_dir = std::path::PathBuf::from(ledger);
    let account_name = require_non_empty_input("account_name", account_name)?;
    Ok(account_config::read_account_config(
        &target_dir,
        &account_name,
    ))
}

#[tauri::command]
fn set_account_extension(
    ledger: String,
    account_name: String,
    extension: String,
) -> Result<(), String> {
    let target_dir = std::path::PathBuf::from(ledger);
    let account_name = require_non_empty_input("account_name", account_name)?;
    let extension = extension.trim().to_string();
    let ext_value = if extension.is_empty() {
        None
    } else {
        Some(extension)
    };
    let config = account_config::AccountConfig {
        extension: ext_value,
    };
    account_config::write_account_config(&target_dir, &account_name, &config)
        .map_err(|err| err.to_string())
}

fn evidence_ref_matches_document(evidence_ref: &str, document_name: &str) -> bool {
    evidence_ref.starts_with(document_name)
        && evidence_ref
            .get(document_name.len()..)
            .map(|rest| rest.starts_with(':') || rest.starts_with('#'))
            .unwrap_or(false)
}

fn resolve_login_account_gl_account(
    ledger_dir: &std::path::Path,
    login_name: &str,
    label: &str,
) -> Result<String, String> {
    let config = login_config::read_login_config(ledger_dir, login_name);
    let account_cfg = config
        .accounts
        .get(label)
        .ok_or_else(|| format!("label '{label}' not found in login '{login_name}'"))?;

    let gl_account = account_cfg
        .gl_account
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| {
            format!(
                "login '{login_name}' label '{label}' is ignored (gl_account is null); set a GL account first"
            )
        })?
        .to_string();

    if let Some(conflict) = login_config::find_gl_account_conflicts(ledger_dir)
        .into_iter()
        .find(|conflict| conflict.gl_account == gl_account)
    {
        let entries = conflict
            .entries
            .iter()
            .map(|entry| format!("{}/{}", entry.login_name, entry.label))
            .collect::<Vec<_>>()
            .join(", ");
        return Err(format!(
            "GL account '{}' has conflicting login mappings: {}; resolve conflicts first",
            conflict.gl_account, entries
        ));
    }

    Ok(gl_account)
}

// --- Login CRUD commands ---

#[tauri::command]
fn list_logins(ledger: String) -> Result<Vec<String>, String> {
    let target_dir = std::path::PathBuf::from(ledger);
    crate::ledger::require_refreshmint_extension(&target_dir).map_err(|err| err.to_string())?;
    login_config::list_logins(&target_dir).map_err(|e| e.to_string())
}

#[tauri::command]
fn get_login_config(
    ledger: String,
    login_name: String,
) -> Result<login_config::LoginConfig, String> {
    let target_dir = std::path::PathBuf::from(ledger);
    let login_name = require_login_name_input(login_name)?;
    Ok(login_config::read_login_config(&target_dir, &login_name))
}

#[tauri::command]
fn create_login(ledger: String, login_name: String, extension: String) -> Result<(), String> {
    let target_dir = std::path::PathBuf::from(ledger);
    crate::ledger::require_refreshmint_extension(&target_dir).map_err(|err| err.to_string())?;
    let login_name = require_login_name_input(login_name)?;

    // Check if login already exists
    let config_path = login_config::login_config_path(&target_dir, &login_name);
    if config_path.exists() {
        return Err(format!("login '{login_name}' already exists"));
    }

    let extension = extension.trim().to_string();
    let ext_value = if extension.is_empty() {
        None
    } else {
        Some(extension)
    };

    let config = login_config::LoginConfig {
        extension: ext_value,
        accounts: std::collections::BTreeMap::new(),
    };
    login_config::write_login_config(&target_dir, &login_name, &config)
        .map_err(|err| err.to_string())
}

#[tauri::command]
fn set_login_extension(
    ledger: String,
    login_name: String,
    extension: String,
) -> Result<(), String> {
    let target_dir = std::path::PathBuf::from(ledger);
    let login_name = require_login_name_input(login_name)?;
    require_existing_login(&target_dir, &login_name)?;
    let _lock = login_config::acquire_login_lock(&target_dir, &login_name)
        .map_err(|err| err.to_string())?;

    let mut config = login_config::read_login_config(&target_dir, &login_name);
    let extension = extension.trim().to_string();
    config.extension = if extension.is_empty() {
        None
    } else {
        Some(extension)
    };
    login_config::write_login_config(&target_dir, &login_name, &config)
        .map_err(|err| err.to_string())
}

#[tauri::command]
fn delete_login(ledger: String, login_name: String) -> Result<(), String> {
    let target_dir = std::path::PathBuf::from(ledger);
    let login_name = require_login_name_input(login_name)?;
    login_config::delete_login(&target_dir, &login_name).map_err(|err| err.to_string())
}

#[tauri::command]
fn set_login_account(
    ledger: String,
    login_name: String,
    label: String,
    gl_account: Option<String>,
) -> Result<(), String> {
    let target_dir = std::path::PathBuf::from(ledger);
    let login_name = require_login_name_input(login_name)?;
    require_existing_login(&target_dir, &login_name)?;
    let label = require_label_input(label)?;

    let _lock = login_config::acquire_login_lock(&target_dir, &login_name)
        .map_err(|err| err.to_string())?;

    // Check GL account uniqueness if setting a non-null GL account
    let gl_account = gl_account
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty());
    if let Some(ref gl) = gl_account {
        login_config::check_gl_account_uniqueness(&target_dir, &login_name, &label, gl)?;
    }

    let mut config = login_config::read_login_config(&target_dir, &login_name);
    config
        .accounts
        .insert(label, login_config::LoginAccountConfig { gl_account });
    login_config::write_login_config(&target_dir, &login_name, &config)
        .map_err(|err| err.to_string())
}

#[tauri::command]
fn remove_login_account(ledger: String, login_name: String, label: String) -> Result<(), String> {
    let target_dir = std::path::PathBuf::from(ledger);
    let login_name = require_login_name_input(login_name)?;
    require_existing_login(&target_dir, &login_name)?;
    let label = require_label_input(label)?;

    let _lock = login_config::acquire_login_lock(&target_dir, &login_name)
        .map_err(|err| err.to_string())?;

    login_config::remove_login_account(&target_dir, &login_name, &label)
        .map_err(|err| err.to_string())
}

#[tauri::command]
fn delete_login_account(ledger: String, login_name: String, label: String) -> Result<(), String> {
    remove_login_account(ledger, login_name, label)
}

// --- Login-keyed secret commands ---

#[tauri::command]
fn list_login_secrets(login_name: String) -> Result<Vec<AccountSecretEntry>, String> {
    let login_name = require_login_name_input(login_name)?;
    let store = crate::secret::SecretStore::new(format!("login/{login_name}"));
    let mut entries = store
        .list_with_value_state()
        .map_err(|err| err.to_string())?
        .into_iter()
        .map(|(domain, name, has_value)| AccountSecretEntry {
            domain,
            name,
            has_value,
        })
        .collect::<Vec<_>>();
    entries.sort_by(|a, b| a.domain.cmp(&b.domain).then_with(|| a.name.cmp(&b.name)));
    Ok(entries)
}

#[tauri::command]
fn sync_login_secrets_for_extension(
    ledger: String,
    login_name: String,
    extension: String,
) -> Result<SecretSyncResult, String> {
    let target_dir = std::path::PathBuf::from(ledger);
    crate::ledger::require_refreshmint_extension(&target_dir).map_err(|err| err.to_string())?;
    let login_name = require_login_name_input(login_name)?;
    require_existing_login(&target_dir, &login_name)?;
    let extension = extension.trim().to_string();

    let extension_dir = account_config::resolve_extension_dir(&target_dir, &extension);
    let declared =
        scrape::load_manifest_secret_declarations(&extension_dir).map_err(|err| err.to_string())?;
    let required = flatten_declared_secret_entries(&declared);

    let store = crate::secret::SecretStore::new(format!("login/{login_name}"));
    let mut existing = store
        .list()
        .map_err(|err| err.to_string())?
        .into_iter()
        .map(|(domain, name)| SecretEntry { domain, name })
        .collect::<Vec<_>>();
    existing.sort_by(|a, b| a.domain.cmp(&b.domain).then_with(|| a.name.cmp(&b.name)));

    let (added, existing_required, extras) = classify_secret_entries(&required, &existing);
    for entry in &added {
        store
            .ensure_indexed(&entry.domain, &entry.name)
            .map_err(|err| err.to_string())?;
    }

    Ok(SecretSyncResult {
        required,
        added,
        existing_required,
        extras,
    })
}

#[tauri::command]
fn add_login_secret(
    login_name: String,
    domain: String,
    name: String,
    value: String,
) -> Result<(), String> {
    let login_name = require_login_name_input(login_name)?;
    let domain = require_non_empty_input("domain", domain)?;
    let name = require_non_empty_input("name", name)?;
    let store = crate::secret::SecretStore::new(format!("login/{login_name}"));
    store
        .set(&domain, &name, &value)
        .map_err(|err| err.to_string())
}

#[tauri::command]
fn reenter_login_secret(
    login_name: String,
    domain: String,
    name: String,
    value: String,
) -> Result<(), String> {
    let login_name = require_login_name_input(login_name)?;
    let domain = require_non_empty_input("domain", domain)?;
    let name = require_non_empty_input("name", name)?;
    let store = crate::secret::SecretStore::new(format!("login/{login_name}"));
    store
        .set(&domain, &name, &value)
        .map_err(|err| err.to_string())
}

#[tauri::command]
fn remove_login_secret(login_name: String, domain: String, name: String) -> Result<(), String> {
    let login_name = require_login_name_input(login_name)?;
    let domain = require_non_empty_input("domain", domain)?;
    let name = require_non_empty_input("name", name)?;
    let store = crate::secret::SecretStore::new(format!("login/{login_name}"));
    store.delete(&domain, &name).map_err(|err| err.to_string())
}

#[tauri::command]
fn clear_login_profile(ledger: String, login_name: String) -> Result<(), String> {
    let target_dir = std::path::PathBuf::from(ledger);
    let login_name = require_login_name_input(login_name)?;
    require_existing_login(&target_dir, &login_name)?;

    let lock = login_config::acquire_login_lock(&target_dir, &login_name)
        .map_err(|err| err.to_string())?;
    scrape::profile::clear_login_profile(&target_dir, &login_name, &lock)
        .map_err(|err| err.to_string())
}

#[tauri::command]
fn migrate_ledger(ledger: String, dry_run: bool) -> Result<migration::MigrationOutcome, String> {
    let target_dir = std::path::PathBuf::from(ledger);
    crate::ledger::require_refreshmint_extension(&target_dir).map_err(|err| err.to_string())?;
    migration::migrate_ledger(&target_dir, dry_run).map_err(|err| err.to_string())
}

#[tauri::command]
fn query_transactions(
    ledger: String,
    query: String,
) -> Result<Vec<ledger_open::TransactionRow>, String> {
    let dir = std::path::PathBuf::from(&ledger);
    let journal_path = dir.join("general.journal");
    let tokens = ledger_open::tokenize_query(&query);
    ledger_open::run_hledger_print_with_query(&journal_path, &tokens)
        .map(|txns| ledger_open::build_transaction_rows(&txns))
        .map_err(|e| e.to_string())
}

#[derive(serde::Serialize)]
struct AccountJournalEntry {
    id: String,
    date: String,
    status: String,
    description: String,
    comment: String,
    evidence: Vec<String>,
    posted: Option<String>,
    #[serde(rename = "isTransfer")]
    is_transfer: bool,
    /// Quantity of the first posting (no commodity symbol), e.g. `"-21.32"`.
    amount: Option<String>,
    /// All tags on the entry, as `(key, value)` pairs.
    tags: Vec<(String, String)>,
}

#[tauri::command]
fn get_account_journal(
    ledger: String,
    account_name: String,
) -> Result<Vec<AccountJournalEntry>, String> {
    let target_dir = std::path::PathBuf::from(ledger);
    let account_name = require_non_empty_input("account_name", account_name)?;
    let entries =
        account_journal::read_journal(&target_dir, &account_name).map_err(|err| err.to_string())?;
    Ok(map_account_journal_entries(entries))
}

#[tauri::command]
fn get_login_account_journal(
    ledger: String,
    login_name: String,
    label: String,
) -> Result<Vec<AccountJournalEntry>, String> {
    let target_dir = std::path::PathBuf::from(ledger);
    let login_name = require_login_name_input(login_name)?;
    let label = require_label_input(label)?;
    let journal_path =
        account_journal::login_account_journal_path(&target_dir, &login_name, &label);
    let entries =
        account_journal::read_journal_at_path(&journal_path).map_err(|err| err.to_string())?;
    Ok(map_account_journal_entries(entries))
}

#[tauri::command]
fn get_unposted(ledger: String, account_name: String) -> Result<Vec<AccountJournalEntry>, String> {
    let target_dir = std::path::PathBuf::from(ledger);
    let account_name = require_non_empty_input("account_name", account_name)?;
    let entries = post::get_unposted(&target_dir, &account_name).map_err(|err| err.to_string())?;
    Ok(map_account_journal_entries(entries))
}

#[tauri::command]
fn get_login_account_unposted(
    ledger: String,
    login_name: String,
    label: String,
) -> Result<Vec<AccountJournalEntry>, String> {
    let target_dir = std::path::PathBuf::from(ledger);
    let login_name = require_login_name_input(login_name)?;
    let label = require_label_input(label)?;
    let entries = post::get_unposted_login_account(&target_dir, &login_name, &label)
        .map_err(|err| err.to_string())?;
    Ok(map_account_journal_entries(entries))
}

#[tauri::command]
fn post_entry(
    ledger: String,
    account_name: String,
    entry_id: String,
    counterpart_account: String,
    posting_index: Option<usize>,
) -> Result<String, String> {
    let target_dir = std::path::PathBuf::from(ledger);
    let account_name = require_non_empty_input("account_name", account_name)?;
    let entry_id = require_non_empty_input("entry_id", entry_id)?;
    let counterpart_account = require_non_empty_input("counterpart_account", counterpart_account)?;

    post::post_entry(
        &target_dir,
        &account_name,
        &entry_id,
        &counterpart_account,
        posting_index,
    )
    .map_err(|err| err.to_string())
}

#[tauri::command]
fn post_login_account_entry(
    ledger: String,
    login_name: String,
    label: String,
    entry_id: String,
    counterpart_account: String,
    posting_index: Option<usize>,
) -> Result<String, String> {
    let target_dir = std::path::PathBuf::from(ledger);
    let login_name = require_login_name_input(login_name)?;
    let label = require_label_input(label)?;
    let entry_id = require_non_empty_input("entry_id", entry_id)?;
    let counterpart_account = require_non_empty_input("counterpart_account", counterpart_account)?;

    // Reject reconciliation when this login label's GL mapping is unset or conflicting.
    let _ = resolve_login_account_gl_account(&target_dir, &login_name, &label)?;

    post::post_login_account_entry(
        &target_dir,
        &login_name,
        &label,
        &entry_id,
        &counterpart_account,
        posting_index,
    )
    .map_err(|err| err.to_string())
}

#[tauri::command]
fn unpost_entry(
    ledger: String,
    account_name: String,
    entry_id: String,
    posting_index: Option<usize>,
) -> Result<(), String> {
    let target_dir = std::path::PathBuf::from(ledger);
    let account_name = require_non_empty_input("account_name", account_name)?;
    let entry_id = require_non_empty_input("entry_id", entry_id)?;

    post::unpost_entry(&target_dir, &account_name, &entry_id, posting_index)
        .map_err(|err| err.to_string())
}

#[tauri::command]
fn unpost_login_account_entry(
    ledger: String,
    login_name: String,
    label: String,
    entry_id: String,
    posting_index: Option<usize>,
) -> Result<(), String> {
    let target_dir = std::path::PathBuf::from(ledger);
    let login_name = require_login_name_input(login_name)?;
    let label = require_label_input(label)?;
    let entry_id = require_non_empty_input("entry_id", entry_id)?;

    post::unpost_login_account_entry(&target_dir, &login_name, &label, &entry_id, posting_index)
        .map_err(|err| err.to_string())
}

#[tauri::command]
fn post_transfer(
    ledger: String,
    account1: String,
    entry_id1: String,
    account2: String,
    entry_id2: String,
) -> Result<String, String> {
    let target_dir = std::path::PathBuf::from(ledger);
    let account1 = require_non_empty_input("account1", account1)?;
    let entry_id1 = require_non_empty_input("entry_id1", entry_id1)?;
    let account2 = require_non_empty_input("account2", account2)?;
    let entry_id2 = require_non_empty_input("entry_id2", entry_id2)?;

    post::post_transfer(&target_dir, &account1, &entry_id1, &account2, &entry_id2)
        .map_err(|err| err.to_string())
}

#[derive(serde::Serialize)]
struct UnpostedTransferResult {
    #[serde(rename = "loginName")]
    login_name: String,
    label: String,
    entry: AccountJournalEntry,
}

#[tauri::command]
fn get_unposted_entries_for_transfer(
    ledger: String,
    exclude_login: String,
    exclude_label: String,
    source_entry_id: String,
) -> Result<Vec<UnpostedTransferResult>, String> {
    let target_dir = std::path::PathBuf::from(ledger);
    let exclude_login = require_login_name_input(exclude_login)?;
    let exclude_label = require_label_input(exclude_label)?;
    let source_entry_id = require_non_empty_input("source_entry_id", source_entry_id)?;
    let triples = post::get_unposted_entries_for_transfer(
        &target_dir,
        &exclude_login,
        &exclude_label,
        &source_entry_id,
    )
    .map_err(|err| err.to_string())?;
    let results = triples
        .into_iter()
        .flat_map(|(login_name, label, e)| {
            map_account_journal_entries(vec![e])
                .into_iter()
                .map(move |entry| UnpostedTransferResult {
                    login_name: login_name.clone(),
                    label: label.clone(),
                    entry,
                })
        })
        .collect();
    Ok(results)
}

#[tauri::command]
fn post_login_account_transfer(
    ledger: String,
    login_name1: String,
    label1: String,
    entry_id1: String,
    login_name2: String,
    label2: String,
    entry_id2: String,
) -> Result<String, String> {
    let target_dir = std::path::PathBuf::from(ledger);
    let login_name1 = require_login_name_input(login_name1)?;
    let label1 = require_label_input(label1)?;
    let entry_id1 = require_non_empty_input("entry_id1", entry_id1)?;
    let login_name2 = require_login_name_input(login_name2)?;
    let label2 = require_label_input(label2)?;
    let entry_id2 = require_non_empty_input("entry_id2", entry_id2)?;

    post::post_login_account_transfer(
        &target_dir,
        &login_name1,
        &label1,
        &entry_id1,
        &login_name2,
        &label2,
        &entry_id2,
    )
    .map_err(|err| err.to_string())
}

#[tauri::command]
fn sync_gl_transaction(
    ledger: String,
    login_name: String,
    label: String,
    entry_id: String,
) -> Result<String, String> {
    let target_dir = std::path::PathBuf::from(ledger);
    let login_name = require_login_name_input(login_name)?;
    let label = require_label_input(label)?;
    let entry_id = require_non_empty_input("entry_id", entry_id)?;

    post::sync_gl_transaction(&target_dir, &login_name, &label, &entry_id)
        .map_err(|err| err.to_string())
}

#[tauri::command]
fn suggest_categories(
    ledger: String,
    login_name: String,
    label: String,
) -> Result<std::collections::HashMap<String, categorize::CategoryResult>, String> {
    let target_dir = std::path::PathBuf::from(ledger);
    let login_name = require_login_name_input(login_name)?;
    let label = require_label_input(label)?;

    categorize::suggest_categories(&target_dir, &login_name, &label).map_err(|err| err.to_string())
}

#[tauri::command]
fn suggest_gl_categories(
    ledger: String,
) -> Result<std::collections::HashMap<String, categorize::GlCategoryResult>, String> {
    let target_dir = std::path::PathBuf::from(ledger);
    categorize::suggest_gl_categories(&target_dir).map_err(|err| err.to_string())
}

#[tauri::command]
fn recategorize_gl_transaction(
    ledger: String,
    txn_id: String,
    new_account: String,
) -> Result<(), String> {
    let target_dir = std::path::PathBuf::from(ledger);
    let txn_id = require_non_empty_input("txn_id", txn_id)?;
    let new_account = require_non_empty_input("new_account", new_account)?;
    post::recategorize_gl_transaction(&target_dir, &txn_id, &new_account)
        .map_err(|err| err.to_string())
}

#[tauri::command]
fn merge_gl_transfer(ledger: String, txn_id_1: String, txn_id_2: String) -> Result<String, String> {
    let target_dir = std::path::PathBuf::from(ledger);
    let txn_id_1 = require_non_empty_input("txn_id_1", txn_id_1)?;
    let txn_id_2 = require_non_empty_input("txn_id_2", txn_id_2)?;
    post::merge_gl_transfer(&target_dir, &txn_id_1, &txn_id_2).map_err(|err| err.to_string())
}

fn map_account_journal_entries(
    entries: Vec<account_journal::AccountEntry>,
) -> Vec<AccountJournalEntry> {
    entries
        .into_iter()
        .map(|e| {
            let is_transfer = transfer_detector::is_probable_transfer(&e.description);
            let status = match e.status {
                account_journal::EntryStatus::Cleared => "cleared",
                account_journal::EntryStatus::Pending => "pending",
                account_journal::EntryStatus::Unmarked => "unmarked",
            };
            let amount = e
                .postings
                .first()
                .and_then(|p| p.amount.as_ref())
                .map(|a| a.quantity.clone());
            let tags = e.tags.clone();
            AccountJournalEntry {
                id: e.id,
                date: e.date,
                status: status.to_string(),
                description: e.description,
                comment: e.comment,
                evidence: e.evidence,
                posted: e.posted,
                is_transfer,
                amount,
                tags,
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::{
        classify_secret_entries, delete_login_account, evidence_ref_matches_document,
        flatten_declared_secret_entries, require_existing_login, require_label_input,
        require_login_name_input, require_non_empty_input, SecretEntry,
    };
    use std::collections::{BTreeMap, BTreeSet};
    use std::fs;
    use std::path::PathBuf;
    use std::time::{SystemTime, UNIX_EPOCH};

    fn create_temp_dir(prefix: &str) -> PathBuf {
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos();
        let dir =
            std::env::temp_dir().join(format!("refreshmint-{prefix}-{}-{now}", std::process::id()));
        if let Err(err) = fs::create_dir_all(&dir) {
            panic!("failed to create temp dir: {err}");
        }
        dir
    }

    #[test]
    fn require_non_empty_input_trims() {
        let value = require_non_empty_input("account", " Assets:Cash ".to_string());
        match value {
            Ok(account) => assert_eq!(account, "Assets:Cash"),
            Err(err) => panic!("expected trimmed account, got error: {err}"),
        }
    }

    #[test]
    fn require_non_empty_input_rejects_blank() {
        let value = require_non_empty_input("account", " ".to_string());
        match value {
            Ok(_) => panic!("expected validation error for blank input"),
            Err(err) => assert_eq!(err, "account is required"),
        }
    }

    #[test]
    fn require_login_name_input_accepts_valid_login_name() {
        let value = require_login_name_input("chase-main".to_string());
        match value {
            Ok(login_name) => assert_eq!(login_name, "chase-main"),
            Err(err) => panic!("expected valid login name, got error: {err}"),
        }
    }

    #[test]
    fn require_login_name_input_rejects_path_like_name() {
        let value = require_login_name_input("../chase".to_string());
        match value {
            Ok(_) => panic!("expected validation error for invalid login name"),
            Err(err) => assert!(err.contains("invalid login_name")),
        }
    }

    #[test]
    fn require_label_input_accepts_valid_label() {
        let value = require_label_input("checking.main".to_string());
        match value {
            Ok(label) => assert_eq!(label, "checking.main"),
            Err(err) => panic!("expected valid label, got error: {err}"),
        }
    }

    #[test]
    fn require_label_input_rejects_path_like_label() {
        let value = require_label_input("bad/label".to_string());
        match value {
            Ok(_) => panic!("expected validation error for invalid label"),
            Err(err) => assert!(err.contains("invalid label")),
        }
    }

    #[test]
    fn require_existing_login_rejects_missing_login() {
        let dir = create_temp_dir("require-existing-login-missing");
        let result = require_existing_login(&dir, "missing-login");
        match result {
            Ok(()) => panic!("expected error for missing login"),
            Err(err) => assert!(err.contains("does not exist")),
        }
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn require_existing_login_accepts_existing_login() {
        let dir = create_temp_dir("require-existing-login-ok");
        let config = crate::login_config::LoginConfig {
            extension: Some("chase-driver".to_string()),
            accounts: BTreeMap::new(),
        };
        if let Err(err) = crate::login_config::write_login_config(&dir, "chase", &config) {
            panic!("failed to write login config: {err}");
        }

        let result = require_existing_login(&dir, "chase");
        assert!(result.is_ok(), "expected existing login, got: {result:?}");
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn evidence_ref_matches_document_requires_delimiter() {
        assert!(evidence_ref_matches_document("foo.csv:1:1", "foo.csv"));
        assert!(evidence_ref_matches_document("foo.csv#row:1", "foo.csv"));
        assert!(!evidence_ref_matches_document("foo.csvx:1:1", "foo.csv"));
        assert!(!evidence_ref_matches_document("foo.csv", "foo.csv"));
    }

    #[test]
    fn flatten_declared_secret_entries_expands_and_sorts() {
        let mut declared = BTreeMap::<String, BTreeSet<String>>::new();
        declared.insert(
            "b.com".to_string(),
            ["token".to_string()].into_iter().collect(),
        );
        declared.insert(
            "a.com".to_string(),
            ["password".to_string(), "username".to_string()]
                .into_iter()
                .collect(),
        );

        let entries = flatten_declared_secret_entries(&declared);
        assert_eq!(
            entries,
            vec![
                SecretEntry {
                    domain: "a.com".to_string(),
                    name: "password".to_string(),
                },
                SecretEntry {
                    domain: "a.com".to_string(),
                    name: "username".to_string(),
                },
                SecretEntry {
                    domain: "b.com".to_string(),
                    name: "token".to_string(),
                },
            ]
        );
    }

    #[test]
    fn classify_secret_entries_returns_added_existing_and_extras() {
        let required = vec![
            SecretEntry {
                domain: "a.com".to_string(),
                name: "username".to_string(),
            },
            SecretEntry {
                domain: "a.com".to_string(),
                name: "password".to_string(),
            },
            SecretEntry {
                domain: "b.com".to_string(),
                name: "token".to_string(),
            },
        ];
        let existing = vec![
            SecretEntry {
                domain: "a.com".to_string(),
                name: "username".to_string(),
            },
            SecretEntry {
                domain: "z.com".to_string(),
                name: "legacy".to_string(),
            },
        ];

        let (added, existing_required, extras) = classify_secret_entries(&required, &existing);
        assert_eq!(
            added,
            vec![
                SecretEntry {
                    domain: "a.com".to_string(),
                    name: "password".to_string(),
                },
                SecretEntry {
                    domain: "b.com".to_string(),
                    name: "token".to_string(),
                },
            ]
        );
        assert_eq!(
            existing_required,
            vec![SecretEntry {
                domain: "a.com".to_string(),
                name: "username".to_string(),
            }]
        );
        assert_eq!(
            extras,
            vec![SecretEntry {
                domain: "z.com".to_string(),
                name: "legacy".to_string(),
            }]
        );
    }

    #[test]
    fn delete_login_account_removes_label_mapping() {
        let dir = create_temp_dir("delete-login-account-ok");
        let mut accounts = BTreeMap::new();
        accounts.insert(
            "checking".to_string(),
            crate::login_config::LoginAccountConfig {
                gl_account: Some("Assets:Chase:Checking".to_string()),
            },
        );
        let config = crate::login_config::LoginConfig {
            extension: Some("chase-driver".to_string()),
            accounts,
        };
        if let Err(err) = crate::login_config::write_login_config(&dir, "chase-personal", &config) {
            panic!("failed to write login config: {err}");
        }

        let result = delete_login_account(
            dir.to_string_lossy().to_string(),
            "chase-personal".to_string(),
            "checking".to_string(),
        );
        assert!(result.is_ok(), "expected success, got: {result:?}");

        let updated = crate::login_config::read_login_config(&dir, "chase-personal");
        assert!(
            !updated.accounts.contains_key("checking"),
            "expected label to be removed from config"
        );
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn delete_login_account_errors_when_label_missing() {
        let dir = create_temp_dir("delete-login-account-missing");
        let config = crate::login_config::LoginConfig {
            extension: Some("chase-driver".to_string()),
            accounts: BTreeMap::new(),
        };
        if let Err(err) = crate::login_config::write_login_config(&dir, "chase-personal", &config) {
            panic!("failed to write login config: {err}");
        }

        let result = delete_login_account(
            dir.to_string_lossy().to_string(),
            "chase-personal".to_string(),
            "checking".to_string(),
        );
        match result {
            Ok(()) => panic!("expected error for missing label"),
            Err(err) => assert!(err.contains("label 'checking' not found")),
        }
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn delete_login_account_errors_when_login_missing() {
        let dir = create_temp_dir("delete-login-account-missing-login");
        let result = delete_login_account(
            dir.to_string_lossy().to_string(),
            "missing-login".to_string(),
            "checking".to_string(),
        );
        match result {
            Ok(()) => panic!("expected error for missing login"),
            Err(err) => assert!(err.contains("login 'missing-login' does not exist")),
        }
        let _ = fs::remove_dir_all(&dir);
    }
}
