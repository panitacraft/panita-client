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
node scripts/verify-beta.mjs
bash scripts/linux/verify-candidate.sh
