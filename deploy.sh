#!/usr/bin/env bash
# Build for the reMarkable 2 and copy the app to it. Git Bash on Windows, Linux or macOS.
# Usage:  ./deploy.sh [tablet address] [--no-build]       (default address: 10.11.99.1, USB)
set -euo pipefail
cd "$(dirname "$0")"
export MSYS_NO_PATHCONV=1   # Git Bash: leave the tablet-side paths alone

device=10.11.99.1
build=1
for arg in "$@"; do
  case "$arg" in
    --no-build) build=0 ;;
    -h|--help)  sed -n '2,3p' "$0"; exit 0 ;;
    *)          device=$arg ;;
  esac
done

target=armv7-unknown-linux-musleabihf
app=hello
bin=target/$target/release/$app
dest=/home/root/xovi/exthome/appload/$app

if [ "$build" = 1 ]; then
  cargo build --release --target "$target"
fi
[ -f "$bin" ] || { echo "missing $bin" >&2; exit 1; }

ssh "root@$device" "mkdir -p $dest"
scp app/external.manifest.json app/icon.png "$bin" "root@$device:$dest/"
# scp lands the binary without the execute bit. Without it AppLoad logs
# "execve: Permission denied" and shows a blank window.
ssh "root@$device" "chmod +x $dest/$app && test -x $dest/$app"

echo "Deployed to $dest. Open AppLoad on the tablet and tap 'Hello'."
echo "Logs: ssh root@$device journalctl -fu xochitl"
