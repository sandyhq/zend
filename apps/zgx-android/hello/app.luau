-- G1: one Luau node, and a tap that changes it.
local taps = ui.state("taps", 0)

return ui.column {
	w_full = true,
	h_full = true,
	bg = "#1d2b53",
	items = "center",
	justify = "center",
	gap = 24,
	ui.text { "Hello from zgx on Android", color = "#ffffff", text_size = 32 },
	ui.button {
		id = "tap",
		px = 32,
		py = 16,
		rounded = 12,
		bg = "#ff7a00",
		on_click = function()
			taps:set(taps:get() + 1)
		end,
		ui.text {
			function(): string
				return `Taps: {taps:get()}`
			end,
			color = "#ffffff",
			text_size = 28,
		},
	},
}
