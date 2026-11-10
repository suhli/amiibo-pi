#!/usr/bin/env bash
# Build deployable archives from a committed checkout and matching release binaries.
set -euo pipefail
cd "$(dirname "$0")/.."
arch=${1:-$(dpkg --print-architecture)}
case "$arch" in
    armhf) target=armv7-unknown-linux-gnueabihf; machine=ARM ;;
    arm64) target=aarch64-unknown-linux-gnu; machine=AArch64 ;;
    *) echo "Supported package architectures: armhf, arm64" >&2; exit 2 ;;
esac
bin=${AMIIBO_RELEASE_DIR:-target/release}
for name in amiibo-bt amiibo-pi; do
    LC_ALL=C readelf -h "$bin/$name" | awk -v m="$machine" '/Machine:/{found=($2==m)} END{exit !found}' || { echo "$name does not match $arch" >&2; exit 1; }
done
test -s web/dist/index.html
out="$PWD/target/packages"
mkdir -p "$out"
stage=$(mktemp -d "$out/.stage.XXXXXXXX")
trap 'rm -rf -- "$stage"' EXIT
root="$stage/amiibo-pi"
mkdir -p "$root"
git archive HEAD | tar -xf - -C "$root"
install -d "$root/target/release" "$root/web/dist"
install -m 0755 "$bin/amiibo-bt" "$bin/amiibo-pi" "$root/target/release/"
cp -R web/dist/. "$root/web/dist/"
git rev-parse HEAD > "$root/SOURCE_REVISION"
cargo metadata --locked --format-version 1 --filter-platform "$target" > "$stage/metadata.json"
python3 - "$stage/metadata.json" "$root/licenses/third-party" <<'PY'
import json
import shutil
import sys
from pathlib import Path

metadata = json.loads(Path(sys.argv[1]).read_text())
out = Path(sys.argv[2])
def notices(folder, group, name):
    for item in folder.iterdir():
        if item.is_file() and item.name.lower().startswith(("license", "licence", "copying", "notice", "copyright")):
            dest = out / group / name.replace("/", "_")
            dest.mkdir(parents=True, exist_ok=True)
            shutil.copyfile(item, dest / item.name)

for pkg in metadata["packages"]:
    if pkg["source"]:
        notices(Path(pkg["manifest_path"]).parent, "cargo", pkg["name"] + "-" + pkg["version"])
modules = Path("web/node_modules/.pnpm")
if not modules.is_dir():
    sys.exit("Install locked web dependencies before packaging the notices")
for pattern in ("*/node_modules/*/package.json", "*/node_modules/@*/*/package.json"):
    for manifest in modules.glob(pattern):
        pkg = json.loads(manifest.read_text())
        notices(manifest.parent, "npm", pkg["name"] + "-" + pkg["version"])
PY
version=$(sed -n 's/^version = "\(.*\)"/\1/p' Cargo.toml | head -1)
name="amiibo-pi-$version-$arch.tar.gz"
tar --sort=name --mtime="@$(git show -s --format=%ct HEAD)" --owner=0 --group=0 --numeric-owner -czf "$stage/$name" -C "$stage" amiibo-pi
mv "$stage/$name" "$out/$name"
(cd "$out"; sha256sum "$name" > "$name.sha256")
echo "$out/$name"
