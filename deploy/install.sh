#!/usr/bin/env bash
set -euo pipefail
cd "$(dirname "$0")/.."
[[ $(id -u) == 0 ]] || { echo "Run sudo bash deploy/install.sh after building." >&2; exit 1; }
source /etc/os-release
echo "Installing on ${PRETTY_NAME:-Linux} ($(uname -m), $(getconf LONG_BIT)-bit)"
for tool in bluetoothctl btmgmt hciconfig hcitool systemctl openssl; do command -v "$tool" >/dev/null || { echo "Missing $tool" >&2; exit 1; }; done
bluetoothctl --version
builddir=${AMIIBO_RELEASE_DIR:-target/release}
for binary in amiibo-bt amiibo-pi; do test -x "$builddir/$binary" || { echo "Missing $builddir/$binary" >&2; exit 1; }; done
test -s web/dist/index.html || { echo "Build the web assets first" >&2; exit 1; }
"$builddir/amiibo-bt" diagnose
getent group amiibo-pi >/dev/null || groupadd --system amiibo-pi
id amiibo-pi >/dev/null 2>&1 || useradd --system -g amiibo-pi -d /var/lib/amiibo-pi -s /usr/sbin/nologin amiibo-pi
[[ $(id -u amiibo-pi) != 0 && $(id -gn amiibo-pi) == amiibo-pi ]]
install -d -m 0755 /usr/local/libexec/amiibo-pi /usr/local/share/amiibo-pi/web /etc/amiibo-pi
install -m 0755 "$builddir/amiibo-bt" "$builddir/amiibo-pi" deploy/bluez-mode /usr/local/libexec/amiibo-pi/
cp -R web/dist/. /usr/local/share/amiibo-pi/web/
if [[ ! -e /etc/amiibo-pi/server.env ]]; then
    umask 077
    tmp=$(mktemp /etc/amiibo-pi/server.env.XXXXXX)
    cat deploy/server.env > "$tmp"
    printf 'AMIIBO_TOKEN=%s\n' "$(openssl rand -hex 32)" >> "$tmp"
    mv "$tmp" /etc/amiibo-pi/server.env
fi
[[ -e /etc/amiibo-pi/bluetooth.env ]] || install -m 0600 deploy/bluetooth.env /etc/amiibo-pi/bluetooth.env
install -m 0644 deploy/amiibo-{pi,bt}.service /etc/systemd/system/
systemctl daemon-reload
systemctl enable amiibo-bt.service amiibo-pi.service
systemctl restart amiibo-bt.service amiibo-pi.service
systemctl --quiet is-active amiibo-bt.service
systemctl --quiet is-active amiibo-pi.service
echo "Installed. Read the access token locally with sudo cat /etc/amiibo-pi/server.env"
