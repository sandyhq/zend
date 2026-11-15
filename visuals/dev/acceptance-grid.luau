-- title: Claimed against checked
-- category: dev
-- description: A grid of criteria by who or what checked them.
-- tags: acceptance, grid, parity
return function()
	return ui.kit.Acceptance {
		title = "Parity with the design",
		columns = { "Agent says", "Browser check", "Owner" },
		criteria = {
			{ name = "Header spacing", cells = { ["Agent says"] = "passed", ["Browser check"] = "passed", Owner = "passed" } },
			{ name = "Empty state", cells = { ["Agent says"] = "passed", ["Browser check"] = { status = "failed", note = "Icon is 4px lower than the design" }, Owner = "untested" } },
			{ name = "Dark mode", cells = { ["Agent says"] = "claimed", ["Browser check"] = "untested", Owner = "untested" } },
		},
	}
end
