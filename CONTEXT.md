# Domain context

## Theme vocabulary

- **Theme family** — A named collection of related theme variants that share an origin, such as Night Owl and Night Owl Light.
- **Theme variant** — One complete, resolved palette for a single appearance (`light` or `dark`). Runtime UI consumes variants, not source-format tokens.
- **Theme source** — The durable origin of a custom family: an imported snapshot, linked file, linked package, or editable native file.
- **Imported snapshot** — A self-contained copy of a compiled theme family. It no longer follows changes to its original source.
- **Linked theme** — A custom family that follows a source on disk and can be reloaded without re-importing it.
- **Linked file** — A link to one VS Code-compatible theme definition.
- **Linked package** — A link to a VS Code extension folder or `package.json` that declares one or more related theme variants.
- **Editable theme** — A duplicate stored as a native resolved-family JSON file. Users edit the file directly and explicitly reload it; invalid edits preserve the last known good family.
- **Last known good** — The most recent successfully compiled family retained by a linked theme when its current source is missing or invalid.
- **Import report** — A per-variant summary of mapped roles, fallbacks, unsupported values, and inferred decisions produced during compilation.
- **Mapping review** — The optional advanced view of an import report. Normal theme selection and import do not expose token names.
- **Theme hardening** — Deterministic post-mapping repairs that prefer stronger semantically related source colors, minimally adjust only shared Zend roles when needed, and record every decision in the mapping review.
- **Theme default accent** — The interaction accent chosen or inferred for a theme variant by its authoring source.
- **Accent override** — A Zend preset that replaces interaction roles only; syntax, terminal ANSI, diff, warning, error, and success colors remain owned by the theme.
- **Recommended surface treatment** — A theme variant's authored recommendation for whether its surfaces are frosted or opaque. It is used only when the user keeps the surface preference at theme default.
- **Surface preference** — A device-local choice of theme default, frosted, or opaque that is independent of appearance, theme, and accent selections.
- **Resolved surface treatment** — The effective frosted or opaque treatment produced by applying the surface preference to the active variant's recommendation.

## Chat vocabulary

- **Chat** — One conversation with an agent, hosted on a device, optionally in a project. Also called a session in the sidebar ("Sessions").
- **Project** — A folder on one specific device. A chat in a project works in that folder.
- **Harness** — The agent program a chat runs on: Pi, Claude Code, Codex, Cursor and others. A harness offers models.
- **Parent** — The chat another chat was created from. Delegation is one level deep: a chat with a parent creates no chats.
- **Subagent** — A chat created by an agent to do delegated work. Listed under its parent's Subagents, not in the sidebar. The default kind when a chat creates a chat.
- **Child session** — A chat created by an agent that is a full chat of its own: its own sidebar row nested under the parent, its own page, kept when the parent is archived or deleted. Kind `session`.
- **Side chat** — A chat beside a parent that the person treats as their own conversation. Kind `side`. Tells its parent nothing unless asked to.
- **Standalone chat** — A chat with no parent. Kind `chat`.
- **Paired server** — Another Zend server of the owner's, paired with this one by a one-time code (`zend peer pair` / `join`) and known by its node key. Reaches this server over the peer link (tailnet only, no relay) with only the grants this side gave it (`Caller::PeerServer`). Not the same as a relay peer (`Caller::Peer`, another device in the device room).
- **Grant** — One thing this server lets a paired server do here: `sessions.read`, `sessions.run:<project>`, `sessions.run.unconfined:<project>`, `projects.share:<project>`, `workers.manage`. Each side keeps its own; none at first.
- **Sandbox marker** — `sandbox: "project"` on a chat a paired server created under `sessions.run`: kept on its host only, and its runs must be confined to the project's folder (refused until sandboxed runs land).
- **Child event** — The `[Zend event: …]` message the engine puts into a parent when a child settles a turn (completed, errored, interrupted, waiting for input). Zend's line, not an agent's.
- **Settle** — A chat's turn ending in one of those four ways.
- **Lead** — An agent that plans with the owner and gives groups of work to child sessions. A working convention, not a kind of chat.

## Plugin vocabulary

- **Plugin** — A signed folder (`zend-plugin.json`, `init.luau`, optionally `service.luau`) that adds to Zend's interface or gives agents tools or instructions.
- **Window part** — A plugin's `init.luau`, run in the window.
- **Service** — A plugin's `service.luau`, run by the engine with no window open.
- **Permission** — One thing a plugin's manifest asks to do (`pages.add`, `commands`, `session.instructions`, …).
- **Allow** — The person granting a plugin its permissions, in Zend's window. Never done by an agent.
- **Pending** — A plugin installed but not yet allowed. It does not run.
- **Place** — A node a plugin puts before, after or in place of one of Zend's named elements (`zend.place`).
- **Page** — A screen of a plugin's own, reached from the sidebar (`zend.page`).
- **Kept** — What a plugin stores with `zend.store`; survives restarts.
- **Plugin instructions** — Text a plugin gives your chats' agents, placed whole in their system prompt as the section `plugin-<id>`. An **entry** is one such text under a key, for every chat, a project's chats or one chat.
- **Preview** — A changed copy of a plugin run for trying. **Apart**: in a Zend of its own that never shows. **In the window**: in the person's window in place of the kept version, until **Keep** or **Revert**.
- **Check** — Syntax and type checking of a plugin's Luau before it runs.
- **Template** — A whole copy of Zend's interface that can be rewritten. Not a plugin.

## Visual vocabulary

- **Visual** — An interactive piece an agent shows in a chat with `show_visual`: a Luau script Zend runs in a sandbox and renders with its own interface.
- **Node** — One element of a visual's tree (`ui.text`, `ui.chart`, …).
- **Input** — A node the reader fills in (planned: field, area, check, checklist, choice, select). Each has a fixed **name**.
- **Entry** — What the reader put into one input. Kept by Zend per visual and input name. **Unsent** until the person sends a message holding it; unsent entries are the person's draft, readable by the agent, not binding.
- **Open visual** — A visual with inputs whose entries are not yet sent. Pinned above the composer while its place in the transcript is out of view; answered by a message holding its heading line, or closed by hand with its ×.
- **Kit** — Ready-made visual components shipped with Zend, called with data (`ui.kit.<Kind> { … }`) (planned).

## Development vocabulary

- **Stand-in instance** — A second Zend (own engine, data folder and port, window drawn offscreen) for trying a change without touching the installed one.
- **Switch** — Replacing the installed build with a staged one. Stops the engine; the owner's step.
- **Plan, group** — A piece of agreed work (proposal, design, tasks) and one independently checkable part of it (`G1`, `G2`, …), built on a branch `<plan>-G<n>`.
