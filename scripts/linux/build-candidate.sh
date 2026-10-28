#!/usr/bin/env bash
set -euo pipefail
revision=${1:?Expected source revision}
mode=${2:-beta}
git clone --no-checkout /input/source.bundle /work/panita-client
cd /work/panita-client
git checkout --detach "$revision"
test "$(git rev-parse HEAD)" = "$revision"
mkdir -p target
ln -s /build-target target/phase-7-build
pnpm install --frozen-lockfile
if [ "$mode" = loader-validation ]; then
    node scripts/build-beta.mjs --loader-validation
    node scripts/verify-beta.mjs --loader-validation
    LOADER_VALIDATION=1 bash scripts/linux/verify-candidate.sh
else
    test "$mode" = beta
    node scripts/build-beta.mjs
    node scripts/verify-beta.mjs
    bash scripts/linux/verify-candidate.sh
    node scripts/build-arch-package.mjs --beta
    node scripts/verify-beta.mjs --with-arch
fi
