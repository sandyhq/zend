-- title: Waitlist go-live
-- category: dev
-- description: Is it really done: each criterion with who checked it, how, and the evidence.
-- tags: acceptance, done, evidence, verify
return function()
	return ui.kit.Acceptance {
		title = "Waitlist: go live",
		criteria = {
			{ name = "Mail provider 2FA on", status = "passed", by = "owner", at = "2026-10-08", method = "Logged out and in; a code was asked" },
			{ name = "Rate limit on /api/waitlist", status = "verified", by = "agent", method = "12 quick curls from one machine", note = "Requests 11–14 in 10 s answered 429; other paths unaffected.", evidence = "docs/evidence/rate-limit.md" },
			{ name = "Sender domain authenticated", status = "claimed", by = "agent", note = "The API says verified; no test mail has arrived yet." },
			{ name = "Hero action opens the email field", status = "not tested", note = "The action changed; the browser smoke predates it." },
			{ name = "Public TestFlight link capped", status = "failed", note = "The link is still open to everyone." },
		},
	}
end
