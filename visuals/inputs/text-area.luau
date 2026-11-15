-- title: Bug report
-- category: inputs
-- description: Several lines of text from the reader.
-- tags: textarea, text, notes
return function()
	return ui.kit.Form {
		title = "What happened?",
		ui.kit.TextArea { name = "steps", label = "Steps to reproduce", placeholder = "1. Open the settings page\n2. ..." },
	}
end
