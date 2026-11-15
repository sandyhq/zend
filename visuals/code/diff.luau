-- title: A unified diff
-- category: code
-- description: A patch with line numbers, each file's lines coloured by its language, additions green and removals red.
-- tags: diff, patch, change
return function()
	return ui.kit.Diff {
		title = "The fix",
		patch = "--- a/src/api/users.ts\n+++ b/src/api/users.ts\n@@ -40,3 +40,3 @@\n export function display(user: User) {\n-  return user.email.toLowerCase();\n+  return (user.email ?? \"\").toLowerCase();\n }\n--- a/tests/users_test.py\n+++ b/tests/users_test.py\n@@ -12,2 +12,4 @@\n def test_display():\n     assert display(alice) == \"alice@x.io\"\n+    # A user without an email shows nothing.\n+    assert display(bob) == \"\"",
	}
end
