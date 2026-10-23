//! Remote secrets (passwords, secret keys) in the OS credential store.

use std::collections::HashMap;
use std::sync::Mutex;

use porter_core::{Error, Result};

const SERVICE: &str = "porter";

/// Stores one secret per remote.
pub trait SecretStore: Send + Sync {
    fn get(&self, remote: &str) -> Result<Option<String>>;
    fn set(&self, remote: &str, secret: &str) -> Result<()>;
    /// Removes the secret; no error if there is none.
    fn delete(&self, remote: &str) -> Result<()>;
}

/// macOS Keychain, Windows Credential Manager or the Secret Service on Linux.
#[derive(Debug, Default, Clone, Copy)]
pub struct Keychain;

impl Keychain {
    fn entry(remote: &str) -> Result<keyring::Entry> {
        keyring::Entry::new(SERVICE, &format!("remote/{remote}")).map_err(keychain_error)
    }
}

impl SecretStore for Keychain {
    fn get(&self, remote: &str) -> Result<Option<String>> {
        match Self::entry(remote)?.get_password() {
            Ok(secret) => Ok(Some(secret)),
            Err(keyring::Error::NoEntry) => Ok(None),
            Err(e) => Err(keychain_error(e)),
        }
    }

    fn set(&self, remote: &str, secret: &str) -> Result<()> {
        Self::entry(remote)?
            .set_password(secret)
            .map_err(keychain_error)
    }

    fn delete(&self, remote: &str) -> Result<()> {
        match Self::entry(remote)?.delete_credential() {
            Ok(()) | Err(keyring::Error::NoEntry) => Ok(()),
            Err(e) => Err(keychain_error(e)),
        }
    }
}

fn keychain_error(e: keyring::Error) -> Error {
    Error::backend("keychain", e)
}

/// In-memory store for tests and sessions without a keychain.
#[derive(Debug, Default)]
pub struct MemorySecrets(Mutex<HashMap<String, String>>);

impl SecretStore for MemorySecrets {
    fn get(&self, remote: &str) -> Result<Option<String>> {
        Ok(self.0.lock().unwrap().get(remote).cloned())
    }

    fn set(&self, remote: &str, secret: &str) -> Result<()> {
        self.0
            .lock()
            .unwrap()
            .insert(remote.to_owned(), secret.to_owned());
        Ok(())
    }

    fn delete(&self, remote: &str) -> Result<()> {
        self.0.lock().unwrap().remove(remote);
        Ok(())
    }
}
