//! Runs copy/move/delete/chmod jobs in the background and reports progress as events.

use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use porter_backends::Registry;
use porter_core::{ModeSpec, Result, Target};
use porter_jobs::{Conflict, CopyOptions, DeleteOptions, Job, Report, Snapshot};
use serde::{Deserialize, Serialize};

const PROGRESS_INTERVAL: Duration = Duration::from_millis(150);

#[derive(Debug, Deserialize)]
#[serde(tag = "kind", rename_all = "lowercase")]
pub enum JobRequest {
    Copy {
        sources: Vec<String>,
        dest: String,
        #[serde(default)]
        conflict: ConflictPolicy,
    },
    Move {
        sources: Vec<String>,
        dest: String,
        #[serde(default)]
        conflict: ConflictPolicy,
    },
    Delete {
        targets: Vec<String>,
    },
    Chmod {
        targets: Vec<String>,
        mode: String,
        #[serde(default)]
        recursive: bool,
    },
}

#[derive(Debug, Default, Clone, Copy, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ConflictPolicy {
    #[default]
    Fail,
    Skip,
    Overwrite,
    Rename,
    Newer,
}

impl From<ConflictPolicy> for Conflict {
    fn from(policy: ConflictPolicy) -> Self {
        match policy {
            ConflictPolicy::Fail => Conflict::Fail,
            ConflictPolicy::Skip => Conflict::Skip,
            ConflictPolicy::Overwrite => Conflict::Overwrite,
            ConflictPolicy::Rename => Conflict::Rename,
            ConflictPolicy::Newer => Conflict::Newer,
        }
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct JobInfo {
    pub id: u64,
    pub kind: &'static str,
    pub title: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct ProgressEvent {
    pub id: u64,
    pub snapshot: Snapshot,
    pub current: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct JobSummary {
    pub id: u64,
    pub files: u64,
    pub dirs: u64,
    pub bytes: u64,
    pub skipped: u64,
    pub errors: Vec<String>,
    pub cancelled: bool,
}

/// Where job events go: the UI in the app, a collector in tests.
pub trait EventSink: Send + Sync + 'static {
    fn started(&self, info: &JobInfo);
    fn progress(&self, event: &ProgressEvent);
    fn finished(&self, summary: &JobSummary);
}

enum Operation {
    Copy(Vec<Target>, Target, CopyOptions),
    Move(Vec<Target>, Target, CopyOptions),
    Delete(Vec<Target>),
    Chmod(Vec<Target>, ModeSpec, bool),
}

#[derive(Default)]
pub struct JobManager {
    next_id: AtomicU64,
    running: Mutex<HashMap<u64, Job>>,
}

impl JobManager {
    /// Validates the request (bad locations or modes fail right away), then runs it
    /// in the background.
    pub fn start(
        self: &Arc<Self>,
        registry: &Registry,
        request: JobRequest,
        sink: Arc<dyn EventSink>,
    ) -> Result<JobInfo> {
        let resolve = |locations: &[String]| -> Result<Vec<Target>> {
            locations.iter().map(|l| registry.target(l)).collect()
        };
        let copy_opts = |conflict: ConflictPolicy| CopyOptions {
            recursive: true,
            conflict: conflict.into(),
            ..Default::default()
        };
        let (kind, operation) = match request {
            JobRequest::Copy {
                sources,
                dest,
                conflict,
            } => (
                "copy",
                Operation::Copy(
                    resolve(&sources)?,
                    registry.target(&dest)?,
                    copy_opts(conflict),
                ),
            ),
            JobRequest::Move {
                sources,
                dest,
                conflict,
            } => (
                "move",
                Operation::Move(
                    resolve(&sources)?,
                    registry.target(&dest)?,
                    copy_opts(conflict),
                ),
            ),
            JobRequest::Delete { targets } => ("delete", Operation::Delete(resolve(&targets)?)),
            JobRequest::Chmod {
                targets,
                mode,
                recursive,
            } => (
                "chmod",
                Operation::Chmod(resolve(&targets)?, ModeSpec::parse(&mode)?, recursive),
            ),
        };

        let info = JobInfo {
            id: self.next_id.fetch_add(1, Ordering::Relaxed) + 1,
            kind,
            title: title(&operation),
        };
        let job = Job::new();
        self.running.lock().unwrap().insert(info.id, job.clone());
        sink.started(&info);

        let manager = Arc::clone(self);
        let id = info.id;
        tokio::spawn(async move {
            let ticker = tokio::spawn({
                let (job, sink) = (job.clone(), sink.clone());
                async move {
                    loop {
                        tokio::time::sleep(PROGRESS_INTERVAL).await;
                        sink.progress(&progress(id, &job));
                    }
                }
            });
            let report = run(&job, operation).await;
            ticker.abort();
            sink.progress(&progress(id, &job));
            manager.running.lock().unwrap().remove(&id);
            sink.finished(&summary(id, report));
        });
        Ok(info)
    }

    /// Returns whether a running job with this id was found.
    pub fn cancel(&self, id: u64) -> bool {
        match self.running.lock().unwrap().get(&id) {
            Some(job) => {
                job.cancel();
                true
            }
            None => false,
        }
    }
}

async fn run(job: &Job, operation: Operation) -> Result<Report> {
    match operation {
        Operation::Copy(sources, dest, opts) => {
            porter_jobs::copy(job, &sources, &dest, &opts).await
        }
        Operation::Move(sources, dest, opts) => {
            porter_jobs::move_to(job, &sources, &dest, &opts).await
        }
        Operation::Delete(targets) => {
            let opts = DeleteOptions {
                recursive: true,
                ..Default::default()
            };
            Ok(porter_jobs::delete(job, &targets, &opts).await)
        }
        Operation::Chmod(targets, spec, recursive) => {
            Ok(porter_jobs::chmod(job, &targets, &spec, recursive).await)
        }
    }
}

fn progress(id: u64, job: &Job) -> ProgressEvent {
    ProgressEvent {
        id,
        snapshot: job.progress().snapshot(),
        current: job.progress().current(),
    }
}

fn summary(id: u64, report: Result<Report>) -> JobSummary {
    match report {
        Ok(report) => JobSummary {
            id,
            files: report.files,
            dirs: report.dirs,
            bytes: report.bytes,
            skipped: report.skipped,
            errors: report.errors.iter().map(ToString::to_string).collect(),
            cancelled: report.cancelled,
        },
        Err(e) => JobSummary {
            id,
            files: 0,
            dirs: 0,
            bytes: 0,
            skipped: 0,
            errors: vec![e.to_string()],
            cancelled: false,
        },
    }
}

/// e.g. `Copying “a.txt” to nas:/photos` or `Deleting 3 items`.
fn title(operation: &Operation) -> String {
    let what = |targets: &[Target]| match targets {
        [one] => format!("“{}”", one.file_name().unwrap_or_else(|| one.to_string())),
        many => format!("{} items", many.len()),
    };
    match operation {
        Operation::Copy(sources, dest, _) => format!("Copying {} to {dest}", what(sources)),
        Operation::Move(sources, dest, _) => format!("Moving {} to {dest}", what(sources)),
        Operation::Delete(targets) => format!("Deleting {}", what(targets)),
        Operation::Chmod(targets, _, _) => format!("Changing permissions of {}", what(targets)),
    }
}

#[cfg(test)]
mod tests {
    use std::fs;

    use tokio::sync::mpsc;

    use super::*;

    struct Collector(mpsc::UnboundedSender<JobSummary>);

    impl EventSink for Collector {
        fn started(&self, _: &JobInfo) {}
        fn progress(&self, _: &ProgressEvent) {}
        fn finished(&self, summary: &JobSummary) {
            let _ = self.0.send(summary.clone());
        }
    }

    #[tokio::test]
    async fn runs_jobs_and_reports() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("a.txt");
        fs::write(&file, "hello").unwrap();
        fs::create_dir(dir.path().join("dst")).unwrap();
        let (tx, mut rx) = mpsc::unbounded_channel();
        let sink = Arc::new(Collector(tx));
        let manager = Arc::new(JobManager::default());
        let registry = Registry::default();
        let loc = |p: &std::path::Path| p.to_str().unwrap().to_owned();

        let request: JobRequest = serde_json::from_value(serde_json::json!({
            "kind": "copy",
            "sources": [loc(&file)],
            "dest": loc(&dir.path().join("dst")),
            "conflict": "rename",
        }))
        .unwrap();
        let info = manager.start(&registry, request, sink.clone()).unwrap();
        assert_eq!(
            info.title,
            format!("Copying “a.txt” to {}", loc(&dir.path().join("dst")))
        );
        let done = rx.recv().await.unwrap();
        assert_eq!((done.id, done.files, done.bytes), (info.id, 1, 5));
        assert!(dir.path().join("dst/a.txt").exists());
        assert!(!manager.cancel(info.id), "finished jobs are gone");

        let delete = JobRequest::Delete {
            targets: vec![loc(&dir.path().join("dst"))],
        };
        manager.start(&registry, delete, sink.clone()).unwrap();
        let done = rx.recv().await.unwrap();
        assert!(done.errors.is_empty(), "{:?}", done.errors);
        assert!(!dir.path().join("dst").exists());

        let bad = JobRequest::Chmod {
            targets: vec![loc(&file)],
            mode: "u+q".into(),
            recursive: false,
        };
        assert!(manager.start(&registry, bad, sink).is_err());
    }
}
