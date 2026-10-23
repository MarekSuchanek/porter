//! Recursive search for the UI: matches stream to the window in small batches.

use std::sync::{Arc, Mutex};
use std::time::Duration;

use porter_backends::Registry;
use porter_core::{EntryKind, Result};
use porter_jobs::{Job, SearchOptions, SizeFilter, parse_time};
use serde::{Deserialize, Serialize};

use crate::api::Item;
use crate::jobs::{EventSink, JobManager};

/// Matches are sent at most this often, so big result sets don't flood the UI.
const BATCH_INTERVAL: Duration = Duration::from_millis(120);
/// Searches stop here; the UI says the results were limited.
pub const RESULT_LIMIT: usize = 5_000;

#[derive(Debug, Deserialize)]
pub struct SearchRequest {
    pub location: String,
    /// Glob (`*.pdf`) or text contained in the name; empty matches everything.
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub kind: Option<KindFilter>,
    /// Size spec such as `+100M`.
    #[serde(default)]
    pub min_size: Option<String>,
    /// Age spec such as `7d`.
    #[serde(default)]
    pub newer: Option<String>,
    #[serde(default)]
    pub hidden: bool,
}

#[derive(Debug, Clone, Copy, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum KindFilter {
    File,
    Dir,
}

#[derive(Debug, Clone, Serialize)]
pub struct SearchProgress {
    pub id: u64,
    /// Matches since the previous event.
    pub items: Vec<Item>,
    pub scanned: u64,
    pub matched: u64,
}

#[derive(Debug, Clone, Serialize)]
pub struct SearchSummary {
    pub id: u64,
    pub matched: u64,
    pub scanned: u64,
    /// Folders that could not be read.
    pub unreadable: usize,
    pub cancelled: bool,
    pub limited: bool,
    /// Why the search could not run at all.
    pub error: Option<String>,
}

impl JobManager {
    /// Validates the request, then searches in the background; returns the search id,
    /// which [`cancel`](JobManager::cancel) accepts.
    pub fn search(
        self: &Arc<Self>,
        registry: &Registry,
        request: SearchRequest,
        sink: Arc<dyn EventSink>,
    ) -> Result<u64> {
        let root = registry.target(&request.location)?;
        let now = chrono::Utc::now();
        let name = request.name.trim();
        let opts = SearchOptions {
            name: (!name.is_empty()).then(|| name.to_owned()),
            kind: request.kind.map(|k| match k {
                KindFilter::File => EntryKind::File,
                KindFilter::Dir => EntryKind::Dir,
            }),
            sizes: request
                .min_size
                .as_deref()
                .map(SizeFilter::parse)
                .transpose()?
                .into_iter()
                .collect(),
            newer: request
                .newer
                .as_deref()
                .map(|t| parse_time(t, now))
                .transpose()?,
            hidden: request.hidden,
            limit: Some(RESULT_LIMIT),
            ..Default::default()
        };

        let (id, job) = self.register();
        let manager = Arc::clone(self);
        tokio::spawn(async move {
            let batch = Arc::new(Mutex::new(Vec::new()));
            let ticker = tokio::spawn({
                let (batch, job, sink) = (batch.clone(), job.clone(), sink.clone());
                async move {
                    loop {
                        tokio::time::sleep(BATCH_INTERVAL).await;
                        flush(id, &batch, &job, sink.as_ref());
                    }
                }
            });
            let report = porter_jobs::search(&job, &root, &opts, |entry| {
                let location = root.with_path(entry.path.clone()).to_string();
                batch.lock().unwrap().push(Item { entry, location });
            })
            .await;
            ticker.abort();
            flush(id, &batch, &job, sink.as_ref());
            manager.unregister(id);

            let scanned = job.progress().snapshot().scanned;
            sink.search_finished(&match report {
                Ok(report) => SearchSummary {
                    id,
                    matched: report.files,
                    scanned,
                    unreadable: report.errors.len(),
                    cancelled: report.cancelled,
                    limited: report.files as usize >= RESULT_LIMIT,
                    error: None,
                },
                Err(e) => SearchSummary {
                    id,
                    matched: 0,
                    scanned,
                    unreadable: 0,
                    cancelled: false,
                    limited: false,
                    error: Some(e.to_string()),
                },
            });
        });
        Ok(id)
    }
}

fn flush(id: u64, batch: &Mutex<Vec<Item>>, job: &Job, sink: &dyn EventSink) {
    let items = std::mem::take(&mut *batch.lock().unwrap());
    let snapshot = job.progress().snapshot();
    sink.search_progress(&SearchProgress {
        id,
        items,
        scanned: snapshot.scanned,
        matched: snapshot.files_done,
    });
}

#[cfg(test)]
mod tests {
    use std::fs;

    use tokio::sync::mpsc;

    use super::*;
    use crate::jobs::{JobInfo, JobSummary, ProgressEvent};

    #[derive(Default)]
    struct Collector {
        found: Mutex<Vec<String>>,
        done: Mutex<Option<mpsc::UnboundedSender<SearchSummary>>>,
    }

    impl EventSink for Collector {
        fn started(&self, _: &JobInfo) {}
        fn progress(&self, _: &ProgressEvent) {}
        fn finished(&self, _: &JobSummary) {}
        fn search_progress(&self, event: &SearchProgress) {
            let mut found = self.found.lock().unwrap();
            found.extend(event.items.iter().map(|i| i.entry.name.clone()));
        }
        fn search_finished(&self, summary: &SearchSummary) {
            if let Some(tx) = &*self.done.lock().unwrap() {
                let _ = tx.send(summary.clone());
            }
        }
    }

    #[tokio::test]
    async fn streams_matches_and_summarizes() {
        let dir = tempfile::tempdir().unwrap();
        fs::create_dir_all(dir.path().join("a/b")).unwrap();
        fs::write(dir.path().join("a/report.pdf"), "x").unwrap();
        fs::write(dir.path().join("a/b/Report-2.PDF"), "y").unwrap();
        fs::write(dir.path().join("notes.txt"), "z").unwrap();

        let (tx, mut rx) = mpsc::unbounded_channel();
        let sink = Arc::new(Collector::default());
        *sink.done.lock().unwrap() = Some(tx);
        let manager = Arc::new(JobManager::default());
        let request = SearchRequest {
            location: dir.path().to_str().unwrap().to_owned(),
            name: "*.pdf".into(),
            kind: Some(KindFilter::File),
            min_size: None,
            newer: Some("1d".into()),
            hidden: false,
        };
        manager
            .search(&Registry::default(), request, sink.clone())
            .unwrap();

        let summary = rx.recv().await.unwrap();
        assert_eq!((summary.matched, summary.error.as_deref()), (2, None));
        let mut found = sink.found.lock().unwrap().clone();
        found.sort();
        assert_eq!(found, ["Report-2.PDF", "report.pdf"]);

        let bad = SearchRequest {
            location: dir.path().to_str().unwrap().to_owned(),
            name: String::new(),
            kind: None,
            min_size: Some("100M".into()),
            newer: None,
            hidden: false,
        };
        assert!(manager.search(&Registry::default(), bad, sink).is_err());
    }
}
