use std::ffi::OsString;
use std::path::Path;
use std::sync::OnceLock;

static HLEDGER_PATH: OnceLock<OsString> = OnceLock::new();

/// Resolve the sidecar binary path from the running app's resource directory.
/// Must be called during `setup()`.
pub fn init_from_app(app: &tauri::AppHandle) {
    use tauri::Manager;

    let sidecar_name = if cfg!(windows) {
        "hledger.exe"
    } else {
        "hledger"
    };

    if let Ok(resource_dir) = app.path().resource_dir() {
        let candidate = resource_dir.join(sidecar_name);
        if is_usable_sidecar(&candidate) {
            let _ = HLEDGER_PATH.set(candidate.into_os_string());
        }
    }
}

/// Return the hledger binary path. Falls back to `"hledger"` (PATH lookup)
/// when no bundled sidecar was found (e.g. during development).
pub fn hledger_path() -> &'static OsString {
    static FALLBACK: OnceLock<OsString> = OnceLock::new();
    HLEDGER_PATH
        .get()
        .unwrap_or_else(|| FALLBACK.get_or_init(|| OsString::from("hledger")))
}

fn is_usable_sidecar(path: &Path) -> bool {
    let Ok(metadata) = std::fs::metadata(path) else {
        return false;
    };

    if !metadata.is_file() || metadata.len() == 0 {
        return false;
    }

    true
}
