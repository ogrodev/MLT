//! OS-keychain-backed `SecretStore` for OUR secrets (refreshed tokens, settings), via the
//! cross-platform `keyring` crate. Distinct from reading a vendor CLI's creds (see claude.rs):
//! this is where MLT keeps things it owns, never the providers' own keychain items.
use keyring::Entry;
use mlt_core::ports::{PortError, SecretStore};

/// Stores secrets under one keychain `service` (e.g. `com.bigshotpictures.mlt`), keyed by name.
#[derive(Debug, Clone)]
pub struct KeyringSecretStore {
    service: String,
}

impl KeyringSecretStore {
    pub fn new(service: impl Into<String>) -> Self {
        Self {
            service: service.into(),
        }
    }

    fn entry(&self, key: &str) -> Result<Entry, PortError> {
        Entry::new(&self.service, key).map_err(|e| PortError::Io(e.to_string()))
    }
}

impl SecretStore for KeyringSecretStore {
    fn get(&self, key: &str) -> Result<Option<String>, PortError> {
        match self.entry(key)?.get_password() {
            Ok(s) => Ok(Some(s)),
            Err(keyring::Error::NoEntry) => Ok(None),
            Err(e) => Err(PortError::Io(e.to_string())),
        }
    }

    fn set(&self, key: &str, value: &str) -> Result<(), PortError> {
        self.entry(key)?
            .set_password(value)
            .map_err(|e| PortError::Io(e.to_string()))
    }

    fn delete(&self, key: &str) -> Result<(), PortError> {
        match self.entry(key)?.delete_credential() {
            Ok(()) | Err(keyring::Error::NoEntry) => Ok(()),
            Err(e) => Err(PortError::Io(e.to_string())),
        }
    }
}
