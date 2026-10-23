//! Long-running file operations with shared progress reporting and cancellation.
//!
//! Every operation takes a [`Job`]; front ends poll [`Job::progress`] to draw progress
//! and call [`Job::cancel`] to stop. Failures of individual items are collected in the
//! returned [`Report`] instead of aborting the whole operation.

mod chmod;
mod copy;
mod delete;
mod job;
mod walk;

pub use chmod::chmod;
pub use copy::{Conflict, CopyOptions, copy, move_to};
pub use delete::{DeleteOptions, delete};
pub use job::{Job, Progress, Report, Snapshot};

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
