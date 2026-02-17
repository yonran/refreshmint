pub mod browser;
pub mod debug;
pub mod js_api;
pub mod profile;
pub mod sandbox;

use std::path::PathBuf;
use std::sync::Arc;
use tokio::sync::Mutex;

use crate::secret::SecretStore;

/// Configuration for a scrape run.
pub struct ScrapeConfig {
    pub account: String,
    pub extension_name: String,
    pub ledger_dir: PathBuf,
    pub profile_override: Option<PathBuf>,
}

/// List extension names that have a runnable `driver.mjs` script.
pub fn list_runnable_extensions(
    ledger_dir: &std::path::Path,
) -> Result<Vec<String>, Box<dyn std::error::Error>> {
    let extensions_dir = ledger_dir.join("extensions");
    if !extensions_dir.exists() {
        return Ok(Vec::new());
    }

    let mut extensions = Vec::new();
    for entry in std::fs::read_dir(&extensions_dir)? {
        let entry = entry?;
        let entry_path = entry.path();
        if !entry_path.is_dir() {
            continue;
        }
        if !entry_path.join("driver.mjs").is_file() {
            continue;
        }
        let Some(name) = entry.file_name().to_str().map(ToOwned::to_owned) else {
            continue;
        };
        extensions.push(name);
    }

    extensions.sort();
    Ok(extensions)
}

/// Run the full scrape orchestration.
///
/// This is the async core called from `run_scrape` which sets up a tokio runtime.
pub async fn run_scrape_async(
    config: ScrapeConfig,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    // 1. Locate the driver script
    let driver_path = config
        .ledger_dir
        .join("extensions")
        .join(&config.extension_name)
        .join("driver.mjs");
    if !driver_path.exists() {
        return Err(format!("driver script not found: {}", driver_path.display()).into());
    }

    // 2. Create secret store for the account
    let secret_store = SecretStore::new(config.account.clone());

    // 3. Resolve browser profile directory
    let profile_dir = profile::resolve_profile_dir(
        &config.ledger_dir,
        &config.account,
        config.profile_override.as_deref(),
    )
    .map_err(|e| -> Box<dyn std::error::Error + Send + Sync> { e.to_string().into() })?;

    // 4. Resolve download directory
    let download_dir =
        profile::resolve_download_dir(&config.extension_name, config.profile_override.as_deref())
            .map_err(|e| -> Box<dyn std::error::Error + Send + Sync> { e.to_string().into() })?;
    std::fs::create_dir_all(&download_dir)?;

    // 5. Find and launch browser
    let chrome_path = browser::find_chrome_binary()
        .map_err(|e| -> Box<dyn std::error::Error + Send + Sync> { e.to_string().into() })?;
    eprintln!("Using browser: {}", chrome_path.display());
    eprintln!("Profile dir: {}", profile_dir.display());

    eprintln!("Launching browser...");
    let (browser_instance, handler_handle) = browser::launch_browser(&chrome_path, &profile_dir)
        .await
        .map_err(|e| -> Box<dyn std::error::Error + Send + Sync> { e.to_string().into() })?;
    eprintln!("Browser launched.");

    // 6. Open a new page
    eprintln!("Opening new page...");
    let page = browser_instance.new_page("about:blank").await?;
    eprintln!("Page opened.");

    // 7. Set up shared state
    let output_dir = config
        .ledger_dir
        .join("extensions")
        .join(&config.extension_name)
        .join("output");
    std::fs::create_dir_all(&output_dir)?;

    let page_inner = Arc::new(Mutex::new(js_api::PageInner {
        page,
        secret_store,
        download_dir,
    }));

    let refreshmint_inner = Arc::new(Mutex::new(js_api::RefreshmintInner { output_dir }));

    // 8. Run the driver script in the sandbox
    eprintln!("Running driver: {}", driver_path.display());
    let result = sandbox::run_driver(&driver_path, page_inner, refreshmint_inner).await;
    eprintln!("Driver finished: {result:?}");

    // 9. Close browser
    eprintln!("Closing browser...");
    drop(browser_instance);
    // Wait briefly for handler to clean up, but don't block indefinitely
    let _ = tokio::time::timeout(std::time::Duration::from_secs(5), handler_handle).await;
    eprintln!("Done.");

    result
}

/// Synchronous entry point that creates a tokio runtime and runs the scrape.
pub fn run_scrape(config: ScrapeConfig) -> Result<(), Box<dyn std::error::Error>> {
    let rt = tokio::runtime::Runtime::new()?;
    rt.block_on(run_scrape_async(config))
        .map_err(|e| -> Box<dyn std::error::Error> { e })?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::list_runnable_extensions;
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
        fs::create_dir_all(&dir).unwrap_or_else(|err| {
            panic!("failed to create temp dir: {err}");
        });
        dir
    }

    #[test]
    fn list_runnable_extensions_filters_and_sorts() {
        let root = create_temp_dir("scrape-ext");
        let extensions = root.join("extensions");
        fs::create_dir_all(extensions.join("beta")).unwrap_or_else(|err| {
            panic!("failed to create beta extension: {err}");
        });
        fs::create_dir_all(extensions.join("alpha")).unwrap_or_else(|err| {
            panic!("failed to create alpha extension: {err}");
        });
        fs::create_dir_all(extensions.join("empty")).unwrap_or_else(|err| {
            panic!("failed to create empty extension: {err}");
        });
        fs::write(extensions.join("alpha").join("driver.mjs"), "// alpha").unwrap_or_else(|err| {
            panic!("failed to write alpha driver: {err}");
        });
        fs::write(extensions.join("beta").join("driver.mjs"), "// beta").unwrap_or_else(|err| {
            panic!("failed to write beta driver: {err}");
        });

        let found = list_runnable_extensions(&root).unwrap_or_else(|err| {
            panic!("unexpected list_runnable_extensions error: {err}");
        });

        assert_eq!(found, vec!["alpha".to_string(), "beta".to_string()]);
        let _ = fs::remove_dir_all(&root);
    }
}
