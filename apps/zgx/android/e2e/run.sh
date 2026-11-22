#!/usr/bin/env bash
# The Android app's e2e suite in one command: boots the emulator (AVD
# zgxpoc, headless) unless one is up, builds and installs the test keyboard
# (ime/) and makes it the keyboard, installs the APK, then runs suite.mjs
# (a stand-in engine and mock model of its own, the app linked to it). Exits
# non-zero when a case fails; stops what it started.
#
#   apps/zgx/android/e2e/run.sh [--apk path/zend.apk] [--only name,name] [--keep]
#
# The APK: an x86_64 build the shell can run-as into (devtools), e.g.
#   OUT=~/.cache/android-app/apk-x86 android/build-apk.sh --release --debuggable --abi x86_64
# Without --apk, the app installed on the emulator. Every step runs under
# nice/ionice. Needs ANDROID_HOME (default ~/Android/Sdk), node, JAVA_HOME
# for the keyboard (default ~/.local/opt/jdk-17). Output (screenshots of
# failures, logs): $E2E_OUT (default ~/.cache/android-app/e2e/out).
set -euo pipefail
here=$(cd "$(dirname "$0")" && pwd)
export ANDROID_HOME=${ANDROID_HOME:-$HOME/Android/Sdk}
export PATH=$ANDROID_HOME/platform-tools:$ANDROID_HOME/emulator:$PATH
export ANDROID_SERIAL=${ANDROID_SERIAL:-emulator-5554}
AVD=${AVD:-zgxpoc}
work=${E2E_WORK:-$HOME/.cache/android-app/e2e}
mkdir -p "$work"
quiet() { nice -n 19 ionice -c3 "$@"; }

booted=0
if ! adb -s "$ANDROID_SERIAL" get-state >/dev/null 2>&1; then
  echo "e2e: booting $AVD"
  quiet emulator -avd "$AVD" -no-window -no-audio -no-boot-anim -no-snapshot-save \
    -gpu swiftshader_indirect -cores 4 >"$work/emulator.log" 2>&1 &
  emu=$!
  booted=1
  stop_emulator() { adb -s "$ANDROID_SERIAL" emu kill >/dev/null 2>&1 || kill "$emu" 2>/dev/null || true; }
  trap stop_emulator EXIT
  adb -s "$ANDROID_SERIAL" wait-for-device
  for _ in $(seq 1 180); do
    [ "$(adb -s "$ANDROID_SERIAL" shell getprop sys.boot_completed 2>/dev/null | tr -d '\r')" = 1 ] && break
    sleep 2
  done
fi
# Animations off: what the suite waits for settles at once.
for k in window_animation_scale transition_animation_scale animator_duration_scale; do
  adb -s "$ANDROID_SERIAL" shell settings put global $k 0
done

# The test keyboard: a real InputMethodService typing through the
# InputConnection, as Gboard does.
OUT=$work quiet "$here/ime/build.sh" >/dev/null
adb -s "$ANDROID_SERIAL" install -r "$work/ime.apk" >/dev/null
for _ in $(seq 1 10); do
  adb -s "$ANDROID_SERIAL" shell ime enable sh.zend.e2e.ime/.TestIme 2>&1 | grep -q "now enabled\|already enabled" && break
  sleep 1
done
adb -s "$ANDROID_SERIAL" shell ime set sh.zend.e2e.ime/.TestIme >/dev/null

cd "$here"
status=0
quiet node suite.mjs "$@" || status=$?
[ $booted = 1 ] && echo "e2e: stopping the emulator it booted"
exit $status
