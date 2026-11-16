-- An open visual (it asks the person something they haven't sent) stays
-- reachable while the transcript moves on: once its place in the
-- transcript has scrolled out of view, the chat's newest such one is
-- pinned above the composer, and its place says so; scrolled back into
-- view, it goes back to its place (plans/live-visuals G4, fix1). Sent
-- (MarkVisualSent), it stays in the transcript, showing what was sent.
-- Moving between the two keeps what was entered (the host keeps it) and
-- the field being typed in, with its caret (ds/visual).
--
--   VisualPin.pick(messages, revs, away) → the block to pin, or nil
--   VisualPin.pinned          ui.shared { [id] = true }: what's pinned now
--   VisualPin.away            ui.shared { [id] = true }: places out of view
--   VisualPin.Place { id, title, visual }   the transcript's place
--   VisualPin.Header { block }   the pinned visual's bar: its title line,
--                            a minimise button and a × (fix2)
--   VisualPin.minimised       ui.shared { [id] = true }: folded to its line
--
-- Minimised, the pinned visual is its one title line above the composer
-- (the line or its button opens it again); this window's, kept per
-- visual, and it doesn't answer anything. The × closes it by hand: it
-- is marked sent as a message holding its answers marks it, and stays
-- answered across reloads.

local locale = require("@self/locale")
local IconButtons = require("@self/icon_button")
local VisualAnswer = require("@self/visual_answer")

local M = {}

M.pinned = ui.shared("visual.pinned", {} :: { [string]: boolean })
M.away = ui.shared("visual.away", {} :: { [string]: boolean })
M.minimised = ui.shared("visual.minimised", {} :: { [string]: boolean })

-- The pinned bar's height, in points.
M.BAR = 28

-- Each place's height as last laid out with its visual in it: a row the
-- list lets go and builds again while pinned keeps its height.
local heights: { [string]: number } = {}

local function set_away(id: string, away: boolean)
	local now = M.away:peek()
	if (now[id] == true) ~= away then
		M.away:set_key({ id }, if away then true else nil)
	end
end

-- Every visual block in `blocks`, nested ones too (a work group's), in order.
local function visuals_in(blocks: { any }?, out: { any })
	for _, b in blocks or {} do
		if b.kind == "visual" then
			table.insert(out, b)
		elseif type(b.blocks) == "table" then
			visuals_in(b.blocks, out)
		end
	end
end

-- The newest visual in `messages` that is open, as the host says
-- (`revs`: shared `visuals`, `{ [id] = { open, sent } }`), whose part
-- doesn't say it was sent, and whose place is out of view (`away`).
function M.pick(messages: { any }, revs: { [string]: any }, away: { [string]: boolean }?): any
	for i = #messages, 1, -1 do
		local found = {}
		visuals_in(messages[i].blocks, found)
		for j = #found, 1, -1 do
			local b = found[j]
			local r = revs[b.id]
			if b.sent ~= true and b.dismissed ~= true and r and r.open == true and (away == nil or away[b.id] == true) then
				return b
			end
		end
	end
	return nil
end

function M.Marker(props: { id: string, title: string? }): Node
	return ui.text {
		class = "visual-pinned-marker",
		locale.s("ds.visual.pinned", { title = props.title or locale.s("ds.visual.untitled") }),
		literal = true,
		-- one line: the pinned visual's strip over the composer
		truncate = true,
		text_size = "$zend.visual.text.sm.size",
		line_height = "$zend.visual.text.sm.line",
		color = "$text.tertiary",
	}
end

-- Fold the pinned visual `id` to its line, or open it again.
function M.toggle(id: string)
	local now = M.minimised:peek()[id] == true
	M.minimised:set_key({ id }, if now then nil else true)
end

-- The pinned visual's bar: `block` is a state holding its transcript block
-- (nil when nothing is pinned).
M.Header = ui.component(function(props: { block: any }): Node
	local folded = ui.computed(function(): boolean
		local b = props.block:get()
		return b ~= nil and M.minimised:get()[b.id] == true
	end)
	local function title(): string
		local b = props.block:get()
		local t = if b and b.title then b.title else ""
		return if t ~= "" then t else locale.s("ds.visual.untitled")
	end
	local function toggle()
		local b = props.block:peek()
		if b then
			M.toggle(b.id)
		end
	end
	return ui.row {
		id = "chat-pinned-visual-bar",
		class = "visual-pinned-bar",
		w_full = true,
		h = M.BAR,
		shrink = 0,
		items = "center",
		gap = "$space.1",
		px = "$space.2",
		IconButtons.IconButton {
			id = "chat-pinned-visual-fold",
			icon = function(): any
				return if folded:get() then "alt-arrow-right" else "alt-arrow-down"
			end,
			label = function(): string
				return locale.s(if folded:get() then "ds.visual.expand" else "ds.visual.minimise")
			end,
			size = "sm",
			tone = "quiet",
			on_click = toggle,
		},
		ui.text {
			class = "visual-pinned-title",
			title,
			literal = true,
			-- one line: the pinned visual's strip over the composer
			truncate = true,
			grow = 1,
			min_w = 0,
			text_size = "$zend.visual.text.sm.size",
			line_height = "$zend.visual.text.sm.line",
			color = "$text.secondary",
			cursor = function(): string
				return if folded:get() then "pointer" else "default"
			end,
			on_click = function()
				if folded:peek() then
					toggle()
				end
			end,
		},
		IconButtons.IconButton {
			id = "chat-pinned-visual-close",
			icon = "close",
			label = locale.s("ds.visual.close"),
			size = "sm",
			tone = "quiet",
			on_click = function()
				local b = props.block:peek()
				if b then
					VisualAnswer.dismiss(b.id, { path = b.path, device = b.device })
				end
			end,
		},
	}
end)

-- A visual's place in the transcript: the visual, or, while it's pinned,
-- a line saying so in a box as tall as the visual was (the transcript
-- around it doesn't move). Whether any of it shows decides the pin: a
-- place out of view (or a row the list let go) is away.
M.Place = ui.component(function(props: { id: string, title: string?, visual: Node }): Node
	local id = props.id
	local tall = ui.use_state(heights[id] or 0)
	local pinned = ui.computed(function(): boolean
		return M.pinned:get()[id] == true
	end)
	ui.on_cleanup(function()
		set_away(id, true)
	end)
	return ui.column {
		class = "visual-place",
		w_full = true,
		min_h = function(): number
			return if pinned:get() then tall:peek() else 0
		end,
		on_visible = function(visible: boolean)
			set_away(id, not visible)
		end,
		on_resize = function(size: { w: number, h: number })
			if not pinned:peek() and math.abs(tall:peek() - size.h) >= 1 then
				tall:set(size.h)
				heights[id] = size.h
			end
		end,
		ui.show(function(): boolean
			return pinned:get()
		end, M.Marker { id = id, title = props.title }, props.visual),
	}
end)

return M
