#!/usr/bin/env bash
set -euo pipefail
root=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/../.." && pwd)
cd "$root"
version=$(node -p "JSON.parse(require('fs').readFileSync('desktop/tauri.beta.conf.json')).version")
[[ "$version" =~ ^[0-9]+\.[0-9]+\.[0-9]+-beta\.[0-9]+$ ]]
artifact="$root/target/beta/$version/linux"
mkdir -p "$artifact/verification"
cat /etc/os-release > "$artifact/verification/build-os.txt"
{ rustc --version; cargo --version; node --version; pnpm --version; } > "$artifact/verification/toolchain.txt"
dpkg-query -W > "$artifact/verification/build-packages.txt"
package="$artifact/panita-client_${version}_linux_x64_unsigned.deb"
appimage="$artifact/panita-client_${version}_linux_x64_unsigned.AppImage"
test -f "$package"
test -f "$appimage"
check_dir=$(mktemp -d)
trap 'rm -rf -- "$check_dir"' EXIT
dpkg-deb --info "$package" > "$artifact/verification/package-info.txt"
dpkg-deb --contents "$package" > "$artifact/verification/package-files.txt"
dpkg-deb --extract "$package" "$check_dir/deb"
desktop-file-validate "$check_dir/deb/usr/share/applications/"*.desktop
ldd "$check_dir/deb/usr/bin/panita-client" > "$artifact/verification/runtime-libraries.txt"
if grep -q 'not found' "$artifact/verification/runtime-libraries.txt"; then
  echo 'Unresolved runtime library in the build environment' >&2
  exit 1
fi
cp "$check_dir/deb/usr/share/applications/"*.desktop "$artifact/verification/"
cp "$appimage" "$check_dir/candidate.AppImage"
chmod u+x "$check_dir/candidate.AppImage"
cd "$check_dir"
"$check_dir/candidate.AppImage" --appimage-extract > "$artifact/verification/appimage-files.txt"
test -x squashfs-root/AppRun
test -x squashfs-root/usr/bin/panita-client
desktop-file-validate squashfs-root/*.desktop
file "$appimage" squashfs-root/usr/bin/panita-client > "$artifact/verification/appimage-info.txt"
cd "$artifact"
sha256sum --check SHA256SUMS
printf '%s\n' 'Package inspection passed. Installed-product testing remains pending.'
