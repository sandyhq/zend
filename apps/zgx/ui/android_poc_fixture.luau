-- The Android POC's screen (plans/android-zgx-poc): Zend's real sidebar
-- (ds.Sidebar) with fixed sessions, open and full width, no engine behind
-- it. apps/zgx-android runs it with the poc-tap plugin
-- (plugins-dev/poc-tap), which adds its button after #sidebar-actions by
-- zend.place: the same files on the desktop (`zgx-poc`) and in the APK
-- (app.zend.zgxpoc).

ui.theme.strict("warn")
local ds = require("@ds")
ds.setup()

ui.effect(function()
	ui.theme.follow_system(false)
	ui.theme.current:set("zend-dark")
end)

local places = ui.state("poc_places", { pinned = {}, sections = {} } :: any)
local selected = ui.state("poc_selected", "a1" :: string?)

local projects = {
	{
		id = "atlas",
		name = "Atlas",
		hue = "violet",
		sessions = {
			{ id = "a1", title = "Fix the build", status = "working", time = "5m", device = "Mac" },
			{ id = "a2", title = "Write the docs", time = "1h", device = "Mac" },
			{ id = "a3", title = "Tidy the tests", time = "2h", device = "Mac" },
		},
	},
	{
		id = "borealis",
		name = "Borealis",
		hue = "blue",
		sessions = {
			{ id = "b1", title = "Ship the release", status = "input", time = "3h", device = "Mac" },
			{ id = "b2", title = "Answer the review", time = "1d", device = "Box" },
		},
	},
}

return ui.column {
	id = "poc",
	w_full = true,
	h_full = true,
	theme = "zend-dark",
	-- (Opaque: a capture of the window's glass comes out white.)
	bg = ui.raw("#111113", "a capture's backdrop: the window's glass snapshots white"),
	ds.Sidebar {
		compact = ui.state("poc_compact", true),
		collapsed = ui.state("poc_collapsed", false),
		island = false,
		selected = selected,
		places = places,
		organize = ui.state("poc_organize", { "project" } :: any),
		projects = projects :: any,
		here = "Mac",
	},
}
