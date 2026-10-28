#!/usr/bin/env bash
# Runs the host build of `hello` against the fake qtfb server; dumps frames to target/frames/.
set -euo pipefail
cd "$(dirname "$0")/.."
cargo build --quiet
OUT=target/frames; SOCK=target/qtfb.sock
mkdir -p "$OUT"; rm -f "$OUT"/*
./target/debug/fake_qtfb_server "$SOCK" "$OUT" & SERVER=$!
sleep 0.2
set +e; QTFB_SOCKET="$SOCK" QTFB_KEY=1 HELLO_TIMEOUT_S=5 ./target/debug/hello; RC=$?; set -e
wait $SERVER || true
echo "hello exited with $RC (expected 0)"; [ "$RC" -eq 0 ]
for f in "$OUT"/*.ppm; do
  python3 -c "from PIL import Image; import sys; Image.open(sys.argv[1]).save(sys.argv[2])" "$f" "${f%.ppm}.png" 2>/dev/null && rm "$f" || true
done
ls "$OUT"
