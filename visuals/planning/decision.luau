-- title: Which job queue
-- category: planning
-- description: Options scored against weighted criteria; the reader can try other weights and the ranking follows.
-- tags: decision, matrix, weights, compare
return function()
	return ui.kit.Decision {
		options = {
			{ name = "pg-boss", scores = { 4, 5, 4, 3, 4 }, pros = { "Runs on the Postgres we already operate", "Jobs commit in the same transaction as the row" }, cons = { "Polling adds load at high volume" } },
			{ name = "BullMQ", scores = { 5, 2, 4, 5, 4 }, pros = { "Fastest of the four", "Rate limiting and flows built in" }, cons = { "Needs Redis with persistence, which we don't run" } },
			{ name = "Graphile Worker", scores = { 4, 5, 4, 2, 3 }, pros = { "LISTEN/NOTIFY, no polling delay" }, cons = { "Smaller community" } },
			{ name = "SQS + Lambda", scores = { 5, 3, 3, 4, 2 }, cons = { "At-least-once only; we must dedupe" }, note = "Would tie the worker to AWS." },
		},
		criteria = {
			{ name = "Throughput", note = "~40 jobs/s at peak" },
			{ name = "Ops cost", weight = 3, note = "two-person team" },
			{ name = "Delivery guarantees", weight = 2 },
			{ name = "Ecosystem and docs" },
			{ name = "Local development", weight = 2 },
		},
	}
end
