-- title: Files to change
-- category: code
-- description: A file to look at: its folder faint, its name bold, an optional line; with code, its lines coloured, that line marked.
-- tags: file, path, location
return function()
	return ui.kit.Card {
		title = "Files to change",
		ui.kit.File {
			path = "src/worker/waitlist.ts", line = 528, from = 526,
			code = "const entry = await db.waitlist.find(email);\nif (entry) return conflict();\nawait db.waitlist.insert({ email, at: Date.now() });",
		},
		ui.kit.File { path = "src/website/waitlist-api.ts", note = "validate the email here too" },
	}
end
