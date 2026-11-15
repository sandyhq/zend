-- title: Node error with a cause
-- category: dev
-- description: An error and its stack: library frames faint, your own frames normal.
-- tags: stack trace, error, exception
return function()
	return ui.kit.StackTrace {
		title = "Crash on startup",
		trace = [[Error: Cannot read config
    at loadConfig (src/config.js:18:11)
    at main (src/index.js:7:3)
    at Module._compile (node:internal/modules/cjs/loader:1256:14)
    at node_modules/ts-node/dist/index.js:851:12
Caused by: SyntaxError: Unexpected token } in JSON at position 41
    at JSON.parse (<anonymous>)
    at readJson (src/config.js:9:15)]],
	}
end
