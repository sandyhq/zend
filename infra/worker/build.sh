#!/usr/bin/env bash
# Build the worker image (localhost/zend-worker:latest) from a built
# zend-engine and the engine image already on this server: nothing is
# downloaded.
#   infra/worker/build.sh [path/to/zend-engine] [tag]
set -euo pipefail
here=$(cd "$(dirname "$0")" && pwd)
bin=${1:-target/debug/zend-engine}
tag=${2:-localhost/zend-worker:latest}
[ -x "$bin" ] || { echo "no zend-engine at $bin (cargo build -p zend --bin zend-engine)" >&2; exit 2; }
ctx=$(mktemp -d)
trap 'rm -rf "$ctx"' EXIT
cp "$here/Containerfile" "$ctx/"
cp "$bin" "$ctx/zend-engine"
podman build --pull=never -t "$tag" "$ctx"
podman image inspect --format '{{.Id}} {{.Size}}' "$tag"
