# AGENTS.md

This file is the source of truth for how work is done in this project. Every contributor and every AI agent must follow it. If a request conflicts with any rule here, stop and ask the project owner to confirm before proceeding. Previously confirmed owner decisions remain authorization; do not repeatedly request confirmation for the same decision.

## 1. Project Overview

Panita Client is an unofficial, plug-and-play Minecraft launcher for friends. It manages accounts, downloads the required game files and Java, and runs isolated instances from the Panita API catalog.

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

### Branch convention

- Use `develop` as the development branch. Keep `master` as the existing primary branch; do not rename it or change remote defaults without explicit authorization.
- When a separate task branch is needed, branch from `develop` and use `<type>/<short-description>`, for example `feat/auth-fundamentals`, `fix/session-renewal`, or `docs/development-setup`.
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
- Use `instances/{slug}/minecraft/` as the game directory. Keep launcher metadata directly in the instance root: schema-versioned `instance.json` combines UUID/directory/catalog/install-state metadata and instance setting overrides; `installed.json` records the exact verified installation. Keep `install-journal.json`, `.installation.lock`, and `.mutation.lock` there for recovery and writer coordination. Extract required natives into `natives-{operation}/` only when needed; do not persist duplicate staged plans or create empty native-generation folders.
- Migrate legacy `.panita` metadata and inline game files under the root writer lock and instance operation lock. Persist migration intent before moving player files, resume after interruption, reject conflicting destinations and unrecognized metadata without deletion, and remove legacy metadata only after publishing the complete new descriptor/manifest. Preserve UUID identity, settings, worlds, shared-cache references, and game-file permissions beneath the private game-directory root. Keep launcher metadata protected and reject redirected entries. Older builds do not understand the new instance schema and must not operate on migrated data.
- Record resolved versions and managed-file ownership per installation. Share only reusable, verified caches; coordinate writes and cleanup across operations and processes.
- Store vanilla clients at `versions/{version}/{version}-{client-hash}.jar`. Store Java at `runtime/java-{major}-{platform}/`, using suffixes such as `-build-2` only when different exact builds need separate directories. Persist and verify the authoritative runtime manifest and artifact hashes; readable names are not proof of identity. Reserve runtime builds under an interprocess writer lock before downloads, reuse matching reservations after interruption, and never adopt unowned directories or overwrite a different build. Existing installation manifests keep resolving their recorded legacy cache paths; do not remove referenced caches during layout changes.
- Consume the public, read-only Panita API. The launcher does not access Supabase directly or need its database credentials.
- Use exact pinned versions. Vanilla uses the catalog Minecraft version; future Modrinth instances use the pinned pack as their authoritative source.
- Fetch the catalog at startup. Show disabled entries dimmed, with a lock icon and Spanish status text. Check the specific instance through the API on every Play request; an explicit disabled result blocks installation/launch. Disabled or removed entries must not delete local installations or worlds. Installed vanilla updates are explicit, not automatic.
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
PanitaClient/
├─ accounts.json          # sensitive: metadata and Microsoft refresh tokens
├─ account-removals.json  # nonsecret pending account-removal recovery
├─ settings.json          # schema-versioned global settings
├─ insts-cache.json
├─ libraries/             # shared, verified caches
├─ assets/
├─ versions/
├─ runtime/
├─ instances/{slug}/      # stable readable directory mapped to a catalog UUID
│  ├─ instance.json       # UUID/catalog/state and instance setting overrides
│  ├─ installed.json      # exact verified installation manifest
│  ├─ install-journal.json
│  ├─ .installation.lock
│  ├─ .mutation.lock
│  ├─ natives-{operation}/ # only for required extracted natives
│  └─ minecraft/         # isolated player/game data
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
Updated: 2026-10-16 (stable slug-derived instance folders with UUID identity; authentication and storage decisions retained)

### Decision

Panita Client persists account metadata and Microsoft refresh tokens in a local, schema-versioned `accounts.json` with restricted filesystem permissions. Refresh tokens are plaintext in this file. This explicitly supersedes the OS credential-store proposal and the subsequent reopened discussion.

Microsoft, Xbox Live, XSTS, and Minecraft access tokens and their expiration metadata remain only in backend memory. Offline accounts have no tokens. Panita never requests or stores Microsoft passwords or MFA codes and never sends credentials to the catalog API or Supabase.

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

Microsoft accounts require successful current Play validation even for installed vanilla. They cannot silently become offline accounts or launch using an expired previously validated profile. Explicit offline accounts may launch fully installed vanilla without connectivity; all required game files, Java, assets, libraries, and natives must be local. Vanilla catalog updates do not force immediate upgrades of installed instances.

Check the specific instance's API status on every Play request. A disabled result blocks the launch workflow; show disabled cards dimmed with a lock and Spanish status text. During catalog API outages, show the last valid cached list and an on-load Spanish toast indicating that installed instances remain available. Require explicit user confirmation of a warning before each fallback launch of a complete local vanilla installation. A previously known disabled state remains blocked. This fallback does not bypass current Microsoft/Minecraft authentication validation, and explicit disabled/missing responses are not outages. Preserve local data in every case.

Future modpack policy remains outside the MVP: require a successful current catalog check and synchronize changed pins before launch. Define allowable CDN cache age and behavior for disabled/deleted records before shipping modpacks. Catalog failures never delete worlds or installations.

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
