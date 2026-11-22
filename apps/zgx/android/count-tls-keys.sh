#!/usr/bin/env bash
# How many pthread keys the app's Rust code can take on Android, read from
# the built library: on Android std has no ELF TLS, so every std
# `thread_local!` static linked in takes a key of its own on first use, and
# bionic aborts past 128 for the whole process ("out of TLS keys"), the
# platform's own libraries included. zgx's thread-locals share one key
# (zgx-tls). Fails above the budget, or when it finds none (std renamed its
# storage symbol: the count can't be read, so it isn't trusted).
#
#   apps/zgx/android/count-tls-keys.sh <libzend_android.so> [budget, default 64]
#
# Needs the unstripped library (the target dir's, not the APK's).
set -euo pipefail
lib=${1:?library}
budget=${2:-64}
nm=nm
for candidate in "${ANDROID_NDK_HOME:-}"/toolchains/llvm/prebuilt/*/bin/llvm-nm; do
  [ -x "$candidate" ] && nm=$candidate && break
done
symbols=$("$nm" --demangle --defined-only "$lib" | cut -d' ' -f3-)
# One storage static per std thread_local! (std's thread_local_inner!);
# " (.llvm.<hash>)": one ThinLTO made local to its unit.
llvm='( \(\.llvm\.[0-9]+\))?$'
std_pattern="::__RUST_STD_INTERNAL_VAL$llvm"
std_keys=$(grep -Ec "$std_pattern" <<<"$symbols" || true)
# zgx-tls's table: one key, whatever the number of its statics.
shared=$(grep -Ec "^zgx_tls::slots::KEY$llvm" <<<"$symbols" || true)
total=$((std_keys + shared))
echo "OS TLS keys from Rust in $(basename "$lib"): $total (std thread_local! $std_keys, zgx-tls table $shared); budget $budget of bionic's 128"
grep -E "$std_pattern" <<<"$symbols" | sed -E 's/^<?([A-Za-z0-9_]+).*/\1/' |
  sort | uniq -c | sort -rn | head -12 | sed 's/^/  /'
if [ "$std_keys" -eq 0 ]; then
  echo "no std thread_local! storage symbol found: stripped library, or std renamed it; can't count" >&2
  exit 1
fi
if [ "$total" -gt "$budget" ]; then
  echo "over budget: $total > $budget (use zgx::tls::thread_local! / zgx_tls::thread_local!)" >&2
  exit 1
fi
