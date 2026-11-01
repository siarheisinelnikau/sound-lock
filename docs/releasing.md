# Release conventions

This document defines Sound Lock's release names and publishing process. The current distribution is a portable Windows x64 application.

## Names

Use the plain semantic version from the root `Cargo.toml`, without a leading `v`, as `VERSION`. Keep `Cargo.lock` in sync. Tags, titles and filenames add their own prefixes as shown below.

| Item | Format | Example |
|---|---|---|
| Cargo version | `VERSION` | `0.2.0` |
| Git tag | `vVERSION` | `v0.2.0` |
| GitHub release title | `Sound Lock vVERSION` | `Sound Lock v0.2.0` |
| Publishing commit, first line | `Release vVERSION` | `Release v0.2.0` |
| Standalone EXE, one architecture | `SoundLock-vVERSION.exe` | `SoundLock-v0.2.0.exe` |
| Current Windows x64 ZIP | `sound-lock-VERSION-windows-x64.zip` | `sound-lock-0.2.0-windows-x64.zip` |
| Executable inside the ZIP | `sound-lock.exe` | `sound-lock.exe` |
| Checksum manifest | `SHA256SUMS.txt` | `SHA256SUMS.txt` |
| Release notes source | `docs/releases/VERSION.md` | `docs/releases/0.2.0.md` |

`Sound Lock` is the display name; `SoundLock` is the standalone asset prefix. Cargo's internal output remains `target/release/sound-lock.exe`. The [packaging script](../scripts/package_release.py) produces the standalone name and ZIP and writes their exact filenames into the checksum manifest. These distinct names are intentional.

### Architecture suffixes

When a release offers only one executable architecture, omit the architecture suffix even when that build targets x64. The ZIP, checksums, source archives and documentation are ancillary files; they do not make the release a multi-architecture release.

Add suffixes only when the same release actually offers multiple executable architecture variants. For example, a future x64 and ARM64 release would use `SoundLock-v0.2.0-x64.exe` and `SoundLock-v0.2.0-arm64.exe`. Use `-x86` for a 32-bit x86 executable. Do not claim an architecture that was not built and checked.

The current packaging workflow builds only x64 and emits the suffix-free standalone EXE. Before offering additional architectures, extend packaging and validation for each target and update the asset selector. The current updater accepts canonical single-architecture names and explicitly suffixed x64 names, and verifies a Windows x64 PE header; it does not install ARM64 or x86 builds.

### Compatibility when renaming assets

Treat published asset names as part of the update protocol. Change packaging, updater asset selection, checksum matching and README download links together. Check what already published updaters accept before making the change.

Version 0.1.3 accepts only the historical `sound-lock-VERSION-windows-x64.exe` pattern. **The transition to 0.1.4 is manual:** its standalone asset is `SoundLock-v0.1.4.exe`, with no legacy-named EXE alias. Quit the old version, download the new EXE and launch it. Settings remain in `%APPDATA%\SoundLock\settings.json`. Version 0.1.4 supports canonical names for later compatible updates and can still read historical release metadata.

Do not add unversioned aliases or rename, replace or republish historical assets as part of a new release. Keep the stable executable name inside the ZIP. An in-app update replaces the installed executable in place and preserves the user's existing filename and location.

## Preparing a release

1. Compare the intended changes with the preceding published stable release. Update the root Cargo version and lockfile and write `docs/releases/VERSION.md` in English. Describe concrete verified improvements, fixes, upgrade steps and relevant limitations; a compare link supplements the description.
2. Refresh the README's versioned download links and screenshots when they change. Include real captures of the app and Settings. The ZIP must contain the referenced screenshot files as well as README and LICENSE.
3. Check the installed-version display, manual **Check for updates**, default asynchronous startup check and persistent opt-out. Keep **Download and install**, progress, integrity verification, useful errors, settings preservation and restart working. Startup checking alone must not download or install an update.
4. Run the checks in the [Windows workflow](../.github/workflows/windows.yml): formatting, all-feature Clippy, tests, release build and native updater replacement/rollback fixtures. For changes affecting tray activation or hidden-window behavior, also verify the packaged application's actual window reopening on an interactive Windows desktop.
5. Package the checked build and verify that its standalone name, ZIP contents, checksum entries and README links agree. The standalone asset must be the same executable bytes as the checked build and the executable inside the ZIP.

## Publishing

Create a commit on `main` whose first line is exactly `Release vVERSION`, matching Cargo's version. Alternatively, run the Windows workflow on `main` with **publish_release** enabled. Both paths wait for the Windows build checks, then package that exact uploaded artifact.

The release job creates a draft with the versioned English notes, uploads the EXE, ZIP and checksum manifest, then publishes it. It refuses to overwrite an already published release, a tag pointing at another commit, or a draft for another target commit. Verify the final tag, English description, asset names and download/checksum flow after publication.

For automated repository operations, use the connected GitHub plugin. If it lacks release or binary upload tools, an already authorized repository workflow can perform publication. Do not substitute remote Git commands, `gh`, authenticated shell HTTP or new login/token setup without an explicit user request. The installed application's normal public HTTPS update client is independent of these authoring tools.

Documentation-only changes use a normal descriptive commit. They do not change Cargo's version, use a `Release v` publishing commit, or modify an existing release.
