use futures::StreamExt;
use porter_core::{Entry, Provider};

use crate::{Job, Report};

/// Walks the tree under `root` in pre-order (parents before children, siblings by
/// name). Each entry carries a payload derived from its parent's, such as the
/// destination path. Symlinks are not followed. Listing errors are recorded and the
/// walk continues.
pub(crate) async fn walk<T>(
    job: &Job,
    provider: &dyn Provider,
    root: Entry,
    payload: T,
    child_payload: impl Fn(&T, &Entry) -> T,
    report: &mut Report,
) -> Vec<(Entry, T)> {
    let mut out = Vec::new();
    let mut stack = vec![(root, payload)];
    while let Some((entry, payload)) = stack.pop() {
        if job.is_cancelled() {
            report.cancelled = true;
            break;
        }
        job.progress().scanned();
        if entry.is_dir() {
            match provider.list(&entry.path).await {
                Ok(mut listing) => {
                    let mut children = Vec::new();
                    while let Some(child) = listing.next().await {
                        match child {
                            Ok(child) => children.push(child),
                            Err(e) => report.record(e),
                        }
                    }
                    // Reverse order so popping from the stack visits names ascending.
                    children.sort_by(|a, b| b.name.cmp(&a.name));
                    stack.extend(children.into_iter().map(|child| {
                        let p = child_payload(&payload, &child);
                        (child, p)
                    }));
                }
                Err(e) => report.record(e),
            }
        }
        out.push((entry, payload));
    }
    out
}
