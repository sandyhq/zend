# Working in this repository

For agents and people changing code here. Short on purpose; each line points at the file that holds the detail.

## What this is
The sandyhq fork of Zend: a controller for coding agents. A Rust engine (`crates/engine`) runs the agents; a window (`apps/zgx`, Luau interface under `apps/zgx/ui`) shows them; an MCP server (`crates/mcp`) lets agents drive Zend. What the fork adds: [docs/features.md](docs/features.md). How the parts fit: [docs/architecture-fork.md](docs/architecture-fork.md). Branches and upstream: [docs/fork.md](docs/fork.md). Words: [CONTEXT.md](CONTEXT.md).

## Where things are
| You change | Look in |
|---|---|
| An MCP tool, its description | `crates/mcp/src/tools.rs`, `crates/mcp/src/tools/`, `crates/mcp/src/zend.rs`; list it in [docs/mcp.md](docs/mcp.md) |
| Plugins, engine side (logs, check, preview, instructions, signing) | `crates/engine/src/plugins/` |
| Plugins, window side (loading, grants, consent, placing) | `apps/zgx/src/plugins/` |
| The plugin SDK and guide | `apps/zgx/plugins-sdk/zend.luau`, [apps/zgx/docs/plugins.md](apps/zgx/docs/plugins.md) |
| Example plugins | `apps/zgx/plugins-dev/` (dev), `apps/zgx/plugins-builtin/` (shipped) |
| Visuals (`show_visual`) | `crates/visual` (script, nodes), `crates/ui/src/visuals.rs`, `apps/zgx/src/visuals.rs`, `apps/zgx/ui/ds/visual.luau`; [docs/visuals.md](docs/visuals.md) |
| Child sessions and child events | `crates/engine/src/child_events.rs`; [docs/mcp.md](docs/mcp.md) |
| Harness adapters (Pi, Claude Code, Codex, …) | `crates/harness`; [docs/pi.md](docs/pi.md) |
| The CLI | `apps/zend/src`, `crates/cli` |
| The interface's components | `apps/zgx/ui/ds/`, screens in `apps/zgx/ui/screens/` |

`apps/zgx` is a Cargo workspace of its own (own `Cargo.lock`); build and test it from `apps/zgx`.

## Build and test
Full rules and numbers: [docs/reference/testing.md](docs/reference/testing.md).
- While editing: `cargo check -p <crate>`, tests with `cargo nextest run --no-fail-fast -p <crate>`. nextest skips doctests: also `cargo test --doc`.
- Before calling work done: the whole workspace once, and `apps/zgx` if you touched it.
- About 10 tests fail on unmodified `main` on the dev machine (see testing.md, "Known failures"), about 11 more in `apps/zgx`. Compare against that baseline; don't chase them in unrelated work.
- Integration tests go into `tests/it/` of the crate, never a new top-level `tests/*.rs`.
- Long builds: in the background with `nohup`, poll the log. Give each worktree its own target dir outside the repo (`CARGO_TARGET_DIR=~/.cache/zend-target-<name>`).
- A green build is not installed behaviour. To try a change, run a stand-in instance: [docs/reference/test-instance.md](docs/reference/test-instance.md).

## Never
- Never stop, restart or reinstall a running installed Zend from inside one of its chats: its engine hosts the chat you are in. Installing is the owner's step: [docs/reference/build-and-deploy.md](docs/reference/build-and-deploy.md).
- Never open a window on the person's screen for a test. Use `ZGX_OFFSCREEN=1` (test-instance.md).
- Never spend real model usage for a live check without saying so; use a mock model.
- Never push, merge to `main`, or force-push without the owner's word.
- Never allow a plugin or keep a preview for the person; those are theirs, in Zend's window.

## Docs to keep in step
A change that adds or changes a feature also changes, in the same commit:
- [CHANGELOG.md](CHANGELOG.md): one line.
- [docs/features.md](docs/features.md) if it is something new a person or agent can use; [docs/roadmap.md](docs/roadmap.md) when an item moves.
- The guide that tells how to use it: `apps/zgx/docs/plugins.md` (plugin authors), `docs/mcp.md` (tools), `docs/visuals.md` (visual scripts), `docs/using-plugins.md` and `docs/working-with-agents.md` (people).
- A decision that will still matter in a year: an ADR in `docs/adr/`. A new word: [CONTEXT.md](CONTEXT.md).
