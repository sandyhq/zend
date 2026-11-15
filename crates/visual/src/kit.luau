--!nocheck
-- `ui.kit`: ready-made components, each a function that takes one props
-- table and returns ordinary `ui` nodes. Nothing here draws by itself: a kit
-- function only arranges the nodes every renderer already knows, in Zend's
-- style (no boxes, theme tokens, the interface font). The rule for adding a
-- kind is in visuals/README.md ("The kit").
local ui = ...
local kit = {}

-- What a kind's error says, without a position: the wrapper raises it again
-- at the agent's call.
local function fail(message)
	error(message, 0)
end

-- `name` becomes `ui.kit.<name>`. `allowed` are its props (the array part,
-- children or a first positional value, is always allowed). An unknown prop
-- is an error, like on a node.
local function define(name, allowed, build)
	local known = {}
	for _, key in ipairs(allowed) do
		known[key] = true
	end
	kit[name] = function(props)
		if type(props) ~= "table" or rawget(props, "__kind") ~= nil then
			props = { props }
		end
		for key in pairs(props) do
			if type(key) ~= "number" and not known[key] then
				error(
					string.format(
						"ui.kit.%s has no prop '%s' (it takes %s)",
						name,
						tostring(key),
						#allowed > 0 and table.concat(allowed, ", ") or "only children"
					),
					2
				)
			end
		end
		local ok, node = pcall(build, props)
		if not ok then
			error("ui.kit." .. name .. " " .. tostring(node), 2)
		end
		return node
	end
end

-- Tones as genui names them, and the theme tokens themselves.
local TONES = {
	neutral = "muted",
	good = "success",
	warn = "warning",
	bad = "danger",
	info = "accent",
	success = "success",
	warning = "warning",
	danger = "danger",
	accent = "accent",
	muted = "muted",
}

local function tone(value)
	if value == nil then
		return nil
	end
	local token = TONES[value]
	if token == nil then
		fail("has an unknown tone '" .. tostring(value) .. "': use neutral, good, warn, bad, or info")
	end
	return token
end

local GLYPHS = { success = "✓", warning = "!", danger = "✕", accent = "●", muted = "●" }

-- `constructor(props)` with the given children appended, skipping nils: a
-- nil in a table constructor ends Luau's array part there, and every child
-- after it would vanish.
local function build(constructor, props, ...)
	for i = 1, select("#", ...) do
		local child = select(i, ...)
		if child ~= nil then
			table.insert(props, child)
		end
	end
	return constructor(props)
end

-- The array part of `props`: its children.
local function children(props)
	local out = {}
	for i, child in ipairs(props) do
		out[i] = child
	end
	return out
end

local function text(value)
	if value == nil then
		return nil
	end
	if type(value) == "table" then
		return value
	end
	return tostring(value)
end

-- A value that may be a node or plain words, as a node.
local function cell(value, options)
	if value == nil then
		return nil
	end
	if type(value) == "table" then
		return value
	end
	local node = { tostring(value) }
	for k, v in pairs(options or {}) do
		node[k] = v
	end
	return ui.text(node)
end

-- Named inputs ------------------------------------------------------------

local INPUT = { "name", "label", "description", "value", "answered" }

local function with(list, ...)
	local out = table.clone(list)
	for _, key in ipairs({ ... }) do
		table.insert(out, key)
	end
	return out
end

local function need_name(props)
	if type(props.name) ~= "string" or props.name == "" then
		fail(
			"needs name = \"...\": a fixed name, unique in the visual, that Zend keeps the "
				.. "reader's entry under (keep it the same when you update the visual)"
		)
	end
end

-- Options as strings or { value, label | title, description | detail }.
local function options(list)
	if type(list) ~= "table" or #list == 0 then
		fail("needs options = { \"...\", ... }")
	end
	local out = {}
	for i, option in ipairs(list) do
		if type(option) == "table" then
			out[i] = {
				value = option.value,
				label = option.label or option.title,
				description = option.description or option.detail,
			}
		else
			out[i] = option
		end
	end
	return out
end

local function input(constructor, props, extra)
	need_name(props)
	local node = {
		name = props.name,
		label = props.label,
		description = props.description,
		value = props.value,
		answered = props.answered,
	}
	for k, v in pairs(extra or {}) do
		node[k] = v
	end
	return constructor(node)
end

define("Input", with(INPUT, "placeholder"), function(p)
	return input(ui.field, p, { placeholder = p.placeholder })
end)

define("TextArea", with(INPUT, "placeholder"), function(p)
	return input(ui.area, p, { placeholder = p.placeholder })
end)

define("DateInput", INPUT, function(p)
	return input(ui.field, p, { placeholder = "yyyy-mm-dd" })
end)

define("Toggle", INPUT, function(p)
	return input(ui.check, p)
end)

define("Checklist", with(INPUT, "options"), function(p)
	return input(ui.checklist, p, { options = options(p.options) })
end)

define("Choice", with(INPUT, "options", "many"), function(p)
	return input(ui.choice, p, { options = options(p.options), many = p.many })
end)

define("Radio", with(INPUT, "options"), function(p)
	return input(ui.choice, p, { options = options(p.options) })
end)

define("Select", with(INPUT, "options", "placeholder"), function(p)
	return input(ui.select, p, { options = options(p.options), placeholder = p.placeholder })
end)

-- Options with a title and a line of detail each; a list `value` (or
-- many = true) picks several.
define("OptionCards", with(INPUT, "options", "many"), function(p)
	local many = p.many
	if many == nil then
		many = type(p.value) == "table"
	end
	return input(ui.choice, p, { options = options(p.options), many = many })
end)

-- Suggested next questions; the one picked goes into the answer. Brings its
-- own answer button unless answer = false (a Form keeps only its own).
define("FollowUps", { "prompts", "name", "label", "answer" }, function(p)
	local prompts = p.prompts or children(p)
	local node = ui.choice({
		name = p.name or "follow_up",
		label = p.label or "Ask next",
		options = options(prompts),
	})
	if p.answer == false then
		return node
	end
	return ui.column({ gap = 12, node, ui.answer({ label = "Ask this" }) })
end)

-- Answer buttons inside `node`, taken out (a Form puts one at its end).
local function strip_answers(node)
	if type(node) ~= "table" then
		return
	end
	for i = #node, 1, -1 do
		local child = node[i]
		if type(child) == "table" and rawget(child, "__kind") == "answer" then
			table.remove(node, i)
		else
			strip_answers(child)
		end
	end
end

-- Questions with one answer button at the end: "submit" is its label.
define("Form", { "title", "description", "submit" }, function(p)
	local body = children(p)
	strip_answers(body)
	if #body == 0 then
		fail("needs its questions as children: ui.kit.Form { ui.kit.Input { ... }, ... }")
	end
	return build(
		ui.column,
		{ gap = 16 },
		p.title and ui.text({ p.title, size = "lg", weight = 600 }),
		p.description and ui.text({ p.description, color = "muted" }),
		body,
		ui.answer({ label = p.submit })
	)
end)

-- Layout and text -----------------------------------------------------------

-- A titled section. No box: a heading over its content.
define("Card", { "title", "subtitle" }, function(p)
	return build(
		ui.column,
		{ gap = 12 },
		p.title and ui.text({ p.title, size = "lg", weight = 600 }),
		p.subtitle and ui.text({ p.subtitle, size = "sm", color = "muted" }),
		children(p)
	)
end)

-- A note to heed: a toned mark, an optional title, the text.
define("Callout", { "text", "title", "tone" }, function(p)
	local token = tone(p.tone) or "accent"
	local body = p.text or p[1]
	if body == nil then
		fail("needs text = \"...\"")
	end
	return ui.row({
		gap = 10,
		align = "start",
		ui.text({ GLYPHS[token], color = token, weight = 700 }),
		build(
			ui.column,
			{ gap = 2, grow = true },
			p.title and ui.text({ p.title, weight = 600, color = token }),
			cell(body)
		),
	})
end)

-- A one- to three-word status.
define("Badge", { "text", "tone" }, function(p)
	local words = p.text or p[1]
	if words == nil then
		fail("needs text = \"...\"")
	end
	return ui.text({ tostring(words), size = "xs", weight = 600, color = tone(p.tone) or "muted" })
end)

-- A collapsible section, closed unless open = true. The reader's toggle lasts
-- while the visual is open (by key, else by title).
local details_open = {}
define("Details", { "title", "open", "key" }, function(p)
	if p.title == nil then
		fail("needs title = \"...\"")
	end
	local id = p.key or p.title
	local open = details_open[id]
	if open == nil then
		open = p.open == true
	end
	return build(
		ui.column,
		{ gap = 8 },
		ui.row({
			gap = 8,
			hover = true,
			on_click = function()
				details_open[id] = not open
			end,
			ui.text({ open and "▾" or "▸", color = "muted" }),
			ui.text({ p.title, weight = 600 }),
		}),
		open and ui.column({ gap = 8, children(p) }) or nil
	)
end)

-- Pairs as given: a list of { key, value } keeps its order; a table of
-- key = value is sorted by key. Values may be nodes (a Badge).
define("KeyValue", { "pairs" }, function(p)
	local pairs_ = p.pairs or children(p)
	local list = {}
	if #pairs_ > 0 then
		for _, pair in ipairs(pairs_) do
			table.insert(list, { pair[1], pair[2] })
		end
	else
		for k, v in pairs(pairs_) do
			table.insert(list, { k, v })
		end
		table.sort(list, function(a, b)
			return tostring(a[1]) < tostring(b[1])
		end)
	end
	return ui.column({
		gap = 6,
		ui.map(list, function(pair)
			return ui.row({
				gap = 12,
				align = "start",
				ui.text({ tostring(pair[1]), color = "muted", width = 160 }),
				ui.row({ grow = true, cell(pair[2]) }),
			})
		end),
	})
end)

-- Rows: strings, or { title, detail?, value?, tone?, icon? } (icon: one
-- character). ordered = true numbers them.
define("List", { "items", "ordered" }, function(p)
	local items = p.items or children(p)
	return ui.column({
		gap = 10,
		ui.map(items, function(item, i)
			if type(item) ~= "table" then
				item = { title = item }
			end
			local token = tone(item.tone)
			local mark = p.ordered and (i .. ".") or item.icon or (token and "●") or "•"
			return build(
				ui.row,
				{ gap = 10, align = "start" },
				ui.text({ mark, color = token or "faint", width = p.ordered and 20 or nil }),
				build(
					ui.column,
					{ gap = 2, grow = true },
					cell(item.title, { weight = 500 }),
					cell(item.detail, { size = "sm", color = "muted" })
				),
				cell(item.value, { size = "sm", color = "muted" })
			)
		end),
	})
end)

-- Status and figures ----------------------------------------------------------

-- A sequence: { title, detail?, status? = done | current | blocked }.
local STEP = {
	done = { "✓", "success" },
	current = { "●", "accent" },
	blocked = { "!", "danger" },
}
define("Steps", { "items" }, function(p)
	local items = p.items or children(p)
	return ui.column({
		gap = 10,
		ui.map(items, function(item)
			if type(item) ~= "table" then
				item = { title = item }
			end
			local mark = STEP[item.status or ""]
			if item.status ~= nil and mark == nil then
				fail("has an unknown status '" .. tostring(item.status) .. "': use done, current, or blocked")
			end
			mark = mark or { "○", "faint" }
			return ui.row({
				gap = 10,
				align = "start",
				ui.text({ mark[1], color = mark[2], weight = 700, width = 16 }),
				build(
					ui.column,
					{ gap = 2, grow = true },
					ui.text({
						item.title,
						weight = item.status == "current" and 600 or 400,
						color = item.status == "done" and "muted" or nil,
					}),
					cell(item.detail, { size = "sm", color = item.status == "blocked" and "danger" or "muted" })
				),
			})
		end),
	})
end)

-- Health of several pieces of work.
local HEALTH = {
	["on-track"] = { "On track", "success" },
	["at-risk"] = { "At risk", "warning" },
	["off-track"] = { "Off track", "danger" },
	blocked = { "Blocked", "danger" },
	done = { "Done", "success" },
	["not-started"] = { "Not started", "faint" },
}
define("Status", { "items" }, function(p)
	local items = p.items or children(p)
	return ui.column({
		gap = 14,
		ui.map(items, function(item)
			local health = HEALTH[item.status or ""]
			if health == nil then
				fail(
					"has an unknown status '"
						.. tostring(item.status)
						.. "': use on-track, at-risk, off-track, blocked, done, or not-started"
				)
			end
			local meta = {}
			if item.owner then
				table.insert(meta, item.owner)
			end
			if item.due then
				table.insert(meta, "due " .. item.due)
			end
			if item.progress then
				table.insert(meta, math.floor(item.progress + 0.5) .. "%")
			end
			return ui.row({
				gap = 10,
				align = "start",
				ui.text({ "●", color = health[2], width = 16 }),
				build(
					ui.column,
					{ gap = 2, grow = true },
					ui.row({
						gap = 8,
						ui.text({ item.name, weight = 600, grow = true }),
						ui.text({ health[1], size = "sm", weight = 600, color = health[2] }),
					}),
					#meta > 0 and ui.text({ table.concat(meta, " · "), size = "sm", color = "muted" }) or nil,
					cell(item.note, { size = "sm", color = "muted" })
				),
			})
		end),
	})
end)

-- A labelled bar: value of max, or a percent without max.
define("Progress", { "value", "max", "label", "tone" }, function(p)
	local value = tonumber(p.value or p[1])
	if value == nil then
		fail("needs value = <number>")
	end
	local max = tonumber(p.max)
	local share = ui.clamp(value / (max or 100), 0, 1)
	local readout = max and (ui.fmt(value) .. " / " .. ui.fmt(max)) or (math.floor(share * 100 + 0.5) .. "%")
	local token = tone(p.tone) or "accent"
	return ui.canvas({
		height = 40,
		draw = function(g, w)
			if p.label then
				g:text(0, 4, p.label, { size = "sm", color = "text" })
			end
			g:text(w, 4, readout, { size = "sm", color = "muted", align = "end" })
			g:rect(0, 28, w, 6, { fill = ui.color("text", 0.12), radius = 3 })
			if share > 0 then
				g:rect(0, 28, math.max(6, w * share), 6, { fill = token, radius = 3 })
			end
		end,
	})
end)

-- A key number: label, value, a note under it. Two to four in a ui.row.
define("Metric", { "label", "value", "note", "tone" }, function(p)
	if p.value == nil then
		fail("needs value = \"...\"")
	end
	return build(
		ui.column,
		{ gap = 2 },
		ui.stat({ value = p.value, label = p.label or "", tone = tone(p.tone) }),
		p.note and ui.text({ p.note, size = "xs", color = "faint" })
	)
end)

-- A dial from min to max (0-100 by default).
define("Gauge", { "value", "min", "max", "label", "tone" }, function(p)
	local value = tonumber(p.value or p[1])
	if value == nil then
		fail("needs value = <number>")
	end
	local min, max = tonumber(p.min) or 0, tonumber(p.max) or 100
	local share = ui.clamp((value - min) / math.max(max - min, 1e-9), 0, 1)
	local token = tone(p.tone) or "accent"
	return ui.canvas({
		height = 120,
		draw = function(g, w, h)
			local r = math.min(w / 2, h - 28) - 6
			local cx, cy = w / 2, r + 8
			g:arc(cx, cy, r, math.pi, 2 * math.pi, { color = ui.color("text", 0.12), width = 10 })
			if share > 0 then
				g:arc(cx, cy, r, math.pi, math.pi * (1 + share), { color = token, width = 10 })
			end
			g:text(cx, cy - 4, ui.fmt(value), { size = "xl", weight = 600, align = "center", baseline = "end" })
			if p.label then
				g:text(cx, cy + 10, p.label, { size = "sm", color = "muted", align = "center" })
			end
		end,
	})
end)

-- Later batches (kit_c.luau, ...) add kinds to `kit` with these helpers; Rust
-- freezes it afterwards.
return kit, {
	define = define,
	fail = fail,
	tone = tone,
	build = build,
	children = children,
	cell = cell,
}
