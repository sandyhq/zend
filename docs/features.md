# Features of this fork

What sandyhq/zend adds on top of the upstream app, one paragraph each, with where to read how to use it. Upstream's own features are in [README.md](../README.md) and the other files in this folder. What comes next: [roadmap.md](roadmap.md). What changed when: [CHANGELOG.md](../CHANGELOG.md).

## For people reading a chat

### Every tool's row opens to what it did
A tool's row in a reply's work opens (a chevron at its end shows under the pointer) to what that call did, drawn for its kind: a read of a picture shows the picture (the reply's frame, lightbox and copy); a read of text, its lines coloured by language; an edit, write or patch, its diff; codemode, its script as JavaScript, then its result; an MCP or other tool, its arguments key by key (JSON for nested ones) and its result (JSON as JSON); a web search, its answer as prose. Commands open as before. What it shows isn't in the chat's doc: the window asks the device that ran the tool, which reads its run journal (`ReadToolExchange`, `ReadToolImageChunk`), with secret inputs, secret files' contents and secret commands' output kept on that device. A device that is offline or runs an older Zend says so, and a press asks again. Pi's edits now carry their diff (from the tool's own numbered diff). In the zgx window.

## For agents working with other agents

### Child sessions and child events
A chat can start other chats as subagents, side chats or child sessions (`create_chat`, `create_chats`). The parent does not wait or poll: when a child settles a turn, the engine sends the parent a `[Zend event: …]` message with the outcome and the child's last reply or question.
How to use: [mcp.md](mcp.md), sections "Parent links", "Child events" and "Delegating across providers".

### Visuals that ask (`ui.answer`, `update_visual`, `read_visual`)
An agent's visual can ask: text fields, checkboxes, choices and a button that puts the reader's entries as words under the draft in the composer, never sent by itself. Zend keeps the entries by input name. While a visual waits for its answers and its place has scrolled out of view, it is pinned above the composer (scrolled back to its place, it returns there, keeping the field being typed in); once the person sends a message holding its heading line (`### <title>`, whatever else they rewrote) it shows "Answers sent". The pinned visual can be folded to its title line with its minimise button, or closed by hand with its × ("Closed without answers": nothing sent, and the agent is told so once, on the person's next message). The agent changes it in place with `update_visual` and reads it with `read_visual`; entries are read-only to it until sent, and each child event names the parent's open visuals so a lead keeps the visual up to date instead of asking again.
How to use: [visuals.md](visuals.md), section "Asking the reader"; [mcp.md](mcp.md), "Child events".

### Visual kit (`ui.kit`)
Ready-made components for visuals, each one call with data instead of a hundred lines of nodes: `ui.kit.Form` with `Input`, `TextArea`, `Checklist`, `Choice`, `Radio`, `Select`, `Toggle`, `OptionCards`, `FollowUps` asks the reader and ends in the answer button; `Slider`, `Stepper`, `Rating`, `ListEditor`, `EditableTable` and `Quiz` ask for numbers, lists, rows and quiz picks; `Timer` counts down; `Card`, `Callout`, `KeyValue`, `List`, `Steps`, `Status`, `Progress`, `Metric` and a few more show things. Built from the existing nodes in Zend's style; every renderer has them. How to use: [visuals.md](visuals.md), section "Kit"; examples in `visuals/`.

### Code in visuals (`ui.code`, `ui.null`)
A visual can show source lines in their language: `ui.code` colours them with the window's own highlighter (the one Markdown code blocks and diffs use), with line numbers or its own gutter and lines marked added, removed or to look at. The kit's Code, Snippet, File and Diff use it, so a diff keeps its added and removed washes and colours each file by its language. `ui.kit.Json` shows `ui.null` as null and reads a JSON text (`text = "<json>"`), naming the line and column of a mistake. In the zgx window; the other renderers show the lines plain. How to use: [visuals.md](visuals.md), "Nodes" and "Kit"; examples in `visuals/code/`.

### Pictures in visuals (`ui.image`)
A visual can show a picture the agent passes in `assets` (PNG, JPEG or WebP), at the reply's width or a height of its choosing: a screenshot with numbered regions named on hover and picked by a click, two pictures over each other with a line the reader drags between them, or a grid. The kit's Image, Gallery, Compare and Hotspots use it. A name that isn't in `assets`, or a file that isn't a picture, is an error `show_visual` names. Nothing is fetched from the network: the file travels with the visual, so the picture shows on other devices and in plugin pages too. In the zgx window; the other renderers show a line naming it. How to use: [visuals.md](visuals.md), "Nodes" and "Kit"; examples in `visuals/media/`.

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
