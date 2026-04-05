#!/bin/zsh
set -e
cd "$(dirname "$0")"

APP_NAME="Selah"
BUNDLE_PATH="src-tauri/target/release/bundle/macos/${APP_NAME}.app"
DEV_BUNDLE="src-tauri/target/debug/bundle"

usage() {
  echo "Usage: ./run.sh [command]"
  echo ""
  echo "Commands:"
  echo "  dev       Start development server (default)"
  echo "  build     Production build and launch"
  echo "  clean     Clean all build caches"
  echo "  rebuild   Clean + build"
  echo "  kill      Kill running Selah processes"
  echo "  open      Open last built app"
}

do_kill() {
  echo "🔪 Killing running processes..."
  pkill -f "selah-app" 2>/dev/null || true
  pkill -f "tauri dev" 2>/dev/null || true
  pkill -f "cargo-tauri" 2>/dev/null || true
  sleep 0.5
}

do_clean() {
  echo "🧹 Cleaning caches..."
  rm -rf dist node_modules/.vite node_modules/.cache
  rm -rf "$DEV_BUNDLE" src-tauri/target/release/bundle
  rm -rf src-tauri/gen/schemas
  find . -name ".DS_Store" -delete 2>/dev/null || true
  echo "✅ Clean complete"
}

do_dev() {
  do_kill
  echo "🚀 Starting dev server..."
  npm run tauri dev
}

do_build() {
  do_kill
  rm -rf dist
  echo "🔨 Building ${APP_NAME}..."
  npx tauri build 2>&1 | tail -30
  echo ""
  if [[ -d "$BUNDLE_PATH" ]]; then
    echo "✅ Build complete: ${BUNDLE_PATH}"
    echo "🚀 Launching ${APP_NAME}.app..."
    open "$BUNDLE_PATH"
  else
    echo "❌ Build failed: ${BUNDLE_PATH} not found"
    exit 1
  fi
}

do_open() {
  if [[ -d "$BUNDLE_PATH" ]]; then
    open "$BUNDLE_PATH"
  else
    echo "❌ No build found. Run './run.sh build' first."
    exit 1
  fi
}

case "${1:-dev}" in
  dev)     do_dev ;;
  build)   do_build ;;
  clean)   do_clean ;;
  rebuild) do_clean && do_build ;;
  kill)    do_kill; echo "✅ Done" ;;
  open)    do_open ;;
  *)       usage ;;
esac
