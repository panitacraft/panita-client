#!/usr/bin/env bash
# Stage the build host's libdecor for the AppImage. Packages installed through
# apt (libdecor-0-0, libdecor-0-plugin-1-cairo) are copied as they are, so no
# mirror URL or point-release hash is pinned. The staging directory lives under
# the ignored target/ tree and never enters the source worktree.
set -euo pipefail
root=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/../.." && pwd)
libdir=${LIBDECOR_HOST_LIBDIR:-/usr/lib/x86_64-linux-gnu}
docdir=${LIBDECOR_HOST_DOCDIR:-/usr/share/doc/libdecor-0-0}
stage="$root/target/linux-libdecor"

library=$(readlink -e -- "$libdir/libdecor-0.so.0" || true)
plugin=$(readlink -e -- "$libdir/libdecor/plugins-1/libdecor-cairo.so" || true)
if [[ -z "$library" || -z "$plugin" || ! -f "$docdir/copyright" ]]; then
  echo 'Install libdecor-0-0 and libdecor-0-plugin-1-cairo on the build host before packaging the AppImage.' >&2
  exit 1
fi

rm -rf -- "$stage"
install -d -m 0755 "$stage/plugins-1"
# Dereference the soname symlink so the AppImage carries a regular file under it.
install -m 0755 -- "$library" "$stage/libdecor-0.so.0"
install -m 0755 -- "$plugin" "$stage/plugins-1/libdecor-cairo.so"
install -m 0644 -- "$docdir/copyright" "$stage/copyright"

# Everything else the plugin needs (cairo, pango, wayland) comes from the host
# desktop, which is always present on a Wayland session. Report it on the build
# host so a missing dependency is visible before the package is built.
for file in "$stage/libdecor-0.so.0" "$stage/plugins-1/libdecor-cairo.so"; do
  if LD_LIBRARY_PATH="$stage" ldd "$file" | grep -q 'not found'; then
    echo "Unresolved library for $(basename -- "$file") on the build host" >&2
    LD_LIBRARY_PATH="$stage" ldd "$file" >&2
    exit 1
  fi
done
echo "Staged libdecor from $library"
