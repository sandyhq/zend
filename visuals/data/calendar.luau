-- title: Release plan calendar
-- category: data
-- description: Events on month grids, a dot per event and the titles listed under each month.
-- tags: calendar, schedule, events, dates
return function()
	return ui.kit.Calendar {
		events = {
			{ date = "2026-10-05", title = "Feature freeze", tone = "warn" },
			{ date = "2026-10-09", title = "RC 1" },
			{ date = "2026-10-09", title = "Docs review" },
			{ date = "2026-10-14", title = "RC 2" },
			{ date = "2026-10-20", title = "Security review", tone = "bad" },
			{ date = "2026-10-20", title = "Changelog draft" },
			{ date = "2026-10-27", title = "Release 3.0", tone = "good" },
			{ date = "2026-11-03", title = "Retro", tone = "neutral" },
			{ date = "2026-11-17", title = "3.0.1 patch" },
		},
	}
end
