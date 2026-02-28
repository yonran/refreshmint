use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::fs::{self, OpenOptions};
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

/// Per-login-account configuration: maps a label to a GL account.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct LoginAccountConfig {
    #[serde(
        rename = "gl_account",
        default,
        skip_serializing_if = "Option::is_none"
    )]
    pub gl_account: Option<String>,
}

/// Per-login configuration stored in `logins/<login_name>/config.json`.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct LoginConfig {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub extension: Option<String>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub accounts: BTreeMap<String, LoginAccountConfig>,
}

/// Validate a label used as a sub-account directory name.
///
/// Allowed: alphanumeric, hyphens, underscores, dots.
/// Rejected: empty, `.`, `..`, colons, slashes, backslashes, length > 255.
pub fn validate_label(label: &str) -> Result<(), String> {
    if label.is_empty() {
        return Err("label must not be empty".to_string());
    }
    if label == "." || label == ".." {
        return Err(format!("label must not be '{label}'"));
    }
    if label.len() > 255 {
        return Err(format!(
            "label exceeds maximum length of 255 characters (got {})",
            label.len()
        ));
    }
    for ch in label.chars() {
        if !ch.is_alphanumeric() && ch != '-' && ch != '_' && ch != '.' {
            return Err(format!(
                "label contains invalid character '{ch}': only alphanumeric, hyphens, underscores, and dots are allowed"
            ));
        }
    }
    Ok(())
}

/// Return the path to `logins/<login_name>/config.json`.
pub fn login_config_path(ledger_dir: &Path, login_name: &str) -> PathBuf {
    ledger_dir
        .join("logins")
        .join(login_name)
        .join("config.json")
}

/// Return the path to `logins/<login_name>/accounts/<label>/documents/`.
pub fn login_account_documents_dir(ledger_dir: &Path, login_name: &str, label: &str) -> PathBuf {
    ledger_dir
        .join("logins")
        .join(login_name)
        .join("accounts")
        .join(label)
        .join("documents")
}

/// Return the path to `logins/<login_name>/accounts/<label>/account.journal`.
pub fn login_account_journal_path(ledger_dir: &Path, login_name: &str, label: &str) -> PathBuf {
    ledger_dir
        .join("logins")
        .join(login_name)
        .join("accounts")
        .join(label)
        .join("account.journal")
}

/// Return the path to the per-login lock file.
pub fn login_lock_path(ledger_dir: &Path, login_name: &str) -> PathBuf {
    ledger_dir.join("logins").join(login_name).join(".lock")
}

/// Return the path to the per-login lock metadata file.
pub fn login_lock_metadata_path(ledger_dir: &Path, login_name: &str) -> PathBuf {
    ledger_dir
        .join("logins")
        .join(login_name)
        .join(".lock.meta.json")
}

/// Return the path to the ledger-wide GL mutation lock file.
pub fn gl_lock_path(ledger_dir: &Path) -> PathBuf {
    ledger_dir.join(".gl.lock")
}

/// Return the path to the ledger-wide GL mutation lock metadata file.
pub fn gl_lock_metadata_path(ledger_dir: &Path) -> PathBuf {
    ledger_dir.join(".gl.lock.meta.json")
}

/// Read the login config, returning defaults if the file is missing.
pub fn read_login_config(ledger_dir: &Path, login_name: &str) -> LoginConfig {
    let path = login_config_path(ledger_dir, login_name);
    match std::fs::read_to_string(&path) {
        Ok(text) => serde_json::from_str(&text).unwrap_or_else(|e| {
            eprintln!("warning: failed to parse '{}': {e}", path.display());
            LoginConfig::default()
        }),
        Err(e) if e.kind() == io::ErrorKind::NotFound => LoginConfig::default(),
        Err(e) => {
            eprintln!("warning: failed to read '{}': {e}", path.display());
            LoginConfig::default()
        }
    }
}

/// Write the login config via temp-file + rename.
pub fn write_login_config(
    ledger_dir: &Path,
    login_name: &str,
    config: &LoginConfig,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let path = login_config_path(ledger_dir, login_name);
    let parent = path
        .parent()
        .ok_or_else(|| std::io::Error::other("config path has no parent"))?;
    std::fs::create_dir_all(parent)?;

    let json = serde_json::to_string_pretty(config)?;
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    let temp_path = parent.join(format!(".config.json.tmp-{}-{nanos}", std::process::id()));
    {
        let mut file = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&temp_path)?;
        file.write_all(json.as_bytes())?;
        file.sync_all()?;
    }
    if let Err(err) = replace_file(&temp_path, &path) {
        let _ = std::fs::remove_file(&temp_path);
        return Err(err.into());
    }
    if let Ok(dir) = std::fs::File::open(parent) {
        let _ = dir.sync_all();
    }
    Ok(())
}

/// Atomically replace a file via rename, with a Windows fallback.
fn replace_file(temp_path: &Path, path: &Path) -> std::io::Result<()> {
    match std::fs::rename(temp_path, path) {
        Ok(()) => Ok(()),
        Err(err) => {
            #[cfg(windows)]
            {
                if err.kind() == std::io::ErrorKind::AlreadyExists {
                    std::fs::remove_file(path)?;
                    return std::fs::rename(temp_path, path);
                }
            }
            Err(err)
        }
    }
}

/// List all login names by scanning the `logins/` directory.
pub fn list_logins(ledger_dir: &Path) -> io::Result<Vec<String>> {
    let logins_dir = ledger_dir.join("logins");
    let entries = match std::fs::read_dir(&logins_dir) {
        Ok(e) => e,
        Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(e) => return Err(e),
    };

    let mut names = Vec::new();
    for entry in entries {
        let Ok(entry) = entry else { continue };
        let path = entry.path();
        if !path.is_dir() {
            continue;
        }
        let Some(name) = entry.file_name().to_str().map(ToOwned::to_owned) else {
            continue;
        };
        // Only include entries that have a config.json (or are valid login dirs)
        names.push(name);
    }
    names.sort();
    Ok(names)
}

/// A conflict entry for GL account uniqueness violations.
#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GlAccountConflictEntry {
    pub login_name: String,
    pub label: String,
}

/// A GL account conflict: multiple login accounts map to the same GL account.
#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GlAccountConflict {
    pub gl_account: String,
    pub entries: Vec<GlAccountConflictEntry>,
}

/// Check that a GL account is not already mapped by another (login, label) pair.
///
/// `exclude_login` and `exclude_label` identify the entry being set, so it
/// doesn't conflict with itself.
pub fn check_gl_account_uniqueness(
    ledger_dir: &Path,
    exclude_login: &str,
    exclude_label: &str,
    gl_account: &str,
) -> Result<(), String> {
    let logins = list_logins(ledger_dir).map_err(|e| e.to_string())?;
    for login in &logins {
        let config = read_login_config(ledger_dir, login);
        for (label, acct_config) in &config.accounts {
            if login == exclude_login && label == exclude_label {
                continue;
            }
            if let Some(existing_gl) = &acct_config.gl_account {
                if existing_gl == gl_account {
                    return Err(format!(
                        "GL account '{gl_account}' is already mapped by login '{login}' label '{label}'"
                    ));
                }
            }
        }
    }
    Ok(())
}

/// Scan all login configs and return a list of GL account conflicts.
pub fn find_gl_account_conflicts(ledger_dir: &Path) -> Vec<GlAccountConflict> {
    let logins = list_logins(ledger_dir).unwrap_or_default();
    let mut gl_map: BTreeMap<String, Vec<GlAccountConflictEntry>> = BTreeMap::new();

    for login in &logins {
        let config = read_login_config(ledger_dir, login);
        for (label, acct_config) in &config.accounts {
            if let Some(gl_account) = &acct_config.gl_account {
                gl_map
                    .entry(gl_account.clone())
                    .or_default()
                    .push(GlAccountConflictEntry {
                        login_name: login.clone(),
                        label: label.clone(),
                    });
            }
        }
    }

    gl_map
        .into_iter()
        .filter(|(_, entries)| entries.len() > 1)
        .map(|(gl_account, entries)| GlAccountConflict {
            gl_account,
            entries,
        })
        .collect()
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum LockResource {
    Login { login_name: String },
    Gl,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct LockMetadata {
    pub version: u8,
    pub owner: String,
    pub purpose: String,
    pub started_at: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub pid: Option<u32>,
    pub resource: LockResource,
}

impl LockMetadata {
    fn new_login(login_name: &str, owner: &str, purpose: &str) -> Self {
        Self {
            version: 1,
            owner: owner.to_string(),
            purpose: purpose.to_string(),
            started_at: chrono::Utc::now().to_rfc3339(),
            pid: Some(std::process::id()),
            resource: LockResource::Login {
                login_name: login_name.to_string(),
            },
        }
    }

    fn new_gl(owner: &str, purpose: &str) -> Self {
        Self {
            version: 1,
            owner: owner.to_string(),
            purpose: purpose.to_string(),
            started_at: chrono::Utc::now().to_rfc3339(),
            pid: Some(std::process::id()),
            resource: LockResource::Gl,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct LockStatus {
    pub locked: bool,
    pub metadata: Option<LockMetadata>,
}

/// A per-login file lock guard. The lock is released when this is dropped.
#[derive(Debug)]
pub struct LoginLock {
    _file: std::fs::File,
    metadata_path: PathBuf,
}

impl Drop for LoginLock {
    fn drop(&mut self) {
        let _ = cleanup_stale_metadata(&self.metadata_path);
    }
}

/// A ledger-wide GL lock guard. The lock is released when this is dropped.
#[derive(Debug)]
pub struct LedgerGlLock {
    _file: std::fs::File,
    metadata_path: PathBuf,
}

impl Drop for LedgerGlLock {
    fn drop(&mut self) {
        let _ = cleanup_stale_metadata(&self.metadata_path);
    }
}

fn write_metadata_file(
    metadata_path: &Path,
    metadata: &LockMetadata,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let parent = metadata_path
        .parent()
        .ok_or_else(|| io::Error::other("metadata path has no parent"))?;
    fs::create_dir_all(parent)?;
    let json = serde_json::to_vec_pretty(metadata)?;
    fs::write(metadata_path, json)?;
    Ok(())
}

fn cleanup_stale_metadata(
    metadata_path: &Path,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    match fs::remove_file(metadata_path) {
        Ok(()) => Ok(()),
        Err(err) if err.kind() == io::ErrorKind::NotFound => Ok(()),
        Err(err) => Err(err.into()),
    }
}

pub fn read_lock_metadata(
    metadata_path: &Path,
) -> Result<Option<LockMetadata>, Box<dyn std::error::Error + Send + Sync>> {
    match fs::read_to_string(metadata_path) {
        Ok(text) => {
            let parsed = serde_json::from_str(&text)?;
            Ok(Some(parsed))
        }
        Err(err) if err.kind() == io::ErrorKind::NotFound => Ok(None),
        Err(err) => Err(err.into()),
    }
}

fn read_lock_metadata_best_effort(metadata_path: &Path) -> Option<LockMetadata> {
    match fs::read_to_string(metadata_path) {
        Ok(text) => serde_json::from_str(&text).ok(),
        Err(_) => None,
    }
}

fn acquire_lock_file(
    lock_path: &Path,
) -> Result<std::fs::File, Box<dyn std::error::Error + Send + Sync>> {
    use fs2::FileExt;

    if let Some(parent) = lock_path.parent() {
        fs::create_dir_all(parent)?;
    }

    let file = OpenOptions::new()
        .create(true)
        .write(true)
        .truncate(false)
        .open(lock_path)?;

    file.try_lock_exclusive()?;

    Ok(file)
}

/// Acquire an exclusive file lock on `logins/<login_name>/.lock`.
///
/// Returns a guard that removes metadata before releasing the real lock.
pub fn acquire_login_lock_with_metadata(
    ledger_dir: &Path,
    login_name: &str,
    owner: &str,
    purpose: &str,
) -> Result<LoginLock, Box<dyn std::error::Error + Send + Sync>> {
    let lock_path = login_lock_path(ledger_dir, login_name);
    let metadata_path = login_lock_metadata_path(ledger_dir, login_name);
    let file = acquire_lock_file(&lock_path)
        .map_err(|_| format!("login '{login_name}' is currently in use by another operation"))?;
    cleanup_stale_metadata(&metadata_path)?;
    write_metadata_file(
        &metadata_path,
        &LockMetadata::new_login(login_name, owner, purpose),
    )?;

    Ok(LoginLock {
        _file: file,
        metadata_path,
    })
}

/// Acquire a metadata-aware per-login lock with generic metadata.
pub fn acquire_login_lock(
    ledger_dir: &Path,
    login_name: &str,
) -> Result<LoginLock, Box<dyn std::error::Error + Send + Sync>> {
    acquire_login_lock_with_metadata(ledger_dir, login_name, "unknown", "unspecified")
}

/// Acquire the ledger-wide GL mutation lock.
pub fn acquire_gl_lock_with_metadata(
    ledger_dir: &Path,
    owner: &str,
    purpose: &str,
) -> Result<LedgerGlLock, Box<dyn std::error::Error + Send + Sync>> {
    let lock_path = gl_lock_path(ledger_dir);
    let metadata_path = gl_lock_metadata_path(ledger_dir);
    let file = acquire_lock_file(&lock_path)
        .map_err(|_| "general journal is currently in use by another operation")?;
    cleanup_stale_metadata(&metadata_path)?;
    write_metadata_file(&metadata_path, &LockMetadata::new_gl(owner, purpose))?;

    Ok(LedgerGlLock {
        _file: file,
        metadata_path,
    })
}

fn probe_lock_status(
    lock_path: &Path,
    metadata_path: &Path,
) -> Result<LockStatus, Box<dyn std::error::Error + Send + Sync>> {
    match acquire_lock_file(lock_path) {
        Ok(_file) => {
            cleanup_stale_metadata(metadata_path)?;
            Ok(LockStatus {
                locked: false,
                metadata: None,
            })
        }
        Err(_) => Ok(LockStatus {
            locked: true,
            metadata: read_lock_metadata_best_effort(metadata_path),
        }),
    }
}

pub fn get_login_lock_status(
    ledger_dir: &Path,
    login_name: &str,
) -> Result<LockStatus, Box<dyn std::error::Error + Send + Sync>> {
    probe_lock_status(
        &login_lock_path(ledger_dir, login_name),
        &login_lock_metadata_path(ledger_dir, login_name),
    )
}

pub fn get_gl_lock_status(
    ledger_dir: &Path,
) -> Result<LockStatus, Box<dyn std::error::Error + Send + Sync>> {
    probe_lock_status(
        &gl_lock_path(ledger_dir),
        &gl_lock_metadata_path(ledger_dir),
    )
}

/// Resolve the extension to use for a login from the login config.
///
/// Returns an error if no extension is configured.
pub fn resolve_login_extension(ledger_dir: &Path, login_name: &str) -> Result<String, String> {
    let config = read_login_config(ledger_dir, login_name);
    if let Some(ext) = config.extension {
        let trimmed = ext.trim();
        if !trimmed.is_empty() {
            return Ok(trimmed.to_string());
        }
    }

    Err(format!(
        "no extension configured for login '{login_name}'. \
         Set it in logins/{login_name}/config.json"
    ))
}

#[cfg(test)]
#[allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::items_after_test_module
)]
mod tests {
    use super::*;
    use std::fs;

    fn create_temp_dir(prefix: &str) -> PathBuf {
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let dir =
            std::env::temp_dir().join(format!("refreshmint-{prefix}-{}-{now}", std::process::id()));
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn validate_label_accepts_valid_names() {
        assert!(validate_label("checking").is_ok());
        assert!(validate_label("my-account").is_ok());
        assert!(validate_label("savings_2").is_ok());
        assert!(validate_label("cc.main").is_ok());
        assert!(validate_label("a").is_ok());
    }

    #[test]
    fn validate_label_rejects_empty() {
        assert!(validate_label("").is_err());
    }

    #[test]
    fn validate_label_rejects_dot_and_dotdot() {
        assert!(validate_label(".").is_err());
        assert!(validate_label("..").is_err());
    }

    #[test]
    fn validate_label_rejects_colons() {
        let err = validate_label("Assets:Checking").unwrap_err();
        assert!(err.contains("invalid character ':'"));
    }

    #[test]
    fn validate_label_rejects_slashes() {
        assert!(validate_label("a/b").is_err());
        assert!(validate_label("a\\b").is_err());
    }

    #[test]
    fn validate_label_rejects_spaces() {
        assert!(validate_label("my account").is_err());
    }

    #[test]
    fn validate_label_rejects_long_names() {
        let long = "a".repeat(256);
        assert!(validate_label(&long).is_err());
        // 255 is OK
        let ok = "a".repeat(255);
        assert!(validate_label(&ok).is_ok());
    }

    #[test]
    fn read_missing_config_returns_defaults() {
        let dir = create_temp_dir("login-cfg-missing");
        let config = read_login_config(&dir, "nonexistent");
        assert!(config.extension.is_none());
        assert!(config.accounts.is_empty());
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn write_and_read_config_roundtrips() {
        let dir = create_temp_dir("login-cfg-roundtrip");
        let mut accounts = BTreeMap::new();
        accounts.insert(
            "checking".to_string(),
            LoginAccountConfig {
                gl_account: Some("Assets:Chase:Checking".to_string()),
            },
        );
        accounts.insert("cc".to_string(), LoginAccountConfig { gl_account: None });
        let config = LoginConfig {
            extension: Some("chase-driver".to_string()),
            accounts,
        };
        write_login_config(&dir, "chase-personal", &config).unwrap();
        let loaded = read_login_config(&dir, "chase-personal");
        assert_eq!(loaded.extension.as_deref(), Some("chase-driver"));
        assert_eq!(loaded.accounts.len(), 2);
        assert_eq!(
            loaded.accounts["checking"].gl_account.as_deref(),
            Some("Assets:Chase:Checking")
        );
        assert!(loaded.accounts["cc"].gl_account.is_none());
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn list_logins_scans_directory() {
        let dir = create_temp_dir("login-list");
        fs::create_dir_all(dir.join("logins").join("chase")).unwrap();
        fs::create_dir_all(dir.join("logins").join("amex")).unwrap();
        // Create a file (should be ignored)
        fs::write(dir.join("logins").join("not-a-dir"), "").unwrap();

        let logins = list_logins(&dir).unwrap();
        assert_eq!(logins, vec!["amex".to_string(), "chase".to_string()]);
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn list_logins_returns_empty_when_no_logins_dir() {
        let dir = create_temp_dir("login-list-empty");
        let logins = list_logins(&dir).unwrap();
        assert!(logins.is_empty());
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn gl_account_uniqueness_rejects_duplicate() {
        let dir = create_temp_dir("login-gl-unique");
        let config = LoginConfig {
            extension: Some("chase-driver".to_string()),
            accounts: {
                let mut m = BTreeMap::new();
                m.insert(
                    "checking".to_string(),
                    LoginAccountConfig {
                        gl_account: Some("Assets:Chase:Checking".to_string()),
                    },
                );
                m
            },
        };
        write_login_config(&dir, "chase", &config).unwrap();

        let err = check_gl_account_uniqueness(&dir, "amex", "checking", "Assets:Chase:Checking")
            .unwrap_err();
        assert!(err.contains("already mapped"));
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn gl_account_uniqueness_allows_self_update() {
        let dir = create_temp_dir("login-gl-self");
        let config = LoginConfig {
            extension: Some("chase-driver".to_string()),
            accounts: {
                let mut m = BTreeMap::new();
                m.insert(
                    "checking".to_string(),
                    LoginAccountConfig {
                        gl_account: Some("Assets:Chase:Checking".to_string()),
                    },
                );
                m
            },
        };
        write_login_config(&dir, "chase", &config).unwrap();

        // Setting the same GL account on the same (login, label) should be OK
        assert!(
            check_gl_account_uniqueness(&dir, "chase", "checking", "Assets:Chase:Checking",)
                .is_ok()
        );
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn gl_account_uniqueness_allows_null() {
        let dir = create_temp_dir("login-gl-null");
        let config = LoginConfig {
            extension: Some("chase-driver".to_string()),
            accounts: {
                let mut m = BTreeMap::new();
                m.insert("cc".to_string(), LoginAccountConfig { gl_account: None });
                m
            },
        };
        write_login_config(&dir, "chase", &config).unwrap();

        // null gl_account entries don't conflict
        assert!(check_gl_account_uniqueness(&dir, "amex", "cc", "Assets:Amex:CC",).is_ok());
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn find_gl_account_conflicts_detects_duplicates() {
        let dir = create_temp_dir("login-gl-conflicts");
        let config1 = LoginConfig {
            extension: Some("chase-driver".to_string()),
            accounts: {
                let mut m = BTreeMap::new();
                m.insert(
                    "checking".to_string(),
                    LoginAccountConfig {
                        gl_account: Some("Assets:Checking".to_string()),
                    },
                );
                m
            },
        };
        let config2 = LoginConfig {
            extension: Some("other-driver".to_string()),
            accounts: {
                let mut m = BTreeMap::new();
                m.insert(
                    "main".to_string(),
                    LoginAccountConfig {
                        gl_account: Some("Assets:Checking".to_string()),
                    },
                );
                m
            },
        };
        write_login_config(&dir, "chase", &config1).unwrap();
        write_login_config(&dir, "other", &config2).unwrap();

        let conflicts = find_gl_account_conflicts(&dir);
        assert_eq!(conflicts.len(), 1);
        assert_eq!(conflicts[0].gl_account, "Assets:Checking");
        assert_eq!(conflicts[0].entries.len(), 2);
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn acquire_login_lock_succeeds() {
        let dir = create_temp_dir("login-lock");
        let lock = acquire_login_lock(&dir, "chase");
        assert!(lock.is_ok());
        drop(lock);
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn acquire_login_lock_fails_when_held() {
        let dir = create_temp_dir("login-lock-held");
        let _lock1 = acquire_login_lock(&dir, "chase").unwrap();
        let lock2 = acquire_login_lock(&dir, "chase");
        assert!(lock2.is_err());
        let err = lock2.unwrap_err().to_string();
        assert!(err.contains("currently in use"));
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn login_lock_writes_and_removes_metadata() {
        let dir = create_temp_dir("login-lock-meta");
        let metadata_path = login_lock_metadata_path(&dir, "chase");
        {
            let _lock =
                acquire_login_lock_with_metadata(&dir, "chase", "gui", "pipeline-extract-all")
                    .unwrap();
            let metadata = read_lock_metadata(&metadata_path).unwrap().unwrap();
            assert_eq!(metadata.owner, "gui");
            assert_eq!(metadata.purpose, "pipeline-extract-all");
            assert_eq!(
                metadata.resource,
                LockResource::Login {
                    login_name: "chase".to_string()
                }
            );
        }
        assert!(!metadata_path.exists());
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn acquire_login_lock_cleans_stale_metadata() {
        let dir = create_temp_dir("login-lock-stale-meta");
        let metadata_path = login_lock_metadata_path(&dir, "chase");
        fs::create_dir_all(metadata_path.parent().unwrap()).unwrap();
        fs::write(
            &metadata_path,
            serde_json::to_vec_pretty(&LockMetadata {
                version: 1,
                owner: "stale".to_string(),
                purpose: "old".to_string(),
                started_at: "2026-01-01T00:00:00Z".to_string(),
                pid: None,
                resource: LockResource::Login {
                    login_name: "chase".to_string(),
                },
            })
            .unwrap(),
        )
        .unwrap();

        {
            let _lock = acquire_login_lock_with_metadata(&dir, "chase", "cli", "set-login-account")
                .unwrap();
            let metadata = read_lock_metadata(&metadata_path).unwrap().unwrap();
            assert_eq!(metadata.owner, "cli");
            assert_eq!(metadata.purpose, "set-login-account");
        }
        assert!(!metadata_path.exists());
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn gl_lock_writes_and_removes_metadata() {
        let dir = create_temp_dir("gl-lock-meta");
        let metadata_path = gl_lock_metadata_path(&dir);
        {
            let _lock = acquire_gl_lock_with_metadata(&dir, "gui", "pipeline-post-all").unwrap();
            let metadata = read_lock_metadata(&metadata_path).unwrap().unwrap();
            assert_eq!(metadata.owner, "gui");
            assert_eq!(metadata.purpose, "pipeline-post-all");
            assert_eq!(metadata.resource, LockResource::Gl);
        }
        assert!(!metadata_path.exists());
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn acquire_gl_lock_fails_when_held() {
        let dir = create_temp_dir("gl-lock-held");
        let _lock1 = acquire_gl_lock_with_metadata(&dir, "gui", "pipeline-post-all").unwrap();
        let lock2 = acquire_gl_lock_with_metadata(&dir, "cli", "recategorize-gl");
        assert!(lock2.is_err());
        assert!(lock2.unwrap_err().to_string().contains("general journal"));
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn login_account_documents_dir_path() {
        let ledger = PathBuf::from("/ledger.refreshmint");
        assert_eq!(
            login_account_documents_dir(&ledger, "chase", "checking"),
            PathBuf::from("/ledger.refreshmint/logins/chase/accounts/checking/documents")
        );
    }

    #[test]
    fn login_account_journal_path_test() {
        let ledger = PathBuf::from("/ledger.refreshmint");
        assert_eq!(
            login_account_journal_path(&ledger, "chase", "checking"),
            PathBuf::from("/ledger.refreshmint/logins/chase/accounts/checking/account.journal")
        );
    }

    #[test]
    fn resolve_login_extension_reads_from_config() {
        let dir = create_temp_dir("login-ext-resolve");
        let config = LoginConfig {
            extension: Some("saved-ext".to_string()),
            accounts: BTreeMap::new(),
        };
        write_login_config(&dir, "chase", &config).unwrap();

        let result = resolve_login_extension(&dir, "chase");
        assert_eq!(result.unwrap(), "saved-ext");
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn resolve_login_extension_errors_when_none_configured() {
        let dir = create_temp_dir("login-ext-none");
        let result = resolve_login_extension(&dir, "chase");
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("no extension configured"));
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn delete_login_refuses_with_documents() {
        let dir = create_temp_dir("login-delete-docs");
        let docs_dir = login_account_documents_dir(&dir, "chase", "checking");
        fs::create_dir_all(&docs_dir).unwrap();
        fs::write(docs_dir.join("statement.pdf"), b"pdf").unwrap();

        let result = delete_login(&dir, "chase");
        assert!(result.is_err());
        assert!(result
            .unwrap_err()
            .to_string()
            .contains("has documents or journal data"));
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn delete_login_succeeds_when_empty() {
        let dir = create_temp_dir("login-delete-ok");
        let config = LoginConfig {
            extension: Some("chase-driver".to_string()),
            accounts: BTreeMap::new(),
        };
        write_login_config(&dir, "chase", &config).unwrap();

        let result = delete_login(&dir, "chase");
        assert!(result.is_ok());
        assert!(!dir.join("logins").join("chase").exists());
        let _ = fs::remove_dir_all(&dir);
    }
}

/// Delete a login directory. Refuses if any sub-account has documents or journal data.
pub fn delete_login(
    ledger_dir: &Path,
    login_name: &str,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let login_dir = ledger_dir.join("logins").join(login_name);
    if !login_dir.exists() {
        return Err(format!("login '{login_name}' does not exist").into());
    }

    // Check if any sub-account has data
    let accounts_dir = login_dir.join("accounts");
    if accounts_dir.exists() {
        let entries = std::fs::read_dir(&accounts_dir).map_err(|e| {
            format!(
                "failed to read accounts directory '{}': {e}",
                accounts_dir.display()
            )
        })?;
        for entry in entries {
            let Ok(entry) = entry else { continue };
            let path = entry.path();
            if !path.is_dir() {
                continue;
            }
            let label = entry.file_name().to_string_lossy().to_string();

            // Check for documents
            let docs_dir = path.join("documents");
            if docs_dir.exists()
                && has_files(&docs_dir).map_err(|e| {
                    format!("failed to check documents in '{}': {e}", docs_dir.display())
                })?
            {
                return Err(format!(
                    "login '{login_name}' label '{label}' has documents or journal data; \
                     remove data before deleting login"
                )
                .into());
            }

            // Check for journal
            let journal = path.join("account.journal");
            if journal.exists() {
                let content = std::fs::read_to_string(&journal)
                    .map_err(|e| format!("failed to read journal '{}': {e}", journal.display()))?;
                if !content.trim().is_empty() {
                    return Err(format!(
                        "login '{login_name}' label '{label}' has documents or journal data; \
                         remove data before deleting login"
                    )
                    .into());
                }
            }
        }
    }

    std::fs::remove_dir_all(&login_dir)?;
    Ok(())
}

/// Check if a directory contains any files (not recursively deep, just immediate).
fn has_files(dir: &Path) -> io::Result<bool> {
    let entries = match std::fs::read_dir(dir) {
        Ok(e) => e,
        Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(false),
        Err(e) => return Err(e),
    };
    for entry in entries {
        let Ok(entry) = entry else { continue };
        if entry.path().is_file() {
            return Ok(true);
        }
    }
    Ok(false)
}

/// Remove a login account (label). Refuses if the sub-account dir has documents or journal data.
pub fn remove_login_account(
    ledger_dir: &Path,
    login_name: &str,
    label: &str,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let config_path = login_config_path(ledger_dir, login_name);
    if !config_path.exists() {
        return Err(format!("login '{login_name}' does not exist").into());
    }

    let config = read_login_config(ledger_dir, login_name);
    if !config.accounts.contains_key(label) {
        return Err(format!("label '{label}' not found in login '{login_name}'").into());
    }

    // Check for data
    let account_dir = ledger_dir
        .join("logins")
        .join(login_name)
        .join("accounts")
        .join(label);
    if account_dir.exists() {
        let docs_dir = account_dir.join("documents");
        if docs_dir.exists()
            && has_files(&docs_dir).map_err(|e| {
                format!("failed to check documents in '{}': {e}", docs_dir.display())
            })?
        {
            return Err(format!(
                "login '{login_name}' label '{label}' has documents; remove data before removing account"
            )
            .into());
        }
        let journal = account_dir.join("account.journal");
        if journal.exists() {
            let content = std::fs::read_to_string(&journal)
                .map_err(|e| format!("failed to read journal '{}': {e}", journal.display()))?;
            if !content.trim().is_empty() {
                return Err(format!(
                    "login '{login_name}' label '{label}' has journal data; remove data before removing account"
                )
                .into());
            }
        }
    }

    // Remove from config
    let mut updated = config;
    updated.accounts.remove(label);
    write_login_config(ledger_dir, login_name, &updated)?;

    // Remove the directory if it exists
    if account_dir.exists() {
        let _ = std::fs::remove_dir_all(&account_dir);
    }

    Ok(())
}
