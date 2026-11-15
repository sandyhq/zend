-- title: Sources
-- category: data
-- description: The pages an answer rests on, numbered so the text can cite them, with a note each.
-- tags: sources, links, citations
return function()
	return ui.kit.Sources {
		links = {
			{ url = "https://en.wikipedia.org/wiki/Bicycle_lighting", title = "Bicycle lighting", note = "Overview of the lighting rules in several countries" },
			{ url = "https://www.gov.uk/guidance/the-highway-code/rules-for-cyclists-59-to-82", title = "The Highway Code: rules for cyclists (59 to 82)", note = "Rule 60 covers lights and reflectors at night" },
			"https://www.cyclinguk.org/article/cycling-guide/lights",
		},
	}
end
