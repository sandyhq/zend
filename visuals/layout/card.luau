-- title: Deploy finished
-- category: layout
-- description: A titled section: a heading over its content, no box.
-- tags: card, section, heading
return function()
	return ui.kit.Card {
		title = "Deploy finished",
		subtitle = "web · production · 2 min 14 s",
		ui.row { gap = 32, wrap = true,
			ui.kit.Metric { label = "Requests", value = "18.2k" },
			ui.kit.Metric { label = "Errors", value = "0.02%", tone = "good" },
		},
	}
end
