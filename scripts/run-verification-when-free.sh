#!/usr/bin/env bash
# Waits for the owner's work window (see scripts/free-window.py) and the screen to be
# unlocked, then runs the overlay end-to-end check unattended.
#
#   nohup scripts/run-verification-when-free.sh >/dev/null 2>&1 &
#
# Nothing here moves the cursor or types: the checks dispatch DOM events inside the
# overlay webview and read the captured pixels.
set -uo pipefail

here="$(cd "$(dirname "$0")/.." && pwd)"
log=/tmp/pinshot-verify.log
py=python3

stamp() { echo "=== $(date '+%Y-%m-%d %H:%M:%S %a')  $*" | tee -a "$log"; }

skip_wait=0
for arg in "$@"; do
  [ "$arg" = "--now" ] && skip_wait=1
done

if [ "$skip_wait" = "1" ]; then
  stamp "running now (--now)"
else
  stamp "waiting for a work window"
  if ! "$py" "$here/scripts/free-window.py" --wait >>"$log" 2>&1; then
    stamp "free-window.py failed; giving up"
    exit 1
  fi
fi

stamp "window open; waiting for the screen to be unlocked"
for _ in $(seq 1 240); do
  state=$("$py" - <<'PY'
import subprocess
out = subprocess.run(["ioreg", "-n", "Root", "-d", "1", "-a"], capture_output=True, text=True).stdout
print("locked" if "CGSSessionScreenIsLocked" in out else "unlocked")
PY
)
  [ "$state" = "unlocked" ] && break
  sleep 15
done

if [ "$state" != "unlocked" ]; then
  stamp "screen stayed locked for an hour; giving up"
  exit 1
fi
stamp "screen unlocked"

# Keep the display on for the duration; a sleeping display makes every capture black.
caffeinate -d -u -t 600 &
caffeinate_pid=$!
trap 'kill "$caffeinate_pid" 2>/dev/null' EXIT

capture_ok() {
  screencapture -x /tmp/pinshot-preflight.png 2>/dev/null || return 1
  "$py" - <<'PY2'
import sys
from PIL import Image
image = Image.open("/tmp/pinshot-preflight.png").convert("L")
low, high = image.getextrema()
# A real desktop has range; a locked/blank capture is a single value.
sys.exit(0 if high - low > 24 else 1)
PY2
}
for attempt in $(seq 1 20); do
  if capture_ok; then
    stamp "preflight: screen capture is live"
    break
  fi
  stamp "preflight: capture looks frozen/blank; waking the display and retrying"
  caffeinate -u -t 2
  sleep 15
  [ "$attempt" = 20 ] && { stamp "preflight failed 20 times; giving up"; exit 1; }
done

stamp "running scripts/verify-overlay.sh"
bash "$here/scripts/verify-overlay.sh" >>"$log" 2>&1
stamp "verify-overlay.sh exit=$?"
stamp "done"
