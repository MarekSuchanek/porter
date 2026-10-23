//! Storage backends and the registry that picks one for a [`Location`].

pub mod config;
pub mod local;
pub mod remote;
pub mod secrets;

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use porter_core::{Error, Location, Provider, Result, Target};

pub use config::{Backend, Config, Remote};
pub use local::LocalProvider;
pub use remote::OpendalProvider;
pub use secrets::{Keychain, MemorySecrets, SecretStore};

/// Resolves locations to providers using the saved remotes. Providers are cached per
/// backend, so operations on many paths of one server share a connection.
pub struct Registry {
    config: Config,
    secrets: Arc<dyn SecretStore>,
    cache: Mutex<HashMap<(String, String), Arc<dyn Provider>>>,
}

impl Registry {
    pub fn new(config: Config, secrets: Arc<dyn SecretStore>) -> Self {
        Self {
            config,
            secrets,
            cache: Mutex::default(),
        }
    }

    /// Uses the default config file and the OS keychain.
    pub fn load() -> Result<Self> {
        let config = Config::load_from(&Config::default_path()?)?;
        Ok(Self::new(config, Arc::new(Keychain)))
    }

    pub fn config(&self) -> &Config {
        &self.config
    }

    pub fn secrets(&self) -> &dyn SecretStore {
        self.secrets.as_ref()
    }

    /// The provider serving `location`.
    pub fn provider_for(&self, location: &Location) -> Result<Arc<dyn Provider>> {
        if location.is_local() {
            return Ok(Arc::new(LocalProvider));
        }
        let key = (location.scheme.clone(), location.authority.clone());
        if let Some(provider) = self.cache.lock().unwrap().get(&key) {
            return Ok(provider.clone());
        }
        let provider: Arc<dyn Provider> = match location.is_remote_alias() {
            true => Arc::new(self.remote_provider(&location.authority)?),
            false => Arc::new(OpendalProvider::from_url(location)?),
        };
        self.cache.lock().unwrap().insert(key, provider.clone());
        Ok(provider)
    }

    /// A fresh provider for the saved remote `name`.
    pub fn remote_provider(&self, name: &str) -> Result<OpendalProvider> {
        let remote = self.config.remotes.get(name).ok_or_else(|| {
            Error::invalid_location(
                format!("{name}:"),
                "unknown remote (see `porter remote list`)",
            )
        })?;
        let secret = match remote.backend.secret_label() {
            Some(_) => self.secrets.get(name)?,
            None => None,
        };
        OpendalProvider::from_remote(name, remote, secret.as_deref())
    }

    /// Parses user input into a [`Target`]; local paths are made absolute.
    pub fn target(&self, input: &str) -> Result<Target> {
        let mut location = Location::parse(input)?;
        if location.is_local() {
            let abs = std::path::absolute(&location.path).map_err(|e| Error::io(input, e))?;
            location.path = abs.display().to_string();
        }
        let provider = self.provider_for(&location)?;
        Ok(Target::new(location, provider))
    }
}

impl Default for Registry {
    /// No saved remotes: local paths and ad-hoc URLs only.
    fn default() -> Self {
        Self::new(Config::default(), Arc::new(MemorySecrets::default()))
    }
}

/// [`Registry::target`] without saved remotes.
pub fn target(input: &str) -> Result<Target> {
    Registry::default().target(input)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resolves_and_caches_remotes() {
        let mut config = Config::default();
        config.remotes.insert(
            "files".into(),
            Remote {
                backend: Backend::Ftp {
                    host: "localhost".into(),
                    port: Some(2121),
                    user: Some("me".into()),
                    tls: false,
                },
                root: None,
            },
        );
        let secrets = Arc::new(MemorySecrets::default());
        secrets.set("files", "pw").unwrap();
        let registry = Registry::new(config, secrets);

        let a = registry.target("files:/a").unwrap();
        let b = registry.target("files:/b").unwrap();
        assert_eq!(a.provider.scheme(), "ftp");
        assert!(Arc::ptr_eq(&a.provider, &b.provider));
        assert!(a.same_backend(&b));
        assert_eq!(a.to_string(), "files:/a");

        let unknown = registry.target("other:/a");
        assert!(matches!(unknown, Err(Error::InvalidLocation { .. })));
        assert!(registry.target("./other:/a").unwrap().location.is_local());
    }
}
