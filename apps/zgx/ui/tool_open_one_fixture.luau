-- One tool's row opened (ds/tool_open), alone, for src/tool_open_tests.rs:
-- the tool is the test's (`test.tool_open.tool`), as are the host's
-- chat.tool_exchange and files.tool_image, so it orders their answers
-- against the tool settling.
-- Here, beside app.luau, as a mounted script loads modules from its own
-- directory down.

ui.theme.strict("warn")
local ds = require("@ds")
ds.setup()
local ToolOpen = require("@ds/tool_open")

local TOOL = ui.shared("test.tool_open.tool", nil :: any)

return ui.column {
	w_full = true,
	h_full = true,
	p = "$space.4",
	ui.presence(function(): string
		local t = TOOL:get()
		return if t then t.id else ""
	end, function(id: string): Node
		if id == "" then
			return ui.column {}
		end
		return ToolOpen.ToolOpen {
			tool = function(): any
				return TOOL:get()
			end,
		}
	end),
}
