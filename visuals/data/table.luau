-- title: Hosting options
-- category: data
-- description: A table whose cells may be words, numbers or other kit parts such as badges.
-- tags: table, compare, columns
return function()
	return ui.kit.Table {
		columns = { "Option", { label = "Cost", align = "end" }, "Cold start", "Notes" },
		rows = {
			{ "Workers", "$5/mo", ui.kit.Badge { "~0 ms", tone = "good" }, "One deploy for assets and API" },
			{ "Pages", "Free", ui.kit.Badge { "n/a", tone = "neutral" }, "Static only" },
			{ "VPS", "$6/mo", ui.kit.Badge { "none", tone = "good" }, "You patch the OS" },
		},
	}
end
