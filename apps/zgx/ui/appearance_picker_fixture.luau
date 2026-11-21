-- Setup's appearance step's picker (ds.AppearancePicker), mounted alone:
-- what tests/android_glass.test.luau drives. `on_android` makes the
-- host's `platform` Android, as the APK's is; the picker drawn again.

ui.theme.strict("warn")
local ds = require("@ds")
ds.setup()

local scheme = ui.state("picker_fixture_scheme", "system")
-- (One item, its key the number of times drawn: a new key builds it again.)
local drawn = ui.state("picker_fixture_drawn", { { n = 0 } } :: any)

ui.command {
	id = "on_android",
	title = "The picker as the phone shows it",
	run = function()
		ui.shared("platform", ""):set("android")
		drawn:set({ { n = drawn:peek()[1].n + 1 } })
	end,
}

return ui.column {
	w = 640,
	bg = "$background",
	ui.each(drawn, function(_item: any): any
		return ds.AppearancePicker { scheme = scheme :: any, frosted = ds.material.frosted, accent = ds.look.accent }
	end, { key = function(item: any): any return item.n end }),
}
