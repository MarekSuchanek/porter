# Changelog

All notable changes to Porter. The release workflow uses the section matching the tag
as the release notes; the first line of it is shown in the in-app update prompt.

## [Unreleased]

## [0.1.0] - 2026-10-24

First release: a desktop app and a `porter` CLI for local and remote file systems.

### Added

- Browse local disks and remotes: SFTP, S3 (and S3-compatible stores), FTP/FTPS, SMB 2/3
  and NFSv3, as saved remotes (`name:/path`) with secrets in the OS keychain.
- Copy, move, delete, create folders and change permissions, with progress,
  cancellation and conflict handling (skip, replace, keep both, newer).
- Recursive search by name, pattern, type, size and date (`porter find`, and in the app).
- Desktop app: tabs, dual pane, sidebar, inspector with a permissions editor, transfer
  drawer, command palette, drag and drop, light and dark themes, automatic updates.
