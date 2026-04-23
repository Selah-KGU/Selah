#!/usr/bin/env bash
# Download sherpa-onnx shared libraries for both macOS architectures
# and combine them into universal (fat) dylibs for the Tauri app bundle.
#
# Usage:
#   ./scripts/build-macos-sherpa-runtime.sh [--version v1.12.39] \
#       [--lib-dir /tmp/sherpa-lib] [--framework-dir src-tauri/macos-runtime]

set -euo pipefail

SHERPA_VERSION="v1.12.39"
LIB_STAGE_DIR=""
FRAMEWORK_STAGE_DIR=""

while [[ $# -gt 0 ]]; do
  case "$1" in
    --version)       SHERPA_VERSION="$2"; shift 2 ;;
    --lib-dir)       LIB_STAGE_DIR="$2"; shift 2 ;;
    --framework-dir) FRAMEWORK_STAGE_DIR="$2"; shift 2 ;;
    *) echo "Unknown option: $1" >&2; exit 1 ;;
  esac
done

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
REPO_ROOT="$(cd "$SCRIPT_DIR/.." && pwd)"
WORK_DIR="${TMPDIR:-/tmp}/sherpa-macos-shared"

# Defaults
: "${LIB_STAGE_DIR:=$WORK_DIR/universal-lib}"
: "${FRAMEWORK_STAGE_DIR:=$REPO_ROOT/src-tauri/macos-runtime}"

RELEASE_BASE="https://github.com/k2-fsa/sherpa-onnx/releases/download"
ARM64_ARCHIVE="sherpa-onnx-${SHERPA_VERSION}-osx-arm64-shared-lib.tar.bz2"
X64_ARCHIVE="sherpa-onnx-${SHERPA_VERSION}-osx-x64-shared-lib.tar.bz2"
ARM64_DIR="sherpa-onnx-${SHERPA_VERSION}-osx-arm64-shared-lib"
X64_DIR="sherpa-onnx-${SHERPA_VERSION}-osx-x64-shared-lib"

REQUIRED_DYLIBS=(
  "libsherpa-onnx-c-api.dylib"
  "libonnxruntime.1.24.4.dylib"
)
SYMLINK_DYLIBS=(
  "libonnxruntime.dylib:libonnxruntime.1.24.4.dylib"
)

echo "=== sherpa-onnx macOS universal shared library builder ==="
echo "  version:       $SHERPA_VERSION"
echo "  lib stage:     $LIB_STAGE_DIR"
echo "  framework dir: $FRAMEWORK_STAGE_DIR"
echo ""

rm -rf "$WORK_DIR"
mkdir -p "$WORK_DIR" "$LIB_STAGE_DIR" "$FRAMEWORK_STAGE_DIR"

# ── Download ────────────────────────────────────────────────────
echo "Downloading arm64 shared libs..."
curl -fSL "$RELEASE_BASE/$SHERPA_VERSION/$ARM64_ARCHIVE" | tar xj -C "$WORK_DIR"

echo "Downloading x86_64 shared libs..."
curl -fSL "$RELEASE_BASE/$SHERPA_VERSION/$X64_ARCHIVE" | tar xj -C "$WORK_DIR"

# ── Create universal (fat) dylibs ───────────────────────────────
echo "Creating universal dylibs..."
for dylib in "${REQUIRED_DYLIBS[@]}"; do
  arm64_path="$WORK_DIR/$ARM64_DIR/lib/$dylib"
  x64_path="$WORK_DIR/$X64_DIR/lib/$dylib"

  if [[ ! -f "$arm64_path" ]]; then
    echo "ERROR: Missing arm64 lib: $arm64_path" >&2; exit 1
  fi
  if [[ ! -f "$x64_path" ]]; then
    echo "ERROR: Missing x86_64 lib: $x64_path" >&2; exit 1
  fi

  lipo -create "$arm64_path" "$x64_path" -output "$LIB_STAGE_DIR/$dylib"
  echo "  ✓ $dylib ($(lipo -info "$LIB_STAGE_DIR/$dylib" 2>&1 | tail -1))"
done

# Create symlinks expected by the linker
for entry in "${SYMLINK_DYLIBS[@]}"; do
  link_name="${entry%%:*}"
  target="${entry##*:}"
  ln -sf "$target" "$LIB_STAGE_DIR/$link_name"
  echo "  ✓ $link_name → $target (symlink)"
done

# ── Stage for Tauri framework bundling ──────────────────────────
echo "Staging dylibs to $FRAMEWORK_STAGE_DIR..."
rm -f "$FRAMEWORK_STAGE_DIR"/*.dylib
for dylib in "${REQUIRED_DYLIBS[@]}"; do
  cp "$LIB_STAGE_DIR/$dylib" "$FRAMEWORK_STAGE_DIR/$dylib"
done

echo ""
echo "Done. Set the following environment variable for cargo:"
echo "  SHERPA_ONNX_LIB_DIR=$LIB_STAGE_DIR"
echo ""
echo "Framework dylibs staged at:"
for dylib in "${REQUIRED_DYLIBS[@]}"; do
  echo "  $FRAMEWORK_STAGE_DIR/$dylib"
done
