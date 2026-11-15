--!nocheck
-- `ui.kit`, batch e (plans/live-visuals G7e): numbers, lists and tables the
-- reader edits, a quiz, a timer. Loaded after kit.luau with the same `ui`;
-- returns its kinds, which Zend joins into the one frozen `ui.kit`. The
-- rule for a kind is in visuals/README.md ("The kit"). The three helpers
-- below are kit.luau's, repeated so the batches stay apart.
local ui = ...
local kit = {}

local function fail(message)
	error(message, 0)
end

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

local function build(constructor, props, ...)
	for i = 1, select("#", ...) do
		local child = select(i, ...)
		if child ~= nil then
			table.insert(props, child)
		end
	end
	return constructor(props)
end

local function need_name(props)
	if type(props.name) ~= "string" or props.name == "" then
		fail(
			"needs name = \"...\": a fixed name, unique in the visual, that Zend keeps the "
				.. "reader's entry under (keep it the same when you update the visual)"
		)
	end
end

-- A named input from `props`, the listed keys passed through.
local function input(constructor, props, keys, extra)
	need_name(props)
	local node = {}
	for _, key in ipairs(keys) do
		node[key] = props[key]
	end
	for k, v in pairs(extra or {}) do
		node[k] = v
	end
	return constructor(node)
end

local INPUT = { "name", "label", "description", "value", "answered" }

local function with(list, ...)
	local out = table.clone(list)
	for _, key in ipairs({ ... }) do
		table.insert(out, key)
	end
	return out
end

-- Numbers ------------------------------------------------------------------

-- The same checks ui.number makes, here so they name the kind and the line.
local function number(p, key)
	local v = p[key]
	if v ~= nil and (type(v) ~= "number" or v ~= v or v == math.huge or v == -math.huge) then
		fail("needs " .. key .. " = <number>, not " .. tostring(v))
	end
	return v
end

local function numbers(p, min, max)
	min, max = number(p, "min") or min, number(p, "max") or max
	if min ~= nil and max ~= nil and max <= min then
		fail("needs max above min")
	end
	local step = number(p, "step")
	if step ~= nil and step <= 0 then
		fail("needs a step above 0")
	end
	number(p, "value")
end

-- A number dragged between min and max (0 to 100 unless given).
local SLIDER = with(INPUT, "min", "max", "step")
define("Slider", SLIDER, function(p)
	numbers(p, 0, 100)
	return input(ui.number, p, SLIDER, { style = "slider" })
end)

-- A count with − and +; min and max are optional ends.
define("Stepper", SLIDER, function(p)
	numbers(p)
	return input(ui.number, p, SLIDER, { style = "stepper" })
end)

-- Stars from 1 to max (5 unless given); unrated until the reader picks.
local RATING = with(INPUT, "max")
define("Rating", RATING, function(p)
	local max = number(p, "max")
	if max ~= nil and (max % 1 ~= 0 or max < 2 or max > 10) then
		fail("needs a whole max from 2 to 10")
	end
	number(p, "value")
	return input(ui.number, p, RATING, { style = "rating" })
end)

-- Lists and tables -------------------------------------------------------------

-- Short texts the reader adds, edits and removes; value is where it starts.
local LIST = with(INPUT, "placeholder")
define("ListEditor", LIST, function(p)
	if p.value ~= nil and type(p.value) ~= "table" then
		fail("needs value = { \"...\", ... }: a list of texts")
	end
	return input(ui.items, p, LIST)
end)

-- Rows under fixed columns: names, or { key, label?, type? = text | number |
-- boolean }. value: rows as tables keyed by column key (or in column order).
local TABLE = with(INPUT, "columns")
define("EditableTable", TABLE, function(p)
	if type(p.columns) ~= "table" or #p.columns == 0 then
		fail("needs columns = { \"...\", ... } or { { key, label?, type? }, ... }")
	end
	local seen = {}
	for i, column in ipairs(p.columns) do
		local key = column
		if type(column) == "table" then
			key = column.key
			local kind = column.type
			if kind ~= nil and kind ~= "text" and kind ~= "number" and kind ~= "boolean" then
				fail("has a column of type '" .. tostring(kind) .. "': use text, number, or boolean")
			end
		end
		if key == nil or key == "" then
			fail("has column " .. i .. " without key")
		end
		if seen[key] then
			fail("has column '" .. tostring(key) .. "' twice")
		end
		seen[key] = true
	end
	if #p.columns > 12 then
		fail("has more than 12 columns")
	end
	if p.value ~= nil then
		if type(p.value) ~= "table" then
			fail("needs value = { { ... }, ... }: rows, each a table")
		end
		for i, row in ipairs(p.value) do
			if type(row) ~= "table" then
				fail("row " .. i .. " is a " .. type(row) .. ": each row is a table")
			end
		end
	end
	return input(ui.rows, p, TABLE)
end)

-- Widgets ----------------------------------------------------------------------

-- Questions, each { q, options, answer, why? }: answer is the right option's
-- number (1 is the first) or its text. Each question is a choice named
-- "<name>.<n>", so what the reader picks is kept and goes into the answer;
-- once picked, the question says whether it was right and why.
define("Quiz", { "name", "questions", "label" }, function(p)
	need_name(p)
	local questions = p.questions
	if type(questions) ~= "table" or #questions == 0 then
		fail("needs questions = { { q = \"...\", options = { ... }, answer = 1, why? }, ... }")
	end
	local right, picked = 0, 0
	local rows = {}
	for i, question in ipairs(questions) do
		if type(question) ~= "table" or question.q == nil then
			fail("question " .. i .. " needs q = \"...\"")
		end
		if type(question.options) ~= "table" or #question.options < 2 then
			fail("question " .. i .. " needs options = { \"...\", \"...\" } (two or more)")
		end
		local options, answer = {}, nil
		for j, option in ipairs(question.options) do
			local words = tostring(option)
			table.insert(options, { value = tostring(j), label = words })
			if question.answer == j or question.answer == words then
				answer = j
			end
		end
		if answer == nil then
			fail(
				"question "
					.. i
					.. " needs answer = <the right option's number, 1 to "
					.. #options
					.. ", or its text>"
			)
		end
		local id = p.name .. "." .. i
		local pick = tonumber(ui.entry(id))
		local feedback = nil
		if pick ~= nil then
			picked += 1
			local ok = pick == answer
			if ok then
				right += 1
			end
			feedback = build(
				ui.column,
				{ gap = 2 },
				ui.text({
					ui.span({ ok and "✓ Right" or "✕ Not quite", color = ok and "success" or "danger", weight = 600 }),
					not ok and ui.span({ ": " .. options[answer].label, color = "muted" }) or nil,
				}),
				question.why and ui.text({ tostring(question.why), size = "sm", color = "muted" }) or nil
			)
		end
		table.insert(
			rows,
			build(
				ui.column,
				{ gap = 6 },
				ui.choice({ name = id, label = i .. ". " .. tostring(question.q), options = options }),
				feedback
			)
		)
	end
	local count = #questions
	local score = picked == count
			and ui.text({
				right .. " of " .. count .. " right",
				weight = 600,
				color = right == count and "success" or nil,
			})
		or ui.text({ picked .. " of " .. count .. " answered", size = "sm", color = "muted" })
	return build(
		ui.column,
		{ gap = 16 },
		p.label and ui.text({ p.label, size = "lg", weight = 600 }) or nil,
		rows,
		score
	)
end)

-- A countdown of `seconds`, or a stopwatch without. Start, pause, reset; it
-- runs while the visual is open and keeps nothing (it's not an answer).
local clocks = {}
local function clock_text(seconds, hours)
	seconds = math.max(0, math.floor(seconds + 0.5))
	local h, m, s = seconds // 3600, (seconds // 60) % 60, seconds % 60
	if hours or h > 0 then
		return string.format("%d:%02d:%02d", h, m, s)
	end
	return string.format("%02d:%02d", m, s)
end
define("Timer", { "seconds", "label", "key" }, function(p)
	local total = nil
	if p.seconds ~= nil then
		total = tonumber(p.seconds)
		if total == nil or total <= 0 then
			fail("needs seconds above 0 (or none, for a stopwatch)")
		end
	end
	local id = p.key or p.label or (total and ("countdown " .. total) or "stopwatch")
	local c = clocks[id]
	if c == nil then
		-- banked: seconds run before the last start; from: the draw clock
		-- at that start (nil: stopped); want: a press waiting for a draw.
		c = { banked = 0, from = nil, last = nil, want = nil }
		clocks[id] = c
	end
	local hours = total ~= nil and total >= 3600
	local function elapsed(t)
		return c.banked + (c.from and (t - c.from) or 0)
	end
	local running = c.from ~= nil or c.want == "start"
	if c.want == "pause" then
		running = false
	end
	return build(
		ui.column,
		{ gap = 8 },
		p.label and ui.text({ p.label, size = "sm", color = "muted" }) or nil,
		ui.canvas({
			height = total and 64 or 52,
			animate = true,
			draw = function(g, w, h, t)
				-- A canvas drawn anew starts its clock again: carry on from here.
				if c.last ~= nil and t < c.last and c.from ~= nil then
					c.from = t - (c.last - c.from)
				end
				c.last = t
				if c.want == "start" then
					c.from = t
				elseif c.want == "pause" and c.from ~= nil then
					c.banked = elapsed(t)
					c.from = nil
				end
				c.want = nil
				local run = elapsed(t)
				local done = total ~= nil and run >= total
				if done and c.from ~= nil then
					c.banked, c.from = total, nil
					run = total
				end
				local shown = total and clock_text(math.ceil(total - run - 1e-6), hours) or clock_text(math.floor(run))
				g:text(0, 0, done and ("✓ " .. shown) or shown, {
					-- (Mono: the digits keep their places as they tick.)
					size = "xl",
					mono = true,
					color = done and "success" or "text",
				})
				if total then
					g:rect(0, h - 6, w, 4, { fill = ui.color("text", 0.12), radius = 2 })
					local share = math.min(1, run / total)
					if share > 0 then
						g:rect(0, h - 6, math.max(4, w * share), 4, { fill = done and "success" or "accent", radius = 2 })
					end
				end
			end,
		}),
		ui.row({
			gap = 8,
			ui.button({
				label = running and "Pause" or "Start",
				primary = not running,
				on_click = function()
					if running then
						c.want = "pause"
					else
						if total ~= nil and c.banked >= total then
							c.banked = 0
						end
						c.want = "start"
					end
				end,
			}),
			ui.button({
				label = "Reset",
				on_click = function()
					c.banked, c.from, c.want = 0, nil, nil
				end,
			}),
		})
	)
end)

return kit
