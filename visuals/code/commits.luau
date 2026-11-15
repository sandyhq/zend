-- title: Branch commits
-- category: code
-- description: A branch's commits: short sha, title, who and when, lines changed.
-- tags: commits, git, history, branch
return function()
	return ui.kit.Commits {
		title = "Since main",
		commits = {
			{ sha = "9f1c2abd41e0", title = "Retry sends with exponential backoff", author = "Ana", date = "Oct 8", files = 2, added = 52, removed = 3 },
			{ sha = "3b7e90c11aa2", title = "Add a test for the retry limit", author = "Ana", date = "Oct 8", files = 1, added = 38, removed = 0 },
			{ sha = "c04d5521be7f", title = "Remove the legacy sender", author = "Bo", date = "Oct 7", files = 1, added = 0, removed = 52 },
		},
	}
end
