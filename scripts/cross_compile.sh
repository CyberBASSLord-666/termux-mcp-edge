#!/usr/bin/env bash
# Cross-compile Termux MCP Server for aarch64-linux-android
set -euo pipefail

TARGET="aarch64-linux-android"
OUTPUT_DIR="target/${TARGET}/release"

echo "=== Cross-compiling Termux MCP Server for ${TARGET} ==="

rustup target add "${TARGET}"

# Ensure Android NDK is available (user must set ANDROID_NDK_HOME)
if [ -z "${ANDROID_NDK_HOME:-}" ]; then
    echo "ERROR: Please set ANDROID_NDK_HOME to your Android NDK path"
    exit 1
fi

export CC_aarch64_linux_android="${ANDROID_NDK_HOME}/toolchains/llvm/prebuilt/linux-x86_64/bin/aarch64-linux-android24-clang"
export AR_aarch64_linux_android="${ANDROID_NDK_HOME}/toolchains/llvm/prebuilt/linux-x86_64/bin/llvm-ar"
export CARGO_TARGET_AARCH64_LINUX_ANDROID_LINKER="${CC_aarch64_linux_android}"

cargo build --release --target "${TARGET}"

echo "Binary ready at: ${OUTPUT_DIR}/termux-mcp-server"
echo "Transfer this binary to your device and place it in a safe location (e.g. ~/bin/)"
