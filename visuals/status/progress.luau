-- title: Migration progress
-- category: status
-- description: Labelled bars: a percent, or value of max.
-- tags: progress, bar, percent
return function()
	return ui.kit.Card {
		title = "Migration",
		ui.kit.Progress { value = 68, label = "Rows copied", tone = "info" },
		ui.kit.Progress { value = 3, max = 12, label = "Tables verified" },
	}
end
