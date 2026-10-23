//! Long-running file operations with shared progress reporting and cancellation.
//!
//! Every operation takes a [`Job`]; front ends poll [`Job::progress`] to draw progress
//! and call [`Job::cancel`] to stop. Failures of individual items are collected in the
//! returned [`Report`] instead of aborting the whole operation.

mod chmod;
mod copy;
mod delete;
mod job;
mod search;
mod walk;

pub use chmod::chmod;
pub use copy::{Conflict, CopyOptions, copy, move_to};
pub use delete::{DeleteOptions, delete};
pub use job::{Job, Progress, Report, Snapshot};
pub use search::{SearchOptions, SizeFilter, parse_time, search};

use porter_core::{Error, Result, Target};

/// Creates a directory; with `parents`, also creates missing ancestors and accepts an
/// existing directory.
pub async fn mkdir(target: &Target, parents: bool) -> Result<()> {
    if !parents {
        return target.provider.mkdir(target.path()).await;
    }
    let mut missing = Vec::new();
    let mut current = Some(target.clone());
    while let Some(dir) = current {
        match dir.provider.stat_follow(dir.path()).await {
            Ok(entry) if entry.is_dir() => break,
            Ok(_) => return Err(Error::AlreadyExists(dir.to_string())),
            Err(Error::NotFound(_)) => {
                current = dir.parent();
                missing.push(dir);
            }
            Err(e) => return Err(e),
        }
    }
    for dir in missing.into_iter().rev() {
        match dir.provider.mkdir(dir.path()).await {
            // Someone else may have created it in the meantime.
            Ok(()) | Err(Error::AlreadyExists(_)) => {}
            Err(e) => return Err(e),
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn assert_send<T: Send>(_: T) {}

    /// Front ends spawn these on a multi-threaded runtime; this only has to compile.
    #[allow(dead_code)]
    fn futures_are_send(job: &Job, targets: &[Target], spec: &porter_core::ModeSpec) {
        let opts = CopyOptions::default();
        assert_send(copy(job, targets, &targets[0], &opts));
        assert_send(move_to(job, targets, &targets[0], &opts));
        assert_send(delete(job, targets, &DeleteOptions::default()));
        assert_send(chmod(job, targets, spec, true));
        assert_send(mkdir(&targets[0], true));
        assert_send(search(job, &targets[0], &SearchOptions::default(), |_| {}));
    }
}
