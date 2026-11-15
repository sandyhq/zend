-- title: Billing period
-- category: inputs
-- description: One option, each with a line of detail.
-- tags: radio, choice, options
return function()
	return ui.kit.Form {
		ui.kit.Radio {
			name = "plan",
			label = "How often should we bill you?",
			options = {
				{ value = "monthly", detail = "Cancel any time" },
				{ value = "yearly", detail = "Two months free" },
				{ value = "lifetime", detail = "One payment" },
			},
			value = "monthly",
		},
	}
end
