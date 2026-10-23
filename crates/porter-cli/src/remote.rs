//! `porter remote …`: manage saved remotes.

use std::io::BufRead;
use std::time::Instant;

use anyhow::{Context, Result, bail};
use clap::{Args, Subcommand, ValueEnum};
use porter_backends::config::KnownHosts;
use porter_backends::{Backend, Config, OpendalProvider, Registry, Remote};
use porter_core::Location;

#[derive(Subcommand)]
pub enum RemoteCommand {
    /// Save a remote, then use it as NAME:/path
    Add {
        #[command(subcommand)]
        kind: AddKind,
    },
    /// List saved remotes
    #[command(alias = "ls")]
    List {
        /// Print JSON instead of text
        #[arg(long)]
        json: bool,
    },
    /// Remove a saved remote and its stored secret
    #[command(alias = "rm")]
    Remove { name: String },
    /// Check that a remote is reachable and its credentials work
    Test { name: String },
}

#[derive(Subcommand)]
pub enum AddKind {
    /// SFTP server; authenticates with SSH keys or the SSH agent
    Sftp {
        #[command(flatten)]
        common: Common,
        #[arg(long)]
        host: String,
        #[arg(long)]
        port: Option<u16>,
        #[arg(long)]
        user: Option<String>,
        /// Private key file (default: the SSH agent and ~/.ssh config)
        #[arg(long)]
        key_file: Option<String>,
        /// How to treat host keys not in known_hosts
        #[arg(long, value_enum)]
        known_hosts: Option<KnownHostsArg>,
    },
    /// AWS S3 bucket or an S3-compatible store (MinIO, R2, B2, Wasabi…)
    S3 {
        #[command(flatten)]
        common: Common,
        #[arg(long)]
        bucket: String,
        /// Detected automatically for AWS when omitted
        #[arg(long)]
        region: Option<String>,
        /// Custom endpoint URL for S3-compatible stores
        #[arg(long)]
        endpoint: Option<String>,
        /// Without one, credentials come from the environment or ~/.aws
        #[arg(long)]
        access_key_id: Option<String>,
        /// Use bucket.endpoint URLs instead of endpoint/bucket
        #[arg(long)]
        virtual_host_style: bool,
        #[command(flatten)]
        secret: SecretArgs,
    },
    /// Windows or Samba share (SMB 2/3); signs in as guest without --ask-secret
    Smb {
        #[command(flatten)]
        common: Common,
        #[arg(long)]
        host: String,
        /// Share name, e.g. Public
        #[arg(long)]
        share: String,
        #[arg(long)]
        port: Option<u16>,
        /// user or DOMAIN\user
        #[arg(long)]
        user: Option<String>,
        #[command(flatten)]
        secret: SecretArgs,
    },
    /// NFSv3 export (AUTH_SYS: your uid/gid unless given)
    Nfs {
        #[command(flatten)]
        common: Common,
        #[arg(long)]
        host: String,
        /// Exported path, e.g. /srv/share
        #[arg(long)]
        export: String,
        /// NFS port; omit to ask the server's portmapper
        #[arg(long)]
        port: Option<u16>,
        /// MOUNT port, if different from --port
        #[arg(long)]
        mount_port: Option<u16>,
        #[arg(long)]
        uid: Option<u32>,
        #[arg(long)]
        gid: Option<u32>,
        /// Connect from a port below 1024 (servers exporting with `secure`; needs root)
        #[arg(long)]
        privileged_port: bool,
    },
    /// FTP or FTPS server
    Ftp {
        #[command(flatten)]
        common: Common,
        #[arg(long)]
        host: String,
        #[arg(long)]
        port: Option<u16>,
        #[arg(long)]
        user: Option<String>,
        /// Use FTPS (explicit TLS)
        #[arg(long)]
        tls: bool,
        #[command(flatten)]
        secret: SecretArgs,
    },
}

#[derive(Args)]
pub struct Common {
    /// Name to use in locations, e.g. `nas` for nas:/photos
    name: String,
    /// Directory on the server (or key prefix) that NAME:/ maps to
    #[arg(long)]
    root: Option<String>,
    /// Replace an existing remote with the same name
    #[arg(long)]
    force: bool,
}

#[derive(Args)]
pub struct SecretArgs {
    /// Prompt for the password / secret key and store it in the OS keychain
    #[arg(long, conflicts_with = "secret_stdin")]
    ask_secret: bool,
    /// Read the password / secret key from the first line of stdin
    #[arg(long)]
    secret_stdin: bool,
}

#[derive(Clone, Copy, ValueEnum)]
pub enum KnownHostsArg {
    /// Refuse unknown hosts (default)
    Strict,
    /// Add unknown hosts to known_hosts
    Add,
    /// Accept any host key (insecure)
    Accept,
}

impl From<KnownHostsArg> for KnownHosts {
    fn from(arg: KnownHostsArg) -> Self {
        match arg {
            KnownHostsArg::Strict => KnownHosts::Strict,
            KnownHostsArg::Add => KnownHosts::Add,
            KnownHostsArg::Accept => KnownHosts::Accept,
        }
    }
}

pub async fn run(command: RemoteCommand, registry: &Registry) -> Result<()> {
    match command {
        RemoteCommand::Add { kind } => add(kind, registry).await,
        RemoteCommand::List { json } => list(registry.config(), json),
        RemoteCommand::Remove { name } => remove(&name, registry),
        RemoteCommand::Test { name } => test(&name, registry).await,
    }
}

async fn add(kind: AddKind, registry: &Registry) -> Result<()> {
    let (common, backend, secret_args) = match kind {
        AddKind::Sftp {
            common,
            host,
            port,
            user,
            key_file,
            known_hosts,
        } => {
            let backend = Backend::Sftp {
                host,
                port,
                user,
                key_file,
                known_hosts: known_hosts.map(Into::into),
            };
            (common, backend, None)
        }
        AddKind::S3 {
            common,
            bucket,
            region,
            endpoint,
            access_key_id,
            virtual_host_style,
            secret,
        } => {
            let region = match (region, &endpoint) {
                (Some(region), _) => Some(region),
                (None, Some(_)) => None,
                (None, None) => {
                    let region = OpendalProvider::detect_s3_region(&bucket, None)
                        .await
                        .with_context(|| {
                            format!(
                                "could not detect the region of bucket `{bucket}`; pass --region"
                            )
                        })?;
                    eprintln!("Detected region {region}");
                    Some(region)
                }
            };
            if (secret.ask_secret || secret.secret_stdin) && access_key_id.is_none() {
                bail!("a secret key needs --access-key-id");
            }
            let backend = Backend::S3 {
                bucket,
                region,
                endpoint,
                access_key_id,
                virtual_host_style,
            };
            (common, backend, Some(secret))
        }
        AddKind::Ftp {
            common,
            host,
            port,
            user,
            tls,
            secret,
        } => {
            let backend = Backend::Ftp {
                host,
                port,
                user,
                tls,
            };
            (common, backend, Some(secret))
        }
        AddKind::Smb {
            common,
            host,
            share,
            port,
            user,
            secret,
        } => {
            let backend = Backend::Smb {
                host,
                share,
                port,
                user,
            };
            (common, backend, Some(secret))
        }
        AddKind::Nfs {
            common,
            host,
            export,
            port,
            mount_port,
            uid,
            gid,
            privileged_port,
        } => {
            let backend = Backend::Nfs {
                host,
                export,
                port,
                mount_port,
                uid,
                gid,
                privileged_port,
            };
            (common, backend, None)
        }
    };

    let name = common.name;
    if !Location::is_valid_remote_name(&name) {
        bail!(
            "invalid remote name `{name}`: use letters, digits, `-` and `_`, starting with a \
             letter, at least two characters"
        );
    }
    let path = Config::default_path()?;
    let mut config = Config::load_from(&path)?;
    if config.remotes.contains_key(&name) && !common.force {
        bail!("remote `{name}` already exists (use --force to replace it)");
    }

    let secret = match (secret_args, backend.secret_label()) {
        (Some(args), Some(label)) => read_secret(&args, label, &name)?,
        _ => None,
    };
    let address = backend.address();
    config.remotes.insert(
        name.clone(),
        Remote {
            backend,
            root: common.root,
        },
    );
    match &secret {
        Some(secret) => registry.secrets().set(&name, secret)?,
        // A replaced remote must not keep a stale secret.
        None if common.force => registry.secrets().delete(&name)?,
        None => {}
    }
    config.save_to(&path)?;

    let stored = if secret.is_some() {
        " (secret in keychain)"
    } else {
        ""
    };
    eprintln!("Saved `{name}` → {address}{stored}. Use it as {name}:/path");
    Ok(())
}

fn read_secret(args: &SecretArgs, label: &str, name: &str) -> Result<Option<String>> {
    let secret = if args.ask_secret {
        rpassword::prompt_password(format!("{} for {name}: ", capitalize(label)))?
    } else if args.secret_stdin {
        let mut line = String::new();
        std::io::stdin().lock().read_line(&mut line)?;
        line.trim_end_matches(['\r', '\n']).to_owned()
    } else {
        return Ok(None);
    };
    if secret.is_empty() {
        bail!("empty {label}");
    }
    Ok(Some(secret))
}

fn capitalize(s: &str) -> String {
    let mut chars = s.chars();
    match chars.next() {
        Some(first) => first.to_uppercase().chain(chars).collect(),
        None => String::new(),
    }
}

fn list(config: &Config, json: bool) -> Result<()> {
    if json {
        println!("{}", serde_json::to_string_pretty(&config.remotes)?);
        return Ok(());
    }
    if config.remotes.is_empty() {
        eprintln!("No remotes saved. Add one with `porter remote add`.");
        return Ok(());
    }
    let rows: Vec<[String; 4]> = config
        .remotes
        .iter()
        .map(|(name, remote)| {
            [
                name.clone(),
                remote.backend.kind().to_owned(),
                remote.backend.address(),
                remote.root.clone().unwrap_or_else(|| "/".into()),
            ]
        })
        .collect();
    let header = ["NAME", "TYPE", "ADDRESS", "ROOT"].map(String::from);
    crate::format::print_table(&header, &rows);
    Ok(())
}

fn remove(name: &str, registry: &Registry) -> Result<()> {
    let path = Config::default_path()?;
    let mut config = Config::load_from(&path)?;
    if config.remotes.remove(name).is_none() {
        bail!("no remote named `{name}`");
    }
    config.save_to(&path)?;
    registry.secrets().delete(name)?;
    eprintln!("Removed `{name}`");
    Ok(())
}

async fn test(name: &str, registry: &Registry) -> Result<()> {
    let provider = registry.remote_provider(name)?;
    let started = Instant::now();
    provider.check().await?;
    let elapsed = started.elapsed();

    let caps = provider.capabilities();
    let supported: Vec<&str> = [
        ("list", caps.list),
        ("read", caps.read),
        ("write", caps.write),
        ("delete", caps.delete),
        ("rename", caps.rename),
        ("mkdir", caps.mkdir),
        ("permissions", caps.set_permissions),
        ("times", caps.set_modified),
        ("server-side copy", caps.server_side_copy),
    ]
    .into_iter()
    .filter_map(|(label, ok)| ok.then_some(label))
    .collect();
    println!("{name}: OK ({} ms)", elapsed.as_millis());
    println!("supports: {}", supported.join(", "));
    Ok(())
}
