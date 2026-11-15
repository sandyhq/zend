-- title: HTTP status codes
-- category: learning
-- description: Cards to turn over, one at a time, with next and previous.
-- tags: flashcards, quiz, memorise
return function()
	return ui.kit.Flashcards {
		cards = {
			{ front = "401", back = "Unauthorized: no or bad credentials" },
			{ front = "403", back = "Forbidden: credentials fine, not allowed" },
			{ front = "404", back = "Not found" },
			{ front = "429", back = "Too many requests" },
		},
	}
end
