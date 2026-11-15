-- title: Workout minutes
-- category: data
-- description: One square per day, darker for more; hover a day for its value.
-- tags: heatmap, calendar, activity, contributions
local days = {}
for month = 7, 9 do
	for day = 1, 28 do
		local minutes = (month * day * 7) % 13 * 7
		if minutes > 20 then
			days[string.format("2026-%02d-%02d", month, day)] = minutes
		end
	end
end
return function()
	return ui.kit.CalendarHeatmap { days = days, label = "minutes", tone = "good" }
end
