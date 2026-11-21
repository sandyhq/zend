--!strict
-- The system's bars over the window, in points: on Android the status bar,
-- the navigation bar, a display cutout, and the soft keyboard (`keyboard`,
-- from the window's bottom), as the app reads them (src/android.rs, kept
-- current as they change). Zero everywhere else: a desktop window has none
-- over it.
--
--   local insets = require("@ds/insets")
--   insets.get().top        -- reactive: read in a prop function
--   insets.pad(node)        -- `node` kept clear of the bars (Android only)

export type Insets = { top: number, right: number, bottom: number, left: number, keyboard: number }

local NONE: Insets = { top = 0, right = 0, bottom = 0, left = 0, keyboard = 0 }
local shared = ui.shared("system.insets", NONE :: any)

local M = {}

-- The insets now (a read in a computed, an effect or a prop function
-- follows them).
function M.get(): Insets
	return (shared:get() or NONE) :: Insets
end

local function side(name: string): () -> number
	return function(): number
		local insets = M.get() :: any
		local n = insets[name]
		if name == "bottom" then
			n = math.max(n, insets.keyboard)
		end
		return ui.raw(n, "the system's bars over the window")
	end
end

-- `node` in a column the size of the window, padded clear of the bars (and
-- of the keyboard, at the bottom), where there are any (Android); elsewhere
-- `node` itself, the window's layout as it was.
function M.pad(node: any): any
	if ui.shared("platform", ""):peek() ~= "android" then
		return node
	end
	return ui.column {
		id = "system-insets",
		w_full = true,
		h_full = true,
		pt = side("top"),
		pr = side("right"),
		pb = side("bottom"),
		pl = side("left"),
		node,
	}
end

return M
