-- title: Before and after
-- category: dev
-- description: Each measure before and after; a change is better, worse or unchanged by which way is better.
-- tags: benchmark, performance, before, after
return function()
	return ui.kit.Benchmark {
		title = "Search service",
		results = {
			{ name = "Cold start", before = 412, after = 298, unit = "ms", stddev = 14 },
			{ name = "p95 /api/search", before = 184, after = 191, unit = "ms", stddev = 9 },
			{ name = "p99 /api/search", before = 402, after = 455, unit = "ms", stddev = 12 },
			{ name = "Throughput", before = 1840, after = 2310, unit = "req/s", better = "higher" },
			{ name = "Peak memory", before = 212, after = 213, unit = "MB" },
			{ name = "Test suite", before = 48.6, after = 49.1, unit = "s", note = "vitest run, 1,214 tests" },
		},
	}
end
