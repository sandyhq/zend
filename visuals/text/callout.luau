-- title: Before you upgrade
-- category: text
-- description: A note to heed, with a tone and an optional title.
-- tags: callout, note, warning
return function()
	return ui.kit.Card {
		title = "Before you upgrade",
		ui.kit.Callout { title = "Irreversible migration", text = "Back up the database first: the migration rewrites the orders table.", tone = "warn" },
		ui.kit.Callout { text = "The upgrade takes about 4 minutes on a 2 GB database.", tone = "info" },
	}
end
