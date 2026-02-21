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
    pub login_name: String,
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
    #[serde(rename = "loginName", alias = "accountName")]
    pub login_name: String,
    #[serde(default = "default_document_label")]
    pub label: String,
    #[serde(rename = "scrapeSessionId")]
    pub scrape_session_id: String,
    #[serde(rename = "coverageEndDate")]
    pub coverage_end_date: String,
    #[serde(rename = "dateRangeStart", skip_serializing_if = "Option::is_none")]
    pub date_range_start: Option<String>,
    #[serde(rename = "dateRangeEnd", skip_serializing_if = "Option::is_none")]
    pub date_range_end: Option<String>,
}

fn default_document_label() -> String {
    "_default".to_string()
}

/// Finalize staged resources: move them to `logins/<login>/accounts/<label>/documents/`
/// with date-prefixed filenames and write `-info.json` sidecars.
pub fn finalize_staged_resources(
    inner: &js_api::RefreshmintInner,
) -> Result<Vec<String>, Box<dyn std::error::Error + Send + Sync>> {
    let scraped_at = chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Secs, true);
    let fallback_date = chrono::Local::now().format("%Y-%m-%d").to_string();
    let mut finalized_names = Vec::new();
    let mut labels_seen = std::collections::BTreeSet::new();
    let mut resources_with_labels = Vec::new();

    for resource in &inner.staged_resources {
        let label = if let Some(raw) = resource.label.as_ref() {
            crate::login_config::validate_label(raw).map_err(|err| {
                format!("invalid label '{}' for '{}': {err}", raw, resource.filename)
            })?;
            raw.clone()
        } else {
            "_default".to_string()
        };

        labels_seen.insert(label.clone());
        resources_with_labels.push((resource, label));
    }

    let mut login_config =
        crate::login_config::read_login_config(&inner.ledger_dir, &inner.login_name);
    let mut login_config_changed = false;
    for label in labels_seen {
        if let std::collections::btree_map::Entry::Vacant(entry) =
            login_config.accounts.entry(label)
        {
            entry.insert(crate::login_config::LoginAccountConfig { gl_account: None });
            login_config_changed = true;
        }
    }
    if login_config_changed {
        crate::login_config::write_login_config(
            &inner.ledger_dir,
            &inner.login_name,
            &login_config,
        )?;
    }

    for (resource, label) in resources_with_labels {
        let coverage_date = resource
            .coverage_end_date
            .as_deref()
            .unwrap_or(&fallback_date);
        let documents_dir = crate::login_config::login_account_documents_dir(
            &inner.ledger_dir,
            &inner.login_name,
            &label,
        );
        std::fs::create_dir_all(&documents_dir)?;

        let final_filename =
            date_prefixed_filename(coverage_date, &resource.filename, &documents_dir);
        let final_path = documents_dir.join(&final_filename);
        if let Some(parent) = final_path.parent() {
            std::fs::create_dir_all(parent)?;
        }

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
            login_name: inner.login_name.clone(),
            label: label.clone(),
            scrape_session_id: inner.scrape_session_id.clone(),
            coverage_end_date: coverage_date.to_string(),
            date_range_start: inner.session_metadata.date_range_start.clone(),
            date_range_end: inner.session_metadata.date_range_end.clone(),
        };

        let sidecar_path = documents_dir.join(format!("{final_filename}-info.json"));
        if let Some(parent) = sidecar_path.parent() {
            std::fs::create_dir_all(parent)?;
        }
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
    let login_name = config.login_name.clone();
    let _login_lock = crate::login_config::acquire_login_lock(&config.ledger_dir, &login_name)
        .map_err(|err| -> Box<dyn std::error::Error + Send + Sync> { err })?;

    let extension_dir =
        crate::account_config::resolve_extension_dir(&config.ledger_dir, &config.extension_name);
    // 1. Locate the driver script
    let driver_path = extension_dir.join("driver.mjs");
    if !driver_path.exists() {
        return Err(format!("driver script not found: {}", driver_path.display()).into());
    }

    // Load full manifest
    let manifest = load_manifest(&extension_dir)?;
    let declared_secrets = manifest.secrets;

    // Generate scrape session ID
    let scrape_session_id = generate_scrape_session_id();
    eprintln!("Scrape session: {scrape_session_id}");

    // 2. Create secret store for the login
    let secret_store = SecretStore::new(format!("login/{login_name}"));

    // 3. Resolve browser profile directory
    let profile_dir = profile::resolve_profile_dir(
        &config.ledger_dir,
        &login_name,
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
    let browser = Arc::new(Mutex::new(browser_instance));

    // 6. Open a new page
    eprintln!("Opening new page...");
    let page = {
        let mut guard = browser.lock().await;
        browser::open_start_page(&mut guard).await?
    };
    let mut known_tab_ids = BTreeSet::new();
    known_tab_ids.insert(page.target_id().as_ref().to_string());
    eprintln!("Page opened.");

    // 7. Set up shared state
    let output_dir = extension_dir.join("output");
    std::fs::create_dir_all(&output_dir)?;

    let page_inner = Arc::new(Mutex::new(js_api::PageInner {
        page,
        browser: browser.clone(),
        known_tab_ids,
        secret_store,
        declared_secrets,
        download_dir,
    }));

    let refreshmint_inner = Arc::new(Mutex::new(js_api::RefreshmintInner {
        output_dir,
        prompt_overrides: config.prompt_overrides.clone(),
        prompt_requires_override: config.prompt_requires_override,
        debug_output_sink: None,
        session_metadata: js_api::SessionMetadata::default(),
        staged_resources: Vec::new(),
        scrape_session_id: scrape_session_id.clone(),
        extension_name: config.extension_name.clone(),
        account_name: login_name.clone(),
        login_name: login_name.clone(),
        ledger_dir: config.ledger_dir.clone(),
    }));

    // 8. Run the driver script in the sandbox
    eprintln!("Running driver: {}", driver_path.display());
    let mut result = sandbox::run_driver(&driver_path, page_inner, refreshmint_inner.clone()).await;
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
                    result = Err(format!("failed to finalize staged resources: {e}").into());
                }
            }
        }
    }

    // 10. Auto-save extension in login config if not already set
    if result.is_ok() {
        let mut existing = crate::login_config::read_login_config(&config.ledger_dir, &login_name);
        let should_save = existing
            .extension
            .as_deref()
            .map(str::trim)
            .map(str::is_empty)
            .unwrap_or(true);
        if should_save {
            existing.extension = Some(config.extension_name.clone());
            if let Err(e) =
                crate::login_config::write_login_config(&config.ledger_dir, &login_name, &existing)
            {
                eprintln!("Warning: failed to save login config: {e}");
            }
        }
    }

    // 11. Close browser
    eprintln!("Closing browser...");
    {
        let guard = browser.lock().await;
        let _ = guard.close().await;
    }
    drop(browser);
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
        finalize_staged_resources, list_runnable_extensions, load_manifest_secret_declarations,
        normalize_manifest_domain,
    };
    use crate::login_config::login_account_documents_dir;
    use crate::scrape::js_api::{
        PromptOverrides, RefreshmintInner, SessionMetadata, StagedResource,
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

    #[test]
    fn finalize_staged_resources_creates_parent_directories_for_nested_filenames() {
        let root = create_temp_dir("scrape-finalize-nested");
        let ledger_dir = root.join("ledger.refreshmint");
        fs::create_dir_all(&ledger_dir).unwrap_or_else(|err| {
            panic!("failed to create ledger dir: {err}");
        });

        let staged_path = root.join("staged-nested.pdf");
        fs::write(&staged_path, b"nested").unwrap_or_else(|err| {
            panic!("failed to write staged file: {err}");
        });

        let login_name = "chase-personal".to_string();
        let inner = RefreshmintInner {
            output_dir: root.join("output"),
            prompt_overrides: PromptOverrides::new(),
            prompt_requires_override: false,
            debug_output_sink: None,
            session_metadata: SessionMetadata::default(),
            staged_resources: vec![StagedResource {
                filename: "statements/2026/jan.pdf".to_string(),
                staging_path: staged_path,
                coverage_end_date: Some("2026-01-31".to_string()),
                original_url: Some("https://example.com/export".to_string()),
                mime_type: Some("application/pdf".to_string()),
                label: Some("checking".to_string()),
            }],
            scrape_session_id: "nested-test".to_string(),
            extension_name: "nested-ext".to_string(),
            account_name: login_name.clone(),
            login_name: login_name.clone(),
            ledger_dir: ledger_dir.clone(),
        };

        let finalized = finalize_staged_resources(&inner).unwrap_or_else(|err| {
            panic!("finalize_staged_resources failed: {err}");
        });
        assert_eq!(finalized, vec!["2026-01-31-statements/2026/jan.pdf"]);

        let documents_dir = login_account_documents_dir(&ledger_dir, &login_name, "checking");
        let finalized_path = documents_dir.join(&finalized[0]);
        assert!(finalized_path.exists(), "expected finalized file to exist");
        let bytes = fs::read(&finalized_path).unwrap_or_else(|err| {
            panic!("failed to read finalized file: {err}");
        });
        assert_eq!(bytes, b"nested");

        let sidecar_path = documents_dir.join(format!("{}-info.json", finalized[0]));
        assert!(sidecar_path.exists(), "expected sidecar file to exist");
        let sidecar = fs::read_to_string(&sidecar_path)
            .unwrap_or_else(|err| panic!("failed to read sidecar file: {err}"));
        assert!(sidecar.contains("\"loginName\": \"chase-personal\""));
        assert!(sidecar.contains("\"label\": \"checking\""));

        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn finalize_staged_resources_rejects_invalid_label() {
        let root = create_temp_dir("scrape-finalize-invalid-label");
        let ledger_dir = root.join("ledger.refreshmint");
        fs::create_dir_all(&ledger_dir).unwrap_or_else(|err| {
            panic!("failed to create ledger dir: {err}");
        });
        let staged_path = root.join("staged.pdf");
        fs::write(&staged_path, b"pdf").unwrap_or_else(|err| {
            panic!("failed to write staged file: {err}");
        });

        let inner = RefreshmintInner {
            output_dir: root.join("output"),
            prompt_overrides: PromptOverrides::new(),
            prompt_requires_override: false,
            debug_output_sink: None,
            session_metadata: SessionMetadata::default(),
            staged_resources: vec![StagedResource {
                filename: "jan.pdf".to_string(),
                staging_path: staged_path,
                coverage_end_date: Some("2026-01-31".to_string()),
                original_url: None,
                mime_type: Some("application/pdf".to_string()),
                label: Some("bad/label".to_string()),
            }],
            scrape_session_id: "invalid-label-test".to_string(),
            extension_name: "nested-ext".to_string(),
            account_name: "chase-personal".to_string(),
            login_name: "chase-personal".to_string(),
            ledger_dir: ledger_dir.clone(),
        };

        let err = finalize_staged_resources(&inner)
            .err()
            .unwrap_or_else(|| panic!("expected invalid label error"));
        let message = err.to_string();
        assert!(message.contains("invalid label"));
        assert!(message.contains("bad/label"));

        let _ = fs::remove_dir_all(&root);
    }
}
