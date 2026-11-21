#!/usr/bin/env bash
# Install PinShot into /Applications and (re)sign it.
#
#   scripts/install.sh [--build]
#
# Why /Applications: macOS ties the Accessibility/Screen-Recording grants to the app
# bundle's identity. Rebuilding *in place* changes the binary hash and invalidates the
# grant; installing a release build once keeps it stable until the next release.

set -euo pipefail

ROOT=$(cd "$(dirname "$0")/.." && pwd)
APP="$ROOT/src-tauri/target/release/bundle/macos/PinShot.app"
TARGET="/Applications/PinShot.app"

if [ "${1:-}" = "--build" ] || [ ! -d "$APP" ]; then
  echo "== building the release bundle"
  (cd "$ROOT" && pnpm tauri build --bundles app)
fi

if [ ! -d "$APP" ]; then
  echo "no bundle at $APP" >&2
  exit 1
fi

echo "== signing"
# Prefer a real signing identity: TCC keys the grant to `identifier + certificate`, so
# rebuilds keep their Screen-Recording/Accessibility permissions. Ad-hoc signing keys it
# to the binary hash, which changes on every build and silently invalidates the grant.
IDENTITY="${PINSHOT_SIGN_IDENTITY:-}"
if [ -z "$IDENTITY" ]; then
  IDENTITY=$(security find-identity -v -p codesigning 2>/dev/null | sed -n 's/.*"\(.*\)"/\1/p' | head -1)
fi
if [ -n "$IDENTITY" ]; then
  echo "   identity: $IDENTITY"
  codesign --force --deep --sign "$IDENTITY" --identifier ai.pinshot.desktop "$APP"
else
  echo "   no identity found, falling back to ad-hoc（每次重编译都会掉权限）" >&2
  codesign --force --deep --sign - --identifier ai.pinshot.desktop "$APP"
fi
codesign --verify --deep --strict --verbose=2 "$APP"

echo "== installing to $TARGET"
rm -rf "$TARGET"
cp -R "$APP" "$TARGET"

# Register with LaunchServices so Spotlight/`open` find the new copy.
/System/Library/Frameworks/CoreServices.framework/Frameworks/LaunchServices.framework/Support/lsregister \
  -f "$TARGET" >/dev/null 2>&1 || true

echo
echo "installed: $TARGET"
echo
echo "首次运行后请授权两项（只需一次）："
echo "  系统设置 → 隐私与安全性 → 屏幕录制   → 勾选 PinShot"
echo "  系统设置 → 隐私与安全性 → 辅助功能   → 勾选 PinShot（自动滚动需要）"
echo
echo "然后运行： open -a PinShot"
