-- The text input that has the keys in a visual, carried when the visual
-- moves (pinned above the composer, or back into the transcript:
-- ds/visual_pin): the visual built in its new place is another, and its
-- same input takes the keys and the selection back (plans/live-visuals
-- fix1). Plain tables, nothing shared: a plugin's page loads it too.
--
--   VisualCarry.hold(id, name, el)   an input took the keys (el nil: gave them up)
--   VisualCarry.save(id)             before visual `id` moves: what has the keys, kept
--   VisualCarry.take(id, name)       → { a, b } once, if it was kept just now

local M = {}

-- (Kept this long: a visual that doesn't come back leaves nothing to take.)
local FOR = 1

local live: { [string]: { name: string, el: any } } = {}
local kept: { [string]: { name: string, a: number?, b: number?, at: number } } = {}

function M.hold(id: string, name: string, el: any)
	if el then
		live[id] = { name = name, el = el }
	elseif live[id] and live[id].name == name then
		live[id] = nil
	end
end

function M.save(id: string)
	local l = live[id]
	if not l then
		return
	end
	local ok, a, b = pcall(function()
		return l.el:selection()
	end)
	kept[id] = { name = l.name, a = if ok then a else nil, b = if ok then b else nil, at = os.clock() }
end

function M.take(id: string, name: string): { a: number?, b: number? }?
	local k = kept[id]
	if not k or k.name ~= name then
		return nil
	end
	kept[id] = nil
	if os.clock() - k.at > FOR then
		return nil
	end
	return { a = k.a, b = k.b }
end

return M
