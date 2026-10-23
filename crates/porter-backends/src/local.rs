//! The local file system (any disk mounted on this machine).

use std::fs::{self, Metadata};
use std::path::{Path, PathBuf};

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use futures::StreamExt;
use porter_core::{
    Capabilities, Entry, EntryKind, EntryStream, Error, FsInfo, Location, Permissions, Provider,
    Result,
};
use sysinfo::Disks;
use tokio::sync::mpsc;
use tokio::task;

/// Entries buffered between the blocking directory reader and the consumer.
const LIST_BUFFER: usize = 256;

#[derive(Debug, Default, Clone, Copy)]
pub struct LocalProvider;

#[async_trait]
impl Provider for LocalProvider {
    fn scheme(&self) -> &str {
        Location::LOCAL_SCHEME
    }

    fn capabilities(&self) -> Capabilities {
        Capabilities {
            list: true,
            fs_info: true,
            ..Default::default()
        }
    }

    async fn stat(&self, path: &str) -> Result<Entry> {
        let path = path.to_owned();
        blocking(move || {
            let abs = absolute(&path)?;
            let meta = fs::symlink_metadata(&abs).map_err(|e| Error::io(&path, e))?;
            Ok(to_entry(&abs, &meta))
        })
        .await
    }

    async fn list(&self, path: &str) -> Result<EntryStream> {
        let path = path.to_owned();
        let (dir, read_dir) = blocking(move || {
            let abs = absolute(&path)?;
            let read_dir = fs::read_dir(&abs).map_err(|e| Error::io(&path, e))?;
            Ok((abs, read_dir))
        })
        .await?;

        // Read on a blocking thread and stream entries out through a bounded channel,
        // so huge directories never have to be held in memory at once.
        let (tx, mut rx) = mpsc::channel(LIST_BUFFER);
        task::spawn_blocking(move || {
            for dir_entry in read_dir {
                let item = dir_entry
                    .and_then(|d| Ok((d.path(), d.metadata()?)))
                    .map(|(p, meta)| to_entry(&p, &meta))
                    .map_err(|e| Error::io(dir.display().to_string(), e));
                if tx.blocking_send(item).is_err() {
                    break; // consumer went away
                }
            }
        });
        Ok(futures::stream::poll_fn(move |cx| rx.poll_recv(cx)).boxed())
    }

    async fn fs_info(&self, path: &str) -> Result<FsInfo> {
        let path = path.to_owned();
        blocking(move || {
            let abs = dunce::canonicalize(&path).map_err(|e| Error::io(&path, e))?;
            let disks = Disks::new_with_refreshed_list();
            // The disk whose mount point is the longest prefix of the path holds it.
            disks
                .iter()
                .filter(|d| abs.starts_with(d.mount_point()))
                .max_by_key(|d| d.mount_point().components().count())
                .map(to_fs_info)
                .ok_or_else(|| Error::NotFound(format!("{path}: no mounted file system")))
        })
        .await
    }
}

/// All file systems currently mounted on this machine, sorted by mount point.
pub async fn disks() -> Result<Vec<FsInfo>> {
    blocking(|| {
        let disks = Disks::new_with_refreshed_list();
        let mut infos: Vec<_> = disks.iter().map(to_fs_info).collect();
        infos.sort_by(|a, b| a.mount_point.cmp(&b.mount_point));
        Ok(infos)
    })
    .await
}

async fn blocking<T, F>(f: F) -> Result<T>
where
    F: FnOnce() -> Result<T> + Send + 'static,
    T: Send + 'static,
{
    task::spawn_blocking(f)
        .await
        .expect("blocking file system task panicked")
}

fn absolute(path: &str) -> Result<PathBuf> {
    std::path::absolute(path).map_err(|e| Error::io(path, e))
}

fn to_entry(path: &Path, meta: &Metadata) -> Entry {
    let file_type = meta.file_type();
    let kind = if file_type.is_symlink() {
        EntryKind::Symlink
    } else if file_type.is_dir() {
        EntryKind::Dir
    } else if file_type.is_file() {
        EntryKind::File
    } else {
        EntryKind::Other
    };
    let name = path
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| path.display().to_string());
    let symlink_target = (kind == EntryKind::Symlink)
        .then(|| fs::read_link(path).ok())
        .flatten()
        .map(|t| t.display().to_string());

    Entry {
        hidden: is_hidden(&name, meta),
        name,
        path: path.display().to_string(),
        kind,
        size: (kind != EntryKind::Dir).then_some(meta.len()),
        modified: meta.modified().ok().map(DateTime::<Utc>::from),
        created: meta.created().ok().map(DateTime::<Utc>::from),
        accessed: meta.accessed().ok().map(DateTime::<Utc>::from),
        permissions: Some(permissions(meta)),
        symlink_target,
    }
}

#[cfg(unix)]
fn permissions(meta: &Metadata) -> Permissions {
    use std::os::unix::fs::MetadataExt;
    Permissions {
        mode: Some(meta.mode() & 0o7777),
        readonly: meta.permissions().readonly(),
        uid: Some(meta.uid()),
        gid: Some(meta.gid()),
    }
}

#[cfg(not(unix))]
fn permissions(meta: &Metadata) -> Permissions {
    Permissions {
        readonly: meta.permissions().readonly(),
        ..Default::default()
    }
}

#[cfg(windows)]
fn is_hidden(_name: &str, meta: &Metadata) -> bool {
    use std::os::windows::fs::MetadataExt;
    const FILE_ATTRIBUTE_HIDDEN: u32 = 0x2;
    meta.file_attributes() & FILE_ATTRIBUTE_HIDDEN != 0
}

#[cfg(not(windows))]
fn is_hidden(name: &str, _meta: &Metadata) -> bool {
    name.starts_with('.')
}

fn to_fs_info(disk: &sysinfo::Disk) -> FsInfo {
    FsInfo {
        name: Some(disk.name().to_string_lossy().into_owned()).filter(|n| !n.is_empty()),
        fs_type: Some(disk.file_system().to_string_lossy().into_owned()),
        mount_point: Some(disk.mount_point().display().to_string()),
        total_bytes: Some(disk.total_space()),
        available_bytes: Some(disk.available_space()),
        read_only: Some(disk.is_read_only()),
        removable: Some(disk.is_removable()),
    }
}

#[cfg(test)]
mod tests {
    use futures::TryStreamExt;

    use super::*;

    fn fixture() -> tempfile::TempDir {
        let dir = tempfile::tempdir().unwrap();
        fs::write(dir.path().join("a.txt"), b"hello").unwrap();
        fs::write(dir.path().join(".hidden"), b"").unwrap();
        fs::create_dir(dir.path().join("sub")).unwrap();
        dir
    }

    fn path_str(p: &Path) -> String {
        p.to_str().unwrap().to_owned()
    }

    #[tokio::test]
    async fn lists_directory() {
        let dir = fixture();
        let mut entries: Vec<Entry> = LocalProvider
            .list(&path_str(dir.path()))
            .await
            .unwrap()
            .try_collect()
            .await
            .unwrap();
        entries.sort_by(|a, b| a.name.cmp(&b.name));

        let names: Vec<_> = entries.iter().map(|e| e.name.as_str()).collect();
        assert_eq!(names, [".hidden", "a.txt", "sub"]);
        assert_eq!(entries[1].size, Some(5));
        assert!(entries[2].is_dir());
        #[cfg(unix)]
        assert!(entries[0].hidden);
    }

    #[tokio::test]
    async fn stat_file_and_missing() {
        let dir = fixture();
        let entry = LocalProvider
            .stat(&path_str(&dir.path().join("a.txt")))
            .await
            .unwrap();
        assert_eq!(entry.kind, EntryKind::File);
        assert!(entry.modified.is_some());

        let missing = LocalProvider
            .stat(&path_str(&dir.path().join("nope")))
            .await;
        assert!(matches!(missing, Err(Error::NotFound(_))));
    }

    #[tokio::test]
    async fn list_of_file_fails() {
        let dir = fixture();
        assert!(
            LocalProvider
                .list(&path_str(&dir.path().join("a.txt")))
                .await
                .is_err()
        );
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn stat_reports_symlink() {
        let dir = fixture();
        let link = dir.path().join("link");
        std::os::unix::fs::symlink("a.txt", &link).unwrap();
        let entry = LocalProvider.stat(&path_str(&link)).await.unwrap();
        assert_eq!(entry.kind, EntryKind::Symlink);
        assert_eq!(entry.symlink_target.as_deref(), Some("a.txt"));
    }

    #[tokio::test]
    async fn fs_info_for_temp_dir() {
        let dir = fixture();
        let info = LocalProvider.fs_info(&path_str(dir.path())).await.unwrap();
        assert!(info.total_bytes.unwrap() > 0);
        assert!(info.mount_point.is_some());
    }
}
