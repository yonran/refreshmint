use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::io;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MigratedAccount {
    pub account_name: String,
    pub login_name: String,
    pub label: String,
}

#[derive(Debug, Clone, Default, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MigrationOutcome {
    pub dry_run: bool,
    pub migrated: Vec<MigratedAccount>,
    pub skipped: Vec<String>,
    pub warnings: Vec<String>,
}

pub fn migrate_ledger(
    ledger_dir: &Path,
    dry_run: bool,
) -> Result<MigrationOutcome, Box<dyn std::error::Error + Send + Sync>> {
    let accounts_dir = ledger_dir.join("accounts");
    let mut outcome = MigrationOutcome {
        dry_run,
        ..MigrationOutcome::default()
    };
    if !accounts_dir.exists() {
        return Ok(outcome);
    }

    let account_names = list_old_accounts(&accounts_dir)?;
    if account_names.is_empty() {
        return Ok(outcome);
    }

    let mut groups: BTreeMap<String, Vec<String>> = BTreeMap::new();
    for account_name in account_names {
        let config = crate::account_config::read_account_config(ledger_dir, &account_name);
        let extension = config
            .extension
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(ToOwned::to_owned);
        let Some(extension) = extension else {
            outcome.skipped.push(account_name.clone());
            outcome.warnings.push(format!(
                "skipping account '{account_name}': missing extension in accounts/{account_name}/config.json"
            ));
            continue;
        };
        groups.entry(extension).or_default().push(account_name);
    }

    for (extension, mut account_group) in groups {
        account_group.sort();
        let login_name = derive_login_name(&extension);
        if let Err(err) = crate::login_config::validate_label(&login_name) {
            outcome.warnings.push(format!(
                "skipping extension group '{extension}': derived login name '{login_name}' is invalid: {err}"
            ));
            outcome.skipped.extend(account_group.into_iter());
            continue;
        }

        let mut config = crate::login_config::read_login_config(ledger_dir, &login_name);
        if config
            .extension
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .is_none()
        {
            config.extension = Some(extension.clone());
        }

        let mut used_labels: BTreeSet<String> = config.accounts.keys().cloned().collect();
        let mut plans = Vec::new();
        for account_name in &account_group {
            let account_dir = accounts_dir.join(account_name);
            if !account_dir.exists() {
                continue;
            }

            let label = existing_or_new_label(&config, &mut used_labels, account_name);
            plans.push((account_name.clone(), label.clone()));
            config.accounts.insert(
                label,
                crate::login_config::LoginAccountConfig {
                    gl_account: Some(account_name.clone()),
                },
            );
        }

        if plans.is_empty() {
            continue;
        }

        for (account_name, label) in &plans {
            outcome.migrated.push(MigratedAccount {
                account_name: account_name.clone(),
                login_name: login_name.clone(),
                label: label.clone(),
            });
        }

        if dry_run {
            continue;
        }

        let _lock = crate::login_config::acquire_login_lock(ledger_dir, &login_name)?;
        crate::login_config::write_login_config(ledger_dir, &login_name, &config)?;

        if let Some((source_account, _)) = plans.first() {
            if let Err(err) = copy_login_secrets_from_account(source_account, &login_name) {
                outcome.warnings.push(format!(
                    "failed to copy secrets from account '{source_account}' to login '{login_name}': {err}"
                ));
            }
        }

        for (account_name, label) in &plans {
            migrate_account_dir(ledger_dir, account_name, &login_name, label, &mut outcome)?;
        }
    }

    if !dry_run {
        remove_dir_if_empty(&accounts_dir)?;
    }
    Ok(outcome)
}

fn list_old_accounts(accounts_dir: &Path) -> io::Result<Vec<String>> {
    let mut names = Vec::new();
    for entry in fs::read_dir(accounts_dir)? {
        let entry = entry?;
        if !entry.file_type()?.is_dir() {
            continue;
        }
        if let Some(name) = entry.file_name().to_str() {
            names.push(name.to_string());
        }
    }
    names.sort();
    Ok(names)
}

fn existing_or_new_label(
    config: &crate::login_config::LoginConfig,
    used_labels: &mut BTreeSet<String>,
    account_name: &str,
) -> String {
    if let Some(existing) = config.accounts.iter().find_map(|(label, account)| {
        if account.gl_account.as_deref() == Some(account_name) {
            Some(label.clone())
        } else {
            None
        }
    }) {
        used_labels.insert(existing.clone());
        return existing;
    }

    let base = derive_label(account_name);
    let mut candidate = base.clone();
    let mut idx = 2usize;
    while used_labels.contains(&candidate) {
        candidate = format!("{base}-{idx}");
        idx += 1;
    }
    used_labels.insert(candidate.clone());
    candidate
}

fn derive_login_name(extension: &str) -> String {
    let base = if extension.contains('/') || extension.contains('\\') || extension.starts_with('.')
    {
        Path::new(extension)
            .file_name()
            .and_then(std::ffi::OsStr::to_str)
            .unwrap_or(extension)
    } else {
        extension
    };
    sanitize_label(base, "login")
}

fn derive_label(account_name: &str) -> String {
    let leaf = account_name.rsplit(':').next().unwrap_or(account_name);
    sanitize_label(leaf, "account")
}

fn sanitize_label(input: &str, fallback: &str) -> String {
    let mut out = String::with_capacity(input.len());
    for ch in input.chars() {
        if ch.is_ascii_alphanumeric() || ch == '-' || ch == '_' || ch == '.' {
            out.push(ch.to_ascii_lowercase());
        } else {
            out.push('-');
        }
    }
    let mut out = out.trim_matches('-').trim().to_string();
    if out.is_empty() || out == "." || out == ".." {
        out = fallback.to_string();
    }
    if out.len() > 255 {
        out.truncate(255);
    }
    if out == "." || out == ".." {
        fallback.to_string()
    } else {
        out
    }
}

fn migrate_account_dir(
    ledger_dir: &Path,
    account_name: &str,
    login_name: &str,
    label: &str,
    outcome: &mut MigrationOutcome,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let source_dir = ledger_dir.join("accounts").join(account_name);
    if !source_dir.exists() {
        return Ok(());
    }

    let target_account_dir = ledger_dir
        .join("logins")
        .join(login_name)
        .join("accounts")
        .join(label);
    fs::create_dir_all(&target_account_dir)?;

    let source_documents_dir = source_dir.join("documents");
    let target_documents_dir = target_account_dir.join("documents");
    move_directory_contents(&source_documents_dir, &target_documents_dir)?;
    rewrite_document_sidecars(&target_documents_dir, login_name, label, outcome)?;

    let source_journal = source_dir.join("account.journal");
    let target_journal = target_account_dir.join("account.journal");
    move_file_if_exists(&source_journal, &target_journal)?;

    let source_operations = source_dir.join("operations.jsonl");
    let target_operations = target_account_dir.join("operations.jsonl");
    move_file_if_exists(&source_operations, &target_operations)?;

    if source_dir.exists() {
        fs::remove_dir_all(&source_dir)?;
    }
    Ok(())
}

fn move_directory_contents(source_dir: &Path, target_dir: &Path) -> io::Result<()> {
    if !source_dir.exists() {
        return Ok(());
    }
    fs::create_dir_all(target_dir)?;

    for entry in fs::read_dir(source_dir)? {
        let entry = entry?;
        let source_path = entry.path();
        let target_path = target_dir.join(entry.file_name());

        if entry.file_type()?.is_dir() {
            move_directory_contents(&source_path, &target_path)?;
            if source_path.exists() {
                let _ = fs::remove_dir_all(&source_path);
            }
            continue;
        }

        move_file_with_collision_handling(&source_path, &target_path)?;
    }

    remove_dir_if_empty(source_dir)?;
    Ok(())
}

fn move_file_if_exists(source: &Path, target: &Path) -> io::Result<()> {
    if !source.exists() {
        return Ok(());
    }
    if let Some(parent) = target.parent() {
        fs::create_dir_all(parent)?;
    }
    move_file_with_collision_handling(source, target)
}

fn move_file_with_collision_handling(source: &Path, target: &Path) -> io::Result<()> {
    let target = if target.exists() {
        if files_equal(source, target)? {
            fs::remove_file(source)?;
            return Ok(());
        }
        next_available_path(target)
    } else {
        target.to_path_buf()
    };
    move_file(source, &target)
}

fn move_file(source: &Path, target: &Path) -> io::Result<()> {
    if let Some(parent) = target.parent() {
        fs::create_dir_all(parent)?;
    }
    match fs::rename(source, target) {
        Ok(()) => Ok(()),
        Err(_) => {
            fs::copy(source, target)?;
            fs::remove_file(source)?;
            Ok(())
        }
    }
}

fn files_equal(a: &Path, b: &Path) -> io::Result<bool> {
    let a_meta = fs::metadata(a)?;
    let b_meta = fs::metadata(b)?;
    if a_meta.len() != b_meta.len() {
        return Ok(false);
    }
    Ok(fs::read(a)? == fs::read(b)?)
}

fn next_available_path(path: &Path) -> PathBuf {
    let parent = path.parent().unwrap_or_else(|| Path::new("."));
    let stem = path
        .file_stem()
        .and_then(std::ffi::OsStr::to_str)
        .unwrap_or("file");
    let ext = path.extension().and_then(std::ffi::OsStr::to_str);

    for i in 2..1000usize {
        let candidate_name = if let Some(ext) = ext {
            format!("{stem}-{i}.{ext}")
        } else {
            format!("{stem}-{i}")
        };
        let candidate = parent.join(candidate_name);
        if !candidate.exists() {
            return candidate;
        }
    }
    parent.join(format!("{stem}-{}", uuid::Uuid::new_v4()))
}

fn remove_dir_if_empty(path: &Path) -> io::Result<()> {
    if !path.exists() || !path.is_dir() {
        return Ok(());
    }
    let mut entries = fs::read_dir(path)?;
    if entries.next().is_none() {
        fs::remove_dir(path)?;
    }
    Ok(())
}

fn rewrite_document_sidecars(
    documents_dir: &Path,
    login_name: &str,
    label: &str,
    outcome: &mut MigrationOutcome,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    if !documents_dir.exists() {
        return Ok(());
    }

    for path in walk_files(documents_dir)? {
        let Some(file_name) = path.file_name().and_then(std::ffi::OsStr::to_str) else {
            continue;
        };
        if !file_name.ends_with("-info.json") {
            continue;
        }

        let raw = match fs::read_to_string(&path) {
            Ok(value) => value,
            Err(err) => {
                outcome
                    .warnings
                    .push(format!("failed to read sidecar {}: {err}", path.display()));
                continue;
            }
        };
        let mut value: serde_json::Value = match serde_json::from_str(&raw) {
            Ok(value) => value,
            Err(err) => {
                outcome
                    .warnings
                    .push(format!("failed to parse sidecar {}: {err}", path.display()));
                continue;
            }
        };
        let Some(obj) = value.as_object_mut() else {
            outcome.warnings.push(format!(
                "sidecar {} is not a JSON object; skipping",
                path.display()
            ));
            continue;
        };

        obj.insert(
            "loginName".to_string(),
            serde_json::Value::String(login_name.to_string()),
        );
        obj.insert(
            "label".to_string(),
            serde_json::Value::String(label.to_string()),
        );
        obj.remove("accountName");

        let content = serde_json::to_string_pretty(&value)?;
        fs::write(&path, content)?;
    }
    Ok(())
}

fn walk_files(root: &Path) -> io::Result<Vec<PathBuf>> {
    let mut files = Vec::new();
    let mut stack = vec![root.to_path_buf()];
    while let Some(dir) = stack.pop() {
        for entry in fs::read_dir(&dir)? {
            let entry = entry?;
            let path = entry.path();
            if entry.file_type()?.is_dir() {
                stack.push(path);
            } else if entry.file_type()?.is_file() {
                files.push(path);
            }
        }
    }
    Ok(files)
}

fn copy_login_secrets_from_account(account_name: &str, login_name: &str) -> Result<(), String> {
    let source = crate::secret::SecretStore::new(account_name.to_string());
    let target = crate::secret::SecretStore::new(format!("login/{login_name}"));

    let entries = source
        .list_with_value_state()
        .map_err(|err| err.to_string())?;
    for (domain, name, has_value) in entries {
        if has_value {
            let value = source.get(&domain, &name).map_err(|err| err.to_string())?;
            target
                .set(&domain, &name, &value)
                .map_err(|err| err.to_string())?;
        } else {
            target
                .ensure_indexed(&domain, &name)
                .map_err(|err| err.to_string())?;
        }
    }
    Ok(())
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;

    fn temp_dir(prefix: &str) -> PathBuf {
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let dir = std::env::temp_dir().join(format!(
            "refreshmint-migrate-{prefix}-{}-{now}",
            std::process::id()
        ));
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn derive_label_uses_last_account_segment() {
        assert_eq!(derive_label("Assets:Chase:Checking"), "checking");
        assert_eq!(derive_label("Liabilities:Joint:CC"), "cc");
    }

    #[test]
    fn migrate_moves_account_documents_and_updates_config() {
        let ledger_dir = temp_dir("moves");
        fs::create_dir_all(ledger_dir.join("accounts").join("Assets:Chase:Checking")).unwrap();
        crate::account_config::write_account_config(
            &ledger_dir,
            "Assets:Chase:Checking",
            &crate::account_config::AccountConfig {
                extension: Some("chase-driver".to_string()),
            },
        )
        .unwrap();

        let src_docs = ledger_dir
            .join("accounts")
            .join("Assets:Chase:Checking")
            .join("documents");
        fs::create_dir_all(&src_docs).unwrap();
        fs::write(src_docs.join("statement.pdf"), b"pdf").unwrap();
        fs::write(
            src_docs.join("statement.pdf-info.json"),
            r#"{"accountName":"Assets:Chase:Checking","mimeType":"application/pdf"}"#,
        )
        .unwrap();
        fs::write(
            ledger_dir
                .join("accounts")
                .join("Assets:Chase:Checking")
                .join("account.journal"),
            "2026-01-01 Test\n    Assets:Chase:Checking  1 USD\n    Equity:Test\n",
        )
        .unwrap();

        let outcome = migrate_ledger(&ledger_dir, false).unwrap();
        assert_eq!(outcome.migrated.len(), 1);
        assert_eq!(outcome.migrated[0].login_name, "chase-driver");
        assert_eq!(outcome.migrated[0].label, "checking");

        let login_config = crate::login_config::read_login_config(&ledger_dir, "chase-driver");
        assert_eq!(login_config.extension.as_deref(), Some("chase-driver"));
        assert_eq!(
            login_config.accounts["checking"].gl_account.as_deref(),
            Some("Assets:Chase:Checking")
        );

        let target_docs = ledger_dir
            .join("logins")
            .join("chase-driver")
            .join("accounts")
            .join("checking")
            .join("documents");
        assert!(target_docs.join("statement.pdf").exists());
        let sidecar = fs::read_to_string(target_docs.join("statement.pdf-info.json")).unwrap();
        assert!(sidecar.contains("\"loginName\": \"chase-driver\""));
        assert!(sidecar.contains("\"label\": \"checking\""));
        assert!(!ledger_dir
            .join("accounts")
            .join("Assets:Chase:Checking")
            .exists());

        let _ = fs::remove_dir_all(&ledger_dir);
    }

    #[test]
    fn migrate_dry_run_does_not_change_filesystem() {
        let ledger_dir = temp_dir("dry-run");
        fs::create_dir_all(ledger_dir.join("accounts").join("Assets:Chase:Savings")).unwrap();
        crate::account_config::write_account_config(
            &ledger_dir,
            "Assets:Chase:Savings",
            &crate::account_config::AccountConfig {
                extension: Some("chase-driver".to_string()),
            },
        )
        .unwrap();

        let outcome = migrate_ledger(&ledger_dir, true).unwrap();
        assert_eq!(outcome.migrated.len(), 1);
        assert!(ledger_dir
            .join("accounts")
            .join("Assets:Chase:Savings")
            .exists());
        assert!(!ledger_dir.join("logins").join("chase-driver").exists());

        let _ = fs::remove_dir_all(&ledger_dir);
    }
}
