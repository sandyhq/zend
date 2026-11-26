-- The update button and its list, alone: the rows as src/updates.rs gives them.
ui.theme.strict("warn")
local ds = require("@ds")
ds.setup()
local Updates = require("@app/screens/shell/updates")
local sessions = require("@account/sessions")

-- Computers: this one, a paired server that grants updates ("Granting") and
-- one that doesn't ("Closed").
sessions.now:set({
	state = "ready",
	devices = {
		{ id = "here", name = "MacBook Pro", platform = "macos", here = true, online = true },
		{ id = "granting", name = "Granting", platform = "linux", here = false, online = true, paired = true, peer_areas = { "updates" } },
		{ id = "closed", name = "Closed", platform = "linux", here = false, online = true, paired = true, peer_areas = { "settings" } },
	},
	projects = {},
	chats = {},
	archived = {},
	places = { pinned = {}, sections = {} },
	places_ready = true,
} :: any)

return ui.column {
	w_full = true,
	h_full = true,
	items = "end",
	p = "$space.3",
	ui.show(Updates.shown, Updates.Button {}),
	require("@ds/toast").Host {},
}
