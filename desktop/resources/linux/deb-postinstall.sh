#!/bin/sh
set -e

# Tauri derives this generated filename from productName. The canonical entry is
# shipped separately because GTK/Wayland requires the desktop basename to match
# the reverse-DNS application ID while the Debian package name remains stable.
applications_dir=${PANITA_DESKTOP_APPLICATIONS_DIR:-/usr/share/applications}
legacy="$applications_dir/panita-client.desktop"
if [ -f "$legacy" ] \
    && grep -Fqx 'X-PanitaClient-Owner=com.panitacraft.client' "$legacy" \
    && grep -Fqx 'StartupWMClass=com.panitacraft.client' "$legacy"; then
    rm -f "$legacy"
fi

# Version 1.0.0 used productName as the generated desktop basename. dpkg removes
# obsolete tracked files during a normal upgrade; keep this guarded cleanup for
# interrupted/manual transitions and remove only the exact launcher signature.
previous="$applications_dir/Panita Client.desktop"
if [ -f "$previous" ] \
    && grep -Fqx 'X-PanitaClient-Owner=com.panitacraft.client' "$previous" \
    && grep -Fqx 'StartupWMClass=panita-desktop' "$previous" \
    && grep -Eq '^Exec=(/usr/bin/)?panita-client %u$' "$previous"; then
    rm -f "$previous"
fi

if command -v update-desktop-database >/dev/null 2>&1; then
    update-desktop-database "$applications_dir" >/dev/null 2>&1 || true
fi

exit 0
