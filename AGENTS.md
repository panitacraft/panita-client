# AGENTS.md

This file is the source of truth for how work is done in this project. Every contributor and every AI agent must follow it. If a request conflicts with any rule here, stop and ask the project owner to confirm before proceeding. Previously confirmed owner decisions remain authorization; do not repeatedly request confirmation for the same decision.

## 1. Project Overview

Panita Launcher is an unofficial, plug-and-play Minecraft launcher for friends. It manages accounts, downloads the required game files and Java, and runs isolated instances from the Panita API catalog.

- The MVP supports at least two independent vanilla instances, Microsoft and explicit offline accounts, and launching to the Minecraft main menu.
- Prioritize Windows. Design for Linux from the beginning and validate each platform before claiming support. macOS and modpacks are future scope.
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

- Identify installations by the immutable catalog UUID, using `instances/{id}/`. Names and slugs are presentation metadata, not installation identity.
- Isolate worlds, settings, mods, configuration, screenshots, resource packs, shaders, logs, and crash reports per instance. Instance settings override global defaults.
- Record resolved versions and managed-file ownership per installation. Share only reusable, verified caches; coordinate writes and cleanup across operations and processes.
- Consume the public, read-only Panita API. The launcher does not access Supabase directly or need its database credentials.
- Use exact pinned versions. Vanilla uses the catalog Minecraft version; future Modrinth instances use the pinned pack as their authoritative source.
- Cache the catalog for offline viewing. Disabled or removed catalog entries must not delete local installations or worlds. Installed vanilla updates are explicit, not automatic.
- Keep the core version-agnostic. Resolve Java and game requirements from authoritative metadata; do not hard-code one Minecraft version, loader, or CPU architecture. Broader compatibility does not expand the MVP automatically.

### 4.2 Accounts and authentication

- Use Microsoft authorization code + PKCE in the system browser with a loopback callback. Do not use an embedded launcher webview or require manual device-code entry for normal login.
- Never request or store Microsoft passwords or MFA codes. Keep credentials and authentication logic in Rust; expose only necessary account metadata to the UI.
- Persist account metadata and only the Microsoft refresh token in local `accounts.json`, in plaintext with restricted permissions. Access tokens remain in backend memory. Do not add a keyring requirement, custom encryption, or API-delivered encryption keys without a new owner decision.
- Associate accounts with stable internal IDs and their issuing public OAuth client ID. Use unique operation and process-session IDs, cancellation, stale-result protection, and proper write coordination.
- Renew the selected Microsoft session at startup and when needed. Validate it remotely with Minecraft Services on every Play request. Attempt at most one renewal and one validation retry after authentication rejection; block launch if unresolved.
- Distinguish service/network failures from revoked credentials. Show actionable Spanish errors without silently deleting accounts or converting Microsoft accounts to offline accounts.
- Explicit offline accounts may launch fully installed vanilla without connectivity. Microsoft accounts require successful current Play validation.
- Signing out removes both profile and refresh token, clears in-memory sessions, and invalidates pending work. Report incomplete cleanup honestly and prevent stale operations from restoring credentials.

### 4.3 Security and data protection

- Do not bundle application secrets, client secrets, database/admin keys, or updater signing private keys. Public OAuth client IDs are not secrets; signing private keys belong in CI secrets.
- Treat `accounts.json` as sensitive: anyone able to read it can extract refresh tokens. Never claim it is encrypted or protected against malware running as the same user.
- Use private application-data paths and platform-appropriate permissions: restricted Windows ACLs and owner-only Unix permissions. Run as a normal user; do not elevate to access accounts.
- Use schema versions, migrations, and atomic writes. Protect atomic-write temporaries from creation; they may contain refresh tokens and require the same restrictions and cleanup as the account file.
- Never put credentials or complete account files in logs, frontend payloads, diagnostics, source control, or launcher-generated backups/exports. Redact sensitive headers, arguments, callbacks, and errors.
- Preserve existing valid data on failed writes. Do not silently replace unreadable or corrupt account data with an empty store.
- Validate network inputs, download hashes, destinations, redirects, and archive paths. Prevent traversal and writes outside intended directories. Apply least-privilege Tauri capabilities and IPC boundaries.
- Updates, repairs, and uninstall operations must protect worlds and user-edited files. Make destructive removal explicit; do not overwrite user data merely because it differs from a pack default.

### 4.4 Data on disk

Use the current user's application-data directory, with platform-specific paths defined in the implementation plan:

```text
PanitaLauncher/
├─ accounts.json          # sensitive: metadata and Microsoft refresh tokens
├─ settings.json          # schema-versioned global settings
├─ catalog-cache.json
├─ libraries/             # shared, verified caches
├─ assets/
├─ versions/
├─ runtime/
├─ instances/{id}/        # isolated game data and .panita settings/lock
└─ logs/                  # redacted launcher logs
```

Do not introduce a separate plaintext keychain-fallback `secrets.json`; the chosen persistence mechanism is `accounts.json`.

## 5. Architecture

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
- Keep storage, networking, and platform integrations testable behind clear boundaries. Avoid abstractions or dependencies without a concrete need.
- Keep detailed schemas, endpoint flows, recovery cases, and phased work in the implementation plan and decision documents rather than duplicating them here.

## 6. Verification and Documentation

- Run checks appropriate to each change. Test authentication failures, cancellation, concurrency, persistence, and user-data preservation meaningfully.
- Validate real browser/PKCE authentication, restart/renewal, Play-time validation, and sign-out before claiming authentication is complete. Device-code diagnostics do not establish these results.
- Validate actual game launch and filesystem protections on every platform advertised as supported; compilation alone is insufficient.
- Update affected decision documents when an owner-approved policy changes. Remove contradictory superseded instructions.
- Use `docs/account-storage-decision.md` for authentication details and `docs/issues/` for implementation tracking. If these conflict with this file, surface the conflict before implementing it.
