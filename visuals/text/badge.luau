-- title: Check results
-- category: text
-- description: A one- to three-word status beside what it explains.
-- tags: badge, status, label
return function()
	return ui.column { gap = 10,
		ui.row { gap = 10, ui.kit.Badge { text = "Passing", tone = "good" }, ui.text { "unit tests, 1,204 of 1,204" } },
		ui.row { gap = 10, ui.kit.Badge { text = "2 failing", tone = "bad" }, ui.text { "e2e, upload and resume" } },
		ui.row { gap = 10, ui.kit.Badge { text = "Skipped" }, ui.text { "visual diffs, no baseline yet" } },
	}
end
