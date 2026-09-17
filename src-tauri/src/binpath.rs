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

/// Warning text when the scraper worker on disk is older than the running app
/// binary, or `None` when it is current (or either mtime is unavailable).
///
/// `cargo run --bin app` rebuilds the app but not the separately linked
/// worker, so a runtime change can sit untested for hours while the app keeps
/// spawning the stale worker (2026-09-17). The app emits this into the scrape
/// output so the mismatch is visible where the scrape is watched.
pub fn stale_worker_warning(worker: &Path, app_exe: &Path) -> Option<String> {
    let worker_mtime = std::fs::metadata(worker).ok()?.modified().ok()?;
    let app_mtime = std::fs::metadata(app_exe).ok()?.modified().ok()?;
    let lag = app_mtime.duration_since(worker_mtime).ok()?;
    // Sub-minute skew is just the two link steps of one build.
    if lag.as_secs() < 60 {
        return None;
    }
    Some(format!(
        "Warning: scraper worker {} is {} min older than the app; runtime changes since then are not in effect. Run `npm run build:sidecars:debug` and restart the app.",
        worker.display(),
        lag.as_secs() / 60
    ))
}

pub fn stale_worker_warning_for_current_app() -> Option<String> {
    let app_exe = std::env::current_exe().ok()?;
    stale_worker_warning(Path::new(scraper_worker_path()), &app_exe)
}

#[cfg(test)]
mod tests {
    use super::stale_worker_warning;
    use std::time::{Duration, SystemTime};

    fn touch(path: &std::path::Path, mtime: SystemTime) {
        std::fs::write(path, b"x").unwrap_or_else(|err| panic!("write failed: {err}"));
        let file = std::fs::File::open(path).unwrap_or_else(|err| panic!("open failed: {err}"));
        file.set_modified(mtime)
            .unwrap_or_else(|err| panic!("set_modified failed: {err}"));
    }

    #[test]
    fn stale_worker_warning_only_when_worker_is_much_older_than_app() {
        let dir = std::env::temp_dir().join(format!(
            "refreshmint-binpath-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(SystemTime::UNIX_EPOCH)
                .unwrap_or_default()
                .as_nanos()
        ));
        std::fs::create_dir_all(&dir).unwrap_or_else(|err| panic!("mkdir failed: {err}"));
        let worker = dir.join("scraper-worker");
        let app = dir.join("app");
        let now = SystemTime::now();

        touch(&app, now);
        touch(&worker, now - Duration::from_secs(3 * 3600));
        let warning = stale_worker_warning(&worker, &app).unwrap_or_default();
        assert!(warning.contains("180 min older"), "{warning}");
        assert!(warning.contains("build:sidecars:debug"), "{warning}");

        // Same build, seconds apart: no warning.
        touch(&worker, now - Duration::from_secs(20));
        assert_eq!(stale_worker_warning(&worker, &app), None);

        // Worker newer than app: no warning.
        touch(&worker, now + Duration::from_secs(3600));
        assert_eq!(stale_worker_warning(&worker, &app), None);

        // Missing worker: no warning (the spawn itself will fail loudly).
        assert_eq!(stale_worker_warning(&dir.join("missing"), &app), None);

        let _ = std::fs::remove_dir_all(&dir);
    }
}
