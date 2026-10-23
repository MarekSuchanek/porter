use futures::{StreamExt, stream};
use porter_core::{Entry, Error, Target};

use crate::walk::walk;
use crate::{Job, Report};

#[derive(Debug, Clone)]
pub struct DeleteOptions {
    pub recursive: bool,
    /// Only report what would be deleted (in [`Report::planned`]).
    pub dry_run: bool,
    pub concurrency: usize,
}

impl Default for DeleteOptions {
    fn default() -> Self {
        Self {
            recursive: false,
            dry_run: false,
            concurrency: 8,
        }
    }
}

/// Deletes files, symlinks (never their targets) and, with `recursive`, directories.
pub async fn delete(job: &Job, targets: &[Target], opts: &DeleteOptions) -> Report {
    let mut report = Report::default();
    let mut doomed = Vec::new();

    job.progress().set_scanning(true);
    for target in targets {
        if target.parent().is_none() {
            report.record(Error::InvalidArgument(format!(
                "{target}: refusing to delete a root directory"
            )));
            continue;
        }
        let entry = match target.provider.stat(target.path()).await {
            Ok(entry) => entry,
            Err(e) => {
                report.record(e);
                continue;
            }
        };
        if entry.is_dir() && !opts.recursive {
            report.record(Error::InvalidArgument(format!(
                "{target}: is a directory (use -r to delete it)"
            )));
            continue;
        }
        collect(job, target, entry, &mut doomed, &mut report).await;
    }
    job.progress().set_scanning(false);

    if opts.dry_run {
        report.planned = doomed.iter().map(|(t, _)| t.to_string()).collect();
        return report;
    }
    remove_all(job, doomed, opts.concurrency, &mut report).await;
    report
}

/// Deletes a whole tree without the safety checks of [`delete`].
pub(crate) async fn delete_tree(
    job: &Job,
    target: &Target,
    entry: Entry,
    concurrency: usize,
    report: &mut Report,
) {
    let mut doomed = Vec::new();
    collect(job, target, entry, &mut doomed, report).await;
    remove_all(job, doomed, concurrency, report).await;
}

async fn collect(
    job: &Job,
    target: &Target,
    entry: Entry,
    doomed: &mut Vec<(Target, Entry)>,
    report: &mut Report,
) {
    let tree = walk(job, target.provider.as_ref(), entry, (), |_, _| (), report).await;
    doomed.extend(
        tree.into_iter()
            .map(|(entry, ())| (target.with_path(entry.path.clone()), entry)),
    );
}

/// `doomed` is in pre-order: files go first (in parallel), then directories in
/// reverse so children are removed before their parents.
async fn remove_all(
    job: &Job,
    doomed: Vec<(Target, Entry)>,
    concurrency: usize,
    report: &mut Report,
) {
    job.progress().planned(doomed.len() as u64, 0);
    let (dirs, files): (Vec<_>, Vec<_>) = doomed.into_iter().partition(|(_, e)| e.is_dir());

    let results: Vec<_> = stream::iter(files)
        .map(|(target, _)| async move {
            job.check()?;
            job.progress().set_current(target.path());
            let result = target.provider.delete(target.path()).await;
            job.progress().file_done();
            result
        })
        .buffer_unordered(concurrency.max(1))
        .collect()
        .await;
    for result in results {
        match result {
            Ok(()) => report.files += 1,
            Err(e) => report.record(e),
        }
    }

    for (target, _) in dirs.into_iter().rev() {
        if job.is_cancelled() {
            report.cancelled = true;
            break;
        }
        match target.provider.delete(target.path()).await {
            Ok(()) => report.dirs += 1,
            Err(e) => report.record(e),
        }
        job.progress().file_done();
    }
}
