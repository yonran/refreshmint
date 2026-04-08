use app_lib::scrape::browser;
use std::path::PathBuf;

fn temp_profile_dir(iter: usize) -> Result<PathBuf, Box<dyn std::error::Error>> {
    let dir = std::env::temp_dir().join(format!(
        "refreshmint-browser-startup-{}-{}-{}",
        std::process::id(),
        iter,
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_err(|err| format!("clock error: {err}"))?
            .as_nanos()
    ));
    std::fs::create_dir_all(&dir)?;
    Ok(dir)
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let iterations = std::env::args()
        .nth(1)
        .and_then(|arg| arg.parse::<usize>().ok())
        .unwrap_or(1);

    let chrome_path = browser::find_chrome_binary()?;
    eprintln!("Using browser: {}", chrome_path.display());
    eprintln!(
        "REFRESHMINT_BROWSER_HEADLESS={}",
        std::env::var("REFRESHMINT_BROWSER_HEADLESS").unwrap_or_default()
    );
    eprintln!("Iterations: {iterations}");

    for iter in 0..iterations {
        let profile_dir = temp_profile_dir(iter)?;
        eprintln!("[iter {iter}] Profile dir: {}", profile_dir.display());
        let (mut browser_instance, handler_handle) =
            browser::launch_browser(&chrome_path, &profile_dir, false).await?;
        eprintln!("[iter {iter}] Browser launched");

        match browser::open_start_page(&mut browser_instance).await {
            Ok(page) => {
                let url = page.url().await?;
                eprintln!("[iter {iter}] Opened page successfully: {:?}", url);
            }
            Err(err) => {
                eprintln!("[iter {iter}] Failed to open page: {err}");
                handler_handle.abort();
                return Err(err.to_string().into());
            }
        }

        drop(browser_instance);
        handler_handle.abort();
    }

    Ok(())
}
