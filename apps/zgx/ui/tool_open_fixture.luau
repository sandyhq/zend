-- A reply's work (ds/activity) whose tools' rows open to what each did as
-- its host keeps it (ds/tool_open), for src/tool_open_tests.rs: the rows
-- are what src/chat.rs builds of an engine-shaped reply, set by the test as
-- `test.tool_open.tools`; the host's chat.tool_exchange and
-- files.tool_image are the test's.
-- Here, beside app.luau, as a mounted script loads modules from its own
-- directory down.

ui.theme.strict("warn")
local ds = require("@ds")
ds.setup()
local Activity = require("@ds/activity")

local TOOLS = ui.shared("test.tool_open.tools", {} :: { any })

return ui.column {
	w_full = true,
	h_full = true,
	p = "$space.4",
	ui.scroll {
		w_full = true,
		h_full = true,
		Activity {
			key = "work-1",
			-- (The chat the transcript shows, as screens/chat/session gives it.)
			chat = function(): string?
				return "doc-7"
			end,
			tools = function(): { any }
				return TOOLS:get() or {}
			end,
		},
	},
}
