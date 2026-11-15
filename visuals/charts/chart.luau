-- title: Charts of every type
-- category: charts
-- description: Bar, line, area, scatter, pie, radar, funnel, heatmap, histogram and box charts from one call each.
-- tags: chart, line, bar, pie, radar, funnel, heatmap, histogram, box
local s = ui.state { type = "line" }
local types = { "line", "bar", "area", "pie", "radar", "funnel", "heatmap", "histogram", "box" }
local data = {
	line = { labels = { "W1", "W2", "W3", "W4", "W5", "W6" }, series = { { name = "Web", values = { 820, 940, 910, 1080, 1210, 1340 } }, { name = "iOS", values = { 310, 360, 420, 470, 560, 610 } } } },
	bar = { labels = { "Mon", "Tue", "Wed", "Thu", "Fri" }, series = { { name = "Deploys", values = { 4, 7, 5, 9, 3 } } } },
	area = { labels = { "Q1", "Q2", "Q3", "Q4" }, series = { { name = "Revenue", values = { 1.2, 1.8, 2.1, 2.9 } } }, unit = "M" },
	pie = { labels = { "Coding", "Meetings", "Review", "Email" }, series = { 20, 12, 6, 4 }, unit = "h" },
	radar = { labels = { "Battery", "Display", "Performance", "Weight", "Ports", "Price" }, series = { { name = "Air 13", values = { 9, 8, 7, 9, 5, 7 } }, { name = "Pro 14", values = { 8, 9, 9, 6, 8, 5 } } } },
	funnel = { labels = { "Visited pricing", "Started trial", "Activated", "Paid" }, series = { 12400, 5210, 2380, 610 } },
	heatmap = { labels = { "00", "03", "06", "09", "12", "15", "18", "21" }, series = { { name = "Mon", values = { 1, 0, 2, 14, 22, 19, 9, 3 } }, { name = "Tue", values = { 0, 0, 3, 17, 25, 21, 12, 4 } }, { name = "Wed", values = { 2, 1, 2, 15, 28, 24, 10, 2 } }, { name = "Thu", values = { 1, 0, 4, 18, 26, 30, 14, 6 } }, { name = "Fri", values = { 0, 0, 1, 12, 19, 11, 4, 1 } } } },
	histogram = { labels = {}, unit = "ms", series = { { name = "Before", values = { 142, 155, 161, 168, 170, 174, 176, 179, 181, 183, 185, 188, 190, 192, 195, 198, 201, 204, 208, 213, 219, 224, 231, 238, 246, 259, 272, 289, 311, 348 } }, { name = "After", values = { 61, 68, 72, 75, 77, 79, 81, 83, 84, 86, 88, 89, 91, 93, 94, 96, 98, 101, 104, 107, 111, 115, 120, 126, 133, 141, 152, 168, 190, 236 } } } },
	box = { unit = "s", series = { { name = "esbuild", values = { 1.8, 1.9, 1.9, 2.0, 2.0, 2.1, 2.1, 2.2, 2.2, 2.3, 2.4, 3.9 } }, { name = "vite", values = { 4.1, 4.3, 4.4, 4.6, 4.7, 4.8, 4.9, 5.0, 5.2, 5.3, 5.6, 5.9 } }, { name = "webpack 5", values = { 9.8, 10.4, 10.9, 11.2, 11.5, 11.8, 12.1, 12.6, 13.0, 13.4, 14.1, 19.7 } } } },
}
return function()
	local d = data[s.type]
	return ui.kit.Card {
		title = "Chart: " .. s.type,
		ui.tabs { options = types, value = s.type, on_change = function(v) s.type = v end },
		ui.kit.Chart { type = s.type, labels = d.labels, series = d.series, unit = d.unit },
	}
end
