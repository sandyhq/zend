#!/usr/bin/env bash
# Build this checkout's engine and window in the build container (run.sh, stable)
# and stage them as an installable Zend in target/zend-next: the same layout as
# ~/.local/opt/zend (docs/reference/build-and-deploy.md). Runs on the build server.
#
#   infra/dev/stage.sh          # then copy target/zend-next to the machine that runs Zend
set -euo pipefail
ROOT="$(cd "$(dirname "$0")/../.." && pwd)"
cd "$ROOT"
NIGHTLY=0 "$ROOT/infra/dev/run.sh" bash -c '
  set -e
  export CARGO_PROFILE_RELEASE_LTO=off LUAU_CXXFLAGS=-DNDEBUG
  cargo build --release --locked -p zend --bin zend-engine --target-dir /opt/target/engine
  (cd apps/zgx && cargo build --release --locked --target-dir /opt/target/zgx)
  S=/src/target/zend-next; rm -rf "$S"; mkdir -p "$S"
  cp /opt/target/zgx/release/zend-zgx /opt/target/engine/release/zend-engine "$S/"
  cp -r apps/zgx/ui "$S/ui"; rm -rf "$S/ui/.zgx" "$S/ui/tests"
  cp -r apps/zgx/plugins-builtin "$S/plugins-builtin"; rm -rf "$S"/plugins-builtin/*/.zgx
  for d in "$S"/plugins-builtin/*/; do cp apps/zgx/plugins-sdk/zend.luau "$d/zend.luau"; done
  echo "$(git rev-parse --short HEAD)$(git diff --quiet HEAD || echo -dirty)" > "$S/BUILT_FROM"
'
echo "BUILD_OK $ROOT/target/zend-next ($(cat "$ROOT/target/zend-next/BUILT_FROM"))"
