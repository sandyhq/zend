-- title: Launch board
-- category: planning
-- description: Tasks in columns by status, with owner, due date, priority and labels.
-- tags: board, kanban, tasks, status
return function()
	return ui.kit.Board {
		items = {
			{ key = "LCH-31", title = "Pricing page: annual toggle and EU VAT note", status = "in_progress", priority = "high", labels = { "web", "billing" }, owner = "Marta Rossi", due = "2026-10-14", agentsWorking = 2 },
			{ key = "LCH-28", title = "Onboarding checklist in the empty project view", status = "in_review", priority = "medium", labels = { "app" }, owner = "Jon Park", due = "2026-10-12" },
			{ key = "LCH-25", title = "Press kit: screenshots at 2x for light and dark", status = "in_progress", priority = "medium", owner = "Ana Lima", due = "2026-10-07" },
			{ key = "LCH-33", title = "Status page on status.orbit.example.com", status = "todo", priority = "low", labels = { "infra" }, due = "2026-10-23" },
			{ key = "LCH-34", title = "Load test the signup flow at 5x launch traffic", status = "todo", priority = "urgent", labels = { "infra", "launch-blocker" }, owner = "Dev Patel", due = "2026-10-16" },
			{ key = "LCH-29", title = "Waitlist invite emails (3 waves)", status = "blocked", priority = "high", owner = "Marta Rossi", due = "2026-10-09", detail = "Waiting on the sending domain's DKIM record." },
			{ key = "LCH-21", title = "Brand refresh: logo, colour tokens, favicon", status = "done", owner = "Ana Lima" },
			{ key = "LCH-19", title = "Privacy policy and DPA reviewed by counsel", status = "done", owner = "Jon Park" },
			{ key = "LCH-36", title = "Product Hunt listing copy", status = "backlog", priority = "none", labels = { "marketing" } },
		},
	}
end
