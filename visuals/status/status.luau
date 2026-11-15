-- title: Launch health
-- category: status
-- description: Health of several pieces of work: status, owner, due date, progress and a note.
-- tags: status, health, project
return function()
	return ui.kit.Status { items = {
		{ name = "Marketing site", status = "on-track", progress = 70, owner = "Marta Rossi", due = "2026-10-16", note = "Pricing page in review; the rest is live on staging." },
		{ name = "Onboarding", status = "at-risk", progress = 40, owner = "Jon Park", due = "2026-10-15", note = "Needs a decision on the checklist order by Monday." },
		{ name = "Waitlist invites", status = "blocked", progress = 10, owner = "Marta Rossi", due = "2026-10-23" },
		{ name = "Load testing", status = "not-started", owner = "Dev Patel", due = "2026-10-20" },
		{ name = "Billing v2", status = "done", progress = 100, owner = "Jon Park" },
	} }
end
