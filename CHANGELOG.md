# Changelog

Changes of this fork (sandyhq/zend), newest first. Upstream changes are in the git history.

## 2026-11-17

### Fixed
- Engine: a Stop sent just after a steer is no longer dropped as aimed at a finished turn when the steer lands first; it stops the turn, steer included (`runtime_survival::a_steer_sent_before_stop_is_cancelled_with_it`). A Stop now names the sends it stops (`Interrupt.cancels`, ADR 0007) and the host decides by that, not by clocks or ledger order: across devices and by relay a send named by a Stop never runs after it, and a late Stop never stops a turn its sender didn't know of. A Stop from an older engine keeps the old rule.

## 2026-11-16

### Added
- Chats: Edit on your own prompt (removes it and everything below, after one confirmation, then sends the edited words; the agent starts a fresh session fed the kept history) and Fork from here on any prompt or reply (`RewindChat`, `ForkSideChat` `messageId`).
- Transcript: every tool's row opens to what it did, by kind (a read's picture or coloured lines, an edit's diff, codemode's script and result, an MCP tool's args and result), read from the host's run journal (`ReadToolExchange`, `ReadToolImageChunk`); a chevron shows a row opens; pi's edits carry their diff, codex's one-file changes their patch (`FileUpdateChange.diff`, v2's `kind: {type}` read too).
- Engine: a read's picture is the bytes its result gave the agent (`AgentEvent::ToolImage` from pi and Claude Code, ≤10 MB, kept beside the journal), never the file at its path when the row opens.

## 2026-11-15

### Added
- Window: words people read now wrap instead of being cut off (field labels, a visual form's questions, settings, dialogs); a tool row in the activity rail shows up to two lines of its command and fades after, a press opens the whole call. A text that stays on one line says why (`-- one line:`), checked by `wrap_lint_tests`.
- Visuals: `ui.image` shows a picture passed in show_visual's `assets` (a wipe between two with `before`, numbered marks with `marks`); kit Image, Gallery, Compare and Hotspots use it, with examples in `visuals/media` and their pictures in `visuals/assets`.
- Visuals: `ui.code` (source lines coloured by the window's own highlighter, gutter, added/removed/focus marks); kit Code, Snippet, File and Diff use it. `ui.null` and `ui.kit.Json { text = "<json>" }`.
- Visual kit, batch c: `ui.kit` Chart (10 types), Sparkline, SegmentedBar, Waterfall, Treemap, Tree, Diagram (Mermaid), Map, Scene, Shader, Carousel, Icon, with examples in `visuals/{charts,drawing,media,3d}`.
- Plugin logs: prints and errors with tracebacks, kept by the engine (`plugin_logs`, `zend plugin logs`).
- Plugin check: Luau syntax and types (`check_plugin`, `zend plugin check`); fetches luau-lsp 1.70.1 when none is found.
- `plugin_status` pictures without a shown window, and of a plugin's own page with `page`, `width`, `height`, `state`.
- Plugin preview in a Zend of its own, offscreen, with `plugin_input` (`preview_plugin`, `zend plugin preview`); stops after 30 minutes without a call and with the engine that started it.
- Plugin preview in the person's window with a Keep / Revert bar (`--in-window`).
- Permission `session.instructions`: `zend.set_instructions`, `zend.instructions`, and `run.preparing` answered with `{ instructions }`; whole in Pi's system prompt.
- Example plugin `plugins-dev/delegation`; a plugin's Settings page shows what it tells agents.
- `docs/features.md`, `docs/roadmap.md`, this file; plugin tools listed in `docs/mcp.md`.
- Visuals that ask: input nodes and `ui.answer` (the inputs as words into the composer); `update_visual` and `read_visual`; a message holding a visual's answers marks it sent (`MarkVisualSent`); open visuals pinned above the composer once their place scrolls out of view, back in place when it scrolls into view; child events name the parent's open visuals.
- Open visuals: a message holding the visual's heading line (`### <title>`) answers it, whatever else was rewritten (two visuals with one title: one per heading line, the newest first); the pinned visual has a × (closes it by hand: marked sent, nothing sent) and a minimise button (folds it to its title line; this window's, per visual).
- Visuals: the pinned visual's × marks it dismissed, a state of its own (`MarkVisualSent` with `dismissed`; the transcript says "Closed without answers"), not sent; `read_visual` says `state: dismissed`, and the next prompt into the chat carries `[Zend: visual "<title>" (<id>) was closed without answers]` once. A later send of its heading line makes it sent.
- Visual kit batch a (dev): Acceptance, Findings, StackTrace, Dependencies, Benchmark, Pipeline, Tests, Logs, Coverage, Changes, Commits, Diff, PullRequest, Environments, Snippet, Code, Json, File (`crates/visual/src/kit_a.luau`), each with an example in `visuals/`.
- Visual kit `ui.kit`: 22 components (Form and its inputs, Card, Callout, KeyValue, List, Steps, Status, Progress, Metric and more), each one call with data; examples in `visuals/`.
- Visual kit, planning and data kinds: Board, Roadmap, Plan, Decision, Estimate, Table, Timeline, Calendar, CalendarHeatmap, Provenance, Sources, Entity, Transcript, Walkthrough, Flashcards (`crates/visual/src/kit_b.luau`).
- Named inputs `ui.number` (slider, stepper, stars), `ui.items`, `ui.rows`, and kit kinds Slider, Stepper, Rating, ListEditor, EditableTable, Quiz, Timer.

### Changed
- `zend.run` without a `cwd` runs in the plugin's own folder.
- Removing a plugin forgets its log and its instructions.
- A message that was not delivered (files over the 24 MB cap, a file that can't be read) stays where it was sent instead of sticking to the end of the chat; it says why, offers Send again and Discard, and is replaced when the same words are sent again. Files over the cap are found before anything is uploaded.

## Earlier
- Child sessions and child events (PR #1).
