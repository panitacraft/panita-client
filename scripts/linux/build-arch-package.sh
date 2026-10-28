#!/usr/bin/env bash
set -euo pipefail
root=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/../.." && pwd)
version=${1:?Expected upstream version}
arch_version=${2:?Expected Arch package version}
[[ "$version" =~ ^[0-9]+\.[0-9]+\.[0-9]+(-[0-9A-Za-z.-]+)?$ ]]
[[ "$arch_version" =~ ^[0-9]+\.[0-9]+\.[0-9]+[0-9A-Za-z._]*$ ]]
mode=${3:-release}
case "$mode" in
  release) artifact="$root/target/releases/$version/linux" ;;
  beta) artifact="$root/target/beta/$version/linux" ;;
  *) echo 'Expected beta or release package mode' >&2; exit 1 ;;
esac
deb="$artifact/panita-client_${version}_linux_x64_unsigned.deb"
destination="$artifact/panita-client_${version}_archlinux_x64_unsigned.pkg.tar.zst"
work=$(mktemp -d)
trap 'rm -rf -- "$work"' EXIT
if [[ ! -r /var/lib/pacman/local/ALPM_DB_VERSION ]]; then
  echo 'Initialize the build-host pacman database with sudo install -d /var/lib/pacman/local /var/lib/pacman/sync and sudo pacman-db-upgrade.' >&2
  exit 1
fi

# Reuse the verified Ubuntu-baseline binary; do not rebuild against newer glibc.
dpkg-deb --extract "$deb" "$work/payload"
test -x "$work/payload/usr/bin/panita-client"
desktop-file-validate "$work/payload/usr/share/applications/"*.desktop
install -Dm644 "$root/LICENSE" "$work/payload/usr/share/licenses/panita-client/LICENSE"
tar -cf "$work/payload.tar" -C "$work/payload" usr
cp "$root/scripts/linux/arch/PKGBUILD" "$work/PKGBUILD"
export PANITA_ARCH_VERSION="$arch_version"
PANITA_PAYLOAD_SHA256=$(sha256sum "$work/payload.tar")
export PANITA_PAYLOAD_SHA256="${PANITA_PAYLOAD_SHA256%% *}"
export SOURCE_DATE_EPOCH
SOURCE_DATE_EPOCH=${SOURCE_DATE_EPOCH:-$(git -C "$root" show -s --format=%ct HEAD)}
cd "$work"
# This host is Ubuntu, so Arch runtime dependencies cannot be installed here.
# makepkg still validates the payload checksum and creates native package metadata.
makepkg --nodeps --cleanbuild --noconfirm --config "$root/scripts/linux/arch/makepkg.conf"
package="$work/panita-client-${arch_version}-1-x86_64.pkg.tar.zst"
test -f "$package"
bsdtar -xOf "$package" .PKGINFO > "$work/pkginfo"
grep -Fx "pkgname = panita-client" "$work/pkginfo"
grep -Fx "pkgver = ${arch_version}-1" "$work/pkginfo"
grep -Fx 'arch = x86_64' "$work/pkginfo"
grep -Fx 'depend = webkit2gtk-4.1' "$work/pkginfo"
bsdtar -tf "$package" > "$work/contents"
grep -Fx 'usr/bin/panita-client' "$work/contents"
grep -Fx 'usr/share/licenses/panita-client/LICENSE' "$work/contents"
mkdir "$work/check"
bsdtar -xf "$package" -C "$work/check"
cmp "$work/payload/usr/bin/panita-client" "$work/check/usr/bin/panita-client"
test -x "$work/check/usr/bin/panita-client"
desktop-file-validate "$work/check/usr/share/applications/"*.desktop
grep -Fx 'MimeType=x-scheme-handler/panitacraft;' "$work/check/usr/share/applications/"*.desktop
cp "$package" "$destination"
printf '%s\n' 'Arch package inspection passed. CachyOS installed-product validation remains pending.'
