-- Placement only: dimensions come from the composer's size contract.
local Dock = require("@app/screens/chat/dock")

local M = {}

-- A new chat's composer: its middle at the hero's line (`Dock.hero_offset`),
-- raised where its `presented` height would run off the page's foot.
function M.new_offset(page: number, rest: number, presented: number, foot: number): number
	local centered = Dock.hero_offset() - rest / 2
	return if page > 0 then math.min(centered, page / 2 - foot - presented) else centered
end

function M.dock_offset(page: number, rest: number, presented: number, foot: number): number
	return page / 2 + M.new_offset(page, rest, presented, foot) + presented - (page - foot)
end

return M
