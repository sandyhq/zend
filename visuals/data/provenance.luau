-- title: Is the new build live
-- category: data
-- description: The stages from commit to live, the facts each reported, and any fact that differs from what was asked or from the stage before.
-- tags: provenance, deploy, build, evidence
return function()
	return ui.kit.Provenance {
		expected = { branch = "main", sha = "44f1d749f06f2a0f573f58dbd68289e510ab408b" },
		stages = {
			{ name = "Commit", status = "done", time = "2026-10-10 01:42", facts = { branch = "main", sha = "44f1d749f06f2a0f573f58dbd68289e510ab408b" } },
			{ name = "Build", status = "done", time = "2026-10-10 03:46", duration = "48s", facts = { sha = "44f1d74", host = "build-mini", artifact = "dist/" } },
			{ name = "Deploy to Cloudflare", status = "done", time = "2026-10-10 03:47", duration = "21s", facts = { sha = "44f1d74", url = "https://orbit.example.com" }, evidence = "wrangler deploy --tag 44f1d74" },
			{ name = "Probe live URL", status = "done", time = "2026-10-10 03:48", duration = "1s", facts = { sha = "9b2e1c0", http = "200" }, evidence = { "https://orbit.example.com/" } },
		},
	}
end
