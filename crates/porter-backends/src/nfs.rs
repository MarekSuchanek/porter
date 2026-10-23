//! NFSv3 exports through the pure-Rust `nfs3_client` crate. Paths are resolved to
//! file handles component by component (and cached); one connection is shared by all
//! operations, one request at a time.

use std::collections::HashMap;
use std::sync::{Arc, Mutex as StdMutex};
use std::time::SystemTime;

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use futures::StreamExt;
use nfs3_client::Nfs3ConnectionBuilder;
use nfs3_client::nfs3_types::nfs3::{
    self, COMMIT3args, CREATE3args, FSINFO3args, FSSTAT3args, GETATTR3args, LOOKUP3args,
    MKDIR3args, Nfs3Option, Nfs3Result, READ3args, READDIRPLUS3args, READLINK3args, REMOVE3args,
    RENAME3args, RMDIR3args, SETATTR3args, WRITE3args, createhow3, diropargs3, fattr3, filename3,
    ftype3, nfs_fh3, nfsstat3, nfstime3, sattr3, set_mtime, stable_how,
};
use nfs3_client::nfs3_types::rpc::{auth_unix, opaque_auth};
use nfs3_client::nfs3_types::xdr_codec::Opaque;
use nfs3_client::tokio::{TokioConnector, TokioIo};
use porter_core::{
    Capabilities, Entry, EntryKind, EntryStream, Error, FsInfo, Permissions, Provider, Reader,
    Result, Writer,
};
use tokio::io::AsyncWriteExt;
use tokio::net::TcpStream;
use tokio::sync::{Mutex, MutexGuard, OnceCell};

use crate::pipe;

type Connection = nfs3_client::Nfs3Connection<TokioIo<TcpStream>>;

/// Largest read/write request, whatever the server prefers beyond this.
const MAX_IO: u32 = 1 << 20;

#[derive(Debug, Clone)]
pub struct NfsOptions {
    pub host: String,
    pub export: String,
    pub port: Option<u16>,
    pub mount_port: Option<u16>,
    pub uid: Option<u32>,
    pub gid: Option<u32>,
    pub privileged_port: bool,
    /// Folder inside the export that `/` maps to.
    pub root: Option<String>,
}

pub struct NfsProvider {
    inner: Arc<Inner>,
}

struct Inner {
    name: String,
    label: String,
    opts: NfsOptions,
    session: OnceCell<Session>,
    /// Path below the export (`a/b`) → file handle.
    handles: StdMutex<HashMap<String, nfs_fh3>>,
}

struct Session {
    conn: Mutex<Connection>,
    read_size: u32,
    write_size: u32,
}

impl NfsProvider {
    pub fn new(name: &str, label: String, opts: NfsOptions) -> Self {
        Self {
            inner: Arc::new(Inner {
                name: name.to_owned(),
                label,
                opts,
                session: OnceCell::new(),
                handles: StdMutex::default(),
            }),
        }
    }
}

impl Inner {
    /// Mounts (once) and returns the shared session.
    async fn session(&self) -> Result<&Session> {
        self.session
            .get_or_try_init(|| async {
                let (uid, gid) = default_ids();
                let auth = auth_unix {
                    stamp: 0,
                    machinename: Opaque::borrowed(b"porter"),
                    uid: self.opts.uid.unwrap_or(uid),
                    gid: self.opts.gid.unwrap_or(gid),
                    gids: vec![],
                };
                let mut builder =
                    Nfs3ConnectionBuilder::new(TokioConnector, &self.opts.host, &self.opts.export)
                        .connect_from_privileged_port(self.opts.privileged_port)
                        .credential(opaque_auth::auth_unix(&auth));
                if let Some(port) = self.opts.port {
                    builder = builder.nfs3_port(port);
                }
                // Single-port servers serve MOUNT on the NFS port too.
                if let Some(port) = self.opts.mount_port.or(self.opts.port) {
                    builder = builder.mount_port(port);
                }
                let mut conn = builder
                    .mount()
                    .await
                    .map_err(|e| Error::backend(&self.name, format!("mount failed: {e}")))?;
                let root = conn.root_nfs_fh3();
                let info = conn.fsinfo(&FSINFO3args { fsroot: root }).await;
                let (read_size, write_size) = match info {
                    Ok(Nfs3Result::Ok(info)) => {
                        (pick(info.rtpref, info.rtmax), pick(info.wtpref, info.wtmax))
                    }
                    _ => (64 << 10, 64 << 10),
                };
                Ok(Session {
                    conn: Mutex::new(conn),
                    read_size,
                    write_size,
                })
            })
            .await
    }

    async fn conn(&self) -> Result<MutexGuard<'_, Connection>> {
        Ok(self.session().await?.conn.lock().await)
    }

    fn at(&self, path: &str) -> String {
        format!("{}{path}", self.label)
    }

    /// Components below the export: the configured root, then `path`.
    fn parts<'a>(&'a self, path: &'a str) -> Vec<&'a str> {
        let root = self.opts.root.as_deref().unwrap_or("");
        root.split('/')
            .chain(path.split('/'))
            .filter(|p| !p.is_empty())
            .collect()
    }

    /// The file handle for `path`, looking up uncached components.
    async fn resolve(&self, conn: &mut Connection, path: &str) -> Result<nfs_fh3> {
        let parts = self.parts(path);
        let (mut fh, start) = {
            let cache = self.handles.lock().unwrap();
            (1..=parts.len())
                .rev()
                .find_map(|i| cache.get(&parts[..i].join("/")).map(|fh| (fh.clone(), i)))
                .unwrap_or_else(|| (conn.root_nfs_fh3(), 0))
        };
        for i in start..parts.len() {
            let found = conn
                .lookup(&LOOKUP3args {
                    what: diropargs3 {
                        dir: fh,
                        name: filename3(Opaque::borrowed(parts[i].as_bytes())),
                    },
                })
                .await
                .map_err(|e| Error::backend(self.at(path), e))?;
            fh = match found {
                Nfs3Result::Ok(ok) => ok.object,
                Nfs3Result::Err((status, _)) => return Err(self.status(path, status)),
            };
            let key = parts[..=i].join("/");
            self.handles.lock().unwrap().insert(key, fh.clone());
        }
        Ok(fh)
    }

    /// The handle of `path`'s parent directory and `path`'s own name.
    async fn parent<'a>(&self, conn: &mut Connection, path: &'a str) -> Result<(nfs_fh3, &'a str)> {
        let trimmed = path.trim_end_matches('/');
        let (parent, name) = trimmed.rsplit_once('/').unwrap_or(("", trimmed));
        if name.is_empty() {
            return Err(Error::InvalidArgument(format!(
                "{}: is the root",
                self.at(path)
            )));
        }
        Ok((self.resolve(conn, parent).await?, name))
    }

    async fn attributes(&self, conn: &mut Connection, path: &str, fh: nfs_fh3) -> Result<fattr3> {
        match conn
            .getattr(&GETATTR3args { object: fh })
            .await
            .map_err(|e| Error::backend(self.at(path), e))?
        {
            Nfs3Result::Ok(ok) => Ok(ok.obj_attributes),
            Nfs3Result::Err((status, _)) => Err(self.status(path, status)),
        }
    }

    /// Forgets cached handles at and below `path` (after delete or rename).
    fn invalidate(&self, path: &str) {
        let key = self.parts(path).join("/");
        let prefix = format!("{key}/");
        self.handles
            .lock()
            .unwrap()
            .retain(|k, _| k != &key && !k.starts_with(&prefix));
    }

    fn status(&self, path: &str, status: nfsstat3) -> Error {
        let at = self.at(path);
        match status {
            nfsstat3::NFS3ERR_NOENT | nfsstat3::NFS3ERR_STALE => Error::NotFound(at),
            nfsstat3::NFS3ERR_EXIST => Error::AlreadyExists(at),
            nfsstat3::NFS3ERR_PERM | nfsstat3::NFS3ERR_ACCES | nfsstat3::NFS3ERR_ROFS => {
                Error::PermissionDenied(at)
            }
            nfsstat3::NFS3ERR_XDEV => Error::CrossDevice(at),
            nfsstat3::NFS3ERR_NOTEMPTY => {
                Error::io(at, std::io::ErrorKind::DirectoryNotEmpty.into())
            }
            nfsstat3::NFS3ERR_NOTDIR => Error::InvalidArgument(format!("{at}: not a directory")),
            nfsstat3::NFS3ERR_ISDIR => Error::InvalidArgument(format!("{at}: is a directory")),
            nfsstat3::NFS3ERR_NOSPC | nfsstat3::NFS3ERR_DQUOT => {
                Error::io(at, std::io::ErrorKind::StorageFull.into())
            }
            other => Error::backend(at, format!("{other:?}")),
        }
    }

    async fn set_attributes(&self, path: &str, attributes: sattr3) -> Result<()> {
        let mut conn = self.conn().await?;
        let object = self.resolve(&mut conn, path).await?;
        match conn
            .setattr(&SETATTR3args {
                object,
                new_attributes: attributes,
                guard: Nfs3Option::None,
            })
            .await
            .map_err(|e| Error::backend(self.at(path), e))?
        {
            Nfs3Result::Ok(_) => Ok(()),
            Nfs3Result::Err((status, _)) => Err(self.status(path, status)),
        }
    }
}

#[async_trait]
impl Provider for NfsProvider {
    fn scheme(&self) -> &str {
        "nfs"
    }

    fn capabilities(&self) -> Capabilities {
        Capabilities {
            list: true,
            read: true,
            write: true,
            delete: true,
            rename: true,
            mkdir: true,
            set_permissions: true,
            set_modified: true,
            fs_info: true,
            ..Default::default()
        }
    }

    async fn stat(&self, path: &str) -> Result<Entry> {
        let inner = &self.inner;
        let mut conn = inner.conn().await?;
        let fh = inner.resolve(&mut conn, path).await?;
        let attrs = inner.attributes(&mut conn, path, fh.clone()).await?;
        let target = match attrs.type_ {
            ftype3::NF3LNK => match conn.readlink(&READLINK3args { symlink: fh }).await {
                Ok(Nfs3Result::Ok(link)) => {
                    Some(String::from_utf8_lossy(&link.data.0.0).into_owned())
                }
                _ => None,
            },
            _ => None,
        };
        Ok(to_entry(path, &attrs, target))
    }

    async fn list(&self, path: &str) -> Result<EntryStream> {
        let inner = &self.inner;
        let mut conn = inner.conn().await?;
        let dir = inner.resolve(&mut conn, path).await?;
        let base = path.trim_end_matches('/').to_owned();
        let dir_key = inner.parts(path).join("/");

        let mut entries = Vec::new();
        let mut cookie = nfs3::cookie3::default();
        let mut cookieverf = nfs3::cookieverf3::default();
        loop {
            let page = conn
                .readdirplus(&READDIRPLUS3args {
                    dir: dir.clone(),
                    cookie,
                    cookieverf,
                    dircount: 64 << 10,
                    maxcount: 256 << 10,
                })
                .await
                .map_err(|e| Error::backend(inner.at(path), e))?;
            let page = match page {
                Nfs3Result::Ok(page) => page,
                Nfs3Result::Err((status, _)) => return Err(inner.status(path, status)),
            };
            cookieverf = page.cookieverf;
            let eof = page.reply.eof;
            for item in page.reply.entries.into_inner() {
                cookie = item.cookie;
                let name = String::from_utf8_lossy(&item.name.0.0).into_owned();
                if name == "." || name == ".." {
                    continue;
                }
                let child = format!("{base}/{name}");
                if let Nfs3Option::Some(fh) = item.name_handle.clone() {
                    let key = if dir_key.is_empty() {
                        name.clone()
                    } else {
                        format!("{dir_key}/{name}")
                    };
                    inner.handles.lock().unwrap().insert(key, fh);
                }
                let attrs = match item.name_attributes {
                    Nfs3Option::Some(attrs) => attrs,
                    Nfs3Option::None => match inner.resolve(&mut conn, &child).await {
                        Ok(fh) => inner.attributes(&mut conn, &child, fh).await?,
                        Err(e) => return Err(e),
                    },
                };
                entries.push(Ok(to_entry(&child, &attrs, None)));
            }
            if eof {
                break;
            }
        }
        Ok(futures::stream::iter(entries).boxed())
    }

    async fn fs_info(&self, path: &str) -> Result<FsInfo> {
        let inner = &self.inner;
        let mut conn = inner.conn().await?;
        let fsroot = inner.resolve(&mut conn, "/").await?;
        match conn
            .fsstat(&FSSTAT3args { fsroot })
            .await
            .map_err(|e| Error::backend(inner.at(path), e))?
        {
            Nfs3Result::Ok(stat) => Ok(FsInfo {
                name: Some(inner.name.clone()),
                fs_type: Some("nfs".into()),
                total_bytes: Some(stat.tbytes),
                available_bytes: Some(stat.abytes),
                ..Default::default()
            }),
            Nfs3Result::Err((status, _)) => Err(inner.status(path, status)),
        }
    }

    async fn read(&self, path: &str) -> Result<Reader> {
        let inner = self.inner.clone();
        let (file, chunk) = {
            let mut conn = inner.conn().await?;
            let fh = inner.resolve(&mut conn, path).await?;
            if inner.attributes(&mut conn, path, fh.clone()).await?.type_ == ftype3::NF3DIR {
                return Err(Error::InvalidArgument(format!(
                    "{}: is a directory",
                    inner.at(path)
                )));
            }
            (fh, inner.session().await?.read_size)
        };
        let path = path.to_owned();
        Ok(Box::new(pipe::reader(|mut out| async move {
            let mut offset = 0;
            loop {
                let reply = {
                    let mut conn = inner.conn().await.map_err(std::io::Error::other)?;
                    conn.read(&READ3args {
                        file: file.clone(),
                        offset,
                        count: chunk,
                    })
                    .await
                    .map_err(std::io::Error::other)?
                };
                let data = match reply {
                    Nfs3Result::Ok(data) => data,
                    Nfs3Result::Err((status, _)) => {
                        return Err(std::io::Error::other(inner.status(&path, status)));
                    }
                };
                out.write_all(&data.data.0).await?;
                offset += data.data.0.len() as u64;
                if data.eof || data.data.0.is_empty() {
                    return Ok(());
                }
            }
        })))
    }

    async fn write(&self, path: &str, overwrite: bool) -> Result<Writer> {
        let inner = self.inner.clone();
        let (file, chunk) = {
            let mut conn = inner.conn().await?;
            let (dir, name) = inner.parent(&mut conn, path).await?;
            let attributes = sattr3 {
                mode: Nfs3Option::Some(0o644),
                size: if overwrite {
                    Nfs3Option::Some(0)
                } else {
                    Nfs3Option::None
                },
                ..Default::default()
            };
            let created = conn
                .create(&CREATE3args {
                    where_: diropargs3 {
                        dir: dir.clone(),
                        name: filename3(Opaque::borrowed(name.as_bytes())),
                    },
                    how: match overwrite {
                        true => createhow3::UNCHECKED(attributes),
                        false => createhow3::GUARDED(attributes),
                    },
                })
                .await
                .map_err(|e| Error::backend(inner.at(path), e))?;
            let fh = match created {
                Nfs3Result::Ok(ok) => match ok.obj {
                    Nfs3Option::Some(fh) => fh,
                    Nfs3Option::None => {
                        inner.invalidate(path);
                        inner.resolve(&mut conn, path).await?
                    }
                },
                Nfs3Result::Err((status, _)) => return Err(inner.status(path, status)),
            };
            (fh, inner.session().await?.write_size)
        };
        let path = path.to_owned();
        Ok(Box::new(pipe::writer(|mut input| async move {
            let mut buf = vec![0; chunk as usize];
            let mut offset = 0;
            let fail = |status| std::io::Error::other(inner.status(&path, status));
            loop {
                let n = read_full(&mut input, &mut buf).await?;
                if n == 0 {
                    break;
                }
                let mut done = 0;
                while done < n {
                    let reply = {
                        let mut conn = inner.conn().await.map_err(std::io::Error::other)?;
                        conn.write(&WRITE3args {
                            file: file.clone(),
                            offset,
                            count: (n - done) as u32,
                            stable: stable_how::UNSTABLE,
                            data: Opaque::borrowed(&buf[done..n]),
                        })
                        .await
                        .map_err(std::io::Error::other)?
                    };
                    let written = match reply {
                        Nfs3Result::Ok(ok) => ok.count as usize,
                        Nfs3Result::Err((status, _)) => return Err(fail(status)),
                    };
                    if written == 0 {
                        return Err(std::io::Error::other("server accepted no data"));
                    }
                    done += written;
                    offset += written as u64;
                }
            }
            // Unstable writes are only durable once committed.
            let mut conn = inner.conn().await.map_err(std::io::Error::other)?;
            match conn
                .commit(&COMMIT3args {
                    file,
                    offset: 0,
                    count: 0,
                })
                .await
                .map_err(std::io::Error::other)?
            {
                Nfs3Result::Ok(_) => Ok(()),
                Nfs3Result::Err((status, _)) => Err(fail(status)),
            }
        })))
    }

    async fn mkdir(&self, path: &str) -> Result<()> {
        let inner = &self.inner;
        let mut conn = inner.conn().await?;
        let (dir, name) = inner.parent(&mut conn, path).await?;
        match conn
            .mkdir(&MKDIR3args {
                where_: diropargs3 {
                    dir,
                    name: filename3(Opaque::borrowed(name.as_bytes())),
                },
                attributes: sattr3 {
                    mode: Nfs3Option::Some(0o755),
                    ..Default::default()
                },
            })
            .await
            .map_err(|e| Error::backend(inner.at(path), e))?
        {
            Nfs3Result::Ok(_) => Ok(()),
            Nfs3Result::Err((status, _)) => Err(inner.status(path, status)),
        }
    }

    async fn delete(&self, path: &str) -> Result<()> {
        let is_dir = self.stat(path).await?.is_dir();
        let inner = &self.inner;
        let mut conn = inner.conn().await?;
        let (dir, name) = inner.parent(&mut conn, path).await?;
        let object = diropargs3 {
            dir,
            name: filename3(Opaque::borrowed(name.as_bytes())),
        };
        let status = if is_dir {
            match conn.rmdir(&RMDIR3args { object }).await {
                Ok(Nfs3Result::Ok(_)) => None,
                Ok(Nfs3Result::Err((status, _))) => Some(status),
                Err(e) => return Err(Error::backend(inner.at(path), e)),
            }
        } else {
            match conn.remove(&REMOVE3args { object }).await {
                Ok(Nfs3Result::Ok(_)) => None,
                Ok(Nfs3Result::Err((status, _))) => Some(status),
                Err(e) => return Err(Error::backend(inner.at(path), e)),
            }
        };
        inner.invalidate(path);
        match status {
            None => Ok(()),
            Some(status) => Err(inner.status(path, status)),
        }
    }

    async fn rename(&self, from: &str, to: &str) -> Result<()> {
        let inner = &self.inner;
        let mut conn = inner.conn().await?;
        let (from_dir, from_name) = inner.parent(&mut conn, from).await?;
        let (to_dir, to_name) = inner.parent(&mut conn, to).await?;
        let reply = conn
            .rename(&RENAME3args {
                from: diropargs3 {
                    dir: from_dir,
                    name: filename3(Opaque::borrowed(from_name.as_bytes())),
                },
                to: diropargs3 {
                    dir: to_dir,
                    name: filename3(Opaque::borrowed(to_name.as_bytes())),
                },
            })
            .await
            .map_err(|e| Error::backend(inner.at(from), e))?;
        inner.invalidate(from);
        inner.invalidate(to);
        match reply {
            Nfs3Result::Ok(_) => Ok(()),
            Nfs3Result::Err((status, _)) => Err(inner.status(from, status)),
        }
    }

    async fn set_permissions(&self, path: &str, mode: u32) -> Result<()> {
        let attributes = sattr3 {
            mode: Nfs3Option::Some(mode & 0o7777),
            ..Default::default()
        };
        self.inner.set_attributes(path, attributes).await
    }

    async fn set_modified(&self, path: &str, time: DateTime<Utc>) -> Result<()> {
        let attributes = sattr3 {
            mtime: set_mtime::SET_TO_CLIENT_TIME(nfstime3 {
                seconds: time.timestamp().clamp(0, u32::MAX.into()) as u32,
                nseconds: time.timestamp_subsec_nanos(),
            }),
            ..Default::default()
        };
        self.inner.set_attributes(path, attributes).await
    }
}

/// The server's preferred transfer size, within sane bounds.
fn pick(preferred: u32, max: u32) -> u32 {
    let size = if preferred > 0 { preferred } else { max };
    size.clamp(4 << 10, MAX_IO.min(max.max(4 << 10)))
}

fn to_entry(path: &str, attrs: &fattr3, symlink_target: Option<String>) -> Entry {
    let trimmed = path.trim_end_matches('/');
    let name = match trimmed.rsplit('/').next() {
        Some(name) if !name.is_empty() => name.to_owned(),
        _ => "/".to_owned(),
    };
    let kind = match attrs.type_ {
        ftype3::NF3REG => EntryKind::File,
        ftype3::NF3DIR => EntryKind::Dir,
        ftype3::NF3LNK => EntryKind::Symlink,
        _ => EntryKind::Other,
    };
    let time = |t: nfstime3| Some(DateTime::<Utc>::from(SystemTime::from(t)));
    Entry {
        hidden: name.starts_with('.'),
        path: if trimmed.is_empty() {
            "/".into()
        } else {
            trimmed.to_owned()
        },
        name,
        kind,
        size: (kind != EntryKind::Dir).then_some(attrs.size),
        modified: time(attrs.mtime),
        created: None,
        accessed: time(attrs.atime),
        permissions: Some(Permissions {
            mode: Some(attrs.mode & 0o7777),
            readonly: attrs.mode & 0o222 == 0,
            uid: Some(attrs.uid),
            gid: Some(attrs.gid),
        }),
        symlink_target,
    }
}

/// The uid/gid a local mount would use: the current user's.
fn default_ids() -> (u32, u32) {
    #[cfg(unix)]
    {
        // SAFETY: getuid/getgid have no preconditions and cannot fail.
        unsafe { (libc::getuid(), libc::getgid()) }
    }
    #[cfg(not(unix))]
    {
        (65534, 65534) // nobody
    }
}

/// Fills `buf` as far as the input allows; 0 means end of input.
async fn read_full(
    input: &mut (impl tokio::io::AsyncRead + Unpin),
    buf: &mut [u8],
) -> std::io::Result<usize> {
    use tokio::io::AsyncReadExt;
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
    fn transfer_sizes() {
        assert_eq!(pick(64 << 10, 1 << 20), 64 << 10);
        assert_eq!(pick(0, 32 << 10), 32 << 10);
        assert_eq!(pick(8 << 20, 8 << 20), MAX_IO);
    }

    #[test]
    fn components() {
        let provider = NfsProvider::new(
            "lab",
            "lab:".into(),
            NfsOptions {
                host: "h".into(),
                export: "/e".into(),
                port: None,
                mount_port: None,
                uid: None,
                gid: None,
                privileged_port: false,
                root: Some("/team/".into()),
            },
        );
        assert_eq!(provider.inner.parts("/a/b/"), ["team", "a", "b"]);
        assert_eq!(provider.inner.parts("/"), ["team"]);
    }
}
