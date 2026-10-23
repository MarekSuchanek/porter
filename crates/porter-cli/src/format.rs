//! Human-readable output.

use chrono::{DateTime, Local, Utc};
use porter_core::{Entry, EntryKind, FsInfo};
use porter_jobs::Report;

pub fn display_name(entry: &Entry) -> String {
    match entry.kind {
        EntryKind::Dir => format!("{}/", entry.name),
        _ => entry.name.clone(),
    }
}

pub fn print_long(entries: &[Entry]) {
    let rows: Vec<[String; 4]> = entries
        .iter()
        .map(|e| {
            let mut name = display_name(e);
            if let Some(target) = &e.symlink_target {
                name = format!("{name} -> {target}");
            }
            [
                permissions(e),
                e.size.map(human_bytes).unwrap_or_else(|| "-".into()),
                e.modified.map(local_time).unwrap_or_else(|| "-".into()),
                name,
            ]
        })
        .collect();
    let size_width = rows.iter().map(|r| r[1].len()).max().unwrap_or(0);
    for [perms, size, modified, name] in rows {
        println!("{perms}  {size:>size_width$}  {modified}  {name}");
    }
}

pub fn print_stat(entry: &Entry) {
    let field = |key: &str, value: &str| println!("{key:>9}: {value}");
    let kind = match entry.kind {
        EntryKind::File => "file",
        EntryKind::Dir => "directory",
        EntryKind::Symlink => "symlink",
        EntryKind::Other => "other",
    };

    field("Path", &entry.path);
    field("Kind", kind);
    if let Some(target) = &entry.symlink_target {
        field("Target", target);
    }
    if let Some(size) = entry.size {
        field("Size", &format!("{} ({size} bytes)", human_bytes(size)));
    }
    if let Some(p) = &entry.permissions {
        let octal = p.mode.map(|m| format!(" ({m:04o})")).unwrap_or_default();
        field("Perms", &format!("{}{octal}", permissions(entry)));
        if let (Some(uid), Some(gid)) = (p.uid, p.gid) {
            field("Owner", &format!("uid {uid}, gid {gid}"));
        }
    }
    for (key, time) in [
        ("Modified", entry.modified),
        ("Created", entry.created),
        ("Accessed", entry.accessed),
    ] {
        if let Some(time) = time {
            field(key, &local_time(time));
        }
    }
    field("Hidden", if entry.hidden { "yes" } else { "no" });
}

pub fn print_df(infos: &[FsInfo]) {
    let opt_bytes = |b: Option<u64>| b.map(human_bytes).unwrap_or_else(|| "-".into());
    let mut rows = vec![[
        "Mounted on".to_owned(),
        "Type".into(),
        "Size".into(),
        "Used".into(),
        "Avail".into(),
        "Use%".into(),
    ]];
    for info in infos {
        let use_pct = match (info.used_bytes(), info.total_bytes) {
            (Some(used), Some(total)) if total > 0 => format!("{}%", used * 100 / total),
            _ => "-".into(),
        };
        rows.push([
            info.mount_point.clone().unwrap_or_else(|| "-".into()),
            info.fs_type.clone().unwrap_or_else(|| "-".into()),
            opt_bytes(info.total_bytes),
            opt_bytes(info.used_bytes()),
            opt_bytes(info.available_bytes),
            use_pct,
        ]);
    }

    let mut widths = [0; 6];
    for row in &rows {
        for (w, cell) in widths.iter_mut().zip(row) {
            *w = (*w).max(cell.chars().count());
        }
    }
    for [mount, fs_type, size, used, avail, pct] in rows {
        let [w0, w1, w2, w3, w4, w5] = widths;
        println!(
            "{mount:<w0$}  {fs_type:<w1$}  {size:>w2$}  {used:>w3$}  {avail:>w4$}  {pct:>w5$}"
        );
    }
}

/// One-line outcome such as `Copied 12 files, 3 directories (4.2M), 1 skipped`.
pub fn summary(verb: &str, report: &Report) -> String {
    let plural = |n: u64, one: &str, many: &str| format!("{n} {}", if n == 1 { one } else { many });
    let mut line = format!("{verb} {}", plural(report.files, "file", "files"));
    if report.dirs > 0 {
        line += &format!(", {}", plural(report.dirs, "directory", "directories"));
    }
    if report.bytes > 0 {
        line += &format!(" ({})", human_bytes(report.bytes));
    }
    if report.skipped > 0 {
        line += &format!(", {} skipped", report.skipped);
    }
    if !report.errors.is_empty() {
        line += &format!(", {} failed", report.errors.len());
    }
    line
}

fn permissions(entry: &Entry) -> String {
    entry
        .permissions
        .as_ref()
        .map(|p| p.to_unix_string(entry.kind))
        .unwrap_or_else(|| "-".repeat(10))
}

fn local_time(time: DateTime<Utc>) -> String {
    time.with_timezone(&Local)
        .format("%Y-%m-%d %H:%M")
        .to_string()
}

/// Formats a byte count with binary units: `512B`, `1.5K`, `20M`.
pub fn human_bytes(bytes: u64) -> String {
    const UNITS: [&str; 7] = ["B", "K", "M", "G", "T", "P", "E"];
    let mut value = bytes as f64;
    let mut unit = 0;
    while value >= 1024.0 && unit < UNITS.len() - 1 {
        value /= 1024.0;
        unit += 1;
    }
    if unit == 0 {
        format!("{bytes}B")
    } else if value < 10.0 {
        format!("{value:.1}{}", UNITS[unit])
    } else {
        format!("{value:.0}{}", UNITS[unit])
    }
}

#[cfg(test)]
mod tests {
    use super::human_bytes;

    #[test]
    fn human_bytes_units() {
        assert_eq!(human_bytes(0), "0B");
        assert_eq!(human_bytes(1023), "1023B");
        assert_eq!(human_bytes(1536), "1.5K");
        assert_eq!(human_bytes(20 * 1024 * 1024), "20M");
        assert_eq!(human_bytes(u64::MAX), "16E");
    }
}
