# Features of this fork

What sandyhq/zend adds on top of the upstream app, one paragraph each, with where to read how to use it. Upstream's own features are in [README.md](../README.md) and the other files in this folder. What comes next: [roadmap.md](roadmap.md). What changed when: [CHANGELOG.md](../CHANGELOG.md).

## For agents working with other agents

### Child sessions and child events
A chat can start other chats as subagents, side chats or child sessions (`create_chat`, `create_chats`). The parent does not wait or poll: when a child settles a turn, the engine sends the parent a `[Zend event: …]` message with the outcome and the child's last reply or question.
How to use: [mcp.md](mcp.md), sections "Parent links", "Child events" and "Delegating across providers".

### Visuals that ask (`ui.answer`, `update_visual`, `read_visual`)
An agent's visual can ask: text fields, checkboxes, choices and a button that puts the reader's entries as words under the draft in the composer, never sent by itself. Zend keeps the entries by input name. While a visual waits for its answers and its place has scrolled out of view, it is pinned above the composer (scrolled back to its place, it returns there, keeping the field being typed in); once the person sends a message holding them it shows "Answers sent". The agent changes it in place with `update_visual` and reads it with `read_visual`; entries are read-only to it until sent, and each child event names the parent's open visuals so a lead keeps the visual up to date instead of asking again.
How to use: [visuals.md](visuals.md), section "Asking the reader"; [mcp.md](mcp.md), "Child events".

### Visual kit (`ui.kit`)
Ready-made components for visuals, each one call with data instead of a hundred lines of nodes: `ui.kit.Form` with `Input`, `TextArea`, `Checklist`, `Choice`, `Radio`, `Select`, `Toggle`, `OptionCards`, `FollowUps` asks the reader and ends in the answer button; `Card`, `Callout`, `KeyValue`, `List`, `Steps`, `Status`, `Progress`, `Metric` and a few more show things. Built from the existing nodes in Zend's style; every renderer has them. How to use: [visuals.md](visuals.md), section "Kit"; examples in `visuals/`.

### Instructions from plugins (`session.instructions`)
A plugin can give text to the agents of your chats: for every chat on the device, one project's chats, or one chat. The text goes whole into the agent's system prompt as its own section and is renewed every turn. 8 KB per chat. Settings shows each plugin's text, read only. Works for chats on Pi; other harnesses run without it.
How to use: [plugins.md](../apps/zgx/docs/plugins.md), section "Telling your agents". Example: `apps/zgx/plugins-dev/delegation`.

## For building plugins

All of these are described in [plugins.md](../apps/zgx/docs/plugins.md), section "The loop". Each has an MCP tool and a CLI command; the tools are listed in [mcp.md](mcp.md), "Plugin tools".

### Logs
The engine keeps what each plugin printed and what failed, with the full traceback: the last 500 lines in memory and in `logs/plugins/<id>.log`.
`plugin_logs { id, since? }` or `zend plugin logs <id> [--follow]`.

### Check
Syntax and types of a plugin's Luau before it runs, against Zend's window API, the plugin's `zend.luau` and the `@ds` modules. A file starting with `--!strict` is checked strictly. Types need luau-lsp; the first check fetches version 1.70.1 when none is found.
`check_plugin { id | path }` or `zend plugin check <id or folder>`.

### Pictures without a shown window
`plugin_status { id, snapshot = true }` returns a PNG of where the plugin shows, drawn for the picture, so the window need not be in front. With `page`, `width`, `height` and `state` it pictures one of the plugin's own pages without opening it.

### Preview apart from your window
A copy of a plugin's folder runs in a Zend of its own that never shows on screen, with a copy of what the plugin keeps and the permissions you allowed. `plugin_input` clicks and hovers there. It stops after 30 minutes without a call.
`preview_plugin { path }` or `zend plugin preview <folder>`.

### Preview in your window, Keep or Revert
A changed copy of one of your own plugins runs in your window in place of the kept version, with a bar offering Revert and Keep. Only the person can keep; an agent can start and revert.
`preview_plugin { path, in_window = true }` or `zend plugin preview <folder> --in-window`.

### `zend.run` in the plugin's folder
A program a plugin runs without a `cwd` starts in the plugin's own folder.
