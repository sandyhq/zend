-- title: Three ways to host the site
-- category: media
-- description: Cards shown one at a time, with arrows to move between them.
-- tags: carousel, options, cards
return function()
	return ui.kit.Carousel { key = "hosting",
		ui.kit.Card { title = "Workers", ui.kit.Badge { text = "Recommended", tone = "good" }, ui.text { "Static assets and the API ship in one deploy." }, ui.kit.Metric { label = "Cold start", value = "~0 ms" } },
		ui.kit.Card { title = "Pages", ui.text { "Git-driven builds with a preview per branch." }, ui.kit.Metric { label = "Build", value = "~40 s" } },
		ui.kit.Card { title = "A small VPS", ui.text { "Full control; you patch and back it up yourself." }, ui.kit.Metric { label = "Cost", value = "$6/mo" } },
	}
end
