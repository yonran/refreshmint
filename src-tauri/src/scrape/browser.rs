use std::collections::HashSet;
use std::error::Error;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};

use chromiumoxide::browser::{Browser, BrowserConfig, HeadlessMode};
use chromiumoxide::error::CdpError;
use futures::StreamExt;

/// PIDs of browser processes this app has launched and not yet cleanly closed.
///
/// chromiumoxide kills the child via tokio's `kill_on_drop`, but that only fires
/// when the `Browser`/`Child` are *dropped*. On `std::process::exit` (which is
/// how the Tauri/tao event loop terminates) the stack is not unwound, so those
/// destructors never run and the browser is reparented to init as an orphan,
/// keeping the profile's `SingletonLock` and breaking the next scrape. The app
/// shutdown hook calls [`kill_active_browsers`] to SIGKILL whatever is left here.
static ACTIVE_BROWSER_PIDS: OnceLock<Mutex<HashSet<u32>>> = OnceLock::new();

fn active_browser_pids() -> &'static Mutex<HashSet<u32>> {
    ACTIVE_BROWSER_PIDS.get_or_init(|| Mutex::new(HashSet::new()))
}

fn register_browser_pid(pid: u32) {
    if let Ok(mut set) = active_browser_pids().lock() {
        set.insert(pid);
    }
}

fn unregister_browser_pid(pid: u32) {
    if let Ok(mut set) = active_browser_pids().lock() {
        set.remove(&pid);
    }
}

/// RAII guard that removes a launched browser's PID from the kill-on-exit
/// registry when dropped.
///
/// Drop runs on every in-process return path — normal completion, a `?`
/// early-return, or a panic — so a browser that is closed (or killed by
/// `kill_on_drop`) in-process never leaves a stale PID behind that could later
/// match a reused PID. Drop does *not* run on `std::process::exit`, which is
/// exactly the case where we want the PID to remain registered so the shutdown
/// hook can kill the otherwise-orphaned browser.
#[must_use = "the guard must be held for the browser session lifetime"]
pub struct BrowserPidGuard {
    pid: Option<u32>,
}

impl Drop for BrowserPidGuard {
    fn drop(&mut self) {
        if let Some(pid) = self.pid {
            unregister_browser_pid(pid);
        }
    }
}

/// Forcibly SIGKILL every browser process still registered as active and clear
/// the registry. Returns the number of PIDs signalled.
///
/// Intended for the app shutdown hook. A PID that has already exited simply
/// yields `ESRCH`, which is ignored.
#[allow(unsafe_code)]
pub fn kill_active_browsers() -> usize {
    let pids: Vec<u32> = match active_browser_pids().lock() {
        Ok(mut set) => set.drain().collect(),
        Err(_) => return 0,
    };
    for &pid in &pids {
        // Safety: `kill(2)` only sends a signal to a PID; it has no memory
        // effects. A dead/invalid PID returns ESRCH, which we ignore.
        unsafe {
            libc::kill(pid as libc::pid_t, libc::SIGKILL);
        }
    }
    pids.len()
}

/// Parse the PID from a Chrome `SingletonLock` symlink target.
///
/// The target has the form `<hostname>-<pid>` (e.g. `my-host.local-48732`).
/// The hostname can itself contain hyphens, so split on the last one.
fn parse_singleton_lock_pid(target: &str) -> Option<u32> {
    let (_, pid) = target.rsplit_once('-')?;
    pid.parse::<u32>().ok()
}

/// Whether a PID currently exists (alive, or alive-but-not-ours).
#[allow(unsafe_code)]
fn process_is_alive(pid: u32) -> bool {
    // Safety: `kill(pid, 0)` sends no signal; it only probes existence.
    let res = unsafe { libc::kill(pid as libc::pid_t, 0) };
    if res == 0 {
        return true;
    }
    // ESRCH => no such process (dead); EPERM => exists but not signalable by us.
    std::io::Error::last_os_error().raw_os_error() == Some(libc::EPERM)
}

/// Whether PID `pid` is a process whose command line references `profile_dir`
/// (i.e. a Chrome we launched with `--user-data-dir=<profile_dir>`). Used to
/// confirm a live SingletonLock owner is really our orphaned browser before
/// killing it.
fn process_uses_profile(pid: u32, profile_dir: &Path) -> bool {
    let output = std::process::Command::new("ps")
        .args(["-p", &pid.to_string(), "-o", "command="])
        .output();
    match output {
        Ok(output) => {
            String::from_utf8_lossy(&output.stdout).contains(profile_dir.to_string_lossy().as_ref())
        }
        Err(_) => false,
    }
}

/// Reclaim a browser profile whose `SingletonLock` is held by an orphan.
///
/// A hard kill of the app (SIGKILL/crash) runs neither `Drop` nor the shutdown
/// hook, so a launched browser can survive as an orphan still holding the
/// profile's `SingletonLock`; Chrome then aborts the next launch with
/// "Failed to create a ProcessSingleton". This is the failure mode that
/// motivated the kill-on-exit registry, for the case that registry can't cover.
///
/// We only reach `launch_browser` while holding the per-login file lock (see
/// [crate::scrape::run_scrape_async] / [crate::login_config::acquire_login_lock_with_metadata]),
/// so a *live* browser on this profile cannot belong to a concurrent scrape — it
/// is an orphan we may kill. We still confirm via `ps` that the live PID is a
/// process using this exact profile, to avoid signalling an unrelated reused PID.
#[allow(unsafe_code)]
fn reclaim_orphaned_profile_lock(profile_dir: &Path) {
    let lock_path = profile_dir.join("SingletonLock");
    let target = match std::fs::read_link(&lock_path) {
        Ok(target) => target,
        Err(_) => return, // no lock, or not a symlink — nothing to reclaim
    };
    let Some(pid) = parse_singleton_lock_pid(&target.to_string_lossy()) else {
        return;
    };

    if process_is_alive(pid) {
        if !process_uses_profile(pid, profile_dir) {
            eprintln!(
                "[browser] SingletonLock held by live pid {pid} that is not using this profile; leaving it"
            );
            return;
        }
        eprintln!("[browser] Reclaiming profile from orphaned browser pid {pid}");
        // Safety: see kill_active_browsers; signalling a confirmed orphan.
        unsafe {
            libc::kill(pid as libc::pid_t, libc::SIGKILL);
        }
        // Give the OS a moment to tear the process down and release the lock.
        std::thread::sleep(std::time::Duration::from_millis(200));
    } else {
        eprintln!("[browser] Removing stale SingletonLock (owner pid {pid} is gone)");
    }
    let _ = std::fs::remove_file(&lock_path);
}

/// Find the Chrome or Edge binary on the system.
pub fn find_chrome_binary() -> Result<PathBuf, Box<dyn Error>> {
    // Respect explicit overrides first so CI can force the browser installed by
    // the workflow instead of falling back to a system path that may behave
    // differently.
    for env_name in ["CHROME", "CHROME_BIN", "GOOGLE_CHROME_BIN"] {
        if let Some(path) = std::env::var_os(env_name) {
            let candidate = PathBuf::from(path);
            if candidate.exists() {
                eprintln!(
                    "[browser] Using browser from ${env_name}: {}",
                    candidate.display()
                );
                return Ok(candidate);
            }
            eprintln!(
                "[browser] Ignoring browser path from ${env_name} because it does not exist: {}",
                candidate.display()
            );
        }
    }

    // Prefer PATH before hard-coded locations so workflow-provided shims win.
    if let Ok(path) = which::which("google-chrome") {
        eprintln!(
            "[browser] Using browser from PATH lookup google-chrome: {}",
            path.display()
        );
        return Ok(path);
    }
    if let Ok(path) = which::which("google-chrome-stable") {
        eprintln!(
            "[browser] Using browser from PATH lookup google-chrome-stable: {}",
            path.display()
        );
        return Ok(path);
    }
    if let Ok(path) = which::which("google-chrome-beta") {
        eprintln!(
            "[browser] Using browser from PATH lookup google-chrome-beta: {}",
            path.display()
        );
        return Ok(path);
    }
    if let Ok(path) = which::which("chromium") {
        eprintln!(
            "[browser] Using browser from PATH lookup chromium: {}",
            path.display()
        );
        return Ok(path);
    }
    if let Ok(path) = which::which("chromium-browser") {
        eprintln!(
            "[browser] Using browser from PATH lookup chromium-browser: {}",
            path.display()
        );
        return Ok(path);
    }
    if let Ok(path) = which::which("microsoft-edge") {
        eprintln!(
            "[browser] Using browser from PATH lookup microsoft-edge: {}",
            path.display()
        );
        return Ok(path);
    }

    // Fallback to well-known installation paths.
    for candidate in chrome_candidates() {
        if candidate.exists() {
            eprintln!(
                "[browser] Using browser from well-known path: {}",
                candidate.display()
            );
            return Ok(candidate);
        }
    }

    Err("could not find Chrome or Edge binary; install Chrome or set PATH".into())
}

#[cfg(target_os = "macos")]
fn chrome_candidates() -> Vec<PathBuf> {
    vec![
        PathBuf::from("/Applications/Google Chrome.app/Contents/MacOS/Google Chrome"),
        PathBuf::from("/Applications/Microsoft Edge.app/Contents/MacOS/Microsoft Edge"),
        PathBuf::from("/Applications/Chromium.app/Contents/MacOS/Chromium"),
    ]
}

#[cfg(target_os = "windows")]
fn chrome_candidates() -> Vec<PathBuf> {
    let program_files =
        std::env::var("PROGRAMFILES").unwrap_or_else(|_| "C:\\Program Files".to_string());
    let program_files_x86 = std::env::var("PROGRAMFILES(X86)")
        .unwrap_or_else(|_| "C:\\Program Files (x86)".to_string());
    vec![
        PathBuf::from(&program_files).join("Google\\Chrome\\Application\\chrome.exe"),
        PathBuf::from(&program_files_x86).join("Google\\Chrome\\Application\\chrome.exe"),
        PathBuf::from(&program_files).join("Microsoft\\Edge\\Application\\msedge.exe"),
        PathBuf::from(&program_files_x86).join("Microsoft\\Edge\\Application\\msedge.exe"),
    ]
}

#[cfg(not(any(target_os = "macos", target_os = "windows")))]
fn chrome_candidates() -> Vec<PathBuf> {
    vec![
        PathBuf::from("/usr/bin/google-chrome-stable"),
        PathBuf::from("/usr/bin/google-chrome"),
        PathBuf::from("/usr/bin/chromium-browser"),
        PathBuf::from("/usr/bin/chromium"),
    ]
}

/// Launch a Chrome/Edge instance with the given profile directory.
///
/// Returns the `Browser` handle and a `tokio::task::JoinHandle` that drives
/// the chromiumoxide event handler loop.
pub async fn launch_browser(
    chrome_path: &Path,
    profile_dir: &Path,
    headless: bool,
) -> Result<(Browser, tokio::task::JoinHandle<()>, BrowserPidGuard), Box<dyn Error>> {
    std::fs::create_dir_all(profile_dir)?;

    // Recover from a browser orphaned by a previous hard kill before launching,
    // otherwise Chrome aborts with "Failed to create a ProcessSingleton".
    reclaim_orphaned_profile_lock(profile_dir);

    let mut builder = BrowserConfig::builder()
        .chrome_executable(chrome_path)
        .user_data_dir(profile_dir)
        .arg("--no-first-run")
        .arg("--no-default-browser-check")
        .arg("--disable-extensions")
        .launch_timeout(std::time::Duration::from_secs(30));

    let force_headless = headless || std::env::var_os("REFRESHMINT_BROWSER_HEADLESS").is_some();
    let is_linux_ci = cfg!(target_os = "linux") && std::env::var_os("CI").is_some();
    let use_headless = force_headless || is_linux_ci;
    eprintln!(
        "[browser] Launch config: chrome={}, profile={}, linux_ci={is_linux_ci}, force_headless={force_headless}",
        chrome_path.display(),
        profile_dir.display()
    );
    if use_headless {
        eprintln!("[browser] Launch mode: headless=old");
        builder = builder.headless_mode(HeadlessMode::True);
        if cfg!(target_os = "linux") {
            eprintln!("[browser] Launch flags: --no-sandbox --disable-dev-shm-usage");
            builder = builder.no_sandbox().arg("--disable-dev-shm-usage");
        }
    } else {
        eprintln!("[browser] Launch mode: headed");
        builder = builder.with_head();
    }

    let config = builder
        .build()
        .map_err(|e| format!("failed to build browser config: {e}"))?;

    let (mut browser, mut handler) = Browser::launch(config).await?;

    // Track the child PID so the shutdown hook can kill it if we exit before a
    // clean close (see `ACTIVE_BROWSER_PIDS`).
    let pid = browser
        .get_mut_child()
        .and_then(|child| child.as_mut_inner().id());
    let pid_guard = match pid {
        Some(pid) => {
            register_browser_pid(pid);
            eprintln!("[browser] Registered browser pid {pid} for shutdown cleanup");
            BrowserPidGuard { pid: Some(pid) }
        }
        None => {
            eprintln!("[browser] Could not determine browser pid; shutdown cleanup unavailable");
            BrowserPidGuard { pid: None }
        }
    };

    let handle = tokio::spawn(async move {
        eprintln!("[browser] Handler loop starting...");
        while let Some(result) = handler.next().await {
            if let Err(err) = result {
                match &err {
                    // Fatal: underlying transport or process is gone.
                    CdpError::Ws(_)
                    | CdpError::Io(_)
                    | CdpError::ChannelSendError(_)
                    | CdpError::LaunchExit(_, _)
                    | CdpError::LaunchTimeout(_)
                    | CdpError::LaunchIo(_, _) => {
                        eprintln!("[browser] Fatal handler error: {err}");
                        return;
                    }
                    // Non-fatal: a single malformed/unexpected CDP message.
                    // Log and keep processing so the session stays alive.
                    _ => {
                        eprintln!("[browser] Non-fatal handler error (continuing): {err}");
                    }
                }
            }
        }
        eprintln!("[browser] Handler loop ended.");
    });

    Ok((browser, handle, pid_guard))
}

/// Get a usable initial page handle for a newly launched browser.
///
/// Chromium often starts with an already-open tab. Prefer attaching to that tab
/// to avoid `Target.createTarget(about:blank)` hanging in some configurations.
pub async fn open_start_page(
    browser: &mut Browser,
) -> Result<chromiumoxide::Page, Box<dyn Error + Send + Sync>> {
    let create_timeout = std::time::Duration::from_secs(30);
    for attempt in 1..=2 {
        eprintln!("[browser] Creating initial about:blank page (attempt {attempt}/2)");
        match tokio::time::timeout(create_timeout, browser.new_page("about:blank")).await {
            Ok(Ok(page)) => {
                eprintln!("[browser] Created initial about:blank page on attempt {attempt}");
                return Ok(page);
            }
            Ok(Err(err)) => {
                eprintln!(
                    "[browser] Failed to create initial about:blank page on attempt {attempt}: {err}"
                );
                if attempt == 2 {
                    return Err(format!("failed to create initial page: {err}").into());
                }
            }
            Err(_) => {
                eprintln!(
                    "[browser] Timed out creating about:blank after {}s on attempt {attempt}",
                    create_timeout.as_secs()
                );
                if attempt == 2 {
                    return Err(format!(
                        "timed out after {}s creating initial page (about:blank)",
                        create_timeout.as_secs()
                    )
                    .into());
                }
            }
        }
        tokio::time::sleep(std::time::Duration::from_secs(1)).await;
    }

    Err("unreachable: initial page retry loop exhausted".into())
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;

    #[test]
    fn find_chrome_binary_returns_existing_path_or_error() {
        match find_chrome_binary() {
            Ok(path) => {
                assert!(path.exists(), "found path should exist: {}", path.display());
            }
            Err(e) => {
                // Acceptable in CI where Chrome may not be installed
                let msg = e.to_string();
                assert!(
                    msg.contains("could not find Chrome"),
                    "unexpected error: {msg}"
                );
            }
        }
    }

    #[test]
    fn chrome_candidates_are_absolute_paths() {
        for path in chrome_candidates() {
            assert!(
                path.is_absolute(),
                "candidate should be absolute: {}",
                path.display()
            );
        }
    }

    #[test]
    fn parse_singleton_lock_pid_handles_hyphenated_hostnames() {
        // hostname can contain hyphens and dots; the PID is after the last '-'.
        assert_eq!(
            parse_singleton_lock_pid("Yonathans-MacBook-Pro-2.local-48732"),
            Some(48732)
        );
        assert_eq!(parse_singleton_lock_pid("host-1"), Some(1));
        assert_eq!(parse_singleton_lock_pid("nohyphen"), None);
        assert_eq!(parse_singleton_lock_pid("host-notanumber"), None);
        assert_eq!(parse_singleton_lock_pid("trailingdash-"), None);
    }

    #[test]
    fn pid_guard_unregisters_and_kill_terminates_registered_process() {
        // A guard removes its PID from the registry when dropped, so an
        // in-process close never leaves a stale PID behind.
        let sentinel = 4_000_000_001u32;
        register_browser_pid(sentinel);
        assert!(active_browser_pids().lock().unwrap().contains(&sentinel));
        drop(BrowserPidGuard {
            pid: Some(sentinel),
        });
        assert!(!active_browser_pids().lock().unwrap().contains(&sentinel));

        // kill_active_browsers SIGKILLs a real registered child and drains it.
        let mut child = std::process::Command::new("sleep")
            .arg("30")
            .spawn()
            .expect("spawn sleep");
        let pid = child.id();
        register_browser_pid(pid);
        let killed = kill_active_browsers();
        assert!(
            killed >= 1,
            "expected at least the sleep child to be killed"
        );
        let status = child.wait().expect("wait for killed child");
        assert!(
            !status.success(),
            "killed process should not report success: {status:?}"
        );
        assert!(!active_browser_pids().lock().unwrap().contains(&pid));
    }
}
