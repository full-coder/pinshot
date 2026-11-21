#!/usr/bin/env python3
"""Wait until the GUI can actually be verified.

Two conditions:
  1. the screen is unlocked — otherwise every synthetic event goes to loginwindow;
  2. we are inside an allowed work window (see free-window.py).

Exit 0 when both hold, so the caller can immediately run scripts/verify-overlay.sh.
"""

from __future__ import annotations

import datetime as dt
import subprocess
import sys
import time


def in_window(moment: dt.datetime) -> bool:
    if moment.weekday() >= 5:
        return True
    clock = moment.time()
    return clock < dt.time(9, 0) or dt.time(12, 0) <= clock < dt.time(14, 0) or clock >= dt.time(18, 0)


def unlocked() -> bool:
    try:
        result = subprocess.run(
            [
                "osascript",
                "-e",
                'tell application "System Events" to get name of first application process whose frontmost is true',
            ],
            capture_output=True,
            text=True,
            timeout=10,
        )
    except (subprocess.TimeoutExpired, OSError):
        return False
    return result.returncode == 0 and bool(result.stdout.strip())


def main() -> int:
    deadline = time.monotonic() + int(sys.argv[1]) if len(sys.argv) > 1 else None
    while True:
        now = dt.datetime.now()
        if in_window(now) and unlocked():
            print(f"{now:%Y-%m-%d %H:%M:%S} ready: unlocked and inside a work window", flush=True)
            return 0
        if deadline and time.monotonic() > deadline:
            print("gave up waiting", flush=True)
            return 3
        time.sleep(30)


if __name__ == "__main__":
    sys.exit(main())
