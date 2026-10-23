use std::sync::Arc;

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use futures::stream::BoxStream;
use serde::Serialize;
use tokio::io::{AsyncRead, AsyncWrite};

use crate::{Entry, EntryKind, Error, FsInfo, Result};

/// Directory listings are streamed so very large directories render progressively.
pub type EntryStream = BoxStream<'static, Result<Entry>>;

/// Cheap pre-filter on an entry's name and kind, applied before any metadata is read.
pub type NameFilter = Arc<dyn Fn(&str, EntryKind) -> bool + Send + Sync>;

/// How to walk a tree recursively.
#[derive(Clone, Default)]
pub struct WalkOptions {
    /// Also yield, and descend into, entries whose names start with a dot.
    pub include_hidden: bool,
    /// Levels below the root to visit; `Some(1)` lists only the root's children.
    pub max_depth: Option<usize>,
    /// Only entries passing this are yielded; directories are descended into regardless.
    pub wants: Option<NameFilter>,
}

impl WalkOptions {
    pub fn wants(&self, name: &str, kind: EntryKind) -> bool {
        self.wants.as_ref().is_none_or(|f| f(name, kind))
    }
}

pub type Reader = Box<dyn AsyncRead + Send + Unpin>;
pub type Writer = Box<dyn AsyncWrite + Send + Unpin>;

/// What a backend supports; front ends use it to hide or disable actions.
#[derive(Debug, Clone, Copy, Default, Serialize)]
pub struct Capabilities {
    pub list: bool,
    pub read: bool,
    pub write: bool,
    pub delete: bool,
    pub rename: bool,
    pub mkdir: bool,
    pub set_permissions: bool,
    pub set_owner: bool,
    pub set_modified: bool,
    pub symlinks: bool,
    pub server_side_copy: bool,
    pub fs_info: bool,
}

/// A file system or object store backend. Paths are backend-native.
///
/// Operations a backend cannot perform keep their default implementation, which
/// returns [`Error::Unsupported`]; [`Capabilities`] tells callers up front.
#[async_trait]
pub trait Provider: Send + Sync {
    /// URI scheme handled by this provider, e.g. `file` or `s3`.
    fn scheme(&self) -> &str;

    fn capabilities(&self) -> Capabilities;

    /// Joins a child name onto a directory path.
    fn join(&self, dir: &str, name: &str) -> String {
        match dir.ends_with('/') {
            true => format!("{dir}{name}"),
            false => format!("{dir}/{name}"),
        }
    }

    /// The parent directory, or `None` for a root.
    fn parent(&self, path: &str) -> Option<String> {
        let trimmed = path.trim_end_matches('/');
        let (parent, _) = trimmed.rsplit_once('/')?;
        Some(if parent.is_empty() { "/" } else { parent }.to_owned())
    }

    fn file_name(&self, path: &str) -> Option<String> {
        let trimmed = path.trim_end_matches('/');
        let name = trimmed.rsplit('/').next()?;
        (!name.is_empty()).then(|| name.to_owned())
    }

    /// Metadata of a single entry. Symlinks are reported as themselves, not followed.
    async fn stat(&self, path: &str) -> Result<Entry>;

    /// Like [`stat`](Self::stat), but follows symlinks to their target.
    async fn stat_follow(&self, path: &str) -> Result<Entry> {
        self.stat(path).await
    }

    /// Entries directly inside the directory at `path`, in no particular order.
    async fn list(&self, path: &str) -> Result<EntryStream>;

    /// Information about the file system holding `path`.
    async fn fs_info(&self, path: &str) -> Result<FsInfo>;

    /// Every entry below `path` (not `path` itself), in no particular order, if the
    /// backend has a faster way than listing directory by directory (the local file
    /// system walks in parallel). `None` means callers should walk with [`list`](Self::list).
    async fn walk(&self, _path: &str, _opts: &WalkOptions) -> Result<Option<EntryStream>> {
        Ok(None)
    }

    async fn read(&self, _path: &str) -> Result<Reader> {
        Err(Error::unsupported("read", self.scheme()))
    }

    /// Opens a file for writing. Fails with [`Error::AlreadyExists`] unless `overwrite`.
    /// Callers must `shutdown()` the writer to complete the write.
    async fn write(&self, _path: &str, _overwrite: bool) -> Result<Writer> {
        Err(Error::unsupported("write", self.scheme()))
    }

    /// Creates a single directory; the parent must exist.
    async fn mkdir(&self, _path: &str) -> Result<()> {
        Err(Error::unsupported("mkdir", self.scheme()))
    }

    /// Deletes a file, a symlink or an empty directory.
    async fn delete(&self, _path: &str) -> Result<()> {
        Err(Error::unsupported("delete", self.scheme()))
    }

    /// Renames within this backend, replacing an existing file at `to`.
    async fn rename(&self, _from: &str, _to: &str) -> Result<()> {
        Err(Error::unsupported("rename", self.scheme()))
    }

    /// Sets POSIX permission bits (backends without them map what they can).
    async fn set_permissions(&self, _path: &str, _mode: u32) -> Result<()> {
        Err(Error::unsupported("set_permissions", self.scheme()))
    }

    async fn set_modified(&self, _path: &str, _time: DateTime<Utc>) -> Result<()> {
        Err(Error::unsupported("set_modified", self.scheme()))
    }

    /// Creates a symlink at `path` pointing to `target`.
    async fn symlink(&self, _target: &str, _path: &str) -> Result<()> {
        Err(Error::unsupported("symlink", self.scheme()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Dummy;

    #[async_trait]
    impl Provider for Dummy {
        fn scheme(&self) -> &str {
            "dummy"
        }
        fn capabilities(&self) -> Capabilities {
            Capabilities::default()
        }
        async fn stat(&self, _: &str) -> Result<Entry> {
            unimplemented!()
        }
        async fn list(&self, _: &str) -> Result<EntryStream> {
            unimplemented!()
        }
        async fn fs_info(&self, _: &str) -> Result<FsInfo> {
            unimplemented!()
        }
    }

    #[test]
    fn default_path_helpers() {
        assert_eq!(Dummy.join("/a", "b"), "/a/b");
        assert_eq!(Dummy.join("/", "b"), "/b");
        assert_eq!(Dummy.parent("/a/b/").as_deref(), Some("/a"));
        assert_eq!(Dummy.parent("/a").as_deref(), Some("/"));
        assert_eq!(Dummy.parent("/"), None);
        assert_eq!(Dummy.file_name("/a/b.txt").as_deref(), Some("b.txt"));
        assert_eq!(Dummy.file_name("/"), None);
    }
}
