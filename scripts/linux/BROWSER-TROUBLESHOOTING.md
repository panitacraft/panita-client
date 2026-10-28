# Linux browser-opening regression

A tester reports that 0.1.1-alpha on CachyOS cannot open Microsoft sign-in from
both the AppImage and the Arch package. Terminal `xdg-open` and `gio open` work.
This weakens an AppImage-only explanation; the original GTK/GIO path discarded
all failure detail. The exact cause has not been reproduced locally.

The launcher keeps GIO desktop activation when available, allows GIO to launch
without an optional GTK context, and falls back once to `/usr/bin/xdg-open` on
GIO/dispatch failure. The fallback runs off the window event loop, passes the
URL as one literal argument and suppresses child output. AppImage library/GTK
module overrides are removed only from the fallback child environment. Desktop,
D-Bus and activation variables remain available. Fallback window focus is best
effort and still needs installed-product validation.

The helper exit is observed for up to 15 seconds. A successful exit means the
opener accepted the request, not proof of a visible browser. A timeout is reported
without killing the helper, because it may have replaced itself with the browser.
A background waiter reaps it when it exits. No second fallback is attempted.

Diagnostics expose only fixed Spanish messages for dispatch, unavailable context,
GIO failure, system-opener failure/timeout and fallback success. They never include
authorization URLs, callbacks, native error text or child output.

## Installed-product retest

Use a new build containing this fix; reinstalling 0.1.1-alpha does not include it.
On CachyOS, test both the native package and AppImage. Record the package/version,
desktop session (Wayland/X11), browser and the launcher's fixed diagnostic stages.
Verify that Microsoft sign-in opens a browser, the HTTPS handoff returns to the
launcher and authentication completes. Also verify cancellation/retry and that a
running browser is not stopped. Do not share OAuth URLs, callback parameters or
account files. Compilation and synthetic opener tests do not qualify this flow.

## Local verification

On 2026-10-27, a separate Debian WSL build environment passes Linux desktop
compilation, six focused Rust tests (opener behavior, URL/environment handling,
diagnostic privacy and login concurrency), and desktop Clippy with warnings
rejected. All 65 frontend tests, the TypeScript/production frontend build, Rust
formatting and diff whitespace checks pass. These are source-level results;
installed-product CachyOS confirmation remains pending.
