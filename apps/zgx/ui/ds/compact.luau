--!strict
-- Where a window turns compact (a phone's): below this many points wide
-- the shell drops the columns beside its page (screens/shell/fit_policy)
-- and the ds pieces that follow it (the command palette's scope row) take
-- their narrow form. One cutoff for both, so nothing switches early.

local M = {}

M.BELOW = 600

-- Whether a window `width` wide is compact (none known, 0: not).
function M.is(width: number): boolean
	return width > 0 and width < M.BELOW
end

return M
