#!/usr/bin/env bash
# The Linux Rust tests that used to run as separate `cargo test` invocations in
# session-sync-regressions, cursor-compatibility and preview-tests, as ONE
# build and ONE nextest run (separate invocations recompiled the same
# dependency graph with different feature sets).
#
# Same coverage as the old invocations:
#   Integration tests are one `it` binary per crate (docs/reference/testing.md),
#   so suites are picked by module (`test(/^name::/)`), not by binary.
#   zend-harness  every suite (cursor-compatibility ran `-p zend-harness`
#                  plus `--test pi_rpc` with native-fixture; native-fixture only
#                  adds the pi_rpc fixture binaries, so it is on for the whole build)
#   zend-preview  every suite (preview-tests)
#   zend-doc      lib + persistence and memory regressions + attachments_roundtrip
#   zend-sync, zend-update   lib only (session-sync-regressions)
#   zend-cli, zend-launch, zend-notify   `zend`'s command line, the engine
#                  helper's start-up and the zend:// links, shared by the apps
#   zend-engine   lib + the integration tests named below (plugin_services: the
#                  plugin host's services, end to end; also on macOS in zgx-tests.yml). Engine has many other
#                  integration suites (live agents etc.) that no workflow runs; they
#                  are compiled into its `it` binary but filtered out by name.
#   voice (moved from voice-tests.yml offline-linux, which ran them as 14 separate
#   `cargo test` builds): zend-voice-media, zend-voice-session and zend-orb
#                  lib, zend-proto lib `voice` tests, zend-mcp lib, zend-rpc
#                  device_room, engine voice_runtime and device_routing. They need no
#                  audio stack, and harness already depends on all of them.
# nextest does not run doctests; these crates have none (their doc comments
# contain no Rust code blocks).
# Extra arguments are passed to nextest (the nightly job adds `--release`).
# NEXTEST_PROFILE=ci (pull requests and main) leaves out the slow and soak tests
# that .config/nextest.toml lists; the nightly runs use the default profile.
set -euo pipefail
cd "$(dirname "$0")/../.."

# Engine suites to run (modules of zend-engine's `it` binary). `misc` holds the
# small suites. `restart_hook` is a binary of its own: the hook it checks is
# process-wide. The orb geometry tests live in zend-orb's `geometry`, device_room
# in zend-rpc.
engine="misc|session_publication|restart_resume|side_chats|message_queue|voice_runtime|device_routing|plugin_services"
tests=(--test it --test snapshot_memory --test restart_hook --test geometry --test device_room)

exec cargo nextest run --config-file .config/nextest.toml --locked --no-fail-fast \
  -p zend-harness -p zend-engine -p zend-sync -p zend-update -p zend-doc -p zend-preview \
  -p zend-mcp -p zend-proto -p zend-rpc -p zend-orb -p zend-voice-session -p zend-voice-media \
  -p zend-cli -p zend-launch -p zend-notify \
  --features zend-harness/native-fixture \
  --lib --bins "${tests[@]}" \
  -E "not ((package(zend-proto) & not test(voice)) | (package(zend-rpc) & kind(lib)) | (binary_id(zend-engine::it) & not test(/^($engine)::/)))" "$@"
