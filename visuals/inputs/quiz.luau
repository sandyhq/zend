-- title: Git basics quiz
-- category: inputs
-- description: Multiple-choice questions that say after each pick whether it was right and why, with a score.
-- tags: quiz, questions, learning, score
return function()
	return ui.column {
		gap = 16,
		ui.kit.Quiz {
			name = "git",
			label = "Git basics",
			questions = {
				{
					q = "Which command shows changes not yet staged?",
					options = { "git diff", "git diff --staged", "git log -p" },
					answer = 1,
					why = "git diff compares the working tree with the index; --staged compares the index with HEAD.",
				},
				{
					q = "What does git rebase -i let you do?",
					options = { "Clone a repository", "Reorder, squash or edit commits", "Delete a remote branch" },
					answer = 2,
				},
				{
					q = "Which is safe on a branch others have pulled?",
					options = { "git push --force", "git revert", "git reset --hard origin~3" },
					answer = "git revert",
					why = "A revert adds a commit; the others rewrite history others already have.",
				},
			},
		},
		ui.answer { label = "Add my answers" },
	}
end
