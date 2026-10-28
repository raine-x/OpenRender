#!/usr/bin/env bash
# Cross-compile OpenRender for Android.
#
# For each ABI this produces:
#   android/jniLibs/<abi>/libopenrender.so         (cdylib)
#   android/jniLibs/static/<abi>/libopenrender.a   (staticlib)
#
# Requirements: rustup targets, cargo-ndk, Android NDK.
#   rustup target add aarch64-linux-android armv7-linux-androideabi x86_64-linux-android
#   cargo install cargo-ndk
#
# Usage:
#   ./scripts/build-android.sh [--debug] [--latex] [--jni]
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT"

PROFILE="release"
PROFILE_ARG="--release"
FEATURES=""
for arg in "$@"; do
  case "$arg" in
    --debug) PROFILE="debug"; PROFILE_ARG="" ;;
    --latex) FEATURES="${FEATURES:+$FEATURES,}latex" ;;
    --jni)   FEATURES="${FEATURES:+$FEATURES,}jni" ;;
    *) echo "unknown argument: $arg" >&2; exit 2 ;;
  esac
done

OUT="android/jniLibs"
ABIS=("arm64-v8a" "armeabi-v7a" "x86_64")
declare -A TRIPLES=(
  ["arm64-v8a"]="aarch64-linux-android"
  ["armeabi-v7a"]="armv7-linux-androideabi"
  ["x86_64"]="x86_64-linux-android"
)

TARGET_ARGS=()
for abi in "${ABIS[@]}"; do
  TARGET_ARGS+=(-t "$abi")
done

FEATURE_ARGS=()
if [ -n "$FEATURES" ]; then
  FEATURE_ARGS=(--features "$FEATURES")
fi

echo "==> cargo ndk (${ABIS[*]}), profile=$PROFILE features=${FEATURES:-none}"
# shellcheck disable=SC2086
cargo ndk "${TARGET_ARGS[@]}" -o "$OUT" build $PROFILE_ARG -p openrender-core "${FEATURE_ARGS[@]}"

for abi in "${ABIS[@]}"; do
  triple="${TRIPLES[$abi]}"
  src="target/$triple/$PROFILE/libopenrender.a"
  if [ -f "$src" ]; then
    mkdir -p "$OUT/static/$abi"
    cp "$src" "$OUT/static/$abi/libopenrender.a"
    echo "    static: $OUT/static/$abi/libopenrender.a"
  fi
done

if command -v cbindgen >/dev/null 2>&1; then
  echo "==> cbindgen include/engine.h"
  cbindgen --config cbindgen.toml --crate openrender-core --output include/engine.h
else
  echo "==> cbindgen not installed; keeping the checked-in header"
fi

echo "==> done"
find "$OUT" -name 'libopenrender.*' -type f
