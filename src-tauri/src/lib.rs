pub mod cli;
pub mod hledger;

mod ledger;
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
        .invoke_handler(tauri::generate_handler![new_ledger])
        .setup(|app| {
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
