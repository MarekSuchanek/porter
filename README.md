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

## Layout

| Crate | Purpose |
|---|---|
| `porter-core` | `Provider` trait, `Entry`/`FsInfo`/`Capabilities`, `Location`/`Target`, `ModeSpec`, errors |
| `porter-backends` | Backend implementations (local for now), `provider_for()` and `target()` |
| `porter-jobs` | Copy, move, delete, chmod, mkdir with progress, cancellation and conflict policies |
| `porter-cli` | The `porter` binary |
