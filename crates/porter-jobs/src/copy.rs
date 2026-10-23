use std::path::Path;

use futures::{StreamExt, stream};
use porter_core::{Entry, EntryKind, Error, Reader, Result, Target, Writer};
use tokio::io::{AsyncReadExt, AsyncWriteExt};

use crate::delete::delete_tree;
use crate::walk::walk;
use crate::{Job, Report};

const CHUNK_SIZE: usize = 1 << 20;

/// What to do when a destination file already exists.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum Conflict {
    /// Report an error for that file.
    #[default]
    Fail,
    Skip,
    Overwrite,
    /// Keep both: the new file becomes `name (1).ext`.
    Rename,
    /// Overwrite only if the source is newer.
    Newer,
}

#[derive(Debug, Clone)]
pub struct CopyOptions {
    pub recursive: bool,
    pub conflict: Conflict,
    /// Files transferred in parallel.
    pub concurrency: usize,
    /// Keep modification times and permission bits.
    pub preserve: bool,
}

impl Default for CopyOptions {
    fn default() -> Self {
        Self {
            recursive: false,
            conflict: Conflict::default(),
            concurrency: 4,
            preserve: true,
        }
    }
}

/// Copies `sources` to `dest`. Like `cp`: when `dest` is an existing directory the
/// sources go inside it, otherwise the single source is copied to that path.
/// Existing directories are merged.
pub async fn copy(
    job: &Job,
    sources: &[Target],
    dest: &Target,
    opts: &CopyOptions,
) -> Result<Report> {
    let mut report = Report::default();
    let pairs = resolve_pairs(sources, dest, &mut report).await?;

    let mut plan = Plan::default();
    job.progress().set_scanning(true);
    for (src, entry, dst) in pairs {
        plan_pair(job, src, entry, dst, opts, &mut plan, &mut report).await;
    }
    job.progress().set_scanning(false);

    execute(job, &plan, opts, &mut report).await;
    Ok(report)
}

/// Moves `sources` to `dest` (same destination rules as [`copy`]). Renames when
/// source and destination share a backend, otherwise copies and then deletes the
/// source — only if every item arrived.
pub async fn move_to(
    job: &Job,
    sources: &[Target],
    dest: &Target,
    opts: &CopyOptions,
) -> Result<Report> {
    let mut report = Report::default();
    let pairs = resolve_pairs(sources, dest, &mut report).await?;
    job.progress().planned(pairs.len() as u64, 0);

    for (src, entry, dst) in pairs {
        if job.is_cancelled() {
            report.cancelled = true;
            break;
        }
        job.progress().set_current(src.path());
        match try_rename(&src, &entry, &dst, opts.conflict).await {
            Ok(Some(true)) if entry.is_dir() => report.dirs += 1,
            Ok(Some(true)) => report.files += 1,
            Ok(Some(false)) => report.skipped += 1,
            Ok(None) => move_by_copy(job, src, entry, dst, opts, &mut report).await,
            Err(e) => report.record(e),
        }
        job.progress().file_done();
    }
    Ok(report)
}

struct Item {
    entry: Entry,
    src: Target,
    dst: Target,
}

#[derive(Default)]
struct Plan {
    /// Pre-order, so parents are created before their children.
    dirs: Vec<Item>,
    files: Vec<Item>,
    links: Vec<Item>,
}

enum Outcome {
    Copied(u64),
    Linked,
    Skipped,
}

async fn resolve_pairs(
    sources: &[Target],
    dest: &Target,
    report: &mut Report,
) -> Result<Vec<(Target, Entry, Target)>> {
    let into_dir = match dest.provider.stat_follow(dest.path()).await {
        Ok(entry) => entry.is_dir(),
        Err(Error::NotFound(_)) => false,
        Err(e) => return Err(e),
    };
    if sources.len() > 1 && !into_dir {
        return Err(Error::InvalidArgument(format!(
            "{dest}: target is not a directory"
        )));
    }

    let mut pairs = Vec::with_capacity(sources.len());
    for src in sources {
        match src.provider.stat(src.path()).await {
            Ok(entry) => {
                let dst = match into_dir {
                    true => dest.child(&entry.name),
                    false => dest.clone(),
                };
                pairs.push((src.clone(), entry, dst));
            }
            Err(e) => report.record(e),
        }
    }
    Ok(pairs)
}

async fn plan_pair(
    job: &Job,
    src: Target,
    entry: Entry,
    mut dst: Target,
    opts: &CopyOptions,
    plan: &mut Plan,
    report: &mut Report,
) {
    if !entry.is_dir() {
        plan_leaf(job, Item { entry, src, dst }, plan, report);
        return;
    }
    if !opts.recursive {
        report.record(Error::InvalidArgument(format!(
            "{src}: is a directory (use -r to copy it)"
        )));
        return;
    }
    if src.same_backend(&dst) && Path::new(dst.path()).starts_with(&entry.path) {
        report.record(Error::InvalidArgument(format!(
            "{src}: cannot copy a directory into itself"
        )));
        return;
    }
    match dst.provider.stat_follow(dst.path()).await {
        Ok(existing) if existing.is_dir() => {
            if opts.conflict == Conflict::Rename {
                match unique_name(&dst).await {
                    Ok(renamed) => dst = renamed,
                    Err(e) => return report.record(e),
                }
            }
        }
        Ok(_) => {
            return report.record(Error::InvalidArgument(format!(
                "{dst}: exists and is not a directory"
            )));
        }
        Err(Error::NotFound(_)) => {}
        Err(e) => return report.record(e),
    }

    let tree = walk(
        job,
        src.provider.as_ref(),
        entry,
        dst,
        |parent, child| parent.child(&child.name),
        report,
    )
    .await;
    for (entry, dst) in tree {
        let item = Item {
            src: src.with_path(entry.path.clone()),
            entry,
            dst,
        };
        match item.entry.kind {
            EntryKind::Dir => plan.dirs.push(item),
            _ => plan_leaf(job, item, plan, report),
        }
    }
}

fn plan_leaf(job: &Job, item: Item, plan: &mut Plan, report: &mut Report) {
    match item.entry.kind {
        EntryKind::File => {
            job.progress().planned(1, item.entry.size.unwrap_or(0));
            plan.files.push(item);
        }
        EntryKind::Symlink => {
            job.progress().planned(1, 0);
            plan.links.push(item);
        }
        // Devices, sockets, FIFOs: reading them could block forever.
        EntryKind::Other | EntryKind::Dir => report.record(Error::InvalidArgument(format!(
            "{}: unsupported file type, skipped",
            item.src
        ))),
    }
}

async fn execute(job: &Job, plan: &Plan, opts: &CopyOptions, report: &mut Report) {
    for item in &plan.dirs {
        if job.is_cancelled() {
            report.cancelled = true;
            return;
        }
        match ensure_dir(&item.dst).await {
            Ok(()) => report.dirs += 1,
            Err(e) => report.record(e),
        }
    }

    // Indices rather than `&Item`s: a closure over borrowed items defeats the `Send`
    // proof callers need to spawn the job (rust-lang/rust#102211).
    let outcomes: Vec<_> = stream::iter(0..plan.files.len())
        .map(|i| copy_file(job, &plan.files[i], opts))
        .buffer_unordered(opts.concurrency.max(1))
        .collect()
        .await;
    for outcome in outcomes {
        tally(report, outcome);
    }
    for item in &plan.links {
        let outcome = copy_link(job, item, opts).await;
        job.progress().file_done();
        tally(report, outcome);
    }

    // Directory modes go last and deepest first, so a read-only directory does not
    // block writing its own contents.
    if opts.preserve && !report.cancelled {
        for item in plan.dirs.iter().rev() {
            if let Some(mode) = mode_of(&item.entry) {
                let result = item
                    .dst
                    .provider
                    .set_permissions(item.dst.path(), mode)
                    .await;
                if let Err(e) = ignore_unsupported(result) {
                    report.record(e);
                }
            }
        }
    }
}

fn tally(report: &mut Report, outcome: Result<Outcome>) {
    match outcome {
        Ok(Outcome::Copied(bytes)) => {
            report.files += 1;
            report.bytes += bytes;
        }
        Ok(Outcome::Linked) => report.files += 1,
        Ok(Outcome::Skipped) => report.skipped += 1,
        Err(e) => report.record(e),
    }
}

async fn ensure_dir(dst: &Target) -> Result<()> {
    match dst.provider.mkdir(dst.path()).await {
        Err(Error::AlreadyExists(_))
            if dst
                .provider
                .stat_follow(dst.path())
                .await
                .is_ok_and(|e| e.is_dir()) =>
        {
            Ok(())
        }
        result => result,
    }
}

async fn copy_file(job: &Job, item: &Item, opts: &CopyOptions) -> Result<Outcome> {
    let outcome = copy_file_inner(job, item, opts).await;
    if matches!(outcome, Ok(Outcome::Skipped)) {
        job.progress().bytes_done(item.entry.size.unwrap_or(0));
    }
    job.progress().file_done();
    outcome
}

async fn copy_file_inner(job: &Job, item: &Item, opts: &CopyOptions) -> Result<Outcome> {
    job.check()?;
    let Some(dst) = resolve_conflict(&item.src, &item.entry, &item.dst, opts.conflict).await?
    else {
        return Ok(Outcome::Skipped);
    };
    job.progress().set_current(item.src.path());
    let bytes = transfer(job, &item.src, &dst).await?;

    if opts.preserve {
        if let Some(time) = item.entry.modified {
            ignore_unsupported(dst.provider.set_modified(dst.path(), time).await)?;
        }
        if let Some(mode) = mode_of(&item.entry) {
            ignore_unsupported(dst.provider.set_permissions(dst.path(), mode).await)?;
        }
    }
    Ok(Outcome::Copied(bytes))
}

async fn copy_link(job: &Job, item: &Item, opts: &CopyOptions) -> Result<Outcome> {
    job.check()?;
    let target = item
        .entry
        .symlink_target
        .as_deref()
        .ok_or_else(|| Error::InvalidArgument(format!("{}: unreadable symlink", item.src)))?;
    let Some(dst) = resolve_conflict(&item.src, &item.entry, &item.dst, opts.conflict).await?
    else {
        return Ok(Outcome::Skipped);
    };
    dst.provider.symlink(target, dst.path()).await?;
    Ok(Outcome::Linked)
}

/// Applies the conflict policy. Returns the path to write to — with any existing
/// file there already removed — or `None` to skip.
async fn resolve_conflict(
    src: &Target,
    entry: &Entry,
    dst: &Target,
    policy: Conflict,
) -> Result<Option<Target>> {
    let existing = match dst.provider.stat(dst.path()).await {
        Err(Error::NotFound(_)) => return Ok(Some(dst.clone())),
        Err(e) => return Err(e),
        Ok(existing) => existing,
    };
    if existing.is_dir() {
        return Err(Error::InvalidArgument(format!("{dst}: is a directory")));
    }
    let replace = match policy {
        Conflict::Fail => return Err(Error::AlreadyExists(dst.to_string())),
        Conflict::Rename => return unique_name(dst).await.map(Some),
        Conflict::Skip => false,
        Conflict::Overwrite => true,
        Conflict::Newer => entry.modified > existing.modified,
    };
    if !replace {
        return Ok(None);
    }
    if src.same_backend(dst) && existing.path == entry.path {
        return Err(Error::InvalidArgument(format!(
            "{dst}: source and destination are the same file"
        )));
    }
    // Delete rather than truncate, so an existing symlink is replaced instead of
    // writing through it, and read-only files can be replaced like `rm` would.
    dst.provider.delete(dst.path()).await?;
    Ok(Some(dst.clone()))
}

/// `name.ext` → the first free `name (N).ext` next to it.
async fn unique_name(dst: &Target) -> Result<Target> {
    let cannot = || Error::InvalidArgument(format!("{dst}: cannot derive a new name"));
    let name = dst.file_name().ok_or_else(cannot)?;
    let parent = dst.parent().ok_or_else(cannot)?;
    let (stem, ext) = match name.rfind('.') {
        Some(i) if i > 0 => name.split_at(i),
        _ => (name.as_str(), ""),
    };
    for n in 1u32.. {
        let candidate = parent.child(&format!("{stem} ({n}){ext}"));
        match candidate.provider.stat(candidate.path()).await {
            Err(Error::NotFound(_)) => return Ok(candidate),
            Err(e) => return Err(e),
            Ok(_) => {}
        }
    }
    Err(cannot())
}

async fn transfer(job: &Job, src: &Target, dst: &Target) -> Result<u64> {
    let mut reader = src.provider.read(src.path()).await?;
    let mut writer = dst.provider.write(dst.path(), false).await?;
    let result = pump(job, src, dst, &mut reader, &mut writer).await;
    if result.is_err() {
        drop(writer);
        // Don't leave a truncated file behind.
        let _ = dst.provider.delete(dst.path()).await;
    }
    result
}

async fn pump(
    job: &Job,
    src: &Target,
    dst: &Target,
    reader: &mut Reader,
    writer: &mut Writer,
) -> Result<u64> {
    let mut buf = vec![0; CHUNK_SIZE];
    let mut total = 0;
    loop {
        job.check()?;
        let n = reader
            .read(&mut buf)
            .await
            .map_err(|e| Error::io(src.to_string(), e))?;
        if n == 0 {
            break;
        }
        writer
            .write_all(&buf[..n])
            .await
            .map_err(|e| Error::io(dst.to_string(), e))?;
        total += n as u64;
        job.progress().bytes_done(n as u64);
    }
    writer
        .shutdown()
        .await
        .map_err(|e| Error::io(dst.to_string(), e))?;
    Ok(total)
}

/// `Some(true)` renamed, `Some(false)` skipped, `None` when it has to be copied.
async fn try_rename(
    src: &Target,
    entry: &Entry,
    dst: &Target,
    policy: Conflict,
) -> Result<Option<bool>> {
    if !src.same_backend(dst) || !src.provider.capabilities().rename {
        return Ok(None);
    }
    let dst = if entry.is_dir() {
        if Path::new(dst.path()).starts_with(&entry.path) {
            return Err(Error::InvalidArgument(format!(
                "{src}: cannot move a directory into itself"
            )));
        }
        match dst.provider.stat(dst.path()).await {
            Err(Error::NotFound(_)) => dst.clone(),
            Ok(existing) if existing.is_dir() && policy == Conflict::Rename => {
                unique_name(dst).await?
            }
            // Merging into an existing directory goes through copy.
            Ok(existing) if existing.is_dir() => return Ok(None),
            Ok(_) => {
                return Err(Error::InvalidArgument(format!(
                    "{dst}: exists and is not a directory"
                )));
            }
            Err(e) => return Err(e),
        }
    } else {
        match resolve_conflict(src, entry, dst, policy).await? {
            Some(dst) => dst,
            None => return Ok(Some(false)),
        }
    };
    match src.provider.rename(src.path(), dst.path()).await {
        Ok(()) => Ok(Some(true)),
        Err(Error::CrossDevice(_)) => Ok(None),
        Err(e) => Err(e),
    }
}

async fn move_by_copy(
    job: &Job,
    src: Target,
    entry: Entry,
    dst: Target,
    opts: &CopyOptions,
    report: &mut Report,
) {
    let opts = CopyOptions {
        recursive: true,
        ..opts.clone()
    };
    let (errors, skipped) = (report.errors.len(), report.skipped);
    let mut plan = Plan::default();
    plan_pair(
        job,
        src.clone(),
        entry.clone(),
        dst,
        &opts,
        &mut plan,
        report,
    )
    .await;
    execute(job, &plan, &opts, report).await;

    // Remove the source only if everything arrived.
    if report.errors.len() == errors && report.skipped == skipped && !report.cancelled {
        let mut cleanup = Report::default();
        delete_tree(job, &src, entry, opts.concurrency, &mut cleanup).await;
        report.errors.append(&mut cleanup.errors);
        report.cancelled |= cleanup.cancelled;
    }
}

fn mode_of(entry: &Entry) -> Option<u32> {
    entry.permissions.as_ref()?.mode
}

fn ignore_unsupported(result: Result<()>) -> Result<()> {
    match result {
        Err(Error::Unsupported { .. }) => Ok(()),
        result => result,
    }
}
