<div align="center">
  <img src="ui/src/assets/logo_background.svg" alt="Panitacraft logo" width="160" height="160" />
  <h1>Panita Client</h1>
  <p><em>Ready to play. Ready for another adventure.</em></p>
</div>

Panita Client is an unofficial Minecraft launcher for the Panitacraft community. Find the
Panitacraft instances, install the one you want, and play from one place.
The launcher handles the downloads and Java setup for you, with a friendly and intuitive interface.

## Made for playing together

- **Your adventures, kept separate.** Each instance has its own worlds,
  settings, and screenshots, so one adventure stays separate from another.
- **Your choice of account.** Sign in with Microsoft through your browser
  or choose an offline account.
- **Simple installation.** Pick an instance and let Panita download what
  it needs. You can cancel and try again later.
- **Settings that fit your computer.** Adjust memory and window size for
  all your instances, or set them differently for each one.
- **Updates on your terms.** Choose when to update, with world backups
  enabled by default before changing Minecraft versions.
- **Help when something goes wrong.** Repair game files, follow the game
  console, and see error details if Minecraft closes unexpectedly.
- **Keep playing in the background.** Close the launcher while Minecraft
  runs and reopen it to return to your game controls. If it stays hidden,
  the launcher exits after your last game closes.

## Before you play

Panita Client is an **unsigned prerelease**. The agreed MVP phases are complete,
and the owner confirms release validation on 2026-10-27. Packaged application
tests are owner-reported as passed on Windows 11 x64 and Ubuntu 26.04.1 x64. The owner
also confirms teammate testing passed on clean Windows 11 and Ubuntu VMs.
Other Windows/Linux versions are unqualified; macOS, ARM, Flatpak and modpacks
are future scope.

Microsoft accounts need an internet connection each time you start a game.
Offline accounts can play an already installed instance without Microsoft
sign-in. The launcher never changes your account type automatically.

If the Panita catalog is temporarily unavailable, you can use a complete
local installation after confirming a warning. An instance already known
to be disabled stays blocked. Installing, updating, and repairing need an
internet connection and an available instance in the catalog.

## Your worlds stay yours

Updating and repairing preserve your worlds and personal settings.
Uninstalling an instance also keeps your worlds, screenshots, and backups
so you can reinstall later.

Before opening a world in a newer Minecraft version, keep a backup.
Returning to an older game version does not undo changes made to a world.

Microsoft sign-in happens in your browser. Panita never asks for your
Microsoft password. Saved sign-in information is kept on your computer
with restricted access, but it is not encrypted. Do not share the launcher's
account file.

<details>
<summary>For contributors</summary>

The app uses Tauri, Rust, React, and TypeScript. For local development, use
Rust 1.96.0, Node.js 24, pnpm 11.25.0, and the platform's
[Tauri prerequisites](https://v2.tauri.app/start/prerequisites/).

```sh
pnpm install --frozen-lockfile
pnpm tauri dev
```

Prepare a local unsigned beta candidate from a clean committed tree with
`pnpm build:beta`. Add `--check` to inspect the version, source revision and output
location without building. Candidates, SHA-256 checksums and provenance are written
under `target/beta/`; the command does not publish them. Installed-product testing
is still required before distribution. Windows candidates embed the WebView2
  bootstrapper; installing WebView2 still requires internet access when it is absent.
  Windows packaging also requires the MSVC x86 tools to compile the native NSIS
  presentation plugin; the launcher itself remains x64.

From Windows with Docker Desktop running Linux containers, use
`pwsh -File scripts/build-linux-beta.ps1` to prepare Linux `.deb` and `.AppImage` files in a separate
Ubuntu 24.04 build environment. The command builds committed `develop`, verifies
the package contents, AppImage extraction and runtime linkage, and copies both candidates into
`target/beta/`. It does not install anything on the testing VM. Compilation and
package inspection do not establish installed-product qualification.

The [manual beta workflow](.github/workflows/prepare-beta.yml) checks and packages
committed `develop` into temporary Actions artifacts for review. It requires an
explicit dispatch with the matching beta version, uses read-only repository
permissions, and does not create releases or tags. Dispatch and publication need
owner authorization. Each newly built artifact requires its own qualification;
prior test reports do not certify a different file. Release preparation documents
remain local under the ignored `docs/` directory.

The [release workflow](.github/workflows/release.yml) runs when a `v*` tag is pushed
to GitHub. Use a semantic version such as `v0.1.0-alpha`, `v0.1.0-beta.1` or
`v0.1.0`. The tagged commit must contain the workflow and packaging scripts.
The tag supplies the packaged application version; the internal beta version
does not need to match. Build metadata (`+...`) is not accepted.

Both Windows x64 and Linux x64 must pass checks before publication. The workflow
attaches an unsigned Windows `.exe`, Linux `.deb` and `.AppImage`, a CachyOS/Arch
Linux `.pkg.tar.zst`, and a combined
`SHA256SUMS` to the matching GitHub Release. If the release does not exist, it
creates one with generated notes, marking prerelease versions accordingly.
Existing release notes are preserved, and existing assets are not overwritten;
an upload with conflicting filenames fails. Only the publication job has write
permission. Unsigned Windows installers may display publisher/SmartScreen warnings.
CI checks do not replace installed-product qualification.

The CachyOS/Arch package reuses the verified Ubuntu-built binary, following
[Tauri's Debian-to-Arch packaging approach](https://v2.tauri.app/distribute/aur/).
It is packaged with `makepkg` on the Ubuntu build runner, with Arch runtime
dependencies declared for installation by `pacman`. It is not compiled on CachyOS
and does not use CachyOS-specific CPU optimizations. Installation and launch on
CachyOS remain pending validation. This does not publish to AUR or an Arch repository.
For a downloaded package, install it on CachyOS with:

```sh
sudo pacman -U ./panita-client_0.1.0-alpha_archlinux_x64_unsigned.pkg.tar.zst
```

Use the actual downloaded filename. Arch package metadata maps `0.1.0-alpha`
to `0.1.0pre.alpha-1` so prereleases sort before `0.1.0-1`; the launcher retains
its original version. Pacman manages the application files; removing the package
preserves player data in the user's private application-data directory.

Creating a local tag alone does not start GitHub Actions. Publish an approved tag
only after the workflow changes are merged into the intended release commit.
Existing tags do not run this workflow retroactively, and tags without `v` do not
trigger it. No tags or branches are pushed by the packaging scripts.

For local inspection, run the following from a clean tree at that tag:

```sh
node scripts/build-beta.mjs --release-tag v0.1.0-alpha --check
```

Omit `--check` to build into
`target/releases/<version>/<platform>/`, then run
`node scripts/verify-beta.mjs --release-tag v0.1.0-alpha`. Linux package inspection
also accepts `RELEASE_TAG=v0.1.0-alpha bash scripts/linux/verify-candidate.sh`.
To additionally create the native CachyOS/Arch package on Ubuntu 24.04, install
`makepkg=6.0.2-6ubuntu2`, `pacman-package-manager=6.0.2-6ubuntu2`, `fakeroot`,
`libarchive-tools`, `zstd` and `desktop-file-utils`. Initialize the empty build-host
package database with `sudo install -d /var/lib/pacman/local /var/lib/pacman/sync`
and `sudo pacman-db-upgrade`, then run
`node scripts/build-arch-package.mjs v0.1.0-alpha`. Verify the full Linux set with
`node scripts/verify-beta.mjs --release-tag v0.1.0-alpha --with-arch`.

Preview the proposed installer design with `node scripts/preview-installer.mjs`
and open the printed local address. Its actions are simulated.

See [AGENTS.md](AGENTS.md) for project rules and accepted decisions, and the
[checks workflow](.github/workflows/checks.yml) for build and test commands.

</details>

## License

Copyright © 2026 Panitacraft. Licensed under
[GPL-3.0-or-later](LICENSE). See [COPYRIGHT](COPYRIGHT) for details.

Panita Client is not affiliated with Mojang or Microsoft.
