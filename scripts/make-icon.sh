#!/bin/bash
# Regenerates the app icon: packaging/icon/InnerWord.iconset and packaging/InnerWord.icns.
# Needs the Xcode command line tools (swift, iconutil). The results are checked in,
# so this only has to run when scripts/make-icon.swift changes.
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
ICONSET="$ROOT/packaging/icon/InnerWord.iconset"
ICNS="$ROOT/packaging/InnerWord.icns"

rm -rf "$ICONSET"
mkdir -p "$ICONSET"
swift "$ROOT/scripts/make-icon.swift" "$ICONSET"
iconutil --convert icns --output "$ICNS" "$ICONSET"
echo "make-icon: wrote $ICNS ($(stat -f %z "$ICNS") bytes)"
