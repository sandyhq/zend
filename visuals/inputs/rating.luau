-- title: Sprint retro ratings
-- category: inputs
-- description: Star ratings from 1 to 5, here how a sprint went on three counts.
-- tags: rating, stars, feedback, retro
return function()
	return ui.kit.Form {
		title = "Sprint 42 retro",
		submit = "Add ratings",
		ui.kit.Rating { name = "focus", label = "Focus on the sprint goal" },
		ui.kit.Rating { name = "estimates", label = "How close the estimates were" },
		ui.kit.Rating { name = "release", label = "How smooth the release was", max = 5 },
	}
end
