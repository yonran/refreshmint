pub mod browser;
pub mod debug;
pub mod js_api;
pub mod profile;
pub mod sandbox;

use serde::Deserialize;
use std::collections::BTreeSet;
use std::path::Path;
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
    pub prompt_overrides: js_api::PromptOverrides,
    pub prompt_requires_override: bool,
}

#[derive(Deserialize)]
struct ExtensionManifest {
    #[serde(default)]
    secrets: std::collections::BTreeMap<String, Vec<String>>,
    #[serde(default)]
    account: Option<String>,
    #[serde(default)]
    extract: Option<String>,
    #[serde(default)]
    rules: Option<String>,
    #[serde(default, rename = "idField")]
    id_field: Option<String>,
    #[serde(default, rename = "autoExtract")]
    auto_extract: Option<bool>,
}

/// Parsed extension manifest with all fields.
pub struct ParsedManifest {
    pub secrets: js_api::SecretDeclarations,
    pub account: Option<String>,
    pub extract: Option<String>,
    pub rules: Option<String>,
    pub id_field: Option<String>,
    pub auto_extract: bool,
}

/// Load and parse the full extension manifest.
pub fn load_manifest(
    extension_dir: &Path,
) -> Result<ParsedManifest, Box<dyn std::error::Error + Send + Sync>> {
    let manifest_path = extension_dir.join("manifest.json");
    let manifest_text = std::fs::read_to_string(&manifest_path)?;
    let manifest: ExtensionManifest = serde_json::from_str(&manifest_text).map_err(|error| {
        format!(
            "invalid {}: {error}",
            manifest_path
                .strip_prefix(extension_dir)
                .unwrap_or(&manifest_path)
                .display()
        )
    })?;

    let mut declared = js_api::SecretDeclarations::new();
    for (domain_input, names) in &manifest.secrets {
        let domain = normalize_manifest_domain(domain_input);
        if domain.is_empty() {
            return Err(format!(
                "invalid manifest secrets domain '{domain_input}' in {}",
                manifest_path.display()
            )
            .into());
        }
        let mut normalized = BTreeSet::new();
        for name in names {
            let trimmed = name.trim();
            if trimmed.is_empty() {
                return Err(format!(
                    "manifest secrets for domain '{domain}' contains an empty name in {}",
                    manifest_path.display()
                )
                .into());
            }
            normalized.insert(trimmed.to_string());
        }
        declared
            .entry(domain)
            .or_default()
            .extend(normalized.into_iter());
    }

    Ok(ParsedManifest {
        secrets: declared,
        account: manifest.account,
        extract: manifest.extract,
        rules: manifest.rules,
        id_field: manifest.id_field,
        auto_extract: manifest.auto_extract.unwrap_or(true),
    })
}

/// Generate a scrape session ID from the current timestamp.
pub fn generate_scrape_session_id() -> String {
    chrono::Local::now().format("%Y%m%d-%H%M%S").to_string()
}

/// Document info sidecar written alongside each evidence document.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct DocumentInfo {
    #[serde(rename = "mimeType")]
    pub mime_type: String,
    #[serde(rename = "originalUrl", skip_serializing_if = "Option::is_none")]
    pub original_url: Option<String>,
    #[serde(rename = "scrapedAt")]
    pub scraped_at: String,
    #[serde(rename = "extensionName")]
    pub extension_name: String,
    #[serde(rename = "accountName")]
    pub account_name: String,
    #[serde(rename = "scrapeSessionId")]
    pub scrape_session_id: String,
    #[serde(rename = "coverageEndDate")]
    pub coverage_end_date: String,
    #[serde(rename = "dateRangeStart", skip_serializing_if = "Option::is_none")]
    pub date_range_start: Option<String>,
    #[serde(rename = "dateRangeEnd", skip_serializing_if = "Option::is_none")]
    pub date_range_end: Option<String>,
}

/// Finalize staged resources: move them to `accounts/<account>/documents/`
/// with date-prefixed filenames and write `-info.json` sidecars.
pub fn finalize_staged_resources(
    inner: &js_api::RefreshmintInner,
) -> Result<Vec<String>, Box<dyn std::error::Error + Send + Sync>> {
    let documents_dir =
        crate::account_journal::account_documents_dir(&inner.ledger_dir, &inner.account_name);
    std::fs::create_dir_all(&documents_dir)?;

    let scraped_at = chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Secs, true);
    let fallback_date = chrono::Local::now().format("%Y-%m-%d").to_string();
    let mut finalized_names = Vec::new();

    for resource in &inner.staged_resources {
        let coverage_date = resource
            .coverage_end_date
            .as_deref()
            .unwrap_or(&fallback_date);

        let final_filename =
            date_prefixed_filename(coverage_date, &resource.filename, &documents_dir);
        let final_path = documents_dir.join(&final_filename);

        // Copy from staging to documents dir
        std::fs::copy(&resource.staging_path, &final_path).map_err(|e| {
            format!(
                "failed to copy {} to {}: {e}",
                resource.staging_path.display(),
                final_path.display()
            )
        })?;

        // Guess MIME type from extension
        let mime = resource
            .mime_type
            .clone()
            .unwrap_or_else(|| guess_mime_type(&resource.filename));

        // Write sidecar
        let info = DocumentInfo {
            mime_type: mime,
            original_url: resource.original_url.clone(),
            scraped_at: scraped_at.clone(),
            extension_name: inner.extension_name.clone(),
            account_name: inner.account_name.clone(),
            scrape_session_id: inner.scrape_session_id.clone(),
            coverage_end_date: coverage_date.to_string(),
            date_range_start: inner.session_metadata.date_range_start.clone(),
            date_range_end: inner.session_metadata.date_range_end.clone(),
        };

        let sidecar_path = documents_dir.join(format!("{final_filename}-info.json"));
        let sidecar_json = serde_json::to_string_pretty(&info)?;
        std::fs::write(&sidecar_path, sidecar_json)?;

        finalized_names.push(final_filename);
    }

    Ok(finalized_names)
}

/// Generate a date-prefixed filename, handling collisions with incrementing suffix.
fn date_prefixed_filename(date: &str, original: &str, dir: &Path) -> String {
    let candidate = format!("{date}-{original}");
    if !dir.join(&candidate).exists() {
        return candidate;
    }

    // Split into stem and extension for the incrementing suffix
    let dot_pos = original.rfind('.');
    let (stem, ext) = match dot_pos {
        Some(pos) => (&original[..pos], &original[pos..]),
        None => (original, ""),
    };

    for i in 2..1000 {
        let candidate = format!("{date}-{stem}-{i}{ext}");
        if !dir.join(&candidate).exists() {
            return candidate;
        }
    }

    // Fallback with timestamp
    format!("{date}-{stem}-{}{}", std::process::id(), ext)
}

/// Guess MIME type from file extension.
fn guess_mime_type(filename: &str) -> String {
    let ext = filename
        .rsplit('.')
        .next()
        .unwrap_or("")
        .to_ascii_lowercase();
    match ext.as_str() {
        "csv" => "text/csv",
        "html" | "htm" => "text/html",
        "pdf" => "application/pdf",
        "json" => "application/json",
        "txt" => "text/plain",
        "xml" => "application/xml",
        "ofx" | "qfx" => "application/x-ofx",
        _ => "application/octet-stream",
    }
    .to_string()
}

pub(crate) fn load_manifest_secret_declarations(
    extension_dir: &Path,
) -> Result<js_api::SecretDeclarations, Box<dyn std::error::Error + Send + Sync>> {
    let manifest = load_manifest(extension_dir)?;
    Ok(manifest.secrets)
}

fn normalize_manifest_domain(input: &str) -> String {
    let trimmed = input.trim();
    if trimmed.is_empty() {
        return String::new();
    }

    let without_scheme = trimmed
        .strip_prefix("https://")
        .or_else(|| trimmed.strip_prefix("http://"))
        .unwrap_or(trimmed);

    without_scheme
        .split('/')
        .next()
        .unwrap_or("")
        .split(':')
        .next()
        .unwrap_or("")
        .to_ascii_lowercase()
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
    let extension_dir = config
        .ledger_dir
        .join("extensions")
        .join(&config.extension_name);
    // 1. Locate the driver script
    let driver_path = extension_dir.join("driver.mjs");
    if !driver_path.exists() {
        return Err(format!("driver script not found: {}", driver_path.display()).into());
    }

    // Load full manifest
    let manifest = load_manifest(&extension_dir)?;
    let declared_secrets = manifest.secrets;

    // Resolve account name: manifest "account" field, or use config.account
    let account_name = manifest.account.unwrap_or_else(|| config.account.clone());

    // Generate scrape session ID
    let scrape_session_id = generate_scrape_session_id();
    eprintln!("Scrape session: {scrape_session_id}");

    // 2. Create secret store for the account
    let secret_store = SecretStore::new(account_name.clone());

    // 3. Resolve browser profile directory
    let profile_dir = profile::resolve_profile_dir(
        &config.ledger_dir,
        &account_name,
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
    let (mut browser_instance, handler_handle) =
        browser::launch_browser(&chrome_path, &profile_dir)
            .await
            .map_err(|e| -> Box<dyn std::error::Error + Send + Sync> { e.to_string().into() })?;
    eprintln!("Browser launched.");

    // 6. Open a new page
    eprintln!("Opening new page...");
    let page = browser::open_start_page(&mut browser_instance).await?;
    eprintln!("Page opened.");

    // 7. Set up shared state
    let output_dir = extension_dir.join("output");
    std::fs::create_dir_all(&output_dir)?;

    let page_inner = Arc::new(Mutex::new(js_api::PageInner {
        page,
        secret_store,
        declared_secrets,
        download_dir,
    }));

    let refreshmint_inner = Arc::new(Mutex::new(js_api::RefreshmintInner {
        output_dir,
        prompt_overrides: config.prompt_overrides.clone(),
        prompt_requires_override: config.prompt_requires_override,
        session_metadata: js_api::SessionMetadata::default(),
        staged_resources: Vec::new(),
        scrape_session_id: scrape_session_id.clone(),
        extension_name: config.extension_name.clone(),
        account_name: account_name.clone(),
        ledger_dir: config.ledger_dir.clone(),
    }));

    // 8. Run the driver script in the sandbox
    eprintln!("Running driver: {}", driver_path.display());
    let result = sandbox::run_driver(&driver_path, page_inner, refreshmint_inner.clone()).await;
    eprintln!("Driver finished: {result:?}");

    // 9. Finalize staged resources (move to accounts/<name>/documents/)
    if result.is_ok() {
        let inner = refreshmint_inner.lock().await;
        if !inner.staged_resources.is_empty() {
            eprintln!(
                "Finalizing {} staged resources...",
                inner.staged_resources.len()
            );
            match finalize_staged_resources(&inner) {
                Ok(names) => {
                    for name in &names {
                        eprintln!("  -> {name}");
                    }
                }
                Err(e) => {
                    eprintln!("Warning: failed to finalize staged resources: {e}");
                }
            }
        }
    }

    // 10. Close browser
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
    use super::{
        list_runnable_extensions, load_manifest_secret_declarations, normalize_manifest_domain,
    };
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

    #[test]
    fn normalize_manifest_domain_accepts_host_or_url() {
        assert_eq!(normalize_manifest_domain("example.com"), "example.com");
        assert_eq!(
            normalize_manifest_domain("https://Example.com/login"),
            "example.com"
        );
    }

    #[test]
    fn load_manifest_secret_declarations_reads_and_normalizes() {
        let root = create_temp_dir("scrape-manifest-secrets");
        let ext = root.join("ext");
        fs::create_dir_all(&ext)
            .unwrap_or_else(|err| panic!("failed to create extension dir: {err}"));
        let manifest = r#"{
  "name": "demo",
  "secrets": {
    "Example.com": ["username", "password", "password"],
    "https://sub.example.com/login": ["otp"]
  }
}"#;
        fs::write(ext.join("manifest.json"), manifest)
            .unwrap_or_else(|err| panic!("failed to write manifest: {err}"));

        let declared = load_manifest_secret_declarations(&ext)
            .unwrap_or_else(|err| panic!("failed to load manifest secrets: {err}"));
        let example = declared
            .get("example.com")
            .unwrap_or_else(|| panic!("missing normalized example.com declaration"));
        assert!(example.contains("username"));
        assert!(example.contains("password"));
        assert_eq!(example.len(), 2);
        let sub = declared
            .get("sub.example.com")
            .unwrap_or_else(|| panic!("missing normalized subdomain declaration"));
        assert!(sub.contains("otp"));

        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn load_manifest_secret_declarations_rejects_empty_name() {
        let root = create_temp_dir("scrape-manifest-invalid");
        let ext = root.join("ext");
        fs::create_dir_all(&ext)
            .unwrap_or_else(|err| panic!("failed to create extension dir: {err}"));
        let manifest = r#"{
  "name": "demo",
  "secrets": {
    "example.com": ["ok", " "]
  }
}"#;
        fs::write(ext.join("manifest.json"), manifest)
            .unwrap_or_else(|err| panic!("failed to write manifest: {err}"));

        let err = load_manifest_secret_declarations(&ext).err();
        assert!(err.is_some(), "expected empty secret name to fail");

        let _ = fs::remove_dir_all(&root);
    }
}
