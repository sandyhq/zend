-- title: Disk usage
-- category: status
-- description: A dial from min to max.
-- tags: gauge, dial, meter
return function()
	return ui.row { gap = 24,
		ui.column { grow = true, ui.kit.Gauge { value = 72, label = "Disk used, %", tone = "warn" } },
		ui.column { grow = true, ui.kit.Gauge { value = 31, label = "CPU, %", tone = "good" } },
	}
end
