-- title: A block of code
-- category: code
-- description: Code as written, with its language and a title.
-- tags: code, block, listing
return function()
	return ui.kit.Code {
		title = "Retry with backoff", language = "typescript",
		code = "for (let attempt = 0; attempt < 5; attempt++) {\n  try { return await send(); }\n  catch { await sleep(2 ** attempt * 100); }\n}",
	}
end
