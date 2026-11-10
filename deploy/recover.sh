#!/usr/bin/env bash
set -euo pipefail
[[ $(id -u) == 0 ]] || { echo "Use sudo bash deploy/recover.sh" >&2; exit 1; }
# Stop clients before touching the adapter. Recovery can be run repeatedly.
systemctl stop amiibo-pi.service amiibo-bt.service
/usr/local/libexec/amiibo-pi/amiibo-bt recover
/usr/local/libexec/amiibo-pi/bluez-mode restore
systemctl --no-pager status bluetooth.service
