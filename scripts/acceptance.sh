#!/usr/bin/env bash
# One-shot acceptance run: capture → clipboard → history → OCR.
#
#   scripts/acceptance.sh
#
# Needs: the app installed in /Applications and screen-recording permission granted.
# It does NOT move the pointer or press keys: the capture is opened with a preset
# region and committed through the same event the Enter key produces.
#
# Scroll capture is checked separately (`--scroll-manual`): it needs a human to scroll.

set -uo pipefail

ROOT=$(cd "$(dirname "$0")/.." && pwd)
CLI="$ROOT/target/debug/pinshot"
APP="/Applications/PinShot.app/Contents/MacOS/pinshot-desktop"
OUT=${1:-/tmp/pinshot-acceptance}

pass=0
fail=0
check() {
  local label=$1 ok=$2 detail=${3:-}
  if [ "$ok" = "1" ]; then
    printf 'PASS  %-42s %s\n' "$label" "$detail"
    pass=$((pass + 1))
  else
    printf 'FAIL  %-42s %s\n' "$label" "$detail"
    fail=$((fail + 1))
  fi
}

mkdir -p "$OUT"
[ -x "$CLI" ] || { echo "build the CLI first: cargo build -p pinshot-cli" >&2; exit 1; }
[ -x "$APP" ] || { echo "install first: scripts/install.sh --build" >&2; exit 1; }

echo "== 1/4 permissions"
rm -f /tmp/pinshot-diag.log
open -a /Applications/PinShot.app --args --self-test permission
sleep 3
PERM=$(tail -1 /tmp/pinshot-diag.log 2>/dev/null)
case "$PERM" in
  *"screen=true"*) check "screen recording granted" 1 ;;
  *) check "screen recording granted" 0 "$PERM" ;;
esac
case "$PERM" in
  *"ax=true"*) echo "      (accessibility granted: auto-scroll can be tested)" ;;
  *) echo "      (accessibility NOT granted: 自动滚动会在 18:00 之后或授权后可用)" ;;
esac

echo "== 2/4 capture + clipboard"
BEFORE=$("$CLI" history --limit 1 --json 2>/dev/null | tr -d '\n' | head -c 200)
pkill -f "PinShot.app/Contents/MacOS" 2>/dev/null
sleep 1
PINSHOT_DEBUG_REGION="300,200,700,450" "$APP" --capture --auto-commit 3500 >"$OUT/app.log" 2>&1 &
sleep 9
CLIP=$(osascript -e 'clipboard info' 2>/dev/null | head -1)
case "$CLIP" in
  *PNG*) check "clipboard holds the capture" 1 ;;
  *) check "clipboard holds the capture" 0 "$CLIP" ;;
esac

echo "== 3/4 history"
LAST=$("$CLI" history --limit 1 --json 2>/dev/null)
case "$LAST" in
  *'"width":1400'*) check "history recorded 1400×900 (700×450 pt @2x)" 1 ;;
  *) check "history recorded the capture" 0 "$LAST" ;;
esac
ID=$(printf '%s' "$LAST" | python3 -c "import json,sys; print(json.load(sys.stdin)[0]['id'])" 2>/dev/null)
HASH=$(find "$HOME/Library/Application Support/ai.pinshot.desktop/history/blobs" -type f -newer "$OUT/app.log" 2>/dev/null | head -1)

echo "== 4/4 OCR"
if [ -n "$HASH" ]; then
  TEXT=$("$CLI" ocr "$HASH" 2>/dev/null | head -3)
  if [ -n "$TEXT" ]; then
    check "OCR returned text" 1 "$(printf '%s' "$TEXT" | head -1 | cut -c1-48)"
  else
    check "OCR returned text" 0 "empty (screen may have had no text)"
  fi
else
  check "OCR returned text" 0 "no blob found for the new capture"
fi

echo
echo "$pass passed, $fail failed"
echo "artifacts: $OUT"
[ "$fail" -eq 0 ]
