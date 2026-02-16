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
    let dir_name = format!("{extension_name}-{timestamp}");
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
}
