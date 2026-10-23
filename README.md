# Panita Client

Panita Client is an unofficial Minecraft launcher for friends. The MVP will
install and launch two independent vanilla instances from the Panita API
catalog, using Microsoft or explicitly selected offline accounts.

This repository is in development. The desktop interface manages accounts,
the catalog, vanilla installation progress/cancellation, and Play. The core
development harness described below remains available for isolated checks.
Phase 5 game launch and process lifecycle are accepted as of
2026-10-19. Desktop Play connects installed vanilla instances to verified launch
preflight, cancellation, per-attempt outage confirmation and game lifecycle
status. Installation and maintenance workflows remain Phase 6; clean-machine
and distribution qualification remain Phase 7. Phase 6 started on 2026-10-19
with typed account commands and tested account UI state transitions. Account
feedback rejects stale results, retries failed initialization, and retains
incomplete sign-out cleanup warnings across account changes. The desktop connects
the core installer, same-version retries, progress, cancellation, and catalog
refresh. Automated lifecycle/state checks pass; native installation and maintenance
UI acceptance and full end-to-end validation remain pending. Phase 6 also provides
global/per-instance RAM, Minecraft window size
and JVM settings, remembered instance selection, and launcher size/maximized-state
persistence. These desktop additions still require native Windows/Linux acceptance.

Global `settings.json` schema 1 is read without rewriting it; the next successful
settings/preference save upgrades it to schema 2 while retaining launch values.
Per-instance settings keep their existing schema. Selection/window writers preserve
the latest launch settings under the shared writer lock. Launcher window state is
saved on close/hide; minimized or hidden state is not restored on the next launch.
Custom Java paths remain unsupported by launch preflight; the settings UI can clear
an existing override. Older builds cannot read the new global settings schema.

The Spanish client-event viewer reads the latest 256 structured events from the
current launcher process on demand. Startup, storage/catalog errors and Play/install/
update/repair/uninstall transitions use fixed event categories and error codes.
Download ticks do not add duplicate events. Optional support details contain only
process/session and operation UUIDs. This command remains available when account
initialization fails. It does not read account storage, callbacks, launch arguments
or raw Minecraft log/console files, and provides no export. These in-memory events
are cleared on process exit; this is not a persistent crash-log viewer. Native
keyboard/layout and full desktop E2E qualification remain incomplete.

Native validation on 2026-10-22 used an isolated Windows x64 debug build and a
synthetic offline account. It verified account/preferences restart, maximized-window
restoration, diagnostic keyboard navigation, installation cancellation/retry,
the preparation close guard, and Minecraft 1.21.1 reaching its actual main menu
with launcher-managed Java 21. Reopening reused the supervising process; normal
game exit also stopped the hidden launcher. No production account/world files or
catalog entries were changed. These observations do not establish installed-package,
clean-machine, Microsoft-login, second-version, Linux or complete maintenance
acceptance.

Native cancellation testing also exposed maintenance buttons on a never-completed
installation. Catalog snapshots now include a structurally validated installation-
manifest hint: Repair/Uninstall require an active recorded plan, while damaged cache
files remain repairable. Incomplete installs retain their install/retry control;
backend ownership and operation checks remain authoritative. Settings validation
also explains rejected JVM memory overrides in Spanish. Both corrections were
verified in the rebuilt native candidate.

The corrected Windows candidate also restored a deliberately damaged recorded
Minecraft client cache through native Repair. The pinned plan, game options and a
synthetic saves marker retained their hashes while a new generation was published.
Native instance RAM override saving kept its 4096 MiB value separate from the
6144 MiB global default. Broader maintenance, interruption and platform acceptance
remain pending.

The repaired installation again reached the real main menu. Normal game exit kept
an open launcher alive, and its diagnostic viewer showed the ordered repair/launch/
exit transitions without download-progress spam. The token-free launch-check
harness confirmed the 4096 MiB instance value. These checks remain separate from
Microsoft, two-version and installed-platform qualification.

Vanilla version changes use an explicit update confirmation for the displayed
installed/catalog versions. The installed version stays available until a verified
new generation is published. A durable update journal recovers an interrupted
publication before Play; old caches and player files are retained. Updates require
a current enabled catalog response and do not use the offline Play fallback.

Repair uses the recorded installation plan and exact Minecraft version, checks
Minecraft/Java caches, probes Java, and prepares a fresh native generation before
publication. Player files and previous native generations are preserved; modified
or unverifiable old native files are reported. Repair requires a current enabled
catalog response. Failed preparation keeps the previous installation descriptor.

Uninstall is available locally and preserves worlds, settings, screenshots,
backups, unknown/modified files, and shared Minecraft/Java caches. It removes only
unchanged manifest-owned native files and marks the installation unavailable for
Play. A schema-3 `installed.json` tombstone retains the exact plan and stable UUID/
directory association for reinstallation. Maintenance journal schema 3 coordinates
repair publication and recoverable uninstall under the instance operation lock.
Older builds cannot operate on these descriptors. Reinstalling a changed catalog
pin uses the explicit version-transition/backup workflow. Full player-data removal
and shared-cache cleanup are outside these controls.

The confirmation defaults to backing up worlds. Completed backups contain only
`minecraft/saves/`, stored as `world-backups/{operation}/worlds.zip` inside the
instance, with a schema-versioned `backup.json` receipt written after ZIP read-back
verification. The receipt identifies the instance and version installed when the
copy was made. Linked/redirected entries, unsafe names and detected source changes
block backup creation. Partial directories are preserved without a completed
receipt. Backups exclude launcher account data; update never restores or deletes
worlds. Opening a world in a new Minecraft version may change it; reverting game
files cannot undo those world changes. Live update and native confirmation/restart
acceptance remain pending.

An isolated Windows build reached the Minecraft 26.3 and 1.21.1 main menus
through desktop Play with a synthetic offline account. Cancellation, launcher
close protection, menu exit and restored Play controls passed. Both synthetic
world files remained unchanged. The owner confirmed Microsoft Play, restart,
offline/outage behavior, account switching, instance independence and stop
controls on Windows, followed by successful tests and actual Minecraft launches
with the separately built Linux acceptance package on BOX. These owner-reported
results establish the tested setup; BOX's development packages still prevent
clean-machine qualification.

Closing the launcher window during gameplay now keeps Panita in the background.
Opening it again restores the same session and active-instance controls. If the
window remains hidden, Panita exits after the last game closes. Other pending
launches are cancelled when hiding; preparation alone still requires completion
or cancellation before closing. Visible and minimized windows remain open.
Windows acceptance verified hiding during Minecraft 1.21.1, restoring the same
launcher process and active controls, and automatic background exit after normal
game shutdown. Multiple-game lifecycle checks pass in automated tests, and the
owner confirms the background regression and supplied Linux acceptance tests
passed. Controlled authentication rejection/recovery and catalog-only outages
pass in composed core tests using synthetic accounts and loopback services.

Phase 2 (accounts, PKCE and session persistence) was accepted by the owner on
2026-10-16. See [the closure record](https://github.com/panitacraft/panita-client/issues/4)
for verification and accepted follow-ups: remaining browser failure observations.
The original WSL atomic-write failure was resolved in Phase 4. Phase 3 catalog
and UUID-based local registry implementation was accepted on 2026-10-16.
[Phase 4 vanilla installation and Java provisioning](https://github.com/panitacraft/panita-client/issues/6)
was formally accepted by the owner on 2026-10-17, with Windows and BOX installer-core
verification and the qualification limits below. Game execution was subsequently
verified and accepted in Phase 5.

Phase 3 catalog startup, disabled/unsupported entries, cached outage behavior,
local-only records and recovery were verified in an isolated Windows release
build and the installed Linux package on BOX. Linux checks also verified cache
filename/schema migration, pending account-removal migration with synthetic
offline accounts, owner-only file permissions and unchanged synthetic world
files. [Phase 3](https://github.com/panitacraft/panita-client/issues/5) is formally
accepted, with the previously documented limits retained. BOX has development packages installed, so this
does not establish clean-install qualification or resolve the deferred WSL
storage-test failure.

The Phase 4 core resolves official version, asset and Java metadata; verifies
downloaded hashes and sizes; reuses shared caches; and commits per-instance
installation journals and locks. Cancellation and interruption preserve user
files. Installation controls are not yet connected to the desktop interface.
Game execution was subsequently verified in Phase 5; final installed-platform
qualification remains separate.

Windows validation installed both catalog pins (26.3 and 1.21.1) with their
required Java 25/21 runtimes, audited the installed files, recovered a deliberately
interrupted installation, and preserved synthetic worlds/options. Final cache
adoption reused all 5,626 and 4,348 artifacts respectively with no artifact
downloads. The workspace suite passed 80 core, two process-writer and three
desktop tests, with one intentional live-test skip. Linux artifact selection and
core compilation passed; actual Linux game execution was subsequently confirmed
by the owner during Phase 5.

The slug-folder follow-up migrated both isolated Windows installations to
`vanilla-actual` and `vanilla-clasica`, preserving all four world/options test
hashes. The updated Windows suite passed 96 tests with one live-test skip;
Clippy and the UI build passed. At that checkpoint Linux compilation passed,
but migration tests were blocked by the protected-write failure; the subsequent
fix and BOX verification are recorded below.

The readable-cache follow-up passed 98 Windows workspace tests with one
intentional live-test skip, plus Clippy and formatting checks. Both isolated
installations were rebuilt using the new version/Java paths, reusing all 4,348
and 5,626 artifacts without artifact downloads and verifying Java 21/25.
All four synthetic world/options hashes remained unchanged. After coordinated
validation-only cleanup, every installed artifact reference still exists with
its expected size; obsolete runtime/client paths, duplicate legacy asset-index
aliases, and seven completed test generations were removed. Installed operation
identities and inactive, matching journals were preserved.

The flat-instance follow-up passed 102 Windows workspace tests (one intentional
live-test skip), Clippy, and formatting. Both practice instances now keep launcher
metadata directly in their root and player/game files in `minecraft/`, with no
`.panita` directory or duplicate staged plans. The four user-file hashes remained
unchanged through migration, a 1.21.1 reinstall (4,348 artifacts reused with zero
downloads), and a 26.3 audit (5,626 artifacts). Reports are consolidated under
`target/phase4-evidence/`; `target/phase4-validation/` contains the installation
data. Linux execution was not qualified at that checkpoint.

The initial 2026-10-17 Linux retest used a fresh Debian WSL source copy under the normal
user: 39 core tests passed, 57 failed, and one live test was intentionally ignored.
Core Clippy passed with warnings denied. A synthetic probe isolated the pinned
`private-fs` atomic-write failure: anonymous-file publication with
`linkat(AT_EMPTY_PATH)` returns `ENOENT` without `CAP_DAC_READ_SEARCH`, before
publishing the destination. This reproduced on both tmpfs and the native Linux
home filesystem. The documented `/proc/self/fd` linking alternative succeeded
with a verified owner-only synthetic file, but is not integrated or qualified.
Linux migration and real installation validation were blocked; no elevated
Linux permissions or weaker storage protections were used. BOX was not used.

The subsequent fix pins a local patch of `private-fs` 0.1.0 under
[`vendor/private-fs`](vendor/private-fs/PANITA-PATCH.md). Linux anonymous-file
publication now falls back to the documented proc-descriptor method when the
original call requires unavailable capabilities. Existing private creation,
identity checks, atomic replacement and durability checks remain in place.
All 96 Linux core tests and both process-writer tests pass, with one intentional
live-test skip. The dependency passes all 53 tests under both its default and
alternate path resolver configurations, including interruption and permission
regressions. Windows workspace regressions passed all 102 tests. Linux core and
Windows workspace Clippy pass with warnings denied; formatting passes. These results resolve
the earlier automated WSL storage/migration blocker. CI now explicitly runs the
patched dependency's Linux tests; no new remote CI run is claimed.

Owner-authorized BOX installation validation passed on 2026-10-17 using the
prebuilt debug installer harness from `932f753`, built separately in Debian WSL.
BOX runs Ubuntu 26.04.1 as a normal user. The first attempt exposed a distinct
Java-permission bug: executable runtime artifacts require owner-only 0700 while
metadata remains 0600. The fix preserves hash/owner/type/link checks and atomic
cache repair; Linux now passes 98 core and two process-writer tests, with one
intentional live-test skip. Windows still passes all 102 workspace tests; both
platforms pass the applicable Clippy and formatting checks.

Both pins installed on BOX with their downloaded Java runtimes (1.21.1 with
Java 21.0.7, and 26.3 with Java 25.0.1; both amd64). Second passes reused all
4,289/5,569 artifacts respectively with zero artifact downloads. Local integrity,
specific corruption rejection and restored integrity passed. Four synthetic
player-file hashes/modes and distinct instance setting overrides were preserved.
Ownership and private modes passed for 735 directories, 10,129 metadata/cache
files and 63 runtime executables. Instance roots use the agreed flat metadata
and minecraft/ layout. Reports are in `target/phase4-evidence/box-phase4/`;
BOX practice data remains at
`/home/vboxuser/.cache/panita-phase4-56c1146-20261017/data`.

These are installer-core checks; the installed GUI/package remains unchanged,
and its package integrity check is clean. BOX was not used for builds, and normal
accounts/worlds were not accessed.
Game execution is accepted in Phase 5, installation UI remains Phase 6, and BOX's
existing development packages still prevent clean-install qualification.

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
installation identity remains the immutable catalog UUID. New installations use
readable folders (`instances/<slug>/`). Schema 2 installation metadata persists
the UUID and assigned directory name; later catalog slug changes keep that folder
stable. Reserved names and collisions use a slug with a UUID suffix. Existing
UUID folders migrate on registry load or before an installation/recovery/audit,
preserving their contents. Busy or running instances are not moved. A persisted
destination allows an interrupted move to resume; occupied destinations are never
merged or replaced. Current instances use schema 3; older launcher builds must
not operate on migrated instance data.

The public catalog is `https://api.panitacraft.com`. Do not store account
credentials in the repository. Public integration checks can be run explicitly
with `node tests/readiness.mjs`; they require network access and may fail when
the live catalog changes.

### Vanilla installation validation

Use the development example with an explicit, isolated absolute data root.
It does not access accounts or start Minecraft. Installation requires a current,
enabled vanilla catalog entry and runs the provisioned Java version check.
Changing an installed Minecraft pin requires the later maintenance workflow.

```text
cargo run -p launcher-core --example vanilla_install -- plan <absolute-root> <minecraft-version> <windows|linux> <os-version>
cargo run -p launcher-core --example vanilla_install -- install <absolute-root> <instance-uuid> <windows|linux> <os-version>
cargo run -p launcher-core --example vanilla_install -- verify <absolute-root> <instance-uuid> <windows|linux> <os-version>
cargo run -p launcher-core --example vanilla_install -- recover <absolute-root> <instance-uuid> <windows|linux> <os-version>
cargo run -p launcher-core --example vanilla_install -- migrate <absolute-root> <instance-uuid> <windows|linux> <os-version>
cargo run -p launcher-core --example vanilla_install -- launch-check <absolute-root> <instance-uuid> <windows|linux> <os-version>
```

This is a development harness, not a product CLI or a platform-support claim.
The `launch-check` operation verifies an existing installation and builds its
launch command using saved instance/global settings and a temporary synthetic
offline identity. It prints only a non-sensitive summary, reads no accounts,
downloads no files and starts neither Java nor Minecraft. Run it against an
isolated practice root; normal registry preparation and coordination locks apply.
Installation runs only on the matching native x64 host. The `plan` operation can
inspect either target's artifacts without downloading its game/runtime binaries.
Libraries and asset indexes use content-qualified cache paths so different
resolved hashes cannot replace another installation's files. Consumers must use
the plan's `library_path` and `asset_index_name` methods when building a launch.
Earlier development layouts remain readable and can supply verified local copies.

New vanilla client downloads use `versions/<version>/<version>-<client-hash>.jar`.
Java downloads use `runtime/java-<major>-<platform>/`, for example
`runtime/java-21-windows-x64/`. A protected, schema-versioned
`.runtime-identity.json` reserves the exact manifest before downloads start.
Matching builds share the directory, including after interrupted downloads;
different builds use `-build-2`, `-build-3`, and so on. Unowned directories are
preserved. Allocation is serialized across processes, while artifact downloads
retain their own locks and hash/size verification. Older recorded cache paths
remain supported; consumers must use `client_path` and the recorded
`java_executable`, never reconstruct a cache path from a display version.
This cache change does not change the instance layout or migrate normal user data.

The `migrate` operation completes only the directory/schema migration and prints
the assigned path; it does not download files or execute Java/Minecraft. Commands
continue to accept the catalog UUID. `install` and `verify` summaries include the
resolved directory. Do not rename folders manually. Unowned folders, duplicate
UUID records, and corrupt ownership metadata are preserved and reported for
recovery instead of being adopted or replaced.

Each instance uses a flat launcher root with `instance.json` (schema 3),
`installed.json` (schema 2), `install-journal.json`, and persistent coordination
locks. Instance setting overrides live in `instance.json`; all game/player files
belong in `minecraft/`. Use `AppPaths::game_directory` for the game working
directory, rather than the instance root. Native generations use
`natives/<operation>/` only when required. Duplicate staged plans and empty native
directories are no longer created. The explicit `plan` harness operation alone
writes developer reports; use a separate evidence root for those reports.

Legacy UUID/slug instances with `.panita` metadata migrate through a durable
intent before any player files move. Interrupted moves resume without merging
conflicting destinations; unknown metadata and corrupt inputs are preserved and
reported. Settings are imported before the new descriptor is committed. Migration
is blocked during game execution or an active installation. Do not run older
launcher builds against migrated instances. Account storage and shared-cache
identities remain separate from this migration.

## License

Copyright © 2026 Panitacraft. Licensed under GPL-3.0-or-later; see
[COPYRIGHT](COPYRIGHT) and [LICENSE](LICENSE).

### Phase 6 owner-requested refinements

The owner reported successful Windows manual checks for the supplied offline/game,
settings, two-instance, lifecycle, repair, preserving uninstall/reinstall and
keyboard/layout scenarios on 2026-10-23. This report does not establish the separate
Microsoft authentication, controlled update, Linux or clean-machine checks.

RAM now has synchronized slider, numeric and editable JVM memory controls. A
single `-Xmx` value accepts MiB (`M`) or GiB (`G`), becomes the authoritative RAM
setting, and is excluded from additional JVM options so the launch command never
gets duplicate heap arguments. Global and instance inheritance remain independent.

Unexpected game exits show a Spanish modal with the exit code and a short redacted
console excerpt. The launcher restores its hidden main window for this notification;
requested stops do not produce crash dialogs. The report-folder action opens only
the current instance's existing `minecraft/crash-reports/` directory and explains
when no report folder exists. A separate per-execution Minecraft log window shows
a bounded, redacted stdout/stderr snapshot, follows new output while running and
remains readable after exit until the launcher closes. It does not read historical
log files, export output or include account data or launch commands. Structured
launcher diagnostics remain separate.

New installations, updates and repairs record `natives/<operation>/` as their
native-generation location. Existing manifests continue to resolve their original
`natives-<operation>/` paths without moving or deleting files during launch. These
are installation-generation UUIDs, not artifact hashes. Runtime libraries may
extract their own DLLs there when Minecraft starts.

Optional legacy library-cache probes no longer create absent Maven directory
trees. JARs continue to use the verified `libraries/<hash-prefix>/<hash>.jar` cache;
referenced legacy JARs remain supported. Play still checks every managed artifact's
full hash and permissions, with parent handles reused within the audit and public
artifact reads checked at their boundaries. Credential-file I/O is unchanged.
SHA-1 is optimized in development builds to reduce debug verification overhead.

A local Windows debug preparation check on the same installed Minecraft 1.21.1
profile fell from 96.8 seconds to 8.65 seconds while verifying all 4348 artifacts.
The final result includes 6132 ms of verification and 231 ms of command construction;
Minecraft's own startup follows separately. This is a single-host measurement,
not a platform-wide timing guarantee. File Explorer launch uses a bounded wait so
a slow Windows shell cannot keep the crash dialog busy indefinitely.
