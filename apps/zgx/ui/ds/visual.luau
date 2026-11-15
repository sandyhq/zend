-- An agent's visual (`show_visual`) in a reply, drawn with this design
-- system. The agent's Luau script runs in the shipping app's own sandbox on
-- the host (src/visuals.rs, crates/visual): no I/O, its memory capped, each
-- call on a time budget, its randomness seeded from the visual's id. Its
-- view comes here as plain data, a tree of nodes, and each is drawn as one
-- of ours:
--
--   column, row, grid   our layout, the script's gaps and alignment; a row
--                       the reader can click or scan washes under the
--                       pointer, the wash hanging past its text
--   text                the transcript's type scale (`zend.visual.text.*`)
--   stat                ds/stat: a headline figure over its label
--   table               ds/data_table: frameless, hairlines, a row wash
--   chart               ds/chart: bars, lines, areas, scatter
--   button, toggle,     ds Button, Switch, Slider; tabs a Segmented control
--   slider, tabs        (a Select for many or long options)
--   link                the accent, opened where the app opens links
--   code                mono lines coloured by ds/syntax for their
--                       language (plain for one it doesn't know), a gutter,
--                       marked lines washed in their tone; scrolls sideways
--   answer              a Button: the inputs as words into the chat's
--                       composer, under its draft (ds/visual_answer); never
--                       sends
--   input               a named input (field, area, check, checklist,
--                       choice, select): ds TextField, TextArea, Checkbox,
--                       Radio, Select. What the reader enters is kept
--                       outside the script (`visuals.input`), so a new
--                       script for the visual keeps it; the one with focus
--                       isn't changed under the reader (`visuals.focus`).
--   rule, space         a hairline, room
--   canvas              zgx's canvas: the script's drawing, in our tokens
--   scene, shader       rendered on the GPU off this thread (the shipping
--                       app's crates/visual-gpu), shown as an image
--
-- Colours are the script's words for our tokens (ds/visual_ink), so a
-- visual follows the theme and any template. Its input goes back to the
-- script's handlers (`visuals.event`), and it is drawn again only when its
-- own tree changes shape: values, labels and colours follow in place, so a
-- slider being dragged keeps its drag while the script renders under it.
-- A script that fails, or runs past its budget, shows one quiet line, as a
-- picture that can't be shown does (its message only behind a secondary
-- click, Copy error); the transcript around it carries on.
--
--   Visual { id = "v:…", path = "/…/v_…-visual.luau", title = "Latency", device = nil }
--   (`source` in place of `path`: a fixture's script, run as is.)
--   (`api`: whose calls draw it, `"visuals"` by default; a plugin's own
--   are `"plugin.<id>.visuals"`, its SDK's `zend.visual`. zend.)

local Button = require("@self/button")
local Switch = require("@self/switch")
local Slider = require("@self/slider")
local Segmented = require("@self/segmented")
local Select = require("@self/select")
local TextField = require("@self/text_field")
local TextArea = require("@self/text_area")
local Checkbox = require("@self/checkbox")
local Radio = require("@self/radio")
local Icon = require("@self/icon")
local Stat = require("@self/stat")
local DataTable = require("@self/data_table")
local Chart = require("@self/chart")
local ink = require("@self/visual_ink")
local motion = require("@self/motion")
local status = require("@self/status")
local locale = require("@self/locale")
local Links = require("@ds/links")
local Syntax = require("@self/syntax")
local ReplyPicture = require("@self/reply_picture")
local VisualAnswer = require("@self/visual_answer")
local VisualCarry = require("@self/visual_carry")

export type Props = {
	id: string,
	-- The chat (its doc) it's in: where `ui.answer` writes.
	chat: string?,
	path: string?,
	title: string?,
	device: string?,
	source: string?,
	api: string?,
	-- Its part's update count (update_visual): the host reads it again.
	rev: number?,
	-- Its part says its answers were sent (MarkVisualSent).
	sent: boolean?,
}

-- What the host says of every visual: `{ [id] = { rev, gen } }`.
-- (Each api's, read as a visual first asks: a plugin reaches only its own.)
local REVS_BY: { [string]: any } = {}
local function revs(api: string): any
	local shared = REVS_BY[api]
	if not shared then
		shared = ui.shared(api, {} :: { [string]: { rev: number, gen: number } })
		REVS_BY[api] = shared
	end
	return shared
end

-- A canvas or scene that animates draws at most this often (ms).
local FRAME = 33

-- What can be built at a size of its own in a row; the rest fill its spare
-- width there (the shipping app's "fluid" nodes).
local FLUID = { slider = true, chart = true, canvas = true, scene = true, shader = true, image = true, table = true, rule = true, input = true }

local function raw(n: number?, why: string): any
	return if n == nil then nil else ui.raw(n, why)
end

-- The tree's shape: everything a build fixes (kinds, structure, layout),
-- none of what follows in place (text, values, labels, colours, handlers).
local FOLLOWS = {
	t = true,
	value = true,
	-- (An input's state the reader's entry changes: read in place. In the
	-- shape, the first keystroke built the visual again and its field lost
	-- the caret to the composer: plans/live-visuals fix1.)
	kept = true,
	fresh = true,
	answered = true,
	label = true,
	delta = true,
	tone = true,
	color = true,
	click = true,
	change = true,
	draw = true,
	series = true,
	x = true,
	yaxis = true,
	xaxis = true,
	fmt = true,
	xfmt = true,
	url = true,
	yaw = true,
	pitch = true,
	zoom = true,
	spin = true,
}

local function signature(v: any, out: { string })
	if type(v) ~= "table" then
		table.insert(out, tostring(v))
		return
	end
	if #v > 0 then
		table.insert(out, "[")
		for _, item in v do
			signature(item, out)
		end
		table.insert(out, "]")
		return
	end
	local keys = {}
	for k in v do
		if not FOLLOWS[k] then
			table.insert(keys, k)
		end
	end
	table.sort(keys)
	table.insert(out, "{")
	for _, k in keys do
		table.insert(out, k)
		table.insert(out, "=")
		signature(v[k], out)
	end
	-- (Handlers come and go with the shape: whether there is one counts.)
	if v.click ~= nil then
		table.insert(out, "click")
	end
	if v.spans then
		for _, s in v.spans do
			table.insert(out, if s.color then "c" else "-")
		end
	end
	table.insert(out, "}")
end

-- Every node by its path.
local function index(node: any, into: { [string]: any })
	into[node.p] = node
	for _, child in node.children or {} do
		index(child, into)
	end
	for _, row in node.rows or {} do
		for _, cell in row do
			index(cell, into)
		end
	end
end

type Ctx = {
	id: string,
	-- Whose calls (`visuals`, or a plugin's own).
	api: string,
	title: () -> string,
	-- A node by path, tracked (`at`) or not (`peek`).
	at: (path: string) -> () -> any,
	peek: (path: string) -> any,
	-- Every render's number: canvases draw again with it.
	gen: () -> number,
	event: (handler: number?, arg: any?) -> (),
	-- What the reader entered in a named input (nil clears it), and the
	-- input that has focus (nil: none).
	input: (name: string, value: any) -> (),
	focus: (name: string?) -> (),
	-- Its inputs as words into its chat's composer (`ui.answer`).
	answer: () -> (),
	-- The person sent its answers: it asks no more, its entries stay.
	sent: () -> boolean,
	-- A failure while drawing (a canvas's, a scene's), shown under it.
	drew: (error: string?) -> (),
}

local build: (n: any, ctx: Ctx, parent: string) -> Node

-- ---- text -------------------------------------------------------------------

local ALIGN_TEXT = { start = "start", center = "center", ["end"] = "right" }

local function text(n: any, ctx: Ctx): Node
	local get = ctx.at(n.p)
	local size, line = ink.size(n.size)
	local props: { [any]: any } = {
		literal = true,
		wrap = not n.truncate,
		truncate = n.truncate,
		lines = if n.truncate then 1 else nil,
		min_w = 0,
		text_size = size,
		line_height = line,
		weight = ink.WEIGHTS[n.weight] or "$font.weight.regular",
		font = if n.mono then "$font.mono" else nil,
		italic = n.italic,
		align = ALIGN_TEXT[n.align] or "start",
		color = function(): any
			local now = get()
			return ink.paint(now and now.color)
		end,
	}
	local spans = n.spans or {}
	local plain = #spans == 1 and not spans[1].weight and not spans[1].color and not spans[1].mono and not spans[1].italic
	if plain or #spans == 0 then
		props[1] = function(): string
			local now = get()
			local s = now and now.spans and now.spans[1]
			return if s then s.t else ""
		end
		return ui.text(props)
	end
	for i, s in spans do
		props[i] = ui.span {
			function(): string
				local now = get()
				local live = now and now.spans and now.spans[i]
				return if live then live.t else ""
			end,
			weight = if s.weight then ink.WEIGHTS[s.weight] else nil,
			font = if s.mono then "$font.mono" else nil,
			italic = s.italic,
			color = if s.color
				then function(): any
					local now = get()
					local live = now and now.spans and now.spans[i]
					return ink.paint(live and live.color)
				end
				else nil,
		}
	end
	return ui.rich(props)
end

-- ---- controls -------------------------------------------------------------------

-- A control's own value, kept in step with the script's: the tree's value
-- becomes the control's, and the reader's change goes to the handler.
local function synced(get: () -> any, peek: () -> any, ctx: Ctx, initial: any): State<any>
	local value = ui.use_state(initial)
	ui.effect(function()
		local now = get()
		if now and now.value ~= nil and now.value ~= value:peek() then
			value:set(now.value)
		end
	end)
	ui.effect(function()
		local v = value:get()
		local now = peek()
		if now and v ~= now.value then
			ctx.event(now.change, v)
		end
	end)
	return value
end

local Tabs = ui.component(function(props: { n: any, ctx: Ctx }): Node
	local n, ctx = props.n, props.ctx
	local get, peek = ctx.at(n.p), function()
		return ctx.peek(n.p)
	end
	local value = synced(get, peek, ctx, n.value)
	local options = {}
	local long = #n.options > 4
	for _, o in n.options do
		table.insert(options, { value = o, label = o })
		if #o > 14 then
			long = true
		end
	end
	if long then
		return Select { value = value, options = options, size = "sm", name = ctx.title() }
	end
	return Segmented { value = value, options = options, label = ctx.title() }
end)

local Toggle = ui.component(function(props: { n: any, ctx: Ctx }): Node
	local n, ctx = props.n, props.ctx
	local get, peek = ctx.at(n.p), function()
		return ctx.peek(n.p)
	end
	local value = synced(get, peek, ctx, n.value == true)
	local function label(): string
		local now = get()
		return if now then now.label or "" else ""
	end
	return ui.row {
		class = "visual-toggle",
		items = "center",
		gap = "$space.3",
		Switch { value = value, label = label() },
		ui.text {
			label,
			literal = true,
			cursor = "pointer",
			-- (The switch is named by it: one control, not two.)
			aria_hidden = true,
			text_size = "$zend.visual.text.body.size",
			line_height = "$zend.visual.text.body.line",
			color = "$text.primary",
			on_click = function()
				value:toggle()
			end,
		},
	}
end)

-- ---- named inputs ---------------------------------------------------------------

-- (Deep: a `ui.rows` value is a list of rows, each a list of cells.)
local function same(a: any, b: any): boolean
	if type(a) == "table" and type(b) == "table" then
		if #a ~= #b then
			return false
		end
		for i, v in a do
			if not same(v, b[i]) then
				return false
			end
		end
		return true
	end
	return a == b
end

local function copy(list: any): { any }
	local out = {}
	if type(list) == "table" then
		for _, v in list do
			table.insert(out, if type(v) == "table" then copy(v) else v)
		end
	end
	return out
end

local IconButton = require("@self/icon_button").IconButton

-- A number as its entry holds it: no more places than its step has.
local function number_text(v: number, step: number?): string
	local places = 0
	if step then
		local s = tostring(step)
		local dot = string.find(s, ".", 1, true)
		if dot then
			places = math.min(#s - dot, 6)
		end
	end
	local t = string.format("%." .. places .. "f", v)
	if places > 0 then
		t = (string.gsub(t, "0+$", ""))
		t = (string.gsub(t, "%.$", ""))
	end
	return t
end

-- One text of a list input: an item of `ui.items`, or a cell (`j`) of a
-- row of `ui.rows`; its field's own state kept in step with the list.
local Cell = ui.component(function(props: {
	list: State<any>,
	i: number,
	j: number?,
	kind: string?,
	name: string,
	on_focus: (on: boolean) -> (),
}): Node
	local list, i, j = props.list, props.i, props.j
	local function at(l: any): string
		local v = if type(l) == "table" then l[i] else nil
		if j then
			v = if type(v) == "table" then v[j] else nil
		end
		return if type(v) == "string" then v else ""
	end
	local text = ui.use_state(at(list:peek()))
	ui.effect(function()
		local now = at(list:get())
		if now ~= text:peek() then
			text:set(now)
		end
	end)
	ui.effect(function()
		local v = text:get()
		local l = list:peek()
		if v ~= at(l) and type(l) == "table" and l[i] ~= nil then
			local next = copy(l)
			if j then
				next[i][j] = v
			else
				next[i] = v
			end
			list:set(next)
		end
	end)
	if props.kind == "boolean" then
		return Checkbox {
			value = function(): boolean
				return text:get() == "true"
			end,
			on_change = function(on: boolean)
				text:set(if on then "true" else "false")
			end,
			name = props.name,
			bare = true,
		}
	end
	return TextField {
		value = text,
		name = props.name,
		size = "sm",
		w_full = true,
		inputmode = if props.kind == "number" then "decimal" else nil,
		on_focus_change = props.on_focus,
	}
end)

-- Indices 1..#list, for `ui.each` over a list input's items or rows.
local function indices(list: State<any>): any
	return ui.computed(function(): { number }
		local out = {}
		for i = 1, #(list:get() or {}) do
			table.insert(out, i)
		end
		return out
	end)
end

-- `ui.number`: a slider, a stepper, or stars. Its entry is the number's
-- digits ("" for stars not picked yet).
local function number_control(n: any, value: State<any>): Node
	local step = n.step or 1
	local lo, hi = n.min, n.max
	local function now(): number?
		return tonumber(value:get())
	end
	local function put(v: number)
		if lo then
			v = math.max(v, lo)
		end
		if hi then
			v = math.min(v, hi)
		end
		value:set(number_text(v, step))
	end
	if n.style == "rating" then
		local stars = {}
		for k = 1, hi or 5 do
			table.insert(stars, IconButton {
				icon = function(): any
					return if (now() or 0) >= k then "star-bold" else "star"
				end,
				lit = function(): boolean
					return (now() or 0) >= k
				end,
				label = (n.label or n.name) .. ": " .. k .. " / " .. (hi or 5),
				size = "sm",
				on_click = function()
					put(k)
				end,
			})
		end
		return ui.row { items = "center", gap = "$space.1", table.unpack(stars) }
	elseif n.style == "stepper" then
		return ui.row {
			items = "center",
			gap = "$space.2",
			IconButton {
				icon = "minus",
				label = locale.s("ds.visual.input.less"),
				size = "sm",
				on_click = function()
					put((now() or lo or 0) - step)
				end,
			},
			ui.text {
				function(): string
					local v = value:get()
					return if v == nil or v == "" then "–" else v
				end,
				literal = true,
				min_w = 24,
				align = "center",
				text_size = "$zend.visual.text.body.size",
				line_height = "$zend.visual.text.body.line",
				weight = "$font.weight.medium",
				color = "$text.primary",
			},
			IconButton {
				icon = "plus",
				label = locale.s("ds.visual.input.more"),
				size = "sm",
				on_click = function()
					put((now() or lo or 0) + step)
				end,
			},
		}
	end
	local num = ui.use_state(tonumber(value:peek()) or lo or 0)
	ui.effect(function()
		local v = tonumber(value:get())
		if v and v ~= num:peek() then
			num:set(v)
		end
	end)
	ui.effect(function()
		local v = num:get()
		if tonumber(value:peek()) ~= v then
			value:set(number_text(v, step))
		end
	end)
	return Slider {
		value = num,
		min = lo or 0,
		max = hi or 100,
		step = step,
		label = n.label or n.name,
		w_full = true,
		format = function(v: number): string
			return number_text(v, step)
		end,
	}
end

-- `ui.items`: a field per item with a way to remove it, and one to add.
local function items_control(n: any, value: State<any>, on_focus: (boolean) -> ()): Node
	local new = ui.use_state("")
	local function add()
		local text = new:peek()
		if string.gsub(text, "%s", "") ~= "" then
			local next = copy(value:peek())
			table.insert(next, text)
			value:set(next)
			new:set("")
		end
	end
	return ui.column {
		gap = "$space.1",
		w_full = true,
		ui.each(indices(value), function(i: number): Node
			return ui.row {
				items = "center",
				gap = "$space.1",
				w_full = true,
				Cell { list = value, i = i, name = (n.label or n.name) .. " " .. i, on_focus = on_focus },
				IconButton {
					icon = "close",
					label = locale.s("ds.visual.input.remove"),
					size = "sm",
					tone = "quiet",
					on_click = function()
						local next = copy(value:peek())
						table.remove(next, i)
						value:set(next)
					end,
				},
			}
		end, { w_full = true, gap = "$space.1" }),
		ui.row {
			items = "center",
			gap = "$space.1",
			w_full = true,
			TextField {
				value = new,
				placeholder = n.placeholder or locale.s("ds.visual.input.add"),
				name = n.placeholder or locale.s("ds.visual.input.add"),
				size = "sm",
				w_full = true,
				on_commit = function()
					add()
				end,
			},
			IconButton { icon = "plus", label = locale.s("ds.visual.input.add"), size = "sm", on_click = add },
		},
	}
end

-- `ui.rows`: the columns' names, a row of fields (a box for a yes/no
-- column) per row with a way to remove it, and a way to add one.
local function rows_control(n: any, value: State<any>, on_focus: (boolean) -> ()): Node
	local columns = n.columns or {}
	local head = { items = "center", gap = "$space.1", w_full = true }
	for _, c in columns do
		table.insert(head, ui.text {
			c.label,
			literal = true,
			truncate = true,
			grow = if c.type == "boolean" then nil else 1,
			min_w = if c.type == "boolean" then 56 else 0,
			text_size = "$zend.visual.text.sm.size",
			line_height = "$zend.visual.text.sm.line",
			color = "$text.secondary",
		})
	end
	table.insert(head, ui.row { w = 22 })
	return ui.column {
		gap = "$space.1",
		w_full = true,
		ui.row(head),
		ui.each(indices(value), function(i: number): Node
			local cells = { items = "center", gap = "$space.1", w_full = true }
			for j, c in columns do
				table.insert(cells, ui.row {
					grow = if c.type == "boolean" then nil else 1,
					min_w = if c.type == "boolean" then 56 else 0,
					Cell { list = value, i = i, j = j, kind = c.type, name = c.label .. " " .. i, on_focus = on_focus },
				})
			end
			table.insert(cells, IconButton {
				icon = "close",
				label = locale.s("ds.visual.input.remove_row"),
				size = "sm",
				tone = "quiet",
				on_click = function()
					local next = copy(value:peek())
					table.remove(next, i)
					value:set(next)
				end,
			})
			return ui.row(cells)
		end, { w_full = true, gap = "$space.1" }),
		ui.row {
			Button {
				tone = "ghost",
				size = "sm",
				on_click = function()
					local next = copy(value:peek())
					local row = {}
					for _, c in columns do
						table.insert(row, if c.type == "boolean" then "false" else "")
					end
					table.insert(next, row)
					value:set(next)
				end,
				locale.s("ds.visual.input.add_row"),
			},
		},
	}
end

-- Words over a group of options: its label, "New" while it's new since
-- the agent's last script, and what it says more.
local function heading(n: any, get: () -> any): Node
	return ui.column {
		gap = "$space.1",
		ui.row {
			items = "center",
			gap = "$space.2",
			ui.text {
				n.label or n.name,
				literal = true,
				text_size = "$zend.visual.text.body.size",
				line_height = "$zend.visual.text.body.line",
				weight = "$font.weight.medium",
				color = "$text.primary",
			},
			ui.show(function(): boolean
				local now = get()
				return now ~= nil and now.fresh == true
			end, ui.text {
				class = "visual-input-new",
				locale.s("ds.visual.input.new"),
				literal = true,
				text_size = "$zend.visual.text.sm.size",
				line_height = "$zend.visual.text.sm.line",
				color = "$accent",
			}),
		},
		if n.description
			then ui.text {
				n.description,
				literal = true,
				text_size = "$zend.visual.text.sm.size",
				line_height = "$zend.visual.text.sm.line",
				color = "$text.secondary",
			}
			else nil,
	}
end

local Input = ui.component(function(props: { n: any, ctx: Ctx }): Node
	local n, ctx = props.n, props.ctx
	local get = ctx.at(n.p)
	local name = n.name
	local focused = false
	local input_ref = ui.ref()
	if n.kind == "field" or n.kind == "area" then
		-- Built in a new place while it had the keys (ds/visual_carry): it
		-- takes them back, its selection too. (Once mounted: after this frame.)
		ui.after(0, function()
			local el = input_ref:peek()
			local c = el and VisualCarry.take(ctx.id, name)
			if c then
				el:focus()
				if c.a then
					pcall(function()
						el:set_selection(c.a, c.b)
					end)
				end
			end
		end)
	end
	local initial = n.value
	if type(initial) == "table" then
		initial = copy(initial)
	end
	local value = ui.use_state(initial)
	-- The tree's value (the reader's entry, else the script's) becomes the
	-- control's, except under the reader's caret.
	ui.effect(function()
		local now = get()
		if now and not focused and not same(now.value, value:peek()) then
			value:set(if type(now.value) == "table" then copy(now.value) else now.value)
		end
	end)
	-- The reader's change is kept, outside the script.
	ui.effect(function()
		local v = value:get()
		local now = ctx.peek(n.p)
		if now and not same(v, now.value) then
			ctx.input(name, v)
		end
	end)
	local reopened = ui.use_state(false)
	local kind = n.kind
	local options = {}
	for _, o in n.options or {} do
		table.insert(options, { value = o.value, label = o.label, description = o.description })
	end
	local control: Node
	if kind == "field" then
		control = TextField {
			value = value,
			label = n.label,
			placeholder = n.placeholder,
			hint = n.description,
			name = n.label or name,
			w_full = true,
			input_ref = input_ref,
			on_focus_change = function(on: boolean)
				focused = on
				VisualCarry.hold(ctx.id, name, if on then input_ref:peek() else nil)
				ctx.focus(if on then name else nil)
			end,
		}
	elseif kind == "area" then
		control = TextArea {
			value = value,
			label = n.label,
			placeholder = n.placeholder,
			hint = n.description,
			name = n.label or name,
			rows = 3,
			w_full = true,
			input_ref = input_ref,
			on_focus_change = function(on: boolean)
				focused = on
				VisualCarry.hold(ctx.id, name, if on then input_ref:peek() else nil)
				ctx.focus(if on then name else nil)
			end,
		}
	elseif kind == "check" then
		control = Checkbox {
			value = function(): boolean
				return value:get() == true
			end,
			on_change = function(on: boolean)
				value:set(on)
			end,
			label = n.label or name,
			description = n.description,
		}
	elseif kind == "select" then
		control = Select {
			value = value,
			options = options,
			label = n.label,
			placeholder = n.placeholder,
			hint = n.description,
			name = n.label or name,
			size = "sm",
		}
	elseif kind == "number" then
		control = ui.column { gap = "$space.2", w_full = true, heading(n, get), number_control(n, value) }
	elseif kind == "items" or kind == "rows" then
		local function on_focus(on: boolean)
			focused = on
			ctx.focus(if on then name else nil)
		end
		control = ui.column {
			gap = "$space.2",
			w_full = true,
			heading(n, get),
			if kind == "items" then items_control(n, value, on_focus) else rows_control(n, value, on_focus),
		}
	elseif n.many then
		-- A checklist, or a choice of several: a box per option.
		local boxes = { gap = "$space.1" }
		for _, o in options do
			local option = o.value
			table.insert(boxes, Checkbox {
				value = function(): boolean
					return table.find(value:get() or {}, option) ~= nil
				end,
				on_change = function(on: boolean)
					local next = {}
					for _, o2 in options do
						local was = table.find(value:peek() or {}, o2.value) ~= nil
						if (o2.value == option and on) or (o2.value ~= option and was) then
							table.insert(next, o2.value)
						end
					end
					value:set(next)
				end,
				label = o.label,
				description = o.description,
			})
		end
		control = ui.column { gap = "$space.2", heading(n, get), ui.column(boxes) }
	else
		control = ui.column {
			gap = "$space.2",
			heading(n, get),
			Radio { value = value, options = options, label = n.label or name },
		}
	end
	-- One with no heading of its own says it's new beside it.
	local own_heading = kind == "checklist"
		or kind == "choice"
		or kind == "number"
		or kind == "items"
		or kind == "rows"
	local body = if own_heading
		then control
		else ui.column {
			gap = "$space.1",
			w_full = true,
			control,
			ui.show(function(): boolean
				local now = get()
				return now ~= nil and now.fresh == true
			end, ui.text {
				class = "visual-input-new",
				locale.s("ds.visual.input.new"),
				literal = true,
				text_size = "$zend.visual.text.sm.size",
				line_height = "$zend.visual.text.sm.line",
				color = "$accent",
			}),
		}
	-- Answered elsewhere: the answer, collapsed, with a way back in.
	return ui.column {
		class = "visual-input",
		w_full = true,
		min_w = 0,
		gap = "$space.2",
		ui.show(function(): boolean
			local now = get()
			return now ~= nil and now.answered ~= nil and not reopened:get()
		end, ui.row {
			class = "visual-input-answered",
			items = "center",
			gap = "$space.2",
			ui.text {
				function(): string
					local now = get()
					return (n.label or name) .. ": " .. (if now and now.answered then now.answered else "")
				end,
				literal = true,
				truncate = true,
				text_size = "$zend.visual.text.body.size",
				line_height = "$zend.visual.text.body.line",
				color = "$text.secondary",
			},
			ui.text {
				locale.s("ds.visual.input.change"),
				literal = true,
				role = "button",
				cursor = "pointer",
				text_size = "$zend.visual.text.sm.size",
				line_height = "$zend.visual.text.sm.line",
				color = "$accent",
				on_click = function()
					reopened:set(true)
				end,
			},
		}),
		ui.show(function(): boolean
			local now = get()
			return now == nil or now.answered == nil or reopened:get()
		end, body),
	}
end)

local VisualSlider = ui.component(function(props: { n: any, ctx: Ctx }): Node
	local n, ctx = props.n, props.ctx
	local get, peek = ctx.at(n.p), function()
		return ctx.peek(n.p)
	end
	local value = synced(get, peek, ctx, n.value)
	local function label(): string
		local now = get()
		return if now then now.label or "" else ""
	end
	return ui.column {
		class = "visual-slider",
		w_full = true,
		min_w = 0,
		gap = "$space.1",
		ui.show(
			function(): boolean
				return label() ~= ""
			end,
			ui.text {
				label,
				literal = true,
				truncate = true,
				text_size = "$zend.visual.text.sm.size",
				line_height = "$zend.visual.text.sm.line",
				color = "$text.secondary",
			}
		),
		Slider {
			value = value,
			min = n.min,
			max = n.max,
			step = n.step,
			label = n.label or ctx.title(),
			w_full = true,
			format = function(v: number): string
				local now = peek()
				return ink.format(v, now and now.fmt)
			end,
		},
	}
end)

-- ---- when it can't be shown ----------------------------------------------------

-- A visual that can't be shown, or stopped: one quiet line, as a picture
-- that can't be shown has (ds/reply_picture `Line`), nothing raw in the
-- reply. A reason a reader can use (plain words, not a script's fault) is
-- its tooltip; the script's own message is a secondary click away (Copy
-- error), for whoever is fixing it. A new message makes a new line.
-- (`stopped`: it shows, its last good view above; it stopped answering.)
local function problem(message: () -> string, stopped: boolean?): Node
	return ui.presence(message, function(raw: string): Node
		return ui.row {
			class = "visual-error",
			w_full = true,
			context_menu = if raw ~= ""
				then function(): { Node }?
					return {
						ui.menu_item {
							label = locale.t("ds.visual.copy_error"),
							on_click = function()
								ui.clipboard.write(raw)
							end,
						},
					}
				end
				else nil,
			ReplyPicture.Line {
				icon = "widget",
				words = locale.s(if stopped then "ds.visual.stopped" else "ds.transcript.visual_failed"),
				reason = ReplyPicture.plain(raw),
			},
		}
	end, { w_full = true })
end

-- ---- canvases -------------------------------------------------------------------

local function text_size(name: string?): number
	local size = ink.size(name)
	local v = ui.theme.get(string.sub(size, 2))
	return if type(v) == "number" then v else 15
end

local WEIGHT = { regular = 400, medium = 500, semibold = 600, bold = 700 }
local BASELINE = { start = "top", center = "middle", ["end"] = "bottom" }

-- The script's drawing on our canvas: its fills, strokes and text in our
-- tokens. (Polygons and polylines, as crates/visual records them.)
local function paint(c: any, cmds: { any })
	for _, cmd in cmds do
		local k = cmd.k
		-- (Circles, rectangles and lines as zgx draws them on the GPU: no
		-- path built, nothing rasterized.)
		if k == "c" then
			c:circle(cmd.x, cmd.y, cmd.r, ink.canvas(cmd.c))
		elseif k == "r" then
			c:rect(cmd.x, cmd.y, cmd.w, cmd.h, ink.canvas(cmd.c))
		elseif k == "l" then
			local pts = cmd.pts
			c:line(pts[1], pts[2], pts[3], pts[4], ink.canvas(cmd.c), cmd.w or 1, {
				cap = "round",
				dash = if cmd.dash then { cmd.dash, cmd.dash } else nil,
			})
		elseif k == "m" then
			-- A curve in runs whose x doesn't turn back (src/visuals.rs): each
			-- one zgx's canvas strokes on the GPU. Where two runs meet the
			-- curve runs straight up or down, so their square-cut ends close
			-- on each other with no seam and no doubled ink.
			local paint = ink.canvas(cmd.c)
			local runs = cmd.runs or {}
			local cap = if #runs == 1 then "round" else "butt"
			for _, pts in runs do
				if #pts >= 4 then
					local p = ui.path():move(pts[1], pts[2])
					for i = 3, #pts - 1, 2 do
						p = p:line(pts[i], pts[i + 1])
					end
					c:stroke(p, paint, cmd.w or 1, { join = "round", cap = cap })
				end
			end
		elseif k == "t" then
			c:text(cmd.t, cmd.x, cmd.y, {
				size = text_size(cmd.size),
				weight = WEIGHT[cmd.weight] or 400,
				color = ink.canvas(cmd.c),
				font = if cmd.mono then "mono" else nil,
				align = cmd.align,
				baseline = BASELINE[cmd.base] or "middle",
			})
		else
			local pts = cmd.pts
			if #pts >= 4 then
				local p = ui.path():move(pts[1], pts[2])
				for i = 3, #pts - 1, 2 do
					p = p:line(pts[i], pts[i + 1])
				end
				if k == "f" or cmd.closed then
					p = p:close()
				end
				if k == "f" then
					c:fill(p, ink.canvas(cmd.c))
				else
					c:stroke(p, ink.canvas(cmd.c), cmd.w or 1, {
						join = "round",
						cap = "round",
						dash = if cmd.dash then { cmd.dash, cmd.dash } else nil,
					})
				end
			end
		end
	end
end

-- Whether the window shows (not hidden, minimized or covered); true where
-- a host doesn't say.
local function window_shown(): boolean
	local ok, shown = pcall(function()
		return ui.window.shown:get()
	end)
	return not ok or shown ~= false
end

-- Seconds since `start`, stepped while something animates, only while it
-- shows (`visible`: on screen, in a window that shows), and holding still
-- with motion reduced. Nothing ticks for one scrolled away.
local function clock(animates: () -> boolean, visible: State<boolean>): State<number>
	local started = ui.now()
	local t = ui.use_state(0)
	ui.effect(function()
		if animates() and visible:get() and window_shown() and not motion.reduced() then
			ui.every(FRAME, function()
				t:set((ui.now() - started) / 1000)
			end)
		end
	end)
	return t
end

local Canvas = ui.component(function(props: { n: any, ctx: Ctx }): Node
	local n, ctx = props.n, props.ctx
	local get = ctx.at(n.p)
	local pointer = ui.use_state(nil :: any)
	local held = false
	-- Whether the drawing looked at the pointer (it draws again as it moves
	-- only then).
	local reads_pointer = false
	-- (Told at its first layout, and as it comes and goes: nothing moves
	-- before it's seen.)
	local visible = ui.use_state(false)
	local t = clock(function(): boolean
		local now = get()
		return now ~= nil and now.animate == true
	end, visible)
	local function at(e: any): any
		return { x = e.local_x, y = e.local_y, down = held }
	end
	return ui.canvas {
		class = "visual-canvas",
		w_full = true,
		h = n.h,
		label = ctx.title(),
		cursor = if n.click then "pointer" else nil,
		on_visible = function(shows: boolean)
			visible:set(shows)
		end,
		draw = function(c: any, w: number, h: number)
			ctx.gen()
			local now = get()
			if not now then
				return
			end
			local p = if reads_pointer then pointer:get() else pointer:peek()
			local arg = { id = ctx.id, h = now.draw, w = w, ht = h, t = if now.animate then t:get() else 0 }
			if p then
				arg.px, arg.py, arg.down = p.x, p.y, p.down
			end
			local ok, drawn = pcall(ui.call_sync, ctx.api .. ".draw", arg)
			if not ok or type(drawn) ~= "table" then
				return
			end
			if drawn.error then
				ctx.drew(drawn.error)
				return
			end
			ctx.drew(nil)
			reads_pointer = drawn.pointer == true
			paint(c, drawn.cmds or {})
		end,
		on_pointer_move = function(e: any)
			pointer:set(at(e))
		end,
		on_hover = function(inside: boolean)
			if not inside and not held then
				pointer:set(nil)
			end
		end,
		on_press = function(e: any)
			held = true
			pointer:set(at(e))
		end,
		on_release = function(e: any)
			held = false
			pointer:set(at(e))
		end,
		-- (Only one the script hears is a control.)
		on_click = if n.click
			then function(e: any)
				local now = ctx.peek(n.p)
				if now and now.click then
					local p = pointer:peek()
					ctx.event(now.click, { x = e.local_x or (p and p.x) or 0, y = e.local_y or (p and p.y) or 0 })
				end
			end
			else nil,
	}
end)

-- A scene or shader: its frames rendered on the host's GPU thread and shown
-- as an image, one asked for at a time (a newer wish waits for it). The
-- camera turns under a drag; a scene's labels are set as our text over it,
-- a hint under the pointer said in the corner.
local Surface = ui.component(function(props: { n: any, ctx: Ctx }): Node
	local n, ctx = props.n, props.ctx
	local get = ctx.at(n.p)
	local box = ui.ref()
	local src = ui.use_state("")
	local labels = ui.use_state({} :: { any })
	local hints: { any } = {}
	local hint = ui.use_state("")
	local failed = ui.use_state("")
	local yaw = ui.use_state(n.yaw or 0)
	local pitch = ui.use_state(n.pitch or 0)
	local mouse = ui.use_state(nil :: any)
	local drag: any = nil
	local function spinning(): boolean
		local now = get()
		return now ~= nil and (now.spin or 0) ~= 0 and drag == nil
	end
	-- (Told at its first layout, and as it comes and goes: nothing moves
	-- before it's seen.)
	local visible = ui.use_state(false)
	local t = clock(function(): boolean
		local now = get()
		return now ~= nil and (now.animate == true or spinning())
	end, visible)
	local busy, again = false, false
	local function request()
		if busy then
			again = true
			return
		end
		-- (Off screen, nothing is rendered: it asks again as it comes back.)
		if not visible:peek() then
			return
		end
		local now = ctx.peek(n.p)
		local w = box.width or 0
		if not now or w <= 0 then
			return
		end
		busy = true
		local time = t:peek()
		local spin = if motion.reduced_now() then 0 else (now.spin or 0)
		local arg = {
			id = ctx.id,
			p = n.p,
			w = w,
			ht = now.h,
			scale = 2,
			t = time,
			pal = ink.palette(),
			yaw = yaw:peek() + (if drag then 0 else spin * time),
			pitch = pitch:peek(),
			mouse = mouse:peek(),
		}
		local ok, promise = pcall(ui.call, ctx.api .. ".frame", arg)
		if not ok or not promise then
			busy = false
			failed:set(if ok then "" else tostring(promise))
			return
		end
		promise:after(function(got: any)
			busy = false
			if type(got) == "table" then
				failed:set("")
				src:set(got.src or "")
				labels:set(got.labels or {})
				hints = got.hints or {}
			end
			if again then
				again = false
				request()
			end
		end, function(err: any)
			busy = false
			again = false
			failed:set(tostring(err))
		end)
	end
	-- A frame for every change it shows: a render, its size, the theme, its
	-- coming into view; then each tick while it moves (which takes the
	-- camera and pointer as they are), else each move of the camera or the
	-- pointer (a shader's) while it's still. At most one frame a tick.
	ui.effect(function()
		ctx.gen()
		local _ = box.width, ui.theme.current:get(), visible:get()
		local now = get()
		if now and (now.animate or spinning()) then
			t:get()
		else
			local _ = yaw:get(), pitch:get(), mouse:get()
		end
		request()
	end)
	ui.on_cleanup(function()
		pcall(ui.invoke, ctx.api .. ".release", { id = ctx.id, p = n.p })
	end)
	local function hint_at(x: number, y: number): string
		for _, h in hints do
			local dx, dy = x - h.x, y - h.y
			if dx * dx + dy * dy <= (h.r or 8) ^ 2 then
				return h.t
			end
		end
		return ""
	end
	local size_xs = text_size("xs")
	return ui.column {
		class = if n.k == "scene" then "visual-scene" else "visual-shader",
		w_full = true,
		gap = "$space.2",
		ui.overlay {
			ref = box,
			w_full = true,
			h = n.h,
			cursor = if n.orbit then "grab" else nil,
			label = ctx.title(),
			on_visible = function(shows: boolean)
				visible:set(shows)
			end,
			on_press = function(e: any)
				local spin = if motion.reduced_now() then 0 else ((get() or {}).spin or 0)
				-- (Where the turntable had got to is where the drag starts.)
				yaw:set(yaw:peek() + spin * t:peek())
				drag = { x = e.local_x, y = e.local_y, yaw = yaw:peek(), pitch = pitch:peek() }
				if n.k == "shader" then
					mouse:set({ x = e.local_x, y = e.local_y, down = true, sx = e.local_x, sy = e.local_y })
				end
			end,
			on_pointer_move = function(e: any)
				if drag and n.orbit then
					yaw:set(drag.yaw + (e.local_x - drag.x) * 0.01)
					pitch:set(math.clamp(drag.pitch + (e.local_y - drag.y) * 0.01, -1.5, 1.5))
				end
				if n.k == "shader" then
					local m = mouse:peek()
					mouse:set({ x = e.local_x, y = e.local_y, down = drag ~= nil, sx = m and m.sx, sy = m and m.sy })
				else
					hint:set(hint_at(e.local_x, e.local_y))
				end
			end,
			on_release = function()
				drag = nil
				-- The turntable goes on from where the drag left it.
				if spinning() then
					local spin = if motion.reduced_now() then 0 else ((get() or {}).spin or 0)
					yaw:set(yaw:peek() - spin * t:peek())
				end
				local m = mouse:peek()
				if m then
					mouse:set({ x = m.x, y = m.y, down = false })
				end
			end,
			on_hover = function(inside: boolean)
				if not inside then
					hint:set("")
				end
			end,
			ui.image {
				src = function(): string
					return src:get()
				end,
				w_full = true,
				h_full = true,
				-- (A frame drawn for an earlier size, while the box resizes:
				-- covered, never stretched.)
				fit = "cover",
				label = ctx.title(),
			},
			ui.canvas {
				w_full = true,
				h_full = true,
				pointer_events = "none",
				draw = function(c: any)
					for _, l in labels:get() do
						c:text(l.t, l.x, l.y, {
							size = text_size(l.size),
							color = ink.canvas(l.c),
							align = "center",
							baseline = "bottom",
						})
					end
				end,
			},
			ui.show(
				function(): boolean
					return hint:get() ~= ""
				end,
				ui.text {
					function(): string
						return hint:get()
					end,
					literal = true,
					absolute = true,
					left = 0,
					top = 0,
					pointer_events = "none",
					text_size = size_xs,
					weight = "$font.weight.medium",
					color = "$text.primary",
				}
			),
		},
		ui.show(
			function(): boolean
				return failed:get() ~= ""
			end,
			problem(function(): string
				return failed:get()
			end)
		),
	}
end)

-- ---- picture ----------------------------------------------------------------

-- A `ui.image`: one of the visual's own assets, from the bytes the window
-- read with its script (`visuals.picture`). With `before`, a wipe the
-- reader drags; with `marks`, points and boxes over it (a click is the
-- script's, the pointer's mark named in the corner).
local Picture = ui.component(function(props: { n: any, ctx: Ctx }): Node
	local n, ctx = props.n, props.ctx
	local get = ctx.at(n.p)
	local function shown(name: string?): any
		if not name then
			return nil
		end
		local ok, got = pcall(ui.call_sync, ctx.api .. ".picture", { id = ctx.id, name = name })
		if not ok then
			return { error = tostring(got) }
		end
		return got
	end
	local after, before = shown(n.src) or {}, shown(n.before)
	local failed = after.error or (before and before.error)
	if failed then
		return problem(function(): string
			return failed
		end)
	end
	local size = ui.use_state({ w = 0, h = 0 })
	local split = ui.use_state(n.split or 0.5)
	local hover = ui.use_state(0)
	local dragging = false
	-- Where the picture sits in its box, in points: all of it (contain) or
	-- the box filled (cover), the middle kept.
	local function frame(): (number, number, number, number)
		local s = size:get()
		local iw, ih = math.max(after.w or 1, 1), math.max(after.h or 1, 1)
		if s.w <= 0 or s.h <= 0 then
			return 0, 0, 0, 0
		end
		local scale = if n.fit == "cover" then math.max(s.w / iw, s.h / ih) else math.min(s.w / iw, s.h / ih)
		local w, h = iw * scale, ih * scale
		return (s.w - w) / 2, (s.h - h) / 2, w, h
	end
	local function mark_at(px: number, py: number): number
		local now = ctx.peek(n.p)
		local ox, oy, w, h = frame()
		local found, nearest = 0, math.huge
		for i, m in (now and now.marks) or {} do
			local x, y = ox + m.x * w, oy + m.y * h
			if m.w then
				if px >= x and px <= x + m.w * w and py >= y and py <= y + m.h * h then
					local area = m.w * m.h
					if area < nearest then
						found, nearest = i, area
					end
				end
			else
				local d = (px - x) ^ 2 + (py - y) ^ 2
				if d <= 14 ^ 2 and d - 1 < nearest then
					found, nearest = i, d - 1
				end
			end
		end
		return found
	end
	local pressed = 0
	local function set_split(px: number)
		local ox, _, w = frame()
		if w > 0 then
			split:set(math.clamp((px - ox) / w, 0, 1))
		end
	end
	local function image(src: string, fixed: boolean): Node
		return ui.image {
			src = src,
			w = if fixed then function(): any
				return raw(size:get().w, "the picture under a wipe, as wide as its box")
			end else nil,
			h = if fixed then function(): any
				return raw(size:get().h, "the picture under a wipe, as tall as its box")
			end else nil,
			w_full = not fixed,
			h_full = not fixed,
			fit = n.fit,
			label = n.alt or ctx.title(),
		}
	end
	local size_xs = text_size("xs")
	local layers: { Node } = { image(after.src, false) }
	if before then
		table.insert(
			layers,
			ui.div {
				absolute = true,
				left = 0,
				top = 0,
				h_full = true,
				overflow = "hidden",
				w = function(): any
					local ox, _, w = frame()
					return raw(math.max(ox + split:get() * w, 0), "how much of a wipe shows the picture before")
				end,
				image(before.src, true),
			}
		)
	end
	table.insert(
		layers,
		ui.canvas {
			w_full = true,
			h_full = true,
			pointer_events = "none",
			draw = function(c: any)
				local now = get()
				local ox, oy, w, h = frame()
				if w <= 0 then
					return
				end
				if before then
					local x = ox + split:get() * w
					c:line(x, oy, x, oy + h, ink.canvas("surface"), 3)
					c:line(x, oy, x, oy + h, ink.canvas("text"), 1)
					c:circle(x, oy + h / 2, 11, ink.canvas("surface"))
					c:circle(x, oy + h / 2, 9, ink.canvas("text"))
					c:line(x - 4, oy + h / 2, x + 4, oy + h / 2, ink.canvas("surface"), 2)
				end
				local over = hover:get()
				for i, m in (now and now.marks) or {} do
					local tone = ink.canvas(m.c or "accent")
					local chosen = now.sel == i or over == i
					local x, y = ox + m.x * w, oy + m.y * h
					if m.w then
						local mw, mh = m.w * w, m.h * h
						local width = if chosen then 3 else 2
						for _, l in { { x, y, x + mw, y }, { x + mw, y, x + mw, y + mh }, { x + mw, y + mh, x, y + mh }, { x, y + mh, x, y } } do
							c:line(l[1], l[2], l[3], l[4], tone, width)
						end
						-- Its number on a pin at its top left corner, as a point's.
						c:circle(x, y, 10, ink.canvas("surface"))
						c:circle(x, y, 8, tone)
						c:text(tostring(i), x, y, { size = size_xs, color = ink.canvas("surface"), align = "center", baseline = "middle" })
					else
						c:circle(x, y, if chosen then 13 else 11, ink.canvas("surface"))
						c:circle(x, y, if chosen then 11 else 9, tone)
						c:text(tostring(i), x, y, { size = size_xs, color = ink.canvas("surface"), align = "center", baseline = "middle" })
					end
				end
			end,
		}
	)
	table.insert(
		layers,
		ui.show(
			function(): boolean
				return hover:get() > 0
			end,
			-- (On a chip of the surface: the picture under it may be any colour.)
			ui.div {
				absolute = true,
				left = 6,
				top = 6,
				px = 6,
				py = 2,
				rounded = "$radius.sm",
				bg = "$surface.overlay",
				pointer_events = "none",
				ui.text {
					function(): string
						local now = ctx.peek(n.p)
						local m = now and now.marks and now.marks[hover:get()]
						return if m then `{hover:get()}. {m.label or ""}` else ""
					end,
					literal = true,
					text_size = size_xs,
					weight = "$font.weight.medium",
					color = "$text.primary",
				},
			}
		)
	)
	local aspect = math.max(after.w or 1, 1) / math.max(after.h or 1, 1)
	local overlay: { [any]: any } = {
		class = "visual-image",
		w_full = true,
		h = if n.h then n.h else nil,
		aspect = if n.h then nil else aspect,
		rounded = "$radius.md",
		overflow = "hidden",
		label = n.alt or ctx.title(),
		cursor = if before then "ew-resize" elseif n.click then "pointer" else nil,
		on_resize = function(s: any)
			size:set({ w = s.w, h = s.h })
		end,
		-- (A click carries no point: the mark is the one pressed on.)
		on_press = if before or n.click
			then function(e: any)
				pressed = if n.marks then mark_at(e.local_x, e.local_y) else 0
				if before then
					dragging = true
					set_split(e.local_x)
				end
			end
			else nil,
		on_release = if before
			then function()
				dragging = false
			end
			else nil,
		on_pointer_move = function(e: any)
			if dragging then
				set_split(e.local_x)
			end
			if n.marks then
				hover:set(mark_at(e.local_x, e.local_y))
			end
		end,
		on_hover = function(inside: boolean)
			if not inside then
				hover:set(0)
			end
		end,
		on_click = if n.click
			then function()
				local now = ctx.peek(n.p)
				local i = pressed
				pressed = 0
				if now and now.click and i > 0 then
					ctx.event(now.click, i)
				end
			end
			else nil,
	}
	for _, layer in layers do
		table.insert(overlay, layer)
	end
	return ui.overlay(overlay)
end)

-- ---- layout -------------------------------------------------------------------

local ITEMS = { start = "start", center = "center", ["end"] = "end" }
local JUSTIFY = { start = "start", center = "center", ["end"] = "end", between = "between" }

-- How a child sits in a row: the width it was given, the spare width it
-- takes (asked, or a node with no width of its own), or its own.
local function place(props: { [any]: any }, n: any, parent: string)
	if n.w then
		props.w = raw(n.w, "a visual's width for this, as its script asks")
		props.shrink = 0
	elseif n.grow or (parent == "row" and FLUID[n.k]) then
		props.grow = 1
		props.basis = 0
		props.min_w = 0
	elseif parent == "row" then
		props.shrink = 1
		props.min_w = 0
	end
	return props
end

-- ---- code -------------------------------------------------------------------

-- A marked line's wash and its gutter label's ink.
local CODE_MARK = {
	added = { wash = "$zend.tone.success.wash", ink = "$zend.diff.added" },
	removed = { wash = "$zend.tone.danger.wash", ink = "$zend.diff.removed" },
	focus = { wash = "$zend.tone.accent.wash", ink = "$accent" },
}

-- Source lines (ui.code), coloured by the app's own highlighter
-- (ds/syntax, as Markdown's code blocks and the diff are) for the script's
-- language; one it doesn't know, or none, is plain text. A marked line
-- takes its tone's wash the whole way across and its gutter label the
-- tone's ink. The lines scroll sideways as one, as the app's diff does.
-- (Their text is in the shape: a change builds them again.)
local function code(n: any, parent: string): Node
	local lines = n.lines or {}
	local source = {}
	for i, l in lines do
		source[i] = l.s
	end
	local runs = if n.lang and Syntax.highlighter(n.lang) then Syntax.lines(table.concat(source, "\n"), n.lang) else nil
	-- (The room the lines scroll in: the washes run its whole width, a
	-- long line's past it. A percentage doesn't reach through the scroll.)
	local room = ui.ref()
	local rows: { Node } = {}
	for i, l in lines do
		local mark = CODE_MARK[l.m]
		local kids: { Node } = {}
		if n.gutter then
			table.insert(kids, ui.text {
				l.g or "",
				class = "visual-code-gutter",
				literal = true,
				shrink = 0,
				pr = "$space.3",
				tabular = true,
				font = "$font.mono",
				text_size = "$zend.visual.text.sm.size",
				line_height = "$zend.visual.text.sm.line",
				color = if mark then mark.ink else "$text.tertiary",
			})
		end
		local line: { [any]: any } = {
			class = "visual-code-text",
			literal = true,
			shrink = 0,
			font = "$font.mono",
			text_size = "$zend.visual.text.sm.size",
			line_height = "$zend.visual.text.sm.line",
			color = "$text.primary",
		}
		local own = runs and runs[i]
		if own and #own > 0 then
			for j, r in own do
				line[j] = ui.span { r.text, color = r.color, literal = true }
			end
			table.insert(kids, ui.rich(line))
		else
			-- (An empty line keeps its height.)
			line[1] = if l.s == "" then " " else l.s
			table.insert(kids, ui.text(line))
		end
		table.insert(rows, ui.row {
			class = "visual-code-line",
			label = l.s,
			w_full = true,
			px = "$space.1",
			bg = if mark then mark.wash else nil,
			table.unpack(kids),
		})
	end
	return ui.column(place({
		class = "visual-code",
		label = n.lang,
		w_full = parent ~= "row",
		min_w = 0,
		ui.scroll_x {
			ref = room,
			w_full = true,
			fade_x = { 16, 32 },
			fade_overflow = true,
			ui.column {
				min_w = function(): number
					return room.width or 0
				end,
				table.unpack(rows),
			},
		},
	}, n, parent))
end

local function stack(n: any, ctx: Ctx, parent: string): Node
	local row = n.k == "row"
	local kids: { Node } = {}
	for _, child in n.children or {} do
		table.insert(kids, build(child, ctx, n.k))
	end
	local props: { [any]: any } = {
		class = if row then "visual-row" else "visual-column",
		w_full = parent ~= "row",
		min_w = 0,
		gap = raw(n.gap, "a visual's gap, as its script asks"),
		items = if row then (ITEMS[n.align] or "start") elseif n.align == "start" then nil else ITEMS[n.align],
		justify = JUSTIFY[n.justify],
		flex_wrap = row and n.wrap or nil,
		kids,
	}
	if n.hover or n.click then
		local h = ui.theme.get("space.2")
		local v = ui.theme.get("space.1")
		local px = if type(h) == "number" then h else 8
		local py = if type(v) == "number" then v else 4
		-- The wash hangs past the text, which stays where it was.
		props.px, props.py = "$space.2", "$space.1"
		props.mx = ui.raw(-px, "the wash's room, out past the text so the text keeps its edge")
		props.my = ui.raw(-py, "the wash's room, out past the text so the text keeps its edge")
		props.rounded = "$radius.md"
		props.hover = { bg = "$hover" }
		if n.click then
			props.cursor = "pointer"
			props.role = "button"
			props.focusable = true
			props.on_click = function()
				local now = ctx.peek(n.p)
				ctx.event(now and now.click)
			end
			props.on_key = function(e: any): boolean
				if e.key == "Enter" or e.key == " " then
					local now = ctx.peek(n.p)
					ctx.event(now and now.click)
					return true
				end
				return false
			end
		end
	end
	place(props, n, parent)
	return if row then ui.row(props) else ui.column(props)
end

build = function(n: any, ctx: Ctx, parent: string): Node
	local k = n.k
	if k == "column" or k == "row" then
		return stack(n, ctx, parent)
	elseif k == "grid" then
		local kids: { Node } = {}
		for _, child in n.children or {} do
			table.insert(kids, ui.column { min_w = 0, build(child, ctx, "grid") })
		end
		return ui.grid(place({ class = "visual-grid", w_full = true, cols = n.columns, gap = raw(n.gap, "a visual's gap, as its script asks"), kids }, n, parent))
	elseif k == "text" then
		local node = text(n, ctx)
		if parent == "row" and (n.w or n.grow) then
			return ui.column(place({ min_w = 0, node }, n, parent))
		end
		return node
	elseif k == "rule" then
		if parent == "row" then
			return ui.div { w = 1, self = "stretch", shrink = 0, bg = "$border" }
		end
		return ui.div { w_full = true, h = 1, shrink = 0, bg = "$border" }
	elseif k == "space" then
		local s = raw(n.size, "a visual's room, as its script asks")
		return ui.div { w = s, h = s, shrink = 0 }
	elseif k == "stat" then
		local get = ctx.at(n.p)
		local function field(name: string): () -> any
			return function(): any
				local now = get()
				return now and now[name]
			end
		end
		return ui.column(place({
			min_w = 0,
			Stat {
				value = field("value"),
				label = field("label"),
				delta = field("delta"),
				tone = function(): any
					local now = get()
					return if now and now.tone then ink.paint(now.tone) else nil
				end,
			},
		}, n, parent))
	elseif k == "table" then
		local count = 0
		for _, row in n.rows or {} do
			count = math.max(count, #row)
		end
		return ui.column(place({
			w_full = parent ~= "row",
			min_w = 0,
			DataTable {
				columns = n.columns or {},
				count = count,
				rows = #(n.rows or {}),
				hover = n.hover,
				label = ctx.title(),
				cell = function(r: number, c: number): Node
					local cell = (n.rows[r] or {})[c]
					if not cell then
						return ui.div {}
					end
					return build(cell, ctx, "cell")
				end,
			},
		}, n, parent))
	elseif k == "chart" then
		local get = ctx.at(n.p)
		return ui.column(place({
			w_full = parent ~= "row",
			min_w = 0,
			Chart { data = get, height = n.h, label = ctx.title() },
		}, n, parent))
	elseif k == "canvas" then
		return ui.column(place({ w_full = parent ~= "row", min_w = 0, Canvas { n = n, ctx = ctx } }, n, parent))
	elseif k == "scene" or k == "shader" then
		return ui.column(place({ w_full = parent ~= "row", min_w = 0, Surface { n = n, ctx = ctx } }, n, parent))
	elseif k == "image" then
		return ui.column(place({ w_full = parent ~= "row", min_w = 0, Picture { n = n, ctx = ctx } }, n, parent))
	elseif k == "button" then
		local get = ctx.at(n.p)
		return Button(place({
			tone = if n.primary then "primary" else "secondary",
			size = "sm",
			on_click = function()
				local now = ctx.peek(n.p)
				ctx.event(now and now.click)
			end,
			ui.text {
				function(): string
					local now = get()
					return if now then now.label else ""
				end,
				literal = true,
			},
		}, n, parent))
	elseif k == "answer" then
		if not VisualAnswer.available() then
			return ui.div {}
		end
		local get = ctx.at(n.p)
		-- Sent: what was sent stays in its inputs, and it says so here.
		return ui.show(ctx.sent, ui.text {
			class = "visual-sent",
			locale.s("ds.visual.answer.sent"),
			literal = true,
			text_size = "$zend.visual.text.sm.size",
			line_height = "$zend.visual.text.sm.line",
			color = "$text.tertiary",
		}, Button(place({
			class = "visual-answer",
			tone = "primary",
			size = "sm",
			on_click = function()
				ctx.answer()
			end,
			ui.text {
				function(): string
					local now = get()
					return if now and now.label then now.label else locale.s("ds.visual.answer.add")
				end,
				literal = true,
			},
		}, n, parent)))
	elseif k == "tabs" then
		return ui.row(place({ min_w = 0, Tabs { n = n, ctx = ctx } }, n, parent))
	elseif k == "toggle" then
		return ui.row(place({ min_w = 0, Toggle { n = n, ctx = ctx } }, n, parent))
	elseif k == "slider" then
		return ui.column(place({ w_full = parent ~= "row", min_w = 0, VisualSlider { n = n, ctx = ctx } }, n, parent))
	elseif k == "input" then
		return ui.column(place({ w_full = parent ~= "row", min_w = 0, Input { n = n, ctx = ctx } }, n, parent))
	elseif k == "code" then
		return code(n, parent)
	elseif k == "link" then
		local get = ctx.at(n.p)
		return ui.text(place({
			function(): string
				local now = get()
				return if now then now.label else ""
			end,
			class = "visual-link",
			literal = true,
			role = "link",
			cursor = "pointer",
			text_size = "$zend.visual.text.body.size",
			line_height = "$zend.visual.text.body.line",
			color = "$accent",
			hover = { color = "$accent.hover" },
			on_click = function()
				local now = ctx.peek(n.p)
				if now and now.url then
					Links.open(now.url)
				end
			end,
			-- A web link's menu (ds/links), for where it goes now.
			context_menu = function(e: PointerEvent): { Node }?
				local now = ctx.peek(n.p)
				if now and type(now.url) == "string" and Links.web(now.url) then
					return Links.menu_for(now.url)(e)
				end
				return nil
			end,
		}, n, parent))
	end
	return ui.div {}
end

-- ---- the visual ---------------------------------------------------------------

return ui.component(function(props: Props): Node
	local id = props.id
	local api = props.api or "visuals"
	local REVS = revs(api)
	-- Read and started once (the host keeps it: a reload, the chat opened
	-- again, another window, none starts it twice).
	local function open()
		pcall(function()
			local promise = ui.call(api .. ".open", {
				id = id,
				path = props.path,
				title = props.title,
				device = props.device,
				source = props.source,
				rev = props.rev,
				sent = props.sent,
			})
			if promise then
				promise:after(function() end, function() end)
			end
		end)
	end
	open()
	-- The agent updated it (its part's rev grew): the host reads it again,
	-- what the reader entered kept.
	ui.effect(function()
		local r = REVS:get()[id]
		if r and r.stale then
			open()
		end
	end)
	local rev = ui.computed(function(): number
		local r = REVS:get()[id]
		return if r then r.rev else -1
	end)
	local gen = ui.computed(function(): number
		local r = REVS:get()[id]
		return if r then r.gen else -1
	end)
	-- Its answers were sent (its part says so, or this window just sent them).
	local sent = ui.computed(function(): boolean
		local r = REVS:get()[id]
		return props.sent == true or (r ~= nil and r.sent == true)
	end)
	local snap = ui.computed(function(): any
		rev:get()
		local ok, s = pcall(ui.call_sync, api .. ".get", id)
		return if ok and type(s) == "table" then s else { state = "loading" }
	end)
	local nodes = ui.computed(function(): { [string]: any }
		local into = {}
		local tree = snap:get().tree
		if tree then
			index(tree, into)
		end
		return into
	end)
	local shape = ui.computed(function(): string
		local s = snap:get()
		if s.state ~= "ready" or not s.tree then
			return s.state or "loading"
		end
		local out = { "ready" }
		signature(s.tree, out)
		return table.concat(out, " ")
	end)
	local drew = ui.use_state("")
	local function title(): string
		local t = props.title or snap:peek().title or ""
		return if t ~= "" then t else locale.s("ds.visual.untitled")
	end
	local ctx: Ctx = {
		id = id,
		api = api,
		title = title,
		at = function(path: string): () -> any
			return function(): any
				return nodes:get()[path]
			end
		end,
		peek = function(path: string): any
			return nodes:peek()[path]
		end,
		gen = function(): number
			return gen:get()
		end,
		event = function(handler: number?, arg: any?)
			if type(handler) == "number" then
				pcall(ui.invoke, api .. ".event", { id = id, h = handler, arg = arg })
			end
		end,
		input = function(name: string, value: any)
			if sent:peek() then
				return
			end
			local ok, promise = pcall(ui.call, api .. ".input", { id = id, name = name, value = value })
			if ok and promise then
				promise:after(function() end, function() end)
			end
		end,
		focus = function(name: string?)
			pcall(ui.invoke, api .. ".focus", { id = id, name = name })
		end,
		sent = function(): boolean
			return sent:get()
		end,
		answer = function()
			if sent:peek() then
				return
			end
			local ok, text = pcall(ui.call_sync, api .. ".answer", id)
			if ok and type(text) == "string" then
				VisualAnswer.put(props.chat, id, text, { path = props.path, device = props.device })
			end
		end,
		drew = function(error: string?)
			local message = error or ""
			if drew:peek() ~= message then
				-- (Told while drawing: shown once the frame is done.)
				ui.after(0, function()
					drew:set(message)
				end)
			end
		end,
	}
	local function error(): string
		local s = snap:get()
		if s.state == "ready" and s.error then
			return s.error
		end
		return drew:get()
	end
	return ui.column {
		class = "visual",
		role = "group",
		label = title(),
		w_full = true,
		gap = "$space.3",
		ui.presence(function(): string
			return shape:get()
		end, function(key: string): Node
			if key == "loading" then
				return ui.row {
					class = "visual-loading",
					items = "center",
					gap = "$space.2",
					h = "$zend.visual.text.sm.line",
					status.Spinner { size = 12, ink = "$text.tertiary" },
					ui.text {
						function(): string
							return locale.s("ds.visual.loading", { title = title() })
						end,
						literal = true,
						text_size = "$zend.visual.text.sm.size",
						line_height = "$zend.visual.text.sm.line",
						color = "$text.tertiary",
					},
				}
			end
			if key == "failed" then
				return problem(function(): string
					return snap:get().error or ""
				end)
			end
			local tree = snap:peek().tree
			if not tree then
				return ui.div {}
			end
			-- (A tree this can't draw stays in its place, said so: the
			-- transcript round it goes on.)
			local ok, built = pcall(build, tree, ctx, "root")
			if ok then
				return built
			end
			return problem(function(): string
				return tostring(built)
			end)
		end, { w_full = true }),
		ui.show(
			function(): boolean
				return error() ~= "" and shape:get() ~= "failed"
			end,
			problem(error, true)
		),
	}
end)
