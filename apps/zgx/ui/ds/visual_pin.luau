-- An open visual (it asks the person something they haven't sent) stays
-- reachable while the transcript moves on: the newest one of the chat
-- shown is pinned above the composer, and where it was in the transcript
-- a line says so (plans/live-visuals G4). Sent (MarkVisualSent), it goes
-- back into the transcript, showing what was sent.
--
--   VisualPin.pick(messages, revs) → the block to pin, or nil
--   VisualPin.pinned          ui.shared { [id] = true }: what's pinned now
--   VisualPin.Marker { id, title }   the transcript's line in its place

local locale = require("@self/locale")

local M = {}

M.pinned = ui.shared("visual.pinned", {} :: { [string]: boolean })

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
-- (`revs`: shared `visuals`, `{ [id] = { open, sent } }`), and whose part
-- doesn't say it was sent.
function M.pick(messages: { any }, revs: { [string]: any }): any
	for i = #messages, 1, -1 do
		local found = {}
		visuals_in(messages[i].blocks, found)
		for j = #found, 1, -1 do
			local b = found[j]
			local r = revs[b.id]
			if b.sent ~= true and r and r.open == true then
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

return M
