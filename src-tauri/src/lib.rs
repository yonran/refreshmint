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
pub mod report;
pub mod transfer_detector;

mod binpath;
mod builtin_extensions;
mod extension;
mod js_module_loader;
mod ledger;
mod ledger_add;
mod ledger_open;
mod ts_strip;
mod version;

use tauri::{Emitter, Manager};

struct UiDebugSession {
    socket_path: std::path::PathBuf,
    join_handle: std::thread::JoinHandle<()>,
}

struct LockMetadataWatcher {
    ledger_path: std::path::PathBuf,
    _watcher: notify::RecommendedWatcher,
}

/// Per-domain credential status returned by list/sync commands.
#[derive(Clone, Debug, serde::Serialize)]
#[serde(rename_all = "camelCase")]
struct DomainSecretEntry {
    domain: String,
    has_username: bool,
    has_password: bool,
}

/// Sync result: which domains are required by the manifest, which are missing
/// credentials, and which are extra (stored but not required).
#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
struct SecretSyncResult {
    /// All domains declared in the extension manifest.
    required: Vec<DomainSecretEntry>,
    /// Required domains missing a username (new scheme only).
    missing_username: Vec<String>,
    /// Required domains missing a password.
    missing_password: Vec<String>,
    /// Domains in the store that are not declared by the manifest.
    extras: Vec<String>,
}

#[derive(Clone, Debug, serde::Serialize)]
#[serde(rename_all = "camelCase")]
struct LockStatusChangedEvent {
    ledger_path: String,
}

#[derive(Clone, Debug, serde::Serialize)]
struct LockStatusSnapshot {
    gl: login_config::LockStatus,
    logins: std::collections::BTreeMap<String, login_config::LockStatus>,
}

#[derive(Clone, Debug, serde::Serialize)]
#[serde(rename_all = "camelCase")]
struct LoginExtractionSupport {
    supported: bool,
    reason: Option<&'static str>,
}

/// Tauri state holding the mpsc sender for an in-progress refreshmint.prompt()
/// call. The scrape thread creates a channel, stores the Sender here, and
/// blocks waiting for the Receiver. The frontend calls submit_prompt_answer
/// to send `Some(answer)` for Submit or `None` for Cancel. Keep this aligned
/// with the receiving half in `scrape/js_api.rs`.
#[derive(Default)]
pub struct PromptAnswerState(pub std::sync::Mutex<Option<std::sync::mpsc::Sender<Option<String>>>>);

static UI_DEBUG_SESSION: std::sync::OnceLock<std::sync::Mutex<Option<UiDebugSession>>> =
    std::sync::OnceLock::new();
static LOCK_METADATA_WATCHER: std::sync::OnceLock<std::sync::Mutex<Option<LockMetadataWatcher>>> =
    std::sync::OnceLock::new();

fn ui_debug_session_state() -> &'static std::sync::Mutex<Option<UiDebugSession>> {
    UI_DEBUG_SESSION.get_or_init(|| std::sync::Mutex::new(None))
}

fn lock_metadata_watcher_state() -> &'static std::sync::Mutex<Option<LockMetadataWatcher>> {
    LOCK_METADATA_WATCHER.get_or_init(|| std::sync::Mutex::new(None))
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
        .manage(PromptAnswerState::default())
        .invoke_handler(tauri::generate_handler![
            new_ledger,
            open_ledger,
            add_transaction,
            validate_transaction,
            add_transaction_text,
            validate_transaction_text,
            list_scrape_extensions,
            load_scrape_extension,
            start_scrape_debug_session_for_login,
            start_scrape_debug_session,
            stop_scrape_debug_session,
            get_scrape_debug_session_socket,
            start_lock_metadata_watch,
            stop_lock_metadata_watch,
            get_lock_status_snapshot,
            get_login_extraction_support,
            run_scrape_for_login,
            run_scrape,
            get_scrape_log,
            list_documents,
            list_login_account_documents,
            read_login_account_document_rows,
            read_login_account_document_text,
            read_attachment_data_url,
            run_extraction,
            run_login_account_extraction,
            get_account_journal,
            get_login_account_journal,
            get_unposted,
            get_login_account_unposted,
            post_entry,
            post_login_account_entry,
            post_login_account_entry_split,
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
            repair_login_account_labels,
            list_login_secrets,
            sync_login_secrets_for_extension,
            set_login_credentials,
            set_login_username,
            set_login_password,
            remove_login_domain,
            get_login_username,
            migrate_login_secrets,
            clear_login_profile,
            migrate_ledger,
            query_transactions,
            run_hledger_report,
            submit_prompt_answer,
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

/// Build a `DomainSecretEntry` list from the manifest's `SecretDeclarations`.
///
/// Each domain in the manifest becomes one entry; the presence flags are
/// determined by comparing against the stored domains.
fn build_required_entries(
    declared: &scrape::js_api::SecretDeclarations,
    stored: &[crate::secret::DomainEntry],
) -> Vec<DomainSecretEntry> {
    let stored_map: std::collections::BTreeMap<&str, &crate::secret::DomainEntry> =
        stored.iter().map(|e| (e.domain.as_str(), e)).collect();
    declared
        .keys()
        .map(|domain| {
            let stored_entry = stored_map.get(domain.as_str());
            DomainSecretEntry {
                domain: domain.clone(),
                has_username: stored_entry.is_some_and(|e| e.has_username),
                has_password: stored_entry.is_some_and(|e| e.has_password),
            }
        })
        .collect()
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

fn inspect_login_extraction_support(
    ledger_dir: &std::path::Path,
    login_name: &str,
) -> Result<LoginExtractionSupport, String> {
    let extension_name = match login_config::resolve_login_extension(ledger_dir, login_name) {
        Ok(extension_name) => extension_name,
        Err(err) if err.contains("no extension configured") => {
            return Ok(LoginExtractionSupport {
                supported: false,
                reason: Some("missing-extension"),
            });
        }
        Err(err) => return Err(err),
    };
    let extension_dir = account_config::resolve_extension_dir(ledger_dir, &extension_name);
    let manifest = scrape::load_manifest(&extension_dir).map_err(|err| err.to_string())?;
    match (manifest.extract.as_deref(), manifest.rules.as_deref()) {
        (None, None) => Ok(LoginExtractionSupport {
            supported: false,
            reason: Some("missing-extractor"),
        }),
        (Some(path), None) => Ok(LoginExtractionSupport {
            supported: extension_dir.join(path).exists(),
            reason: if extension_dir.join(path).exists() {
                None
            } else {
                Some("broken-extractor")
            },
        }),
        (None, Some(path)) => Ok(LoginExtractionSupport {
            supported: extension_dir.join(path).exists(),
            reason: if extension_dir.join(path).exists() {
                None
            } else {
                Some("broken-extractor")
            },
        }),
        (Some(_), Some(_)) => Ok(LoginExtractionSupport {
            supported: false,
            reason: Some("broken-extractor"),
        }),
    }
}

#[tauri::command]
fn get_login_extraction_support(
    ledger: String,
    login_name: String,
) -> Result<LoginExtractionSupport, String> {
    let target_dir = std::path::PathBuf::from(ledger);
    let login_name = require_login_name_input(login_name)?;
    require_existing_login(&target_dir, &login_name)?;
    inspect_login_extraction_support(&target_dir, &login_name)
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
fn start_lock_metadata_watch(app: tauri::AppHandle, ledger: String) -> Result<(), String> {
    use notify::Watcher;

    let target_dir = std::path::PathBuf::from(ledger);
    crate::ledger::require_refreshmint_extension(&target_dir).map_err(|err| err.to_string())?;

    let state = lock_metadata_watcher_state();
    let mut guard = state
        .lock()
        .map_err(|_| "failed to acquire lock metadata watcher state".to_string())?;

    if let Some(existing) = guard.as_ref() {
        if existing.ledger_path == target_dir {
            return Ok(());
        }
    }

    guard.take();

    let app_handle = app.clone();
    let ledger_path = target_dir.clone();
    let watcher =
        notify::recommended_watcher(move |result: Result<notify::Event, notify::Error>| {
            let Ok(event) = result else {
                return;
            };
            let should_emit = event.paths.iter().any(|path| {
                matches!(
                    path.file_name().and_then(|name| name.to_str()),
                    Some(".lock.meta.json" | ".gl.lock.meta.json")
                )
            });
            if should_emit {
                let _ = app_handle.emit(
                    "refreshmint://lock-status-changed",
                    LockStatusChangedEvent {
                        ledger_path: ledger_path.to_string_lossy().to_string(),
                    },
                );
            }
        })
        .map_err(|err| err.to_string())?;

    let mut watcher = watcher;
    watcher
        .watch(&target_dir, notify::RecursiveMode::Recursive)
        .map_err(|err| err.to_string())?;

    *guard = Some(LockMetadataWatcher {
        ledger_path: target_dir,
        _watcher: watcher,
    });
    Ok(())
}

#[tauri::command]
fn stop_lock_metadata_watch() -> Result<(), String> {
    let state = lock_metadata_watcher_state();
    let mut guard = state
        .lock()
        .map_err(|_| "failed to acquire lock metadata watcher state".to_string())?;
    guard.take();
    Ok(())
}

#[tauri::command]
fn get_lock_status_snapshot(
    ledger: String,
    login_names: Vec<String>,
) -> Result<LockStatusSnapshot, String> {
    let target_dir = std::path::PathBuf::from(ledger);
    crate::ledger::require_refreshmint_extension(&target_dir).map_err(|err| err.to_string())?;

    let mut logins = std::collections::BTreeMap::new();
    for login_name in login_names {
        let login_name = require_login_name_input(login_name)?;
        let status = login_config::get_login_lock_status(&target_dir, &login_name)
            .map_err(|err| err.to_string())?;
        logins.insert(login_name, status);
    }

    let gl = login_config::get_gl_lock_status(&target_dir).map_err(|err| err.to_string())?;
    Ok(LockStatusSnapshot { gl, logins })
}

#[tauri::command]
async fn run_scrape_for_login(
    app_handle: tauri::AppHandle,
    ledger: String,
    login_name: String,
    source: String,
) -> Result<(), String> {
    let login_name = require_login_name_input(login_name)?;

    let target_dir = std::path::PathBuf::from(&ledger);
    // Validate ledger and login BEFORE any logging: append_jsonl creates
    // directories, so we must not call it if the ledger or login dir may not exist.
    crate::ledger::require_refreshmint_extension(&target_dir).map_err(|err| err.to_string())?;
    require_existing_login(&target_dir, &login_name)?;

    // From here ledger and login are confirmed to exist; logging is safe.
    let timestamp = operations::now_timestamp();

    let result: Result<(), String> = async {
        let extension = login_config::resolve_login_extension(&target_dir, &login_name)
            .map_err(|err| err.to_string())?;
        let prompt_ui_handler = {
            let app_handle = app_handle.clone();
            std::sync::Arc::new(move |message: String| request_prompt_answer(&app_handle, message))
        };

        let config = scrape::ScrapeConfig {
            login_name: login_name.clone(),
            extension_name: extension,
            ledger_dir: target_dir.clone(),
            profile_override: None,
            prompt_overrides: scrape::js_api::PromptOverrides::new(),
            prompt_requires_override: false,
            prompt_ui_handler: Some(prompt_ui_handler),
        };

        tokio::task::spawn_blocking(move || {
            scrape::run_scrape(config).map_err(|err| err.to_string())
        })
        .await
        .map_err(|err| err.to_string())?
    }
    .await;

    let entry = operations::ScrapeLogEntry {
        login_name: login_name.clone(),
        timestamp,
        success: result.is_ok(),
        error: result.as_ref().err().cloned(),
        source,
    };
    if let Err(e) = operations::append_scrape_log_entry(&target_dir, &entry) {
        eprintln!("warning: failed to write scrape log: {e}");
    }

    result
}

#[tauri::command]
async fn run_scrape(
    app_handle: tauri::AppHandle,
    ledger: String,
    account: String,
) -> Result<(), String> {
    let login_name = require_non_empty_input("account", account)?;
    run_scrape_for_login(app_handle, ledger, login_name, "manual".to_string()).await
}

#[tauri::command]
fn get_scrape_log(
    ledger: String,
    login_name: String,
) -> Result<Vec<operations::ScrapeLogEntry>, String> {
    let ledger_dir = std::path::PathBuf::from(&ledger);
    crate::ledger::require_refreshmint_extension(&ledger_dir).map_err(|err| err.to_string())?;
    let login_name = require_login_name_input(login_name)?;
    require_existing_login(&ledger_dir, &login_name)?;
    let mut entries =
        operations::read_scrape_log(&ledger_dir, &login_name).map_err(|err| err.to_string())?;
    entries.reverse(); // newest-first to match prior localStorage behaviour
    Ok(entries)
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
fn read_login_account_document_text(
    ledger: String,
    login_name: String,
    label: String,
    document_name: String,
) -> Result<String, String> {
    let target_dir = std::path::PathBuf::from(ledger);
    let login_name = require_login_name_input(login_name)?;
    let label = require_label_input(label)?;
    extract::read_login_account_document_text(&target_dir, &login_name, &label, &document_name)
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
    // gl_account is optional for extraction: extensions that supply explicit
    // tpostings (e.g. the target extractor) do not need a pre-configured GL
    // account. The gl_account is still required by post_login_account_entry /
    // post_login_account_transfer at posting time.
    let gl_account: String = {
        let config = login_config::read_login_config(&target_dir, &login_name);
        config
            .accounts
            .get(&*label)
            .and_then(|a| a.gl_account.as_deref())
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(str::to_string)
            .unwrap_or_default()
    };

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

        // When gl_account is empty (no glAccount configured), default_account
        // falls back to "" on the very first extraction run (empty journal).
        // This is safe only if every proposed transaction supplies explicit
        // tpostings — if any transaction has tpostings: None, we fail loudly
        // rather than silently writing blank-account journal entries.
        let default_account = all_updated
            .first()
            .and_then(|e| e.postings.first())
            .map(|p| p.account.clone())
            .unwrap_or_else(|| gl_account.clone());
        if default_account.is_empty() {
            let has_implicit = doc_txns.iter().any(|t| t.tpostings.is_none());
            if has_implicit {
                return Err(format!(
                    "login '{login_name}' label '{label}': extractor produced a \
                     transaction without explicit tpostings but no glAccount is \
                     configured; set a GL account or fix the extractor"
                ));
            }
        }
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
    let _lock = login_config::acquire_login_lock_with_metadata(
        &target_dir,
        &login_name,
        "gui",
        "set-login-extension",
    )
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
    let _lock = login_config::acquire_login_lock_with_metadata(
        &target_dir,
        &login_name,
        "gui",
        "delete-login",
    )
    .map_err(|err| err.to_string())?;
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

    let _lock = login_config::acquire_login_lock_with_metadata(
        &target_dir,
        &login_name,
        "gui",
        "set-login-account",
    )
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

    let _lock = login_config::acquire_login_lock_with_metadata(
        &target_dir,
        &login_name,
        "gui",
        "remove-login-account",
    )
    .map_err(|err| err.to_string())?;

    login_config::remove_login_account(&target_dir, &login_name, &label)
        .map_err(|err| err.to_string())
}

#[tauri::command]
fn delete_login_account(ledger: String, login_name: String, label: String) -> Result<(), String> {
    remove_login_account(ledger, login_name, label)
}

#[tauri::command]
fn repair_login_account_labels(
    ledger: String,
    login_name: String,
) -> Result<migration::MigrationOutcome, String> {
    let target_dir = std::path::PathBuf::from(ledger);
    let login_name = require_login_name_input(login_name)?;
    require_existing_login(&target_dir, &login_name)?;

    let aliases: &[(&str, &str)] = match login_name.as_str() {
        "provident-yonran" => &[
            ("4569_signature_cash_back", "signature_cash_back_4569"),
            ("6500_membership_savings", "membership_savings_6500"),
            ("6590_super_reward_checking", "super_reward_checking_6590"),
            ("7000_savings_plus_00", "savings_plus_00_7000"),
            (
                "savings_plus_00_x7000available_7000",
                "savings_plus_00_7000",
            ),
            (
                "super_reward_checking_6590available_61_131_92",
                "super_reward_checking_6590",
            ),
            (
                "signature_cash_back_statement_4569",
                "signature_cash_back_4569",
            ),
        ],
        "bankofamerica" => &[("_default", "bankofamerica")],
        "citi" => &[("_default", "costco_anywhere_visa_card_by_citi_3743")],
        _ => &[],
    };

    migration::repair_login_account_labels(&target_dir, &login_name, aliases)
        .map_err(|err| err.to_string())
}

// --- Login-keyed secret commands ---

#[tauri::command]
fn list_login_secrets(login_name: String) -> Result<Vec<DomainSecretEntry>, String> {
    let login_name = require_login_name_input(login_name)?;
    let store = crate::secret::SecretStore::new(format!("login/{login_name}"));
    let mut entries = store
        .list_domains()
        .map_err(|err| err.to_string())?
        .into_iter()
        .map(|e| DomainSecretEntry {
            domain: e.domain,
            has_username: e.has_username,
            has_password: e.has_password,
        })
        .collect::<Vec<_>>();
    entries.sort_by_key(|e| e.domain.clone());
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

    let store = crate::secret::SecretStore::new(format!("login/{login_name}"));
    let stored = store.list_domains().map_err(|err| err.to_string())?;
    let required_domains: std::collections::BTreeSet<&str> =
        declared.keys().map(String::as_str).collect();
    let required = build_required_entries(&declared, &stored);
    let missing_username = required
        .iter()
        .filter(|e| {
            declared
                .get(&e.domain)
                .and_then(|c| c.username.as_ref())
                .is_some()
                && !e.has_username
        })
        .map(|e| e.domain.clone())
        .collect();
    let missing_password = required
        .iter()
        .filter(|e| {
            let creds = declared.get(&e.domain);
            let has_password_decl = creds.and_then(|c| c.password.as_ref()).is_some()
                || creds.is_some_and(|c| !c.extra_names.is_empty());
            has_password_decl && !e.has_password
        })
        .map(|e| e.domain.clone())
        .collect();
    let extras = stored
        .iter()
        .filter(|e| !required_domains.contains(e.domain.as_str()))
        .map(|e| e.domain.clone())
        .collect();

    Ok(SecretSyncResult {
        required,
        missing_username,
        missing_password,
        extras,
    })
}

/// Store username + password together for a domain (one biometric prompt on macOS).
#[tauri::command]
fn set_login_credentials(
    login_name: String,
    domain: String,
    username: String,
    password: String,
) -> Result<(), String> {
    let login_name = require_login_name_input(login_name)?;
    let domain = require_non_empty_input("domain", domain)?;
    let store = crate::secret::SecretStore::new(format!("login/{login_name}"));
    store
        .set_credentials(&domain, &username, &password)
        .map_err(|err| err.to_string())
}

/// Store only the username for a domain (no biometric prompt on macOS).
#[tauri::command]
fn set_login_username(login_name: String, domain: String, username: String) -> Result<(), String> {
    let login_name = require_login_name_input(login_name)?;
    let domain = require_non_empty_input("domain", domain)?;
    let store = crate::secret::SecretStore::new(format!("login/{login_name}"));
    store
        .set_username(&domain, &username)
        .map_err(|err| err.to_string())
}

/// Store only the password for a domain (biometric prompt on macOS).
#[tauri::command]
fn set_login_password(login_name: String, domain: String, password: String) -> Result<(), String> {
    let login_name = require_login_name_input(login_name)?;
    let domain = require_non_empty_input("domain", domain)?;
    let store = crate::secret::SecretStore::new(format!("login/{login_name}"));
    store
        .set_password(&domain, &password)
        .map_err(|err| err.to_string())
}

/// Delete all credentials for a domain.
#[tauri::command]
fn remove_login_domain(login_name: String, domain: String) -> Result<(), String> {
    let login_name = require_login_name_input(login_name)?;
    let domain = require_non_empty_input("domain", domain)?;
    let store = crate::secret::SecretStore::new(format!("login/{login_name}"));
    store.delete_domain(&domain).map_err(|err| err.to_string())
}

/// Read the username for a domain — no biometric prompt.
#[tauri::command]
fn get_login_username(login_name: String, domain: String) -> Result<String, String> {
    let login_name = require_login_name_input(login_name)?;
    let domain = require_non_empty_input("domain", domain)?;
    let store = crate::secret::SecretStore::new(format!("login/{login_name}"));
    store.get_username(&domain).map_err(|err| err.to_string())
}

/// Migrate legacy keychain entries (service=`refreshmint/<login>`, account=`<domain>/<name>`)
/// to the new scheme (service=`refreshmint/login/<login>/<domain>`, account=username).
///
/// Returns a list of domains that were migrated.
///
/// Behavior notes:
/// - Idempotent: returns an empty list when no legacy entries remain.
/// - Reads each legacy secret value first; on read failure it aborts with an error.
/// - Writes new entries per domain, then best-effort removes migrated legacy entries.
/// - See `scrape/js_api.rs` `ENABLE_LEGACY_SECRET_FALLBACK` for runtime fallback policy.
#[tauri::command]
fn migrate_login_secrets(login_name: String) -> Result<Vec<String>, String> {
    let login_name = require_login_name_input(login_name)?;
    let store = crate::secret::SecretStore::new(format!("login/{login_name}"));
    let legacy = store.list_legacy_entries().map_err(|err| err.to_string())?;
    if legacy.is_empty() {
        return Ok(Vec::new());
    }

    // Group by domain: collect all (domain, name, value) triples
    let mut by_domain: std::collections::BTreeMap<String, Vec<(String, String)>> =
        std::collections::BTreeMap::new();
    for (domain, name) in &legacy {
        let value = store
            .get_legacy_value(domain, name)
            .map_err(|err| format!("failed to read legacy secret '{domain}/{name}': {err}"))?;
        by_domain
            .entry(domain.clone())
            .or_default()
            .push((name.clone(), value));
    }

    let mut migrated = Vec::new();
    for (domain, name_values) in &by_domain {
        // Heuristic: name containing "username"/"user"/"login" → username role, else password
        let username = name_values
            .iter()
            .find(|(n, _)| {
                n.to_ascii_lowercase().contains("username")
                    || n.to_ascii_lowercase().contains("_user")
            })
            .map(|(_, v)| v.clone())
            .unwrap_or_default();
        let password = name_values
            .iter()
            .find(|(n, _)| {
                n.to_ascii_lowercase().contains("password")
                    || n.to_ascii_lowercase().contains("_pass")
            })
            .or_else(|| name_values.last())
            .map(|(_, v)| v.clone())
            .unwrap_or_default();

        store
            .set_credentials(domain, &username, &password)
            .map_err(|err| format!("failed to migrate domain '{domain}': {err}"))?;

        // Clean up legacy entries for this domain
        for (name, _) in name_values {
            let _ = store.delete_legacy_entry(domain, name);
        }
        migrated.push(domain.clone());
    }

    // Clean up legacy index if all entries were migrated
    if migrated.len() == by_domain.len() {
        let _ = store.delete_legacy_index();
    }

    Ok(migrated)
}

#[tauri::command]
fn clear_login_profile(ledger: String, login_name: String) -> Result<(), String> {
    let target_dir = std::path::PathBuf::from(ledger);
    let login_name = require_login_name_input(login_name)?;
    require_existing_login(&target_dir, &login_name)?;

    let lock = login_config::acquire_login_lock_with_metadata(
        &target_dir,
        &login_name,
        "gui",
        "clear-login-profile",
    )
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

#[tauri::command]
fn run_hledger_report(
    ledger: String,
    command: String,
    args: Vec<String>,
) -> Result<report::ReportResult, String> {
    let journal_path = std::path::PathBuf::from(&ledger).join("general.journal");
    report::run_report(&journal_path, &command, &args).map_err(|e| e.to_string())
}

#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
struct AccountJournalEntry {
    id: String,
    date: String,
    status: String,
    description: String,
    comment: String,
    evidence: Vec<String>,
    posted: Option<String>,
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
        "gui",
    )
    .map_err(|err| err.to_string())
}

#[tauri::command]
fn post_login_account_entry_split(
    ledger: String,
    login_name: String,
    label: String,
    entry_id: String,
    counterparts: Vec<post::SplitCounterpart>,
) -> Result<String, String> {
    let target_dir = std::path::PathBuf::from(ledger);
    let login_name = require_login_name_input(login_name)?;
    let label = require_label_input(label)?;
    let entry_id = require_non_empty_input("entry_id", entry_id)?;

    // Reject reconciliation when this login label's GL mapping is unset or conflicting.
    let _ = resolve_login_account_gl_account(&target_dir, &login_name, &label)?;

    post::post_login_account_entry_split(
        &target_dir,
        &login_name,
        &label,
        &entry_id,
        counterparts,
        "gui",
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

    post::unpost_login_account_entry(
        &target_dir,
        &login_name,
        &label,
        &entry_id,
        posting_index,
        "gui",
    )
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
        "gui",
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

    post::sync_gl_transaction(&target_dir, &login_name, &label, &entry_id, "gui")
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
    posting_index: usize,
    new_account: String,
) -> Result<(), String> {
    let target_dir = std::path::PathBuf::from(ledger);
    let txn_id = require_non_empty_input("txn_id", txn_id)?;
    let new_account = require_non_empty_input("new_account", new_account)?;
    post::recategorize_gl_transaction(&target_dir, &txn_id, posting_index, &new_account, "gui")
        .map_err(|err| err.to_string())
}

#[tauri::command]
fn merge_gl_transfer(ledger: String, txn_id_1: String, txn_id_2: String) -> Result<String, String> {
    let target_dir = std::path::PathBuf::from(ledger);
    let txn_id_1 = require_non_empty_input("txn_id_1", txn_id_1)?;
    let txn_id_2 = require_non_empty_input("txn_id_2", txn_id_2)?;
    post::merge_gl_transfer(&target_dir, &txn_id_1, &txn_id_2, "gui").map_err(|err| err.to_string())
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

/// Called by the frontend to deliver the user's answer to a pending
/// `refreshmint.prompt()` call that is blocking the scrape thread.
/// Sends `Some(answer)` for Submit or `None` for Cancel.
fn send_prompt_answer(answer: Option<String>, state: &PromptAnswerState) -> Result<(), String> {
    let mut guard = state.0.lock().map_err(|e| e.to_string())?;
    if let Some(sender) = guard.take() {
        // Ignore send errors: the scrape thread may have already timed out.
        let _ = sender.send(answer);
    }
    Ok(())
}

fn request_prompt_answer(
    app_handle: &tauri::AppHandle,
    message: String,
) -> Result<Option<String>, String> {
    let (tx, rx) = std::sync::mpsc::channel::<Option<String>>();
    {
        let state = app_handle.state::<PromptAnswerState>();
        let mut guard = state.0.lock().map_err(|e| e.to_string())?;
        *guard = Some(tx);
    }

    #[derive(serde::Serialize, Clone)]
    struct PromptRequestedPayload {
        message: String,
    }

    app_handle
        .emit(
            "refreshmint://prompt-requested",
            PromptRequestedPayload { message },
        )
        .map_err(|e| format!("prompt emit failed: {e}"))?;

    rx.recv().map_err(|_| "prompt cancelled".to_string())
}

/// Called by the frontend to deliver the user's answer to a pending
/// `refreshmint.prompt()` call that is blocking the scrape thread.
#[tauri::command]
fn submit_prompt_answer(
    answer: Option<String>,
    state: tauri::State<PromptAnswerState>,
) -> Result<(), String> {
    send_prompt_answer(answer, &state)
}

#[cfg(test)]
#[allow(clippy::expect_used)]
mod tests {
    use super::{
        delete_login_account, evidence_ref_matches_document, inspect_login_extraction_support,
        require_existing_login, require_label_input, require_login_name_input,
        require_non_empty_input, send_prompt_answer, PromptAnswerState,
    };
    use std::collections::BTreeMap;
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
    fn send_prompt_answer_delivers_cancel_as_none() {
        let (tx, rx) = std::sync::mpsc::channel();
        let state = PromptAnswerState(std::sync::Mutex::new(Some(tx)));

        send_prompt_answer(None, &state)
            .unwrap_or_else(|err| panic!("send_prompt_answer failed: {err}"));

        assert_eq!(
            rx.recv()
                .unwrap_or_else(|err| panic!("failed to receive prompt answer: {err}")),
            None
        );
    }

    #[test]
    fn send_prompt_answer_preserves_empty_string_submission() {
        let (tx, rx) = std::sync::mpsc::channel();
        let state = PromptAnswerState(std::sync::Mutex::new(Some(tx)));

        send_prompt_answer(Some(String::new()), &state)
            .unwrap_or_else(|err| panic!("send_prompt_answer failed: {err}"));

        assert_eq!(
            rx.recv()
                .unwrap_or_else(|err| panic!("failed to receive prompt answer: {err}")),
            Some(String::new())
        );
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
    fn inspect_login_extraction_support_reports_missing_extension() {
        let dir = create_temp_dir("missing-extractor-extension");
        let login_dir = dir.join("logins").join("chase");
        fs::create_dir_all(&login_dir).expect("create login dir");
        fs::write(login_dir.join("config.json"), "{}").expect("write config");

        let support =
            inspect_login_extraction_support(&dir, "chase").expect("inspect extraction support");
        assert!(!support.supported);
        assert_eq!(support.reason, Some("missing-extension"));
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn inspect_login_extraction_support_reports_missing_extractor() {
        let dir = create_temp_dir("missing-extractor");
        let extension_dir = dir.join("extensions").join("testbank");
        fs::create_dir_all(&extension_dir).expect("create extension dir");
        fs::write(
            extension_dir.join("manifest.json"),
            "{\n  \"name\": \"testbank\",\n  \"secrets\": {}\n}\n",
        )
        .expect("write manifest");

        let login_dir = dir.join("logins").join("testbank");
        fs::create_dir_all(&login_dir).expect("create login dir");
        fs::write(
            login_dir.join("config.json"),
            "{\n  \"extension\": \"testbank\"\n}\n",
        )
        .expect("write config");

        let support =
            inspect_login_extraction_support(&dir, "testbank").expect("inspect extraction support");
        assert!(!support.supported);
        assert_eq!(support.reason, Some("missing-extractor"));
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn inspect_login_extraction_support_reports_broken_extractor() {
        let dir = create_temp_dir("broken-extractor");
        let extension_dir = dir.join("extensions").join("testbank");
        fs::create_dir_all(&extension_dir).expect("create extension dir");
        fs::write(
            extension_dir.join("manifest.json"),
            "{\n  \"name\": \"testbank\",\n  \"extract\": \"extract.mjs\",\n  \"secrets\": {}\n}\n",
        )
        .expect("write manifest");

        let login_dir = dir.join("logins").join("testbank");
        fs::create_dir_all(&login_dir).expect("create login dir");
        fs::write(
            login_dir.join("config.json"),
            "{\n  \"extension\": \"testbank\"\n}\n",
        )
        .expect("write config");

        let support =
            inspect_login_extraction_support(&dir, "testbank").expect("inspect extraction support");
        assert!(!support.supported);
        assert_eq!(support.reason, Some("broken-extractor"));
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn inspect_login_extraction_support_reports_supported_extractor() {
        let dir = create_temp_dir("supported-extractor");
        let extension_dir = dir.join("extensions").join("testbank");
        fs::create_dir_all(&extension_dir).expect("create extension dir");
        fs::write(
            extension_dir.join("manifest.json"),
            "{\n  \"name\": \"testbank\",\n  \"extract\": \"extract.mjs\",\n  \"secrets\": {}\n}\n",
        )
        .expect("write manifest");
        fs::write(
            extension_dir.join("extract.mjs"),
            "export async function extract() { return []; }\n",
        )
        .expect("write extract script");

        let login_dir = dir.join("logins").join("testbank");
        fs::create_dir_all(&login_dir).expect("create login dir");
        fs::write(
            login_dir.join("config.json"),
            "{\n  \"extension\": \"testbank\"\n}\n",
        )
        .expect("write config");

        let support =
            inspect_login_extraction_support(&dir, "testbank").expect("inspect extraction support");
        assert!(support.supported);
        assert_eq!(support.reason, None);
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
