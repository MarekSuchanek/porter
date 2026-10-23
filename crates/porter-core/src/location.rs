use std::fmt;

use percent_encoding::percent_decode_str;
use url::Url;

use crate::{Error, Result};

/// A place on some backend: a plain OS path, a saved remote such as `nas:/photos`,
/// or a URI such as `sftp://user@host:22/home/user` or `s3://bucket/some/prefix`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Location {
    /// Backend scheme; `file` for the local file system, `remote` for saved remotes.
    pub scheme: String,
    /// `user@host:port`, bucket, remote name…; empty for local paths.
    pub authority: String,
    /// Backend-native path (an OS path for `file`, `/`-separated otherwise).
    pub path: String,
}

impl Location {
    pub const LOCAL_SCHEME: &'static str = "file";
    pub const REMOTE_SCHEME: &'static str = "remote";

    pub fn local(path: impl Into<String>) -> Self {
        Self {
            scheme: Self::LOCAL_SCHEME.to_owned(),
            authority: String::new(),
            path: path.into(),
        }
    }

    /// Remote names start with a letter and are at least two characters long, so
    /// they never collide with Windows drive letters (`C:`).
    pub fn is_valid_remote_name(name: &str) -> bool {
        let mut chars = name.chars();
        chars.next().is_some_and(|c| c.is_ascii_alphabetic())
            && name.len() >= 2
            && chars.all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
    }

    pub fn parse(input: &str) -> Result<Self> {
        if input.is_empty() {
            return Err(Error::invalid_location(input, "empty location"));
        }
        if !input.contains("://") {
            // `name:path` addresses a saved remote; use `./name:path` for such a local file.
            if let Some((name, path)) = input.split_once(':')
                && Self::is_valid_remote_name(name)
            {
                return Ok(Self {
                    scheme: Self::REMOTE_SCHEME.to_owned(),
                    authority: name.to_owned(),
                    path: format!("/{}", path.trim_start_matches('/')),
                });
            }
            // Anything else is an OS path; this keeps `C:\dir` working on Windows.
            return Ok(Self::local(input));
        }

        let url = Url::parse(input).map_err(|e| Error::invalid_location(input, e.to_string()))?;
        if url.scheme() == Self::LOCAL_SCHEME {
            let path = url
                .to_file_path()
                .map_err(|()| Error::invalid_location(input, "not a local file URL"))?;
            return Ok(Self::local(path.to_string_lossy()));
        }

        let host = url
            .host_str()
            .ok_or_else(|| Error::invalid_location(input, "missing host"))?;
        let mut authority = String::new();
        if !url.username().is_empty() {
            authority.push_str(url.username());
            authority.push('@');
        }
        authority.push_str(host);
        if let Some(port) = url.port() {
            authority.push_str(&format!(":{port}"));
        }
        let path = percent_decode_str(url.path())
            .decode_utf8()
            .map_err(|e| Error::invalid_location(input, e.to_string()))?;
        let path = format!("/{}", path.trim_start_matches('/'));

        Ok(Self {
            scheme: url.scheme().to_owned(),
            authority,
            path,
        })
    }

    pub fn is_local(&self) -> bool {
        self.scheme == Self::LOCAL_SCHEME
    }

    pub fn is_remote_alias(&self) -> bool {
        self.scheme == Self::REMOTE_SCHEME
    }
}

impl fmt::Display for Location {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self.scheme.as_str() {
            Self::LOCAL_SCHEME => f.write_str(&self.path),
            Self::REMOTE_SCHEME => write!(f, "{}:{}", self.authority, self.path),
            scheme => write!(f, "{scheme}://{}{}", self.authority, self.path),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn remote_aliases() {
        let loc = Location::parse("nas:/photos/2024").unwrap();
        assert!(loc.is_remote_alias());
        assert_eq!(
            (loc.authority.as_str(), loc.path.as_str()),
            ("nas", "/photos/2024")
        );
        assert_eq!(loc.to_string(), "nas:/photos/2024");

        assert_eq!(Location::parse("my-s3:").unwrap().path, "/");
        assert_eq!(Location::parse("my_s3:a/b").unwrap().path, "/a/b");
        assert!(Location::parse("./nas:file").unwrap().is_local());
    }

    #[test]
    fn remote_names() {
        for name in ["nas", "my-s3", "b2_backup", "x1"] {
            assert!(Location::is_valid_remote_name(name), "{name}");
        }
        for name in ["", "C", "1nas", "my nas", "a.b", "nas/x"] {
            assert!(!Location::is_valid_remote_name(name), "{name}");
        }
    }

    #[test]
    fn url_without_path_is_root() {
        assert_eq!(Location::parse("s3://bucket").unwrap().path, "/");
    }

    #[test]
    fn plain_paths_are_local() {
        for input in [
            "/home/user",
            "relative/dir",
            ".",
            r"C:\Users\me",
            "C:/Users/me",
        ] {
            let loc = Location::parse(input).unwrap();
            assert!(loc.is_local());
            assert_eq!(loc.path, input);
        }
    }

    #[cfg(unix)]
    #[test]
    fn file_url_is_local() {
        let loc = Location::parse("file:///tmp/a%20b").unwrap();
        assert!(loc.is_local());
        assert_eq!(loc.path, "/tmp/a b");
    }

    #[test]
    fn remote_url() {
        let loc = Location::parse("sftp://me@example.com:2222/home/me/My%20Docs").unwrap();
        assert_eq!(loc.scheme, "sftp");
        assert_eq!(loc.authority, "me@example.com:2222");
        assert_eq!(loc.path, "/home/me/My Docs");
        assert_eq!(
            loc.to_string(),
            "sftp://me@example.com:2222/home/me/My Docs"
        );
    }

    #[test]
    fn bucket_url() {
        let loc = Location::parse("s3://my-bucket/photos/2024").unwrap();
        assert_eq!(loc.scheme, "s3");
        assert_eq!(loc.authority, "my-bucket");
        assert_eq!(loc.path, "/photos/2024");
    }

    #[test]
    fn invalid_inputs() {
        assert!(Location::parse("").is_err());
        assert!(Location::parse("sftp:///no-host").is_err());
    }
}
