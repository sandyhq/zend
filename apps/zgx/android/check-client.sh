#!/usr/bin/env bash
# The phone is a client of a computer's engine (plans/client-server, step 1):
# its library links no engine and no harness. Fails, naming them, when the
# Android build's normal dependencies hold zend-engine or zend-harness.
# build-apk.sh runs it before building; run it alone the same way:
#
#   apps/zgx/android/check-client.sh [target triple, default aarch64-linux-android]
set -euo pipefail
here=$(cd "$(dirname "$0")" && pwd)
target=${1:-aarch64-linux-android}
cd "$here"
tree=$(cargo tree -p zend-android --target "$target" -e normal --prefix none --format '{p}')
found=$(printf '%s\n' "$tree" | grep -E '^zend-(engine|harness) ' | sort -u || true)
if [ -n "$found" ]; then
  echo "check-client: the Android build ($target) links what only a computer runs:" >&2
  printf '%s\n' "$found" | sed 's/^/  /' >&2
  echo "check-client: why: cargo tree -p zend-android --target $target -e normal -i zend-engine" >&2
  exit 1
fi
echo "check-client: ok ($target: no zend-engine, no zend-harness)"
