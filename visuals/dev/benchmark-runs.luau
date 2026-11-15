-- title: Benchmark runs
-- category: dev
-- description: Every run with its metrics and status.
-- tags: benchmark, runs, devices
return function()
	return ui.kit.Benchmark {
		title = "Scroll test on devices",
		results = {
			{ name = "iPhone 15", metrics = { fps = 59.4, ["jank frames"] = 2, ["memory MB"] = 310 }, status = "passed" },
			{ name = "Pixel 8", metrics = { fps = 55.1, ["jank frames"] = 9, ["memory MB"] = 402 }, status = "passed" },
			{ name = "Moto G", metrics = { fps = 38.7, ["jank frames"] = 41, ["memory MB"] = 515 }, status = "failed" },
		},
	}
end
