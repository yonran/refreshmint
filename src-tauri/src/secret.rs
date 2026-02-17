use std::error::Error;

/// Keyring-based secret storage for a single account.
///
/// Secrets are stored in the OS keychain with:
/// - Service: `refreshmint/<account>`
/// - User: `<domain>/<name>`
///
/// An index entry at user=`_index` maintains a JSON array of all
/// `"domain/name"` strings for enumeration (since keyring doesn't
/// support listing).
pub struct SecretStore {
    account: String,
}

impl SecretStore {
    pub fn new(account: String) -> Self {
        Self { account }
    }

    fn service(&self) -> String {
        format!("refreshmint/{}", self.account)
    }

    fn entry(&self, user: &str) -> Result<keyring::Entry, keyring::Error> {
        keyring::Entry::new(&self.service(), user)
    }

    fn index_entry(&self) -> Result<keyring::Entry, keyring::Error> {
        self.entry("_index")
    }

    fn read_index(&self) -> Result<Vec<String>, Box<dyn Error>> {
        let entry = self.index_entry()?;
        match entry.get_password() {
            Ok(json) => {
                let keys: Vec<String> = serde_json::from_str(&json)?;
                Ok(keys)
            }
            Err(keyring::Error::NoEntry) => Ok(Vec::new()),
            Err(e) => Err(e.into()),
        }
    }

    fn write_index(&self, keys: &[String]) -> Result<(), Box<dyn Error>> {
        let json = serde_json::to_string(keys)?;
        // Keep index metadata as a plain keychain entry so list/index operations
        // do not require biometric auth.
        self.set_entry_password("_index", &json, false)?;
        Ok(())
    }

    fn key(domain: &str, name: &str) -> String {
        format!("{domain}/{name}")
    }

    pub fn set(&self, domain: &str, name: &str, value: &str) -> Result<(), Box<dyn Error>> {
        let user = Self::key(domain, name);
        self.set_entry_password(&user, value, true)?;

        let mut index = self.read_index()?;
        if !index.contains(&user) {
            index.push(user);
            self.write_index(&index)?;
        }
        Ok(())
    }

    pub fn get(&self, domain: &str, name: &str) -> Result<String, Box<dyn Error>> {
        let user = Self::key(domain, name);
        let entry = self.entry(&user)?;
        let value = entry.get_password()?;
        Ok(value)
    }

    pub fn delete(&self, domain: &str, name: &str) -> Result<(), Box<dyn Error>> {
        let user = Self::key(domain, name);
        let entry = self.entry(&user)?;
        match entry.delete_credential() {
            Ok(()) => {}
            Err(keyring::Error::NoEntry) => {}
            Err(e) => return Err(e.into()),
        }

        let mut index = self.read_index()?;
        index.retain(|k| k != &user);
        self.write_index(&index)?;
        Ok(())
    }

    /// List all (domain, name) pairs stored for this account.
    pub fn list(&self) -> Result<Vec<(String, String)>, Box<dyn Error>> {
        let index = self.read_index()?;
        let mut pairs = Vec::new();
        for key in &index {
            if let Some((domain, name)) = key.split_once('/') {
                pairs.push((domain.to_string(), name.to_string()));
            }
        }
        Ok(pairs)
    }

    /// Return all secret values for this account (used for scrubbing).
    pub fn all_values(&self) -> Result<Vec<String>, Box<dyn Error>> {
        let index = self.read_index()?;
        let mut values = Vec::new();
        for key in &index {
            let entry = self.entry(key)?;
            match entry.get_password() {
                Ok(v) => values.push(v),
                Err(keyring::Error::NoEntry) => {}
                Err(e) => return Err(e.into()),
            }
        }
        Ok(values)
    }

    fn set_entry_password(
        &self,
        user: &str,
        value: &str,
        require_biometry: bool,
    ) -> Result<(), Box<dyn Error>> {
        #[cfg(target_os = "macos")]
        if require_biometry {
            self.set_entry_password_with_biometry(user, value)?;
            return Ok(());
        }

        let entry = self.entry(user)?;
        entry.set_password(value)?;
        Ok(())
    }

    #[cfg(target_os = "macos")]
    fn set_entry_password_with_biometry(
        &self,
        user: &str,
        value: &str,
    ) -> Result<(), Box<dyn Error>> {
        use security_framework::passwords::{
            set_generic_password_options, AccessControlOptions, PasswordOptions,
        };

        let service = self.service();
        let mut options = PasswordOptions::new_generic_password(&service, user);
        options.set_access_control_options(AccessControlOptions::BIOMETRY_ANY);
        if set_generic_password_options(value.as_bytes(), options).is_ok() {
            return Ok(());
        }

        // Fallback for environments where pure-biometry constraints are unavailable.
        let mut options = PasswordOptions::new_generic_password(&service, user);
        options.set_access_control_options(AccessControlOptions::USER_PRESENCE);
        set_generic_password_options(value.as_bytes(), options)?;
        Ok(())
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    fn test_account() -> String {
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
        if let Ok(pairs) = store.list() {
            for (d, n) in &pairs {
                let _ = store.delete(d, n);
            }
        }
        // Also clean up the index entry itself
        if let Ok(entry) = store.index_entry() {
            let _ = entry.delete_credential();
        }
    }

    #[test]
    fn set_get_roundtrip() {
        let store = SecretStore::new(test_account());
        let result = store.set("example.com", "password", "hunter2");
        if let Err(e) = &result {
            // keyring may fail in CI or headless environments; skip gracefully
            eprintln!("skipping keyring test (set failed): {e}");
            return;
        }

        let value = store.get("example.com", "password").unwrap();
        assert_eq!(value, "hunter2");

        cleanup(&store);
    }

    #[test]
    fn list_returns_stored_pairs() {
        let store = SecretStore::new(test_account());
        if store.set("a.com", "user", "u").is_err() {
            eprintln!("skipping keyring test");
            return;
        }
        store.set("b.com", "token", "t").unwrap();

        let pairs = store.list().unwrap();
        assert_eq!(pairs.len(), 2);
        assert!(pairs.contains(&("a.com".to_string(), "user".to_string())));
        assert!(pairs.contains(&("b.com".to_string(), "token".to_string())));

        cleanup(&store);
    }

    #[test]
    fn delete_removes_entry() {
        let store = SecretStore::new(test_account());
        if store.set("x.com", "key", "val").is_err() {
            eprintln!("skipping keyring test");
            return;
        }

        store.delete("x.com", "key").unwrap();
        let pairs = store.list().unwrap();
        assert!(pairs.is_empty());

        // get should fail after delete
        assert!(store.get("x.com", "key").is_err());

        cleanup(&store);
    }

    #[test]
    fn all_values_returns_secret_values() {
        let store = SecretStore::new(test_account());
        if store.set("a.com", "pw", "secret1").is_err() {
            eprintln!("skipping keyring test");
            return;
        }
        store.set("b.com", "pw", "secret2").unwrap();

        let values = store.all_values().unwrap();
        assert_eq!(values.len(), 2);
        assert!(values.contains(&"secret1".to_string()));
        assert!(values.contains(&"secret2".to_string()));

        cleanup(&store);
    }

    #[test]
    fn set_is_idempotent_for_index() {
        let store = SecretStore::new(test_account());
        if store.set("d.com", "name", "v1").is_err() {
            eprintln!("skipping keyring test");
            return;
        }
        // Set same key again with different value
        store.set("d.com", "name", "v2").unwrap();

        let pairs = store.list().unwrap();
        assert_eq!(pairs.len(), 1);

        let value = store.get("d.com", "name").unwrap();
        assert_eq!(value, "v2");

        cleanup(&store);
    }

    #[test]
    fn delete_nonexistent_is_ok() {
        let store = SecretStore::new(test_account());
        // Should not error when deleting something that doesn't exist
        let result = store.delete("nonexistent.com", "nope");
        // May fail if keyring itself is unavailable, but shouldn't panic
        if let Err(e) = result {
            eprintln!("skipping keyring test (delete failed): {e}");
        }
        cleanup(&store);
    }

    #[test]
    fn list_empty_account() {
        let store = SecretStore::new(test_account());
        match store.list() {
            Ok(pairs) => assert!(pairs.is_empty()),
            Err(e) => eprintln!("skipping keyring test: {e}"),
        }
    }
}
