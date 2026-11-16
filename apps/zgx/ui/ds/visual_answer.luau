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
-- Sent in a message that still holds its heading line (the block's first
-- line, `### <title>`, as a line of its own), a visual is answered, however
-- the rest was rewritten: the app marks it sent (engine `MarkVisualSent`),
-- so it shows what was sent and asks no more (plans/live-visuals G4, fix2).
-- Two visuals with one title: each heading line in the message answers one,
-- the one whose block it holds as put first, then the one put last.
-- `dismiss(id, file)` is the reader's own ×: answered, with nothing sent.
--
--   VisualAnswer.dismiss(id, file)   marks it sent, as sending does

local M = {}

M.read = nil :: ((chat: string?) -> string)?
M.write = nil :: ((chat: string?, text: string) -> ())?

export type File = { path: string?, device: string? }
M.mark = nil :: ((id: string, file: File?) -> ())?

-- What each visual last put in, by its id.
-- (A plugin's page has no composer and the app doesn't share this with
-- plugins: there it's a table of its own, and no answer button shows.)
local given = (function(): any
	local ok, shared = pcall(ui.shared, "visual.answers", {} :: { [string]: string })
	if ok then
		return shared
	end
	local held: { [string]: string } = {}
	return {
		peek = function(_): { [string]: string }
			return held
		end,
		get = function(_): { [string]: string }
			return held
		end,
		set = function(_, v: { [string]: string })
			held = v
		end,
	}
end)()
-- And where each one's script is, to mark it sent.
local files: { [string]: File } = {}
-- When each was last put in (a count): the newest one wins a shared title.
local stamps: { [string]: number } = {}
local clock = 0

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

-- A block's heading: its first line, trimmed.
local function heading_of(block: string): string
	return (string.match(string.match(block, "^[^\n]*") or "", "^%s*(.-)%s*$")) or ""
end

-- The ids of the visuals `text` answers: it holds a visual's heading line
-- (as a line of its own, trimmed), whatever else was written. With several
-- visuals under one heading, as many of them as the text has such lines:
-- those whose whole block it holds first, then the newest put.
function M.held_in(text: string): { string }
	local groups: { [string]: { string } } = {}
	local blocks = given:peek()
	for id, block in blocks do
		local heading = heading_of(block)
		if heading ~= "" then
			groups[heading] = groups[heading] or {}
			table.insert(groups[heading], id)
		end
	end
	if next(groups) == nil then
		return {}
	end
	local seen: { [string]: number } = {}
	for line in string.gmatch(text .. "\n", "([^\n]*)\n") do
		local trimmed = string.match(line, "^%s*(.-)%s*$") or ""
		if groups[trimmed] then
			seen[trimmed] = (seen[trimmed] or 0) + 1
		end
	end
	local out = {}
	for heading, ids in groups do
		local n = seen[heading] or 0
		if n > 0 then
			table.sort(ids, function(a: string, b: string): boolean
				local whole_a = string.find(text, blocks[a], 1, true) ~= nil
				local whole_b = string.find(text, blocks[b], 1, true) ~= nil
				if whole_a ~= whole_b then
					return whole_a
				end
				local sa, sb = stamps[a] or 0, stamps[b] or 0
				if sa ~= sb then
					return sa > sb
				end
				return a < b
			end)
			for i = 1, math.min(n, #ids) do
				table.insert(out, ids[i])
			end
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
		stamps[id] = nil
	end
	given:set(all)
	return held
end

-- The reader closes visual `id` by hand: answered all the same, as sent
-- (`file`: where its script is, the transcript part's path and device, for
-- one whose answers were never put in the draft). Nothing is sent.
function M.dismiss(id: string, file: File?)
	local mine = files[id]
	if M.mark then
		M.mark(id, if file and file.path and file.path ~= "" then file else mine)
	end
	files[id], stamps[id] = nil, nil
	if given:peek()[id] ~= nil then
		local all = table.clone(given:peek())
		all[id] = nil
		given:set(all)
	end
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
	clock += 1
	stamps[id] = clock
	if file then
		files[id] = file
	end
	if now ~= draft then
		write(chat, now)
	end
	return true
end

return M
