pub const STAGING_PREFIX: &str = "Equity:Staging";
pub const LEGACY_STAGING_PREFIX: &str = "Equity:Unreconciled";

pub fn canonical_staging_account(suffix: &str) -> String {
    let trimmed = suffix.trim_matches(':').trim();
    if trimmed.is_empty() {
        STAGING_PREFIX.to_string()
    } else {
        format!("{STAGING_PREFIX}:{trimmed}")
    }
}

pub fn canonicalize_account_name(account: &str) -> String {
    if let Some(rest) = account.strip_prefix(LEGACY_STAGING_PREFIX) {
        format!("{STAGING_PREFIX}{rest}")
    } else {
        account.to_string()
    }
}

pub fn is_staging_account(account: &str) -> bool {
    account == STAGING_PREFIX
        || account == LEGACY_STAGING_PREFIX
        || account.starts_with(&format!("{STAGING_PREFIX}:"))
        || account.starts_with(&format!("{LEGACY_STAGING_PREFIX}:"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn canonicalize_legacy_staging_account_name() {
        assert_eq!(
            canonicalize_account_name("Equity:Unreconciled:bank:checking"),
            "Equity:Staging:bank:checking"
        );
    }

    #[test]
    fn staging_match_supports_both_prefixes() {
        assert!(is_staging_account("Equity:Staging:bank"));
        assert!(is_staging_account("Equity:Unreconciled:bank"));
        assert!(!is_staging_account("Equity:OpeningBalances"));
    }
}
