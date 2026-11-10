#!/usr/bin/env bash
set -euo pipefail
cd "$(dirname "$0")/.."
echo "Build host: $(uname -srm); userspace: $(getconf LONG_BIT) bits"
rustc -vV
cargo build --locked --workspace --release -j "${CARGO_BUILD_JOBS:-1}"
if [[ ${AMIIBO_PREBUILT_WEB:-0} != 1 ]]; then
    pnpm --dir web install --frozen-lockfile
    pnpm --dir web run typecheck
    pnpm --dir web run check
    pnpm --dir web run build
fi
test -s web/dist/index.html
