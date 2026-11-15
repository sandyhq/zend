--!nocheck
-- `ui.kit`, batch a, dev: what tools print and what a change looks like
-- (Acceptance, Findings, StackTrace, Dependencies, Benchmark, Pipeline,
-- Tests, Logs, Coverage, Changes, Commits, Diff, PullRequest, Environments,
-- Snippet, Code, Json, File). Loaded after kit.luau; returns its kinds, which
-- Zend adds to `ui.kit` before it is frozen. The rule is the same as for the
-- core kit (visuals/README.md, "The kit"): one call with data, ordinary `ui`
-- nodes, no boxes, theme tokens, errors named "ui.kit.<Kind> ...".
--
-- The sandbox has no files and no network, so these kinds take the data
-- itself: a report is a table (or, for logs, traces and diffs, the text),
-- never a path to load.
local ui = ...
local kit = {}

local function fail(message)
	error(message, 0)
end

-- Same as kit.luau's `define`: `allowed` are the props; the array part is
-- always allowed. An unknown prop, and anything `build` fails with, comes
-- back as "ui.kit.<Kind> ..." at the line of the agent's call.
local function define(name, allowed, build)
	local known = {}
	for _, key in ipairs(allowed) do
		known[key] = true
	end
	kit[name] = function(props)
		if type(props) ~= "table" or rawget(props, "__kind") ~= nil then
			props = { props }
		end
		for key in pairs(props) do
			if type(key) ~= "number" and not known[key] then
				error(
					string.format(
						"ui.kit.%s has no prop '%s' (it takes %s)",
						name,
						tostring(key),
						#allowed > 0 and table.concat(allowed, ", ") or "only children"
					),
					2
				)
			end
		end
		local ok, node = pcall(build, props)
		if not ok then
			error("ui.kit." .. name .. " " .. tostring(node), 2)
		end
		return node
	end
end

local TONES = {
	neutral = "muted",
	good = "success",
	warn = "warning",
	bad = "danger",
	info = "accent",
}

-- Joins children onto `props`, skipping nils (a nil in a table constructor
-- ends the array part there).
local function build(constructor, props, ...)
	for i = 1, select("#", ...) do
		local child = select(i, ...)
		if child ~= nil then
			table.insert(props, child)
		end
	end
	return constructor(props)
end

-- Helpers -------------------------------------------------------------------

local function is_list(value)
	return type(value) == "table" and rawget(value, "__kind") == nil and (#value > 0 or next(value) == nil)
end

local function need_list(value, what)
	if type(value) ~= "table" then
		fail("needs " .. what)
	end
	return value
end

local function strip_ansi(s)
	return (s:gsub("\27%[[%d;]*[A-Za-z]", ""))
end

local function split_lines(s)
	s = strip_ansi(tostring(s)):gsub("\r\n", "\n")
	local out = {}
	for line in (s .. "\n"):gmatch("(.-)\n") do
		table.insert(out, line)
	end
	while #out > 0 and out[#out] == "" do
		table.remove(out)
	end
	return out
end

local function plain(line)
	return (line:gsub("\t", "    "))
end

local function heading(title, summary)
	if title == nil and not summary then
		return false
	end
	return build(
		ui.column,
		{ gap = 2 },
		title and ui.text({ title, size = "lg", weight = 600 }),
		summary
	)
end

-- A line of counts in colours: { { 3, "passed", "success" }, ... }, zeros left out.
local function counts(list)
	local spans = {}
	for _, c in ipairs(list) do
		if c[1] > 0 then
			if #spans > 0 then
				table.insert(spans, ui.span({ "  ·  ", color = "faint" }))
			end
			table.insert(spans, ui.span({ ui.fmt(c[1]) .. " " .. c[2], color = c[3], weight = 600 }))
		end
	end
	if #spans == 0 then
		return false
	end
	return ui.text({ size = "sm", table.unpack(spans) })
end

local function mono(s, color)
	return ui.text({ s == "" and " " or s, mono = true, size = "sm", color = color })
end

-- Source lines, one mono text each, with an optional gutter of line numbers.
local function code_lines(lines, first, color)
	local width = first and #tostring(first + #lines - 1) or 0
	local out = {}
	for i, line in ipairs(lines) do
		if first then
			table.insert(
				out,
				ui.text({
					size = "sm",
					mono = true,
					ui.span({ string.format("%" .. width .. "d  ", first + i - 1), color = "faint", mono = true }),
					ui.span({ plain(line) == "" and " " or plain(line), color = color, mono = true }),
				})
			)
		else
			table.insert(out, mono(plain(line), color))
		end
	end
	return ui.column({ gap = 0, out })
end

local function pct_text(n)
	return string.format("%.1f%%", n)
end

local function cell(value, options)
	if value == nil then
		return nil
	end
	if type(value) == "table" then
		return value
	end
	local node = { tostring(value) }
	for k, v in pairs(options or {}) do
		node[k] = v
	end
	return ui.text(node)
end

-- A bar with a label above it, like Progress: `share` is 0 to 1.
local function bar(label, share, readout, token)
	return ui.canvas({
		height = 30,
		draw = function(g, w)
			g:text(0, 2, label, { size = "sm", color = "text" })
			g:text(w, 2, readout, { size = "sm", color = "muted", align = "end" })
			g:rect(0, 22, w, 4, { fill = ui.color("text", 0.12), radius = 2 })
			if share > 0 then
				g:rect(0, 22, math.max(4, w * ui.clamp(share, 0, 1)), 4, { fill = token, radius = 2 })
			end
		end,
	})
end

-- Opens and closes by key while the visual is open, like ui.kit.Details.
local expanded = {}
local function toggler(key, default)
	local open = expanded[key]
	if open == nil then
		open = default == true
	end
	return open, function()
		expanded[key] = not open
	end
end

-- Version "1.2.3" or "^1.2.3": its numbers.
local function version_parts(v)
	local a, b, c = tostring(v):match("(%d+)%.?(%d*)%.?(%d*)")
	return tonumber(a), tonumber(b) or 0, tonumber(c) or 0
end

local function signed(n)
	return n >= 0 and ("+" .. ui.fmt(n)) or ("−" .. ui.fmt(-n))
end

-- Status words ------------------------------------------------------------------

local function lookup(table_, aliases, value, what, allowed)
	local key = tostring(value == nil and "" or value):lower()
	key = aliases[key] or key
	local found = table_[key]
	if found == nil then
		fail("has an unknown " .. what .. " '" .. tostring(value) .. "': use " .. allowed)
	end
	return key, found
end

-- The same marks for a CI run, its jobs, a PR's checks.
local RUN = {
	passed = { "✓", "success", "Passed" },
	failed = { "✕", "danger", "Failed" },
	running = { "●", "accent", "Running" },
	queued = { "○", "faint", "Queued" },
	skipped = { "–", "faint", "Skipped" },
	cancelled = { "–", "warning", "Cancelled" },
}
local RUN_ALIAS = {
	success = "passed",
	pass = "passed",
	ok = "passed",
	failure = "failed",
	fail = "failed",
	error = "failed",
	in_progress = "running",
	pending = "queued",
	waiting = "queued",
	canceled = "cancelled",
	neutral = "skipped",
	skip = "skipped",
}
local RUN_ALLOWED = "passed, failed, running, queued, skipped, or cancelled"

local function run_status(item)
	local raw = item.conclusion or item.status
	if raw == "completed" then
		raw = item.conclusion
	end
	local key, mark = lookup(RUN, RUN_ALIAS, raw, "status", RUN_ALLOWED)
	return key, mark
end

-- Acceptance -----------------------------------------------------------------------

local ACC = {
	passed = { "✓", "success", "Passed" },
	failed = { "✕", "danger", "Failed" },
	blocked = { "!", "danger", "Blocked" },
	untested = { "○", "faint", "Untested" },
	claimed = { "?", "warning", "Claimed" },
}
local ACC_ALIAS = {
	verified = "passed",
	done = "passed",
	pass = "passed",
	ok = "passed",
	fail = "failed",
	unverified = "untested",
	["not tested"] = "untested",
	pending = "untested",
}
local ACC_ALLOWED = "passed, failed, blocked, untested, or claimed"

local function evidence_text(e)
	if type(e) == "table" then
		return e.label or e.url or e.path
	end
	return e
end

local function acceptance_meta(c)
	local meta = {}
	for _, key in ipairs({ "by", "at", "method", "run" }) do
		if c[key] ~= nil then
			table.insert(meta, tostring(c[key]))
		end
	end
	return #meta > 0 and table.concat(meta, " · ") or false
end

-- Is it really done: each criterion with what was checked, by whom, and the
-- evidence. With `columns`, a grid of claimed against checked.
define("Acceptance", { "criteria", "columns", "title" }, function(p)
	local criteria = need_list(p.criteria or p[1], "criteria = { { name, status, note?, evidence? }, ... }")
	local tally = { passed = 0, failed = 0, blocked = 0, untested = 0, claimed = 0 }
	local rows = {}
	local grid = p.columns ~= nil
	for _, c in ipairs(criteria) do
		if type(c) ~= "table" or c.name == nil then
			fail("needs every criterion as { name = \"...\", status = \"...\" }")
		end
		if grid then
			local cells = {}
			for _, column in ipairs(p.columns) do
				local value = (c.cells or {})[column]
				local status = type(value) == "table" and value.status or value
				local key, mark = lookup(ACC, ACC_ALIAS, status, "status", ACC_ALLOWED)
				tally[key] += 1
				table.insert(cells, {
					key = key,
					mark = mark,
					note = type(value) == "table" and value.note or nil,
					evidence = type(value) == "table" and value.evidence or nil,
					column = column,
				})
			end
			table.insert(rows, { criterion = c, cells = cells })
		else
			local key, mark = lookup(ACC, ACC_ALIAS, c.status, "status", ACC_ALLOWED)
			tally[key] += 1
			table.insert(rows, { criterion = c, key = key, mark = mark })
		end
	end
	local total = 0
	for _, n in pairs(tally) do
		total += n
	end
	local summary = build(
		ui.column,
		{ gap = 2 },
		ui.text({
			size = "sm",
			color = "muted",
			string.format("%s of %s %s", ui.fmt(tally.passed), ui.fmt(total), grid and "checks passed" or "criteria passed"),
		}),
		counts({
			{ tally.failed, "failed", "danger" },
			{ tally.blocked, "blocked", "danger" },
			{ tally.claimed, "claimed only", "warning" },
			{ tally.untested, "untested", "faint" },
		})
	)
	local body
	if grid then
		local table_rows, notes = {}, {}
		for _, row in ipairs(rows) do
			local line = { ui.text({ row.criterion.name }) }
			for _, c in ipairs(row.cells) do
				table.insert(line, ui.text({ ui.span({ c.mark[1] .. " " .. c.mark[3], color = c.mark[2] }), size = "sm" }))
				if c.note or c.evidence then
					table.insert(
						notes,
						ui.text({
							size = "sm",
							color = "muted",
							row.criterion.name .. " · " .. c.column .. ": " .. (c.note or "") .. (c.evidence and ("  [" .. evidence_text(c.evidence) .. "]") or ""),
						})
					)
				end
			end
			table.insert(table_rows, line)
		end
		local columns = { "Criterion" }
		for _, column in ipairs(p.columns) do
			table.insert(columns, column)
		end
		body = ui.column({ gap = 10, ui.table({ columns = columns, rows = table_rows }), notes })
	else
		body = ui.column({
			gap = 12,
			ui.map(rows, function(row)
				local c = row.criterion
				return ui.row({
					gap = 10,
					align = "start",
					ui.text({ row.mark[1], color = row.mark[2], weight = 700, width = 16 }),
					build(
						ui.column,
						{ gap = 2, grow = true },
						ui.row({
							gap = 8,
							ui.text({ c.name, weight = 600, grow = true }),
							ui.text({ row.mark[3], size = "sm", weight = 600, color = row.mark[2] }),
						}),
						acceptance_meta(c) and ui.text({ acceptance_meta(c), size = "sm", color = "muted" }) or false,
						c.note and ui.text({ c.note, size = "sm" }) or false,
						c.needs and ui.text({ "Needs: " .. c.needs, size = "sm", color = "warning" }) or false,
						c.evidence and mono(evidence_text(c.evidence), "faint") or false
					),
				})
			end),
		})
	end
	return ui.column({ gap = 14, heading(p.title, summary) or summary, body })
end)

-- Findings ---------------------------------------------------------------------------

local SEV = {
	critical = { "Critical", "danger", 1 },
	high = { "High", "danger", 2 },
	medium = { "Medium", "warning", 3 },
	low = { "Low", "accent", 4 },
	info = { "Info", "muted", 5 },
}
local SEV_ALIAS = { error = "high", major = "high", warning = "medium", warn = "medium", minor = "low", note = "info", hint = "info" }
local FSTATUS = {
	open = { "Open", "text" },
	fixed = { "Fixed", "success" },
	confirmed = { "Confirmed", "warning" },
	rejected = { "Rejected", "faint" },
}

local function is_patch(text)
	return text:match("^%-%-%- ") ~= nil or text:match("\n%-%-%- ") ~= nil or text:match("^@@") ~= nil
end

-- A line of a unified diff in a colour, no fill.
local function diff_color(line)
	local c = line:sub(1, 1)
	if line:sub(1, 2) == "@@" then
		return "accent"
	elseif c == "+" then
		return "success"
	elseif c == "-" then
		return "danger"
	end
	return nil
end

-- Review, lint or type errors: severity, where, why, a fix.
define("Findings", { "items", "title" }, function(p)
	local items = need_list(p.items or p[1], "items = { { severity, title, file?, line?, detail?, fix? }, ... }")
	local rows, tally = {}, {}
	for i, item in ipairs(items) do
		if type(item) ~= "table" or item.title == nil then
			fail("needs every finding as { severity = \"...\", title = \"...\" }")
		end
		local key, sev = lookup(SEV, SEV_ALIAS, item.severity or "info", "severity", "critical, high, medium, low, or info")
		tally[key] = (tally[key] or 0) + 1
		local status
		if item.status ~= nil then
			local _, found = lookup(FSTATUS, {}, item.status, "status", "open, fixed, confirmed, or rejected")
			status = found
		end
		table.insert(rows, { item = item, sev = sev, rank = sev[3], status = status, index = i })
	end
	table.sort(rows, function(a, b)
		if a.rank ~= b.rank then
			return a.rank < b.rank
		end
		return a.index < b.index
	end)
	local summary = counts({
		{ tally.critical or 0, "critical", "danger" },
		{ tally.high or 0, "high", "danger" },
		{ tally.medium or 0, "medium", "warning" },
		{ tally.low or 0, "low", "accent" },
		{ tally.info or 0, "info", "muted" },
	})
	return ui.column({
		gap = 14,
		heading(p.title, summary) or summary,
		ui.map(rows, function(row)
			local f = row.item
			local where
			if f.file then
				where = f.file
					.. (f.line and (":" .. f.line .. (f.column and (":" .. f.column) or "")) or "")
					.. (f.rule and ("  " .. f.rule) or "")
			elseif f.rule then
				where = f.rule
			end
			local fix
			if f.fix then
				local lines = split_lines(f.fix)
				local diff = is_patch(f.fix)
				local nodes = {}
				for _, line in ipairs(lines) do
					table.insert(nodes, mono(plain(line), diff and diff_color(line) or nil))
				end
				fix = ui.column({ gap = 2, ui.text({ "Fix", size = "xs", weight = 600, color = "muted" }), ui.column({ gap = 0, nodes }) })
			end
			return ui.row({
				gap = 10,
				align = "start",
				ui.text({ row.sev[1], size = "xs", weight = 700, color = row.sev[2], width = 56 }),
				build(
					ui.column,
					{ gap = 3, grow = true },
					ui.row({
						gap = 8,
						ui.text({ f.title, weight = 600, grow = true, color = row.status and row.status[1] ~= "Open" and "muted" or nil }),
						row.status and ui.text({ row.status[1], size = "sm", weight = 600, color = row.status[2] }) or false,
					}),
					where and mono(where, "muted") or false,
					f.detail and ui.text({ f.detail, size = "sm" }) or false,
					fix,
					f.url and mono(f.url, "faint") or false
				),
			})
		end),
	})
end)

-- StackTrace -----------------------------------------------------------------------------

local LIBRARY = { "node_modules", "node:internal", "rustlib", "/library/", "site%-packages", "java%.base", "/usr/lib", "<frozen", "/rustc/", "/pkg/mod/" }

local function is_library(line)
	for _, marker in ipairs(LIBRARY) do
		if line:find(marker) then
			return true
		end
	end
	return false
end

local function is_frame(line, next_line)
	return line:match("^%s+at ") ~= nil
		or line:match("^%s*%d+:%s") ~= nil
		or line:match("^%s+File \"") ~= nil
		or line:match("^\t") ~= nil
		or line:match("^%s+%.%.%. %d+ more") ~= nil
		or (next_line ~= nil and next_line:match("^\t") ~= nil)
end

-- An error and its stack in any language, read from the text a tool printed
-- (or { message, frames = { "at ...", ... } }). Frames in libraries are faint.
define("StackTrace", { "trace", "title" }, function(p)
	local trace = p.trace or p[1]
	local lines
	if type(trace) == "table" then
		lines = { tostring(trace.message or "Error") }
		for _, frame in ipairs(trace.frames or {}) do
			if type(frame) == "table" then
				frame = "    at " .. tostring(frame.fn or frame["function"] or "?") .. " (" .. tostring(frame.file or "?") .. (frame.line and (":" .. frame.line) or "") .. ")"
			end
			table.insert(lines, "    " .. frame:gsub("^%s+", ""))
		end
	elseif type(trace) == "string" then
		lines = split_lines(trace)
	else
		fail("needs trace = \"<the text of the error and its stack>\"")
	end
	if #lines == 0 then
		fail("needs a trace with at least one line")
	end
	local nodes, frames, own, seen_header = {}, 0, 0, false
	for i, raw in ipairs(lines) do
		if raw:match("%S") then
			local line = plain(raw)
			if is_frame(raw, lines[i + 1]) then
				-- A Rust frame is two lines ("0: fn", then "at file"): one frame,
				-- in a library if either line says so.
				local numbered = raw:match("^%s*%d+:%s") ~= nil
				local continuation = raw:match("^%s+at ") ~= nil and lines[i - 1] ~= nil and lines[i - 1]:match("^%s*%d+:%s") ~= nil
				local library = is_library(line) or (numbered and lines[i + 1] ~= nil and is_library(lines[i + 1])) or (continuation and is_library(lines[i - 1]))
				if not continuation then
					frames += 1
					if not (is_library(line) or (numbered and lines[i + 1] ~= nil and is_library(lines[i + 1]))) then
						own += 1
					end
				end
				table.insert(nodes, mono("  " .. line:gsub("^%s+", ""), library and "faint" or "text"))
			else
				local quiet = line:match("^Traceback") or line:match("^During handling") or line:match("^The above exception") or line:match("^stack backtrace")
				local loud = not quiet and (not seen_header or line:match("[Ee]rror") or line:match("[Ee]xception") or line:match("[Pp]anic") or line:match("^%s*[Cc]aused by"))
				if not quiet then
					seen_header = true
				end
				table.insert(nodes, ui.text({ line, mono = true, size = "sm", weight = loud and 600 or 400, color = loud and "danger" or "muted" }))
			end
		end
	end
	local summary = frames > 0 and ui.text({ string.format("%d frames, %d in your code", frames, own), size = "sm", color = "muted" }) or false
	return ui.column({ gap = 8, heading(p.title, summary) or summary, ui.column({ gap = 1, nodes }) })
end)

-- Dependencies -----------------------------------------------------------------------------

-- How far behind: major, minor, patch, or pre (same numbers, other text).
local function bump(current, latest)
	if current == nil then
		return "missing"
	end
	local a1, a2, a3 = version_parts(current)
	local b1, b2, b3 = version_parts(latest)
	if a1 == nil or b1 == nil then
		return tostring(current) == tostring(latest) and "current" or "pre"
	end
	if b1 ~= a1 then
		return b1 > a1 and "major" or "current"
	elseif b2 ~= a2 then
		return b2 > a2 and "minor" or "current"
	elseif b3 ~= a3 then
		return b3 > a3 and "patch" or "current"
	end
	return tostring(current) == tostring(latest) and "current" or "pre"
end

local BUMP = {
	major = { "major", "warning" },
	minor = { "minor", "accent" },
	patch = { "patch", "muted" },
	pre = { "pre-release", "muted" },
	missing = { "not installed", "danger" },
	current = { "up to date", "success" },
}

-- Outdated packages: name, current, latest and how far behind.
define("Dependencies", { "report", "title" }, function(p)
	local items = need_list(p.report or p[1], "report = { { name, current, latest }, ... }")
	local tally, rows, notes, wanted = {}, {}, {}, false
	for _, d in ipairs(items) do
		if type(d) ~= "table" or d.name == nil then
			fail("needs every package as { name = \"...\", current = \"...\", latest = \"...\" }")
		end
		wanted = wanted or d.wanted ~= nil
	end
	for _, d in ipairs(items) do
		local kind = bump(d.current, d.latest)
		tally[kind] = (tally[kind] or 0) + 1
		local meta = BUMP[kind]
		local risky = d.severity == "critical" or d.severity == "high"
		local row = {
			ui.text({ ui.span({ d.name, weight = 600 }), d.type and ui.span({ "  " .. d.type, color = "faint" }) or false, size = "sm" }),
			ui.text({ d.current and tostring(d.current) or "—", mono = true, size = "sm", color = "muted" }),
		}
		if wanted then
			table.insert(row, ui.text({ d.wanted and tostring(d.wanted) or "", mono = true, size = "sm", color = "muted" }))
		end
		table.insert(row, ui.text({ tostring(d.latest or "?"), mono = true, size = "sm" }))
		table.insert(row, ui.text({ risky and (meta[1] .. " · " .. d.severity) or meta[1], size = "sm", weight = 600, color = risky and "danger" or meta[2] }))
		table.insert(rows, row)
		if d.note then
			table.insert(notes, ui.text({ size = "sm", color = "muted", d.name .. ": " .. d.note }))
		end
	end
	local columns = { "Package", "Current" }
	if wanted then
		table.insert(columns, "Wanted")
	end
	table.insert(columns, "Latest")
	table.insert(columns, "Behind")
	local summary = counts({
		{ tally.major or 0, "major", "warning" },
		{ tally.minor or 0, "minor", "accent" },
		{ tally.patch or 0, "patch", "muted" },
		{ tally.pre or 0, "pre-release", "muted" },
		{ tally.missing or 0, "not installed", "danger" },
		{ tally.current or 0, "up to date", "success" },
	})
	return ui.column({ gap = 10, heading(p.title, summary) or summary, ui.table({ columns = columns, rows = rows }), notes })
end)

-- Benchmark ----------------------------------------------------------------------------------

local function num(value)
	if type(value) == "number" then
		return ui.fmt(value, { decimals = math.abs(value) >= 100 and 0 or 1 })
	end
	return tostring(value)
end

-- Before and after for each measure, or every run with its metrics. A change
-- inside 2 percent (or the standard deviation) counts as unchanged.
define("Benchmark", { "results", "unit", "title" }, function(p)
	local results = need_list(p.results or p[1], "results = { { name, before, after, unit?, better? }, ... }")
	if #results == 0 then
		fail("needs at least one result")
	end
	if results[1].metrics ~= nil then
		local keys, seen = {}, {}
		for _, run in ipairs(results) do
			for k in pairs(run.metrics or {}) do
				if not seen[k] then
					seen[k] = true
					table.insert(keys, k)
				end
			end
		end
		table.sort(keys)
		local columns, rows = { "Run" }, {}
		for _, k in ipairs(keys) do
			table.insert(columns, k)
		end
		table.insert(columns, "Status")
		for _, run in ipairs(results) do
			local row = { ui.text({ run.name, weight = 600, size = "sm" }) }
			for _, k in ipairs(keys) do
				local v = (run.metrics or {})[k]
				table.insert(row, v == nil and ui.text({ "—", color = "faint" }) or (type(v) == "number" and ui.text({ num(v), size = "sm", mono = true }) or ui.text({ tostring(v), size = "sm" })))
			end
			local key = run.status and select(1, lookup(RUN, RUN_ALIAS, run.status, "status", RUN_ALLOWED))
			table.insert(row, key and ui.text({ RUN[key][3], size = "sm", weight = 600, color = RUN[key][2] }) or ui.text({ "" }))
			table.insert(rows, row)
		end
		return ui.column({ gap = 10, heading(p.title, nil), ui.table({ columns = columns, rows = rows }) })
	end
	local rows, notes = {}, {}
	local better_n, worse_n, same_n = 0, 0, 0
	for _, r in ipairs(results) do
		if type(r.before) ~= "number" or type(r.after) ~= "number" then
			fail("needs before and after as numbers for '" .. tostring(r.name) .. "'")
		end
		local higher = r.better == "higher"
		if r.better ~= nil and r.better ~= "higher" and r.better ~= "lower" then
			fail("has an unknown better '" .. tostring(r.better) .. "': use lower or higher")
		end
		local delta = r.before ~= 0 and ((r.after - r.before) / math.abs(r.before) * 100) or 0
		local noise = math.max(2, r.stddev and r.before ~= 0 and (r.stddev / math.abs(r.before) * 100) or 0)
		local token, word = "muted", "unchanged"
		if math.abs(delta) >= noise then
			if (delta > 0) == higher then
				token, word = "success", "better"
				better_n += 1
			else
				token, word = "danger", "worse"
				worse_n += 1
			end
		else
			same_n += 1
		end
		local unit = r.unit or p.unit
		local suffix = unit and (" " .. unit) or ""
		table.insert(rows, {
			ui.text({ r.name, weight = 600, size = "sm" }),
			ui.text({ num(r.before) .. suffix, mono = true, size = "sm", color = "muted" }),
			ui.text({ num(r.after) .. suffix, mono = true, size = "sm" }),
			ui.text({ (delta >= 0 and "+" or "−") .. string.format("%.1f%%", math.abs(delta)) .. "  " .. word, size = "sm", weight = 600, color = token }),
		})
		if r.note or r.context then
			table.insert(notes, ui.text({ size = "sm", color = "muted", r.name .. ": " .. (r.note or "") .. (r.context and (" (" .. tostring(r.context) .. ")") or "") }))
		end
	end
	local summary = counts({
		{ better_n, "better", "success" },
		{ worse_n, "worse", "danger" },
		{ same_n, "unchanged", "muted" },
	})
	return ui.column({
		gap = 10,
		heading(p.title, summary) or summary,
		ui.table({ columns = { "Measure", "Before", "After", "Change" }, rows = rows }),
		notes,
	})
end)

-- Pipeline -------------------------------------------------------------------------------------

local function rollup(items)
	local any_running, any_failed, all_skipped = false, false, true
	for _, it in ipairs(items) do
		local key = run_status(it)
		any_failed = any_failed or key == "failed"
		any_running = any_running or key == "running" or key == "queued"
		all_skipped = all_skipped and key == "skipped"
	end
	return any_failed and "failed" or any_running and "running" or all_skipped and "skipped" or "passed"
end

local function pipeline_rows(items, depth, out)
	for _, item in ipairs(items) do
		if type(item) ~= "table" or item.name == nil then
			fail("needs every stage, job and step as { name = \"...\", status = \"...\" }")
		end
		local key, mark = run_status(item)
		table.insert(
			out,
			ui.row({
				gap = 8,
				align = "start",
				ui.text({ "", width = depth * 18 }),
				ui.text({ mark[1], color = mark[2], weight = 700, width = 14 }),
				build(
					ui.column,
					{ gap = 1, grow = true },
					ui.text({ item.name, weight = depth == 0 and 600 or 400, color = key == "skipped" and "faint" or nil }),
					item.message and ui.text({ item.message, size = "sm", color = "danger" }) or false
				),
				item.duration and ui.text({ tostring(item.duration), size = "sm", color = "muted" }) or false,
			})
		)
		local inner = item.jobs or item.steps
		-- Passed jobs stay one line; a failing or running one opens its steps.
		if inner and (depth == 0 or key == "failed" or key == "running") then
			pipeline_rows(inner, depth + 1, out)
		end
	end
	return out
end

-- A CI run: its stages, jobs and, where something failed, the steps.
define("Pipeline", { "run", "title" }, function(p)
	local run = need_list(p.run or p[1], "run = { name?, stages = { { name, status }, ... } }")
	local stages = run.stages or run.jobs
	if stages == nil and #run > 0 then
		stages, run = run, {}
	end
	if stages == nil then
		fail("needs stages = { { name, status, jobs? }, ... }")
	end
	local key = run.status ~= nil and run_status(run) or rollup(stages)
	local mark = RUN[key]
	local meta = {}
	for _, f in ipairs({ "branch", "sha", "duration" }) do
		if run[f] then
			table.insert(meta, tostring(run[f]))
		end
	end
	local head = build(
		ui.column,
		{ gap = 2 },
		ui.row({
			gap = 8,
			ui.text({ p.title or run.name or "Pipeline", size = "lg", weight = 600, grow = true }),
			ui.text({ mark[1] .. " " .. mark[3], weight = 600, color = mark[2] }),
		}),
		#meta > 0 and ui.text({ table.concat(meta, " · "), size = "sm", color = "muted" }) or false
	)
	return ui.column({ gap = 12, head, ui.column({ gap = 8, pipeline_rows(stages, 0, {}) }) })
end)

-- Tests ----------------------------------------------------------------------------------------

local TEST = {
	pass = { "✓", "success", "passed" },
	fail = { "✕", "danger", "failed" },
	skip = { "–", "warning", "skipped" },
	todo = { "○", "faint", "todo" },
}
local TEST_ALIAS = { passed = "pass", ok = "pass", success = "pass", failed = "fail", failure = "fail", error = "fail", skipped = "skip", pending = "skip" }

-- A test run: totals, then each failure with its message, then the rest.
define("Tests", { "results", "title" }, function(p)
	local results = need_list(p.results or p[1], "results = { { name, status, message?, file?, ms? }, ... }")
	local tally = { pass = 0, fail = 0, skip = 0, todo = 0 }
	local fails, others, passes = {}, {}, {}
	for _, t in ipairs(results) do
		if type(t) ~= "table" or t.name == nil then
			fail("needs every test as { name = \"...\", status = \"pass\" }")
		end
		local key = lookup(TEST, TEST_ALIAS, t.status, "status", "pass, fail, skip, or todo")
		tally[key] += 1
		table.insert(key == "fail" and fails or key == "pass" and passes or others, { t = t, key = key })
	end
	local total = #results
	local summary = ui.column({
		gap = 6,
		counts({
			{ tally.pass, "passed", "success" },
			{ tally.fail, "failed", "danger" },
			{ tally.skip, "skipped", "warning" },
			{ tally.todo, "todo", "faint" },
		}),
		ui.canvas({
			height = 6,
			draw = function(g, w)
				g:rect(0, 0, w, 6, { fill = ui.color("text", 0.12), radius = 3 })
				local x = 0
				for _, part in ipairs({ { tally.pass, "success" }, { tally.fail, "danger" }, { tally.skip, "warning" } }) do
					local seg = total > 0 and w * part[1] / total or 0
					if seg > 0 then
						g:rect(x, 0, seg, 6, { fill = part[2], radius = 3 })
					end
					x += seg
				end
			end,
		}),
	})
	local function where(t)
		return t.file and (t.file .. (t.line and (":" .. t.line) or "")) or false
	end
	local function row(item)
		local t, mark = item.t, TEST[item.key]
		local message = t.message and split_lines(t.message) or false
		local msg_nodes = {}
		for _, line in ipairs(message or {}) do
			table.insert(msg_nodes, mono(plain(line), item.key == "fail" and "muted" or "muted"))
		end
		return ui.row({
			gap = 10,
			align = "start",
			ui.text({ mark[1], color = mark[2], weight = 700, width = 14 }),
			build(
				ui.column,
				{ gap = 3, grow = true },
				ui.text({ t.name, weight = item.key == "fail" and 600 or 400 }),
				where(t) and mono(where(t), "faint") or false,
				#msg_nodes > 0 and ui.column({ gap = 0, msg_nodes }) or false
			),
			t.ms and ui.text({ ui.fmt(t.ms) .. " ms", size = "sm", color = "muted" }) or false,
		})
	end
	local nodes = {}
	for _, group in ipairs({ fails, others }) do
		for _, item in ipairs(group) do
			table.insert(nodes, row(item))
		end
	end
	local open, flip = toggler("tests:" .. tostring(p.title), #fails == 0 and #others == 0)
	if #passes > 0 then
		table.insert(
			nodes,
			ui.row({
				gap = 8,
				hover = true,
				on_click = flip,
				ui.text({ open and "▾" or "▸", color = "muted" }),
				ui.text({ ui.fmt(#passes) .. " passed", color = "muted" }),
			})
		)
		if open then
			for _, item in ipairs(passes) do
				table.insert(nodes, row(item))
			end
		end
	end
	return ui.column({ gap = 12, heading(p.title, nil), summary, ui.column({ gap = 10, nodes }) })
end)

-- Logs -------------------------------------------------------------------------------------------

local function level(line)
	local low = line:lower()
	if low:match("%f[%a]error%f[%A]") or low:match("%f[%a]fatal%f[%A]") or low:match("%f[%a]panic") or low:match("%f[%a]exception") or low:match("%f[%a]fail") then
		return "danger"
	elseif low:match("%f[%a]warn") then
		return "warning"
	elseif low:match("%f[%a]debug%f[%A]") or low:match("%f[%a]trace%f[%A]") then
		return "faint"
	end
	return nil
end

local LOG_LINES = 300

-- A log or command output: levels coloured, repeated lines folded.
define("Logs", { "output", "command", "exit", "title" }, function(p)
	local output = p.output or p[1]
	local lines
	if type(output) == "table" then
		lines = {}
		for _, line in ipairs(output) do
			for _, l in ipairs(split_lines(line)) do
				table.insert(lines, l)
			end
		end
	elseif type(output) == "string" then
		lines = split_lines(output)
	else
		fail("needs output = \"<the text>\" or a list of lines")
	end
	local folded = {}
	for _, line in ipairs(lines) do
		local last = folded[#folded]
		if last and last.text == line and line:match("%S") and not line:match("^%s") then
			last.n += 1
		else
			table.insert(folded, { text = line, n = 1 })
		end
	end
	local nodes, previous = {}, nil
	for i, entry in ipairs(folded) do
		if i > LOG_LINES then
			table.insert(nodes, ui.text({ "… " .. ui.fmt(#folded - LOG_LINES) .. " more lines", size = "sm", color = "faint" }))
			break
		end
		local line = plain(entry.text)
		local token = level(line)
		if line:match("^%s") then
			token = previous == "danger" and "muted" or "muted"
		else
			previous = token
		end
		table.insert(
			nodes,
			ui.text({
				mono = true,
				size = "sm",
				ui.span({ line == "" and " " or line, color = token, mono = true }),
				entry.n > 1 and ui.span({ "  ×" .. entry.n, color = "faint", mono = true }) or false,
			})
		)
	end
	local head = false
	if p.command or p.exit ~= nil or p.title then
		local ok = p.exit == 0
		head = build(
			ui.row,
			{ gap = 10 },
			p.title and ui.text({ p.title, size = "lg", weight = 600 }) or false,
			p.command and ui.text({ "$ " .. p.command, mono = true, size = "sm", grow = true }) or false,
			p.exit ~= nil and ui.text({ "exit " .. tostring(p.exit), size = "sm", weight = 600, color = ok and "success" or "danger" }) or false
		)
	end
	return ui.column({ gap = 8, head, ui.column({ gap = 0, nodes }) })
end)

-- Coverage ----------------------------------------------------------------------------------------

local function coverage_token(pct)
	return pct >= 80 and "success" or pct >= 60 and "warning" or "danger"
end

local FILES_SHOWN = 15

-- A coverage report: the total, then the least covered files first.
define("Coverage", { "report", "title" }, function(p)
	local report = need_list(p.report or p[1], "report = { total?, files = { { path, pct }, ... } }")
	local files = report.files or (#report > 0 and report) or nil
	if files == nil then
		fail("needs files = { { path = \"...\", pct = 87.5 }, ... } (or covered and total)")
	end
	local rows, covered_sum, total_sum, pct_sum = {}, 0, 0, 0
	for _, f in ipairs(files) do
		if type(f) ~= "table" or f.path == nil then
			fail("needs every file as { path = \"...\", pct = <0-100> }")
		end
		local pct = f.pct
		if pct == nil and f.covered and f.total then
			pct = f.total > 0 and (f.covered / f.total * 100) or 100
			covered_sum += f.covered
			total_sum += f.total
		end
		if type(pct) ~= "number" then
			fail("needs pct (or covered and total) for '" .. f.path .. "'")
		end
		pct_sum += pct
		table.insert(rows, { path = f.path, pct = pct })
	end
	table.sort(rows, function(a, b)
		return a.pct < b.pct
	end)
	local total = report.total
	if type(total) == "table" then
		total = total.pct or total.lines
	end
	if total == nil then
		total = total_sum > 0 and (covered_sum / total_sum * 100) or (#rows > 0 and pct_sum / #rows or 0)
	end
	local nodes = {}
	for i, r in ipairs(rows) do
		if i > FILES_SHOWN then
			table.insert(nodes, ui.text({ "+ " .. (#rows - FILES_SHOWN) .. " better covered files", size = "sm", color = "faint" }))
			break
		end
		table.insert(nodes, bar(r.path, r.pct / 100, pct_text(r.pct), coverage_token(r.pct)))
	end
	return ui.column({
		gap = 12,
		heading(p.title, nil),
		ui.stat({ value = pct_text(total), label = "covered", tone = coverage_token(total) }),
		ui.column({ gap = 6, nodes }),
	})
end)

-- Diff, Changes, Commits ----------------------------------------------------------------------------------

-- The body of a unified patch: a gutter of old and new line numbers, the line
-- coloured by what it does. Several files are fine.
local DIFF_LINES = 400

local function diff_body(patch)
	local lines = split_lines(patch)
	local nodes, old, new, rem_old, rem_new = {}, 0, 0, 0, 0
	for i, line in ipairs(lines) do
		if i > DIFF_LINES then
			table.insert(nodes, ui.text({ "… " .. (#lines - DIFF_LINES) .. " more lines", size = "sm", color = "faint" }))
			break
		end
		local text = plain(line)
		local c = text:sub(1, 1)
		local gutter, token, weight = "          ", nil, 400
		if rem_old > 0 or rem_new > 0 then
			if c == "+" then
				new += 1
				rem_new -= 1
				gutter = string.format("     %4d ", new)
				token = "success"
			elseif c == "-" then
				old += 1
				rem_old -= 1
				gutter = string.format("%4d      ", old)
				token = "danger"
			elseif c == "\\" then
				token = "faint"
			else
				old += 1
				new += 1
				rem_old -= 1
				rem_new -= 1
				gutter = string.format("%4d %4d ", old, new)
			end
		elseif text:sub(1, 2) == "@@" then
			local a, b, c2, d = text:match("^@@ %-(%d+),?(%d*) %+(%d+),?(%d*) @@")
			if a then
				old, new = tonumber(a) - 1, tonumber(c2) - 1
				rem_old = b == "" and 1 or tonumber(b)
				rem_new = d == "" and 1 or tonumber(d)
			end
			token = "accent"
		else
			token, weight = text:match("^diff ") and "text" or "muted", text:match("^diff ") and 600 or 400
		end
		table.insert(
			nodes,
			ui.text({
				mono = true,
				size = "sm",
				ui.span({ gutter, color = "faint", mono = true }),
				ui.span({ text == "" and " " or text, color = token, weight = weight, mono = true }),
			})
		)
	end
	return ui.column({ gap = 0, nodes })
end

-- A unified patch, as the changes it makes.
define("Diff", { "patch", "title" }, function(p)
	local patch = p.patch or p[1]
	if type(patch) ~= "string" or not patch:match("@@") then
		fail("needs patch = \"<a unified diff, with @@ hunks>\"")
	end
	return ui.column({ gap = 8, heading(p.title, nil), diff_body(patch) })
end)

local STATUS_FILE = {
	added = { "A", "success" },
	modified = { "M", "warning" },
	deleted = { "D", "danger" },
	renamed = { "R", "accent" },
}
local FILE_ALIAS = { a = "added", m = "modified", d = "deleted", r = "renamed", new = "added", changed = "modified", removed = "deleted" }

local function path_span(path)
	local dir, name = path:match("^(.*/)([^/]+)$")
	if dir == nil then
		return ui.text({ path, mono = true, size = "sm" })
	end
	return ui.text({ mono = true, size = "sm", ui.span({ dir, color = "muted", mono = true }), ui.span({ name, mono = true, weight = 600 }) })
end

local function delta_text(added, removed)
	if added == nil and removed == nil then
		return false
	end
	return ui.text({
		size = "sm",
		mono = true,
		ui.span({ "+" .. ui.fmt(added or 0), color = "success", mono = true }),
		ui.span({ " " }),
		ui.span({ "−" .. ui.fmt(removed or 0), color = "danger", mono = true }),
	})
end

-- Files changed, with their line counts; a file with a `diff` opens to it.
define("Changes", { "files", "title" }, function(p)
	local files = need_list(p.files or p[1], "files = { { path, added, removed, status?, diff? }, ... }")
	local added, removed, nodes = 0, 0, {}
	for _, f in ipairs(files) do
		if type(f) ~= "table" or f.path == nil then
			fail("needs every file as { path = \"...\", added = 3, removed = 1 }")
		end
		local _, mark = lookup(STATUS_FILE, FILE_ALIAS, f.status or "modified", "status", "added, modified, deleted, or renamed")
		added += f.added or 0
		removed += f.removed or 0
		local open, flip
		if f.diff then
			open, flip = toggler("change:" .. f.path, false)
		end
		table.insert(
			nodes,
			build(
				ui.column,
				{ gap = 6 },
				ui.row({
					gap = 10,
					hover = f.diff ~= nil,
					on_click = flip,
					ui.text({ mark[1], color = mark[2], weight = 700, width = 14 }),
					ui.row({ grow = true, path_span(f.path) }),
					delta_text(f.added, f.removed),
					f.diff and ui.text({ open and "▾" or "▸", color = "muted" }) or false,
				}),
				f.diff and open and diff_body(f.diff) or false
			)
		)
	end
	local summary = ui.text({
		size = "sm",
		ui.span({ ui.fmt(#files) .. (#files == 1 and " file" or " files") .. " changed  ", color = "muted" }),
		ui.span({ "+" .. ui.fmt(added), color = "success", weight = 600 }),
		ui.span({ " " }),
		ui.span({ "−" .. ui.fmt(removed), color = "danger", weight = 600 }),
	})
	return ui.column({ gap = 10, heading(p.title, summary) or summary, ui.column({ gap = 8, nodes }) })
end)

-- A branch's commits: short sha, title, who and when, lines changed.
define("Commits", { "commits", "title" }, function(p)
	local commits = need_list(p.commits or p[1], "commits = { { sha, title, author?, date? }, ... }")
	return ui.column({
		gap = 12,
		heading(p.title, ui.text({ ui.fmt(#commits) .. (#commits == 1 and " commit" or " commits"), size = "sm", color = "muted" })),
		ui.map(commits, function(c)
			if type(c) ~= "table" or c.title == nil then
				fail("needs every commit as { sha = \"...\", title = \"...\" }")
			end
			local meta = {}
			for _, k in ipairs({ "author", "date" }) do
				if c[k] then
					table.insert(meta, tostring(c[k]))
				end
			end
			if c.files then
				table.insert(meta, ui.fmt(c.files) .. (c.files == 1 and " file" or " files"))
			end
			return ui.row({
				gap = 10,
				align = "start",
				ui.text({ c.sha and tostring(c.sha):sub(1, 7) or "", mono = true, size = "sm", color = "accent", width = 60 }),
				build(
					ui.column,
					{ gap = 2, grow = true },
					ui.text({ c.title, weight = 500 }),
					#meta > 0 and ui.text({ table.concat(meta, " · "), size = "sm", color = "muted" }) or false,
					c.body and ui.text({ c.body, size = "sm", color = "muted" }) or false
				),
				delta_text(c.added, c.removed),
			})
		end),
	})
end)

-- PullRequest, Environments --------------------------------------------------------------------------------

local PR_STATE = {
	open = { "Open", "success" },
	draft = { "Draft", "muted" },
	merged = { "Merged", "accent" },
	closed = { "Closed", "danger" },
}
local REVIEW = {
	approved = { "Approved", "success" },
	["changes-requested"] = { "Changes requested", "danger" },
	pending = { "Review pending", "muted" },
	none = { "No review yet", "faint" },
}
local REVIEW_ALIAS = { changes_requested = "changes-requested", requested = "changes-requested", review_required = "pending" }

-- A pull request: state, branch, checks, review.
define("PullRequest", { "title", "number", "state", "author", "branch", "base", "url", "added", "removed", "checks", "review", "body" }, function(p)
	if p.title == nil then
		fail("needs title = \"...\"")
	end
	local _, state = lookup(PR_STATE, {}, p.state or "open", "state", "open, draft, merged, or closed")
	local meta = {}
	if p.author then
		table.insert(meta, tostring(p.author))
	end
	if p.branch then
		table.insert(meta, tostring(p.branch) .. (p.base and (" → " .. tostring(p.base)) or ""))
	end
	local checks_node
	if p.checks then
		local tally, rows = { passed = 0, failed = 0, running = 0, queued = 0, skipped = 0, cancelled = 0 }, {}
		for _, check in ipairs(p.checks) do
			local key, mark = run_status(check)
			tally[key] += 1
			table.insert(
				rows,
				ui.row({
					gap = 8,
					ui.text({ mark[1], color = mark[2], weight = 700, width = 14 }),
					ui.text({ check.name, grow = true, size = "sm" }),
					check.duration and ui.text({ tostring(check.duration), size = "sm", color = "muted" }) or false,
				})
			)
		end
		local pending = tally.running + tally.queued
		checks_node = ui.column({
			gap = 6,
			ui.text({
				size = "sm",
				weight = 600,
				color = tally.failed > 0 and "danger" or pending > 0 and "accent" or "success",
				tally.failed > 0 and (tally.failed .. " of " .. #p.checks .. " checks failing")
					or pending > 0 and (pending .. " checks pending")
					or "All checks passed",
			}),
			ui.column({ gap = 4, rows }),
		})
	end
	local review_node
	if p.review then
		local r = type(p.review) == "table" and p.review or { state = p.review }
		local _, review = lookup(REVIEW, REVIEW_ALIAS, r.state or "none", "review state", "approved, changes-requested, pending, or none")
		local who = r.reviewers and #r.reviewers > 0 and (" · " .. table.concat(r.reviewers, ", ")) or ""
		review_node = ui.text({ review[1] .. who, size = "sm", weight = 600, color = review[2] })
	end
	return build(
		ui.column,
		{ gap = 10 },
		ui.row({
			gap = 8,
			align = "start",
			ui.text({ (p.number and ("#" .. p.number .. "  ") or "") .. p.title, size = "lg", weight = 600, grow = true }),
			ui.text({ state[1], weight = 600, color = state[2] }),
		}),
		#meta > 0 and ui.text({ table.concat(meta, " · "), size = "sm", color = "muted" }) or false,
		(p.added or p.removed) and delta_text(p.added, p.removed) or false,
		p.body and ui.text({ p.body, size = "sm" }) or false,
		checks_node,
		review_node,
		p.url and mono(p.url, "faint") or false
	)
end)

-- A project's worktrees: branch, threads, changes, pull request.
define("Environments", { "items", "title" }, function(p)
	local items = need_list(p.items or p[1], "items = { { name, branch, threads?, changes?, pr? }, ... }")
	return ui.column({
		gap = 12,
		heading(p.title, nil),
		ui.map(items, function(e)
			if type(e) ~= "table" or (e.name == nil and e.branch == nil) then
				fail("needs every environment as { name = \"...\", branch = \"...\" }")
			end
			local meta = {}
			if e.threads then
				table.insert(meta, ui.fmt(e.threads) .. (e.threads == 1 and " thread" or " threads"))
			end
			local changes = e.changes
			if type(changes) == "number" then
				changes = { files = changes }
			end
			if changes and changes.files then
				table.insert(meta, ui.fmt(changes.files) .. (changes.files == 1 and " file" or " files"))
			end
			local pr = e.pr
			local pr_state
			if pr then
				local _, found = lookup(PR_STATE, {}, pr.state or "open", "state", "open, draft, merged, or closed")
				pr_state = found
			end
			return ui.row({
				gap = 10,
				align = "start",
				ui.text({ e.current and "●" or "○", color = e.current and "accent" or "faint", width = 14 }),
				build(
					ui.column,
					{ gap = 2, grow = true },
					ui.row({
						gap = 8,
						ui.text({ e.name or e.branch, weight = 600 }),
						e.current and ui.text({ "this chat", size = "xs", weight = 600, color = "accent" }) or false,
					}),
					e.branch and e.name and mono(e.branch, "muted") or false,
					#meta > 0 and ui.text({ table.concat(meta, " · "), size = "sm", color = "muted" }) or false,
					e.note and ui.text({ e.note, size = "sm", color = "muted" }) or false
				),
				changes and delta_text(changes.added, changes.removed) or false,
				pr and ui.text({ "#" .. tostring(pr.number or "") .. " " .. pr_state[1], size = "sm", weight = 600, color = pr_state[2] }) or false,
			})
		end),
	})
end)

-- Snippet, Code, File, Json -----------------------------------------------------------------------------------------

local function location(path, line)
	return line and (path .. ":" .. line) or path
end

-- Lines of a file with their numbers. The sandbox can't read the file, so
-- `code` is the excerpt and `from` the number of its first line.
define("Snippet", { "path", "code", "from", "language" }, function(p)
	if p.path == nil then
		fail("needs path = \"...\"")
	end
	if type(p.code) ~= "string" then
		fail("needs code = \"<the lines>\": a visual cannot read files, so pass the excerpt itself")
	end
	local from = p.from or 1
	local lines = split_lines(p.code)
	local to = from + #lines - 1
	return ui.column({
		gap = 6,
		ui.text({
			size = "sm",
			mono = true,
			ui.span({ p.path, mono = true, weight = 600 }),
			ui.span({ "  lines " .. from .. (to > from and ("–" .. to) or ""), color = "muted", mono = true }),
		}),
		code_lines(lines, from),
	})
end)

-- A block of code, as written (no highlighting), with its language named.
define("Code", { "code", "language", "title" }, function(p)
	local code = p.code or p[1]
	if type(code) ~= "string" then
		fail("needs code = \"...\"")
	end
	return build(
		ui.column,
		{ gap = 4 },
		(p.title or p.language) and ui.row({
			gap = 8,
			p.title and ui.text({ p.title, weight = 600, grow = true }) or false,
			p.language and ui.text({ p.language, size = "xs", color = "faint" }) or false,
		}) or false,
		code_lines(split_lines(code), nil)
	)
end)

-- A file to look at: its folder faint, its name bold, an optional line.
define("File", { "path", "line", "note" }, function(p)
	local path = p.path or p[1]
	if type(path) ~= "string" or path == "" then
		fail("needs path = \"src/main.rs\"")
	end
	local dir, name = path:match("^(.*/)([^/]+)$")
	return build(
		ui.row,
		{ gap = 10, align = "center" },
		ui.text({
			mono = true,
			size = "sm",
			dir and ui.span({ dir, color = "muted", mono = true }) or false,
			ui.span({ name or path, mono = true, weight = 600 }),
			p.line and ui.span({ ":" .. p.line, color = "accent", mono = true }) or false,
		}),
		p.note and ui.text({ p.note, size = "sm", color = "muted" }) or false
	)
end)

local JSON_SHOWN = 60

local function json_scalar(v)
	local t = type(v)
	if t == "string" then
		return string.format("%q", v):gsub("\\\n", "\\n"), "success"
	elseif t == "number" then
		return ui.fmt(v, { decimals = v == math.floor(v) and 0 or 6 }):gsub(",", ""), "accent"
	elseif t == "boolean" then
		return tostring(v), "warning"
	end
	return tostring(v), "faint"
end

-- A value as a tree you open and close: tables become objects (keys sorted)
-- or arrays (a sequence). Luau has no null: leave a key out or use a string.
define("Json", { "value", "label", "open", "key" }, function(p)
	local root = p.value
	if root == nil then
		root = p[1]
	end
	if root == nil then
		fail("needs value = <a table or a value>")
	end
	local id = p.key or p.label or "json"
	local depth_open = p.open or 1
	local nodes = {}
	local budget = 400
	local function walk(key, value, path, depth)
		if budget <= 0 then
			return
		end
		budget -= 1
		local indent = ui.text({ "", width = depth * 16 })
		if type(value) ~= "table" then
			local shown, token = json_scalar(value)
			table.insert(
				nodes,
				ui.row({
					gap = 6,
					align = "start",
					indent,
					key ~= nil and ui.text({ tostring(key), mono = true, size = "sm", color = "muted" }) or false,
					ui.text({ shown, mono = true, size = "sm", color = token }),
				})
			)
			return
		end
		local array = #value > 0 or next(value) == nil
		local keys = {}
		if array then
			for i = 1, #value do
				table.insert(keys, i)
			end
		else
			for k in pairs(value) do
				table.insert(keys, k)
			end
			table.sort(keys, function(a, b)
				return tostring(a) < tostring(b)
			end)
		end
		local state_key = "json:" .. id .. ":" .. path
		local open, flip = toggler(state_key, depth < depth_open)
		table.insert(
			nodes,
			ui.row({
				gap = 6,
				hover = #keys > 0,
				on_click = #keys > 0 and flip or nil,
				indent,
				ui.text({ #keys > 0 and (open and "▾" or "▸") or " ", color = "muted", width = 10 }),
				key ~= nil and ui.text({ tostring(key), mono = true, size = "sm", color = "muted" }) or false,
				ui.text({ (array and "[" or "{") .. #keys .. (array and "]" or "}"), mono = true, size = "sm", color = "faint" }),
			})
		)
		if open then
			for i, k in ipairs(keys) do
				if i > JSON_SHOWN then
					table.insert(nodes, ui.row({ indent, ui.text({ "… " .. (#keys - JSON_SHOWN) .. " more", size = "sm", color = "faint" }) }))
					break
				end
				walk(array and nil or k, value[k], path .. "/" .. tostring(k), depth + 1)
			end
		end
	end
	walk(nil, root, "", 0)
	return build(
		ui.column,
		{ gap = 2 },
		p.label and ui.text({ p.label, weight = 600 }) or false,
		ui.column({ gap = 1, nodes })
	)
end)

return kit
