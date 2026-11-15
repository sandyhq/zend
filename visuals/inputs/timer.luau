-- title: Standup timer
-- category: inputs
-- description: A countdown with start, pause and reset (or a stopwatch without seconds); it keeps nothing.
-- tags: timer, countdown, stopwatch, clock
return function()
	return ui.column {
		gap = 20,
		ui.kit.Timer { label = "Standup, 15 minutes", seconds = 15 * 60 },
		ui.kit.Timer { label = "Time on this incident" },
	}
end
