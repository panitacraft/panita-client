# Panita Client

Panita Client is an unofficial Minecraft launcher for friends. The MVP will
install and launch two independent vanilla instances from the Panita API
catalog, using Microsoft or explicitly selected offline accounts.

This repository is in development. The current application shell does not yet
install or launch Minecraft.

## Development

The workspace uses Rust 1.96, Node.js 24, and pnpm 11.25. Tauri v2 requires
the platform's system dependencies. See the
[Tauri prerequisites](https://v2.tauri.app/start/prerequisites/). The Linux
build baseline is Ubuntu 24.04; actual player requirements will be published
after clean installed-product tests.

```sh
pnpm install --frozen-lockfile
pnpm build:ui
cargo test --workspace
cargo clippy --workspace --all-targets -- -D warnings
pnpm tauri dev
```

The application host lives in `desktop/`, the React interface in `ui/`, and
UI-independent code in `crates/launcher-core/`. The `pnpm tauri` wrapper sets
their paths for both Windows and Linux. Phase-specific implementation notes
remain in local `docs/`, which is ignored by Git.

The public catalog is `https://api.panitacraft.com`. Do not store account
credentials in the repository. Public integration checks can be run explicitly
with `node tests/readiness.mjs`; they require network access and may fail when
the live catalog changes.

## License

Copyright © 2026 Panitacraft. Licensed under GPL-3.0-or-later; see
[COPYRIGHT](COPYRIGHT) and [LICENSE](LICENSE).
