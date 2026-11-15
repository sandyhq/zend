-- Where a visual's answers go (`ui.answer`, plans/live-visuals D8): into
-- the composer of the chat the visual is in, under what's written there.
-- Nothing is sent: the reader edits and sends. Pressed again, the block it
-- put in is replaced, as long as the draft still holds it as it was; the
-- rest of the draft stays. (Edited by the reader, it's theirs: the new
-- block goes under it.)
--
-- The app sets `read` and `write` (screens/chat/store), a chat's draft by
-- the doc the visual is in (nil: the chat shown). Without them (a
-- plugin's page, a fixture) there's no answer button.
--
--   VisualAnswer.read = function(chat) return draft end
--   VisualAnswer.write = function(chat, text) … end
--   VisualAnswer.put(chat, id, block)   true when the draft took it

local M = {}

M.read = nil :: ((chat: string?) -> string)?
M.write = nil :: ((chat: string?, text: string) -> ())?

-- What each visual last put in, by its id.
local given = ui.shared("visual.answers", {} :: { [string]: string })

function M.available(): boolean
	return M.read ~= nil and M.write ~= nil
end

-- `draft` with `block` in: where `previous` (the block put in before) is,
-- if the draft holds it unchanged, else under what's there, a blank line
-- between.
function M.merge(draft: string, previous: string?, block: string): string
	if previous and previous ~= "" then
		local at, to = string.find(draft, previous, 1, true)
		if at and to then
			return string.sub(draft, 1, at - 1) .. block .. string.sub(draft, to + 1)
		end
	end
	if draft == "" then
		return block
	end
	if string.sub(draft, -2) == "\n\n" then
		return draft .. block
	end
	if string.sub(draft, -1) == "\n" then
		return draft .. "\n" .. block
	end
	return draft .. "\n\n" .. block
end

function M.put(chat: string?, id: string, block: string): boolean
	local read, write = M.read, M.write
	if not read or not write or block == "" then
		return false
	end
	local draft = read(chat)
	local now = M.merge(draft, given:peek()[id], block)
	local all = table.clone(given:peek())
	all[id] = block
	given:set(all)
	if now ~= draft then
		write(chat, now)
	end
	return true
end

return M
