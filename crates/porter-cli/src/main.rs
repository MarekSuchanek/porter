mod format;

use std::cmp::Reverse;
use std::process::ExitCode;

use anyhow::Result;
use clap::{Args, Parser, Subcommand, ValueEnum};
use futures::TryStreamExt;
use porter_backends::{local, provider_for};
use porter_core::{Entry, FsInfo, Location};
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

async fn run(cli: Cli) -> Result<()> {
    match cli.command {
        Command::Ls(args) => ls(args).await,
        Command::Stat { location, json } => stat(&location, json).await,
        Command::Df { location, json } => df(location.as_deref(), json).await,
    }
}

async fn ls(args: LsArgs) -> Result<()> {
    let location = Location::parse(&args.location)?;
    let provider = provider_for(&location)?;

    // Like `ls`, listing a file shows just that file.
    let target = provider.stat(&location.path).await?;
    let mut entries: Vec<Entry> = if target.is_dir() {
        provider.list(&location.path).await?.try_collect().await?
    } else {
        vec![target]
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
    let location = Location::parse(location)?;
    let entry = provider_for(&location)?.stat(&location.path).await?;
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
            let location = Location::parse(location)?;
            vec![provider_for(&location)?.fs_info(&location.path).await?]
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

fn print_json<T: Serialize + ?Sized>(value: &T) -> Result<()> {
    println!("{}", serde_json::to_string_pretty(value)?);
    Ok(())
}
