-- title: Billing yearly
-- category: inputs
-- description: A yes or no the reader sets.
-- tags: toggle, check, boolean
return function()
	local annual = ui.entry("annual")
	return ui.kit.Form {
		title = "Pro plan",
		ui.kit.Toggle { name = "annual", label = "Bill yearly (2 months free)" },
		ui.kit.Metric { label = "Price", value = annual and "$240/yr" or "$24/mo", tone = "good" },
	}
end
