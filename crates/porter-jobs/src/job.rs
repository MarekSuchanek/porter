use std::sync::atomic::{AtomicBool, AtomicU64, Ordering::Relaxed};
use std::sync::{Arc, Mutex};

use porter_core::{Error, Result};
use serde::Serialize;
use tokio_util::sync::CancellationToken;

/// Live counters of a running job. Cheap to update from many tasks and to poll.
#[derive(Debug, Default)]
pub struct Progress {
    scanned: AtomicU64,
    files_total: AtomicU64,
    files_done: AtomicU64,
    bytes_total: AtomicU64,
    bytes_done: AtomicU64,
    scanning: AtomicBool,
    current: Mutex<Option<String>>,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize)]
pub struct Snapshot {
    /// Entries discovered while scanning source trees.
    pub scanned: u64,
    /// Items (files, links, directories…) the job will process.
    pub files_total: u64,
    pub files_done: u64,
    pub bytes_total: u64,
    pub bytes_done: u64,
    pub scanning: bool,
}

impl Progress {
    pub fn snapshot(&self) -> Snapshot {
        Snapshot {
            scanned: self.scanned.load(Relaxed),
            files_total: self.files_total.load(Relaxed),
            files_done: self.files_done.load(Relaxed),
            bytes_total: self.bytes_total.load(Relaxed),
            bytes_done: self.bytes_done.load(Relaxed),
            scanning: self.scanning.load(Relaxed),
        }
    }

    /// The item most recently started.
    pub fn current(&self) -> Option<String> {
        self.current.lock().unwrap().clone()
    }

    pub(crate) fn scanned(&self) {
        self.scanned.fetch_add(1, Relaxed);
    }

    pub(crate) fn planned(&self, files: u64, bytes: u64) {
        self.files_total.fetch_add(files, Relaxed);
        self.bytes_total.fetch_add(bytes, Relaxed);
    }

    pub(crate) fn file_done(&self) {
        self.files_done.fetch_add(1, Relaxed);
    }

    pub(crate) fn bytes_done(&self, bytes: u64) {
        self.bytes_done.fetch_add(bytes, Relaxed);
    }

    pub(crate) fn set_scanning(&self, scanning: bool) {
        self.scanning.store(scanning, Relaxed);
    }

    pub(crate) fn set_current(&self, path: &str) {
        *self.current.lock().unwrap() = Some(path.to_owned());
    }
}

/// Handle to a running operation; clones share progress and cancellation.
#[derive(Debug, Clone, Default)]
pub struct Job {
    progress: Arc<Progress>,
    cancel: CancellationToken,
}

impl Job {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn progress(&self) -> &Progress {
        &self.progress
    }

    /// Requests cancellation; the job stops at the next item or chunk boundary.
    pub fn cancel(&self) {
        self.cancel.cancel();
    }

    pub fn is_cancelled(&self) -> bool {
        self.cancel.is_cancelled()
    }

    pub(crate) fn check(&self) -> Result<()> {
        match self.is_cancelled() {
            true => Err(Error::Cancelled),
            false => Ok(()),
        }
    }
}

/// What an operation did. Per-item failures are collected, not fatal.
#[derive(Debug, Default)]
pub struct Report {
    pub files: u64,
    pub dirs: u64,
    pub bytes: u64,
    pub skipped: u64,
    pub errors: Vec<Error>,
    pub cancelled: bool,
    /// Items a dry run would have processed.
    pub planned: Vec<String>,
}

impl Report {
    pub fn is_success(&self) -> bool {
        self.errors.is_empty() && !self.cancelled
    }

    pub(crate) fn record(&mut self, err: Error) {
        match err {
            Error::Cancelled => self.cancelled = true,
            err => self.errors.push(err),
        }
    }
}
