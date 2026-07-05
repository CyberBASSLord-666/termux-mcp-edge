#!/usr/bin/env bash
# Cross-compile Termux MCP Server for aarch64-linux-android.

set -euo pipefail
IFS=$'\n\t'

TARGET="aarch64-linux-android"
ANDROID_API_LEVEL="${ANDROID_API_LEVEL:-24}"
OUTPUT_DIR="target/${TARGET}/release"

log() {
  printf '[cross_compile] %s\n' "$*"
}

fail() {
  printf '[cross_compile] ERROR: %s\n' "$*" >&2
  exit 1
}

require_command() {
  command -v "$1" >/dev/null 2>&1 || fail "Required command not found: $1"
}

require_executable() {
  local path="$1"
  [[ -x "$path" ]] || fail "Required executable not found or not executable: $path"
}

cleanup() {
  # Reserved for future temporary build artifacts. Keeping this trap in place
  # ensures additions to this script inherit a single cleanup path.
  :
}
trap cleanup EXIT INT TERM

require_command cargo
require_command rustup

[[ -n "${ANDROID_NDK_HOME:-}" ]] || fail "Please set ANDROID_NDK_HOME to your Android NDK path"
[[ -d "$ANDROID_NDK_HOME" ]] || fail "ANDROID_NDK_HOME does not exist: $ANDROID_NDK_HOME"

LLVM_BIN="${ANDROID_NDK_HOME}/toolchains/llvm/prebuilt/linux-x86_64/bin"
CC_PATH="${LLVM_BIN}/aarch64-linux-android${ANDROID_API_LEVEL}-clang"
AR_PATH="${LLVM_BIN}/llvm-ar"

require_executable "$CC_PATH"
require_executable "$AR_PATH"

log "Cross-compiling Termux MCP Server for ${TARGET} using Android API ${ANDROID_API_LEVEL}"

if rustup target list --installed | grep -Fxq "$TARGET"; then
  log "Rust target already installed: ${TARGET}"
else
  rustup target add "$TARGET"
fi

export CC_aarch64_linux_android="$CC_PATH"
export AR_aarch64_linux_android="$AR_PATH"
export CARGO_TARGET_AARCH64_LINUX_ANDROID_LINKER="$CC_PATH"

cargo build --release --target "$TARGET"

log "Binary ready at: ${OUTPUT_DIR}/termux-mcp-server"
log "Transfer this binary to your device and place it in a safe location, such as ~/bin/."
