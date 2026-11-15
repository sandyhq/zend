-- title: Pull request
-- category: code
-- description: A pull request: state, branch, checks and review.
-- tags: pull request, pr, checks, review
return function()
	return ui.kit.PullRequest {
		number = 412, title = "Retry sends with exponential backoff", state = "open",
		author = "ana", branch = "retry-backoff", base = "main", added = 90, removed = 55,
		checks = {
			{ name = "build", status = "success", duration = "1m 12s" },
			{ name = "unit", status = "success", duration = "2m 10s" },
			{ name = "e2e (chromium)", status = "failure", duration = "4m 02s" },
			{ name = "deploy preview", status = "pending" },
		},
		review = { state = "changes-requested", reviewers = { "bo" } },
	}
end
