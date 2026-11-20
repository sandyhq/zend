# Changelog

Changes of this fork (sandyhq/zend), newest first. Upstream changes are in the git history.

## 2026-11-20

### Fixed
- File viewer: a Markdown file's front matter shows as a table of keys and values above the preview instead of as text, and a long Markdown file is parsed and built lazily around the view (`Markdown { window }`), so it opens and scrolls without stalling.

### Changed
- Pi: an extension's error names the extension and the event it threw in (`Pi extension (<path>, <event>): <error>`), not only the error text.

## 2026-11-17

### Changed
- zgx comes from sandyhq/zengx, pinned (`apps/zgx/Cargo.toml`), no longer vendored in third_party/zgx: zengx is zeronsh/zgx main 558554440 (49 commits past 1accd1b7) with the fork's four zgx edits as commits on top, listed in its FORK.md; Zend's rebrand stays out of zgx. Upstream zgx comes in by merging into zengx main (docs/fork.md, "zgx"). The engine's plugin check reads its own copy of zgx's types, kept in step by `the_engine_s_zgx_types_are_the_window_s`.
- zgx updated to zeronsh/zgx 1accd1b7 (vendored in third_party/zgx, the fork's edits listed in its FORK.md); a window drawn on zgx's software raster still pictures nodes for `plugin_status`. The composer takes upstream's coordinated layout (its size and place eased, cd769402..7a188dfe), with 6df5169d and 82d410d7 (the series alone broke a sidebar-drag perf limit); `scripts/zend-pick-range.sh` takes an upstream range file by file (docs/fork.md).

### Fixed
- Engine: a Stop sent just after a steer is no longer dropped as aimed at a finished turn when the steer lands first; it stops the turn, steer included (`runtime_survival::a_steer_sent_before_stop_is_cancelled_with_it`). A Stop now names the sends it stops (`Interrupt.cancels`, ADR 0007) and the host decides by that, not by clocks or ledger order: across devices and by relay a send named by a Stop never runs after it, and a late Stop never stops a turn its sender didn't know of. What a Stop names stays cancelled when the Stop itself is moot or gives way to a newer one, and across a restart of the host; a Stop waits for no send (a named one checks the names as its last step before the agent, a revival after a restart too) and kills only a runtime handed nothing its sender never saw, decided and retired in one step (`runtime_survival::a_named_send_stalled_before_its_runtime_never_starts`, `a_stuck_queue_send_does_not_hold_up_the_stop_naming_it`, `run_recovery::a_turn_a_stop_named_is_not_revived_after_shutdown`); one aimed at a turn a rewind removed is moot. A Stop from an older engine keeps the old rule. A send its sender had seen (its row up to the Stop's turn) is claimed like a named one, every road to the agent asks one check that reads disk and memory (a queue row sent now after a restart too), and a runtime remembers every message it was handed, with no count (`runtime_survival::a_queue_row_a_stop_named_is_not_steered_in_after_a_restart`, `a_send_the_stops_sender_saw_stalled_before_its_runtime_never_starts`, `a_stop_leaves_a_runtime_handed_an_unseen_message_before_256_named_ones`). A send a Stop claims while its run is preparing never reaches the harness, and the Stop doesn't wait for the preparation; the turn a Stop was aimed at is claimed even before its row reaches the host (`a_send_a_stop_claims_while_it_prepares_never_reaches_the_harness`, `a_queue_row_a_stop_was_aimed_at_is_not_steered_in_after_a_restart`). A Stop carries its sender's doc version (`based_on.frontier`), and what its sender saw is the rows that version holds, not the rows before its turn in the host's order: a turn started after the Stop was built is never killed or claimed by it, wherever the merge puts it; the version is read back from the doc, or from the ledger for a relayed Stop, after a restart; the per-send check is a lookup instead of a transcript walk (`a_stop_spares_a_turn_its_sender_never_saw_merged_before_its_aim`, `a_stops_version_is_read_back_from_the_doc_after_a_restart`). Whether that version held a row is read from the doc's ops (the one that gave the row its id, the ones that removed it), not from the row's map: a send the Stop saw stays stopped after its row is rewound and the host restarts, and when its row's id reaches the host after the Stop; a send whose empty row the sender held before it had an id runs (`a_send_a_stop_saw_stays_claimed_once_its_row_is_rewound_and_the_host_restarts`, `a_send_a_stop_saw_is_claimed_when_its_rows_id_arrives_after_the_stop`, `a_stop_built_before_a_row_had_its_id_lets_that_send_run`, `zend-doc stop_version`). The window's Stop says the version of the transcript it shows, not of the doc, which may hold an import not shown yet (`zend-client a_stop_says_the_version_of_the_transcript_it_was_pressed_on`), and an engine Stop whose caller named its sends carries its version too (`an_engine_stop_with_its_sends_named_says_its_version`).

## 2026-11-16

### Added
- Visuals: the window asks the chat's host which visuals are open (new read-only RPC `ListOpenVisuals {chatId}`, forwardable like `MarkVisualSent`; a host without it answers unknown method and the transcript's own visuals still pin), so an open visual is pinned before a long chat's history has loaded, and only visuals that ask something are ever opened for the pin.
- Visuals: an open visual is pinned above the composer whenever its place in the transcript is not in view, also when that place was never built (a long chat opened at the bottom, a chat opened again); it goes only when answered or closed with ×.
- Chats: Auto access mode (ported from upstream) lets the agent's own reviewer approve safe actions and ask about risky ones; Claude Code and Codex offer it, other agents don't. When Auto is unavailable for a chat the composer holds Send and says why; the engine refuses the run rather than falling back to a looser mode.
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
