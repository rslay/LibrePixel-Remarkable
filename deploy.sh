#!/usr/bin/env bash
# Build an app for the reMarkable 2 and copy it to the tablet. Git Bash on Windows, Linux or macOS.
# Usage:  ./deploy.sh [--app librepixel|hello] [tablet address] [--no-build] [--no-native-ink]
#         defaults: --app librepixel, address 10.11.99.1 (USB)
# The librepixel-ink xovi extension (native ink) is built with zig and copied too, unless
# --no-native-ink is given, and if it is not running afterwards the script restarts xochitl
# (which closes whatever is open on the tablet) and waits for it to report. If any of that
# fails the script warns and the app is still deployed.
set -euo pipefail
cd "$(dirname "$0")"
export MSYS_NO_PATHCONV=1   # Git Bash: leave the tablet-side paths alone

app=librepixel
device=10.11.99.1
build=1
ink=1
while [ $# -gt 0 ]; do
  case "$1" in
    --app)           app=$2; shift ;;
    --app=*)         app=${1#--app=} ;;
    --no-build)      build=0 ;;
    --no-native-ink) ink=0 ;;
    --ink)           ink=1 ;;   # the old opt-in spelling, now the default
    -h|--help)       sed -n '2,7p' "$0"; exit 0 ;;
    *)               device=$1 ;;
  esac
  shift
done
[ -d "app/$app" ] || { echo "no app/$app directory (apps: $(ls app | tr '\n' ' '))" >&2; exit 1; }

target=armv7-unknown-linux-musleabihf
bin=target/$target/release/$app
dest=/home/root/xovi/exthome/appload/$app

if [ "$build" = 1 ]; then
  cargo build --release --target "$target" --bin "$app"
fi
[ -f "$bin" ] || { echo "missing $bin" >&2; exit 1; }

ssh "root@$device" "mkdir -p $dest"
scp "app/$app/external.manifest.json" "app/$app/icon.png" "$bin" "root@$device:$dest/"
# scp lands the binary without the execute bit. Without it AppLoad logs
# "execve: Permission denied" and shows a blank window.
ssh "root@$device" "chmod +x $dest/$app && test -x $dest/$app"

echo "Deployed $app to $dest. Open AppLoad on the tablet and tap it."

# Native ink: build (zig) and copy the extension, then make sure a xochitl that has it loaded
# is running (restart it if not). Every failure is a warning, not an exit.
ink_status() {
  # The first line of the extension's status file if it was refreshed in the last 5 s, else "".
  ssh "root@$device" 'f=/tmp/librepixel-ink.status; now=$(date +%s); m=$(stat -c %Y "$f" 2>/dev/null || echo 0); [ $((now - m)) -le 5 ] && head -n 1 "$f" 2>/dev/null; true' 2>/dev/null || true
}
install_ink() {
  local so=extension/librepixel-ink.so old new changed=1 st
  if [ "$build" = 1 ] || [ ! -f "$so" ]; then
    if command -v zig >/dev/null 2>&1; then
      bash tools/build_ink.sh || { echo "WARN: building $so failed"; return 1; }
    elif [ -f "$so" ]; then
      echo "WARN: zig not found; using the existing $so as it is"
    else
      echo "WARN: zig not found and no prebuilt $so (Windows: winget install zig.zig)"
      return 1
    fi
  fi
  old=$(ssh "root@$device" 'md5sum /home/root/xovi/extensions.d/librepixel-ink.so 2>/dev/null | cut -d" " -f1' 2>/dev/null || true)
  new=$(md5sum "$so" | cut -d' ' -f1)
  [ "$old" = "$new" ] && changed=0
  if [ "$changed" = 1 ]; then
    # Copy to a temp name and rename, so xovi never loads a half-written file.
    scp "$so" "root@$device:/home/root/xovi/extensions.d/.librepixel-ink.new" || { echo "WARN: copying $so to the tablet failed"; return 1; }
    ssh "root@$device" "mv -f /home/root/xovi/extensions.d/.librepixel-ink.new /home/root/xovi/extensions.d/librepixel-ink.so" \
      || { echo "WARN: could not move the extension into place on the tablet"; return 1; }
    echo "Copied the librepixel-ink extension."
  else
    echo "The tablet already has this build of the librepixel-ink extension."
  fi
  st=$(ink_status)
  if [ "$changed" = 0 ]; then
    case "$st" in
      ready*) echo "Native ink is running: $st"; return 0 ;;
      off*)   echo "WARN: the extension is loaded but staying off: $st (restarting xochitl cannot change that)"; return 1 ;;
    esac
  fi
  # Not running (first install, new build, or xochitl started without it): restart xochitl.
  if ssh "root@$device" 'pgrep -x librepixel >/dev/null' 2>/dev/null; then
    echo "LibrePixel is open on the tablet; restarting xochitl closes it."
  fi
  echo "Restarting xochitl so the extension loads (the tablet's screen reloads)..."
  local t0
  t0=$(ssh "root@$device" 'date +%s' 2>/dev/null || echo 0)
  ssh "root@$device" 'systemctl restart xochitl' || { echo "WARN: systemctl restart xochitl failed"; return 1; }
  # xochitl takes a while to come back. Watch its journal from the restart on: the extension's
  # first line settles it (ready, or why it stays off); the status file is the fallback.
  printf 'Waiting for xochitl and the extension'
  st=$(ssh "root@$device" "t0=$t0"'; f=/tmp/librepixel-ink.status; i=0
    while [ $i -lt 90 ]; do
      line=$(journalctl -u xochitl --since "@$t0" --no-pager 2>/dev/null | grep -o "\[librepixel-ink\] .*" | tail -1)
      case "$line" in
        *"ready on"*) echo "$line" | sed "s/.*ready on \([^:]*\).*/ready \1/"; exit 0 ;;
        *"staying off"*|*"not found"*|*"odd pen"*|*"missing"*) echo "off ${line#*] }"; exit 0 ;;
      esac
      now=$(date +%s); m=$(stat -c %Y "$f" 2>/dev/null || echo 0); l=$(head -n 1 "$f" 2>/dev/null)
      if [ $((now - m)) -le 5 ] && [ -n "$l" ]; then echo "$l"; exit 0; fi
      [ $((i % 5)) -eq 4 ] && printf . >&2
      sleep 1; i=$((i + 1))
    done; exit 1') \
    || { echo; echo "WARN: xochitl restarted but the extension did not report within 90 s; look at: ssh root@$device journalctl -b -u xochitl | grep librepixel-ink"; return 1; }
  echo
  case "$st" in
    ready*) echo "Native ink is running: $st" ;;
    *)      echo "WARN: the extension is staying off: $st"; return 1 ;;
  esac
}
if [ "$ink" = 1 ] && [ "$app" = librepixel ]; then
  install_ink || echo "WARN: native ink is not active; the editor offers INK:LIVE and INK:BATCH only (--no-native-ink silences this)"
fi
echo "Logs: ssh root@$device journalctl -fu xochitl"
