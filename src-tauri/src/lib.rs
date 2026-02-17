pub mod cli;
pub mod hledger;
pub mod scrape;
pub mod secret;

mod binpath;
mod extension;
mod ledger;
mod ledger_add;
mod ledger_open;
mod version;

use tauri::Manager;

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
            run_scrape
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
    };

    tokio::task::spawn_blocking(move || scrape::run_scrape(config).map_err(|err| err.to_string()))
        .await
        .map_err(|err| err.to_string())?
        .map_err(|err| err.to_string())
}
