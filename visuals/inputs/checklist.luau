-- title: Launch checklist
-- category: inputs
-- description: Several options the reader ticks.
-- tags: checklist, multi-select
return function()
	return ui.kit.Form {
		ui.kit.Checklist {
			name = "done",
			label = "What is done already?",
			options = { "Changelog written", "Migration tested", "Docs updated", "Status page ready" },
			value = { "Changelog written" },
		},
	}
end
