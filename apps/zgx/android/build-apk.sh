#!/usr/bin/env bash
# Build Zend's Android debug APK (sh.zend.zgx): this crate (the whole app,
# zend-zgx, with the engine in it) for each ABI with cargo-ndk, packed with
# aapt2, aligned and signed with a debug key. No Gradle, no Java code
# (NativeActivity).
#
#   apps/zgx/android/build-apk.sh [--release] [--abi x86_64|arm64-v8a]...
#
# Needs: ANDROID_HOME (SDK with build-tools 36.1.0 and platforms;android-36),
# ANDROID_NDK_HOME, JAVA_HOME, cargo-ndk, the rustup targets.
# Output: $OUT (default ~/.cache/android-app/apk)/zend.apk
set -euo pipefail
here=$(cd "$(dirname "$0")" && pwd)
profile=debug
abis=()
while [ $# -gt 0 ]; do
  case "$1" in
    --release) profile=release ;;
    --abi) abis+=("$2"); shift ;;
    *) echo "unknown argument $1" >&2; exit 2 ;;
  esac
  shift
done
[ ${#abis[@]} -eq 0 ] && abis=(x86_64 arm64-v8a)
: "${ANDROID_HOME:?}" "${ANDROID_NDK_HOME:?}"
BT=$ANDROID_HOME/build-tools/36.1.0
JAR=$ANDROID_HOME/platforms/android-36/android.jar
OUT=${OUT:-$HOME/.cache/android-app/apk}
TARGET=${CARGO_TARGET_DIR:-$HOME/.cache/android-app/target-app}
JOBS=${JOBS:-5}
rm -rf "$OUT/stage" && mkdir -p "$OUT/stage"
cd "$here"
targs=()
for abi in "${abis[@]}"; do targs+=(-t "$abi"); done
flag=(); [ $profile = release ] && flag=(--release)
CARGO_TARGET_DIR=$TARGET cargo ndk "${targs[@]}" -P 29 --link-libcxx-shared -o "$OUT/stage/lib" \
  build -j "$JOBS" -p zend-android "${flag[@]}"
"$BT/aapt2" link -o "$OUT/unsigned.apk" --manifest AndroidManifest.xml -I "$JAR"
# Native libraries stored uncompressed (extractNativeLibs=false): loaded
# straight from the APK.
(cd "$OUT/stage" && zip -q -0 -r "$OUT/unsigned.apk" lib)
"$BT/zipalign" -f -P 16 4 "$OUT/unsigned.apk" "$OUT/aligned.apk"
KS=${KEYSTORE:-$HOME/.cache/android-poc/debug.keystore}
if [ ! -f "$KS" ]; then
  keytool -genkeypair -keystore "$KS" -storepass android -keypass android -alias poc \
    -keyalg RSA -keysize 2048 -validity 10000 -dname "CN=Zend Android debug" >/dev/null
fi
"$BT/apksigner" sign --ks "$KS" --ks-pass pass:android --key-pass pass:android --out "$OUT/zend.apk" "$OUT/aligned.apk"
"$BT/apksigner" verify "$OUT/zend.apk"
ls -la "$OUT/zend.apk"
