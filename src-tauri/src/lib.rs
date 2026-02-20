pub mod cli;
pub mod hledger;
pub mod scrape;
pub mod secret;

pub mod account_journal;
pub mod dedup;
pub mod extract;
pub mod operations;
pub mod reconcile;
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

#[derive(serde::Serialize)]
struct SecretEntry {
    domain: String,
    name: String,
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
            add_account_secret,
            reenter_account_secret,
            remove_account_secret,
            start_scrape_debug_session,
            stop_scrape_debug_session,
            get_scrape_debug_session_socket,
            run_scrape,
            list_documents,
            run_extraction,
            get_account_journal,
            get_unreconciled,
            reconcile_entry,
            unreconcile_entry,
            reconcile_transfer,
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
fn list_account_secrets(account: String) -> Result<Vec<SecretEntry>, String> {
    let account = require_non_empty_input("account", account)?;
    let store = crate::secret::SecretStore::new(account);
    let mut entries = store
        .list()
        .map_err(|err| err.to_string())?
        .into_iter()
        .map(|(domain, name)| SecretEntry { domain, name })
        .collect::<Vec<_>>();
    entries.sort_by(|a, b| a.domain.cmp(&b.domain).then_with(|| a.name.cmp(&b.name)));
    Ok(entries)
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

#[tauri::command]
fn start_scrape_debug_session(
    ledger: String,
    account: String,
    extension: String,
) -> Result<String, String> {
    let account = account.trim().to_string();
    if account.is_empty() {
        return Err("account is required".to_string());
    }
    let extension = extension.trim().to_string();
    if extension.is_empty() {
        return Err("extension is required".to_string());
    }

    let target_dir = std::path::PathBuf::from(ledger);
    crate::ledger::require_refreshmint_extension(&target_dir).map_err(|err| err.to_string())?;
    let socket_path =
        crate::scrape::debug::default_debug_socket_path(&account).map_err(|err| err.to_string())?;

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
        account,
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
async fn run_scrape(ledger: String, account: String, extension: String) -> Result<(), String> {
    let account = account.trim().to_string();
    if account.is_empty() {
        return Err("account is required".to_string());
    }
    let extension = extension.trim().to_string();
    if extension.is_empty() {
        return Err("extension is required".to_string());
    }

    let target_dir = std::path::PathBuf::from(ledger);
    crate::ledger::require_refreshmint_extension(&target_dir).map_err(|err| err.to_string())?;

    let config = scrape::ScrapeConfig {
        account,
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
fn list_documents(
    ledger: String,
    account_name: String,
) -> Result<Vec<extract::DocumentWithInfo>, String> {
    let target_dir = std::path::PathBuf::from(ledger);
    let account_name = require_non_empty_input("account_name", account_name)?;
    extract::list_documents(&target_dir, &account_name).map_err(|err| err.to_string())
}

#[tauri::command]
fn run_extraction(
    ledger: String,
    account_name: String,
    extension_name: String,
    document_names: Vec<String>,
) -> Result<usize, String> {
    let target_dir = std::path::PathBuf::from(ledger);
    let account_name = require_non_empty_input("account_name", account_name)?;
    let extension_name = require_non_empty_input("extension_name", extension_name)?;

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

fn evidence_ref_matches_document(evidence_ref: &str, document_name: &str) -> bool {
    evidence_ref.starts_with(document_name)
        && evidence_ref
            .get(document_name.len()..)
            .map(|rest| rest.starts_with(':') || rest.starts_with('#'))
            .unwrap_or(false)
}

#[derive(serde::Serialize)]
struct AccountJournalEntry {
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

#[tauri::command]
fn get_account_journal(
    ledger: String,
    account_name: String,
) -> Result<Vec<AccountJournalEntry>, String> {
    let target_dir = std::path::PathBuf::from(ledger);
    let account_name = require_non_empty_input("account_name", account_name)?;
    let entries =
        account_journal::read_journal(&target_dir, &account_name).map_err(|err| err.to_string())?;

    Ok(entries
        .into_iter()
        .map(|e| {
            let is_transfer = transfer_detector::is_probable_transfer(&e.description);
            let status = match e.status {
                account_journal::EntryStatus::Cleared => "cleared",
                account_journal::EntryStatus::Pending => "pending",
                account_journal::EntryStatus::Unmarked => "unmarked",
            };
            AccountJournalEntry {
                id: e.id,
                date: e.date,
                status: status.to_string(),
                description: e.description,
                comment: e.comment,
                evidence: e.evidence,
                reconciled: e.reconciled,
                is_transfer,
            }
        })
        .collect())
}

#[tauri::command]
fn get_unreconciled(
    ledger: String,
    account_name: String,
) -> Result<Vec<AccountJournalEntry>, String> {
    let target_dir = std::path::PathBuf::from(ledger);
    let account_name = require_non_empty_input("account_name", account_name)?;
    let entries =
        reconcile::get_unreconciled(&target_dir, &account_name).map_err(|err| err.to_string())?;

    Ok(entries
        .into_iter()
        .map(|e| {
            let is_transfer = transfer_detector::is_probable_transfer(&e.description);
            let status = match e.status {
                account_journal::EntryStatus::Cleared => "cleared",
                account_journal::EntryStatus::Pending => "pending",
                account_journal::EntryStatus::Unmarked => "unmarked",
            };
            AccountJournalEntry {
                id: e.id,
                date: e.date,
                status: status.to_string(),
                description: e.description,
                comment: e.comment,
                evidence: e.evidence,
                reconciled: e.reconciled,
                is_transfer,
            }
        })
        .collect())
}

#[tauri::command]
fn reconcile_entry(
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

    reconcile::reconcile_entry(
        &target_dir,
        &account_name,
        &entry_id,
        &counterpart_account,
        posting_index,
    )
    .map_err(|err| err.to_string())
}

#[tauri::command]
fn unreconcile_entry(
    ledger: String,
    account_name: String,
    entry_id: String,
    posting_index: Option<usize>,
) -> Result<(), String> {
    let target_dir = std::path::PathBuf::from(ledger);
    let account_name = require_non_empty_input("account_name", account_name)?;
    let entry_id = require_non_empty_input("entry_id", entry_id)?;

    reconcile::unreconcile_entry(&target_dir, &account_name, &entry_id, posting_index)
        .map_err(|err| err.to_string())
}

#[tauri::command]
fn reconcile_transfer(
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

    reconcile::reconcile_transfer(&target_dir, &account1, &entry_id1, &account2, &entry_id2)
        .map_err(|err| err.to_string())
}

#[cfg(test)]
mod tests {
    use super::{evidence_ref_matches_document, require_non_empty_input};

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
    fn evidence_ref_matches_document_requires_delimiter() {
        assert!(evidence_ref_matches_document("foo.csv:1:1", "foo.csv"));
        assert!(evidence_ref_matches_document("foo.csv#row:1", "foo.csv"));
        assert!(!evidence_ref_matches_document("foo.csvx:1:1", "foo.csv"));
        assert!(!evidence_ref_matches_document("foo.csv", "foo.csv"));
    }
}
