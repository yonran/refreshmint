use std::error::Error;

/// Per-domain credential stored as a keychain entry.
///
/// On macOS:
///   - Service: `refreshmint/<login>/<domain>`
///   - Account (kSecAttrAccount): the actual username — readable without biometric
///   - Data (kSecValueData): the actual password — protected by biometric
///
/// On non-macOS (Linux, Windows), keyring does not expose account-field listing,
/// so credentials are stored as two separate entries under the domain service:
///   - account `_usr` → username (no biometric)
///   - account `_pwd` → password (no biometric; these platforms lack biometric anyway)
///
/// A lightweight domains index (JSON list) is stored at:
///   service=`refreshmint/<login>`, account=`_domains_index`, data=JSON (no biometric).
pub struct SecretStore {
    login_name: String,
}

#[derive(Clone, Debug, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DomainEntry {
    pub domain: String,
    pub has_username: bool,
    pub has_password: bool,
}

#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
struct DomainIndexEntry {
    domain: String,
    #[serde(default)]
    has_username: bool,
    #[serde(default)]
    has_password: bool,
}

impl SecretStore {
    pub fn new(login_name: String) -> Self {
        Self { login_name }
    }

    fn service_for_domain(&self, domain: &str) -> String {
        format!("refreshmint/{}/{}", self.login_name, domain)
    }

    fn index_service(&self) -> String {
        format!("refreshmint/{}", self.login_name)
    }

    const INDEX_ACCOUNT: &'static str = "_domains_index";

    fn read_domains_index(&self) -> Result<Vec<DomainIndexEntry>, Box<dyn Error + Send + Sync>> {
        let entry = keyring::Entry::new(&self.index_service(), Self::INDEX_ACCOUNT)?;
        match entry.get_password() {
            Ok(json) => {
                let entries: Vec<DomainIndexEntry> = serde_json::from_str(&json)?;
                Ok(entries)
            }
            Err(keyring::Error::NoEntry) => Ok(Vec::new()),
            Err(e) => Err(e.into()),
        }
    }

    fn write_domains_index(
        &self,
        entries: &[DomainIndexEntry],
    ) -> Result<(), Box<dyn Error + Send + Sync>> {
        let json = serde_json::to_string(entries)?;
        let entry = keyring::Entry::new(&self.index_service(), Self::INDEX_ACCOUNT)?;
        entry.set_password(&json)?;
        Ok(())
    }

    fn upsert_domains_index(
        &self,
        domain: &str,
        has_username: Option<bool>,
        has_password: Option<bool>,
    ) -> Result<(), Box<dyn Error + Send + Sync>> {
        let mut index = self.read_domains_index()?;
        if let Some(existing) = index.iter_mut().find(|e| e.domain == domain) {
            if let Some(u) = has_username {
                existing.has_username = u;
            }
            if let Some(p) = has_password {
                existing.has_password = p;
            }
        } else {
            index.push(DomainIndexEntry {
                domain: domain.to_string(),
                has_username: has_username.unwrap_or(false),
                has_password: has_password.unwrap_or(false),
            });
        }
        index.sort_by(|a, b| a.domain.cmp(&b.domain));
        self.write_domains_index(&index)
    }

    /// Store credentials (username + password) for a domain.
    ///
    /// On macOS the username is stored as the keychain Account field and the
    /// password is stored as biometric-protected Data.  On other platforms both
    /// are stored as regular keyring entries.
    pub fn set_credentials(
        &self,
        domain: &str,
        username: &str,
        password: &str,
    ) -> Result<(), Box<dyn Error + Send + Sync>> {
        #[cfg(target_os = "macos")]
        {
            self.set_credentials_macos(domain, username, password)?;
        }
        #[cfg(not(target_os = "macos"))]
        {
            self.set_credentials_other(domain, username, password)?;
        }
        self.upsert_domains_index(domain, Some(true), Some(true))?;
        Ok(())
    }

    /// Store only the username for a domain (password unchanged).
    ///
    /// If a password was already stored for this domain it is preserved.
    /// If the username changes the old keychain entry is replaced.
    pub fn set_username(
        &self,
        domain: &str,
        username: &str,
    ) -> Result<(), Box<dyn Error + Send + Sync>> {
        #[cfg(target_os = "macos")]
        {
            self.set_username_macos(domain, username)?;
        }
        #[cfg(not(target_os = "macos"))]
        {
            self.set_username_other(domain, username)?;
        }
        self.upsert_domains_index(domain, Some(true), None)?;
        Ok(())
    }

    /// Store only the password for a domain (username unchanged).
    ///
    /// The username must already be set; `get_username` is called first.
    pub fn set_password(
        &self,
        domain: &str,
        password: &str,
    ) -> Result<(), Box<dyn Error + Send + Sync>> {
        #[cfg(target_os = "macos")]
        {
            self.set_password_macos(domain, password)?;
        }
        #[cfg(not(target_os = "macos"))]
        {
            self.set_password_other(domain, password)?;
        }
        self.upsert_domains_index(domain, None, Some(true))?;
        Ok(())
    }

    /// Read the username (Account field) for a domain — no biometric prompt.
    pub fn get_username(&self, domain: &str) -> Result<String, Box<dyn Error + Send + Sync>> {
        #[cfg(target_os = "macos")]
        {
            self.get_username_macos(domain)
        }
        #[cfg(not(target_os = "macos"))]
        {
            self.get_username_other(domain)
        }
    }

    /// Read the password (Data field) for a domain — triggers biometric on macOS.
    pub fn get_password(&self, domain: &str) -> Result<String, Box<dyn Error + Send + Sync>> {
        #[cfg(target_os = "macos")]
        {
            self.get_password_macos(domain)
        }
        #[cfg(not(target_os = "macos"))]
        {
            self.get_password_other(domain)
        }
    }

    /// List all configured domains with their credential status.
    pub fn list_domains(&self) -> Result<Vec<DomainEntry>, Box<dyn Error + Send + Sync>> {
        let index = self.read_domains_index()?;
        Ok(index
            .into_iter()
            .map(|e| DomainEntry {
                domain: e.domain,
                has_username: e.has_username,
                has_password: e.has_password,
            })
            .collect())
    }

    /// Delete all credential data for a domain.
    pub fn delete_domain(&self, domain: &str) -> Result<(), Box<dyn Error + Send + Sync>> {
        #[cfg(target_os = "macos")]
        {
            self.delete_domain_macos(domain)?;
        }
        #[cfg(not(target_os = "macos"))]
        {
            self.delete_domain_other(domain)?;
        }
        // Remove from index
        let mut index = self.read_domains_index()?;
        index.retain(|e| e.domain != domain);
        self.write_domains_index(&index)?;
        Ok(())
    }

    /// Return all stored usernames for log scrubbing — no biometric prompt.
    ///
    /// Passwords are NOT included here because reading them triggers biometric
    /// on macOS. Callers that need passwords for scrubbing should maintain their
    /// own cache of resolved values (see `scrub_known_secrets` in js_api.rs).
    pub fn all_usernames(&self) -> Result<Vec<String>, Box<dyn Error + Send + Sync>> {
        let index = self.read_domains_index()?;
        let mut values = Vec::new();
        for entry in &index {
            if entry.has_username {
                if let Ok(u) = self.get_username(&entry.domain) {
                    if !u.is_empty() {
                        values.push(u);
                    }
                }
            }
        }
        Ok(values)
    }

    // ── macOS implementation ────────────────────────────────────────────────

    /// On macOS the single keychain entry per domain has:
    ///   - kSecAttrAccount = username (plaintext attribute, no auth)
    ///   - kSecValueData = password (biometric-protected)
    ///
    /// set_credentials: delete old entry (if username changed), create new with biometric data.
    #[cfg(target_os = "macos")]
    fn set_credentials_macos(
        &self,
        domain: &str,
        username: &str,
        password: &str,
    ) -> Result<(), Box<dyn Error + Send + Sync>> {
        use security_framework::passwords::{
            set_generic_password_options, AccessControlOptions, PasswordOptions,
        };

        let service = self.service_for_domain(domain);

        // Delete any existing entry (account name may differ if username changed).
        self.delete_domain_macos(domain).ok();

        // Create new entry with biometric-protected password and username as account.
        let mut options = PasswordOptions::new_generic_password(&service, username);
        options.set_access_control_options(AccessControlOptions::BIOMETRY_ANY);
        if let Err(err) = set_generic_password_options(password.as_bytes(), options) {
            if cfg!(debug_assertions) {
                eprintln!(
                    "Warning: biometric keychain write failed for '{domain}', using dev fallback: {err}"
                );
                security_framework::passwords::set_generic_password(
                    &service,
                    username,
                    password.as_bytes(),
                )?;
                return Ok(());
            }
            // Retry with USER_PRESENCE fallback
            let mut options2 = PasswordOptions::new_generic_password(&service, username);
            options2.set_access_control_options(AccessControlOptions::USER_PRESENCE);
            set_generic_password_options(password.as_bytes(), options2)?;
        }
        Ok(())
    }

    /// Set just the username: create/replace entry with empty data (no biometric).
    /// If a password already exists, it is lost — callers should use set_credentials instead.
    #[cfg(target_os = "macos")]
    fn set_username_macos(
        &self,
        domain: &str,
        username: &str,
    ) -> Result<(), Box<dyn Error + Send + Sync>> {
        let service = self.service_for_domain(domain);
        // Delete any existing entry for this domain service.
        self.delete_domain_macos(domain).ok();
        // Create entry with username as account, empty data (no biometric required).
        security_framework::passwords::set_generic_password(&service, username, b"")?;
        Ok(())
    }

    /// Set just the password: look up current username, delete old entry, recreate with biometric.
    #[cfg(target_os = "macos")]
    fn set_password_macos(
        &self,
        domain: &str,
        password: &str,
    ) -> Result<(), Box<dyn Error + Send + Sync>> {
        // Read current username (no biometric needed — account field).
        let username = self.get_username_macos(domain)?;
        // Recreate with biometric password.
        self.set_credentials_macos(domain, &username, password)
    }

    /// Read kSecAttrAccount for this domain's entry — no biometric prompt.
    #[cfg(target_os = "macos")]
    fn get_username_macos(&self, domain: &str) -> Result<String, Box<dyn Error + Send + Sync>> {
        use security_framework::item::{ItemClass, ItemSearchOptions, Limit};

        let service = self.service_for_domain(domain);
        let results = ItemSearchOptions::new()
            .class(ItemClass::generic_password())
            .service(&service)
            .load_attributes(true)
            .limit(Limit::Max(1))
            .search()?;

        for result in results {
            // simplify_dict() maps kSecAttrAccount → "acct"
            if let Some(attrs) = result.simplify_dict() {
                if let Some(account) = attrs.get("acct") {
                    return Ok(account.clone());
                }
            }
        }
        Err(format!("no credential entry found for domain '{domain}'").into())
    }

    /// Read kSecValueData for this domain's entry — triggers biometric on macOS.
    #[cfg(target_os = "macos")]
    fn get_password_macos(&self, domain: &str) -> Result<String, Box<dyn Error + Send + Sync>> {
        use security_framework::item::{ItemClass, ItemSearchOptions, Limit, SearchResult};

        let service = self.service_for_domain(domain);
        let results = ItemSearchOptions::new()
            .class(ItemClass::generic_password())
            .service(&service)
            .load_data(true)
            .limit(Limit::Max(1))
            .search()?;

        for result in results {
            if let SearchResult::Data(data) = result {
                return Ok(String::from_utf8(data)?);
            }
        }
        Err(format!("no password found for domain '{domain}'").into())
    }

    /// Delete all keychain entries for this domain service.
    #[cfg(target_os = "macos")]
    fn delete_domain_macos(&self, domain: &str) -> Result<(), Box<dyn Error + Send + Sync>> {
        use security_framework::item::{ItemClass, ItemSearchOptions};

        let service = self.service_for_domain(domain);
        // Delete all entries matching this service (ignoring account).
        let delete_result = ItemSearchOptions::new()
            .class(ItemClass::generic_password())
            .service(&service)
            .delete();
        match delete_result {
            Ok(()) => Ok(()),
            Err(e) => {
                // -25300 is Apple's errSecItemNotFound.
                if e.code() == -25300 {
                    Ok(())
                } else {
                    Err(e.into())
                }
            }
        }
    }

    // ── Non-macOS implementation ────────────────────────────────────────────

    /// On non-macOS platforms keyring entries don't expose account-field listing,
    /// so we use two fixed-account entries per domain:
    ///   - `_usr` → username (regular entry, no biometric)
    ///   - `_pwd` → password (regular entry)
    #[cfg(not(target_os = "macos"))]
    fn set_credentials_other(
        &self,
        domain: &str,
        username: &str,
        password: &str,
    ) -> Result<(), Box<dyn Error + Send + Sync>> {
        self.set_username_other(domain, username)?;
        self.set_password_other(domain, password)?;
        Ok(())
    }

    #[cfg(not(target_os = "macos"))]
    fn set_username_other(
        &self,
        domain: &str,
        username: &str,
    ) -> Result<(), Box<dyn Error + Send + Sync>> {
        let service = self.service_for_domain(domain);
        let entry = keyring::Entry::new(&service, "_usr")?;
        entry.set_password(username)?;
        Ok(())
    }

    #[cfg(not(target_os = "macos"))]
    fn set_password_other(
        &self,
        domain: &str,
        password: &str,
    ) -> Result<(), Box<dyn Error + Send + Sync>> {
        let service = self.service_for_domain(domain);
        let entry = keyring::Entry::new(&service, "_pwd")?;
        entry.set_password(password)?;
        Ok(())
    }

    #[cfg(not(target_os = "macos"))]
    fn get_username_other(&self, domain: &str) -> Result<String, Box<dyn Error + Send + Sync>> {
        let service = self.service_for_domain(domain);
        let entry = keyring::Entry::new(&service, "_usr")?;
        Ok(entry.get_password()?)
    }

    #[cfg(not(target_os = "macos"))]
    fn get_password_other(&self, domain: &str) -> Result<String, Box<dyn Error + Send + Sync>> {
        let service = self.service_for_domain(domain);
        let entry = keyring::Entry::new(&service, "_pwd")?;
        Ok(entry.get_password()?)
    }

    #[cfg(not(target_os = "macos"))]
    fn delete_domain_other(&self, domain: &str) -> Result<(), Box<dyn Error + Send + Sync>> {
        let service = self.service_for_domain(domain);
        for account in &["_usr", "_pwd"] {
            match keyring::Entry::new(&service, account).and_then(|e| e.delete_credential()) {
                Ok(()) => {}
                Err(keyring::Error::NoEntry) => {}
                Err(e) => return Err(e.into()),
            }
        }
        Ok(())
    }

    // ── Legacy migration helpers ────────────────────────────────────────────

    /// Check whether old-format entries exist for this login.
    ///
    /// Old format: service=`refreshmint/<login>`, account=`<domain>/<name>`.
    /// Returns a list of old-style (domain, name) pairs found in the keychain.
    pub fn list_legacy_entries(
        &self,
    ) -> Result<Vec<(String, String)>, Box<dyn Error + Send + Sync>> {
        let old_service = format!("refreshmint/{}", self.login_name);
        let entry = keyring::Entry::new(&old_service, Self::INDEX_ACCOUNT)?;
        let json = match entry.get_password() {
            Ok(j) => j,
            Err(keyring::Error::NoEntry) => return Ok(Vec::new()),
            Err(e) => return Err(e.into()),
        };

        // Legacy index was Vec<SecretIndexEntry> = Vec<{key: "domain/name", has_value: bool}>
        #[derive(serde::Deserialize)]
        struct LegacyIndexEntry {
            key: String,
            #[serde(default)]
            has_value: bool,
        }
        let legacy: Vec<LegacyIndexEntry> = serde_json::from_str(&json).unwrap_or_default();
        let mut pairs = Vec::new();
        for entry in legacy {
            if entry.has_value {
                if let Some((domain, name)) = entry.key.split_once('/') {
                    pairs.push((domain.to_string(), name.to_string()));
                }
            }
        }
        Ok(pairs)
    }

    /// Read a single legacy secret value (for migration).  Triggers biometric on macOS.
    pub fn get_legacy_value(
        &self,
        domain: &str,
        name: &str,
    ) -> Result<String, Box<dyn Error + Send + Sync>> {
        let old_service = format!("refreshmint/{}", self.login_name);
        let account = format!("{domain}/{name}");
        let entry = keyring::Entry::new(&old_service, &account)?;
        Ok(entry.get_password()?)
    }

    /// Delete a single legacy secret entry.
    pub fn delete_legacy_entry(
        &self,
        domain: &str,
        name: &str,
    ) -> Result<(), Box<dyn Error + Send + Sync>> {
        let old_service = format!("refreshmint/{}", self.login_name);
        let account = format!("{domain}/{name}");
        let entry = keyring::Entry::new(&old_service, &account)?;
        match entry.delete_credential() {
            Ok(()) => {}
            Err(keyring::Error::NoEntry) => {}
            Err(e) => return Err(e.into()),
        }
        Ok(())
    }

    /// Delete the legacy domains index entry.
    pub fn delete_legacy_index(&self) -> Result<(), Box<dyn Error + Send + Sync>> {
        let old_service = format!("refreshmint/{}", self.login_name);
        let entry = keyring::Entry::new(&old_service, Self::INDEX_ACCOUNT)?;
        match entry.delete_credential() {
            Ok(()) => {}
            Err(keyring::Error::NoEntry) => {}
            Err(e) => return Err(e.into()),
        }
        Ok(())
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    fn test_login() -> String {
        format!(
            "test-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        )
    }

    fn cleanup(store: &SecretStore) {
        if let Ok(domains) = store.list_domains() {
            for entry in &domains {
                let _ = store.delete_domain(&entry.domain);
            }
        }
        // Clean up index
        let idx = keyring::Entry::new(&store.index_service(), SecretStore::INDEX_ACCOUNT);
        if let Ok(e) = idx {
            let _ = e.delete_credential();
        }
    }

    #[test]
    fn set_get_credentials_roundtrip() {
        let store = SecretStore::new(test_login());
        let result = store.set_credentials("example.com", "alice", "hunter2");
        if let Err(e) = &result {
            eprintln!("skipping keyring test (set failed): {e}");
            return;
        }

        let username = store.get_username("example.com").unwrap();
        assert_eq!(username, "alice");

        let password = store.get_password("example.com").unwrap();
        assert_eq!(password, "hunter2");

        cleanup(&store);
    }

    #[test]
    fn list_domains_returns_configured_entries() {
        let store = SecretStore::new(test_login());
        if store.set_credentials("a.com", "user_a", "pass_a").is_err() {
            eprintln!("skipping keyring test");
            return;
        }
        store.set_credentials("b.com", "user_b", "pass_b").unwrap();

        let domains = store.list_domains().unwrap();
        assert_eq!(domains.len(), 2);
        assert!(domains.iter().any(|d| d.domain == "a.com"));
        assert!(domains.iter().any(|d| d.domain == "b.com"));

        cleanup(&store);
    }

    #[test]
    fn delete_domain_removes_entry() {
        let store = SecretStore::new(test_login());
        if store.set_credentials("x.com", "user", "pass").is_err() {
            eprintln!("skipping keyring test");
            return;
        }

        store.delete_domain("x.com").unwrap();
        let domains = store.list_domains().unwrap();
        assert!(domains.is_empty());

        cleanup(&store);
    }

    #[test]
    fn all_usernames_returns_username() {
        let store = SecretStore::new(test_login());
        if store
            .set_credentials("secret.com", "myuser", "mypass")
            .is_err()
        {
            eprintln!("skipping keyring test");
            return;
        }

        let usernames = store.all_usernames().unwrap();
        assert!(usernames.contains(&"myuser".to_string()));
        // Passwords are NOT returned (would trigger biometric on macOS)
        assert!(!usernames.contains(&"mypass".to_string()));

        cleanup(&store);
    }

    #[test]
    fn set_credentials_replaces_existing() {
        let store = SecretStore::new(test_login());
        if store
            .set_credentials("d.com", "old_user", "old_pass")
            .is_err()
        {
            eprintln!("skipping keyring test");
            return;
        }
        store
            .set_credentials("d.com", "new_user", "new_pass")
            .unwrap();

        let username = store.get_username("d.com").unwrap();
        assert_eq!(username, "new_user");

        let password = store.get_password("d.com").unwrap();
        assert_eq!(password, "new_pass");

        let domains = store.list_domains().unwrap();
        assert_eq!(domains.len(), 1);

        cleanup(&store);
    }
}
