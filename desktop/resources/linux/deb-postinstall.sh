#!/bin/sh
set -e

# Tauri derives this generated filename from productName. The canonical entry is
# shipped separately because GTK/Wayland requires the desktop basename to match
# the reverse-DNS application ID while the Debian package name remains stable.
legacy=/usr/share/applications/panita-client.desktop
if [ -f "$legacy" ] \
    && grep -Fqx 'X-PanitaClient-Owner=com.panitacraft.client' "$legacy" \
    && grep -Fqx 'StartupWMClass=com.panitacraft.client' "$legacy"; then
    rm -f "$legacy"
fi

if command -v update-desktop-database >/dev/null 2>&1; then
    update-desktop-database /usr/share/applications >/dev/null 2>&1 || true
fi

exit 0
