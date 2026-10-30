# Windows installer

The unsigned beta uses `installer.nsi` with native NSIS dialogs from
`installer-ui.nsh`. The HTML preview is a separate simulation, not an installer.
The package executable is `panita-client.exe`; the Rust desktop crate keeps its
internal name. Preserve the application identifier and product name on upgrades.

The template is adapted from Tauri CLI 2.12.2:
https://github.com/tauri-apps/tauri/blob/tauri-cli-v2.12.2/crates/tauri-bundler/src/bundle/windows/nsis/installer.nsi

Original SHA-256:
`166f9677d42e81c7f712a1bf80d9d8c089c21d2967d8398474768b5eac14b494`.
The upstream MIT license is retained in `TAURI-LICENSE-MIT`.

Panita changes the welcome/completion/uninstall pages, layout, colors and progress
presentation. It retains upstream WebView2 setup, upgrade/version checks,
old-executable and shortcut migration, protocol registration and precise uninstall
behavior. Exact-file Restart Manager probes replace force-closing with rejection
while a launcher process is running, including the legacy executable before
maintenance and copy operations. Ordinary launcher uninstall preserves application
data; full removal is available only when the player explicitly selects full-profile removal and confirms its irreversible scope.

When updating the pinned Tauri CLI, compare this template against that version
and repeat installer upgrade/uninstall/ownership tests. Test native dialog layout
at the target Windows DPI settings; browser-preview checks are insufficient.
The installer/uninstaller outer window is frameless. Interactive pages provide
their own Spanish minimize/close controls and a draggable brand header; button
widgets remain Windows-native. The main launcher also uses a custom title bar
with window controls and dragging; its existing close supervision remains active.
Busy uninstall is checked by the confirmation page's leave callback, so dismissing
the warning keeps that page available for retry. The section guard still checks
again before deletion to catch a launcher started after preflight. The NSIS
uninstaller's outer wrapper may report zero after its child rejects an operation;
validate the UI and filesystem/registration outcomes rather than that code alone.

## Design preview

Run `node scripts/preview-installer.mjs` from the repository root, then open
`http://127.0.0.1:1431/`. Pass a different port as the first argument when needed,
for example `node scripts/preview-installer.mjs 1432`. The preview simulates installation, update and uninstall
with selectable ready, progress, completion and error states. Routes and progress
are synthetic; no executable is run and no files, accounts or settings are changed.

The standalone preview uses the launcher palette and navigation logo. The supplied
installer and uninstaller artwork is kept in `desktop/icons/*-source.svg`, with
PNG and ICO variants for preview and later native packaging. Embedded PNG MIME
labels are normalized without changing the supplied artwork. This preview review
does not apply its visual changes or icon selection to the native NSIS installer.
