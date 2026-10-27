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
maintenance and copy operations. Launcher uninstall always preserves application
data; no delete-data checkbox or recursive app-data removal is included.

When updating the pinned Tauri CLI, compare this template against that version
and repeat installer upgrade/uninstall/ownership tests. Test native dialog layout
at the target Windows DPI settings; browser-preview checks are insufficient.
The installer/uninstaller outer window is frameless. Interactive pages provide
their own Spanish minimize/close controls and a draggable brand header; button
widgets remain Windows-native. The main launcher retains its window decorations.
Busy uninstall is checked by the confirmation page's leave callback, so dismissing
the warning keeps that page available for retry. The section guard still checks
again before deletion to catch a launcher started after preflight. The NSIS
uninstaller's outer wrapper may report zero after its child rejects an operation;
validate the UI and filesystem/registration outcomes rather than that code alone.
