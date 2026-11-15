-- title: Project name
-- category: inputs
-- description: One line of text from the reader.
-- tags: input, text, field
return function()
	return ui.kit.Form {
		ui.kit.Input { name = "project", label = "Project name", placeholder = "my-app", description = "It deploys to <name>.workers.dev." },
	}
end
