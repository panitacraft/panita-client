#!/usr/bin/env bash
set -euo pipefail
revision=${1:?Expected source revision}
git clone --branch develop /input/source.bundle /work/panita-client
cd /work/panita-client
test "$(git rev-parse HEAD)" = "$revision"
mkdir -p target
ln -s /build-target target/phase-7-build
pnpm install --frozen-lockfile
node scripts/build-beta.mjs
version=$(node -p "JSON.parse(require('fs').readFileSync('desktop/tauri.beta.conf.json')).version")
artifact="target/beta/$version/linux"
mkdir -p "$artifact/verification"
cat /etc/os-release > "$artifact/verification/build-os.txt"
{ rustc --version; cargo --version; node --version; pnpm --version; } > "$artifact/verification/toolchain.txt"
dpkg-query -W > "$artifact/verification/build-packages.txt"
package=$(find "$artifact" -maxdepth 1 -type f -name '*.deb' -print -quit)
test -n "$package"
dpkg-deb --info "$package" > "$artifact/verification/package-info.txt"
dpkg-deb --contents "$package" > "$artifact/verification/package-files.txt"
mkdir /work/package-check
dpkg-deb --extract "$package" /work/package-check
desktop-file-validate /work/package-check/usr/share/applications/*.desktop
ldd /work/package-check/usr/bin/panita-client > "$artifact/verification/runtime-libraries.txt"
if grep -q 'not found' "$artifact/verification/runtime-libraries.txt"; then
  echo 'Unresolved runtime library in the build environment' >&2
  exit 1
fi
cp /work/package-check/usr/share/applications/*.desktop "$artifact/verification/"
printf '%s\n' 'Package inspection passed. Installed-product testing remains pending.'
