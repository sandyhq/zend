-- title: Files to change
-- category: code
-- description: A file to look at: its folder faint, its name bold, an optional line.
-- tags: file, path, location
return function()
	return ui.kit.Card {
		title = "Files to change",
		ui.kit.File { path = "src/worker/waitlist.ts", line = 528 },
		ui.kit.File { path = "src/website/waitlist-api.ts", note = "validate the email here too" },
	}
end
