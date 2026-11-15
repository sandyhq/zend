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
--   VisualAnswer.put(chat, id, block, file)   true when the draft took it
--   VisualAnswer.mark = function(id, file) … end   (the app: `visuals.sent`)
--   VisualAnswer.sending(text)   the composer, as it sends `text`
--
-- Sent in a message that still holds its block, a visual is answered: the
-- app marks it sent (engine `MarkVisualSent`), so it shows what was sent
-- and asks no more (plans/live-visuals G4).

local M = {}

M.read = nil :: ((chat: string?) -> string)?
M.write = nil :: ((chat: string?, text: string) -> ())?

export type File = { path: string?, device: string? }
M.mark = nil :: ((id: string, file: File?) -> ())?

-- What each visual last put in, by its id.
local given = ui.shared("visual.answers", {} :: { [string]: string })
-- And where each one's script is, to mark it sent.
local files: { [string]: File } = {}

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

-- The ids of the visuals whose block `text` holds as it was put in.
function M.held_in(text: string): { string }
	local out = {}
	for id, block in given:peek() do
		if block ~= "" and string.find(text, block, 1, true) then
			table.insert(out, id)
		end
	end
	table.sort(out)
	return out
end

-- `text` is being sent: each visual whose answers it holds is marked
-- sent, and forgotten here (pressed again, its block goes in anew).
function M.sending(text: string): { string }
	local held = M.held_in(text)
	if #held == 0 then
		return held
	end
	local all = table.clone(given:peek())
	for _, id in held do
		all[id] = nil
		if M.mark then
			M.mark(id, files[id])
		end
		files[id] = nil
	end
	given:set(all)
	return held
end

function M.put(chat: string?, id: string, block: string, file: File?): boolean
	local read, write = M.read, M.write
	if not read or not write or block == "" then
		return false
	end
	local draft = read(chat)
	local now = M.merge(draft, given:peek()[id], block)
	local all = table.clone(given:peek())
	all[id] = block
	given:set(all)
	if file then
		files[id] = file
	end
	if now ~= draft then
		write(chat, now)
	end
	return true
end

return M
