# Releasing Porter

A release is a version tag (`v1.2.3`) pushed to GitHub. The
[release workflow](../.github/workflows/release.yml) then builds everything into a
**draft** release, which you review and publish:

| Artifact | Built on | Notes |
|---|---|---|
| `Porter_<v>_universal.dmg`, `Porter.app.tar.gz` | macOS | one universal app for Apple silicon and Intel; signed and notarized when configured |
| `Porter_<v>_x64_en-US.msi`, `Porter_<v>_x64-setup.exe` | Windows | MSI (WiX) and NSIS installers; signed when configured |
| `porter_<v>_amd64.deb`, `Porter-<v>-1.x86_64.rpm`, `Porter_<v>_amd64.AppImage` | Linux (Ubuntu 22.04) | |
| `porter-v<v>-<target>.tar.gz` / `.zip` + `.sha256` | all | the CLI for macOS (arm64, x64), Linux (x64, arm64), Windows (x64) |
| `latest.json` + `.sig` files | all | the auto-update feed and its signatures |

[CI](../.github/workflows/ci.yml) runs formatting, clippy, the tests and the UI type
check on Linux, macOS and Windows for every push and pull request.

## One-time setup

Add these as repository secrets (GitHub → Settings → Secrets and variables → Actions).
Only the first two are required; without the others, builds are simply unsigned.

### Update signing (required)

Updates are verified with a key pair; the public key is in
[`tauri.conf.json`](../crates/porter-gui/tauri.conf.json) (`plugins.updater.pubkey`).

| Secret | Value |
|---|---|
| `TAURI_SIGNING_PRIVATE_KEY` | contents of `~/.tauri/porter-updater.key` |
| `TAURI_SIGNING_PRIVATE_KEY_PASSWORD` | the password chosen when generating it |

```bash
gh secret set TAURI_SIGNING_PRIVATE_KEY --repo MarekSuchanek/porter < ~/.tauri/porter-updater.key
gh secret set TAURI_SIGNING_PRIVATE_KEY_PASSWORD --repo MarekSuchanek/porter
```

Keep a backup of the key file and password somewhere safe (e.g. a password manager).
If the key is lost, installed apps can no longer verify updates and users must
reinstall manually. To create a new pair:
`npx tauri signer generate -w ~/.tauri/porter-updater.key`, then put the new `.pub`
contents into `tauri.conf.json`.

### macOS signing and notarization (recommended)

Unsigned apps trigger Gatekeeper warnings ("cannot be opened because the developer
cannot be verified"). Signing and notarizing needs a paid Apple Developer account.

1. In Xcode (Settings → Accounts → Manage Certificates) or on developer.apple.com,
   create a **Developer ID Application** certificate.
2. Export it from Keychain Access as a `.p12` with a password.
3. Create an [app-specific password](https://account.apple.com) for your Apple ID.

| Secret | Value |
|---|---|
| `APPLE_CERTIFICATE` | `base64 -i certificate.p12` |
| `APPLE_CERTIFICATE_PASSWORD` | the `.p12` export password |
| `APPLE_SIGNING_IDENTITY` | e.g. `Developer ID Application: Marek Suchánek (TEAMID)` (`security find-identity -v -p codesigning`) |
| `APPLE_ID` | your Apple ID email |
| `APPLE_PASSWORD` | the app-specific password |
| `APPLE_TEAM_ID` | your 10-character team ID |

With `APPLE_ID` set, Tauri notarizes and staples the app during the build.

### Windows signing (optional)

Unsigned installers show a SmartScreen warning until they gain reputation. With an
Authenticode code-signing certificate as a `.pfx`:

| Secret | Value |
|---|---|
| `WINDOWS_CERTIFICATE` | base64 of the `.pfx` (`[Convert]::ToBase64String([IO.File]::ReadAllBytes("cert.pfx"))`) |
| `WINDOWS_CERTIFICATE_PASSWORD` | its password |

The workflow imports it and signs with SHA-256 and DigiCert's timestamp server.
Certificates kept in cloud HSMs (Azure Trusted Signing, SSL.com eSigner, …) can't be
exported as `.pfx`; use `bundle.windows.signCommand` in `tauri.conf.json` with your
provider's signing tool instead.

## Making a release

1. Make sure `main` is green in CI.
2. Set the version (it lives in `Cargo.toml` and is used by the CLI and the app):
   ```bash
   scripts/bump-version.sh 0.2.0
   ```
3. Move the `[Unreleased]` notes in `CHANGELOG.md` to a `## [0.2.0] - <date>` section.
   Its first line is shown in the in-app update prompt; the section becomes the
   release notes.
4. Commit, tag and push:
   ```bash
   git commit -am "Release 0.2.0"
   git tag v0.2.0
   git push origin main v0.2.0
   ```
   The workflow refuses tags that don't match the `Cargo.toml` version.
5. When the workflow finishes, open the draft release, check that every artifact is
   there (the table above), try an installer, then **publish** it.

Publishing makes the release "latest", which is what the update feed
(`releases/latest/download/latest.json`) points at. Pre-releases (`v0.2.0-beta.1`)
work the same way; publish them with "Set as a pre-release" so installed apps don't
pick them up.

## How updates reach users

The app checks the feed about five seconds after start (release builds only) and
from ⌘K → "Check for updates…". When a newer version exists it asks to install;
Porter then downloads the package, verifies its signature against the built-in public
key, installs it and restarts. On Windows the installer runs in passive mode; Linux
updates work for the AppImage (`.deb`/`.rpm` installs update through the package
manager).

The CLI has no self-update; users download it from the release page.

## Building locally

```bash
npm ci
npm run tauri build                       # installers for this OS, unsigned
npm run tauri build -- --bundles app,dmg  # macOS: only the app and the dmg
cargo build --release -p porter-cli       # just the CLI: target/release/porter
```

Output lands in `target/release/bundle/`. Local builds don't produce update packages
(that needs the private key); to test that, export `TAURI_SIGNING_PRIVATE_KEY` and
`TAURI_SIGNING_PRIVATE_KEY_PASSWORD` and add
`--config crates/porter-gui/tauri.release.conf.json`.

An unsigned macOS app from someone else's machine is quarantined; open it with
right-click → Open, or `xattr -dr com.apple.quarantine /Applications/Porter.app`.
