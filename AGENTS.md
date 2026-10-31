# AGENTS.md

This file is the source of truth for how work is done in this project. Every contributor and every AI agent must follow it. If a request conflicts with any rule here, stop and ask the project owner to confirm before proceeding. Previously confirmed owner decisions remain authorization; do not repeatedly request confirmation for the same decision.

## 1. Project Overview

Panita Client is an unofficial, plug-and-play Minecraft launcher for friends. It manages accounts, downloads the required game files and Java, and runs isolated instances from the Panita API catalog.

- The MVP supports at least two independent vanilla instances, Microsoft and explicit offline accounts, and launching to the Minecraft main menu.
- Prioritize Windows. Design for Linux from the beginning and validate each platform before claiming support. The 0.2 alpha plan includes public custom and pinned Modrinth modpacks, with NeoForge first, on Windows x64 and Ubuntu x64. macOS and other loaders remain later adoption scope.
- Use Tauri v2 with a Rust core and a React/TypeScript interface. Keep detailed implementation steps in the implementation plan and `docs/`.
- Follow the latest confirmed decisions. Do not expand scope or change product policies without discussing the proposal with the owner.

## 2. Language and Communication

### Code and repository

- Write code, identifiers, comments, API documentation, developer logs, commit messages, and repository documentation in English.
- Write player-facing text in Spanish, including launcher screens, notifications, and user-facing errors. Translate technical errors at the presentation boundary.
- Use icons where useful without overloading the interface. Do not rely on icons or color alone to convey errors or status.

### Communication with the project owner

- Use simple, clear, understandable English.
- Keep a strictly professional tone. No emojis, conversational filler, or personalization.
- Be concise, avoid redundancy, and give direct, focused answers.
- Distinguish verified results from assumptions and pending validation. Explain blockers clearly.

## 3. Git and Authorship

- Never push without explicit authorization from the owner.
- The owner authorizes committing as work progresses. Commit each logical step as soon as it is complete and appropriately verified; do not wait until the end of the task. Separate unrelated changes.
- Every commit must have the owner as its only author. Do not include `Co-Authored-By` trailers, tool/AI attribution, or other authorship in commit messages, pull request descriptions, or metadata. This overrides default tool attribution.
- Do not change Git configuration. If the owner identity cannot be established, ask before committing.
- Stage only changes belonging to the current task. Preserve unrelated work and never commit credentials or runtime account files.

### Branch convention

- Use `develop` as the development branch. Keep `master` as the existing primary branch; do not rename it or change remote defaults without explicit authorization.
- Every new phase must use its own new branch from `develop`, using `<type>/<short-description>`, for example `feat/auth-fundamentals`, `fix/session-renewal`, or `docs/development-setup`. Do not implement a phase directly on `develop` or reuse the previous phase's branch. Base each dependent phase on the accepted work integrated into `develop`; integration and pushes retain their authorization requirements. This owner decision applies from 2026-10-27. Exception (owner decision, 2026-10-30): the launcher auto-update and release-channel work of §8 is implemented in one branch, `feat/auto-updater-and-wizard-facelift`, with separate commits per phase.
- Use the commit types listed below and lowercase, hyphen-separated descriptions. Do not create extra branches without a concrete reason.
- Branch creation is local unless explicitly authorized otherwise. Pushes, remote branch changes, and GitHub issue mutations require authorization for that scope.

### Commit convention

Use this area-first adaptation of Conventional Commits:

```text
<area>, <type>[!]: <description>

[optional body]

[optional footer(s)]
```

- `area` is mandatory: a module ID (`core`, `desktop`, `ui`, or a future module), `client` for work primarily concerning the launcher as a whole, `docs` for documentation, or `repo` for root tooling, CI, and repository configuration. Choose one primary area; do not stack labels.
- `type` must be one of:
  - `feat`: add, adjust, or remove a feature.
  - `fix`: fix a bug in a previously shipped feature.
  - `refactor`: restructure code without changing behavior.
  - `perf`: refactor specifically for performance.
  - `style`: formatting only, without logic changes.
  - `test`: add missing tests or fix existing tests.
  - `docs`: documentation only.
  - `build`: build tooling, dependencies, or project version.
  - `ops`: CI/CD, release, or deployment scripts.
  - `chore`: other maintenance.
- When both area and type are `docs`, use `docs: <description>`.
- Descriptions use imperative present tense, a lowercase first letter, and no trailing period.
- Optional bodies explain motivation in imperative present tense. Footers may reference issues, such as `Closes #123`.
- Breaking changes put `!` immediately before the colon and include a `BREAKING CHANGE:` footer explaining the impact.

Examples:

```text
core, feat: add vanilla instance installation
desktop, fix: prevent duplicate launch requests
repo, ops: add Linux build checks
docs: document account persistence
core, feat!: change the account file schema

BREAKING CHANGE: migrate existing account metadata before loading it.
```

## 4. Design Rules

### 4.1 Instance independence and catalog

- Identify installations internally by the immutable catalog UUID. Use a readable directory derived from the catalog slug, `instances/{slug}/`, and persist its association with the UUID. Folder names are not installation identity.
- Assign the directory once and keep it stable across later catalog name/slug changes, disabled states, or removal. Validate directory names for traversal, Windows reserved names, and case-insensitive collisions. If a slug is unsafe or unavailable, use a safe slug-derived name with a deterministic UUID suffix; never adopt, merge, or overwrite another directory based on its name alone.
- Migrate existing `instances/{id}/` installations through a coordinated, recoverable operation that preserves all worlds, settings, installation metadata, and user-owned files. Do not move a directory during installation, repair, or game execution. Preserve the existing installation and report migration failures; do not silently create an empty replacement or lose discoverability. Record migration verification separately from the original UUID-folder installation evidence.
- Isolate worlds, settings, mods, configuration, screenshots, resource packs, shaders, logs, and crash reports per instance. Instance settings override global defaults.
- Use `instances/{slug}/minecraft/` as the game directory. Keep launcher metadata directly in the instance root: `instance.json` schema 4 stores one UUID, `directory`, `catalog` without a duplicate UUID, local `source`, `status`, and setting overrides. Preserve nonempty legacy ownership inventories as `legacy_files`; omit empty inventories. `installed.json` disk schema 4 uses `id`, explicit installed/uninstalled `status`, the exact verified plan, and `natives`. Keep `.installation.lock` and `.mutation.lock` for writer coordination. Use transient `transaction.json` for pending install/update/repair/uninstall recovery, with safe support for legacy `install-journal.json`. Publish a durable inactive marker before removing a completed transaction; reject conflicting copies and preserve unknown or corrupt intents. Prepare new native generations under `natives/{operation}/` only when needed. Preserve legacy `natives-{operation}/` paths recorded by older installations; do not move or delete them during launch. Do not persist duplicate staged plans or create empty native-generation folders.
- Native directories are disposable instance data, approved by the owner on 2026-10-25. The complete instances/{slug}/natives/ container and canonical legacy natives-{uuid}/ entries belong to this disposable namespace. Manual cleanup removes obsolete native entries while preserving the active installed generation. Successful explicit instance updates automatically remove obsolete natives after publishing the new pointer; update recovery repeats that cleanup for a published update, never for an uncommitted one. Uninstall first publishes an uninstalled marker, then removes all native generations, including the former active generation and unknown, edited or receipt-less native contents. Worlds, settings, screenshots, backups, other instances and shared caches remain preserved. Hold the idle instance lease and validate recovery/identity before deletion. Use bounded handle-relative traversal, reject redirected paths, never follow links outside the native area, and report OS locks/access failures honestly. Incomplete uninstall cleanup keeps its recovery intent until deletion finishes; an otherwise successful update may report pending native cleanup without rolling back its published installation. Ownership receipts and DLL/JAR hashes no longer gate native deletion, and runtime ownership tracking/receipt writes are removed. This explicitly supersedes the former ownership, baseline and corrupt-generation-receipt preservation rules only inside disposable native directories. All other client metadata retains its existing corruption/future-schema protections.
- Migrate legacy `.panita` metadata and inline game files under the root writer lock and instance operation lock. Persist migration intent before moving player files, resume after interruption, reject conflicting destinations and unrecognized metadata without deletion, and remove legacy metadata only after publishing the complete new descriptor/manifest. Preserve UUID identity, settings, worlds, shared-cache references, and game-file permissions beneath the private game-directory root. Keep launcher metadata protected and reject redirected entries. Older builds do not understand the new instance schema and must not operate on migrated data.
- Record resolved versions and managed-file ownership per installation. Share only reusable, verified caches; coordinate writes and cleanup across operations and processes.
- Use Prism's cached-check strategy as the Play-time reference, approved by the owner on 2026-10-23. Keep full content verification for installation, repair, recovery and explicit audits. Play may reuse protected, manifest-bound verification receipts when expected hashes, sizes and file metadata remain unchanged; unknown or changed files require full hashes. Apply changed-file revalidation to assets as well, rather than Prism's size-only asset reuse. Keep permission, redirected-path, installation-completeness, catalog and authentication checks on every Play. Missing files or detected mismatches still block Play and require explicit Repair. Receipts are disposable and contain no credentials; absent, malformed or outdated receipts require fresh verification. Accept that metadata-preserving corruption can escape the cached check; this is not an anti-malware boundary.
- Store vanilla clients at `versions/{version}/{version}-{client-hash}.jar`. Store Java at `runtime/java-{major}-{platform}/`, using suffixes such as `-build-2` only when different exact builds need separate directories. Persist and verify the authoritative runtime manifest and artifact hashes; readable names are not proof of identity. Reserve runtime builds under an interprocess writer lock before downloads, reuse matching reservations after interruption, and never adopt unowned directories or overwrite a different build. Existing installation manifests keep resolving their recorded legacy cache paths; do not remove referenced caches during layout changes.
- Use `installed.json` disk schema 5 for modpack installations, approved by the owner on 2026-10-27. It adds the validated pack state (source identity, managed-file ownership with desired and baseline hashes, consumed defaults, skipped configuration and pending manual files) to the exact verified base/loader plan. Vanilla installations remain schema 4. Older builds reject schema 5 and must not operate on it; preserve unknown, corrupt or future data.
- Keep verified public pack files and configuration archives in the shared pack-content cache `content/{algorithm}/{hash[0..2]}/{hash}-{size}`, approved by the owner on 2026-10-27. The algorithm is the strongest supplied hash (`sha512`, `sha1` for CurseForge and manual imports, `sha256` for configuration archives) and the key adds the exact size. Entries have no file extension and are published only after the exact size and every supplied hash verify. Copy cache entries into instances, hashing again, and never hard-link them; a cache name is a claim, never proof. Coordinate writers with interprocess locks per cache shard directory. Instances are independent of the cache, so it is disposable: an explicit player action may clear it. Name the folder and show its size before confirming. Hold the writer locks, skip or report entries used by an active operation, reject redirected paths, never follow links outside `content/`, and report partial failures honestly. Clearing never touches instances, worlds, other caches (`libraries/`, `assets/`, `versions/`, `runtime/`), accounts or settings, and later installs or repairs download the files again. No automatic garbage collection runs.
- Consume the Panita API. The public catalog is read-only; Panita activation (§4.5) adds authenticated calls to the API's auth routes, including POST and DELETE. The launcher does not access Supabase directly or need its database credentials.
- Use exact pinned versions. Vanilla uses the catalog Minecraft version; future Modrinth instances use the pinned pack as their authoritative source.
- Fetch the catalog at startup. Show disabled entries dimmed, with a lock icon and Spanish status text. Check the specific instance through the API on every Play request; an explicit disabled result blocks installation/launch. Disabled or removed entries must not delete local installations or worlds. On 2026-10-30 the owner makes vanilla version transitions mandatory too: a fresh catalog pin mismatch replaces Play with Update, with explicit confirmation and the default world backup. Updates never install automatically. An unavailable current check keeps the existing warning-confirmed offline fallback.
- If the catalog API is unreachable, show the last valid cached catalog and an on-load Spanish toast explaining that installed instances remain available. Before each fallback launch, require a warning confirmation because current instance status could not be checked. Permit only complete local vanilla installations; do not bypass a known disabled state, install missing files, or treat an explicit disabled/missing response as a network outage. Microsoft authentication requirements remain independent.
- Keep the core version-agnostic. Resolve Java and game requirements from authoritative metadata; do not hard-code one Minecraft version, loader, or CPU architecture. Broader compatibility does not expand the MVP automatically.

### 4.2 Accounts and authentication

- Use Microsoft authorization code + PKCE in the system browser with the HTTPS redirect `https://www.panitacraft.com/auth/callback` and `response_mode=fragment`. The website hands the result to `panitacraft://oauth/callback`; Rust validates and completes authentication. Keep the chosen redirect bound to each pending login through token exchange. Direct deep-link authorization is a developer-only fallback. Verify the full handoff and OS registration before claiming support. Do not use an embedded launcher webview or require manual device-code entry for normal login.
- Never request or store Microsoft passwords or MFA codes. Keep credentials and authentication logic in Rust; expose only necessary account metadata to the UI.
- Persist account metadata and only the Microsoft refresh token in local `accounts.json`, in plaintext with restricted permissions. Access tokens remain in backend memory. Do not add a keyring requirement, custom encryption, or API-delivered encryption keys without a new owner decision.
- Associate accounts with stable internal IDs and their issuing public OAuth client ID. Use unique operation and process-session IDs, cancellation, stale-result protection, and proper write coordination.
- Renew the selected Microsoft session at startup and when needed. Validate it remotely with Minecraft Services on every Play request. Attempt at most one renewal and one validation retry after authentication rejection; block launch if unresolved.
- Authentication network/service failures use explicit manual retry, without an unattended transport retry loop. A Microsoft Play request may initiate session recovery, but game execution waits for successful current validation. Explicit offline accounts bypass Microsoft authentication only; catalog and installation checks still apply.
- Distinguish service/network failures from revoked credentials. Show actionable Spanish errors without silently deleting accounts or converting Microsoft accounts to offline accounts.
- Explicit offline accounts may launch fully installed vanilla without connectivity. Microsoft accounts require successful current Play validation.
- Signing out removes both profile and refresh token, clears in-memory sessions, and invalidates pending work. Report incomplete cleanup honestly and prevent stale operations from restoring credentials.
- Keep `account-removals.json` only while account-removal recovery is needed. After
  the account-file update succeeds durably, publish an empty marker before deleting
  it under the same writer lock. Retire validated empty current/legacy markers on
  reconciliation; preserve active, corrupt, unknown or unsafe markers. If deleting
  an inactive marker fails, retain it for a later reconciliation without treating
  completed credential removal as failed. This owner-approved cleanup applies from
  2026-10-24 and does not weaken failed-sign-out recovery.

### 4.3 Security and data protection

- Do not bundle application secrets, client secrets, database/admin keys, or updater signing private keys. Public OAuth client IDs are not secrets; signing private keys belong in CI secrets.
- The Panita launcher key (`X-Api-Key`) is a declared public client identifier, approved by the owner on 2026-10-27. A distributed binary can always be searched for it, so nothing may depend on hiding it: accounts are protected by the member's session token and the confirmation from Discord. Inject it at build time from a repository variable, never as a secret, and never treat it as one. The keys of the bot and the mod are real secrets and never ship in the launcher.
- Treat `accounts.json` as sensitive: anyone able to read it can extract refresh tokens. Never claim it is encrypted or protected against malware running as the same user.
- Use private application-data paths and platform-appropriate permissions: restricted Windows ACLs and owner-only Unix permissions. Run as a normal user; do not elevate to access accounts.
- Use schema versions, migrations, and atomic writes. Protect atomic-write temporaries from creation; they may contain refresh tokens and require the same restrictions and cleanup as the account file.
- Never put credentials or complete account files in logs, frontend payloads, diagnostics, source control, or launcher-generated backups/exports. Redact sensitive headers, arguments, callbacks, and errors, including `Authorization`, `X-Api-Key` and the Panita session token.
- Preserve existing valid data on failed writes. Do not silently replace unreadable or corrupt account data with an empty store.
- Validate network inputs, download hashes, destinations, redirects, and archive paths. Prevent traversal and writes outside intended directories. Apply least-privilege Tauri capabilities and IPC boundaries.
- Updates, repairs, and uninstall operations must protect worlds and user-edited files. Make destructive removal explicit; do not overwrite user data merely because it differs from a pack default.
- On 2026-10-29 the owner approved an optional, unchecked-by-default instance
  uninstall action that permanently erases the selected instance's entire folder,
  including worlds, screenshots, settings, added mods and backups. Show its exact
  UUID-resolved folder and require a separate acknowledgement before deletion.
  Ordinary uninstall continues preserving player data. Exclude active games and
  mutations, reject redirected paths, use bounded handle-relative traversal and
  preserve other instances, accounts and shared caches. Keep a validated external
  `instance-erasures.json` intent with directory identity while cleanup is pending;
  block reuse of a partially erased instance and require explicit retry, never
  automatic erasure at startup. Preserve corrupt/future metadata and intents and
  report partial failures honestly. This exception applies only to the separately
  confirmed full-instance erasure.
- On 2026-10-26 the owner authorizes an optional Windows launcher-uninstall
  action that erases the entire current-user `%APPDATA%\PanitaClient` folder.
  Keep it unchecked by default and require a separate explicit confirmation.
  Name the exact folder and list saved accounts/sessions, settings, all instances
  including worlds, screenshots, configuration and backups, Minecraft/Java
  downloads, caches and local logs. This is irreversible. Ordinary uninstall,
  silent/passive uninstall and installer updates preserve all profile data.
  Exclude live launchers and concurrent startup, reject redirected paths, never
  follow links outside that root, and report partial failures honestly. Do not
  open a partially erased profile or retry erasure automatically at startup.
  This explicitly supersedes data-preservation requirements only for this
  separately confirmed full-profile erasure; all other operations retain them.
- On 2026-10-30 the owner approves a narrow exception to the normal-user rule for
  updating the Linux `.deb` launcher package: the signed, verified package is
  installed through the operating system's `pkexec` prompt only. The launcher
  never reads, stores or relays an administrator password, never falls back to
  `sudo` password piping or a launcher-drawn password dialog, and never elevates
  for accounts, instances or any other operation. If `pkexec` is unavailable or
  denied, show the manual update-required screen of §8 and preserve all data.

### 4.4 Data on disk

Use the current user's application-data directory, with platform-specific paths defined in the implementation plan:

```text
PanitaClient/
├─ accounts.json          # sensitive: metadata and Microsoft refresh tokens
├─ account-removals.json  # temporary nonsecret pending account-removal recovery
├─ panita-session.json    # sensitive: Panitacraft session token and a nonsecret member snapshot; hidden on Windows
├─ settings.json          # indented disk schema 3; compact, explicit keys
├─ update-state.json      # nonsecret launcher-update channel and pending requirement (§8)
├─ insts-cache.json
├─ instance-erasures.json # temporary explicit full-instance erasure intents; never auto-retried
├─ libraries/             # shared, verified caches
├─ assets/
├─ versions/
├─ runtime/
├─ content/               # disposable shared pack-file cache; player-clearable
├─ instances/{slug}/      # stable readable directory mapped to a catalog UUID
│  ├─ instance.json       # schema 4: one UUID, catalog/status/source/settings
│  ├─ installed.json      # schema 4 (vanilla) or 5 (modpack): exact plan and explicit installed/uninstalled status
│  ├─ verification-cache.json # disposable manifest-bound Play verification receipts
│  ├─ transaction.json    # pending operation only; legacy install-journal.json remains recoverable
│  ├─ .installation.lock
│  ├─ .mutation.lock
│  ├─ natives/{operation}/ # disposable platform libraries
│  └─ minecraft/         # isolated player/game data
└─ logs/                  # redacted launcher logs
```

Do not introduce a separate plaintext keychain-fallback `secrets.json`; the chosen persistence mechanisms are `accounts.json` and `panita-session.json`.

### JSON naming and formatting

- Use consistent, conventional English `snake_case` keys in client-owned disk JSON.
  Put `schema` first, then `type` for a discriminator and `id` when applicable.
  Use two-space indentation and a trailing newline. Keep dynamic transaction
  headers in this order as well; JSON readers must not depend on property order.
- Prefer concise domain terms, preserving explicit units and distinctions such as
  `ram_mib`, `account_id` versus `profile_id`, and `java_path`. Use the same names
  for equivalent global and instance settings. Avoid inconsistent synonyms such as
  `schema_version`/`schema` and `kind`/`type` in new disk writes.
- Read supported legacy spellings through validated migrations or aliases, reject
  conflicting spellings, and preserve corrupt/future data. Keep atomic writes,
  privacy, identity checks and recovery guarantees. Never inspect real account
  files or copy credentials into diagnostics to validate formatting.
- Keep disk naming separate from IPC, public API payloads and authoritative
  third-party JSON. Do not rename downloaded Minecraft/Java/catalog fields or
  invalidate their integrity proofs merely for presentation consistency.

This rule was requested by the owner on 2026-10-24. Accounts and catalog caches now
write `schema`; operation transactions write `type`, with `kind` retained only as
a supported legacy spelling. Existing numeric format versions remain validated.

### 4.5 Panita activation (2026-10-27)

The owner approved Panita activation on 2026-10-27. Its API contract is
`launcher-activation-plan.md` in the Panita API repository. It is separate from the
§7 modpack plan, whose exclusion of Panita login and restricted instances scopes
that plan only.

- Activation is a community gate, independent of Minecraft authentication. It is
  not DRM: a modified launcher can skip it, and the real locks are restricted
  instances (the API) and the server whitelist. Public instances stay playable by
  any activated launcher. Do not promise users more.
- The member types their ign or Discord name; the launcher asks the API for a login
  request, shows the serial (`XXXX-YYYY-ZZZZ`) with the Discord instructions, and
  polls every 2 to 3 seconds until the member confirms it in Discord, the serial
  expires (3 minutes) or the login is cancelled. The launcher never talks to
  Discord and uses neither an embedded webview nor a deep link for this flow. Send
  the computer name and operating system as informational device text, reduced to
  a safe alphabet and not editable; never an IP address or a location.
- States are no session, creating, pending, active and active offline. Show
  activation first and then, if no Minecraft account is saved, onboarding to add a
  Microsoft or offline account. A saved Minecraft account never skips activation.
  Minecraft status and the nickname are only hints, never requirements.
- Store `panita-session.json` through the same private store, atomic writes,
  interprocess writer lock and schema validation as `accounts.json`, apart from
  Minecraft accounts and their removal recovery. It holds the session token in
  plaintext with restricted permissions and a nonsecret snapshot of the member (the
  date of birth is never kept). Hide it on Windows after every write as a cosmetic
  aid only. Never claim it is encrypted or protected against malware running as
  the same user; the safety net is revocation with `/sessions` in the Discord bot.
  Preserve a damaged or future file untouched, report it and block activation
  instead of replacing it. If the write fails after confirmation, keep a
  memory-only session, say so and offer a retry.
- A 401 ends an activation only with `details.reason: invalid_token`. `invalid_key`
  (for example after a key rotation), a 403, a proxy page, a timeout, a 5xx or no
  network never delete the session. Signing out closes the session on the API as a
  courtesy and always removes the local copy and the cached catalog; offline it
  says the session stays open on the server until closed with `/sessions`.
- An unverified session launches complete installed instances, restricted ones
  included, with no time limit, keeping the catalog-outage warning confirmation of
  §4.1. Installing, updating and repairing need a recently verified session. A
  revocation takes effect when the launcher is next online. Check the session at
  startup, before Play when it is unverified, on explicit retry and when the window
  regains focus; do not run an unattended retry loop.
- Send the key and the session to the catalog: restricted instances are returned
  only to a request carrying both. Bind the cached catalog to the member's identity
  and remove it on sign-out or rejection. A 404 for a restricted instance means it
  is not visible to the member, never a network failure.
- Ending or rejecting a session cancels launches that have not started and never
  touches a running game, the hide-on-close supervision or an operation already in
  flight.
- Record activation failures in diagnostics as a fixed event with only the error
  code, never the member name, serial, token or service response.

## 5. Architecture

Global settings use indented disk schema 3 with `schema`, `ram_mib`, `jvm_args`,
`java_path`, `game_window`, `console_on_start`, and `launcher` (`selected_instance`,
`window`). Preserve schema 1/2 values when saving in the new format; keep storage
names separate from the frontend command contract. Mark verified launcher
coordination files and `.artifact-locks` hidden on Windows without changing their
permissions or locking behavior. Unix dot-prefixed coordination entries remain hidden
by convention. These owner-requested refinements were accepted on 2026-10-23.

```text
panita-client/
├─ desktop/               # Tauri application, commands, and events
├─ ui/                    # React and TypeScript interface
├─ crates/
│  └─ launcher-core/      # accounts, catalog, installation, and execution
├─ tests/
├─ docs/
├─ .agents/skills/        # optional project skills
└─ .github/workflows/
```

- Keep business logic in the UI-independent Rust core. Keep desktop integration thin and UI state separate from account secrets.
- Closing the launcher window while games run hides it and keeps the owning process supervising those games. Opening the launcher again restores that process and its active-instance controls. Exit automatically after the last game and cancelled pending launch finish if the window remains hidden; keep an open or minimized window alive. Cancel other pending launches when hiding, and preserve the preparation-only close guard. This owner-approved behavior supersedes requiring the launcher window to remain open during gameplay (2026-10-18).
- The owner requested crash notifications and a separate Minecraft log window on 2026-10-23. Unexpected nonzero game exit restores a hidden launcher and shows a Spanish dialog with its exit code, a bounded redacted crash excerpt, a continuously scrolling execution-log viewer and an option to open that instance's existing `minecraft/crash-reports/` directory. Requested stops do not produce crash notifications. Missing reports are reported honestly. On 2026-10-24 the owner explicitly approved unlimited Minecraft log entry retention in memory only, superseding the 256-event console ring. Keep all accepted redacted entries for the current/most recent execution of each instance until replaced or the launcher exits; memory use can grow with output. Bound each IPC read and rendered window, and retain per-line size and redaction protections. The log viewer never accepts arbitrary paths or commands, reads account files, persists/exports console output, or exposes credentials. Structured launcher diagnostics remain separate and retain their 256-event limit.
- Keep storage, networking, and platform integrations testable behind clear boundaries. Avoid abstractions or dependencies without a concrete need.
- The owner approved Prism as a behavioral reference for live Minecraft logs, formatting and local viewer controls on 2026-10-25. Use scoped change notifications with short burst coalescing, sequential cursor catch-up, Log4j time/level/thread/logger formatting, full retained-history search with bounded scans, wrap/colors, and local clear/restore. Keep the approved unlimited memory-only history, redaction, per-line protections and bounded IPC/rendering. Clear must not delete retained history. Do not add persistence, copying, uploading or export. Implement independently; do not copy Prism source.
- Keep phased implementation work in the implementation plan and decision documents. Annex A preserves the accepted account-storage decision within this file so these policies remain available without separate documents.

## 6. Verification and Documentation

- Run checks appropriate to each change. Test authentication failures, cancellation, concurrency, persistence, and user-data preservation meaningfully.
- Validate real browser/PKCE authentication, restart/renewal, Play-time validation, and sign-out before claiming authentication is complete. Device-code diagnostics do not establish these results.
- Validate actual game launch and filesystem protections on every platform advertised as supported; compilation alone is insufficient.
- The owner's Linux VM BOX is for testing the final installed product, not development or builds. Use a separate build environment. Its previously installed development packages may mask missing runtime dependencies; record this limitation and arrange a clean test baseline before claiming clean-install qualification. Do not remove packages or reset the VM without owner authorization.
- Update affected decision documents when an owner-approved policy changes. Remove contradictory superseded instructions.
- Use Annex A for the accepted authentication and storage policy. Local decision documents and implementation issues must remain consistent with it; surface conflicts before implementing them.

## Annex A. Account and Token Storage Decision

This annex is an integral part of AGENTS.md. It preserves the accepted decision, rationale, recovery cases, and validation requirements from the local source document `docs/account-storage-decision.md` as of 2026-10-08. The complete policy is included below; reading a separate file is not required. Update this annex alongside any owner-approved policy changes.
Status: Accepted
Date: 2026-10-05
Updated: 2026-10-24 (temporary completed account-removal markers; authentication and token-storage decisions retained)

### Decision

Panita Client persists account metadata and Microsoft refresh tokens in a local, schema-versioned `accounts.json` with restricted filesystem permissions. Refresh tokens are plaintext in this file. This explicitly supersedes the OS credential-store proposal and the subsequent reopened discussion.

Microsoft, Xbox Live, XSTS, and Minecraft access tokens and their expiration metadata remain only in backend memory. Offline accounts have no tokens. Panita never requests or stores Microsoft passwords or MFA codes and never sends Microsoft credentials or tokens to the Panita API or Supabase. The Panita session token of §4.5 is a separate credential issued by the Panita API and sent only to it.

The selected design prioritizes automatic session persistence across Windows, Linux, and eventually macOS without requiring a keyring service, a master password, or a remote key service. No custom encryption or API-delivered encryption key is planned. Windows is the first implementation target; each additional platform requires permission and lifecycle validation before support is announced.

### Account identity and concurrency

- Persist a stable internal `account_id` UUID, account type, username, Minecraft profile UUID, and the issuing public OAuth client ID. The internal ID is distinct from the Minecraft UUID and does not change with a username change. Microsoft accounts additionally contain the refresh token.
- Give each authentication/renewal operation a unique `operation_id` and each launcher process execution a `process_session_id`. These identify work; they are not secrets or authorization mechanisms.
- Use single-flight renewal per account and stale-result protection. Signing out invalidates pending operations so a late result cannot restore credentials.
- Coordinate all writers, including any future CLI. Use a single-writer process or an interprocess lock with reload-before-mutation; unique IDs and an in-process mutex alone are insufficient.
- Do not hold a global metadata lock while waiting for network responses. Recheck account existence and operation generation before committing results.
- Reauthentication must match the intended profile identity. Do not replace an existing account's credentials with another profile's credentials accidentally. A change of issuing client ID requires reauthentication.

### Interactive authentication

Use the system browser with authorization code + PKCE and the HTTPS redirect `https://www.panitacraft.com/auth/callback`, approved by the owner on 2026-10-12. Request `response_mode=fragment`. The website removes the fragment from the visible URL and forwards the authorization response to `panitacraft://oauth/callback`. It displays continuation instructions, not proof of Minecraft authentication. Tokens and the PKCE verifier remain in Rust. The callback page must exclude sensitive parameters from analytics, logs and diagnostics. This supersedes direct deep-link authorization as the default; that flow remains a developer-only fallback without player-facing controls. Bind the chosen redirect to each pending login and use it unchanged in authorization and token exchange. The owner reports successful HTTPS/fragment Microsoft authorization and public-client token exchange, and completed web implementation; the full launcher-to-website-to-Minecraft flow and installed-platform handling still require validation. Manual device-code entry and an embedded launcher webview are not the normal sign-in experience. Device-code remains diagnostic evidence only.

Use PKCE S256, a cryptographically random verifier and state, strict state validation, and cancellation/timeout cleanup. Validate the exact callback scheme, authority, and path; accept callbacks only for an active operation and reject duplicate or stale results. Route callbacks to the owning launcher process and handle authentication entirely in Rust. If that process exits, require a new sign-in. Custom schemes are not exclusive OS ownership; PKCE remains essential. Do not log the callback URL, callback-bearing process arguments, authorization code, verifier, tokens, or sensitive request/response bodies. Configure the native/public-client redirect in Microsoft Entra and verify it with the approved application before declaring this flow implemented. No client secret is distributed.

Public client ID: `cd7a6116-ad43-4eff-aa9b-a177e1101c45`.

Complete Microsoft → Xbox Live → XSTS → Minecraft Services → Java profile. Request only the required scopes (`XboxLive.signin offline_access`, verified in integration). Microsoft may still request login, consent, or MFA in its own pages.

### Session lifecycle and Play validation

- At startup, proactively acquire/renew the selected Microsoft account's session using its refresh token. Also acquire a session on account selection; do not refresh every saved account unnecessarily.
- A Microsoft Play request may initiate session acquisition/recovery after a previous failure. Never start the game until authentication and current Play validation succeed. Prevent duplicate requests while work is pending. On failure, show an actionable error: retry for temporary failures or sign in again when required. Cancellation and account switching must remain safe.
- Track expiration from `expires_in`/`NotAfter` with a safety margin. Proactively renew near expiry and reassess after system resume. Do not invent a guaranteed expiration date for refresh tokens.
- On every Play request with a Microsoft account, make a current authenticated request to Minecraft Services, such as fetching the Java profile. Local expiration checks alone do not detect early rejection.
- If that request fails due to invalid authentication, attempt one renewal and repeat validation once. Persistent rejection blocks launch and requests reauthentication. Do not retry every 403 as an expired token: inspect the failure and distinguish account/permission errors.
- Network/service failures block launch with that account and offer retry; they do not erase credentials or imply revocation. A successful request establishes acceptance by that service at that moment, not immunity from later revocation.
- Recovery from network/service failures is an explicit manual action, without automatic transport retry/backoff loops. This does not remove normal startup/selection/near-expiry renewal or the bounded authentication-rejection recovery above. Explicit offline accounts skip Microsoft session acquisition and validation, while retaining catalog and installation safety checks. Verify the actual Play UI and game-launch integration in Phase 5.
- Preserve a newly returned refresh token through an atomic account-file update before continuing subsequent authentication stages. If no replacement is returned, retain the existing token. On write failure preserve the old file, retain the new credential only in memory for retry, and do not claim persistence succeeded.
- Account removal/sign-out clears in-memory sessions and removes both the persisted profile and refresh token. There is no separate user-facing sign-out that keeps the profile. A running game is a separate process and can retain its own session.

### File protection and atomic writes

- Place `accounts.json` in the current user's private application-data directory, outside instance folders and downloaded pack content. Run without administrator/root privileges.
- On Linux/macOS, target owner-only directory/file access (0700/0600). On Windows, establish and verify an appropriate restricted DACL; do not assume a POSIX mode setting provides equivalent protection. Do not persist credentials if adequate protection cannot be established.
- Distinguish an absent file from an unreadable or malformed file. Never overwrite unreadable/corrupt account data with an empty account list.
- Create a uniquely named temporary file in the same protected directory, protected from creation. Write the complete document, synchronize it, then use platform-appropriate atomic replacement. Never delete the original first.
- Because the document contains refresh tokens, the atomic-write temporary also contains refresh tokens. This is the only permitted temporary persistence, with identical protections, no export/backup, and safe cleanup of abandoned application-owned temporaries. Access tokens must never be written there.
- Serialize account mutations. A failure before replacement leaves the previous valid file intact. Do not create token-bearing backup copies.
- If persistence fails after successful login, a memory-only authenticated session may continue, with an accurate indication that it was not saved. Never silently loosen permissions or claim a write succeeded.
- For failed sign-out persistence, block in-memory reuse and report cleanup incomplete. Use a durable nonsecret pending-removal marker where writable, reconcile it before restoring sessions, and retry account-file removal. If neither file nor marker can be written, explicitly report that persisted credentials may remain after restart; do not promise successful durable sign-out.
- After the account-file removal is durable, publish a validated empty recovery
  marker and remove it. An interrupted/failed deletion can leave only completed
  work, which reconciliation may remove again. Legacy marker migration must
  durably transfer every pending ID before clearing/deleting the old marker.
  Never delete active or unrecognized markers merely to simplify the directory.

### Failure and recovery cases

| Situation | Behavior |
|---|---|
| Login succeeds and protected save succeeds | Allow authenticated use and remember the account. |
| Startup finds a saved refresh token | Renew the selected account automatically; keep Play disabled until ready. |
| Access token remains in memory | Reuse it, but validate with Minecraft Services on Play. |
| Authentication is rejected | Attempt the bounded renewal described above, then require reauthentication if unresolved. |
| Microsoft/Minecraft authentication service unavailable | Block Microsoft launch, preserve data, offer retry or explicit selection of an offline account. |
| File absent | Show account setup; do not treat as corruption. |
| File unreadable or malformed | Preserve it; show recovery/error state, without resetting it silently. |
| Save/permission failure after login | Keep a temporary in-memory session; preserve existing file and retry safely. |
| Process stops before atomic replacement | Previous account file remains authoritative; safely clean owned stale temporaries. |
| Sign-out succeeds | Remove profile and refresh token, clear memory, invalidate pending work. |
| Sign-out write fails | Apply pending-removal recovery where possible; report incomplete durable cleanup. |
| A renewal completes after sign-out | Discard it; never restore the removed account. |

### Instance policy

The immutable catalog UUID remains the installation identity. Use a stable, validated slug-derived directory as specified in §4.1, with collision handling and recoverable migration from legacy UUID folders. Catalog renames must not move an installation or change its identity.

Microsoft accounts require successful current Play validation even for installed vanilla. They cannot silently become offline accounts or launch using an expired previously validated profile. Explicit offline accounts may launch fully installed vanilla without connectivity; all required game files, Java, assets, libraries, and natives must be local. A fresh vanilla catalog pin mismatch requires an explicit update before Play (owner decision, 2026-10-30); the warning-confirmed outage fallback remains available.

Check the specific instance's API status on every Play request. A disabled result blocks the launch workflow; show disabled cards dimmed with a lock and Spanish status text. During catalog API outages, show the last valid cached list and an on-load Spanish toast indicating that installed instances remain available. Require explicit user confirmation of a warning before each fallback launch of a complete local vanilla installation. A previously known disabled state remains blocked. This fallback does not bypass current Microsoft/Minecraft authentication validation, and explicit disabled/missing responses are not outages. Preserve local data in every case.

The owner-approved 0.2 alpha scope adds public custom and pinned Modrinth modpacks, prioritizing NeoForge. Check the instance status on each Play request. Modpack updates are mandatory: when the current by-ID response names a newer pack version than the complete installed one, replace Play with an Update action naming the installed and available versions, and do not launch until the update is applied. The update needs current availability, a recently verified Panita session and the explicit version-transition confirmation, with the default world backup, of Annex B. During a catalog or network outage, when the current version cannot be fetched, a complete local modpack may launch after an explicit warning on each request that current status/version could not be verified and the pack may be outdated; an earlier observation of a newer version does not block it, because no update can be fetched offline. Never bypass known disabled/missing results, corrupt/incomplete installations, or current Microsoft authentication requirements. Updates/installations/repair still require current availability checks. Catalog failures never delete worlds or installations. This owner decision on 2026-10-27 restores mandatory pack-pin synchronization for modpacks, replacing the same day's optional-update decision, and supersedes the vanilla-only outage fallback restriction for complete modpacks.

### Accepted tradeoffs and boundaries

Anyone able to read `accounts.json` can extract refresh tokens; the file is not encrypted. Restricted permissions reduce access by other ordinary OS users but do not protect against malware running as the same user, administrators, or malicious game code. Copying the file may expose reusable credentials; this design does not claim device binding or prevent external backup software from copying it.

A refresh token can enable continued access within its granted scope. This does not automatically grant full control of the Microsoft account. Deleting local credentials is distinct from server-side revocation and cannot invalidate a stolen copy. File replacement/deletion does not promise forensic secure erasure.

Exclude the complete account file, tokens, atomic-write temporaries, and sensitive error payloads from frontend DTOs, logs, diagnostics, source control, and launcher-generated exports/backups. Exports, if added, must deliberately serialize metadata only. Keep token handling in Rust; expose only required account metadata to the UI.

### Validation required

- Verify browser/PKCE login through the full Minecraft chain with the approved app; existing device-code diagnostics do not establish PKCE support or persistent-storage correctness.
- Verify login, restart, startup renewal, and Play-time validation with real accounts.
- Verify restrictive permissions from creation, atomic replacement, and CRUD under a normal user on every supported OS.
- Test interruption, malformed data, denied access, failed writes, failed sign-out, startup cleanup, and concurrent operations/processes.
- Test expiration, remote rejection before expiry, one-renewal limits, network failures, late callbacks, account switches, and explicit offline account selection.
- Check that only refresh tokens persist in the protected account file and its strictly controlled atomic-write temporaries; no other artifacts contain tokens.
- Verify account-file schema migration across application updates without loss or broader permissions.

### Implementation details to settle during development

Exact OS paths, Windows ACL implementation, interprocess coordination primitive, and pending-removal marker format are implementation tasks, not unresolved product choices. Test the registered deep-link redirect, OS handler installation and process forwarding, and full PKCE chain early. Do not declare tests complete based on this document.

### References and supersession

This annex supersedes the prior OS credential-store decision, the proposal to persist Minecraft access tokens, and the allowance to launch using an expired Microsoft profile. Implementation issues may track progress separately; this annex contains the accepted policy.

Prism source is background for JSON persistence, not code to copy or a guarantee about future releases:
- https://github.com/PrismLauncher/PrismLauncher/blob/develop/launcher/minecraft/auth/AccountData.cpp
- https://github.com/PrismLauncher/PrismLauncher/blob/develop/launcher/minecraft/auth/AccountList.cpp

OAuth reference: https://learn.microsoft.com/en-us/entra/identity-platform/v2-oauth2-auth-code-flow

## Annex B. Maintenance and Desktop Decisions Preserved from the README

These existing decisions and implementation boundaries were consolidated from the
README on 2026-10-25. They supplement the rules above; their historical approval
boundaries do not revoke later explicit owner decisions. Historical test counts
and individual measurements are evidence, not product guarantees.

### Updates, repair, and world backups

- Require explicit confirmation showing the installed and requested Minecraft
  versions before a version transition. Keep the previous installation authoritative
  until verified publication, and recover interrupted publication before Play.
  Updates and repair require a current enabled catalog response; neither uses the
  offline Play fallback. Local uninstall does not require catalog connectivity.
- Repair uses the recorded exact installation plan and Minecraft version, verifies
  Minecraft/Java caches, probes Java, and prepares a fresh native generation when
  required. Failed preparation preserves the previous installation descriptor.
  Repair retains previous native generations until explicit cleanup. Apply §4.1's
  disposable-native policy; old ownership receipts do not gate native deletion.
- Default the version-transition confirmation to backing up worlds. Completed
  backups contain only `minecraft/saves/`, stored inside the instance as
  `world-backups/{operation}/worlds.zip`. Publish schema-versioned `backup.json`
  only after ZIP read-back verification, identifying the instance and installed
  version at backup time. Reject linked/redirected entries, unsafe names, and
  detected source changes. Preserve incomplete directories without a completed receipt.
- Backups exclude account data. Updating never restores or deletes worlds. Explain
  that opening a world in another Minecraft version can change it and reverting
  game files cannot undo those changes. Reinstalling against a changed catalog pin
  uses the same explicit version-transition/backup workflow. Full player-data
  removal and shared-cache cleanup remain outside these maintenance controls.
- Repair/Uninstall controls require an active recorded installation plan; damaged
  cache files remain repairable. Incomplete installations retain install/retry
  controls. Frontend manifest hints never replace backend operation/identity checks.
- Do not run native cleanup or runtime-ownership tracking on Play or game exit.

### Settings and application paths

- Use `%APPDATA%\PanitaClient` on Windows and
  `${XDG_DATA_HOME:-~/.local/share}/PanitaClient` on Linux for private app data.
- Read global settings schemas 1/2 without rewriting them until a successful save.
  Selection/window writers preserve the latest launch settings under the shared
  writer lock. Save window state on close/hide, but do not restore hidden or
  minimized state on the next launch. Older builds cannot read disk schema 3.
- Keep RAM sliders, numeric controls, and editable JVM memory synchronized. Accept
  one `-Xmx` value in MiB (`M`) or GiB (`G`) as authoritative RAM and exclude it
  from additional JVM options to prevent duplicate heap arguments. Preserve global
  versus per-instance inheritance. Custom Java paths remain unsupported by launch
  preflight; the settings UI may clear an existing override.
- `console_on_start` defaults to false globally; instances inherit it unless
  explicitly overridden. Capture the effective preference for each Play attempt
  and open the execution viewer after process creation. Report viewer-opening
  failures separately without stopping the owned game.

### Diagnostics and execution viewer boundaries

- Keep the latest 256 structured launcher events in memory for the current process,
  separate from unlimited Minecraft execution history. Use fixed event categories
  and error codes; download ticks must not flood the viewer. Diagnostics remain
  available when account initialization fails and provide no export or account,
  callback, launch-argument, or raw game-log access.
- Optional support identifiers are random process/session and operation UUIDs.
  They correlate one process or action; they are neither content hashes nor online
  report lookup codes. Clear structured events on process exit.
- Bound execution-viewer IPC/rendered pages and search scans to 500 process events
  per request. Coalesce scoped change notifications over approximately 100 ms and
  catch up sequentially. Search is literal and case-insensitive across retained
  redacted output; it never reads files. Local clear only hides earlier output.
- Omit lines over 8 KiB with a notice; bound child-exit draining to two seconds.
  Capture depends on stdout/stderr flushing and does not guarantee every internal
  Minecraft event. Bound the Windows shell wait when opening crash reports.

### Build and qualification boundaries

- On 2026-10-26 the owner requests a consistent installer/uninstaller height and
  header across phases, minimize/close icons, dragging from non-interactive
  window areas, Windows error sounds and an explicit finish-without-launching
  option. Failed progress must remain closable/minimizable; native navigation
  must not reappear over content. Preserve native worker-completion checks and
  never force-close an active file mutation to implement these controls.

- On 2026-10-26 the owner requests Panita-controlled installer/update/uninstaller
  warnings inside the branded window, including busy-instance errors and the
  separate full-data erasure confirmation. Keep Spanish text, explicit consent,
  safe defaults and idle checks. OS-owned security/runtime dialogs are separate.

- On 2026-10-26 the owner requests keeping the beta checklist only in ignored
  `docs/`. On 2026-10-30 the owner extends frameless styling to the main launcher:
  provide visible accessible window controls and a draggable header on loading,
  activation, onboarding and library screens. Keep the existing close supervision
  and preparation guards. The owner expands the rail on 2026-10-30 to at least
  six visible instances, using additional vertical space before paging. Use a
  700-pixel minimum window height and one selected account avatar; preserve legacy smaller saved sizes by
  clamping their restored height. A busy-launcher uninstall
  warning must leave its confirmation screen available for retry, with a final
  idle check before deletion. Do not force-close the supervising launcher.

- The packaged executable is `panita-client` (`panita-client.exe` on Windows),
  requested by the owner on 2026-10-25. Preserve the application identifier and
  private data paths during this rename. The internal Rust desktop crate and
  stable GTK window class remain `panita-desktop`; legacy executable names remain
  valid only for migration. Installer maintenance must reject a running launcher
  rather than force-close the process supervising games. Verify old-executable,
  shortcut and callback migration without deleting unknown files or player data.

- The owner authorized starting Phase 7 with an unsigned beta on 2026-10-25.
  Code signing is deferred; disclose unsigned Windows publisher/SmartScreen
  limitations. Prepare a compact Panita-branded graphical installer and matching
  uninstall confirmation, using the supplied screenshots as visual references.
  Keep Spanish player-facing text, per-user installation, WebView2 handling,
  protocol ownership checks and preservation of existing player/account data.
  Manual installer updates remain the distribution model; this does not authorize
  self-update, pushes, release publication, deployment or a master merge.
- The owner identifies BOX as the Ubuntu 26.04 installed-product test target.
  Verify its actual OS before recording results. The Ubuntu 24.04 build baseline
  remains separate. BOX is not a clean runtime baseline while development packages
  remain installed; no reset or package removal is authorized.

- Retain the local `private-fs` patch and its Linux regression checks. Its
  proc-descriptor publication fallback resolves the earlier unprivileged
  `linkat(AT_EMPTY_PATH)` blocker while retaining private creation, identity,
  atomicity, and durability checks. See `vendor/private-fs/PANITA-PATCH.md`.
- The Linux build baseline is Ubuntu 24.04. Runtime executables require owner-only
  0700 permissions; private metadata remains 0600. Recheck BOX's current package
  inventory before describing its baseline. On 2026-10-25, read-only inspection
  verifies Ubuntu 26.04.1 x86_64, an existing panita-client 0.1.0 installation and
  GTK/WebKit runtimes; the previously recorded development tools/packages are
  absent. This inventory does not establish a fresh clean-install baseline or
  qualify the Phase 7 candidate. No package removal or VM reset is authorized.
- Phases 0–7 are accepted and complete; Phase 6 closed on 2026-10-25 and
  Phase 7 on 2026-10-26. The owner accepts Windows/Linux functional results and
  teammate-reported clean Windows 11/Ubuntu VM tests. On 2026-10-27 the owner
  confirms release validation. Preserve these as owner-reported results;
  historical BOX observations and beta candidate hashes retain their own scope.
  The current release tag is `v0.1.1-alpha` at this documentation checkpoint.
  CachyOS installation/game launch remain unverified in the available records;
  OS code signing remains deferred. The 0.2 plan now includes signed launcher
  self-update; it is planned, not implemented or qualified at this checkpoint.
  Later explicit release decisions supersede earlier preparation-only boundaries
  for their authorized scope. Future pushes, tags, merges and publication still
  require authorization.

## 7. Post-MVP 0.2 Alpha Decisions (2026-10-27)

- Deliver public custom and pinned Modrinth `.mrpack` installation first, the
  reference-based interim UI facelift second, and mandatory launcher updates and
  automatic release management third. NeoForge is the first qualified loader;
  Fabric, Quilt and Forge adoption follows separately. Do not silently install
  unsupported packs as vanilla. Restricted instances and Panita account login
  are outside this plan. Microsoft/offline account policies remain unchanged.
- For custom packs, use the API's resolved URLs and exact hashes/sizes without
  provider API keys. Files with permitted URLs download automatically. Explicit
  manual CurseForge files require verified player import; required missing files
  block completion and Play. Importing a manual file and re-verifying the pending
  list are local operations that need no Panita session or connectivity;
  downloading the remaining files still needs the §4.5 gate. Install optional
  client files automatically for now.
- Modpack updates are mandatory (owner decision, 2026-10-27): a newer published
  pack version replaces Play with Update until applied, except during a catalog or
  network outage, with the warning described in Annex A. This supersedes the
  earlier optional-update decision for packs. On 2026-10-30 mandatory pin synchronization also applies to vanilla, with explicit update confirmation and the existing outage fallback.
- Player-added mods and configuration are permitted and remain player-owned.
  Preserve them during update, repair and ordinary instance uninstall. Report
  pack/user destination collisions rather than adopting or overwriting user files.
  Additional mods are not a guarantee of compatibility. Preserve worlds and all
  existing user-data protections; separately confirmed full-profile erasure and
  disposable native cleanup retain their explicit scopes.
- Apply defaults only to absent, never-consumed paths. Preserve that history so
  deleted defaults are not recreated on update. Replace managed configuration
  only when it still matches its recorded installed hash; otherwise keep the
  entire edited file and report the skipped replacement. A change in one file
  does not skip other unchanged configuration. Reset to pack requires an explicit
  player action; do not silently merge arbitrary configuration formats.
- Each newer published launcher release is mandatory. Automatically download and
  verify updates; once an update is known and pending, block new game launches
  until applied. Keep supervision, stop/log controls and safe recovery available.
  Apply updates only after games and active mutations finish; do not force-close
  games or interrupt transactions. If update checking is unavailable and no
  mandatory update is already known, permit use with a warning. A failed check
  never clears a known pending requirement. Verify updater signatures; private
  signing keys stay in CI. This does not claim Windows publisher code signing.
- Release channels follow §8 (owner decision, 2026-10-30): merges to `develop`
  publish beta releases and merges to `master` publish stable releases. This
  supersedes the earlier master-only publication rule and the `0.2.x-beta.x`
  scheme. Keep version allocation serialized, retries idempotent, artifacts
  immutable and update-feed publication last. Plan/workflow preparation does not
  itself push, merge, create release tags or publish binaries.
- Use the annotated launcher layout reference and supplied palette for the
  interim facelift: dark `#141414`, `#1B1B1B`, `#515151`; light `#F9FFF8`,
  `#E4FFDA`, `#BABABA`; primary `#54A636`, `#047500`. Theme switching is not
  implied. Use the owner-updated `desktop/icons/source.svg` as the application
  icon source (2026-10-30); preserve its artwork and colors. Use the
  background-bearing logo in navigation headers (owner decision, 2026-10-30).
  Keep Spanish player-facing text, keyboard access, contrast and accessible custom
  main-window controls and dragging. Final design remains a later plan.
- Qualify the new installed-product scope on Windows 11 x64 and Ubuntu x64;
  retain the separate Ubuntu 24.04 build baseline. macOS is outside scope.
  Earlier MVP results do not qualify new modpack or updater behavior.

## 8. Release Channels, Updates and Versioning (2026-10-30)

Owner decisions of 2026-10-30. They replace the master-only release rule and the
`0.2.x-beta.x` scheme of §7; the mandatory-update behavior of §7 and Annex A
still applies to both channels.

### Channels

- A merge to `develop` publishes a **beta** release (`X.Y.Z-beta.N`, GitHub
  prerelease). A merge to `master` publishes a **stable** release (`X.Y.Z`).
  Nothing else publishes: pull requests and unmerged branches only run CI.
- The launcher follows the stable feed by default. Receiving beta versions is an
  explicit player opt-in. The beta feed also carries every stable release, so a
  beta player moves to the stable build when it supersedes the beta.
- Both feeds are static, HTTPS, signed `latest.json` files served as assets of the
  fixed releases `updater-stable` and `updater-beta`. Do not rely on GitHub's
  generic latest-release selection, which skips prereleases, and do not call the
  GitHub REST API from the launcher. The Panita API supplies no update data.
- One workflow builds from the exact merged commit, tags that commit, uploads
  installers, signatures and checksums, and advances the feed last. A feed update
  must carry a strictly greater version than the current feed. Retries are
  idempotent: an existing tag is never rebuilt or replaced. Published binaries are
  immutable; a bad release is superseded by a forward fix.
- Updater signatures are mandatory and distinct from Windows publisher code
  signing. Only the verification public key ships; the private key and its password
  stay in CI secrets. The launcher accepts only artifact URLs under this
  repository's release assets and only a version newer than the running one.
- Update delivery by package: Windows NSIS (Windows 10 and 11, one installer) and
  Linux AppImage update in place; the Linux `.deb` updates through the system
  package manager (see §4.3 exception). Packages the updater cannot own, such as
  the Arch/CachyOS package, show the Spanish update-required screen with a link
  to the release instead of attempting an install. Existing installations without
  an updater need one manual install; do not claim automatic upgrade from them.
- Apply updates only when no supervised game and no active mutation exists, as
  required by §7. Keep a durable pending-update requirement that a failed check or
  a restart never clears.

### Versioning

- The committed version is the single authority across `package.json`,
  `ui/package.json`, both Cargo manifests and `Cargo.lock`, the Tauri
  configurations and installer preview. CI verifies that they agree and that a
  release tag equals the committed version; it never rewrites it and never creates
  version commits. Commits keep the owner as sole author.
- `develop` carries `X.Y.Z-beta.N`, where `X.Y.Z` is the next stable version.
  `master` carries `X.Y.Z` with no suffix. The format stays compatible with
  `scripts/release-version.mjs`.
- Every change that ships in the application binary updates the version by
  magnitude: a breaking change (`!`) raises the major number, `feat` the minor
  number, and `fix`, `perf` or shipped `refactor` the patch number. `docs`,
  `test`, `style`, `chore` and `ops` without binary effect do not change it.
- Derive the minimum base from the last stable tag and the magnitude. If the
  current base already satisfies it, increment `N`; otherwise start the new base
  at `-beta.1`. Promotion to stable removes the suffix in a
  `client, build: release X.Y.Z` commit on `develop` before the pull request to
  `master`.
- The agent states the intended resulting version when it starts a task, accepts
  the owner's override, and applies it once with `scripts/bump-version.mjs` in a
  separate `client, build: bump version to X.Y.Z[-beta.N]` commit immediately
  before the final commit of the change. Parallel branches may collide on the
  number; re-derive it at merge time.
