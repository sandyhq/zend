-- title: Before and after the redesign
-- category: media
-- description: Two pictures over each other; drag the line to wipe from one to the other.
-- tags: compare, before, after, wipe, images, assets
-- (Show it with assets = { ["before.png"] = ..., ["after.png"] = ... }.)
return function()
	return ui.kit.Compare {
		before = "before.png",
		after = "after.png",
		alt = "The dashboard in grey, then in colour",
		labels = { "Before", "After" },
		caption = "Same layout; the stat cards and the chart get their colours back.",
	}
end
