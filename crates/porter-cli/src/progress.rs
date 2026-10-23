//! Progress bars for long-running jobs.

use std::future::Future;
use std::time::Duration;

use indicatif::{ProgressBar, ProgressStyle};
use porter_jobs::{Job, Snapshot};

const REFRESH: Duration = Duration::from_millis(100);

#[derive(Clone, Copy, PartialEq, Eq)]
enum Mode {
    Scanning,
    Bytes,
    Items,
}

/// Runs `work` while drawing the job's progress on stderr (hidden when stderr is not a
/// terminal). Ctrl+C cancels the job; it then stops at the next item or chunk.
pub async fn track<T>(job: &Job, verb: &'static str, work: impl Future<Output = T>) -> T {
    let bar = ProgressBar::new(0).with_style(style(Mode::Scanning));
    let ticker = tokio::spawn({
        let (job, bar) = (job.clone(), bar.clone());
        async move {
            let mut mode = Some(Mode::Scanning);
            let mut interval = tokio::time::interval(REFRESH);
            loop {
                interval.tick().await;
                draw(&bar, &job.progress().snapshot(), verb, &mut mode);
            }
        }
    });
    let interrupt = tokio::spawn({
        let job = job.clone();
        async move {
            if tokio::signal::ctrl_c().await.is_ok() {
                job.cancel();
            }
        }
    });

    let output = work.await;
    ticker.abort();
    interrupt.abort();
    bar.finish_and_clear();
    output
}

fn draw(bar: &ProgressBar, s: &Snapshot, verb: &str, mode: &mut Option<Mode>) {
    let next = if s.scanning || s.files_total == 0 {
        Mode::Scanning
    } else if s.bytes_total > 0 {
        Mode::Bytes
    } else {
        Mode::Items
    };
    match next {
        Mode::Scanning => {
            bar.set_message(format!("Scanning… {} items", s.scanned));
            bar.tick();
        }
        Mode::Bytes => {
            bar.set_length(s.bytes_total);
            bar.set_position(s.bytes_done);
            bar.set_message(format!("{verb} {}/{}", s.files_done, s.files_total));
        }
        Mode::Items => {
            bar.set_length(s.files_total);
            bar.set_position(s.files_done);
            bar.set_message(verb.to_owned());
        }
    }
    // Switch styles after updating, so the first frame already has its values.
    if *mode != Some(next) {
        bar.set_style(style(next));
        *mode = Some(next);
    }
}

fn style(mode: Mode) -> ProgressStyle {
    let template = match mode {
        Mode::Scanning => "{spinner} {msg}",
        Mode::Bytes => {
            "{msg} [{bar:30}] {binary_bytes}/{binary_total_bytes} {binary_bytes_per_sec} ETA {eta}"
        }
        Mode::Items => "{msg} [{bar:30}] {pos}/{len} ETA {eta}",
    };
    ProgressStyle::with_template(template)
        .expect("valid progress template")
        .progress_chars("=> ")
}
