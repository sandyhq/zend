-- title: A day west of Lisbon
-- category: drawing
-- description: Places on a coordinate plot, joined in order, with a note each.
-- tags: map, route, places, trip
return function()
	return ui.kit.Map {
		route = true,
		places = {
			{ name = "Belém Tower, Lisbon", lat = 38.6916, lng = -9.2160, note = "Go at opening, 9:30, before the coaches." },
			{ name = "Pena Palace, Sintra", lat = 38.7876, lng = -9.3906, note = "Book the 10:30 slot; the park takes two hours." },
			{ name = "Cabo da Roca", lat = 38.7804, lng = -9.4989, note = "Westernmost point of mainland Europe. Windy." },
			{ name = "Cascais", lat = 38.6979, lng = -9.4215, note = "Dinner by the marina; train back to Cais do Sodré." },
		},
	}
end
