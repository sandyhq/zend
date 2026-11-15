-- title: Starter features
-- category: inputs
-- description: Options with a title and a line of detail; a list value picks several.
-- tags: options, cards, multi-select
return function()
	return ui.kit.Form {
		title = "What should the starter include?",
		ui.kit.OptionCards {
			name = "features",
			label = "Pick any",
			value = { "sync" },
			options = {
				{ value = "auth", title = "Sign-in", detail = "Email links and passkeys" },
				{ value = "sync", title = "Offline sync", detail = "Local-first, merges conflicts" },
				{ value = "search", title = "Search", detail = "Full-text over every note" },
				{ value = "ci", title = "CI pipeline", detail = "Tests and previews on every push" },
			},
		},
	}
end
