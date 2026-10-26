# Unsigned beta testing

Use the exact version, source revision and SHA-256 in the candidate's
`candidate.json` and `SHA256SUMS`. Mark each case **Pass**, **Fail**, or **Not run**;
record the OS version, package version, expected behavior and observed behavior.
Screenshots and short error codes are sufficient for reporting failures. Never
share account files, tokens, callback URLs or complete launch arguments.

The beta.1 installer uses the original wizard. The native branded installer and
`panita-client.exe` rename start with beta.2. A browser design preview is a
simulation and does not establish installer acceptance.

## Windows checklist

Use disposable test worlds for version changes. Keep a separate copy of any world
you value before testing. Launcher uninstall and instance uninstall are different:
launcher uninstall preserves all application data; instance uninstall removes its
managed installation and disposable natives while preserving player data.

| ID | Action | Expected result | Result |
| --- | --- | --- | --- |
| W01 | Open the exact candidate; inspect the signature/version and cancel before installing | Unsigned beta version is correct; Panita branding and Spanish controls appear; cancelling leaves the installed launcher unchanged | Agent pass on Windows 11 Pro 10.0.26200; current display scale only |
| W02 | Install as a normal user, including a folder containing spaces | Installation completes without requiring administrator access for the launcher; `panita-client.exe`, Start menu entry and uninstaller exist | Not run |
| W03 | Upgrade beta.1 to beta.2 with the launcher and games closed | Existing accounts, selection, settings, instances and worlds remain; shortcuts point to `panita-client.exe`; the recorded old `panita-desktop.exe` is removed | Not run |
| W04 | Start from the Start menu, then open the launcher again | One owning launcher process/window; reopening restores it rather than creating competing controls | Not run |
| W05 | Sign in with Microsoft through the system browser | HTTPS callback returns to this launcher; one Panita handler choice; the correct Minecraft profile appears. Enter passwords/MFA yourself | Not run |
| W06 | Exit normally, reopen, select the saved Microsoft account and Play | Session renews as required and Play validates it remotely; a service failure shows actionable retry/sign-in text and does not erase the account | Not run |
| W07 | Install and launch both catalog vanilla instances | Each reaches the main menu with downloaded Java; worlds/settings remain independent | Not run |
| W08 | Cancel an installation, restart the launcher and retry | Cancellation is reported; no incomplete installation becomes playable; retry/recovery completes safely | Not run |
| W09 | Minimize while Minecraft runs, then close/hide and reopen the launcher | Game continues; controls return; WebView2 helper processes may remain while the UI exists. If the launcher stays hidden, it exits after the last game/pending launch finishes | Not run |
| W10 | Stop a game through the launcher; separately test a controlled unexpected exit | Requested stop produces no crash dialog. Unexpected failure restores a hidden launcher and reports the exit code/logs honestly | Not run |
| W11 | Edit global/instance settings and restart | Values and inheritance persist; window state restores visibly; optional console preference is respected | Not run |
| W12 | Repair a test instance, then explicitly update it with worlds backup enabled | Repair uses the recorded version; update shows old/new versions and verifies the backup; player data remains and obsolete natives are cleaned after committed update | Not run |
| W13 | Uninstall and reinstall a disposable instance | Worlds/settings/screenshots/backups remain; all disposable natives are removed; reinstall rediscovers the same instance identity | Not run |
| W14 | Sign out one test account, then restart | Removed account stays absent; other accounts remain; no stale pending login restores it | Not run |
| W15 | With games closed, uninstall the launcher and reinstall | Confirmation explains retained data; launcher, owned shortcut/handler are removed; reinstall finds existing accounts/settings/instances/worlds | Not run |
| W16 | On a separate clean Windows test environment without WebView2, install and start | Embedded bootstrapper installs the required runtime using connectivity; launcher starts. Record actual runtime permissions/prompts separately | Not run |

Do not deliberately damage real installations to test corruption or service
outages. Controlled fixtures cover those cases independently. A clean installation
claim requires W16 on a documented clean baseline, not just this development PC.

## Linux checklist

Wait for the matching Linux candidate. BOX's currently installed 0.1.0 package
cannot establish results for the new beta. Build separately on Ubuntu 24.04; use
BOX (verified Ubuntu 26.04.1 x86_64) for installed-product testing only.

- Install the `.deb` through the package manager; record package dependencies and
  start it as a normal user from the application menu.
- Repeat W03–W15 where applicable, including browser return, one launcher identity,
  saved sessions, both real game releases and data preservation.
- Verify owner-only application-directory/metadata/runtime permissions through
  metadata only; never open account-file contents for a permission test.
- Check desktop icon, grouping, callback association and no duplicate menu entries
  after upgrading from the old executable name.
- Record the existing app/runtime package baseline. The queried development tools
  are absent, but existing runtimes can still mask missing package dependencies;
  clean-install qualification remains a separate result.

## Current evidence

All 21 desktop regression tests pass after the packaged executable rename. This
covers lifecycle, concurrency, supervision and callback-entry unit regressions;
it does not replace installed-product results. All 27 isolated native installer
checks pass using a sleep-only fixture executable, unique test registration keys
and workspace-owned folders. They cover fresh install, unknown legacy metadata,
running-process rejection without termination, executable/shortcut/callback
migration, uninstall, preservation of user/sibling markers and foreign handlers.
The production launcher registration stays unchanged; no account data is read.

Native welcome and uninstall confirmation were visually inspected. Folder and
shortcut controls fit, checkbox contrast is corrected, and cancellation was
observed without installing the real candidate. Standard Windows title bars and
buttons remain native; the browser preview is not an exact pixel specification.
Other DPI settings, progress/completion visual acceptance, real-account/game
workflows and clean-machine qualification remain pending. The NSIS uninstaller's
outer wrapper can return zero when its child rejects an operation, so the fixture
suite verifies payload, registration and process outcomes rather than accepting
that exit code as proof of removal.
