mod find;
mod format;
mod progress;
mod remote;

use std::cmp::Reverse;
use std::io::IsTerminal;
use std::process::ExitCode;
use std::sync::OnceLock;

use anyhow::{Result, bail};
use clap::{Args, Parser, Subcommand, ValueEnum};
use futures::TryStreamExt;
use porter_backends::{Registry, local};
use porter_core::{Entry, FsInfo, ModeSpec, Target};
use porter_jobs::{Conflict, CopyOptions, DeleteOptions, Job, Report};
use serde::Serialize;

#[derive(Parser)]
#[command(
    name = "porter",
    version,
    about = "Browse and manage local and remote file systems"
)]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// List directory contents
    Ls(LsArgs),
    /// Show detailed metadata of a file, directory or object
    Stat {
        location: String,
        /// Print JSON instead of text
        #[arg(long)]
        json: bool,
    },
    /// Show file system size and free space (all local disks when no location is given)
    Df {
        location: Option<String>,
        /// Print JSON instead of text
        #[arg(long)]
        json: bool,
    },
    /// Copy files and directories
    Cp(CopyArgs),
    /// Move or rename files and directories
    Mv(TransferArgs),
    /// Delete files and directories
    Rm {
        #[arg(required = true)]
        locations: Vec<String>,
        /// Delete directories and their contents
        #[arg(short, long)]
        recursive: bool,
        /// Only print what would be deleted
        #[arg(short = 'n', long)]
        dry_run: bool,
    },
    /// Create directories
    Mkdir {
        #[arg(required = true)]
        locations: Vec<String>,
        /// Create missing parents; no error if the directory exists
        #[arg(short, long)]
        parents: bool,
    },
    /// Change permissions: octal (755) or symbolic (u+x, go-w, a=rX)
    Chmod {
        mode: String,
        #[arg(required = true)]
        locations: Vec<String>,
        /// Apply to directory contents too
        #[arg(short = 'R', long)]
        recursive: bool,
    },
    /// Search recursively by name, pattern, type, size or date
    Find(find::FindArgs),
    /// Manage saved remotes (SFTP, S3, FTP)
    Remote {
        #[command(subcommand)]
        command: remote::RemoteCommand,
    },
}

#[derive(Args)]
struct LsArgs {
    /// Directory or file to list: a path or a URI
    #[arg(default_value = ".")]
    location: String,
    /// Long format: permissions, size, modification time
    #[arg(short, long)]
    long: bool,
    /// Include hidden entries
    #[arg(short, long)]
    all: bool,
    /// Sort key
    #[arg(long, value_enum, default_value_t = SortKey::Name)]
    sort: SortKey,
    /// Reverse the sort order
    #[arg(short, long)]
    reverse: bool,
    /// Print JSON instead of text
    #[arg(long)]
    json: bool,
}

#[derive(Clone, Copy, ValueEnum)]
enum SortKey {
    Name,
    Size,
    Modified,
}

#[derive(Args)]
struct TransferArgs {
    /// Sources followed by the destination
    #[arg(required = true, num_args = 2.., value_name = "SOURCE... DEST")]
    paths: Vec<String>,
    /// What to do when a destination file already exists
    #[arg(long, value_enum, default_value_t = ConflictArg::Fail)]
    on_conflict: ConflictArg,
    /// Files transferred in parallel
    #[arg(short, long, default_value_t = 4)]
    jobs: usize,
}

#[derive(Args)]
struct CopyArgs {
    #[command(flatten)]
    transfer: TransferArgs,
    /// Copy directories recursively
    #[arg(short, long)]
    recursive: bool,
    /// Don't keep modification times and permissions
    #[arg(long)]
    no_preserve: bool,
}

#[derive(Clone, Copy, ValueEnum)]
enum ConflictArg {
    Fail,
    Skip,
    Overwrite,
    /// Keep both, naming the new one `name (1).ext`
    Rename,
    /// Overwrite only when the source is newer
    Newer,
}

impl From<ConflictArg> for Conflict {
    fn from(arg: ConflictArg) -> Self {
        match arg {
            ConflictArg::Fail => Conflict::Fail,
            ConflictArg::Skip => Conflict::Skip,
            ConflictArg::Overwrite => Conflict::Overwrite,
            ConflictArg::Rename => Conflict::Rename,
            ConflictArg::Newer => Conflict::Newer,
        }
    }
}

#[tokio::main]
async fn main() -> ExitCode {
    match run(Cli::parse()).await {
        Ok(()) => ExitCode::SUCCESS,
        Err(err) => {
            eprintln!("porter: {err:#}");
            ExitCode::FAILURE
        }
    }
}

/// Saved remotes and cached connections, loaded once at startup.
static REGISTRY: OnceLock<Registry> = OnceLock::new();

fn registry() -> &'static Registry {
    REGISTRY.get().expect("registry is initialized in run()")
}

async fn run(cli: Cli) -> Result<()> {
    let _ = REGISTRY.set(Registry::load()?);
    match cli.command {
        Command::Ls(args) => ls(args).await,
        Command::Stat { location, json } => stat(&location, json).await,
        Command::Df { location, json } => df(location.as_deref(), json).await,
        Command::Cp(args) => cp(args).await,
        Command::Mv(args) => mv(args).await,
        Command::Rm {
            locations,
            recursive,
            dry_run,
        } => rm(&locations, recursive, dry_run).await,
        Command::Mkdir { locations, parents } => mkdir(&locations, parents).await,
        Command::Chmod {
            mode,
            locations,
            recursive,
        } => chmod(&mode, &locations, recursive).await,
        Command::Find(args) => find::run(args, registry()).await,
        Command::Remote { command } => remote::run(command, registry()).await,
    }
}

async fn ls(args: LsArgs) -> Result<()> {
    let target = target(&args.location)?;

    // Like `ls`, listing a file shows just that file.
    let entry = target.provider.stat(target.path()).await?;
    let mut entries: Vec<Entry> = if entry.is_dir() {
        target
            .provider
            .list(target.path())
            .await?
            .try_collect()
            .await?
    } else {
        vec![entry]
    };

    if !args.all {
        entries.retain(|e| !e.hidden);
    }
    match args.sort {
        SortKey::Name => entries.sort_by_cached_key(|e| e.name.to_lowercase()),
        SortKey::Size => entries.sort_by_key(|e| Reverse(e.size.unwrap_or(0))),
        SortKey::Modified => entries.sort_by_key(|e| Reverse(e.modified)),
    }
    if args.reverse {
        entries.reverse();
    }

    if args.json {
        return print_json(&entries);
    }
    if args.long {
        format::print_long(&entries);
    } else {
        for entry in &entries {
            println!("{}", format::display_name(entry));
        }
    }
    Ok(())
}

async fn stat(location: &str, json: bool) -> Result<()> {
    let target = target(location)?;
    let entry = target.provider.stat(target.path()).await?;
    if json {
        print_json(&entry)
    } else {
        format::print_stat(&entry);
        Ok(())
    }
}

async fn df(location: Option<&str>, json: bool) -> Result<()> {
    let infos: Vec<FsInfo> = match location {
        Some(location) => {
            let target = target(location)?;
            vec![target.provider.fs_info(target.path()).await?]
        }
        None => local::disks().await?,
    };
    if json {
        print_json(&infos)
    } else {
        format::print_df(&infos);
        Ok(())
    }
}

async fn cp(args: CopyArgs) -> Result<()> {
    let opts = CopyOptions {
        recursive: args.recursive,
        conflict: args.transfer.on_conflict.into(),
        concurrency: args.transfer.jobs,
        preserve: !args.no_preserve,
    };
    let (sources, dest) = split_dest(&args.transfer.paths)?;
    let job = Job::new();
    let report = progress::track(
        &job,
        "Copying",
        porter_jobs::copy(&job, &sources, &dest, &opts),
    )
    .await?;
    finish("Copied", &report)
}

async fn mv(args: TransferArgs) -> Result<()> {
    let opts = CopyOptions {
        recursive: true,
        conflict: args.on_conflict.into(),
        concurrency: args.jobs,
        preserve: true,
    };
    let (sources, dest) = split_dest(&args.paths)?;
    let job = Job::new();
    let report = progress::track(
        &job,
        "Moving",
        porter_jobs::move_to(&job, &sources, &dest, &opts),
    )
    .await?;
    finish("Moved", &report)
}

async fn rm(locations: &[String], recursive: bool, dry_run: bool) -> Result<()> {
    let opts = DeleteOptions {
        recursive,
        dry_run,
        ..Default::default()
    };
    let targets = targets(locations)?;
    let job = Job::new();
    let report =
        progress::track(&job, "Deleting", porter_jobs::delete(&job, &targets, &opts)).await;
    for path in &report.planned {
        println!("would delete {path}");
    }
    finish("Deleted", &report)
}

async fn mkdir(locations: &[String], parents: bool) -> Result<()> {
    for target in targets(locations)? {
        porter_jobs::mkdir(&target, parents).await?;
    }
    Ok(())
}

async fn chmod(mode: &str, locations: &[String], recursive: bool) -> Result<()> {
    let spec = ModeSpec::parse(mode)?;
    let targets = targets(locations)?;
    let job = Job::new();
    let report = progress::track(
        &job,
        "Changing",
        porter_jobs::chmod(&job, &targets, &spec, recursive),
    )
    .await;
    finish("Changed", &report)
}

fn target(location: &str) -> Result<Target> {
    Ok(registry().target(location)?)
}

fn targets(locations: &[String]) -> Result<Vec<Target>> {
    locations.iter().map(|l| target(l)).collect()
}

fn split_dest(paths: &[String]) -> Result<(Vec<Target>, Target)> {
    let (dest, sources) = paths.split_last().expect("clap requires two paths");
    Ok((targets(sources)?, target(dest)?))
}

/// Prints per-item errors and, on a terminal, a summary; fails if anything failed.
fn finish(verb: &str, report: &Report) -> Result<()> {
    for err in &report.errors {
        eprintln!("porter: {err}");
    }
    if std::io::stderr().is_terminal() && report.planned.is_empty() {
        eprintln!("{}", format::summary(verb, report));
    }
    if report.cancelled {
        bail!("cancelled");
    }
    match report.errors.len() {
        0 => Ok(()),
        1 => bail!("1 item failed"),
        n => bail!("{n} items failed"),
    }
}

fn print_json<T: Serialize + ?Sized>(value: &T) -> Result<()> {
    println!("{}", serde_json::to_string_pretty(value)?);
    Ok(())
}
