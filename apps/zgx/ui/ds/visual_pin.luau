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

local locale = require("@self/locale")

local M = {}

M.pinned = ui.shared("visual.pinned", {} :: { [string]: boolean })
M.away = ui.shared("visual.away", {} :: { [string]: boolean })

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
			if b.sent ~= true and r and r.open == true and (away == nil or away[b.id] == true) then
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
		truncate = true,
		text_size = "$zend.visual.text.sm.size",
		line_height = "$zend.visual.text.sm.line",
		color = "$text.tertiary",
	}
end

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
