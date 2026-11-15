-- title: What the change touches
-- category: drawing
-- description: A file tree from paths, with a note on a branch.
-- tags: tree, files, outline
return function()
	return ui.kit.Card {
		title = "What the change touches",
		ui.kit.Tree { items = {
			"src/worker/research.ts",
			"src/worker/index.ts",
			"src/research/forms.ts",
			"src/research/page.ts",
			{ label = "migrations", children = { "0004_answers.sql" }, note = "new" },
		} },
	}
end
