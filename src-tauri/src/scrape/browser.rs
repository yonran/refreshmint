use std::error::Error;
use std::path::{Path, PathBuf};

use chromiumoxide::browser::{Browser, BrowserConfig};
use chromiumoxide::error::CdpError;
use futures::StreamExt;

/// Find the Chrome or Edge binary on the system.
pub fn find_chrome_binary() -> Result<PathBuf, Box<dyn Error>> {
    // Check well-known paths first
    let candidates = chrome_candidates();
    for candidate in &candidates {
        if candidate.exists() {
            return Ok(candidate.clone());
        }
    }

    // Fallback to PATH search
    if let Ok(path) = which::which("google-chrome-stable") {
        return Ok(path);
    }
    if let Ok(path) = which::which("google-chrome") {
        return Ok(path);
    }
    if let Ok(path) = which::which("chromium") {
        return Ok(path);
    }
    if let Ok(path) = which::which("microsoft-edge") {
        return Ok(path);
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
) -> Result<(Browser, tokio::task::JoinHandle<()>), Box<dyn Error>> {
    std::fs::create_dir_all(profile_dir)?;

    let config = BrowserConfig::builder()
        .chrome_executable(chrome_path)
        .user_data_dir(profile_dir)
        .with_head()
        .arg("--no-first-run")
        .arg("--no-default-browser-check")
        .arg("--disable-extensions")
        .launch_timeout(std::time::Duration::from_secs(30))
        .build()
        .map_err(|e| format!("failed to build browser config: {e}"))?;

    let (browser, mut handler) = Browser::launch(config).await?;

    let handle = tokio::spawn(async move {
        eprintln!("[browser] Handler loop starting...");
        let mut count = 0u64;
        loop {
            match handler.next().await {
                Some(Ok(())) => {
                    count += 1;
                    if count <= 5 || count % 100 == 0 {
                        eprintln!("[browser] Handler event #{count}");
                    }
                }
                Some(Err(err)) => {
                    match &err {
                        // Fatal: underlying transport or process is gone.
                        CdpError::Ws(_)
                        | CdpError::Io(_)
                        | CdpError::ChannelSendError(_)
                        | CdpError::LaunchExit(_, _)
                        | CdpError::LaunchTimeout(_)
                        | CdpError::LaunchIo(_, _) => {
                            eprintln!("[browser] Fatal handler error after {count} events: {err}");
                            break;
                        }
                        // Non-fatal: a single malformed/unexpected CDP message.
                        // Log and keep processing so the session stays alive.
                        _ => {
                            eprintln!("[browser] Non-fatal handler error after {count} events (continuing): {err}");
                        }
                    }
                }
                None => {
                    eprintln!("[browser] Handler stream ended after {count} events.");
                    break;
                }
            }
        }
    });

    Ok((browser, handle))
}

/// Get a usable initial page handle for a newly launched browser.
///
/// Chromium often starts with an already-open tab. Prefer attaching to that tab
/// to avoid `Target.createTarget(about:blank)` hanging in some configurations.
pub async fn open_start_page(
    browser: &mut Browser,
) -> Result<chromiumoxide::Page, Box<dyn Error + Send + Sync>> {
    browser.fetch_targets().await?;
    tokio::time::sleep(std::time::Duration::from_millis(250)).await;

    if let Some(page) = browser.pages().await?.into_iter().next() {
        return Ok(page);
    }

    let create_timeout = std::time::Duration::from_secs(10);
    match tokio::time::timeout(create_timeout, browser.new_page("about:blank")).await {
        Ok(Ok(page)) => Ok(page),
        Ok(Err(err)) => Err(format!("failed to create initial page: {err}").into()),
        Err(_) => Err(format!(
            "timed out after {}s creating initial page (about:blank)",
            create_timeout.as_secs()
        )
        .into()),
    }
}

#[cfg(test)]
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
}
