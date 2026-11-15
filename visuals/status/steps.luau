-- title: Release steps
-- category: status
-- description: A sequence with done, current and blocked steps.
-- tags: steps, sequence, progress
return function()
	return ui.kit.Card {
		title = "Release 3.0",
		ui.kit.Steps { items = {
			{ title = "Freeze", detail = "Thursday noon", status = "done" },
			{ title = "QA", detail = "Smoke tests on staging", status = "current" },
			{ title = "Ship", detail = "Waiting on the App Store review", status = "blocked" },
			{ title = "Announce" },
		} },
	}
end
