# Panita Client

Panita Client is an unofficial Minecraft launcher for friends. The MVP will
install and launch two independent vanilla instances from the Panita API
catalog, using Microsoft or explicitly selected offline accounts.

This repository is in development. The current application shell does not yet
install or launch Minecraft.

Phase 2 (accounts, PKCE and session persistence) was accepted by the owner on
2026-10-16. See [the closure record](https://github.com/panitacraft/panita-client/issues/4)
for verification and accepted follow-ups: remaining browser failure observations
and the WSL atomic-write test failure. Phase 3 catalog and UUID-based local
registry implementation is in progress; installation and game execution remain
later work.

Phase 3 catalog startup, disabled/unsupported entries, cached outage behavior,
local-only records and recovery were verified in an isolated Windows release
build. [Phase 3](https://github.com/panitacraft/panita-client/issues/5) remains
open for Linux installed-product checks and owner acceptance.

## Development

The workspace uses Rust 1.96, Node.js 24, and pnpm 11.25. Tauri v2 requires
the platform's system dependencies. See the
[Tauri prerequisites](https://v2.tauri.app/start/prerequisites/). The Linux
build baseline is Ubuntu 24.04; actual player requirements will be published
after clean installed-product tests. Linux automation is defined but has not
run for these local commits.

For an Ubuntu 24.04 build host, install the packages used by CI:

```sh
sudo apt-get update
sudo apt-get install -y libwebkit2gtk-4.1-dev build-essential curl wget file libxdo-dev libssl-dev libayatana-appindicator3-dev librsvg2-dev
```

```sh
pnpm install --frozen-lockfile
pnpm build:ui
cargo test --workspace
cargo clippy --workspace --all-targets -- -D warnings
pnpm tauri dev
pnpm tauri build --no-bundle
```

The application host lives in `desktop/`, the React interface in `ui/`, and
UI-independent code in `crates/launcher-core/`. The `pnpm tauri` wrapper sets
their paths for both Windows and Linux. Phase-specific implementation notes
remain in local `docs/`, which is ignored by Git.

The desktop stores private application data in `%APPDATA%\PanitaClient` on
Windows and `${XDG_DATA_HOME:-~/.local/share}/PanitaClient` on Linux. The
browser-only development preview uses local mock instances; the desktop
catalog fetches the public API and keeps a validated local cache for outages.
Disabled and unsupported instances remain visible with their status; local
installation metadata is kept separately under each immutable instance UUID.

The public catalog is `https://api.panitacraft.com`. Do not store account
credentials in the repository. Public integration checks can be run explicitly
with `node tests/readiness.mjs`; they require network access and may fail when
the live catalog changes.

## License

Copyright © 2026 Panitacraft. Licensed under GPL-3.0-or-later; see
[COPYRIGHT](COPYRIGHT) and [LICENSE](LICENSE).
