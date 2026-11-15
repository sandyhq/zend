-- title: Where the week went
-- category: charts
-- description: Shares of one whole as a single bar with a legend.
-- tags: share, bar, breakdown
return function()
	return ui.kit.Card {
		title = "Where the week went",
		ui.kit.SegmentedBar {
			unit = "h",
			parts = {
				{ label = "Deep work", value = 18 },
				{ label = "Meetings", value = 9.5 },
				{ label = "Code review", value = 6 },
				{ label = "Admin", value = 3, tone = "neutral" },
			},
		},
	}
end
