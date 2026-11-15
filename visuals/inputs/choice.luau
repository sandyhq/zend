-- title: Audience
-- category: inputs
-- description: One of a few short options; many = true picks several.
-- tags: choice, single, multi
return function()
	return ui.kit.Form {
		ui.kit.Choice { name = "audience", label = "Who should get it?", options = { "Everyone", "Waitlist", "Beta testers" } },
		ui.kit.Choice { name = "platforms", label = "On which platforms?", options = { "macOS", "Linux", "Windows" }, many = true },
	}
end
