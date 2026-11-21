#!/usr/bin/env bash
# Build a distributable DMG from a *signed* PinShot.app.
#
#   scripts/make-dmg.sh                       # host arch
#   scripts/make-dmg.sh x86_64-apple-darwin   # Intel Mac
#
# Tauri's `--bundles dmg` re-bundles and re-signs the app on the way in, which throws
# away our stable identifier (`ai.pinshot.desktop`) and leaves the linker's random
# `pinshot_desktop-<hash>` in its place. A stable identifier is what keeps the
# Screen-Recording/Accessibility grants alive, so we sign first and build the DMG
# ourselves.

set -euo pipefail

ROOT=$(cd "$(dirname "$0")/.." && pwd)
VERSION=$(python3 -c 'import json,sys; print(json.load(open(sys.argv[1]))["version"])' \
  "$ROOT/src-tauri/tauri.conf.json")
TARGET="${1:-${PINSHOT_TARGET:-}}"

if [ -n "$TARGET" ]; then
  BUNDLE="$ROOT/src-tauri/target/$TARGET/release/bundle"
else
  BUNDLE="$ROOT/src-tauri/target/release/bundle"
fi
APP="$BUNDLE/macos/PinShot.app"

if [ ! -d "$APP" ]; then
  echo "== building the app bundle (${TARGET:-host})"
  if [ -n "$TARGET" ]; then
    (cd "$ROOT" && pnpm tauri build --target "$TARGET" --bundles app)
  else
    (cd "$ROOT" && pnpm tauri build --bundles app)
  fi
fi

# Name the file after what is actually inside it, not after the host: one Mac can
# build for another in a minute.
ARCH=$(lipo -archs "$APP/Contents/MacOS/pinshot-desktop")
ARCH=${ARCH// /-}
OUT="$BUNDLE/dmg/PinShot_${VERSION}_${ARCH}.dmg"

echo "== signing"
IDENTITY="${PINSHOT_SIGN_IDENTITY:-}"
if [ -z "$IDENTITY" ]; then
  IDENTITY=$(security find-identity -v -p codesigning 2>/dev/null | sed -n 's/.*"\(.*\)"/\1/p' | head -1)
fi
if [ -n "$IDENTITY" ]; then
  codesign --force --deep --sign "$IDENTITY" --identifier ai.pinshot.desktop "$APP"
else
  codesign --force --deep --sign - --identifier ai.pinshot.desktop "$APP"
fi
codesign --verify --deep --strict "$APP"

echo "== staging"
STAGE=$(mktemp -d)
trap 'rm -rf "$STAGE"' EXIT
cp -R "$APP" "$STAGE/PinShot.app"
ln -s /Applications "$STAGE/Applications"

echo "== creating the dmg"
mkdir -p "$(dirname "$OUT")"
rm -f "$OUT"
hdiutil create -volname "PinShot" -srcfolder "$STAGE" -ov -format UDZO -quiet "$OUT"

echo
echo "built: $OUT"
hdiutil imageinfo "$OUT" | grep -E "^Format|^Total Bytes" | head -2 || true
