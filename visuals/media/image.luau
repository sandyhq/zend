-- title: The new dashboard
-- category: media
-- description: One picture shipped with the visual (show_visual's assets), its caption under it.
-- tags: image, picture, screenshot, assets
-- (Show it with assets = { ["dashboard.png"] = "<path to the file>" }.)
return function()
	return ui.kit.Image {
		src = "dashboard.png",
		alt = "The dashboard: a sidebar, three stat cards and a line chart",
		caption = "The dashboard after the redesign, at 320 × 200.",
	}
end
