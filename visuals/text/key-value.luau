-- title: Pull request facts
-- category: text
-- description: Labels and values in two columns; values may be nodes.
-- tags: key-value, facts, properties
return function()
	return ui.kit.Card {
		title = "Pull request #482",
		ui.kit.KeyValue { pairs = {
			{ "Branch", "fix/upload-race" },
			{ "Author", "sam" },
			{ "Checks", ui.kit.Badge { text = "2 failing", tone = "bad" } },
			{ "Updated", "2 hours ago" },
		} },
	}
end
