-- title: Worktrees
-- category: code
-- description: A project's worktrees: branch, threads, changes and pull request.
-- tags: environments, worktrees, branches
return function()
	return ui.kit.Environments {
		title = "Worktrees",
		items = {
			{ name = "main", branch = "main", threads = 1 },
			{ name = "retry-backoff", branch = "ana/retry-backoff", current = true, threads = 2, changes = { files = 3, added = 90, removed = 55 }, pr = { number = 412, state = "open" } },
			{ name = "legacy-cleanup", branch = "bo/legacy-cleanup", threads = 1, changes = { files = 1, added = 0, removed = 52 }, pr = { number = 409, state = "merged" } },
		},
	}
end
