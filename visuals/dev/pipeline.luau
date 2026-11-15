-- title: CI run
-- category: dev
-- description: A CI run: stages and jobs, with the failing job's steps open.
-- tags: ci, pipeline, build, github actions
return function()
	return ui.kit.Pipeline {
		run = {
			name = "Deploy web", branch = "release/2.4", sha = "9f1c2ab", duration = "9m 31s",
			stages = {
				{ name = "Build", status = "passed", duration = "1m 12s" },
				{ name = "Test", status = "failed", jobs = {
					{ name = "unit", status = "passed", duration = "2m 10s" },
					{ name = "e2e (chromium)", status = "failed", duration = "4m 02s", steps = {
						{ name = "Install browsers", status = "passed", duration = "38s" },
						{ name = "Run playwright", status = "failed", duration = "3m 20s", message = "3 failed: checkout.spec.ts › pays with a saved card" },
						{ name = "Upload report", status = "passed", duration = "4s" },
					} },
					{ name = "lint", status = "passed", duration = "48s" },
				} },
				{ name = "Deploy", status = "skipped" },
			},
		},
	}
end
