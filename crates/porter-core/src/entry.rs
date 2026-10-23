use chrono::{DateTime, Utc};
use serde::Serialize;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum EntryKind {
    File,
    Dir,
    Symlink,
    Other,
}

/// Normalized metadata of a file, directory or object.
#[derive(Debug, Clone, Serialize)]
pub struct Entry {
    pub name: String,
    /// Backend-native path of the entry.
    pub path: String,
    pub kind: EntryKind,
    pub size: Option<u64>,
    pub modified: Option<DateTime<Utc>>,
    pub created: Option<DateTime<Utc>>,
    pub accessed: Option<DateTime<Utc>>,
    pub permissions: Option<Permissions>,
    pub hidden: bool,
    pub symlink_target: Option<String>,
}

impl Entry {
    pub fn is_dir(&self) -> bool {
        self.kind == EntryKind::Dir
    }
}

#[derive(Debug, Clone, Default, Serialize)]
pub struct Permissions {
    /// POSIX permission bits without the file-type bits (e.g. `0o755`).
    pub mode: Option<u32>,
    pub readonly: bool,
    pub uid: Option<u32>,
    pub gid: Option<u32>,
}

impl Permissions {
    /// Renders an `ls -l` style string such as `drwxr-xr-x`.
    pub fn to_unix_string(&self, kind: EntryKind) -> String {
        let type_char = match kind {
            EntryKind::Dir => 'd',
            EntryKind::Symlink => 'l',
            EntryKind::File => '-',
            EntryKind::Other => '?',
        };
        let mut s = String::with_capacity(10);
        s.push(type_char);
        match self.mode {
            Some(mode) => {
                const BITS: [(u32, char); 9] = [
                    (0o400, 'r'),
                    (0o200, 'w'),
                    (0o100, 'x'),
                    (0o040, 'r'),
                    (0o020, 'w'),
                    (0o010, 'x'),
                    (0o004, 'r'),
                    (0o002, 'w'),
                    (0o001, 'x'),
                ];
                for (bit, c) in BITS {
                    s.push(if mode & bit != 0 { c } else { '-' });
                }
            }
            // No POSIX mode (e.g. Windows): approximate from the read-only flag.
            None if self.readonly => s.push_str("r--r--r--"),
            None => s.push_str("rw-rw-rw-"),
        }
        s
    }
}

/// Information about the file system (volume, bucket, share) holding a location.
#[derive(Debug, Clone, Default, Serialize)]
pub struct FsInfo {
    pub name: Option<String>,
    pub fs_type: Option<String>,
    pub mount_point: Option<String>,
    pub total_bytes: Option<u64>,
    pub available_bytes: Option<u64>,
    pub read_only: Option<bool>,
    pub removable: Option<bool>,
}

impl FsInfo {
    pub fn used_bytes(&self) -> Option<u64> {
        Some(self.total_bytes?.saturating_sub(self.available_bytes?))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unix_string_from_mode() {
        let p = Permissions {
            mode: Some(0o754),
            ..Default::default()
        };
        assert_eq!(p.to_unix_string(EntryKind::Dir), "drwxr-xr--");
        assert_eq!(p.to_unix_string(EntryKind::File), "-rwxr-xr--");
    }

    #[test]
    fn unix_string_without_mode() {
        let p = Permissions {
            readonly: true,
            ..Default::default()
        };
        assert_eq!(p.to_unix_string(EntryKind::File), "-r--r--r--");
    }
}
