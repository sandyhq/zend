# Visual examples and the kit

Example scripts for `show_visual`, one file per example: `visuals/<category>/<name>.luau`. G5 of plan `live-visuals` indexes them for the gallery and `visual_examples`.

## A file
It starts with four comment lines, in this order, then the script:

```lua
-- title: Release questions
-- category: inputs
-- description: A few questions before a release, with one button that puts the answers into the message.
-- tags: form, questions, answer
return function()
	return ui.kit.Form { ... }
end
```

`category` is the folder's name. Every file must pass the check `show_visual` runs: `cargo nextest run -p zend-visual kit_tests` checks all of them, their headers, and that each kit kind has an example.

## The kit
`ui.kit` (`crates/visual/src/kit.luau`) holds ready-made components. Every script sees it, in every renderer, because it is plain Luau loaded right after the `ui` prelude. A kit function follows these rules:

1. **One call with data.** `ui.kit.Kind { props }` takes one props table (the array part is its children, or its first value: `ui.kit.Badge { "Passing" }`) and returns ordinary `ui` nodes. It draws nothing itself and adds no node to Rust. A kind the existing nodes can't express waits for a node first.
2. **Zend's style.** No boxes, fills, borders or padding: a "card" is a heading over its content. Colours are theme tokens; tones are genui's `neutral, good, warn, bad, info` (muted, success, warning, danger, accent). Text uses `ui.text` sizes. Bars and dials are a `ui.canvas` with `ui.color` fills.
3. **Input from the reader goes through named inputs.** A kind that takes input builds `ui.field`, `ui.area`, `ui.check`, `ui.checklist`, `ui.choice` or `ui.select`, and needs `name`, so Zend keeps the entry and `ui.answer` collects it. Never `ui.state` for an answer: it is lost on an update and never reaches the composer.
4. **Errors like a node's.** Register it with `define(name, props, build)`: an unknown prop, and anything `build` raises with `fail(...)`, comes back as "ui.kit.<Kind> ..." at the line of the agent's call.
5. **No nils inside a table constructor.** `{ a, nil, b }` drops `b`; join children with `build(constructor, props, ...)`, which skips nils.
6. **Props follow genui** (github.com/ComicBit/bb-plugin-genui, `core/catalog.ts`) where they fit; `$state` bindings become `name`, and `detail` and `title` on options map to `description` and `label`.
7. **Each kind has** a line in `KINDS` and a case in `crates/visual/src/kit_tests.rs`, an example here, and a line in docs/visuals.md, section "Kit".
