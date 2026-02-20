use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

/// Per-account configuration stored in `accounts/<name>/config.json`.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct AccountConfig {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub extension: Option<String>,
}

/// Return the path to `accounts/<account_name>/config.json`.
fn config_path(ledger_dir: &Path, account_name: &str) -> PathBuf {
    ledger_dir
        .join("accounts")
        .join(account_name)
        .join("config.json")
}

/// Read the account config, returning defaults if the file is missing.
pub fn read_account_config(ledger_dir: &Path, account_name: &str) -> AccountConfig {
    let path = config_path(ledger_dir, account_name);
    match std::fs::read_to_string(&path) {
        Ok(text) => serde_json::from_str(&text).unwrap_or_default(),
        Err(_) => AccountConfig::default(),
    }
}

/// Write the account config atomically.
pub fn write_account_config(
    ledger_dir: &Path,
    account_name: &str,
    config: &AccountConfig,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let path = config_path(ledger_dir, account_name);
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let json = serde_json::to_string_pretty(config)?;
    std::fs::write(&path, json)?;
    Ok(())
}

/// Determine whether an extension value looks like a path (contains `/` or `\`
/// or starts with `.`) versus a plain extension name.
fn is_extension_path(value: &str) -> bool {
    value.contains('/') || value.contains('\\') || value.starts_with('.')
}

/// Resolve an extension value to a directory path.
///
/// If the value looks like a path, return it directly as a `PathBuf`.
/// Otherwise treat it as an extension name under `extensions/` in the ledger.
pub fn resolve_extension_dir(ledger_dir: &Path, extension_value: &str) -> PathBuf {
    if is_extension_path(extension_value) {
        PathBuf::from(extension_value)
    } else {
        ledger_dir.join("extensions").join(extension_value)
    }
}

/// Resolve the extension to use for an account.
///
/// Priority:
/// 1. Explicitly provided value (if non-empty)
/// 2. Account config `extension` field
/// 3. Error
pub fn resolve_extension(
    ledger_dir: &Path,
    account_name: &str,
    explicit: Option<&str>,
) -> Result<String, String> {
    if let Some(ext) = explicit {
        let trimmed = ext.trim();
        if !trimmed.is_empty() {
            return Ok(trimmed.to_string());
        }
    }

    let config = read_account_config(ledger_dir, account_name);
    if let Some(ext) = config.extension {
        let trimmed = ext.trim();
        if !trimmed.is_empty() {
            return Ok(trimmed.to_string());
        }
    }

    Err(format!(
        "no extension configured for account '{account_name}'. \
         Specify --extension or set it in accounts/{account_name}/config.json"
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn create_temp_dir(prefix: &str) -> PathBuf {
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
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
    fn read_missing_config_returns_defaults() {
        let dir = create_temp_dir("acfg-missing");
        let config = read_account_config(&dir, "nonexistent");
        assert!(config.extension.is_none());
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn write_and_read_config_roundtrips() {
        let dir = create_temp_dir("acfg-roundtrip");
        let config = AccountConfig {
            extension: Some("chase-driver".to_string()),
        };
        write_account_config(&dir, "chase", &config)
            .unwrap_or_else(|err| panic!("failed to write config: {err}"));
        let loaded = read_account_config(&dir, "chase");
        assert_eq!(loaded.extension.as_deref(), Some("chase-driver"));
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn resolve_extension_dir_name_vs_path() {
        let ledger = PathBuf::from("/ledger.refreshmint");
        assert_eq!(
            resolve_extension_dir(&ledger, "chase-driver"),
            PathBuf::from("/ledger.refreshmint/extensions/chase-driver")
        );
        assert_eq!(
            resolve_extension_dir(&ledger, "/Users/me/dev/chase-driver"),
            PathBuf::from("/Users/me/dev/chase-driver")
        );
        assert_eq!(
            resolve_extension_dir(&ledger, "./local-ext"),
            PathBuf::from("./local-ext")
        );
    }

    #[test]
    fn resolve_extension_prefers_explicit() {
        let dir = create_temp_dir("acfg-resolve");
        let config = AccountConfig {
            extension: Some("saved-ext".to_string()),
        };
        write_account_config(&dir, "acct", &config)
            .unwrap_or_else(|err| panic!("failed to write config: {err}"));

        let result = resolve_extension(&dir, "acct", Some("explicit-ext"));
        match result {
            Ok(ext) => assert_eq!(ext, "explicit-ext"),
            Err(err) => panic!("expected Ok, got error: {err}"),
        }
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn resolve_extension_falls_back_to_config() {
        let dir = create_temp_dir("acfg-fallback");
        let config = AccountConfig {
            extension: Some("saved-ext".to_string()),
        };
        write_account_config(&dir, "acct", &config)
            .unwrap_or_else(|err| panic!("failed to write config: {err}"));

        let result = resolve_extension(&dir, "acct", None);
        match result {
            Ok(ext) => assert_eq!(ext, "saved-ext"),
            Err(err) => panic!("expected Ok, got error: {err}"),
        }

        let result2 = resolve_extension(&dir, "acct", Some(""));
        match result2 {
            Ok(ext) => assert_eq!(ext, "saved-ext"),
            Err(err) => panic!("expected Ok, got error: {err}"),
        }
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn resolve_extension_errors_when_none_configured() {
        let dir = create_temp_dir("acfg-none");
        let result = resolve_extension(&dir, "acct", None);
        match result {
            Ok(ext) => panic!("expected Err, got Ok: {ext}"),
            Err(err) => assert!(
                err.contains("no extension configured"),
                "unexpected error: {err}"
            ),
        }
        let _ = fs::remove_dir_all(&dir);
    }
}
