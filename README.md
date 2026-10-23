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
porter ls --json /tmp              # machine-readable output (any command)
```

## Layout

| Crate | Purpose |
|---|---|
| `porter-core` | `Provider` trait, `Entry`/`FsInfo`/`Capabilities`, `Location` URIs, errors |
| `porter-backends` | Backend implementations (local for now) and `provider_for()` |
| `porter-cli` | The `porter` binary |
