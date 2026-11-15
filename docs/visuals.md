# Visuals

A visual is something an agent shows in a chat instead of, or beside, prose: a chart, a table, a dashboard, a diagram, a simulation, a 3D view. This page is the reference for writing one. How the runtime works inside: [mcp.md](mcp.md), "Visual replies". The authoritative, complete wording is the description of the `show_visual` tool (`SHOW_VISUAL_DESCRIPTION` in `crates/mcp/src/tools.rs`); when this page and the tool disagree, the tool is right.

## The tool
`show_visual { script, title, assets? }`, offered only to an agent Zend launched in a chat. Call it before the final text reply; the visual appears above that reply. The reader already sees it, so the reply adds only what the visual doesn't say. For an image file as it is, use `show_image { path }`.

Zend runs the script before showing it (setup, the first render, every draw, and each tab, toggle, slider and button once) and returns the first error with its line. Fix and call again; the reader sees nothing until it passes.

## A script
Luau (Lua 5.1 plus `+=`, `continue`, `if` expressions), in a sandbox: no I/O, no network, no `require`; `math.random` is seeded, so data is the same on every render. Build data and state at the top, return a view function. Zend calls the view again after every handler, so handlers only assign to state.

```lua
local s = ui.state { metric = "p95" }
local data = { { region = "us-east", p95 = 182, p99 = 240 }, { region = "eu-west", p95 = 140, p99 = 210 } }
return function()
  return ui.column { gap = 16,
    ui.text { "Latency by region", size = "lg", weight = 600 },
    ui.tabs { options = { "p95", "p99" }, value = s.metric, on_change = function(v) s.metric = v end },
    ui.chart { type = "bar", height = 220, unit = "ms",
      x = ui.map(data, function(d) return d.region end),
      series = { { name = s.metric, values = ui.map(data, function(d) return d[s.metric] end) } } },
  }
end
```

Nodes are props tables: children are the array part, strings are text, `nil` and `false` children are skipped, lists flatten. An unknown prop is an error.

## Nodes
| Node | For |
|---|---|
| `ui.column`, `ui.row`, `ui.grid` | layout: `gap`, `align`, `justify`, `wrap`, `columns`; any node takes `width`, `grow`, `key`; a row or column with `on_click` is clickable |
| `ui.text`, `ui.span` | text: `size` (xs, sm, body, lg, xl, 2xl), `weight`, `color`, `mono`, `italic`, `align`, `truncate` |
| `ui.stat` | a headline figure over a label, with `delta` and `tone` |
| `ui.table` | `columns`, `rows`; cells are strings, numbers or nodes |
| `ui.chart` | `type` bar, line, area, scatter; `x`, `series`, `stacked`, `horizontal`, `min`, `max`, `unit`, `prefix`, `decimals`, `legend`; axes and hover readout built in |
| `ui.rule`, `ui.space`, `ui.link` | divider, gap, link |
| `ui.tabs`, `ui.toggle`, `ui.slider`, `ui.button` | controls, each with a handler |
| `ui.canvas` | 2D drawing in pixels: `draw = function(g, w, h, t)`, `g:line`, `polyline`, `polygon`, `rect`, `circle`, `arc`, `text`; `g.pointer` for hover; `animate` |
| `ui.scene` | real 3D on the GPU: `s:box`, `sphere`, `cylinder`, `surface`, `mesh`, `line`, `path`, `point`, `label`, `grid`, `axes`, `model`, `environment`, `sprite`; materials `roughness`, `metal`, `glow`, `glass` |
| `ui.shader` | Shadertoy-style GLSL (`mainImage`), up to four buffer passes, named float `inputs` bound to state |

Helpers: `ui.state`, `ui.map`, `ui.range`, `ui.scale`, `ui.ticks`, `ui.fmt`, `ui.clamp`, `ui.lerp`, `ui.series`, `ui.color`, `ui.mix`. `print(...)` output comes back to the agent.

## Rules Zend enforces
- **No boxes.** Layout nodes have no fill, border, shadow, padding or font props. No cards, tiles, panels, banners or canvas backgrounds: content lies directly on the chat.
- **Theme tokens, not colours.** `text`, `muted`, `faint`, `accent`, `on-accent`, `success`, `warning`, `danger`, `border`, `chart1` to `chart6`. Hex only for a material's own colour in a scene. Never assume a background.
- **Hover must not move the page.** Anything shown only on hover keeps its space at rest or is drawn where it takes none (inside a canvas, a scene point's hint).
- **Fluid width.** The reply column is about 720px and as narrow as 360px. Charts, canvases and scenes are 160 to 420px high.

## Limits
| | |
|---|---|
| Script | 256 KB |
| Title | 200 characters |
| Assets | 16 files, 8 MB each, 10 MB together: glTF (.glb, or .gltf with everything embedded), PNG, JPEG, WebP, Radiance .hdr |
| Memory | 64 MB |
| Time | 250 ms for setup, a render or a handler; 60 ms per canvas or scene frame |

## Asking the reader
Input nodes take `name` (required, unique), `label`, `description`, `value` (the first value only) and `answered` (decided elsewhere: shown collapsed, with Change):

| Node | Entry |
|---|---|
| `ui.field`, `ui.area` | one line, several lines of text |
| `ui.check` | a boolean |
| `ui.checklist`, `ui.choice { many = true }` | the options picked |
| `ui.choice`, `ui.select` | one option |
| `ui.answer { label? }` | the button "Add to message" |

- **Entries are the reader's.** Zend keeps them beside the script, by input name, across reloads and updates.
- **The answer goes into the composer, never sent by itself.** `ui.answer` puts the inputs as words ("label: value", the title as a heading) under the draft; pressed again it replaces its own block. When the reader sends a message that still holds that block, the visual is answered: it shows "Answers sent", takes no more entries, and `read_visual` says `sent`.
- **Open visuals stay in reach.** A visual with inputs that isn't sent yet stays in its place in the transcript while that place is in view. Once the transcript moves on and its place scrolls out of view, it is pinned above the composer, and a line as tall as it was marks its place; scrolled back to that place, it returns there. The newest open visual is the one pinned. Moving between the two keeps its entries, the field being typed in and its caret. Sent, it goes back to its place for good.
- **Update, don't ask again.** `update_visual { id, script, title? }` replaces the script in place (the same spot, the entries kept by name; a new input shows "New"). `read_visual { id }` returns the inputs, the entries, `open` and `sent`. Entries are the reader's draft until `sent`: read-only for the agent, not answers. While a visual is open, `show_visual` with its title is refused, and each child event to that chat names it ("Open visual <id> …"): keep working and put what you learn into it with `update_visual`.

## Kit
`ui.kit` holds ready-made components: one call with data, built from the nodes above, so they look and behave like the rest of the visual. Unknown props are errors. Inputs need `name` and are kept like the input nodes. Tones: `neutral`, `good`, `warn`, `bad`, `info`. Examples: `visuals/` in the repo; the rule for adding a kind: `visuals/README.md`.

| Kind | What it is |
|---|---|
| `Form { title?, description?, submit?, ... }` | questions as children, ending in one answer button (`submit` its label); answer buttons inside are taken out |
| `Input { name, label?, placeholder?, value? }` | one line of text |
| `TextArea { name, label?, placeholder? }` | several lines |
| `DateInput { name, label? }` | a date as yyyy-mm-dd (a text field) |
| `Toggle { name, label, value? }` | yes or no (a check) |
| `Checklist { name, options, value? }` | options to tick |
| `Choice { name, options, many? }` | one of a few short options, or several with `many` |
| `Radio { name, options }` | one option; options `{ value, detail }` |
| `Select { name, options, placeholder? }` | one of many, in a drop-down |
| `OptionCards { name, options, value?, many? }` | options `{ value, title, detail }`; a list `value` picks several |
| `FollowUps { prompts, name?, answer? }` | suggested next questions; the pick goes into the answer; brings its own answer button unless `answer = false` |
| `Card { title?, subtitle?, ... }` | a titled section, no box |
| `Details { title, open?, ... }` | a section the reader opens and closes |
| `Callout { text, title?, tone? }` | a note to heed, with a toned mark |
| `Badge { text, tone? }` | a one- to three-word status |
| `KeyValue { pairs }` | `{ { key, value }, ... }` in order, or `{ key = value }` sorted; values may be nodes |
| `List { items, ordered? }` | rows: strings or `{ title, detail?, value?, tone?, icon? }` |
| `Steps { items }` | `{ title, detail?, status? = done, current, blocked }` |
| `Status { items }` | health: `{ name, status = on-track, at-risk, off-track, blocked, done, not-started; owner?, due?, progress?, note? }` |
| `Progress { value, max?, label?, tone? }` | a labelled bar: a percent, or value of max |
| `Metric { label, value, note?, tone? }` | a key number; two to four in a `ui.row` |
| `Gauge { value, min?, max?, label?, tone? }` | a dial, 0 to 100 by default |
| `Acceptance { criteria, columns?, title? }` | is it really done: `{ name, status = passed, failed, blocked, untested, claimed; note?, evidence?, by?, at?, method? }`; with `columns`, a grid of claimed against checked |
| `Findings { items, title? }` | review, lint or type errors `{ severity = critical…info, title, file?, line?, rule?, detail?, fix?, status? }`, sorted by severity; a patch `fix` is coloured |
| `StackTrace { trace, title? }` | an error and its stack, from the text a tool printed (any language) or `{ message, frames }`; library frames faint |
| `Dependencies { report, title? }` | outdated packages `{ name, current, latest, wanted?, severity?, type?, note? }` with how far behind |
| `Benchmark { results, unit?, title? }` | before and after `{ name, before, after, unit?, better?, stddev? }` judged by which way is better, or runs `{ name, metrics, status? }` |
| `Pipeline { run, title? }` | a CI run `{ name?, branch?, sha?, stages = { { name, status, jobs?, steps? } } }`; a failing job opens its steps |
| `Tests { results, title? }` | `{ name, status = pass, fail, skip, todo; file?, line?, ms?, message? }`: totals, failures with messages, passes folded |
| `Logs { output, command?, exit?, title? }` | text or a list of lines: levels coloured, repeats folded (×3) |
| `Coverage { report, title? }` | `{ total?, files = { { path, pct | covered, total } } }`: the total, then the least covered files first |
| `Changes { files, title? }` | files changed `{ path, added, removed, status?, diff? }`; a file with a `diff` opens to it |
| `Commits { commits, title? }` | `{ sha, title, author?, date?, files?, added?, removed? }` |
| `Diff { patch, title? }` | a unified patch with old and new line numbers, additions and removals coloured |
| `PullRequest { title, number?, state?, author?, branch?, base?, checks?, review?, … }` | state, branch, checks and review of a pull request |
| `Environments { items, title? }` | worktrees `{ name, branch, current?, threads?, changes?, pr? }` |
| `Snippet { path, code, from? }` | an excerpt with line numbers from `from` (a visual cannot read files: pass the lines) |
| `Code { code, language?, title? }` | a block of code as written |
| `Json { value, label?, open? }` | a table as a tree to open and close; `open` levels start open (1) |
| `File { path, line?, note? }` | a file path, folder faint, name bold |
| `Board { items, columns? }` | cards in status columns: `{ key?, title, status, priority?, labels?, owner?, due?, agentsWorking?, detail? }`; `columns` fixes the order |
| `Roadmap { items }` | a Gantt chart: `{ title, start, end | days, lane?, status?, progress?, after?, detail? }` or `{ title, milestone }`; dates `yyyy-mm-dd` |
| `Plan { steps, name?, label?, submit?, answer? }` | steps to approve as a checklist: `{ title, detail?, risk?, estimate?, files?, optional? }`; ends in its own answer button unless `answer = false` |
| `Decision { options, criteria, key? }` | options ranked by weighted scores; sliders try other weights; pros and cons listed |
| `Estimate { tasks, unit? }` | three-point estimates `{ title, low, likely?, high }` summed to an expected total, P50 and P90 |
| `Table { columns, rows, hover? }` | a table whose cells may be text, numbers, booleans or kit parts |
| `Timeline { items }` | `{ time?, title, detail?, tone? }` in order |
| `Calendar { events, month? }` | month grids with a dot per event and an agenda under each month: `{ date, title, tone? }` |
| `CalendarHeatmap { days, label?, tone? }` | one square per day, by value; `days` is `{ [date] = n }` or `{ { date, value? } }` |
| `Provenance { stages, expected? }` | stages from commit to live with the facts each reported; flags a fact that differs from `expected` or the stage before, and a stage with no receipt |
| `Sources { links }` | numbered links: urls or `{ url, title?, note? }` |
| `Entity { name, kind?, facts?, url?, note? }` | one product, place, person or the like with its facts |
| `Transcript { tracks }` | tracks `{ name, text | words }` compared word by word with the first; word error rate and coloured differences |
| `Walkthrough { steps, key? }` | one step at a time with Back and Next: `{ title, body?, code?, language? }` |
| `Flashcards { cards, key? }` | click a card to turn it; `{ front, back }` |
| `Chart { type, labels?, series, unit?, stacked?, height?, max? }` | bar, line, area, scatter, pie, radar, funnel, heatmap, histogram, box; series: numbers, `{ name, values, tone? }` or `{ name = values }` |
| `Sparkline { values, tone?, width?, height? }` | a tiny trend line for a table cell |
| `SegmentedBar { parts, unit? }` | shares of one whole, `{ label, value, tone? }`, with a legend |
| `Waterfall { spans, unit? }` | trace spans `{ name, start, duration, depth?, tone?, detail? }` on one time axis |
| `Treemap { items, unit?, height? }` | `{ path, size }` as nested rectangles by folder |
| `Tree { items }` | paths, or `{ label, children?, note? }`, as an outline |
| `Diagram { mermaid, details?, height? }` | a Mermaid flowchart or sequence diagram; `details` by node id shows on click |
| `Map { places, route?, height? }` | `{ name, lat, lng, note?, day? }` plotted by coordinates (no basemap, no lookups) |
| `Scene { objects \| function(t), height?, camera?, orbit?, spin?, animate? }` | a 3D scene from `{ shape = box, sphere, cylinder, surface, mesh, model, line, path, point, label, sprite, grid, axes, environment, ... }` |
| `Shader { code, inputs?, height?, ... }` | a Shadertoy-style fragment shader |
| `Carousel { key?, ... }` | slides (children), one at a time, with arrows |
| `Icon { name, tone? }` | a glyph for a common lucide icon name |

## What a visual cannot do yet
Kit kinds left out of batch c: Image, Gallery (no node shows a picture: use `show_image`) and Swatches (a canvas takes theme tokens, not hex colours). Kit kinds still to come: Slider, Rating, ListEditor, EditableTable, Quiz, Stepper, Timer (G6 leftovers), and the rest of the genui catalogue. An example gallery and `visual_examples` are being built (plan `live-visuals`, [roadmap.md](roadmap.md)).
