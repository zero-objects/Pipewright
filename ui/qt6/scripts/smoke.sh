#!/usr/bin/env bash
# Headless end-to-end smoke test of the Qt6 UI: launches the real app offscreen
# in DRYCLEANER_SMOKE mode, where Main.qml self-checks the FFI bridge + a render
# path and Qt.exit()s with a pass/fail code. Catches QML compile/binding errors,
# FFI linkage breaks, and basic render regressions without a display.
#
# Usage: ui/qt6/scripts/smoke.sh [path/to/build] [optional-pipeline.yml]
set -euo pipefail

BUILD_DIR="${1:-ui/qt6/build}"
FIXTURE="${2:-}"
APP="$BUILD_DIR/pipewright-ui.app/Contents/MacOS/pipewright-ui"
[ -x "$APP" ] || APP="$BUILD_DIR/pipewright-ui"   # non-macOS layout

if [ ! -x "$APP" ]; then
  echo "smoke: UI binary not found under $BUILD_DIR — build it first" >&2
  exit 2
fi

QT_QPA_PLATFORM=offscreen \
  DRYCLEANER_SMOKE=1 \
  DRYCLEANER_SMOKE_FILE="$FIXTURE" \
  "$APP"
