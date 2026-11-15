-- title: Bundle size
-- category: status
-- description: Key numbers with a note, two to four in a row.
-- tags: metric, number, kpi
return function()
	return ui.kit.Card {
		title = "Bundle size",
		ui.row { gap = 32, wrap = true,
			ui.kit.Metric { label = "JavaScript", value = "184 KB", note = "gzipped", tone = "good" },
			ui.kit.Metric { label = "CSS", value = "22 KB" },
			ui.kit.Metric { label = "Change", value = "−31%", note = "vs last release", tone = "good" },
		},
	}
end
