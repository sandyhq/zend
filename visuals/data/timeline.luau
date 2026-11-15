-- title: Incident timeline
-- category: data
-- description: What happened when, each moment with a time, a title, a detail and a tone.
-- tags: timeline, incident, events
return function()
	return ui.kit.Timeline {
		items = {
			{ time = "09:12", title = "Deploy started", detail = "v2.4.0 to production" },
			{ time = "09:20", title = "Error rate up", detail = "5xx at 2.1%", tone = "bad" },
			{ time = "09:31", title = "Rolled back", tone = "warn" },
			{ time = "09:40", title = "Recovered", tone = "good" },
		},
	}
end
