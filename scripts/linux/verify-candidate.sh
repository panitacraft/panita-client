#!/usr/bin/env bash
set -euo pipefail
root=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/../.." && pwd)
cd "$root"
suffix=
if [[ "${LOADER_VALIDATION:-}" = 1 ]]; then
  test -z "${RELEASE_TAG:-}"
  version=$(node -p "require('./desktop/tauri.beta.conf.json').version")
  artifact="$root/target/loader-validation/$version/linux"
  suffix=_loader-validation
elif [[ -n "${RELEASE_TAG:-}" ]]; then
  version=$(node scripts/release-version.mjs "$RELEASE_TAG")
  artifact="$root/target/releases/$version/linux"
else
  version=$(node -p "JSON.parse(require('fs').readFileSync('desktop/tauri.beta.conf.json')).version")
  [[ "$version" =~ ^[0-9]+\.[0-9]+\.[0-9]+-beta\.[0-9]+$ ]]
  artifact="$root/target/beta/$version/linux"
fi
mkdir -p "$artifact/verification"
cat /etc/os-release > "$artifact/verification/build-os.txt"
{ rustc --version; cargo --version; node --version; pnpm --version; } > "$artifact/verification/toolchain.txt"
dpkg-query -W > "$artifact/verification/build-packages.txt"
package="$artifact/panita-client_${version}_linux_x64_unsigned${suffix}.deb"
appimage="$artifact/panita-client_${version}_linux_x64_unsigned${suffix}.AppImage"
test -f "$package"
test -f "$appimage"
check_dir=$(mktemp -d)
trap 'rm -rf -- "$check_dir"' EXIT
dpkg-deb --info "$package" > "$artifact/verification/package-info.txt"
dpkg-deb --contents "$package" > "$artifact/verification/package-files.txt"
dpkg-deb --extract "$package" "$check_dir/deb"
dpkg-deb --control "$package" "$check_dir/control"
generated_desktop="$check_dir/deb/usr/share/applications/panita-client.desktop"
desktop="$check_dir/deb/usr/share/applications/com.panitacraft.client.desktop"
test -f "$generated_desktop"
test -f "$desktop"
desktop-file-validate "$generated_desktop" "$desktop"
grep -Fx 'StartupWMClass=com.panitacraft.client' "$generated_desktop"
grep -Fx 'X-PanitaClient-Owner=com.panitacraft.client' "$generated_desktop"
grep -Fx 'Name=Panita Client' "$desktop"
grep -Fx 'Exec=/usr/bin/panita-client %u' "$desktop"
grep -Fx 'Icon=panita-client' "$desktop"
grep -Fx 'StartupWMClass=com.panitacraft.client' "$desktop"
grep -Fx 'MimeType=x-scheme-handler/panitacraft;' "$desktop"
grep -Fx 'X-PanitaClient-Owner=com.panitacraft.client' "$desktop"
grep -Fq '/usr/share/applications/panita-client.desktop' "$check_dir/control/postinst"
grep -Fq 'X-PanitaClient-Owner=com.panitacraft.client' "$check_dir/control/postinst"
grep -Fq 'StartupWMClass=com.panitacraft.client' "$check_dir/control/postinst"
test ! -e "$check_dir/deb/usr/share/applications/Panita Client.desktop"
for icon in \
  32x32/apps/panita-client.png \
  64x64/apps/panita-client.png \
  128x128/apps/panita-client.png \
  256x256@2/apps/panita-client.png \
  512x512/apps/panita-client.png; do
  test -f "$check_dir/deb/usr/share/icons/hicolor/$icon"
done
ldd "$check_dir/deb/usr/bin/panita-client" > "$artifact/verification/runtime-libraries.txt"
if grep -q 'not found' "$artifact/verification/runtime-libraries.txt"; then
  echo 'Unresolved runtime library in the build environment' >&2
  exit 1
fi
cp "$desktop" "$artifact/verification/"
cp "$appimage" "$check_dir/candidate.AppImage"
chmod u+x "$check_dir/candidate.AppImage"
cd "$check_dir"
"$check_dir/candidate.AppImage" --appimage-extract > "$artifact/verification/appimage-files.txt"
test -x squashfs-root/AppRun
test -x squashfs-root/usr/bin/panita-client
test -f squashfs-root/panita-client.png
test "$(readlink squashfs-root/.DirIcon)" = panita-client.png
mapfile -d '' -t root_desktops < <(find squashfs-root -maxdepth 1 -name '*.desktop' -print0)
if [[ "${#root_desktops[@]}" -ne 1 ]]; then
  echo "Expected exactly one root AppImage desktop entry, found ${#root_desktops[@]}" >&2
  printf '  %s\n' "${root_desktops[@]}" >&2
  exit 1
fi
appimage_desktop=${root_desktops[0]}
test "$(basename -- "$appimage_desktop")" = panita-client.desktop
desktop-file-validate "$appimage_desktop"
grep -Fx 'Name=Panita Client' "$appimage_desktop"
grep -Fx 'Exec=panita-client %u' "$appimage_desktop"
grep -Fx 'Icon=panita-client' "$appimage_desktop"
grep -Fx 'StartupWMClass=com.panitacraft.client' "$appimage_desktop"
packaged_appimage_desktop=squashfs-root/usr/share/applications/panita-client.desktop
test -f "$packaged_appimage_desktop"
desktop-file-validate "$packaged_appimage_desktop"
grep -Fx 'Exec=panita-client %u' "$packaged_appimage_desktop"
grep -Fx 'StartupWMClass=com.panitacraft.client' "$packaged_appimage_desktop"
test ! -e squashfs-root/usr/share/applications/com.panitacraft.client.desktop
if grep -Fq 'Exec=/usr/bin/panita-client' "$appimage_desktop" "$packaged_appimage_desktop"; then
  echo 'AppImage desktop entries must not execute the host /usr/bin/panita-client path' >&2
  exit 1
fi
test -f squashfs-root/usr/share/icons/hicolor/512x512/apps/panita-client.png
cmp squashfs-root/panita-client.png squashfs-root/usr/share/icons/hicolor/512x512/apps/panita-client.png
file "$appimage" squashfs-root/usr/bin/panita-client > "$artifact/verification/appimage-info.txt"
test ! -e /usr/bin/panita-client
test -x /usr/bin/test
mv squashfs-root/usr/bin/panita-client squashfs-root/usr/bin/panita-client.real
install -m 0755 /usr/bin/test squashfs-root/usr/bin/panita-client
squashfs-root/AppRun panita-apprun-probe = panita-apprun-probe
cd "$artifact"
sha256sum --check SHA256SUMS
printf '%s\n' 'Package inspection passed. Installed-product testing remains pending.'
