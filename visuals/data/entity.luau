-- title: A product to consider
-- category: data
-- description: A product, place or person: name, kind, a note, facts, and a link.
-- tags: entity, product, facts, card
return function()
	return ui.column {
		gap = 20,
		ui.kit.Entity {
			name = "Shackle 13 U-lock",
			kind = "product",
			facts = { { "Price", "$54" }, { "Weight", "1.1 kg" }, { "Rating", "4.7" }, { "Shackle", "13 mm hardened steel" } },
			url = "https://example.com/shackle-13",
			note = "Fits most frames plus the rear wheel; the sturdiest of the three.",
		},
		ui.kit.Entity { name = "Ana Ruiz", kind = "person", facts = { { "Role", "Ranger" }, { "Walks", "Saturdays, 6 am" } }, note = "Runs the guided dawn walks from the campsite gate." },
	}
end
