-- title: Lines of a file
-- category: code
-- description: An excerpt with its path and line numbers, coloured by the file's language; from is the number of its first line, marks picks lines out.
-- tags: snippet, source, lines
return function()
	return ui.kit.Card {
		title = "Where the retry happens",
		ui.kit.Snippet {
			path = "src/api/send.ts", from = 4, marks = { [7] = "focus" },
			code = "export async function send(body: Body) {\n  for (let attempt = 0; attempt < 5; attempt++) {\n    try { return await post(body); }\n    catch { await sleep(2 ** attempt * 100); }\n  }\n}",
		},
	}
end
