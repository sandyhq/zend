-- title: Endpoint latency trends
-- category: charts
-- description: A tiny trend line beside a number, for a table row.
-- tags: sparkline, trend, table
return function()
	return ui.table {
		columns = { "Endpoint", "Last 24 h", "p95" },
		rows = {
			{ "/api/search", ui.kit.Sparkline { values = { 120, 118, 130, 125, 160, 210, 190, 140 }, tone = "warn" }, "210 ms" },
			{ "/api/users", ui.kit.Sparkline { values = { 40, 42, 41, 39, 43, 40, 38, 41 }, tone = "good" }, "43 ms" },
			{ "/api/orders", ui.kit.Sparkline { values = { 80, 85, 90, 140, 200, 260, 310, 330 }, tone = "bad" }, "330 ms" },
		},
	}
end
