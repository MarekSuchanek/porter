# Porter — Plan

Porter is a cross-platform file manager for local and remote file systems, with a
CLI (`porter`) and a desktop GUI sharing one Rust core.

## Goals

- Browse, search, inspect, copy/transfer, delete and change permissions on files and
  objects — locally (Windows, macOS, Linux) and remotely (SFTP, S3, FTP, SMB, NFS, …).
- Fast: native code, streaming listings, concurrent chunked transfers, server-side
  copy whenever the backend supports it.
- Portable: a single binary per OS for the CLI, a small native-webview app for the GUI.
- Intuitive: the GUI and CLI expose the same operations with the same names.

## Stack

| Concern | Choice |
|---|---|
| Core | Rust (edition 2024), `tokio` async runtime |
| Backend abstraction | Own `Provider` trait in `porter-core` |
| Object stores & protocols | Apache OpenDAL (S3, Azure Blob, GCS, FTP, SFTP, WebDAV, …) |
| SMB | `smb` crate (pure Rust SMB2/3 client) |
| NFS | libnfs bindings (evaluate pure-Rust alternatives first) |
| Local disks / FS info | `std::fs` + `sysinfo` |
| Local walking & search | `jwalk` (parallel walk), `globset`, `regex` |
| CLI | `clap` (derive), `indicatif` for progress, `--json` on every command |
| GUI | Tauri 2 + Svelte 5 + TypeScript, TanStack Virtual for big listings |
| Secrets | `keyring` (macOS Keychain, Windows Credential Manager, Secret Service) |
| Config | TOML in the OS config dir (`directories` crate) |

## Architecture

```
porter/
├─ crates/
│  ├─ porter-core/      # Provider trait, Entry/FsInfo/Capabilities, Location (URIs), errors
│  ├─ porter-backends/  # local now; opendal-backed, smb, nfs later; provider registry
│  ├─ porter-jobs/      # (M2) job engine: copy/move/delete/search with progress events
│  ├─ porter-cli/       # `porter` binary
│  └─ porter-gui/       # (M4) Tauri app: src-tauri + Svelte UI
```

### Core concepts

- **Location** — every place is addressed by a URI: `file:///…` (or a plain OS path),
  `s3://bucket/key`, `sftp://user@host/path`, `smb://server/share/path`, `nfs://…`.
  Saved connection profiles get aliases usable as `alias:/path` (like rclone remotes).
- **Provider** — one implementation per backend. Async, object-safe (`dyn Provider`).
  Listings are *streams* so huge directories render progressively.
- **Capabilities** — each provider declares what it supports (chmod, chown, symlinks,
  server-side copy, rename, fs info, …). The CLI and GUI use this to hide/disable
  actions instead of failing with cryptic errors (S3 has no chmod; FTP has no checksums).
- **Entry** — normalized metadata: kind, size, timestamps, permissions (POSIX mode,
  read-only flag, uid/gid), hidden flag, symlink target. Backend-specific extras go in
  a later `extra` map (S3 storage class, ETag, SMB attributes…).
- **Jobs** (M2) — long-running operations run in the job engine: a queue with bounded
  concurrency, chunked/parallel transfers, resume, checksum verification, conflict
  policy (skip / overwrite / rename / newer-wins), pause/cancel. Both CLI and GUI
  subscribe to the same progress event stream.

### Provider trait evolution

| Milestone | Methods |
|---|---|
| M1 | `stat`, `list`, `fs_info`, `capabilities` |
| M2 | `read`/`write` (async streams), `mkdir`, `delete`, `rename`, `set_permissions`, `set_owner`, `copy` (server-side, optional) |
| M5 | `search` (optional native search; default falls back to walking) |

## GUI concept

- Left sidebar: local disks (with free-space bars), saved remotes, favorites.
- Main area: **tabs**, each a location; toggle into **dual-pane** for transfers
  (drag & drop between panes or tabs).
- Path bar: breadcrumbs, click to edit, accepts any URI or alias.
- List / grid views, virtualized; sortable columns; quick filter-as-you-type.
- Inspector panel: metadata, FS info, permissions editor (rwx grid, owner, ACL where supported).
- Transfer drawer: queue with speed, ETA, per-file errors, retry.
- Command palette (⌘K / Ctrl+K) and full keyboard navigation.
- Light/dark theme following the OS.

## CLI shape

```
porter ls [LOCATION] [-l] [-a] [--sort name|size|modified] [-r] [--json]
porter stat LOCATION [--json]
porter df [LOCATION] [--json]
porter cp SRC… DST [-r] [--jobs N] [--on-conflict skip|overwrite|rename|newer]   # M2
porter mv SRC… DST                                                                 # M2
porter rm LOCATION… [-r] [--dry-run]                                                # M2
porter mkdir LOCATION [-p]                                                          # M2
porter chmod MODE LOCATION… [-R]                                                    # M2
porter chown OWNER[:GROUP] LOCATION… [-R]                                           # M2
porter find LOCATION [--name GLOB] [--regex RE] [--size +10M] [--newer 7d] [--type f|d]  # M5
porter remote add|list|remove|test                                                  # M3
```

## Milestones

1. **M1 Foundation** — workspace, core types, Location parsing, local provider,
   CLI `ls`, `stat`, `df`. ✅ scaffolded
2. **M2 Operations** — job engine; copy/move/delete/mkdir/rename/chmod/chown on local;
   CLI with progress bars; conflict policies; dry-run.
3. **M3 Remotes I** — OpenDAL-backed providers (SFTP, S3, FTP, WebDAV); connection
   profiles in config; credentials in the OS keychain; `porter remote …`.
4. **M4 GUI v1** — Tauri + Svelte app: browser with tabs, sidebar, inspector,
   transfer drawer, dual-pane toggle.
5. **M5 Search** — `find` with name/glob/regex/size/date/type filters; streaming
   results in CLI and GUI; parallel local walk.
6. **M6 Remotes II** — SMB and NFS providers.
7. **M7 Polish & release** — installers (dmg, msi, AppImage/deb), auto-update,
   signing/notarization, docs.

### Later ideas

Sync/mirror with diff preview, archive browsing (zip/tar as a provider), content
search, background index for instant search, TUI (`ratatui`), checksums/dedupe view,
bookmarks sync.

## Open questions

- License (MIT/Apache-2.0 dual is the Rust norm).
- NFS: pure-Rust client vs. libnfs FFI (affects Windows builds).
- Whether to also offer mounting (FUSE / WinFsp) — out of scope for now.
