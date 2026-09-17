//! Sekrety wyłącznie w systemowym credential store (macOS Keychain / Windows Credential Manager).
//! Brak dostępu = jasny błąd. Celowo nie istnieje żaden fallback do pliku.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

/// Wartość sekretu: nie da się jej przypadkiem wypisać ani zserializować.
#[derive(Clone)]
pub struct Secret(String);

impl Secret {
    pub fn new(value: String) -> Self {
        crate::diagnostics::register_secret(&value);
        Self(value)
    }
    pub fn expose(&self) -> &str {
        &self.0
    }
}

impl std::fmt::Debug for Secret {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("Secret([REDACTED])")
    }
}

/// Osobny wpis per provider + źródło + rodzaj sekretu (np. `api_token`, w przyszłości `refresh_token`).
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct SecretKey {
    pub provider: String,
    pub source_id: String,
    pub kind: String,
}

impl SecretKey {
    pub fn new(provider: &str, source_id: &str, kind: &str) -> Self {
        Self { provider: provider.into(), source_id: source_id.into(), kind: kind.into() }
    }
    /// Konto w credential store; usługą (namespace) jest identyfikator aplikacji.
    fn account(&self) -> String {
        format!("{}/{}/{}", self.provider, self.source_id, self.kind)
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct SecretError(pub String);

impl std::fmt::Display for SecretError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

pub trait SecretStore: Send + Sync {
    fn set(&self, key: &SecretKey, value: &Secret) -> Result<(), SecretError>;
    fn get(&self, key: &SecretKey) -> Result<Option<Secret>, SecretError>;
    /// Usunięcie nieistniejącego wpisu nie jest błędem.
    fn delete(&self, key: &SecretKey) -> Result<(), SecretError>;
    /// Czy magazyn jest w ogóle dostępny (diagnostyka).
    fn status(&self) -> Result<(), SecretError>;
}

pub struct KeyringStore;

impl KeyringStore {
    fn entry(key: &SecretKey) -> Result<keyring::Entry, SecretError> {
        keyring::Entry::new(crate::APP_ID, &key.account()).map_err(store_error)
    }
}

fn store_error(e: keyring::Error) -> SecretError {
    SecretError(crate::diagnostics::redact(&format!("system credential store error: {e}")))
}

impl SecretStore for KeyringStore {
    fn set(&self, key: &SecretKey, value: &Secret) -> Result<(), SecretError> {
        // Surowe bajty UTF-8: na Windows wpis mieści 2560 bajtów, a `set_password` koduje UTF-16 (połowa pojemności) —
        // tokeny OAuth (JWT ~1,5 KB) by się nie zmieściły.
        Self::entry(key)?.set_secret(value.expose().as_bytes()).map_err(store_error)
    }

    fn get(&self, key: &SecretKey) -> Result<Option<Secret>, SecretError> {
        match Self::entry(key)?.get_secret() {
            Ok(bytes) => String::from_utf8(bytes).map(|value| Some(Secret::new(value))).map_err(|_| SecretError("stored credential is not valid UTF-8".into())),
            Err(keyring::Error::NoEntry) => Ok(None),
            Err(e) => Err(store_error(e)),
        }
    }

    fn delete(&self, key: &SecretKey) -> Result<(), SecretError> {
        match Self::entry(key)?.delete_credential() {
            Ok(()) | Err(keyring::Error::NoEntry) => Ok(()),
            Err(e) => Err(store_error(e)),
        }
    }

    fn status(&self) -> Result<(), SecretError> {
        // Odczyt nieistniejącego wpisu: sprawdza dostęp bez tworzenia czegokolwiek.
        self.get(&SecretKey::new("diagnostics", "probe", "status")).map(|_| ())
    }
}

/// Fake do testów i CI — nigdy nie dotyka dysku.
#[derive(Default)]
pub struct MemoryStore {
    entries: Mutex<HashMap<SecretKey, String>>,
    unavailable: bool,
}

impl MemoryStore {
    /// Symuluje zablokowany/niedostępny credential store.
    pub fn unavailable() -> Self {
        Self { unavailable: true, ..Default::default() }
    }

    fn check(&self) -> Result<(), SecretError> {
        if self.unavailable {
            return Err(SecretError("credential store unavailable".into()));
        }
        Ok(())
    }
}

/// Limit pojedynczego wpisu Windows Credential Manager (`CRED_MAX_CREDENTIAL_BLOB_SIZE`).
pub const MAX_SECRET_BYTES: usize = 2560;

impl SecretStore for MemoryStore {
    fn set(&self, key: &SecretKey, value: &Secret) -> Result<(), SecretError> {
        self.check()?;
        // fake zachowuje się jak najciaśniejszy prawdziwy store, żeby za duży sekret wyszedł w testach, a nie u użytkownika Windows
        if value.expose().len() > MAX_SECRET_BYTES {
            return Err(SecretError(format!("secret exceeds {MAX_SECRET_BYTES} bytes")));
        }
        self.entries.lock().unwrap().insert(key.clone(), value.expose().to_string());
        Ok(())
    }
    fn get(&self, key: &SecretKey) -> Result<Option<Secret>, SecretError> {
        self.check()?;
        Ok(self.entries.lock().unwrap().get(key).cloned().map(Secret::new))
    }
    fn delete(&self, key: &SecretKey) -> Result<(), SecretError> {
        self.check()?;
        self.entries.lock().unwrap().remove(key);
        Ok(())
    }
    fn status(&self) -> Result<(), SecretError> {
        self.check()
    }
}

pub fn default_store() -> Arc<dyn SecretStore> {
    // Tylko buildy debug (testy E2E uruchamiają prawdziwe binarium bez Keychaina):
    // ECOMMERCE_MCP_TEST_SECRETS='{"baselinker/<source_id>/api_token":"..."}'. W release tego kodu nie ma.
    #[cfg(debug_assertions)]
    if let Ok(raw) = std::env::var("ECOMMERCE_MCP_TEST_SECRETS") {
        let store = MemoryStore::default();
        let entries: HashMap<String, String> = serde_json::from_str(&raw).unwrap_or_default();
        for (account, value) in entries {
            if let [provider, source_id, kind] = account.split('/').collect::<Vec<_>>()[..] {
                let _ = store.set(&SecretKey::new(provider, source_id, kind), &Secret::new(value));
            }
        }
        return Arc::new(store);
    }
    Arc::new(KeyringStore)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn memory_store_set_get_delete() {
        let store = MemoryStore::default();
        let key = SecretKey::new("baselinker", "sklep", "api_token");
        assert!(store.get(&key).unwrap().is_none());
        store.set(&key, &Secret::new("abc-123-token".into())).unwrap();
        assert_eq!(store.get(&key).unwrap().unwrap().expose(), "abc-123-token");
        store.delete(&key).unwrap();
        store.delete(&key).unwrap(); // idempotentne
        assert!(store.get(&key).unwrap().is_none());
    }

    #[test]
    fn keys_are_namespaced_per_provider_source_and_kind() {
        assert_eq!(SecretKey::new("baselinker", "sklep", "api_token").account(), "baselinker/sklep/api_token");
        let store = MemoryStore::default();
        store.set(&SecretKey::new("baselinker", "a", "api_token"), &Secret::new("token-a".into())).unwrap();
        assert!(store.get(&SecretKey::new("baselinker", "b", "api_token")).unwrap().is_none());
        assert!(store.get(&SecretKey::new("allegro", "a", "api_token")).unwrap().is_none());
    }

    #[test]
    fn unavailable_store_fails_loudly() {
        let store = MemoryStore::unavailable();
        let key = SecretKey::new("baselinker", "sklep", "api_token");
        assert!(store.set(&key, &Secret::new("token".into())).is_err());
        assert!(store.status().is_err());
    }

    #[test]
    fn memory_store_enforces_the_windows_entry_size_limit() {
        let store = MemoryStore::default();
        let key = SecretKey::new("allegro", "konto", "access_token");
        assert!(store.set(&key, &Secret::new("x".repeat(MAX_SECRET_BYTES))).is_ok());
        assert!(store.set(&key, &Secret::new("x".repeat(MAX_SECRET_BYTES + 1))).is_err());
    }

    #[test]
    fn secret_debug_is_redacted() {
        assert_eq!(format!("{:?}", Secret::new("super-tajne".into())), "Secret([REDACTED])");
    }

    /// Prawdziwy Keychain / Credential Manager: `cargo test -- --ignored real_credential_store`.
    /// Poza CI — tworzy i od razu usuwa wpis z atrapą wartości.
    #[test]
    #[ignore = "dotyka prawdziwego systemowego credential store"]
    fn real_credential_store_roundtrip() {
        let store = KeyringStore;
        let key = SecretKey::new("selftest", "roundtrip", "api_token");
        store.status().unwrap();
        // 2000 znaków: rozmiar tokenu OAuth (JWT); jako UTF-16 nie zmieściłby się we wpisie Windows
        let value = "atrapa-".repeat(286);
        store.set(&key, &Secret::new(value.clone())).unwrap();
        assert_eq!(store.get(&key).unwrap().unwrap().expose(), value);
        store.delete(&key).unwrap();
        assert!(store.get(&key).unwrap().is_none());
        store.delete(&key).unwrap();
    }
}
