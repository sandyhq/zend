-- title: Plan to approve
-- category: planning
-- description: Steps the reader approves before work starts; untick a step to drop it, then add the answer to the message.
-- tags: plan, approve, steps, answer
return function()
	return ui.kit.Plan {
		submit = "Approve the plan",
		steps = {
			{ title = "Honour 429 and Retry-After in safeGet", detail = "Return the wait instead of failing, capped at 60 s; other 4xx stay errors.", files = { "web/net.ts:180" }, risk = "medium", estimate = "2h" },
			{ title = "Replace the global limiter with a per-host token bucket", detail = "Commons 10/s, Photon 1/s, OSM tiles 2 in flight.", files = { "web/lookups.ts:48" }, risk = "medium", estimate = "3h" },
			{ title = "Keep 429s out of the cache", detail = "A rate-limited miss must not be stored for a day like a real miss.", files = { "web/cache.ts" }, risk = "high", estimate = "1h" },
			{ title = "Show \"rate limited, retrying\" in Map and ImageSearch", risk = "low", estimate = "2h", optional = true },
			{ title = "Document the limits in the README", estimate = "20m" },
		},
	}
end
