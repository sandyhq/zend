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
- **Open visuals stay in reach.** A visual with inputs that isn't sent yet is pinned above the composer while the transcript moves on; in the transcript a line marks its place. Sent, it goes back there.
- **Update, don't ask again.** `update_visual { id, script, title? }` replaces the script in place (the same spot, the entries kept by name; a new input shows "New"). `read_visual { id }` returns the inputs, the entries, `open` and `sent`. Entries are the reader's draft until `sent`: read-only for the agent, not answers. While a visual is open, `show_visual` with its title is refused, and each child event to that chat names it ("Open visual <id> …"): keep working and put what you learn into it with `update_visual`.

## What a visual cannot do yet
A component kit (`ui.kit`), an example gallery and `visual_examples` are being built (plan `live-visuals`, [roadmap.md](roadmap.md)).
