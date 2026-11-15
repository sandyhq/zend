-- title: Review findings
-- category: dev
-- description: Review, lint or type errors by severity, with where, why and a fix.
-- tags: findings, review, lint, severity
return function()
	return ui.kit.Findings {
		title = "Review: map math and test durations",
		items = {
			{ severity = "critical", title = "load_report opens any path it is given", file = "data/src/durations.py", line = 48, column = 14, rule = "path-traversal",
				detail = "The report path comes from the test runner's arguments. Resolve it under the workspace before opening.",
				fix = "root = Path(workspace).resolve()\ntarget = (root / path).resolve()\nif root not in target.parents:\n    return {}" },
			{ severity = "high", title = "fitView drops the zoom it computed", file = "data/src/mapMath.ts", line = 74, column = 20,
				detail = "zoom is computed on line 73 but never passed to clampView.",
				fix = "--- a/data/src/mapMath.ts\n+++ b/data/src/mapMath.ts\n@@ -73,2 +73,2 @@\n   const zoom = Math.min(maxZoom, zoomFor(w, h));\n-  return clampView({ x, y }, size);\n+  return clampView({ x, y, zoom }, size);" },
			{ severity = "medium", title = "Loose equality in zoomFor", file = "data/src/mapMath.ts", line = 72, rule = "eqeqeq", status = "fixed" },
			{ severity = "info", title = "summarize re-reads every status", detail = "Fine at today's sizes; use a Counter if reports grow." },
		},
	}
end
