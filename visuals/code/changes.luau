-- title: Files changed
-- category: code
-- description: Files changed with line counts; a file with a diff opens to it.
-- tags: changes, files, git, diff
return function()
	return ui.kit.Changes {
		title = "Retry with backoff",
		files = {
			{ path = "src/api/send.ts", status = "modified", added = 14, removed = 3,
				diff = "--- a/src/api/send.ts\n+++ b/src/api/send.ts\n@@ -10,4 +10,7 @@\n export async function send(body: Body) {\n-  return post(body);\n+  for (let attempt = 0; attempt < 5; attempt++) {\n+    try { return await post(body); }\n+    catch { await sleep(2 ** attempt * 100); }\n+  }\n }" },
			{ path = "src/api/send.test.ts", status = "added", added = 38, removed = 0 },
			{ path = "src/api/legacy.ts", status = "deleted", added = 0, removed = 52 },
		},
	}
end
