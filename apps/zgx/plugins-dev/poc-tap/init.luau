-- POC tap: a button after the sidebar's New chat and Search ("ui.add");
-- each tap counts, its words say how often and it turns from orange to
-- green. The Android POC's
-- plugin (plans/android-zgx-poc): this folder, unchanged, on the desktop
-- and on the phone.

local zend = require("./zend")("poc-tap")

local taps = ui.state("taps", 0)

zend.place {
	at = "#sidebar-actions",
	after = function(): Node
		return ui.button {
			id = "poc-tap",
			label = "Count a tap",
			w_full = true,
			mt = "$space.2",
			px = "$space.3",
			py = "$space.2",
			rounded = "$radius.md",
			cursor = "pointer",
			bg = function(): string
				return if taps:get() == 0 then "$palette.orange.600" else "$palette.emerald.600"
			end,
			color = "$accent.text",
			on_click = function()
				taps:set(taps:peek() + 1)
			end,
			ui.text {
				function(): string
					local n = taps:get()
					return if n == 0 then "Tap me" elseif n == 1 then "Tapped once" else `Tapped {n} times`
				end,
				text_size = "$font.size.sm",
				weight = "$font.weight.medium",
				tabular = true,
			},
		}
	end,
}
