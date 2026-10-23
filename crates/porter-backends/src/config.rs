//! Saved remote profiles, stored as TOML in the OS config directory.
//!
//! ```toml
//! [remotes.nas]
//! type = "sftp"
//! host = "nas.local"
//! user = "me"
//! root = "/volume1"
//!
//! [remotes.backup]
//! type = "s3"
//! bucket = "my-backups"
//! region = "eu-central-1"
//! ```
//!
//! Secrets (passwords, secret keys) never go here; they live in the OS keychain.

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

use porter_core::{Error, Location, Result};
use serde::{Deserialize, Serialize};

/// Overrides the config directory (useful for tests and portable setups).
pub const CONFIG_DIR_ENV: &str = "PORTER_CONFIG_DIR";
const FILE_NAME: &str = "config.toml";

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Config {
    #[serde(default)]
    pub remotes: BTreeMap<String, Remote>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Remote {
    #[serde(flatten)]
    pub backend: Backend,
    /// Directory on the server (or key prefix) that `name:/` maps to.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub root: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "lowercase")]
pub enum Backend {
    /// Authenticates with SSH keys or the SSH agent (no passwords).
    Sftp {
        host: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        port: Option<u16>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        user: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        key_file: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        known_hosts: Option<KnownHosts>,
    },
    /// AWS S3 or any S3-compatible store (MinIO, R2, B2, Wasabi…). Without a stored
    /// secret, credentials come from the environment or `~/.aws`.
    S3 {
        bucket: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        region: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        endpoint: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        access_key_id: Option<String>,
        #[serde(default, skip_serializing_if = "std::ops::Not::not")]
        virtual_host_style: bool,
    },
    Ftp {
        host: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        port: Option<u16>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        user: Option<String>,
        /// FTPS (explicit TLS).
        #[serde(default, skip_serializing_if = "std::ops::Not::not")]
        tls: bool,
    },
}

/// How to treat SSH host keys missing from `known_hosts`.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum KnownHosts {
    /// Refuse unknown hosts.
    #[default]
    Strict,
    /// Add unknown hosts to `known_hosts`.
    Add,
    /// Accept any host key (insecure).
    Accept,
}

impl Backend {
    pub fn kind(&self) -> &'static str {
        match self {
            Self::Sftp { .. } => "sftp",
            Self::S3 { .. } => "s3",
            Self::Ftp { .. } => "ftp",
        }
    }

    /// What the secret stored for this backend is, if it uses one.
    pub fn secret_label(&self) -> Option<&'static str> {
        match self {
            Self::Sftp { .. } => None,
            Self::S3 { .. } => Some("secret access key"),
            Self::Ftp { .. } => Some("password"),
        }
    }

    /// Short human-readable address, e.g. `me@nas.local:2222` or `s3://bucket`.
    pub fn address(&self) -> String {
        let host_port = |host: &str, port: &Option<u16>, user: &Option<String>| {
            let user = user.as_ref().map(|u| format!("{u}@")).unwrap_or_default();
            let port = port.map(|p| format!(":{p}")).unwrap_or_default();
            format!("{user}{host}{port}")
        };
        match self {
            Self::Sftp {
                host, port, user, ..
            } => format!("sftp://{}", host_port(host, port, user)),
            Self::Ftp {
                host,
                port,
                user,
                tls,
            } => {
                let scheme = if *tls { "ftps" } else { "ftp" };
                format!("{scheme}://{}", host_port(host, port, user))
            }
            Self::S3 {
                bucket, endpoint, ..
            } => match endpoint {
                Some(endpoint) => format!("s3://{bucket} @ {endpoint}"),
                None => format!("s3://{bucket}"),
            },
        }
    }
}

impl Config {
    /// The default config file location.
    pub fn default_path() -> Result<PathBuf> {
        let dir = match std::env::var_os(CONFIG_DIR_ENV) {
            Some(dir) => PathBuf::from(dir),
            None => directories::ProjectDirs::from("", "", "porter")
                .ok_or_else(|| Error::backend("config", "no home directory found"))?
                .config_dir()
                .to_owned(),
        };
        Ok(dir.join(FILE_NAME))
    }

    /// Loads the config; a missing file is an empty config.
    pub fn load_from(path: &Path) -> Result<Self> {
        let text = match fs::read_to_string(path) {
            Ok(text) => text,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Self::default()),
            Err(e) => return Err(Error::io(path.display().to_string(), e)),
        };
        let config: Self =
            toml::from_str(&text).map_err(|e| Error::backend(path.display().to_string(), e))?;
        if let Some(name) = config
            .remotes
            .keys()
            .find(|n| !Location::is_valid_remote_name(n))
        {
            return Err(Error::backend(
                path.display().to_string(),
                format!("invalid remote name `{name}`"),
            ));
        }
        Ok(config)
    }

    /// Saves atomically (write to a temporary file, then rename).
    pub fn save_to(&self, path: &Path) -> Result<()> {
        let io = |e| Error::io(path.display().to_string(), e);
        if let Some(dir) = path.parent() {
            fs::create_dir_all(dir).map_err(io)?;
        }
        let text = toml::to_string_pretty(self).map_err(|e| Error::backend("config", e))?;
        let tmp = path.with_extension("toml.tmp");
        fs::write(&tmp, text).map_err(io)?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(&tmp, fs::Permissions::from_mode(0o600)).map_err(io)?;
        }
        fs::rename(&tmp, path).map_err(io)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample() -> Config {
        let mut config = Config::default();
        config.remotes.insert(
            "nas".into(),
            Remote {
                backend: Backend::Sftp {
                    host: "nas.local".into(),
                    port: Some(2222),
                    user: Some("me".into()),
                    key_file: None,
                    known_hosts: Some(KnownHosts::Add),
                },
                root: Some("/volume1".into()),
            },
        );
        config.remotes.insert(
            "minio".into(),
            Remote {
                backend: Backend::S3 {
                    bucket: "photos".into(),
                    region: Some("us-east-1".into()),
                    endpoint: Some("http://localhost:9000".into()),
                    access_key_id: Some("AKIA".into()),
                    virtual_host_style: false,
                },
                root: None,
            },
        );
        config.remotes.insert(
            "ftp1".into(),
            Remote {
                backend: Backend::Ftp {
                    host: "ftp.example.com".into(),
                    port: None,
                    user: None,
                    tls: true,
                },
                root: None,
            },
        );
        config
    }

    #[test]
    fn round_trips_through_toml() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("sub/config.toml");
        sample().save_to(&path).unwrap();

        let text = fs::read_to_string(&path).unwrap();
        assert!(text.contains("[remotes.nas]\ntype = \"sftp\""), "{text}");
        assert!(!text.contains("virtual_host_style"), "defaults are omitted");
        assert_eq!(Config::load_from(&path).unwrap(), sample());
    }

    #[test]
    fn missing_file_is_empty() {
        let dir = tempfile::tempdir().unwrap();
        let config = Config::load_from(&dir.path().join("none.toml")).unwrap();
        assert!(config.remotes.is_empty());
    }

    #[test]
    fn rejects_invalid_names_and_types() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.toml");
        fs::write(&path, "[remotes.C]\ntype = \"ftp\"\nhost = \"h\"\n").unwrap();
        assert!(Config::load_from(&path).is_err());
        fs::write(&path, "[remotes.nas]\ntype = \"gopher\"\n").unwrap();
        assert!(Config::load_from(&path).is_err());
    }

    #[test]
    fn addresses() {
        let config = sample();
        assert_eq!(
            config.remotes["nas"].backend.address(),
            "sftp://me@nas.local:2222"
        );
        assert_eq!(
            config.remotes["minio"].backend.address(),
            "s3://photos @ http://localhost:9000"
        );
        assert_eq!(
            config.remotes["ftp1"].backend.address(),
            "ftps://ftp.example.com"
        );
    }
}
