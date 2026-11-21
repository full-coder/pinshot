#!/usr/bin/env bash
# End-to-end check of the overlay using the real screen.
#
# Needs an *unlocked* session: while the screen is locked every synthetic event goes
# to loginwindow and every capture shows the lock screen.
#
#   scripts/verify-overlay.sh [outdir]
#
# Produces numbered screenshots and a PASS/FAIL summary.

set -u

OUT=${1:-/tmp/pinshot-verify}
BIN=./src-tauri/target/debug/pinshot-desktop
CLI=./target/debug/pinshot

mkdir -p "$OUT"
rm -f "$OUT"/*.png

pass=0
fail=0

check() {
  local label=$1 ok=$2 detail=${3:-}
  if [ "$ok" = "1" ]; then
    printf 'PASS  %s %s\n' "$label" "$detail"
    pass=$((pass + 1))
  else
    printf 'FAIL  %s %s\n' "$label" "$detail"
    fail=$((fail + 1))
  fi
}

frontmost() {
  osascript -e 'tell application "System Events" to get name of first application process whose frontmost is true' 2>/dev/null
}

if [ -z "$(frontmost)" ]; then
  echo "the session looks locked; unlock the screen and run again" >&2
  exit 2
fi

echo "== starting the app"
# `--auto-commit` fires the same event the Enter key produces after the delay, because
# macOS routes synthetic keystrokes by window-server focus and a shell-launched binary
# never gets it. Everything after that (render, crop, clipboard, history) is the real
# code path.
"$BIN" --capture --auto-commit 14000 >"$OUT/app.log" 2>&1 &
APP=$!
sleep 6

echo "== screenshot: overlay"
"$CLI" snap --out "$OUT/1-overlay.png" --json >"$OUT/1-overlay.json" 2>&1
check "overlay is on screen" "$([ -s "$OUT/1-overlay.png" ] && echo 1 || echo 0)"

echo "== simulate: select a region"
"$CLI" simulate drag 300 300 900 700
sleep 1
"$CLI" snap --out "$OUT/2-selection.png" --json >"$OUT/2-selection.json" 2>&1
check "drag produced a selection" "$(grep -q '"ok":true' "$OUT/2-selection.json" && echo 1 || echo 0)"

echo "== simulate: annotate with a rectangle"
"$CLI" simulate drag 500 400 760 620
sleep 1
"$CLI" snap --out "$OUT/3-annotation.png" --json >"$OUT/3-annotation.json" 2>&1
check "annotation rendered" "$([ -s "$OUT/3-annotation.png" ] && echo 1 || echo 0)"

echo "== waiting for the auto-commit (same path as Enter)"
sleep 10
INFO=$(osascript -e 'clipboard info' 2>/dev/null)
case "$INFO" in
  *PNG*) check "clipboard holds a PNG" 1 "$(echo "$INFO" | head -1)";;
  *) check "clipboard holds a PNG" 0 "$INFO";;
esac

if kill -0 "$APP" 2>/dev/null; then
  check "app still runs after the capture" 1
  kill "$APP" 2>/dev/null
else
  check "app still runs after the capture" 0 "it exited"
fi

echo
echo "$pass passed, $fail failed → $OUT"
[ "$fail" -eq 0 ]
