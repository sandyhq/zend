-- title: A unified diff
-- category: code
-- description: A patch with line numbers, additions green and removals red.
-- tags: diff, patch, change
return function()
	return ui.kit.Diff {
		title = "The fix",
		patch = "--- a/src/api/users.ts\n+++ b/src/api/users.ts\n@@ -40,3 +40,3 @@\n export function display(user: User) {\n-  return user.email.toLowerCase();\n+  return (user.email ?? \"\").toLowerCase();\n }",
	}
end
