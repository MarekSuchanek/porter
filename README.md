# Porter

A cross-platform manager for local and remote file systems (SFTP, S3, FTP, SMB, NFS, …)
with a CLI and a desktop GUI. See [PLAN.md](PLAN.md) for the architecture and roadmap.

## Build

```bash
cargo build --release
```

## Usage

```bash
porter ls -la ~/Documents          # list, long format, include hidden
porter ls --sort size -r .         # sort by size, smallest first
porter stat Cargo.toml             # detailed metadata
porter df                          # all mounted disks
porter ls --json /tmp              # machine-readable output (ls, stat, df)

porter cp -r photos /Volumes/Backup            # copy a tree, with a progress bar
porter cp --on-conflict rename a.txt dir/      # keep both: dir/a (1).txt
porter mv old.txt new.txt                      # rename, or copy + delete across disks
porter rm -rn build                            # dry run: show what would be deleted
porter mkdir -p a/b/c
porter chmod -R go-w,a+rX shared               # octal (755) or symbolic modes
```

Existing destination files are an error by default; pick a policy with
`--on-conflict fail|skip|overwrite|rename|newer`. Ctrl+C cancels a running job and
removes any partially written file.

### Remotes

```bash
porter remote add sftp nas --host nas.local --user me --root /volume1   # SSH key / agent
porter remote add s3 backup --bucket my-backups                        # AWS creds from env/~/.aws
porter remote add s3 minio --bucket photos --endpoint http://localhost:9000 \
    --access-key-id KEY --ask-secret                                    # secret → OS keychain
porter remote add ftp files --host ftp.example.com --user me --tls --ask-secret
porter remote list
porter remote test nas

porter ls -l nas:/photos
porter cp -r ~/Pictures/2024 backup:/pictures/
porter mv nas:/inbox/report.pdf nas:/archive/
```

Ad-hoc URLs work too: `sftp://me@host:2222/path`, `ftp://host/path`, `s3://bucket/key`.
Profiles are stored in `config.toml` in the OS config directory (override with
`PORTER_CONFIG_DIR`); passwords and secret keys only ever go to the OS keychain.

## Desktop app

```bash
npm install
npm run tauri dev      # run the app (Vite dev server + Tauri window)
npm run tauri build    # installer for this OS in target/release/bundle
npm run dev            # UI only, in a browser, against an in-memory mock backend
```

Tabs, an optional dual-pane view (⌘\\), sidebar with places, disks and saved remotes,
an inspector with a permissions editor, and a transfer drawer with live progress.
Press ⌘K (Ctrl+K) for every command and its shortcut. Drag files between panes or
onto folders to copy them; hold Shift to move.

## Layout

| Crate | Purpose |
|---|---|
| `porter-core` | `Provider` trait, `Entry`/`FsInfo`/`Capabilities`, `Location`/`Target`, `ModeSpec`, errors |
| `porter-backends` | Local and OpenDAL (SFTP, S3, FTP) providers, remote config, keychain secrets, `Registry` |
| `porter-jobs` | Copy, move, delete, chmod, mkdir with progress, cancellation and conflict policies |
| `porter-cli` | The `porter` binary |
| `porter-gui` | Tauri desktop app: commands and job events for the UI in `ui/` |
