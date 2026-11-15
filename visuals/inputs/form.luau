-- title: Release questions
-- category: inputs
-- description: A few questions before a release, with one button that puts the answers into the message.
-- tags: form, questions, answer
return function()
	return ui.kit.Form {
		title = "Before we ship 3.0",
		description = "Three questions; the answers go into your message.",
		submit = "Add answers",
		ui.kit.Choice { name = "audience", label = "Who gets it first?", options = { "Everyone", "Waitlist", "Beta testers" }, value = "Waitlist" },
		ui.kit.Checklist { name = "channels", label = "Announce on", options = { "Email", "Push", "In-app" } },
		ui.kit.TextArea { name = "notes", label = "Anything else?", placeholder = "Optional" },
	}
end
