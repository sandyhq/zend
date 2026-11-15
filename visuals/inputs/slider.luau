-- title: Weekly review budget
-- category: inputs
-- description: A slider for one number in a range, here the hours a team sets aside for review each week.
-- tags: slider, number, range, input
return function()
	return ui.kit.Form {
		title = "Review time",
		submit = "Add answers",
		ui.kit.Slider {
			name = "review_hours",
			label = "Hours per week for code review",
			description = "Across the whole team of six; last quarter it was about 9.",
			min = 0,
			max = 20,
			step = 1,
			value = 9,
		},
	}
end
