# Features of this fork

What sandyhq/zend adds on top of the upstream app, one paragraph each, with where to read how to use it. Upstream's own features are in [README.md](../README.md) and the other files in this folder. What comes next: [roadmap.md](roadmap.md). What changed when: [CHANGELOG.md](../CHANGELOG.md).

## For people reading a chat

### Every tool's row opens to what it did
A tool's row in a reply's work opens (a chevron at its end shows under the pointer) to what that call did, drawn for its kind: a read of a picture shows the picture the read gave the agent (the reply's frame, lightbox and copy; kept by the device that ran it, up to 10 MB, from pi and Claude Code; an older read shows its path and says no picture was kept); a read of text, its lines coloured by language; an edit, write or patch, its diff; codemode, its script as JavaScript, then its result; an MCP or other tool, its arguments key by key (JSON for nested ones) and its result (JSON as JSON); a web search, its answer as prose. Commands open as before. What it shows isn't in the chat's doc: the window asks the device that ran the tool, which reads its run journal (`ReadToolExchange`, `ReadToolImageChunk`), with secret inputs, secret files' contents and secret commands' output kept on that device. That last is by name: a codemode script that reads `.env` (`tools.read` inside it) is one call to Zend, so its result gets only the text redaction every output gets (private key blocks dropped, `KEY=value` values and token shapes hidden), not withheld whole. A device that is offline or runs an older Zend says so, and a press asks again. Pi's edits now carry their diff (from the tool's own numbered diff), Codex's a one-file change's patch (the app-server's `FileUpdateChange.diff`). In the zgx window.

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

## For people in a chat

### A phone-sized window (compact)
Below 600 pt wide (a phone, a narrow split) the window rearranges instead of squeezing: the sidebar is a drawer over the page (the title bar's toggle or a swipe in from the left edge opens it; a tap beside it, a swipe back, Escape or choosing anything closes it), the panel opens as a sheet over the whole page, Settings shows its list of pages or one page with a Settings button back to the list, and the title bar's controls are 44 pt touch targets (no Forward). Wider windows look exactly as before.
Code: `apps/zgx/ui/screens/shell/fit_policy.luau` (`COMPACT_BELOW`), `fit.luau`, `init.luau` (the drawer), `settings.luau`, `ds/title_bar.luau` (touch). Plan: `plans/android-zgx-app` (G4).

### Connect a phone by QR code
Settings → Devices on a computer has Show code: a QR code with this computer's relay (edge) address and its own key, for a minute or until a new device joins. On the Android app the same page has Scan code (the camera; Android asks for it the first time) and a box to paste the code's text. The phone keeps the relay and key in its own files and reconnects at once; your computer's sessions then show on it. Anyone who sees the code can join your relay as you: show it only to your own phone.
Code: `apps/zgx/src/phone_link.rs`, `src/android_camera.rs`, `ui/screens/settings/devices.luau`. Plan: `plans/android-zgx-app` (G3).

### A phone link over the tailnet
With no relay, the phone can be a window onto this computer directly: Show code (Settings → Devices) turns on the phone link, a second listener of this computer's engine on its tailnet address only, and the code holds that address (its MagicDNS name or its tailnet IP, chosen under it), the port and the link's own key. A phone that scans it keeps the computer, runs no engine of its own and shows exactly this computer's sessions: reading, sending and new chats all go to its engine, and it reconnects when the engine restarts or the network comes back. The computer must be running Zend. The link refuses addresses and peers outside the tailnet, takes only its own key, and serves only what a window needs (no credentials, sign-in, vault, relay commands or plugin authority). The switch beside it turns it off again; New key cuts every phone that had the old one. It stays on across restarts until turned off. While the phone scans, the camera's picture shows in the scan area. A phone linked this way uses the computer's account as it is: the username is claimed on the computer (setup on the phone passes over the account's steps, and Profile says so).
Code: `crates/rpc/src/remote.rs`, `crates/engine/src/phone_link.rs`, `apps/zgx/src/phone_link.rs`, `apps/zgx/src/engine/rpc.rs`, `ui/screens/settings/devices.luau`. Plan: `plans/android-zgx-app/direct-link.md`.

### The computer's plugins on a linked phone
A phone linked this way shows the plugins that are on at the computer: their sidebar items, pages and menus. The computer's engine has checked each one's signature and your decisions; it tells the phone no paths of its own, only which files each plugin is made of (relative path and sha256). The phone fetches them one by one (`ReadPluginFile`: only those files, from the plugin's checked copy, never through a link), checks each against the list, keeps them under `<app data>/plugins-remote/<id>/` and runs the window part from there. What a phone can't do (`commands`, `native`, `files`, `windows`, `web`) is left out as on any phone. A plugin's window calls its service on the computer, answered only for a plugin that's on and allowed there, with what it was allowed there; never another device's through the computer.
Code: `crates/engine/src/plugins/client.rs`, `apps/zgx/src/plugins/fetched.rs`.

### Ideas & Intents
A page under Search in the desktop sidebar for what comes to mind before it belongs anywhere. Type an idea and save it (⌘Enter): no title, project or chat. Or record a voice memo between Record and Stop, up to a minute, with the composer's dictation waveform under the buttons following what the microphone hears (from the recording's own samples): the recording is written to `<data>/ideas` as a WAV first, then transcribed on this device with the speech model Settings → Voice downloads. Without the model, or when transcription fails, the memo keeps its audio and says why, with Transcribe or Try again. Memos play from the page, also after a restart; newest first. Leaving the page (Settings, a chat, anywhere else) stops a recording and saves what was said until then; none records out of sight. A Save that fails keeps the typed words in the box. Nothing leaves the device and nothing goes to an agent. The composer's dictation is separate and still keeps no audio.
Code: `apps/zgx/src/ideas.rs`, `apps/zgx/ui/screens/ideas`. Plan: `plans/ideas-intents`.

### Edit a prompt, fork from a message
Hovering one of your prompts shows Edit: the prompt turns into an editor where it stands, its files and pictures kept as chips you can take off. Save & send asks once ("Removes N messages below. Files the agent changed are not reverted."), then the engine removes the prompt and everything below it for good and the edited words go as a new turn. The agent forgets the removed part: Zend drops the chat's provider session and the next turn starts a fresh one fed the kept history, on every harness. Not while a turn runs. Hovering any prompt or reply also shows Fork from here: a side chat with the conversation up to that reply, or up to just before that prompt with its words waiting in the side chat's message box.
How to use: [working-with-agents.md](working-with-agents.md), "Editing a prompt, forking from a message". Engine: `RewindChat`, `ForkSideChat` with `messageId`.

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
