//! SMB 2/3 shares (Windows, Samba, NAS boxes) through the pure-Rust `smb` crate.

use std::str::FromStr;
use std::sync::Arc;
use std::time::{Duration, SystemTime};

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use futures::StreamExt;
use porter_core::{
    Capabilities, Entry, EntryKind, EntryStream, Error, FsInfo, Permissions, Provider, Reader,
    Result, Writer,
};
use smb::{
    Client, ClientConfig, ConnectionConfig, CreateOptions, DirAccessMask, Directory,
    FileAccessMask, FileAttributes, FileBasicInformation, FileCreateArgs,
    FileDispositionInformation, FileFsFullSizeInformation, FileIdBothDirectoryInformation,
    FileNetworkOpenInformation, FileRenameInformation, Resource, Status, UncPath,
};
use smb_dtyp::binrw_util::prelude::FileTime;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::sync::OnceCell;

use crate::pipe;

/// Bytes per SMB read/write request; well below the usual negotiated maximum (8 MiB).
const BLOCK: usize = 1 << 20;
const TIMEOUT: Duration = Duration::from_secs(20);
/// Seconds between 1601-01-01 (the FILETIME epoch) and 1970-01-01.
const FILETIME_UNIX_OFFSET: u64 = 11_644_473_600;

pub struct SmbProvider {
    name: String,
    /// Prefix that turns a path into a user-facing location, e.g. `office:`.
    label: String,
    host: String,
    share: String,
    port: Option<u16>,
    user: Option<String>,
    password: Option<String>,
    /// Folder inside the share that `/` maps to, `\`-separated.
    root: String,
    client: OnceCell<Client>,
}

impl SmbProvider {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        name: &str,
        label: String,
        host: &str,
        share: &str,
        port: Option<u16>,
        user: Option<&str>,
        password: Option<&str>,
        root: Option<&str>,
    ) -> Self {
        Self {
            name: name.to_owned(),
            label,
            host: host.to_owned(),
            share: share.to_owned(),
            port,
            user: user.map(str::to_owned),
            password: password.map(str::to_owned),
            root: smb_path("", root.unwrap_or("")),
            client: OnceCell::new(),
        }
    }

    /// Connects (once) and signs in to the share; as guest without a password.
    async fn client(&self) -> Result<&Client> {
        self.client
            .get_or_try_init(|| async {
                let guest = self.password.is_none();
                let client = Client::new(ClientConfig {
                    connection: ConnectionConfig {
                        port: self.port,
                        timeout: Some(TIMEOUT),
                        allow_unsigned_guest_access: guest,
                        ..Default::default()
                    },
                    ..Default::default()
                });
                // `/GUEST` is how the `sspi` crate asks for a guest session.
                let user = self.user.as_deref().unwrap_or("/GUEST");
                let password = self.password.clone().unwrap_or_default();
                client
                    .share_connect(&self.share_unc()?, user, password)
                    .await
                    .map_err(|e| self.error("", e))?;
                Ok(client)
            })
            .await
    }

    fn share_unc(&self) -> Result<UncPath> {
        let unc = format!(r"\\{}\{}", self.host, self.share);
        UncPath::from_str(&unc).map_err(|e| Error::invalid_location(unc, e.to_string()))
    }

    fn unc(&self, path: &str) -> Result<UncPath> {
        let relative = smb_path(&self.root, path);
        let share = self.share_unc()?;
        Ok(match relative.is_empty() {
            true => share,
            false => share.with_path(&relative),
        })
    }

    async fn open(&self, path: &str, args: &FileCreateArgs) -> Result<Resource> {
        let client = self.client().await?;
        client
            .create_file(&self.unc(path)?, args)
            .await
            .map_err(|e| self.error(path, e))
    }

    fn at(&self, path: &str) -> String {
        format!("{}{path}", self.label)
    }

    fn error(&self, path: &str, e: smb::Error) -> Error {
        let at = self.at(path);
        let status = match &e {
            smb::Error::UnexpectedMessageStatus(code)
            | smb::Error::ReceivedErrorMessage(code, _) => Some(*code),
            _ => None,
        };
        let is = |s: Status| status == Some(s as u32);
        if is(Status::ObjectNameNotFound) || is(Status::ObjectPathNotFound) {
            Error::NotFound(at)
        } else if is(Status::ObjectNameCollision) {
            Error::AlreadyExists(at)
        } else if is(Status::AccessDenied) {
            Error::PermissionDenied(at)
        } else if is(Status::LogonFailure) {
            Error::backend(&self.name, "sign-in failed: check the user and password")
        } else if is(Status::BadNetworkName) {
            Error::backend(&self.name, format!("no share named `{}`", self.share))
        } else if is(Status::DirectoryNotEmpty) {
            Error::io(at, std::io::ErrorKind::DirectoryNotEmpty.into())
        } else {
            Error::backend(at, e)
        }
    }

    /// Runs `op` on the opened resource and always closes it.
    async fn with_open<T>(
        &self,
        path: &str,
        args: &FileCreateArgs,
        op: impl AsyncFnOnce(&Resource) -> smb::Result<T>,
    ) -> Result<T> {
        let resource = self.open(path, args).await?;
        let result = op(&resource).await;
        let closed = close(&resource).await;
        let value = result.map_err(|e| self.error(path, e))?;
        closed.map_err(|e| self.error(path, e))?;
        Ok(value)
    }
}

#[async_trait]
impl Provider for SmbProvider {
    fn scheme(&self) -> &str {
        "smb"
    }

    fn capabilities(&self) -> Capabilities {
        Capabilities {
            list: true,
            read: true,
            write: true,
            delete: true,
            rename: true,
            mkdir: true,
            set_modified: true,
            fs_info: true,
            ..Default::default()
        }
    }

    async fn stat(&self, path: &str) -> Result<Entry> {
        let access = FileAccessMask::new().with_file_read_attributes(true);
        let (info, is_dir) = self
            .with_open(
                path,
                &FileCreateArgs::make_open_existing(access),
                async |r: &Resource| {
                    let info = handle(r).query_info::<FileNetworkOpenInformation>().await?;
                    Ok((info, r.is_dir()))
                },
            )
            .await?;
        let kind = if is_dir || info.file_attributes.directory() {
            EntryKind::Dir
        } else {
            EntryKind::File
        };
        Ok(entry(
            path,
            kind,
            info.end_of_file,
            &info.file_attributes,
            [
                info.last_write_time,
                info.creation_time,
                info.last_access_time,
            ],
        ))
    }

    async fn list(&self, path: &str) -> Result<EntryStream> {
        let access = DirAccessMask::new()
            .with_list_directory(true)
            .with_synchronize(true);
        let args = FileCreateArgs::make_open_existing(access.into());
        let resource = self.open(path, &args).await?;
        let Resource::Directory(dir) = resource else {
            let _ = close(&resource).await;
            return Err(Error::InvalidArgument(format!(
                "{}: not a directory",
                self.at(path)
            )));
        };
        let dir = Arc::new(dir);
        let listed: Vec<smb::Result<FileIdBothDirectoryInformation>> =
            match Directory::query::<FileIdBothDirectoryInformation>(&dir, "*").await {
                Ok(stream) => stream.collect().await,
                Err(e) => vec![Err(e)],
            };
        let _ = dir.close().await;

        let base = path.trim_end_matches('/').to_owned();
        let mut entries = Vec::with_capacity(listed.len());
        for item in listed {
            let info = item.map_err(|e| self.error(path, e))?;
            let name = info.file_name.to_string();
            if name == "." || name == ".." {
                continue;
            }
            let kind = match info.file_attributes.directory() {
                true => EntryKind::Dir,
                false => EntryKind::File,
            };
            entries.push(Ok(entry(
                &format!("{base}/{name}"),
                kind,
                info.end_of_file,
                &info.file_attributes,
                [
                    info.last_write_time,
                    info.creation_time,
                    info.last_access_time,
                ],
            )));
        }
        Ok(futures::stream::iter(entries).boxed())
    }

    async fn fs_info(&self, path: &str) -> Result<FsInfo> {
        let access = FileAccessMask::new().with_file_read_attributes(true);
        let size = self
            .with_open(
                "/",
                &FileCreateArgs::make_open_existing(access),
                async |r: &Resource| handle(r).query_fs_info::<FileFsFullSizeInformation>().await,
            )
            .await
            .map_err(|e| match e {
                Error::NotFound(_) => Error::NotFound(self.at(path)),
                e => e,
            })?;
        let unit = u64::from(size.sectors_per_allocation_unit) * u64::from(size.bytes_per_sector);
        Ok(FsInfo {
            name: Some(self.name.clone()),
            fs_type: Some("smb".into()),
            total_bytes: Some(size.total_allocation_units * unit),
            available_bytes: Some(size.caller_available_allocation_units * unit),
            ..Default::default()
        })
    }

    async fn read(&self, path: &str) -> Result<Reader> {
        let access = FileAccessMask::new()
            .with_generic_read(true)
            .with_file_read_data(true);
        let file = match self
            .open(path, &FileCreateArgs::make_open_existing(access))
            .await?
        {
            Resource::File(file) => file,
            other => {
                let _ = close(&other).await;
                return Err(Error::InvalidArgument(format!(
                    "{}: is a directory",
                    self.at(path)
                )));
            }
        };
        Ok(Box::new(pipe::reader(|mut out| async move {
            let mut buf = vec![0; BLOCK];
            let mut pos = 0;
            let copied = async {
                loop {
                    let n = file.read_block(&mut buf, pos, None, false).await?;
                    if n == 0 {
                        return Ok(());
                    }
                    out.write_all(&buf[..n]).await?;
                    pos += n as u64;
                }
            }
            .await;
            let closed = file.close().await.map_err(std::io::Error::other);
            copied.and(closed)
        })))
    }

    async fn write(&self, path: &str, overwrite: bool) -> Result<Writer> {
        let mut args = match overwrite {
            true => FileCreateArgs::make_overwrite(FileAttributes::new(), CreateOptions::new()),
            false => FileCreateArgs::make_create_new(FileAttributes::new(), CreateOptions::new()),
        };
        // The helpers ask for GENERIC_ALL, which includes changing permissions and
        // owner; servers refuse that to guests and restricted users who may write.
        // smb-rs checks FILE_WRITE_DATA itself before writing, so name it explicitly.
        args.desired_access = FileAccessMask::new()
            .with_file_write_data(true)
            .with_file_append_data(true)
            .with_file_read_attributes(true)
            .with_file_write_attributes(true);
        let file = match self.open(path, &args).await? {
            Resource::File(file) => file,
            other => {
                let _ = close(&other).await;
                return Err(Error::InvalidArgument(format!(
                    "{}: is a directory",
                    self.at(path)
                )));
            }
        };
        Ok(Box::new(pipe::writer(|mut input| async move {
            let mut buf = vec![0; BLOCK];
            let mut pos = 0;
            let copied = async {
                loop {
                    let n = read_full(&mut input, &mut buf).await?;
                    if n == 0 {
                        return Ok(());
                    }
                    let mut done = 0;
                    while done < n {
                        let written = file.write_block(&buf[done..n], pos, None).await?;
                        if written == 0 {
                            return Err(std::io::Error::other("server accepted no data"));
                        }
                        done += written;
                        pos += written as u64;
                    }
                }
            }
            .await;
            let closed = file.close().await.map_err(std::io::Error::other);
            copied.and(closed)
        })))
    }

    async fn mkdir(&self, path: &str) -> Result<()> {
        let mut args = FileCreateArgs::make_create_new(
            FileAttributes::new().with_directory(true),
            CreateOptions::new().with_directory_file(true),
        );
        args.desired_access = FileAccessMask::new().with_file_read_attributes(true);
        self.with_open(path, &args, async |_| Ok(())).await
    }

    async fn delete(&self, path: &str) -> Result<()> {
        let access = FileAccessMask::new()
            .with_delete(true)
            .with_file_read_attributes(true);
        self.with_open(
            path,
            &FileCreateArgs::make_open_existing(access),
            async |r: &Resource| {
                handle(r)
                    .set_info(FileDispositionInformation {
                        delete_pending: true.into(),
                    })
                    .await
            },
        )
        .await
    }

    async fn rename(&self, from: &str, to: &str) -> Result<()> {
        let access = FileAccessMask::new()
            .with_delete(true)
            .with_file_read_attributes(true);
        let target = smb_path(&self.root, to);
        self.with_open(
            from,
            &FileCreateArgs::make_open_existing(access),
            async |r: &Resource| {
                handle(r)
                    .set_info(FileRenameInformation {
                        replace_if_exists: true.into(),
                        root_directory: 0,
                        file_name: target.as_str().into(),
                    })
                    .await
            },
        )
        .await
    }

    async fn set_modified(&self, path: &str, time: DateTime<Utc>) -> Result<()> {
        let access = FileAccessMask::new().with_file_write_attributes(true);
        let unchanged = || FileTime::from(0u64);
        self.with_open(
            path,
            &FileCreateArgs::make_open_existing(access),
            async |r: &Resource| {
                handle(r)
                    .set_info(FileBasicInformation {
                        creation_time: unchanged(),
                        last_access_time: unchanged(),
                        last_write_time: to_filetime(time),
                        change_time: unchanged(),
                        file_attributes: FileAttributes::new(),
                    })
                    .await
            },
        )
        .await
    }
}

/// The handle behind any kind of opened resource.
fn handle(resource: &Resource) -> &smb::resource::ResourceHandle {
    match resource {
        Resource::File(f) => f,
        Resource::Directory(d) => d,
        Resource::Pipe(p) => p,
    }
}

async fn close(resource: &Resource) -> smb::Result<()> {
    handle(resource).close().await
}

/// Porter path (`/a/b`) under `root` → share-relative SMB path (`root\a\b`).
fn smb_path(root: &str, path: &str) -> String {
    root.split(['\\', '/'])
        .chain(path.split(['\\', '/']))
        .filter(|part| !part.is_empty())
        .collect::<Vec<_>>()
        .join("\\")
}

fn entry(
    path: &str,
    kind: EntryKind,
    size: u64,
    attributes: &FileAttributes,
    [modified, created, accessed]: [FileTime; 3],
) -> Entry {
    let trimmed = path.trim_end_matches('/');
    let name = match trimmed.rsplit('/').next() {
        Some(name) if !name.is_empty() => name.to_owned(),
        _ => "/".to_owned(),
    };
    Entry {
        hidden: attributes.hidden() || name.starts_with('.'),
        path: if trimmed.is_empty() {
            "/".into()
        } else {
            trimmed.to_owned()
        },
        name,
        kind,
        size: (kind == EntryKind::File).then_some(size),
        modified: from_filetime(modified),
        created: from_filetime(created),
        accessed: from_filetime(accessed),
        permissions: Some(Permissions {
            readonly: attributes.readonly(),
            ..Default::default()
        }),
        symlink_target: None,
    }
}

/// `None` for unset times (FILETIME 0 is 1601).
fn from_filetime(time: FileTime) -> Option<DateTime<Utc>> {
    let time = DateTime::<Utc>::from(SystemTime::from(time));
    (time.timestamp() > 0).then_some(time)
}

fn to_filetime(time: DateTime<Utc>) -> FileTime {
    let seconds = (time.timestamp().max(0) as u64) + FILETIME_UNIX_OFFSET;
    FileTime::from(seconds * 10_000_000 + u64::from(time.timestamp_subsec_nanos()) / 100)
}

/// Fills `buf` as far as the input allows; 0 means end of input.
async fn read_full(
    input: &mut (impl tokio::io::AsyncRead + Unpin),
    buf: &mut [u8],
) -> std::io::Result<usize> {
    let mut filled = 0;
    while filled < buf.len() {
        let n = input.read(&mut buf[filled..]).await?;
        if n == 0 {
            break;
        }
        filled += n;
    }
    Ok(filled)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn paths() {
        assert_eq!(smb_path("", "/"), "");
        assert_eq!(smb_path("", "/a/b.txt"), r"a\b.txt");
        assert_eq!(smb_path(r"Team\Docs", "/a/"), r"Team\Docs\a");
    }

    #[test]
    fn filetime_round_trip() {
        let time = DateTime::parse_from_rfc3339("2026-10-23T12:34:56.789Z")
            .unwrap()
            .to_utc();
        assert_eq!(from_filetime(to_filetime(time)), Some(time));
        assert_eq!(from_filetime(FileTime::from(0u64)), None);
    }
}
