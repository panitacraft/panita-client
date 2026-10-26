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

Panita Client is still **in development**. Windows is the main focus.
Linux testing is underway, and final installer and clean-computer checks
are still pending. macOS and modpacks are planned for the future.

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

Preview the proposed installer design with `node scripts/preview-installer.mjs`
and open the printed local address. Its actions are simulated.

See [AGENTS.md](AGENTS.md) for project rules and accepted decisions, and the
[checks workflow](.github/workflows/checks.yml) for build and test commands.

</details>

## License

Copyright © 2026 Panitacraft. Licensed under
[GPL-3.0-or-later](LICENSE). See [COPYRIGHT](COPYRIGHT) for details.

Panita Client is not affiliated with Mojang or Microsoft.
