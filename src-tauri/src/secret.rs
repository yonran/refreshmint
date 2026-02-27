use std::error::Error;

#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
struct SecretIndexEntry {
    key: String,
    #[serde(default)]
    has_value: bool,
}

pub type SecretValueStateRow = (String, String, bool);

/// Keyring-based secret storage for a single account.
///
/// Secrets are stored in the OS keychain with:
/// - Service: `refreshmint/<account>`
/// - User: `<domain>/<name>`
///
/// An index entry at user=`_index` maintains metadata for all known
/// `"domain/name"` strings for enumeration (since keyring doesn't
/// support listing) and whether a value is currently stored.
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

    fn read_index_entries(&self) -> Result<Vec<SecretIndexEntry>, Box<dyn Error>> {
        let entry = self.index_entry()?;
        match entry.get_password() {
            Ok(json) => {
                if let Ok(entries) = serde_json::from_str::<Vec<SecretIndexEntry>>(&json) {
                    return Ok(entries);
                }

                // Backward compatibility with legacy index format: `["domain/name", ...]`.
                let keys = serde_json::from_str::<Vec<String>>(&json)?;
                Ok(keys
                    .into_iter()
                    .map(|key| SecretIndexEntry {
                        key,
                        has_value: false,
                    })
                    .collect())
            }
            Err(keyring::Error::NoEntry) => Ok(Vec::new()),
            Err(e) => Err(e.into()),
        }
    }

    fn write_index_entries(&self, entries: &[SecretIndexEntry]) -> Result<(), Box<dyn Error>> {
        let json = serde_json::to_string(entries)?;
        // Keep index metadata as a plain keychain entry so list/index operations
        // do not require biometric auth.
        self.set_entry_password("_index", &json, false)?;
        Ok(())
    }

    fn key(domain: &str, name: &str) -> String {
        format!("{domain}/{name}")
    }

    fn upsert_index_entry(entries: &mut Vec<SecretIndexEntry>, key: String, has_value: bool) {
        if let Some(existing) = entries.iter_mut().find(|entry| entry.key == key) {
            if has_value {
                existing.has_value = true;
            }
        } else {
            entries.push(SecretIndexEntry { key, has_value });
        }
    }

    pub fn set(&self, domain: &str, name: &str, value: &str) -> Result<(), Box<dyn Error>> {
        let user = Self::key(domain, name);
        self.set_entry_password(&user, value, true)?;

        let mut index = self.read_index_entries()?;
        Self::upsert_index_entry(&mut index, user, true);
        index.sort_by(|a, b| a.key.cmp(&b.key));
        self.write_index_entries(&index)?;
        Ok(())
    }

    /// Ensure a (domain, name) pair is present in the index without setting a value.
    ///
    /// This is useful for preparing required secret slots ahead of time so UI can
    /// prompt for values later, without forcing a keychain write/biometric prompt.
    pub fn ensure_indexed(&self, domain: &str, name: &str) -> Result<(), Box<dyn Error>> {
        let user = Self::key(domain, name);
        let mut index = self.read_index_entries()?;
        Self::upsert_index_entry(&mut index, user, false);
        index.sort_by(|a, b| a.key.cmp(&b.key));
        self.write_index_entries(&index)?;
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

        let mut index = self.read_index_entries()?;
        index.retain(|entry| entry.key != user);
        self.write_index_entries(&index)?;
        Ok(())
    }

    /// List all (domain, name) pairs stored for this account.
    pub fn list(&self) -> Result<Vec<(String, String)>, Box<dyn Error>> {
        let index = self.read_index_entries()?;
        let mut pairs = Vec::new();
        for entry in &index {
            if let Some((domain, name)) = entry.key.split_once('/') {
                pairs.push((domain.to_string(), name.to_string()));
            }
        }
        Ok(pairs)
    }

    /// List all (domain, name) pairs and whether a value exists.
    pub fn list_with_value_state(&self) -> Result<Vec<SecretValueStateRow>, Box<dyn Error>> {
        let index = self.read_index_entries()?;
        let mut entries = Vec::new();
        for entry in &index {
            if let Some((domain, name)) = entry.key.split_once('/') {
                entries.push((domain.to_string(), name.to_string(), entry.has_value));
            }
        }
        Ok(entries)
    }

    /// Return all secret values for this account (used for scrubbing).
    pub fn all_values(&self) -> Result<Vec<String>, Box<dyn Error>> {
        let index = self.read_index_entries()?;
        let mut values = Vec::new();
        for indexed in &index {
            if !indexed.has_value {
                continue;
            }
            let entry = self.entry(&indexed.key)?;
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
        #[cfg(not(target_os = "macos"))]
        let _ = require_biometry;

        #[cfg(target_os = "macos")]
        if require_biometry {
            if let Err(err) = self.set_entry_password_with_biometry(user, value) {
                if cfg!(debug_assertions) {
                    eprintln!(
                        "Warning: secure keychain write failed for '{user}', using dev fallback: {err}"
                    );
                    let entry = self.entry(user)?;
                    entry.set_password(value)?;
                    return Ok(());
                }
                return Err(err);
            }
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
