-- title: Trip follow-ups
-- category: inputs
-- description: Suggested next questions under an answer; the one picked goes into the message.
-- tags: follow-ups, suggestions, next
return function()
	return ui.kit.Card {
		title = "Lisbon, five days",
		ui.text { "Five days for two come to about €1,240, flights included. Most of it is the flat (€610) and the flights (€380)." },
		ui.kit.FollowUps { prompts = { "Where can we save the most?", "Plan the five days hour by hour", "How does Porto compare for the same week?" } },
	}
end
