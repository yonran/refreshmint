use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};
use std::path::PathBuf;

/// Resolve the browser profile directory for a given account.
///
/// Default base: `dirs::data_dir()/refreshmint/Default/account-profiles/`
/// Per-account dir: `<ledger-path-hash>/<sanitized-account>/`
///
/// If `profile_override` is provided, it replaces the base directory.
pub fn resolve_profile_dir(
    ledger_path: &std::path::Path,
    account: &str,
    profile_override: Option<&std::path::Path>,
) -> Result<PathBuf, Box<dyn std::error::Error>> {
    let base = match profile_override {
        Some(p) => p.to_path_buf(),
        None => {
            let data_dir = dirs::data_dir().ok_or("could not determine data directory")?;
            data_dir
                .join("refreshmint")
                .join("Default")
                .join("account-profiles")
        }
    };

    let ledger_hash = hash_path(ledger_path);
    let sanitized = sanitize_account_name(account);

    Ok(base.join(ledger_hash).join(sanitized))
}

/// Delete the browser profile directory for a given login.
pub fn clear_login_profile(
    ledger_path: &std::path::Path,
    login_name: &str,
    _lock: &crate::login_config::LoginLock,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let profile_dir = resolve_profile_dir(ledger_path, login_name, None)
        .map_err(|e| -> Box<dyn std::error::Error + Send + Sync> { e.to_string().into() })?;
    if profile_dir.exists() {
        std::fs::remove_dir_all(&profile_dir)?;
    }
    Ok(())
}

/// Resolve the download directory for a scrape run.
///
/// `<base>/downloads/<extname>-<timestamp>/`
pub fn resolve_download_dir(
    extension_name: &str,
    profile_override: Option<&std::path::Path>,
) -> Result<PathBuf, Box<dyn std::error::Error>> {
    let base = match profile_override {
        Some(p) => p.to_path_buf(),
        None => {
            let data_dir = dirs::data_dir().ok_or("could not determine data directory")?;
            data_dir.join("refreshmint").join("Default")
        }
    };

    let timestamp = chrono_like_timestamp();
    let dir_stem = sanitize_extension_label(extension_name);
    let dir_name = format!("{dir_stem}-{timestamp}");
    Ok(base.join("downloads").join(dir_name))
}

fn hash_path(path: &std::path::Path) -> String {
    let mut hasher = DefaultHasher::new();
    path.hash(&mut hasher);
    format!("{:016x}", hasher.finish())
}

fn sanitize_account_name(account: &str) -> String {
    account
        .chars()
        .map(|c| {
            if c.is_alphanumeric() || c == '-' || c == '_' {
                c
            } else {
                '_'
            }
        })
        .collect()
}

fn sanitize_extension_label(extension_name: &str) -> String {
    let tail = extension_name
        .trim_end_matches(['/', '\\'])
        .rsplit(['/', '\\'])
        .find(|segment| !segment.is_empty())
        .unwrap_or(extension_name);
    let sanitized: String = tail
        .chars()
        .map(|c| {
            if c.is_alphanumeric() || c == '-' || c == '_' {
                c
            } else {
                '_'
            }
        })
        .collect();
    if sanitized.is_empty() {
        "extension".to_string()
    } else {
        sanitized
    }
}

/// Generate a timestamp string like `20260215T120000` without pulling in chrono.
fn chrono_like_timestamp() -> String {
    use std::time::SystemTime;

    let now = SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .unwrap_or_default();
    let secs = now.as_secs();

    // Simple UTC breakdown
    let days = secs / 86400;
    let time_of_day = secs % 86400;
    let hours = time_of_day / 3600;
    let minutes = (time_of_day % 3600) / 60;
    let seconds = time_of_day % 60;

    // Days since 1970-01-01 to Y-M-D (simplified)
    let (year, month, day) = days_to_ymd(days);

    format!("{year:04}{month:02}{day:02}T{hours:02}{minutes:02}{seconds:02}")
}

fn days_to_ymd(mut days: u64) -> (u64, u64, u64) {
    let mut year = 1970u64;
    loop {
        let days_in_year = if is_leap(year) { 366 } else { 365 };
        if days < days_in_year {
            break;
        }
        days -= days_in_year;
        year += 1;
    }
    let leap = is_leap(year);
    let month_days: [u64; 12] = [
        31,
        if leap { 29 } else { 28 },
        31,
        30,
        31,
        30,
        31,
        31,
        30,
        31,
        30,
        31,
    ];
    let mut month = 1u64;
    for &md in &month_days {
        if days < md {
            break;
        }
        days -= md;
        month += 1;
    }
    (year, month, days + 1)
}

fn is_leap(y: u64) -> bool {
    (y % 4 == 0 && y % 100 != 0) || y % 400 == 0
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_sanitize_account_name() {
        assert_eq!(sanitize_account_name("my-account"), "my-account");
        assert_eq!(sanitize_account_name("my account!"), "my_account_");
        assert_eq!(sanitize_account_name("a/b:c"), "a_b_c");
    }

    #[test]
    fn test_hash_path_deterministic() {
        let p = std::path::Path::new("/some/path");
        assert_eq!(hash_path(p), hash_path(p));
    }

    #[test]
    fn test_resolve_profile_dir() {
        let dir = resolve_profile_dir(
            std::path::Path::new("/ledger"),
            "chase",
            Some(std::path::Path::new("/tmp/profiles")),
        );
        let dir = match dir {
            Ok(d) => d,
            Err(e) => panic!("unexpected error: {e}"),
        };
        assert!(dir.starts_with("/tmp/profiles"));
        assert!(dir.ends_with("chase"));
    }

    #[test]
    fn test_resolve_download_dir_with_absolute_extension_path_stays_under_base() {
        let base = std::path::Path::new("/tmp/profiles");
        let dir = resolve_download_dir("/Users/me/ext-driver", Some(base));
        let dir = match dir {
            Ok(d) => d,
            Err(e) => panic!("unexpected error: {e}"),
        };
        assert!(dir.starts_with(base.join("downloads")));
        let file_name = dir
            .file_name()
            .and_then(|s| s.to_str())
            .unwrap_or_default()
            .to_string();
        assert!(file_name.starts_with("ext-driver-"));
    }

    #[test]
    fn test_resolve_download_dir_with_windows_style_path_uses_leaf_name() {
        let base = std::path::Path::new("/tmp/profiles");
        let dir = resolve_download_dir(r"C:\Users\me\ext-driver", Some(base));
        let dir = match dir {
            Ok(d) => d,
            Err(e) => panic!("unexpected error: {e}"),
        };
        assert!(dir.starts_with(base.join("downloads")));
        let file_name = dir
            .file_name()
            .and_then(|s| s.to_str())
            .unwrap_or_default()
            .to_string();
        assert!(file_name.starts_with("ext-driver-"));
    }
}
