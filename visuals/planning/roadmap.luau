-- title: Launch roadmap
-- category: planning
-- description: A Gantt chart by lane: bars with progress, milestones and what waits on what.
-- tags: roadmap, gantt, schedule, milestones
return function()
	return ui.kit.Roadmap {
		items = {
			{ title = "Brand refresh", start = "2026-09-07", ["end"] = "2026-09-25", lane = "Design", status = "done" },
			{ title = "Marketing site", start = "2026-09-21", ["end"] = "2026-10-16", lane = "Design", status = "active", progress = 70, after = "Brand refresh" },
			{ title = "Press kit", start = "2026-09-28", ["end"] = "2026-10-08", lane = "Design", status = "active", progress = 60, detail = "Founder photos still missing." },
			{ title = "Billing v2", start = "2026-09-14", ["end"] = "2026-10-09", lane = "Product", status = "done" },
			{ title = "Onboarding checklist", start = "2026-10-01", days = 15, lane = "Product", status = "at-risk", progress = 40, detail = "Design changed late; the empty-state copy is not final." },
			{ title = "Waitlist invites", start = "2026-10-12", ["end"] = "2026-10-23", lane = "Product", status = "blocked", after = "Billing v2", detail = "Blocked on the sending domain's DKIM record." },
			{ title = "Load test at 5x", start = "2026-10-13", ["end"] = "2026-10-20", lane = "Infra", status = "planned", after = "Onboarding checklist" },
			{ title = "Public beta", milestone = "2026-10-26", lane = "Launch", after = "Load test at 5x" },
			{ title = "Launch day", milestone = "2026-11-03", lane = "Launch", after = "Public beta" },
		},
	}
end
