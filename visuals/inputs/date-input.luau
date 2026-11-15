-- title: Release date
-- category: inputs
-- description: A date as yyyy-mm-dd.
-- tags: date, input
return function()
	return ui.kit.Form {
		ui.kit.DateInput { name = "release", label = "Release on", value = "2026-12-01" },
	}
end
