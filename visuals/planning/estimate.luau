-- title: Effort estimate
-- category: planning
-- description: Three-point estimates per task, summed into a total with even odds and nine-in-ten.
-- tags: estimate, effort, pert, planning
return function()
	return ui.kit.Estimate {
		unit = "d",
		tasks = {
			{ title = "Environments RPCs in server.ts (list, status, PR)", low = 1, likely = 1.5, high = 3 },
			{ title = "Lazy per-row git status and PR, a few at a time", low = 1, likely = 2, high = 4 },
			{ title = "Dashboard UI: rows, live status, ahead/behind", low = 2, likely = 3, high = 5 },
			{ title = "Polling with backoff, only while visible", low = 0.5, likely = 1, high = 2 },
			{ title = "Destroyed and non-git environments", low = 0.5, high = 3 },
		},
	}
end
