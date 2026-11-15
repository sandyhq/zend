-- title: Service logs
-- category: dev
-- description: A log with levels coloured and repeated lines folded, under the command and its exit code.
-- tags: logs, output, command, docker
return function()
	return ui.kit.Logs {
		command = "docker compose logs shop",
		exit = 1,
		output = {
			"[info] shop api listening on :8080",
			"[info] GET /health 200 2ms",
			"[warn] slow query: SELECT * FROM carts WHERE id = $1 (812ms)",
			"[error] POST /cart/c_42/price 500: Cannot read properties of undefined (reading 'toEur')",
			"    at lineTotal (pricing.mjs:10:46)",
			"[info] GET /health 200 1ms",
			"[info] GET /health 200 1ms",
			"[info] GET /health 200 1ms",
		},
	}
end
