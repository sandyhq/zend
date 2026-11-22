#!/usr/bin/env bash
# Build the e2e keyboard (sh.zend.e2e.ime): javac, d8 and aapt2 from the SDK,
# no Gradle. Output: $OUT/ime.apk (default ~/.cache/android-app/e2e).
set -euo pipefail
here=$(cd "$(dirname "$0")" && pwd)
: "${ANDROID_HOME:=$HOME/Android/Sdk}"
JAVA_HOME=${JAVA_HOME:-$HOME/.local/opt/jdk-17}
export PATH=$JAVA_HOME/bin:$PATH
BT=$ANDROID_HOME/build-tools/36.1.0
JAR=$ANDROID_HOME/platforms/android-36/android.jar
OUT=${OUT:-$HOME/.cache/android-app/e2e}
work=$OUT/ime-build
rm -rf "$work" && mkdir -p "$work/classes" "$work/dex"
"$JAVA_HOME/bin/javac" -nowarn -source 11 -target 11 -classpath "$JAR" -d "$work/classes" \
  $(find "$here/src" -name '*.java') 2>&1 | grep -v "warning: \[options\]" || true
"$BT/d8" --lib "$JAR" --min-api 29 --output "$work/dex" $(find "$work/classes" -name '*.class')
"$BT/aapt2" compile --dir "$here/res" -o "$work/res.zip"
"$BT/aapt2" link -o "$work/unsigned.apk" --manifest "$here/AndroidManifest.xml" -I "$JAR" "$work/res.zip"
(cd "$work/dex" && zip -q "$work/unsigned.apk" classes.dex)
"$BT/zipalign" -f -P 16 4 "$work/unsigned.apk" "$work/aligned.apk"
KS=${KEYSTORE:-$HOME/.cache/android-poc/debug.keystore}
"$BT/apksigner" sign --ks "$KS" --ks-pass pass:android --key-pass pass:android --out "$OUT/ime.apk" "$work/aligned.apk"
echo "$OUT/ime.apk"
