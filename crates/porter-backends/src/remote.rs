//! Remote backends (SFTP, S3, FTP) built on Apache OpenDAL.
//!
//! Porter paths are `/`-rooted (`/photos/a.jpg`); OpenDAL paths are relative to the
//! operator root, with directories ending in `/` (`photos/`). Object stores have no
//! real directories, so a directory may exist only as a key prefix.

use std::time::SystemTime;

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use futures::StreamExt;
use opendal::{EntryMode, ErrorKind, Metadata, Operator, services};
use porter_core::{
    Capabilities, Entry, EntryKind, EntryStream, Error, FsInfo, Location, Provider, Reader, Result,
    Writer,
};
use tokio_util::compat::{FuturesAsyncReadCompatExt, FuturesAsyncWriteCompatExt};

#[cfg(unix)]
use crate::config::KnownHosts;
use crate::config::{Backend, Remote};

/// Chunk size for transfers; S3 multipart uploads need at least 5 MiB parts.
const CHUNK_SIZE: usize = 8 << 20;
/// Chunks in flight per file.
const CONCURRENT_CHUNKS: usize = 4;

pub struct OpendalProvider {
    scheme: String,
    /// Remote name, bucket or host, for display.
    name: String,
    /// Prefix that turns a path into a user-facing location, e.g. `nas:`.
    label: String,
    op: Operator,
}

impl OpendalProvider {
    pub fn new(scheme: impl Into<String>, name: impl Into<String>, op: Operator) -> Self {
        let name = name.into();
        Self {
            scheme: scheme.into(),
            label: format!("{name}:"),
            name,
            op,
        }
    }

    /// A provider for a saved remote; `secret` is its password or secret key.
    pub fn from_remote(name: &str, remote: &Remote, secret: Option<&str>) -> Result<Self> {
        let root = remote.root.as_deref().unwrap_or("/");
        let op = match &remote.backend {
            #[cfg(not(unix))]
            Backend::Sftp { .. } => {
                return Err(Error::backend(
                    name,
                    "SFTP is not supported on Windows yet (it needs the system ssh client)",
                ));
            }
            #[cfg(unix)]
            Backend::Sftp {
                host,
                port,
                user,
                key_file,
                known_hosts,
            } => {
                let mut builder = services::Sftp::default()
                    .endpoint(&ssh_endpoint(host, *port, user.as_deref()))
                    .root(root)
                    .known_hosts_strategy(match known_hosts.unwrap_or_default() {
                        KnownHosts::Strict => "strict",
                        KnownHosts::Add => "add",
                        KnownHosts::Accept => "accept",
                    });
                if let Some(user) = user {
                    builder = builder.user(user);
                }
                if let Some(key) = key_file {
                    builder = builder.key(&expand_home(key));
                }
                Operator::new(builder)
            }
            Backend::S3 {
                bucket,
                region,
                endpoint,
                access_key_id,
                virtual_host_style,
            } => {
                let mut builder = services::S3::default().bucket(bucket).root(root);
                // S3-compatible servers (MinIO, …) accept any region.
                let region = region.as_deref().or(endpoint.as_ref().map(|_| "us-east-1"));
                if let Some(region) = region {
                    builder = builder.region(region);
                }
                if let Some(endpoint) = endpoint {
                    builder = builder.endpoint(endpoint);
                }
                if let (Some(id), Some(secret)) = (access_key_id, secret) {
                    builder = builder.access_key_id(id).secret_access_key(secret);
                }
                if *virtual_host_style {
                    builder = builder.enable_virtual_host_style();
                }
                Operator::new(builder)
            }
            Backend::Ftp {
                host,
                port,
                user,
                tls,
            } => {
                let scheme = if *tls { "ftps" } else { "ftp" };
                let port = port.map(|p| format!(":{p}")).unwrap_or_default();
                let mut builder = services::Ftp::default()
                    .endpoint(&format!("{scheme}://{host}{port}"))
                    .root(root);
                if let Some(user) = user {
                    builder = builder.user(user);
                }
                if let Some(password) = secret {
                    builder = builder.password(password);
                }
                Operator::new(builder)
            }
            Backend::Smb { .. } | Backend::Nfs { .. } => {
                return Err(Error::backend(name, "not an OpenDAL backend"));
            }
        }
        .map_err(|e| match e.kind() {
            ErrorKind::ConfigInvalid if e.to_string().contains("region is missing") => {
                Error::backend(
                    name,
                    "S3 region unknown: set AWS_REGION or save the bucket with \
                     `porter remote add s3`",
                )
            }
            _ => Error::backend(name, e),
        })?;
        Ok(Self::new(remote.backend.kind(), name, op))
    }

    /// Asks AWS which region a bucket is in.
    pub async fn detect_s3_region(bucket: &str, endpoint: Option<&str>) -> Option<String> {
        services::S3::detect_region(endpoint.unwrap_or("https://s3.amazonaws.com"), bucket).await
    }

    /// A provider for an ad-hoc URL such as `sftp://me@host:2222/…`, `ftp://host/…` or
    /// `s3://bucket/…` (credentials from the SSH agent, anonymous FTP, or the AWS
    /// environment).
    pub fn from_url(location: &Location) -> Result<Self> {
        let (user, host_port) = match location.authority.split_once('@') {
            Some((user, rest)) => (Some(user.to_owned()), rest),
            None => (None, location.authority.as_str()),
        };
        let (host, port) = match host_port.rsplit_once(':') {
            Some((host, port)) => (host, port.parse().ok()),
            None => (host_port, None),
        };
        let backend = match location.scheme.as_str() {
            "sftp" => Backend::Sftp {
                host: host.to_owned(),
                port,
                user,
                key_file: None,
                known_hosts: None,
            },
            "ftp" | "ftps" => Backend::Ftp {
                host: host.to_owned(),
                port,
                user,
                tls: location.scheme == "ftps",
            },
            "s3" => Backend::S3 {
                bucket: host.to_owned(),
                region: None,
                endpoint: None,
                access_key_id: None,
                virtual_host_style: false,
            },
            "smb" | "nfs" => {
                return Err(Error::invalid_location(
                    location.to_string(),
                    format!(
                        "save the share first with `porter remote add {}` and use NAME:/path",
                        location.scheme
                    ),
                ));
            }
            other => {
                return Err(Error::invalid_location(
                    location.to_string(),
                    format!("no backend for scheme `{other}`"),
                ));
            }
        };
        let remote = Remote {
            backend,
            root: None,
        };
        let mut provider = Self::from_remote(&location.authority, &remote, None)?;
        provider.scheme = location.scheme.clone();
        provider.label = format!("{}://{}", location.scheme, location.authority);
        Ok(provider)
    }

    /// `path` as the user addresses it, for error messages.
    fn at(&self, path: &str) -> String {
        format!("{}{path}", self.label)
    }

    async fn exists(&self, path: &str) -> Result<bool> {
        match self.stat(path).await {
            Ok(_) => Ok(true),
            Err(Error::NotFound(_)) => Ok(false),
            Err(e) => Err(e),
        }
    }
}

#[async_trait]
impl Provider for OpendalProvider {
    fn scheme(&self) -> &str {
        &self.scheme
    }

    fn capabilities(&self) -> Capabilities {
        let cap = self.op.info().capability();
        Capabilities {
            list: cap.list,
            read: cap.read,
            write: cap.write,
            delete: cap.delete,
            rename: cap.rename,
            mkdir: cap.create_dir,
            server_side_copy: cap.copy,
            fs_info: true,
            ..Default::default()
        }
    }

    async fn stat(&self, path: &str) -> Result<Entry> {
        let file = op_path(path);
        if file == "/" {
            return Ok(dir_entry(path));
        }
        match self.op.stat(&file).await {
            Ok(meta) => return Ok(to_entry(path, &meta)),
            Err(e) if e.kind() != ErrorKind::NotFound => return Err(map_err(&self.at(path), e)),
            Err(_) => {}
        }
        // Not a file: try a directory marker, then a bare key prefix.
        let dir = dir_path(path);
        match self.op.stat(&dir).await {
            Ok(meta) => return Ok(to_entry(path, &meta)),
            Err(e) if e.kind() != ErrorKind::NotFound => return Err(map_err(&self.at(path), e)),
            Err(_) => {}
        }
        match self.op.list_with(&dir).limit(1).await {
            Ok(children) if !children.is_empty() => Ok(dir_entry(path)),
            Ok(_) => Err(Error::NotFound(self.at(path))),
            Err(e) => Err(map_err(&self.at(path), e)),
        }
    }

    async fn list(&self, path: &str) -> Result<EntryStream> {
        let dir = dir_path(path);
        let lister = self
            .op
            .lister(&dir)
            .await
            .map_err(|e| map_err(&self.at(path), e))?;
        let context = self.at(path);
        Ok(lister
            .filter_map(move |item| {
                let item = match item {
                    // Some services list the directory itself.
                    Ok(entry) if entry.path() == dir => None,
                    Ok(entry) => {
                        let (p, meta) = entry.into_parts();
                        let full = format!("/{}", p.trim_end_matches('/'));
                        Some(Ok(to_entry(&full, &meta)))
                    }
                    Err(e) => Some(Err(map_err(&context, e))),
                };
                std::future::ready(item)
            })
            .boxed())
    }

    async fn check(&self) -> Result<()> {
        self.op
            .check()
            .await
            .map_err(|e| Error::backend(&self.name, e))
    }

    async fn fs_info(&self, _path: &str) -> Result<FsInfo> {
        Ok(FsInfo {
            name: Some(self.name.clone()),
            fs_type: Some(self.scheme.clone()),
            ..Default::default()
        })
    }

    async fn read(&self, path: &str) -> Result<Reader> {
        let reader = self
            .op
            .reader_with(&op_path(path))
            .chunk(CHUNK_SIZE)
            .concurrent(CONCURRENT_CHUNKS)
            .await
            .map_err(|e| map_err(&self.at(path), e))?
            .into_futures_async_read(..)
            .await
            .map_err(|e| map_err(&self.at(path), e))?;
        Ok(Box::new(reader.compat()))
    }

    async fn write(&self, path: &str, overwrite: bool) -> Result<Writer> {
        // Not atomic, but no backend here offers a portable create-if-absent.
        if !overwrite && self.exists(path).await? {
            return Err(Error::AlreadyExists(self.at(path)));
        }
        let writer = self
            .op
            .writer_with(&op_path(path))
            .chunk(CHUNK_SIZE)
            .concurrent(CONCURRENT_CHUNKS)
            .await
            .map_err(|e| map_err(&self.at(path), e))?
            .into_futures_async_write();
        Ok(Box::new(writer.compat_write()))
    }

    async fn mkdir(&self, path: &str) -> Result<()> {
        if self.exists(path).await? {
            return Err(Error::AlreadyExists(self.at(path)));
        }
        self.op
            .create_dir(&dir_path(path))
            .await
            .map_err(|e| map_err(&self.at(path), e))
    }

    async fn delete(&self, path: &str) -> Result<()> {
        let target = match self.stat(path).await?.kind {
            EntryKind::Dir => dir_path(path),
            _ => op_path(path),
        };
        self.op
            .delete(&target)
            .await
            .map_err(|e| map_err(&self.at(path), e))
    }

    async fn rename(&self, from: &str, to: &str) -> Result<()> {
        match self.op.rename(&op_path(from), &op_path(to)).await {
            Ok(()) => Ok(()),
            // e.g. directories, or a service without rename: let callers copy instead.
            Err(e) if matches!(e.kind(), ErrorKind::Unsupported | ErrorKind::IsADirectory) => {
                Err(Error::CrossDevice(self.at(from)))
            }
            Err(e) => Err(map_err(&self.at(from), e)),
        }
    }
}

/// `/a/b` → `a/b`; the root stays `/`.
fn op_path(path: &str) -> String {
    match path.trim_matches('/') {
        "" => "/".to_owned(),
        p => p.to_owned(),
    }
}

/// `/a/b` → `a/b/`; the root stays `/`.
fn dir_path(path: &str) -> String {
    match op_path(path) {
        root if root == "/" => root,
        p => format!("{p}/"),
    }
}

fn name_of(path: &str) -> String {
    match path.trim_end_matches('/').rsplit('/').next() {
        Some(name) if !name.is_empty() => name.to_owned(),
        _ => "/".to_owned(),
    }
}

fn dir_entry(path: &str) -> Entry {
    new_entry(path, EntryKind::Dir)
}

fn to_entry(path: &str, meta: &Metadata) -> Entry {
    let kind = match meta.mode() {
        EntryMode::FILE => EntryKind::File,
        EntryMode::DIR => EntryKind::Dir,
        EntryMode::Unknown => EntryKind::Other,
    };
    Entry {
        size: (kind == EntryKind::File).then(|| meta.content_length()),
        modified: meta
            .last_modified()
            .map(|t| DateTime::<Utc>::from(SystemTime::from(t))),
        ..new_entry(path, kind)
    }
}

fn new_entry(path: &str, kind: EntryKind) -> Entry {
    let name = name_of(path);
    Entry {
        hidden: name.starts_with('.'),
        path: match path.trim_end_matches('/') {
            "" => "/".to_owned(),
            p => p.to_owned(),
        },
        name,
        kind,
        size: None,
        modified: None,
        created: None,
        accessed: None,
        permissions: None,
        symlink_target: None,
    }
}

fn map_err(path: &str, e: opendal::Error) -> Error {
    match e.kind() {
        ErrorKind::NotFound => Error::NotFound(path.to_owned()),
        ErrorKind::AlreadyExists => Error::AlreadyExists(path.to_owned()),
        ErrorKind::PermissionDenied => Error::PermissionDenied(path.to_owned()),
        _ => Error::backend(path, e),
    }
}

/// `ssh://[user@]host[:port]`, the endpoint format `openssh` expects.
#[cfg(unix)]
fn ssh_endpoint(host: &str, port: Option<u16>, user: Option<&str>) -> String {
    let user = user.map(|u| format!("{u}@")).unwrap_or_default();
    let port = port.map(|p| format!(":{p}")).unwrap_or_default();
    format!("ssh://{user}{host}{port}")
}

#[cfg(unix)]
fn expand_home(path: &str) -> String {
    match (path.strip_prefix("~/"), directories::BaseDirs::new()) {
        (Some(rest), Some(dirs)) => dirs.home_dir().join(rest).display().to_string(),
        _ => path.to_owned(),
    }
}

#[cfg(test)]
mod tests {
    use futures::TryStreamExt;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    use super::*;

    fn memory() -> OpendalProvider {
        OpendalProvider::new(
            "memory",
            "test",
            Operator::new(services::Memory::default()).unwrap(),
        )
    }

    async fn put(p: &OpendalProvider, path: &str, data: &str) {
        let mut w = p.write(path, false).await.unwrap();
        w.write_all(data.as_bytes()).await.unwrap();
        w.shutdown().await.unwrap();
    }

    async fn names(p: &OpendalProvider, path: &str) -> Vec<String> {
        let mut names: Vec<String> = p
            .list(path)
            .await
            .unwrap()
            .map_ok(|e| e.name)
            .try_collect()
            .await
            .unwrap();
        names.sort();
        names
    }

    #[test]
    fn path_mapping() {
        assert_eq!(op_path("/"), "/");
        assert_eq!(op_path("/a/b/"), "a/b");
        assert_eq!(dir_path("/a/b"), "a/b/");
        assert_eq!(dir_path("/"), "/");
        assert_eq!(name_of("/a/b/"), "b");
        assert_eq!(name_of("/"), "/");
        #[cfg(unix)]
        assert_eq!(ssh_endpoint("h", Some(22), Some("me")), "ssh://me@h:22");
    }

    #[tokio::test]
    async fn write_read_stat_list() {
        let p = memory();
        put(&p, "/docs/a.txt", "hello").await;
        put(&p, "/docs/deep/b.txt", "b").await;
        assert!(matches!(
            p.write("/docs/a.txt", false).await,
            Err(Error::AlreadyExists(_))
        ));

        let file = p.stat("/docs/a.txt").await.unwrap();
        assert_eq!((file.kind, file.size), (EntryKind::File, Some(5)));
        assert_eq!(file.path, "/docs/a.txt");

        // `docs/` exists only as a prefix of its objects.
        let dir = p.stat("/docs").await.unwrap();
        assert_eq!((dir.kind, dir.name.as_str()), (EntryKind::Dir, "docs"));
        assert_eq!(p.stat("/").await.unwrap().kind, EntryKind::Dir);
        assert!(matches!(p.stat("/nope").await, Err(Error::NotFound(_))));

        assert_eq!(names(&p, "/docs").await, ["a.txt", "deep"]);
        assert_eq!(names(&p, "/").await, ["docs"]);

        let mut content = String::new();
        p.read("/docs/a.txt")
            .await
            .unwrap()
            .read_to_string(&mut content)
            .await
            .unwrap();
        assert_eq!(content, "hello");
    }

    #[tokio::test]
    async fn mkdir_and_delete() {
        let p = memory();
        p.mkdir("/empty").await.unwrap();
        assert_eq!(p.stat("/empty").await.unwrap().kind, EntryKind::Dir);
        assert!(matches!(
            p.mkdir("/empty").await,
            Err(Error::AlreadyExists(_))
        ));
        p.delete("/empty").await.unwrap();
        assert!(matches!(p.stat("/empty").await, Err(Error::NotFound(_))));

        put(&p, "/f.txt", "x").await;
        p.delete("/f.txt").await.unwrap();
        assert!(matches!(p.stat("/f.txt").await, Err(Error::NotFound(_))));
    }

    #[test]
    fn urls_build_providers() {
        let urls: &[&str] = if cfg!(unix) {
            &[
                "sftp://me@host:2222/x",
                "ftp://host/x",
                "ftps://u@host:990/",
            ]
        } else {
            &["ftp://host/x", "ftps://u@host:990/"]
        };
        for &url in urls {
            let loc = Location::parse(url).unwrap();
            let provider = OpendalProvider::from_url(&loc).unwrap();
            assert_eq!(provider.scheme(), loc.scheme, "{url}");
        }
        // Region comes from the environment, which tests can't rely on.
        if std::env::var_os("AWS_REGION").is_none() {
            let s3 = Location::parse("s3://bucket/x").unwrap();
            let err = OpendalProvider::from_url(&s3).err().unwrap();
            assert!(err.to_string().contains("AWS_REGION"), "{err}");
        }
        let gopher = Location::parse("gopher://host/x").unwrap();
        assert!(OpendalProvider::from_url(&gopher).is_err());
    }
}
