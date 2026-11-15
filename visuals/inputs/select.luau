-- title: Region
-- category: inputs
-- description: One of many options, in a drop-down.
-- tags: select, dropdown
return function()
	return ui.kit.Form {
		ui.kit.Select {
			name = "region",
			label = "Deploy to",
			placeholder = "Pick a region",
			options = { "us-east-1", "us-west-2", "eu-west-1", "eu-central-1", "ap-southeast-1", "ap-northeast-1" },
		},
	}
end
