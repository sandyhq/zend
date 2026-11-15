-- title: Screens in this release
-- category: media
-- description: Several pictures in a grid, cut to one height, each with its caption.
-- tags: gallery, images, screenshots, grid, assets
-- (Show it with assets = { ["before.png"] = ..., ["after.png"] = ..., ["phone.png"] = ... }.)
return function()
	return ui.kit.Gallery {
		columns = 3,
		height = 120,
		items = {
			{ src = "before.png", caption = "Desktop, before" },
			{ src = "after.png", caption = "Desktop, after" },
			{ src = "phone.png", caption = "Phone" },
		},
	}
end
