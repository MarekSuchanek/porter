//! Recursive search with name, path, type, size and date filters.

use std::collections::VecDeque;
use std::sync::Arc;

use chrono::{DateTime, Local, NaiveDate, TimeDelta, TimeZone, Utc};
use futures::future::BoxFuture;
use futures::stream::{self, FuturesUnordered};
use futures::{FutureExt, StreamExt, TryStreamExt};
use globset::{GlobBuilder, GlobMatcher};
use porter_core::{Entry, EntryKind, EntryStream, Error, Result, Target, WalkOptions};
use regex::{Regex, RegexBuilder};

use crate::{Job, Report};

/// Directories listed at once when a backend has no fast walk of its own.
const LIST_CONCURRENCY: usize = 8;

#[derive(Debug, Clone, Default)]
pub struct SearchOptions {
    /// Glob on the name, e.g. `*.pdf`; text without glob characters matches anywhere
    /// in the name (`report` finds `Q3 report.pdf`).
    pub name: Option<String>,
    /// Regular expression on the name, or on the path below the root with `full_path`.
    pub regex: Option<String>,
    pub full_path: bool,
    pub case_sensitive: bool,
    pub kind: Option<EntryKind>,
    /// All must hold; entries without a size (folders) never match a size filter.
    pub sizes: Vec<SizeFilter>,
    /// Modified at or after this time.
    pub newer: Option<DateTime<Utc>>,
    /// Modified before this time.
    pub older: Option<DateTime<Utc>>,
    /// Include, and descend into, entries whose names start with a dot.
    pub hidden: bool,
    /// Levels below the root to search; `Some(1)` searches only the root's children.
    pub max_depth: Option<usize>,
    /// Stop after this many matches.
    pub limit: Option<usize>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SizeFilter {
    AtLeast(u64),
    AtMost(u64),
}

impl SizeFilter {
    /// `+10M` (at least 10 MB), `-500k` (at most 500 kB). Units: `k`, `M`, `G`, `T`
    /// (powers of 1000) or `Ki`, `Mi`, `Gi`, `Ti` (powers of 1024); none means bytes.
    pub fn parse(spec: &str) -> Result<Self> {
        let invalid = || {
            Error::InvalidArgument(format!(
                "invalid size `{spec}`: use e.g. +10M (at least) or -1G (at most)"
            ))
        };
        let (make, rest): (fn(u64) -> Self, &str) = match spec.split_at_checked(1) {
            Some(("+", rest)) => (Self::AtLeast, rest),
            Some(("-", rest)) => (Self::AtMost, rest),
            _ => return Err(invalid()),
        };
        let split = rest
            .find(|c: char| !(c.is_ascii_digit() || c == '.'))
            .unwrap_or(rest.len());
        let (number, unit) = rest.split_at(split);
        let value: f64 = number.parse().map_err(|_| invalid())?;
        let multiplier: f64 = match unit.to_ascii_lowercase().as_str() {
            "" | "b" => 1.0,
            "k" | "kb" => 1e3,
            "m" | "mb" => 1e6,
            "g" | "gb" => 1e9,
            "t" | "tb" => 1e12,
            "ki" | "kib" => 1024.0,
            "mi" | "mib" => 1024f64.powi(2),
            "gi" | "gib" => 1024f64.powi(3),
            "ti" | "tib" => 1024f64.powi(4),
            _ => return Err(invalid()),
        };
        Ok(make((value * multiplier) as u64))
    }

    fn matches(self, size: u64) -> bool {
        match self {
            Self::AtLeast(min) => size >= min,
            Self::AtMost(max) => size <= max,
        }
    }
}

/// A point in time: relative to `now` (`30min`, `12h`, `7d`, `2w`, `6mo`, `1y`) or a
/// date (`2026-01-31`, local midnight) or an RFC 3339 timestamp.
pub fn parse_time(spec: &str, now: DateTime<Utc>) -> Result<DateTime<Utc>> {
    let invalid = || {
        Error::InvalidArgument(format!(
            "invalid time `{spec}`: use an age like 7d, 12h, 2w, 6mo, 1y or a date like 2026-01-31"
        ))
    };
    if let Ok(date) = NaiveDate::parse_from_str(spec, "%Y-%m-%d") {
        let midnight = date.and_hms_opt(0, 0, 0).ok_or_else(invalid)?;
        return Local
            .from_local_datetime(&midnight)
            .earliest()
            .map(|t| t.to_utc())
            .ok_or_else(invalid);
    }
    if let Ok(time) = DateTime::parse_from_rfc3339(spec) {
        return Ok(time.to_utc());
    }
    let split = spec
        .find(|c: char| !c.is_ascii_digit())
        .ok_or_else(invalid)?;
    let (number, unit) = spec.split_at(split);
    let n: i64 = number.parse().map_err(|_| invalid())?;
    let delta = match unit {
        "s" | "sec" => TimeDelta::try_seconds(n),
        "m" | "min" => TimeDelta::try_minutes(n),
        "h" => TimeDelta::try_hours(n),
        "d" => TimeDelta::try_days(n),
        "w" => TimeDelta::try_weeks(n),
        "mo" => TimeDelta::try_days(n * 30),
        "y" => TimeDelta::try_days(n * 365),
        _ => None,
    }
    .ok_or_else(invalid)?;
    Ok(now - delta)
}

/// The compiled filters.
struct Matcher {
    glob: Option<GlobMatcher>,
    regex: Option<Regex>,
    full_path: bool,
    kind: Option<EntryKind>,
    sizes: Vec<SizeFilter>,
    newer: Option<DateTime<Utc>>,
    older: Option<DateTime<Utc>>,
    root: String,
}

impl Matcher {
    fn new(opts: &SearchOptions, root: &str) -> Result<Self> {
        let glob = match &opts.name {
            Some(name) => {
                let has_glob = name.contains(['*', '?', '[', '{']);
                let pattern = if has_glob {
                    name.clone()
                } else {
                    format!("*{name}*")
                };
                let glob = GlobBuilder::new(&pattern)
                    .case_insensitive(!opts.case_sensitive)
                    .literal_separator(false)
                    .build()
                    .map_err(|e| Error::InvalidArgument(format!("invalid name pattern: {e}")))?;
                Some(glob.compile_matcher())
            }
            None => None,
        };
        let regex = match &opts.regex {
            Some(re) => Some(
                RegexBuilder::new(re)
                    .case_insensitive(!opts.case_sensitive)
                    .build()
                    .map_err(|e| Error::InvalidArgument(format!("invalid regex: {e}")))?,
            ),
            None => None,
        };
        Ok(Self {
            glob,
            regex,
            full_path: opts.full_path,
            kind: opts.kind,
            sizes: opts.sizes.clone(),
            newer: opts.newer,
            older: opts.older,
            root: root.to_owned(),
        })
    }

    /// Filters that need only the name and kind, checked before any metadata is read.
    fn quick(&self, name: &str, kind: EntryKind) -> bool {
        self.kind.is_none_or(|k| k == kind)
            && self.glob.as_ref().is_none_or(|g| g.is_match(name))
            && (self.full_path || self.regex.as_ref().is_none_or(|r| r.is_match(name)))
    }

    /// The remaining filters, on full metadata.
    fn full(&self, entry: &Entry) -> bool {
        if self.full_path {
            let relative = entry
                .path
                .strip_prefix(&self.root)
                .unwrap_or(&entry.path)
                .trim_start_matches(['/', '\\']);
            if !self.regex.as_ref().is_none_or(|r| r.is_match(relative)) {
                return false;
            }
        }
        if !self.sizes.is_empty() {
            match entry.size {
                Some(size) if entry.kind != EntryKind::Dir => {
                    if !self.sizes.iter().all(|f| f.matches(size)) {
                        return false;
                    }
                }
                _ => return false,
            }
        }
        if self.newer.is_some() || self.older.is_some() {
            let Some(modified) = entry.modified else {
                return false;
            };
            if self.newer.is_some_and(|t| modified < t) || self.older.is_some_and(|t| modified >= t)
            {
                return false;
            }
        }
        true
    }
}

/// Searches the tree under `root`, calling `on_match` for each match as soon as it is
/// found (in no particular order). Progress: `scanned` counts examined entries,
/// `files_done` counts matches. Unreadable directories are collected as errors.
pub async fn search(
    job: &Job,
    root: &Target,
    opts: &SearchOptions,
    mut on_match: impl FnMut(Entry) + Send,
) -> Result<Report> {
    let dir = root.provider.stat_follow(root.path()).await?;
    if !dir.is_dir() {
        return Err(Error::InvalidArgument(format!("{root}: not a directory")));
    }
    let matcher = Arc::new(Matcher::new(opts, &dir.path)?);
    let walk_opts = WalkOptions {
        include_hidden: opts.hidden,
        max_depth: opts.max_depth,
        wants: Some({
            let (matcher, job) = (matcher.clone(), job.clone());
            Arc::new(move |name: &str, kind| {
                job.progress().scanned();
                matcher.quick(name, kind)
            })
        }),
    };

    let mut report = Report::default();
    job.progress().set_scanning(true);
    let mut entries = match root.provider.walk(&dir.path, &walk_opts).await? {
        Some(fast) => fast,
        None => listing_walk(root.with_path(dir.path.clone()), walk_opts),
    };
    while let Some(item) = entries.next().await {
        if job.is_cancelled() {
            report.cancelled = true;
            break;
        }
        match item {
            Ok(entry) if matcher.full(&entry) => {
                report.files += 1;
                job.progress().file_done();
                on_match(entry);
                if opts
                    .limit
                    .is_some_and(|limit| report.files as usize >= limit)
                {
                    break;
                }
            }
            Ok(_) => {}
            Err(e) => report.record(e),
        }
    }
    job.progress().set_scanning(false);
    Ok(report)
}

struct ListingWalk {
    root: Target,
    opts: WalkOptions,
    /// Directories still to list, with their depth below the root.
    pending: Vec<(String, usize)>,
    listing: FuturesUnordered<BoxFuture<'static, (usize, Result<Vec<Entry>>)>>,
    ready: VecDeque<Result<Entry>>,
}

/// Walks with [`Provider::list`](porter_core::Provider::list), several directories at a
/// time — the fallback for backends without a fast walk (remotes).
fn listing_walk(root: Target, opts: WalkOptions) -> EntryStream {
    let state = ListingWalk {
        pending: vec![(root.path().to_owned(), 0)],
        root,
        opts,
        listing: FuturesUnordered::new(),
        ready: VecDeque::new(),
    };
    stream::unfold(state, |mut s| async move {
        loop {
            if let Some(item) = s.ready.pop_front() {
                return Some((item, s));
            }
            while s.listing.len() < LIST_CONCURRENCY {
                let Some((dir, depth)) = s.pending.pop() else {
                    break;
                };
                let provider = s.root.provider.clone();
                s.listing.push(
                    async move {
                        let children = async { provider.list(&dir).await?.try_collect().await };
                        (depth, children.await)
                    }
                    .boxed(),
                );
            }
            // Nothing pending and nothing in flight: done.
            let (depth, children) = s.listing.next().await?;
            match children {
                Ok(children) => {
                    for child in children {
                        if !s.opts.include_hidden && child.name.starts_with('.') {
                            continue;
                        }
                        let depth = depth + 1;
                        if child.is_dir() && s.opts.max_depth.is_none_or(|max| depth < max) {
                            s.pending.push((child.path.clone(), depth));
                        }
                        if s.opts.wants(&child.name, child.kind) {
                            s.ready.push_back(Ok(child));
                        }
                    }
                }
                Err(e) => s.ready.push_back(Err(e)),
            }
        }
    })
    .boxed()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sizes() {
        assert_eq!(
            SizeFilter::parse("+10M").unwrap(),
            SizeFilter::AtLeast(10_000_000)
        );
        assert_eq!(
            SizeFilter::parse("-1.5k").unwrap(),
            SizeFilter::AtMost(1500)
        );
        assert_eq!(
            SizeFilter::parse("+2Mi").unwrap(),
            SizeFilter::AtLeast(2 << 20)
        );
        assert_eq!(SizeFilter::parse("+512").unwrap(), SizeFilter::AtLeast(512));
        for bad in ["10M", "+", "+M", "+10X", ""] {
            assert!(SizeFilter::parse(bad).is_err(), "{bad}");
        }
    }

    #[test]
    fn times() {
        let now = DateTime::parse_from_rfc3339("2026-10-23T12:00:00Z")
            .unwrap()
            .to_utc();
        let at = |spec| parse_time(spec, now).unwrap().to_rfc3339();
        assert_eq!(at("7d"), "2026-10-16T12:00:00+00:00");
        assert_eq!(at("90min"), "2026-10-23T10:30:00+00:00");
        assert_eq!(at("2w"), "2026-10-09T12:00:00+00:00");
        assert_eq!(at("2026-01-01T00:00:00Z"), "2026-01-01T00:00:00+00:00");
        assert!(parse_time("2026-01-31", now).is_ok());
        for bad in ["", "7", "d7", "7x", "2026-13-01"] {
            assert!(parse_time(bad, now).is_err(), "{bad}");
        }
    }

    #[test]
    fn name_matching() {
        let matcher = |name: &str, case_sensitive| {
            Matcher::new(
                &SearchOptions {
                    name: Some(name.into()),
                    case_sensitive,
                    ..Default::default()
                },
                "/",
            )
            .unwrap()
        };
        let file = EntryKind::File;
        assert!(matcher("*.pdf", false).quick("Report.PDF", file));
        assert!(!matcher("*.pdf", true).quick("Report.PDF", file));
        assert!(matcher("report", false).quick("Q3 Report final.docx", file));
        assert!(!matcher("report", false).quick("summary.docx", file));
        assert!(
            Matcher::new(
                &SearchOptions {
                    name: Some("[".into()),
                    ..Default::default()
                },
                "/"
            )
            .is_err()
        );
    }
}
