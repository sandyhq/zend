# Changelog

Changes of this fork (sandyhq/zend), newest first. Upstream changes are in the git history.

## 2026-11-15

### Added
- Plugin logs: prints and errors with tracebacks, kept by the engine (`plugin_logs`, `zend plugin logs`).
- Plugin check: Luau syntax and types (`check_plugin`, `zend plugin check`); fetches luau-lsp 1.70.1 when none is found.
- `plugin_status` pictures without a shown window, and of a plugin's own page with `page`, `width`, `height`, `state`.
- Plugin preview in a Zend of its own, offscreen, with `plugin_input` (`preview_plugin`, `zend plugin preview`); stops after 30 minutes without a call and with the engine that started it.
- Plugin preview in the person's window with a Keep / Revert bar (`--in-window`).
- Permission `session.instructions`: `zend.set_instructions`, `zend.instructions`, and `run.preparing` answered with `{ instructions }`; whole in Pi's system prompt.
- Example plugin `plugins-dev/delegation`; a plugin's Settings page shows what it tells agents.
- `docs/features.md`, `docs/roadmap.md`, this file; plugin tools listed in `docs/mcp.md`.
- Visuals that ask: input nodes and `ui.answer` (the inputs as words into the composer); `update_visual` and `read_visual`; a message holding a visual's answers marks it sent (`MarkVisualSent`); open visuals pinned above the composer; child events name the parent's open visuals.
- Visual kit `ui.kit`: 22 components (Form and its inputs, Card, Callout, KeyValue, List, Steps, Status, Progress, Metric and more), each one call with data; examples in `visuals/`.

### Changed
- `zend.run` without a `cwd` runs in the plugin's own folder.
- Removing a plugin forgets its log and its instructions.

## Earlier
- Child sessions and child events (PR #1).
