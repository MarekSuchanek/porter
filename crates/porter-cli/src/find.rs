//! `porter find`: recursive search with results streamed as they are found.

use std::io::{BufWriter, IsTerminal, Write};
use std::path::Path;
use std::time::{Duration, Instant};

use anyhow::{Result, bail};
use clap::{Args, ValueEnum};
use indicatif::{ProgressBar, ProgressStyle};
use porter_backends::Registry;
use porter_core::{Entry, EntryKind, Target};
use porter_jobs::{Job, SearchOptions, SizeFilter, parse_time, search};
use serde::Serialize;

use crate::format;

#[derive(Args)]
pub struct FindArgs {
    /// Where to search: a path, NAME:/path or a URL
    #[arg(default_value = ".")]
    location: String,
    /// Name glob such as '*.pdf', or text the name contains
    #[arg(short, long)]
    name: Option<String>,
    /// Regular expression on the name (on the path below LOCATION with --full-path)
    #[arg(short = 'e', long)]
    regex: Option<String>,
    /// Match --regex against the path below LOCATION, e.g. 'src/.*\.rs$'
    #[arg(long)]
    full_path: bool,
    /// Match case exactly (default: ignore case)
    #[arg(short = 's', long)]
    case_sensitive: bool,
    /// Only files, directories or symlinks
    #[arg(short = 't', long = "type", value_enum)]
    kind: Option<KindArg>,
    /// Size bound, repeatable: +10M (at least), -1G (at most); k/M/G/T or Ki/Mi/Gi/Ti
    #[arg(long, allow_hyphen_values = true)]
    size: Vec<String>,
    /// Modified within an age (30min, 12h, 7d, 2w, 6mo, 1y) or since a date (2026-01-31)
    #[arg(long)]
    newer: Option<String>,
    /// Modified longer ago than an age, or before a date
    #[arg(long)]
    older: Option<String>,
    /// Include hidden files and folders
    #[arg(short = 'H', long)]
    hidden: bool,
    /// Levels to descend; 1 searches only LOCATION's direct contents
    #[arg(short = 'd', long)]
    max_depth: Option<usize>,
    /// Stop after this many matches
    #[arg(long)]
    limit: Option<usize>,
    /// Long format: permissions, size, modification time
    #[arg(short, long)]
    long: bool,
    /// Print one JSON object per line
    #[arg(long, conflicts_with = "long")]
    json: bool,
    /// List every folder that could not be read, not just how many
    #[arg(long)]
    show_errors: bool,
}

#[derive(Clone, Copy, ValueEnum)]
enum KindArg {
    #[value(name = "f", alias = "file")]
    File,
    #[value(name = "d", alias = "dir")]
    Dir,
    #[value(name = "l", alias = "symlink")]
    Symlink,
}

#[derive(Serialize)]
struct Found<'a> {
    location: String,
    #[serde(flatten)]
    entry: &'a Entry,
}

pub async fn run(args: FindArgs, registry: &Registry) -> Result<()> {
    let now = chrono::Utc::now();
    let opts = SearchOptions {
        name: args.name,
        regex: args.regex,
        full_path: args.full_path,
        case_sensitive: args.case_sensitive,
        kind: args.kind.map(|k| match k {
            KindArg::File => EntryKind::File,
            KindArg::Dir => EntryKind::Dir,
            KindArg::Symlink => EntryKind::Symlink,
        }),
        sizes: args
            .size
            .iter()
            .map(|s| SizeFilter::parse(s))
            .collect::<porter_core::Result<_>>()?,
        newer: args
            .newer
            .as_deref()
            .map(|t| parse_time(t, now))
            .transpose()?,
        older: args
            .older
            .as_deref()
            .map(|t| parse_time(t, now))
            .transpose()?,
        hidden: args.hidden,
        max_depth: args.max_depth,
        limit: args.limit,
    };
    let root = registry.target(&args.location)?;
    let label = Labeller::new(&root, &args.location);

    let job = Job::new();
    let spinner = spinner(&job);
    let interrupt = tokio::spawn({
        let job = job.clone();
        async move {
            if tokio::signal::ctrl_c().await.is_ok() {
                job.cancel();
            }
        }
    });

    let stdout = std::io::stdout();
    let interactive = stdout.is_terminal();
    let mut out = BufWriter::new(stdout);
    let mut broken_pipe = false;
    let started = Instant::now();
    let report = search(&job, &root, &opts, |entry| {
        if broken_pipe {
            return;
        }
        let name = label.of(&entry);
        let line = if args.json {
            serde_json::to_string(&Found {
                location: root.with_path(entry.path.clone()).to_string(),
                entry: &entry,
            })
            .unwrap_or_default()
        } else if args.long {
            format::long_line(&entry, &name)
        } else {
            name
        };
        let written = match &spinner {
            Some(bar) => bar.suspend(|| write_line(&mut out, &line, interactive)),
            None => write_line(&mut out, &line, interactive),
        };
        if written.is_err() {
            // e.g. `porter find … | head`: the reader is gone, stop searching.
            broken_pipe = true;
            job.cancel();
        }
    })
    .await;
    interrupt.abort();
    if let Some(bar) = &spinner {
        bar.finish_and_clear();
    }
    let report = report?;
    if broken_pipe {
        return Ok(());
    }
    out.flush()?;

    if args.show_errors {
        for err in &report.errors {
            eprintln!("porter: {err}");
        }
    } else if !report.errors.is_empty() {
        eprintln!(
            "porter: {} {} could not be read (--show-errors lists them)",
            report.errors.len(),
            if report.errors.len() == 1 {
                "folder"
            } else {
                "folders"
            }
        );
    }
    if spinner.is_some() {
        let snapshot = job.progress().snapshot();
        eprintln!(
            "Found {} in {:.1} s ({} checked)",
            report.files,
            started.elapsed().as_secs_f64(),
            snapshot.scanned
        );
    }
    if report.cancelled {
        bail!("cancelled");
    }
    Ok(())
}

fn write_line(out: &mut impl Write, line: &str, flush: bool) -> std::io::Result<()> {
    writeln!(out, "{line}")?;
    if flush {
        out.flush()?;
    }
    Ok(())
}

/// A live "Searching…" line on stderr, when it is a terminal.
fn spinner(job: &Job) -> Option<ProgressBar> {
    if !std::io::stderr().is_terminal() {
        return None;
    }
    let bar = ProgressBar::new_spinner();
    bar.set_style(ProgressStyle::with_template("{spinner} {msg}").expect("valid template"));
    bar.enable_steady_tick(Duration::from_millis(100));
    tokio::spawn({
        let (job, bar) = (job.clone(), bar.clone());
        async move {
            while !bar.is_finished() {
                let s = job.progress().snapshot();
                bar.set_message(format!(
                    "Searching… {} checked, {} found",
                    s.scanned, s.files_done
                ));
                tokio::time::sleep(Duration::from_millis(100)).await;
            }
        }
    });
    Some(bar)
}

/// Prints matches the way the search root was given: relative to the current directory
/// for relative local paths, as full locations otherwise.
struct Labeller {
    root: Target,
    relative_to: Option<std::path::PathBuf>,
}

impl Labeller {
    fn new(root: &Target, input: &str) -> Self {
        let relative_to = (root.location.is_local() && !Path::new(input).is_absolute())
            .then(|| std::env::current_dir().ok())
            .flatten();
        Self {
            root: root.clone(),
            relative_to,
        }
    }

    fn of(&self, entry: &Entry) -> String {
        let suffix = if entry.is_dir() { "/" } else { "" };
        if let Some(base) = &self.relative_to
            && let Ok(relative) = Path::new(&entry.path).strip_prefix(base)
        {
            return format!("{}{suffix}", relative.display());
        }
        format!("{}{suffix}", self.root.with_path(entry.path.clone()))
    }
}
