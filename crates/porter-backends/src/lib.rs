//! Storage backends and the registry that picks one for a [`Location`].

pub mod config;
pub mod local;
pub mod nfs;
mod pipe;
pub mod remote;
pub mod secrets;
pub mod smb;

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use porter_core::{Error, Location, Provider, Result, Target};

pub use config::{Backend, Config, Remote};
pub use local::LocalProvider;
pub use nfs::{NfsOptions, NfsProvider};
pub use remote::OpendalProvider;
pub use secrets::{Keychain, MemorySecrets, SecretStore};
pub use smb::SmbProvider;

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
            true => self.remote_provider(&location.authority)?,
            false => Arc::new(OpendalProvider::from_url(location)?),
        };
        self.cache.lock().unwrap().insert(key, provider.clone());
        Ok(provider)
    }

    /// A fresh provider for the saved remote `name`.
    pub fn remote_provider(&self, name: &str) -> Result<Arc<dyn Provider>> {
        let remote = self.config.remotes.get(name).ok_or_else(|| {
            Error::invalid_location(
                format!("{name}:"),
                "unknown remote (see `porter remote list`)",
            )
        })?;
        let secret = match remote.backend.secret_label() {
            Some(_) => match std::env::var(secret_env_var(name)) {
                Ok(secret) => Some(secret),
                Err(_) => self.secrets.get(name)?,
            },
            None => None,
        };
        let label = format!("{name}:");
        Ok(match &remote.backend {
            Backend::Smb {
                host,
                share,
                port,
                user,
            } => Arc::new(SmbProvider::new(
                name,
                label,
                host,
                share,
                *port,
                user.as_deref(),
                secret.as_deref(),
                remote.root.as_deref(),
            )),
            Backend::Nfs {
                host,
                export,
                port,
                mount_port,
                uid,
                gid,
                privileged_port,
            } => Arc::new(NfsProvider::new(
                name,
                label,
                NfsOptions {
                    host: host.clone(),
                    export: export.clone(),
                    port: *port,
                    mount_port: *mount_port,
                    uid: *uid,
                    gid: *gid,
                    privileged_port: *privileged_port,
                    root: remote.root.clone(),
                },
            )),
            _ => Arc::new(OpendalProvider::from_remote(
                name,
                remote,
                secret.as_deref(),
            )?),
        })
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

/// Environment variable that overrides a remote's stored secret, e.g.
/// `PORTER_SECRET_MY_NAS` for `my-nas` (for scripts, CI and machines without a keychain).
pub fn secret_env_var(remote: &str) -> String {
    format!(
        "PORTER_SECRET_{}",
        remote.to_ascii_uppercase().replace('-', "_")
    )
}

/// [`Registry::target`] without saved remotes.
pub fn target(input: &str) -> Result<Target> {
    Registry::default().target(input)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn secret_env_var_names() {
        assert_eq!(secret_env_var("office"), "PORTER_SECRET_OFFICE");
        assert_eq!(secret_env_var("my-nas_2"), "PORTER_SECRET_MY_NAS_2");
    }

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
