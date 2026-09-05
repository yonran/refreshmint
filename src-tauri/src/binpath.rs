use std::ffi::OsString;
use std::path::Path;
use std::sync::OnceLock;

static HLEDGER_PATH: OnceLock<OsString> = OnceLock::new();
static SCRAPER_WORKER_PATH: OnceLock<OsString> = OnceLock::new();

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

        let worker_name = if cfg!(windows) {
            "refreshmint-scraper-worker.exe"
        } else {
            "refreshmint-scraper-worker"
        };
        let candidate = resource_dir.join(worker_name);
        if is_usable_sidecar(&candidate) {
            let _ = SCRAPER_WORKER_PATH.set(candidate.into_os_string());
        }
    }

    // During development Cargo places sibling binary targets together. Resolve
    // from the running executable instead of relying on the caller's PATH or cwd.
    if SCRAPER_WORKER_PATH.get().is_none() {
        if let Ok(exe) = std::env::current_exe() {
            let worker_name = if cfg!(windows) {
                "scraper-worker.exe"
            } else {
                "scraper-worker"
            };
            if let Some(parent) = exe.parent() {
                let candidate = parent.join(worker_name);
                if is_usable_sidecar(&candidate) {
                    let _ = SCRAPER_WORKER_PATH.set(candidate.into_os_string());
                }
            }
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

/// Return the separately linked scraper worker path. The PATH fallback is
/// useful for direct CLI development; Tauri development resolves a sibling
/// target binary and packaged builds resolve the bundled sidecar above.
pub fn scraper_worker_path() -> &'static OsString {
    SCRAPER_WORKER_PATH.get_or_init(|| {
        let worker_name = if cfg!(windows) {
            "scraper-worker.exe"
        } else {
            "scraper-worker"
        };
        std::env::current_exe()
            .ok()
            .and_then(|exe| exe.parent().map(|parent| parent.join(worker_name)))
            .filter(|candidate| is_usable_sidecar(candidate))
            .map(std::path::PathBuf::into_os_string)
            .unwrap_or_else(|| OsString::from(worker_name))
    })
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
