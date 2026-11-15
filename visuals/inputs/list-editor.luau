-- title: Launch risks
-- category: inputs
-- description: A list the reader adds to, edits and trims, here the risks before a launch.
-- tags: list, editor, items, risks
return function()
	return ui.kit.Form {
		title = "Before the 3.0 launch",
		ui.kit.ListEditor {
			name = "risks",
			label = "Risks we should track",
			description = "Edit or remove the ones from the planning doc, add what's missing.",
			placeholder = "Add a risk",
			value = {
				"Payment provider migration not done by the 14th",
				"Docs for the new API still in draft",
				"Only one on-call engineer during launch week",
			},
		},
	}
end
