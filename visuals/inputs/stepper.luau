-- title: Offsite headcount
-- category: inputs
-- description: A stepper for a small count with minus and plus, here seats to book per team.
-- tags: stepper, count, number, input
return function()
	return ui.kit.Form {
		title = "Seats for the offsite",
		description = "The venue holds 40; booking closes Friday.",
		ui.kit.Stepper { name = "engineering", label = "Engineering", min = 0, max = 40, value = 12 },
		ui.kit.Stepper { name = "design", label = "Design", min = 0, max = 40, value = 4 },
		ui.kit.Stepper { name = "support", label = "Support", min = 0, max = 40, value = 6 },
	}
end
