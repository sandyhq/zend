-- title: Transcription errors
-- category: data
-- description: What was said against what each tool wrote, word by word, with the word error rate.
-- tags: transcript, speech, wer, diff
return function()
	return ui.kit.Transcript {
		tracks = {
			{ name = "What I said", text = "Yes I can hear you fine. Great, we have 25 minutes. Then let us look at the transcript at nineteen forty nine." },
			{ name = "whisper.cpp base.en", text = "Yes I can hear you fine. Great, we have twenty five minutes. Then let us look at the transcript." },
			{ name = "whisper.cpp tiny.en", text = "Yes I can hear your fine. Great we have 25 minute. Than let us look at the script at nineteen forty nine." },
		},
	}
end
