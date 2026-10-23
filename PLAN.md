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
│  ├─ porter-jobs/      # job engine: copy/move/delete/chmod with progress + cancellation
│  ├─ porter-cli/       # `porter` binary
│  └─ porter-gui/       # Tauri app (Rust side); the Svelte UI lives in ui/
```

### Core concepts

- **Location** — every place is addressed by a URI: `file:///…` (or a plain OS path),
  `s3://bucket/key`, `sftp://user@host/path`, `smb://server/share/path`, `nfs://…`.
  Saved remotes are addressed as `name:/path` (like rclone). Names start with a letter
  and have at least two characters, so `C:\dir` stays a Windows path; `./name:x` forces
  a local path.
- **Remotes** — profiles live in `config.toml` in the OS config dir (`$PORTER_CONFIG_DIR`
  overrides it). Each remote has at most one secret (FTP password, S3 secret key),
  stored in the OS keychain under service `porter`, account `remote/<name>` — never in
  the file. A `Registry` resolves locations and caches one provider per backend.
- **Provider** — one implementation per backend. Async, object-safe (`dyn Provider`).
  Listings are *streams* so huge directories render progressively.
- **Capabilities** — each provider declares what it supports (chmod, chown, symlinks,
  server-side copy, rename, fs info, …). The CLI and GUI use this to hide/disable
  actions instead of failing with cryptic errors (S3 has no chmod; FTP has no checksums).
- **Entry** — normalized metadata: kind, size, timestamps, permissions (POSIX mode,
  read-only flag, uid/gid), hidden flag, symlink target. Backend-specific extras go in
  a later `extra` map (S3 storage class, ETag, SMB attributes…).
- **Target** — a `Location` plus the `Provider` serving it; what operations take.
- **Jobs** — long-running operations take a `Job` handle: lock-free progress counters
  (scanned, files, bytes, current item) that front ends poll, plus a cancellation
  token. Operations plan first (walk sources, compute totals), then execute with
  bounded concurrency in 1 MiB chunks. Per-item failures are collected in a `Report`
  instead of aborting. Conflict policy: fail / skip / overwrite / rename (`name (1).ext`)
  / newer. Moves rename within a backend and fall back to copy + delete (source is
  removed only if every item arrived). Cancelled or failed transfers remove their
  partial file.

### Provider trait evolution

| Milestone | Methods |
|---|---|
| M1 ✅ | `stat`, `list`, `fs_info`, `capabilities` |
| M2 ✅ | `stat_follow`, `read`/`write` (tokio `AsyncRead`/`AsyncWrite`), `mkdir`, `delete`, `rename`, `set_permissions`, `set_modified`, `symlink`, path helpers (`join`, `parent`, `file_name`) — all but the M1 methods default to `Unsupported` |
| M5 ✅ | `walk` (optional fast recursive listing with a name/kind pre-filter; local uses a parallel `jwalk`; `None` → callers walk with `list`, several directories at a time) |
| later | `set_owner`, server-side `copy`, native search (e.g. Spotlight, S3 prefix listing) |

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
porter cp SRC… DST [-r] [--jobs N] [--on-conflict fail|skip|overwrite|rename|newer] [--no-preserve]
porter mv SRC… DST [--jobs N] [--on-conflict …]
porter rm LOCATION… [-r] [-n/--dry-run]
porter mkdir LOCATION… [-p]
porter chmod MODE LOCATION… [-R]                     # MODE: 755 or u+x,go-w,a=rX
porter chown OWNER[:GROUP] LOCATION… [-R]                                           # later
porter find [LOCATION] [-n GLOB|TEXT] [-e RE [--full-path]] [-t f|d|l] [--size +10M] [--size -1G]
            [--newer 7d] [--older 2026-01-01] [-H] [-d DEPTH] [--limit N] [-l|--json] [--show-errors]
porter remote add sftp|s3|ftp|smb|nfs NAME [options] [--ask-secret|--secret-stdin]
porter remote list|remove|test
```

## Milestones

1. **M1 Foundation** — workspace, core types, Location parsing, local provider,
   CLI `ls`, `stat`, `df`. ✅
2. **M2 Operations** — job engine; copy/move/delete/mkdir/chmod on local; CLI with
   progress bars and Ctrl+C cancellation; conflict policies; dry-run. ✅
   Deferred: chown/owner names, pause/resume, checksum verification, local fast
   paths (APFS `clonefile`, Linux `copy_file_range`/reflinks), trash instead of
   delete, Windows symlinks, non-UTF-8 file names.
3. **M3 Remotes I** — OpenDAL-backed providers (SFTP, S3, FTP); connection profiles in
   config; credentials in the OS keychain; `porter remote …`. ✅ Verified live against
   MinIO, OpenSSH sftp-server and vsftpd.
   Known limits / follow-ups: SFTP is key/agent-only and Unix-only (OpenDAL drives the
   system `ssh`; Windows needs a russh-based backend); remotes don't keep mtimes or
   permissions; the engine doesn't use server-side copy yet; S3/FTP moves are
   copy + delete; write-if-absent is check-then-write (racy); error texts from servers
   are passed through verbatim; WebDAV and other OpenDAL services are not wired up.
4. **M4 GUI v1** — Tauri + Svelte app: browser with tabs, sidebar, inspector,
   transfer drawer, dual-pane toggle. ✅ Also: ⌘K palette, context menus, drag & drop
   (Shift moves), conflict dialog (Skip / Replace / Keep both), rename, new folder,
   permissions editor, type-to-filter, session restore, light/dark. In a plain browser
   the UI runs on an in-memory mock backend (`ui/src/lib/mock.ts`) for development.
   Follow-ups: drops from/to Finder/Explorer, trash instead of delete, folder sizes,
   thumbnails/Quick Look, editing remotes in the GUI, keyboard customization,
   Windows/Linux packaging checks.
5. **M5 Search** — `find` with name/glob/regex/size/date/type filters; streaming
   results in CLI and GUI; parallel local walk. ✅ Name/kind filters run before any
   metadata is read; results stream (CLI stops early when piped into `head`); GUI:
   Enter in a pane's filter searches subfolders, with type/size/date/hidden filters,
   live counts, a Folder column and "Show in enclosing folder" (results capped at 5,000).
   On ~/Projects (≈637k entries) a name search took ≈1.5 s; BSD `find` took 1.3–9 s
   depending on cache state.
   Follow-ups: content search (grep), an index for instant search, Spotlight /
   Windows Search integration, `.gitignore`-aware mode, saved searches.
6. **M6 Remotes II** — SMB and NFS providers. ✅ SMB 2/3 via the pure-Rust `smb`
   crate (user/password in the keychain, or guest); NFSv3 via `nfs3_client` (AUTH_SYS
   with your uid/gid, POSIX permissions and times supported). Both connect lazily on
   first use. Verified live against Samba 4.23 and the Linux kernel NFS server: up/down
   200 MiB byte-identical, server-side renames, chmod (NFS), times preserved.
   `PORTER_SECRET_<NAME>` overrides a stored secret (scripts, CI, no keychain).
   Known limits: SMB guest sessions can read but not write — smb-rs checks writes
   against the server's "maximal access" reply, which Samba reports without write
   data for guests; NFS needs an export with `insecure` (or `--privileged-port` as
   root), NFSv3 only (no NFSv4, no Kerberos); ad-hoc `smb://`/`nfs://` URLs aren't
   supported (save a remote); SMB permissions (ACLs) aren't exposed.
7. **M7 Polish & release** — installers (dmg, msi, AppImage/deb), auto-update,
   signing/notarization, docs. ✅ Tag-driven GitHub Actions release into a draft:
   universal macOS app/dmg, Windows MSI + NSIS, Linux deb/rpm/AppImage, CLI archives
   for 5 targets with checksums, and a signed `latest.json` update feed. Signing is
   optional per platform (secrets), notarization via Tauri. In-app updater with
   ⌘K → "Check for updates…". CI on Linux/macOS/Windows. Version lives in
   `Cargo.toml` (`scripts/bump-version.sh`). See [docs/RELEASING.md](docs/RELEASING.md).
   Not yet verified: the workflows themselves (need the GitHub repo and secrets),
   Windows/Linux builds (no local toolchains), signing (no certificates yet).
   SFTP is Unix-only for now.

### Later ideas

Sync/mirror with diff preview, archive browsing (zip/tar as a provider), content
search, background index for instant search, TUI (`ratatui`), checksums/dedupe view,
bookmarks sync.

## Open questions

- License (MIT/Apache-2.0 dual is the Rust norm).
- NFS: pure-Rust client vs. libnfs FFI (affects Windows builds).
- Whether to also offer mounting (FUSE / WinFsp) — out of scope for now.
