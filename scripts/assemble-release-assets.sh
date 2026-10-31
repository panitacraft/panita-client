#!/usr/bin/env bash
# Verify the downloaded platform builds and gather the files a release publishes into ./assets.
# Run from the directory holding installers/windows and installers/linux. Shared by the real
# publication and the dry run so the two cannot drift apart.
set -euo pipefail
version=${1:?Expected the release version}
(cd installers/windows && sha256sum --check SHA256SUMS)
(cd installers/linux && sha256sum --check SHA256SUMS)
mkdir assets
for file in \
  "installers/windows/panita-client_${version}_windows_x64_unsigned.exe" \
  "installers/linux/panita-client_${version}_linux_x64_unsigned.deb" \
  "installers/linux/panita-client_${version}_linux_x64_unsigned.AppImage"; do
  cp "$file" "$file.sig" assets/
done
cp "installers/linux/panita-client_${version}_archlinux_x64_unsigned.pkg.tar.zst" assets/
cat installers/windows/SHA256SUMS installers/linux/SHA256SUMS > assets/SHA256SUMS
(cd assets && sha256sum --check SHA256SUMS)
