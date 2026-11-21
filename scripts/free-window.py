#!/usr/bin/env python3
"""Work-window gate.

The owner asked for work to happen only outside office hours:
  * Mon–Fri  12:00–14:00  and  18:00–24:00
  * Mon–Fri  00:00–09:00   (the night after "18:00 onwards")
  * Sat, Sun  all day

Usage:
  free-window.py --check     exit 0 inside a window, 1 outside (prints the status)
  free-window.py --wait      sleep until the next window opens, then exit 0
  free-window.py --wait --max-seconds 900   same, but give up after a bounded wait
"""

from __future__ import annotations

import argparse
import datetime as dt
import sys
import time

WORKDAY_START = dt.time(9, 0)
LUNCH_START = dt.time(12, 0)
LUNCH_END = dt.time(14, 0)
EVENING_START = dt.time(18, 0)


def in_window(moment: dt.datetime) -> bool:
    if moment.weekday() >= 5:  # Saturday / Sunday
        return True
    clock = moment.time()
    return (
        clock < WORKDAY_START
        or LUNCH_START <= clock < LUNCH_END
        or clock >= EVENING_START
    )


def next_window(moment: dt.datetime) -> dt.datetime:
    """First moment strictly after `moment` that falls inside a window."""
    candidate = moment.replace(second=0, microsecond=0) + dt.timedelta(minutes=1)
    for _ in range(60 * 24 * 10):  # ten days is plenty
        if in_window(candidate):
            return candidate
        candidate += dt.timedelta(minutes=1)
    raise RuntimeError("no work window found in the next ten days")


def describe(moment: dt.datetime) -> str:
    state = "OPEN" if in_window(moment) else "CLOSED"
    return f"{moment:%Y-%m-%d %H:%M:%S %a}  window={state}"


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--check", action="store_true")
    parser.add_argument("--wait", action="store_true")
    parser.add_argument("--max-seconds", type=int, default=0)
    args = parser.parse_args()

    now = dt.datetime.now()
    print(describe(now), flush=True)

    if args.check:
        return 0 if in_window(now) else 1

    if not args.wait:
        parser.error("pass --check or --wait")
        return 2

    if in_window(now):
        print("already inside a work window", flush=True)
        return 0

    target = next_window(now)
    print(f"waiting until {target:%Y-%m-%d %H:%M:%S}", flush=True)
    deadline = time.monotonic() + args.max_seconds if args.max_seconds else None

    while True:
        left = (target - dt.datetime.now()).total_seconds()
        if left <= 0:
            print(describe(dt.datetime.now()), flush=True)
            return 0
        if deadline is not None and time.monotonic() >= deadline:
            print("gave up waiting (max-seconds reached)", flush=True)
            return 3
        time.sleep(min(60.0, left))


if __name__ == "__main__":
    sys.exit(main())
