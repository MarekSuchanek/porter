use async_trait::async_trait;
use futures::stream::BoxStream;
use serde::Serialize;

use crate::{Entry, FsInfo, Result};

/// Directory listings are streamed so very large directories render progressively.
pub type EntryStream = BoxStream<'static, Result<Entry>>;

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
    pub symlinks: bool,
    pub server_side_copy: bool,
    pub fs_info: bool,
}

/// A file system or object store backend. Paths are backend-native.
#[async_trait]
pub trait Provider: Send + Sync {
    /// URI scheme handled by this provider, e.g. `file` or `s3`.
    fn scheme(&self) -> &str;

    fn capabilities(&self) -> Capabilities;

    /// Metadata of a single entry. Symlinks are reported as themselves, not followed.
    async fn stat(&self, path: &str) -> Result<Entry>;

    /// Entries directly inside the directory at `path`, in no particular order.
    async fn list(&self, path: &str) -> Result<EntryStream>;

    /// Information about the file system holding `path`.
    async fn fs_info(&self, path: &str) -> Result<FsInfo>;
}
