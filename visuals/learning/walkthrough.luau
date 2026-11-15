-- title: Undo a pushed commit
-- category: learning
-- description: A procedure shown one step at a time, each with an explanation and a command to copy.
-- tags: walkthrough, steps, tutorial, git
return function()
	return ui.kit.Walkthrough {
		steps = {
			{ title = "Find the commit", body = "List recent commits on the branch and copy the hash of the one to undo.", code = "git log --oneline -5", language = "bash" },
			{ title = "Revert it", body = "git revert adds a new commit that applies the inverse change, so everyone who already pulled stays in sync.", code = "git revert 3f2a91c", language = "bash" },
			{ title = "Resolve conflicts if any", body = "If later commits touched the same lines, Git stops and marks the conflicts. Fix the files, then continue:", code = "git add -A\ngit revert --continue", language = "bash" },
			{ title = "Push", body = "Push the revert like any other commit. No force push is needed.", code = "git push", language = "bash" },
		},
	}
end
