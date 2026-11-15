-- title: Lines of a file
-- category: code
-- description: An excerpt with its path and line numbers; from is the number of its first line.
-- tags: snippet, source, lines
return function()
	return ui.kit.Card {
		title = "Where the retry happens",
		ui.kit.Snippet { path = "src/api/send.ts", from = 4, code = "export async function send(body: Body) {\n  for (let attempt = 0; attempt < 5; attempt++) {\n    try { return await post(body); }\n    catch { await sleep(2 ** attempt * 100); }\n  }\n}" },
	}
end
