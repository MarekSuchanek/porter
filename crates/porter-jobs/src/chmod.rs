use porter_core::{EntryKind, ModeSpec, Target};

use crate::walk::walk;
use crate::{Job, Report};

/// Changes permission bits. Named targets that are symlinks are followed; with
/// `recursive`, symlinks found inside directories are skipped like `chmod -R` does.
pub async fn chmod(job: &Job, targets: &[Target], spec: &ModeSpec, recursive: bool) -> Report {
    let mut report = Report::default();
    let mut items = Vec::new();

    job.progress().set_scanning(true);
    for target in targets {
        match target.provider.stat_follow(target.path()).await {
            Ok(entry) if recursive => {
                let tree = walk(
                    job,
                    target.provider.as_ref(),
                    entry,
                    (),
                    |_, _| (),
                    &mut report,
                )
                .await;
                items.extend(
                    tree.into_iter()
                        .map(|(e, ())| (target.with_path(e.path.clone()), e)),
                );
            }
            Ok(entry) => items.push((target.clone(), entry)),
            Err(e) => report.record(e),
        }
    }
    job.progress().set_scanning(false);
    job.progress().planned(items.len() as u64, 0);

    // Deepest first, so taking access away from a directory doesn't block its contents.
    for (target, entry) in items.into_iter().rev() {
        if job.is_cancelled() {
            report.cancelled = true;
            break;
        }
        job.progress().set_current(target.path());
        let current = entry.permissions.as_ref().and_then(|p| p.mode);
        let mode = spec.apply(current.unwrap_or(0), entry.is_dir());
        if entry.kind == EntryKind::Symlink || current == Some(mode) {
            report.skipped += 1;
        } else {
            match target.provider.set_permissions(target.path(), mode).await {
                Ok(()) if entry.is_dir() => report.dirs += 1,
                Ok(()) => report.files += 1,
                Err(e) => report.record(e),
            }
        }
        job.progress().file_done();
    }
    report
}
