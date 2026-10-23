//! Data the UI asks for, independent of Tauri so it can be tested directly.

use futures::TryStreamExt;
use porter_backends::{Registry, local};
use porter_core::{Capabilities, Entry, Error, FsInfo, Location, Result, Target};
use porter_jobs::{Conflict, CopyOptions, Job, move_to};
use serde::Serialize;

/// An entry plus the location string the UI uses to address it.
#[derive(Debug, Clone, Serialize)]
pub struct Item {
    #[serde(flatten)]
    pub entry: Entry,
    pub location: String,
}

#[derive(Debug, Serialize)]
pub struct Crumb {
    pub name: String,
    pub location: String,
}

#[derive(Debug, Serialize)]
pub struct Listing {
    /// Normalized location of the directory (absolute for local paths).
    pub location: String,
    /// Root first.
    pub crumbs: Vec<Crumb>,
    pub parent: Option<String>,
    pub items: Vec<Item>,
    pub capabilities: Capabilities,
    pub fs: Option<FsInfo>,
}

#[derive(Debug, Serialize)]
pub struct Sidebar {
    pub places: Vec<Place>,
    pub disks: Vec<Disk>,
    pub remotes: Vec<RemoteItem>,
    pub config_error: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct Place {
    pub name: String,
    pub location: String,
    pub icon: &'static str,
}

#[derive(Debug, Serialize)]
pub struct Disk {
    pub name: String,
    pub location: String,
    pub info: FsInfo,
}

#[derive(Debug, Serialize)]
pub struct RemoteItem {
    pub name: String,
    pub kind: &'static str,
    pub address: String,
    pub location: String,
}

pub async fn list_dir(registry: &Registry, location: &str) -> Result<Listing> {
    let target = registry.target(location)?;
    let dir = target.provider.stat_follow(target.path()).await?;
    if !dir.is_dir() {
        return Err(Error::InvalidArgument(format!("{target}: not a directory")));
    }
    let entries: Vec<Entry> = target
        .provider
        .list(target.path())
        .await?
        .try_collect()
        .await?;
    let items = entries
        .into_iter()
        .map(|entry| Item {
            location: target.child(&entry.name).to_string(),
            entry,
        })
        .collect();
    Ok(Listing {
        location: target.to_string(),
        crumbs: crumbs(&target),
        parent: target.parent().map(|p| p.to_string()),
        items,
        capabilities: target.provider.capabilities(),
        fs: target.provider.fs_info(target.path()).await.ok(),
    })
}

pub async fn stat(registry: &Registry, location: &str) -> Result<Item> {
    let target = registry.target(location)?;
    Ok(Item {
        entry: target.provider.stat(target.path()).await?,
        location: target.to_string(),
    })
}

pub async fn sidebar(registry: &Registry, config_error: Option<String>) -> Result<Sidebar> {
    let mut places = Vec::new();
    if let Some(dirs) = directories::UserDirs::new() {
        let mut add = |name: &str, path: Option<&std::path::Path>, icon| {
            if let Some(path) = path {
                places.push(Place {
                    name: name.to_owned(),
                    location: path.display().to_string(),
                    icon,
                });
            }
        };
        add("Home", Some(dirs.home_dir()), "home");
        add("Desktop", dirs.desktop_dir(), "desktop");
        add("Documents", dirs.document_dir(), "documents");
        add("Downloads", dirs.download_dir(), "downloads");
        add("Pictures", dirs.picture_dir(), "pictures");
    }

    let disks = local::disks()
        .await?
        .into_iter()
        .filter_map(|info| {
            let mount = info.mount_point.clone()?;
            if is_system_mount(&mount) {
                return None;
            }
            Some(Disk {
                name: info.name.clone().unwrap_or_else(|| mount.clone()),
                location: mount,
                info,
            })
        })
        .collect();

    let remotes = registry
        .config()
        .remotes
        .iter()
        .map(|(name, remote)| RemoteItem {
            name: name.clone(),
            kind: remote.backend.kind(),
            address: remote.backend.address(),
            location: format!("{name}:/"),
        })
        .collect();

    Ok(Sidebar {
        places,
        disks,
        remotes,
        config_error,
    })
}

/// Creates `name` inside `parent`; returns the new folder's location.
pub async fn mkdir(registry: &Registry, parent: &str, name: &str) -> Result<String> {
    validate_name(name)?;
    let target = registry.target(parent)?.child(name);
    porter_jobs::mkdir(&target, false).await?;
    Ok(target.to_string())
}

/// Renames an entry within its directory; returns the new location.
pub async fn rename(registry: &Registry, location: &str, name: &str) -> Result<String> {
    validate_name(name)?;
    let src = registry.target(location)?;
    let dst = src
        .parent()
        .ok_or_else(|| Error::InvalidArgument(format!("{src}: cannot rename a root")))?
        .child(name);
    match dst.provider.stat(dst.path()).await {
        Ok(_) => return Err(Error::AlreadyExists(dst.to_string())),
        Err(Error::NotFound(_)) => {}
        Err(e) => return Err(e),
    }
    let opts = CopyOptions {
        recursive: true,
        conflict: Conflict::Fail,
        ..Default::default()
    };
    let mut report = move_to(&Job::new(), &[src], &dst, &opts).await?;
    match report.errors.is_empty() {
        true => Ok(dst.to_string()),
        false => Err(report.errors.remove(0)),
    }
}

/// The local path behind `location`, for opening it with the OS.
pub fn local_path(registry: &Registry, location: &str) -> Result<String> {
    let target = registry.target(location)?;
    match target.location.is_local() {
        true => Ok(target.path().to_owned()),
        false => Err(Error::InvalidArgument(format!(
            "{target}: only local files can be opened (copy it to this computer first)"
        ))),
    }
}

fn validate_name(name: &str) -> Result<()> {
    let invalid = name.is_empty()
        || name == "."
        || name == ".."
        || name.contains('/')
        || (cfg!(windows) && name.contains('\\'));
    match invalid {
        true => Err(Error::InvalidArgument(format!("invalid name `{name}`"))),
        false => Ok(()),
    }
}

/// Volumes the OS mounts for itself; they only add noise to the sidebar.
fn is_system_mount(mount: &str) -> bool {
    const PREFIXES: [&str; 6] = [
        "/System/Volumes/",
        "/private/var/vm",
        "/boot",
        "/snap/",
        "/run/",
        "/var/lib/docker",
    ];
    PREFIXES.iter().any(|p| mount.starts_with(p))
}

fn crumbs(target: &Target) -> Vec<Crumb> {
    let mut crumbs = Vec::new();
    let mut current = Some(target.clone());
    while let Some(t) = current {
        crumbs.push(Crumb {
            name: t.file_name().unwrap_or_else(|| root_name(&t)),
            location: t.to_string(),
        });
        current = t.parent();
    }
    crumbs.reverse();
    crumbs
}

fn root_name(target: &Target) -> String {
    match target.location.scheme.as_str() {
        Location::LOCAL_SCHEME => target.path().to_owned(),
        Location::REMOTE_SCHEME => target.location.authority.clone(),
        scheme => format!("{scheme}://{}", target.location.authority),
    }
}

#[cfg(test)]
mod tests {
    use std::fs;

    use super::*;

    fn path(p: &std::path::Path) -> String {
        p.to_str().unwrap().to_owned()
    }

    #[tokio::test]
    async fn lists_with_locations_and_crumbs() {
        let dir = tempfile::tempdir().unwrap();
        fs::create_dir(dir.path().join("sub")).unwrap();
        fs::write(dir.path().join("a.txt"), "a").unwrap();
        let registry = Registry::default();

        let listing = list_dir(&registry, &path(dir.path())).await.unwrap();
        let mut names: Vec<_> = listing
            .items
            .iter()
            .map(|i| i.entry.name.as_str())
            .collect();
        names.sort();
        assert_eq!(names, ["a.txt", "sub"]);
        let sub = listing
            .items
            .iter()
            .find(|i| i.entry.name == "sub")
            .unwrap();
        assert_eq!(sub.location, path(&dir.path().join("sub")));

        // Breadcrumbs start at the file system root: `/`, or a drive such as `C:\`.
        let root = &listing.crumbs[0];
        assert!(
            std::path::Path::new(&root.location).parent().is_none(),
            "{root:?}"
        );
        assert_eq!(root.name, root.location);
        assert_eq!(listing.crumbs.last().unwrap().location, listing.location);
        assert!(listing.capabilities.write);

        let file = list_dir(&registry, &path(&dir.path().join("a.txt"))).await;
        assert!(matches!(file, Err(Error::InvalidArgument(_))));
    }

    #[tokio::test]
    async fn mkdir_and_rename() {
        let dir = tempfile::tempdir().unwrap();
        let registry = Registry::default();
        let root = path(dir.path());

        let folder = mkdir(&registry, &root, "New folder").await.unwrap();
        assert!(dir.path().join("New folder").is_dir());
        assert!(mkdir(&registry, &root, "New folder").await.is_err());
        assert!(mkdir(&registry, &root, "a/b").await.is_err());

        let renamed = rename(&registry, &folder, "Renamed").await.unwrap();
        assert_eq!(renamed, path(&dir.path().join("Renamed")));
        assert!(!dir.path().join("New folder").exists());

        fs::write(dir.path().join("taken"), "").unwrap();
        assert!(matches!(
            rename(&registry, &renamed, "taken").await,
            Err(Error::AlreadyExists(_))
        ));
    }

    #[test]
    fn opening_is_local_only() {
        let registry = Registry::default();
        #[cfg(unix)]
        assert_eq!(local_path(&registry, "/tmp").unwrap(), "/tmp");
        assert!(local_path(&registry, "sftp://host/x").is_err());
    }
}
