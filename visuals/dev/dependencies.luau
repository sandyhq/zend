-- title: Outdated packages
-- category: dev
-- description: Outdated packages: current, latest and how far behind.
-- tags: dependencies, npm, outdated, upgrade
return function()
	return ui.kit.Dependencies {
		report = {
			{ name = "next", current = "14.2.5", latest = "15.0.0", severity = "high", note = "15.0.0 fixes the cache-poisoning advisory" },
			{ name = "react", current = "18.2.0", wanted = "18.3.1", latest = "19.0.0" },
			{ name = "typescript", current = "5.4.5", latest = "5.6.3", type = "dev" },
			{ name = "vitest", current = "2.1.1", latest = "2.1.4", type = "dev" },
			{ name = "zod", current = "3.23.8", latest = "3.23.8" },
			{ name = "left-pad", latest = "1.3.0" },
		},
	}
end
