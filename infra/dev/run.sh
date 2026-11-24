#!/usr/bin/env bash
# Run a command in this repository's build container (infra/dev/Dockerfile),
# with the checkout mounted at /src and the caches kept in Docker volumes.
#
#   infra/dev/run.sh cargo nextest run -p zend-harness         # nightly, kache
#   NIGHTLY=0 infra/dev/run.sh cargo check -p zend-engine      # stable, as CI
#   KACHE=0 infra/dev/run.sh cargo build -p zend               # without the cache
#
# Default: nightly with -Zunstable-options --jobs-frontend=$FRONTEND (default 8), and
# kache keeping workspace crates incremental (KACHE_PRESERVE_INCREMENTAL=1), measured
# in docs/reference/testing.md. Releases are stable: stage.sh sets NIGHTLY=0.
# NIGHTLY=0   stable toolchain (rust-toolchain.toml), as CI builds.
#             Nightly has its own target volume: stable and nightly artifacts don't mix.
# TARGET=x    the target volume's name part (default: the checkout's folder name), so
#             each worktree has its own target and two builds never wait on one lock.
# KACHE=0     no rustc wrapper. KACHE_* variables are passed to kache.
# The image is rebuilt when the Dockerfile or the nextest pin changes.
set -euo pipefail
ROOT="$(cd "$(dirname "$0")/../.." && pwd)"
HASH="$(cat "$ROOT/infra/dev/Dockerfile" "$ROOT/scripts/ci/install-nextest.sh" | sha256sum | cut -c1-12)"
IMAGE="zend-dev:$HASH"

if ! docker image inspect "$IMAGE" >/dev/null 2>&1; then
  echo "run.sh: building $IMAGE" >&2
  docker build -t "$IMAGE" -f "$ROOT/infra/dev/Dockerfile" "$ROOT/scripts/ci" >&2
fi

env=(-e CARGO_TERM_COLOR="${CARGO_TERM_COLOR:-auto}")
name="${TARGET:-$(basename "$ROOT" | tr -c 'a-zA-Z0-9_.\n-' '_')}"
target="zend-dev-target-$name"
if [ "${NIGHTLY:-1}" = 1 ]; then
  env+=(-e RUSTUP_TOOLCHAIN=nightly -e RUSTFLAGS="-Zunstable-options --jobs-frontend=${FRONTEND:-8}")
  target="$target-nightly"
else
  env+=(-e RUSTUP_TOOLCHAIN=stable)
fi
[ "${KACHE:-1}" = 1 ] && env+=(-e RUSTC_WRAPPER=kache)
# kache's own settings (KACHE_PRESERVE_INCREMENTAL, KACHE_PROGRESS, …) pass through.
export KACHE_PRESERVE_INCREMENTAL="${KACHE_PRESERVE_INCREMENTAL:-1}"
for v in $(compgen -e | grep '^KACHE_' || true); do env+=(-e "$v"); done

tty=(); [ -t 0 ] && [ -t 1 ] && tty=(-it)
# ZEND_DEV_SANDBOX=1: let bubblewrap and pasta run inside (the harness sandbox
# tests). Off by default: it drops Docker's seccomp and AppArmor profiles and
# its /proc masks.
sbx=()
[ "${ZEND_DEV_SANDBOX:-}" = 1 ] && sbx=(--security-opt seccomp=unconfined --security-opt apparmor=unconfined --security-opt systempaths=unconfined --device /dev/net/tun)
exec docker run --rm "${tty[@]}" "${sbx[@]}" --init \
  -v "$ROOT:/src" \
  -v "$target:/opt/target" \
  -v zend-dev-registry:/opt/cargo/registry \
  -v zend-dev-git:/opt/cargo/git \
  -v zend-dev-kache:/opt/kache-store \
  "${env[@]}" -w /src "$IMAGE" "$@"
