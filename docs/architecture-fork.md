# Architecture: what this fork adds

[ARCHITECTURE.md](../ARCHITECTURE.md) is upstream's design document from the rewrite. It describes the engine, the docs and sync, and a gpui interface; it says nothing about plugins, visuals, the MCP server, child sessions or the zgx window. This page covers those, as they are in the code on 2026-11-15. It is a map, not a specification: each part names the code to read.

## The pieces

```
 agent (Pi, Claude Code, Codex, …)
   │  stdio JSON-RPC
 zend mcp  (crates/mcp)  ──────────────┐
                                       │ engine RPC (crates/rpc), port ZEND_IPC_PORT
 window zend-zgx (apps/zgx) ───────────┤
   Luau interface (apps/zgx/ui)        │
   plugins' window parts (init.luau)   ▼
                              zend-engine (crates/engine)
                                chats, harness adapters (crates/harness)
                                plugins: signing, grants, services (service.luau),
                                         logs, check, previews, instructions
                                child events
```

- **Engine** (`crates/engine`, binary `zend-engine`): runs agents through harness adapters, hosts the chats' docs, and is the only thing the MCP server and the CLI talk to.
- **Window** (`apps/zgx`, binary `zend-zgx`): the desktop interface. Rust host with the interface written in Luau (`apps/zgx/ui`: `screens/`, `ds/` components) on the zgx toolkit (`third_party/zgx`). This is the window the fork installs. `crates/ui` holds the gpui interface ARCHITECTURE.md describes.
- **MCP server** (`crates/mcp`, run as `zend mcp`): the tools agents use to drive Zend. Started per agent with `ZEND_CHAT_ID` naming the chat it speaks for. Reference: [mcp.md](mcp.md).
- **CLI** (`apps/zend`, `crates/cli`): `zend status`, `zend plugin …`, `zend mcp`.

## Child sessions and child events
A chat records its parent (`parentChatId`). When a child settles a turn, the engine that hosts it (`crates/engine/src/child_events.rs`) puts one `[Zend event: …]` line into the parent as a durable chat command: a working parent is steered, a resting parent starts a turn, a parent waiting on the person gets it at that turn's end. Children settling within 2 s are told together. Details and the cases with no event: [mcp.md](mcp.md), "Child events".

## Plugins
A plugin is a signed folder. Its **window part** (`init.luau`) runs in the window's Luau, sandboxed, each call checked against the permissions the person allowed (`apps/zgx/src/plugins/`: `grants.rs`, `consent.rs`, `mod.rs`). Its **service** (`service.luau`) runs in the engine without a window (`crates/engine/src/plugins/service`). The engine owns identity and trust: manifest, signing, scan, install, lifecycle (`crates/engine/src/plugins/`).

What the fork added there:

| Part | Engine | Window |
|---|---|---|
| Logs | `plugins/logs.rs` | `plugins/plugin_log.rs` |
| Check | `plugins/check.rs`, `plugins/check/` (luau-lsp) | checks on save, writes to the log |
| Pictures | `plugins/snapshot.rs` | `plugins/mod.rs` (draws offscreen) |
| Preview apart | `plugins/staging.rs` | `plugins/stage.rs`; `ZGX_OFFSCREEN` in zgx-desktop |
| Preview in the window | `plugins/window_preview.rs` | `plugins/plugin_preview.rs`, `plugins/host_bar.rs` |
| Instructions | `plugins/instructions.rs`, `plugins/agents.rs` | Settings page of the plugin |

Decisions: [adr/0003](adr/0003-plugin-logs-live-in-the-engine.md), [adr/0004](adr/0004-plugin-preview-is-a-second-zend.md).

## Plugin instructions
Entries are kept by the engine per plugin (`plugin-instructions.json` in the data folder). At the start of each turn the harness adapter gives the agent each allowed plugin's section. Only the Pi adapter does this today. Decision: [adr/0005](adr/0005-plugin-instructions-go-into-the-system-prompt.md).

## Visuals
`show_visual` sends a Luau script to the engine (`PublishVisual`). The engine runs it once to check it (`crates/visual`, `zend_visual::check`), stores it beside the chat's uploads and folds a `visual` part into the live turn; the doc carries only path and title. The window runs the script again to render it (`apps/zgx/src/visuals.rs`, `apps/zgx/ui/ds/visual.luau`; `crates/ui/src/visuals.rs` for the gpui interface), with scenes and shaders on the GPU (`crates/visual-gpu`). Reference: [visuals.md](visuals.md). Planned change: [adr/0006](adr/0006-visual-entries-are-kept-by-zend.md).

## Not covered here
Sync, the edge workers, mobile apps, voice, terminals and previews are upstream's and described in ARCHITECTURE.md and the other files under `docs/`. Which of the two interfaces (`crates/ui`, `apps/zgx`) upstream ships where was not checked.
