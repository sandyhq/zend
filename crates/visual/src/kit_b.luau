
--!nocheck
-- Batch b of `ui.kit`: planning and data. Loaded right after kit.luau,
-- before the kit is frozen; it needs nothing from kit.luau but the table
-- itself (the few helpers it shares are copied, so the files merge freely).
-- The rule for a kind is in visuals/README.md ("The kit").
local ui, kit = ...

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

local TONES = {
	neutral = "muted",
	good = "success",
	warn = "warning",
	bad = "danger",
	info = "accent",
	success = "success",
	warning = "warning",
	danger = "danger",
	accent = "accent",
	muted = "muted",
}

local function tone(value)
	if value == nil then
		return nil
	end
	local token = TONES[value]
	if token == nil then
		fail("has an unknown tone '" .. tostring(value) .. "': use neutral, good, warn, bad, or info")
	end
	return token
end

-- `constructor(props)` with the children appended, skipping nils.
local function build(constructor, props, ...)
	for i = 1, select("#", ...) do
		local child = select(i, ...)
		if child ~= nil then
			table.insert(props, child)
		end
	end
	return constructor(props)
end

local function children(props)
	local out = {}
	for i, child in ipairs(props) do
		out[i] = child
	end
	return out
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

local function num(n)
	return (string.format("%.1f", n):gsub("%.0$", ""))
end

local function list_of(props, key, what)
	local items = props[key] or children(props)
	if type(items) ~= "table" or #items == 0 then
		fail("needs " .. key .. " = { " .. what .. ", ... }")
	end
	return items
end

local function item_table(item, index, needs)
	if type(item) ~= "table" or rawget(item, "__kind") ~= nil then
		fail("item " .. index .. " must be a table { " .. needs .. " }")
	end
	return item
end

-- Dates ---------------------------------------------------------------------

local MONTHS = { "Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec" }
local MONTH_NAMES =
	{ "January", "February", "March", "April", "May", "June", "July", "August", "September", "October", "November", "December" }

-- Days since 1970-01-01 of a calendar date, and back.
local function days_from_civil(y, m, d)
	if m <= 2 then
		y = y - 1
	end
	local era = math.floor(y / 400)
	local yoe = y - era * 400
	local doy = math.floor((153 * ((m + 9) % 12) + 2) / 5) + d - 1
	local doe = yoe * 365 + math.floor(yoe / 4) - math.floor(yoe / 100) + doy
	return era * 146097 + doe - 719468
end

local function civil_from_days(z)
	z = z + 719468
	local era = math.floor(z / 146097)
	local doe = z - era * 146097
	local yoe = math.floor((doe - math.floor(doe / 1460) + math.floor(doe / 36524) - math.floor(doe / 146096)) / 365)
	local y = yoe + era * 400
	local doy = doe - (365 * yoe + math.floor(yoe / 4) - math.floor(yoe / 100))
	local mp = math.floor((5 * doy + 2) / 153)
	local d = doy - math.floor((153 * mp + 2) / 5) + 1
	local m = mp < 10 and mp + 3 or mp - 9
	if m <= 2 then
		y = y + 1
	end
	return y, m, d
end

local function parse_date(value, what)
	local y, m, d = tostring(value):match("^(%d%d%d%d)-(%d%d)-(%d%d)")
	if y == nil then
		fail("has a bad " .. what .. " '" .. tostring(value) .. "': use yyyy-mm-dd")
	end
	y, m, d = tonumber(y), tonumber(m), tonumber(d)
	local z = days_from_civil(y, m, d)
	local ry, rm, rd = civil_from_days(z)
	if ry ~= y or rm ~= m or rd ~= d then
		fail("has a bad " .. what .. " '" .. tostring(value) .. "': that day does not exist")
	end
	return z
end

local function short_date(z)
	local _, m, d = civil_from_days(z)
	return MONTHS[m] .. " " .. d
end

local function month_start(y, m)
	return days_from_civil(y, m, 1)
end

local function trunc(s, chars)
	s = tostring(s)
	if #s <= chars then
		return s
	end
	return s:sub(1, math.max(1, chars - 1)) .. "…"
end

-- Board -----------------------------------------------------------------------

local BOARD_ORDER = { "backlog", "todo", "in_progress", "in_review", "blocked", "done" }
local PRIORITY = { urgent = "danger", high = "warning", medium = "muted", low = "faint" }

local function status_key(value)
	return (tostring(value):lower():gsub("[%s%-]+", "_"))
end

local function pretty(value)
	local s = tostring(value):gsub("[_%-]+", " ")
	return s:sub(1, 1):upper() .. s:sub(2)
end

local function board_item(item)
	local meta = {}
	local function add(node)
		table.insert(meta, node)
	end
	if item.key then
		add(ui.text({ tostring(item.key), size = "xs", color = "faint" }))
	end
	local priority = item.priority and status_key(item.priority) or nil
	if priority ~= nil and priority ~= "none" then
		local token = PRIORITY[priority]
		if token == nil then
			fail("has an unknown priority '" .. tostring(item.priority) .. "': use urgent, high, medium, low, or none")
		end
		add(ui.text({ priority, size = "xs", weight = 600, color = token }))
	end
	if item.owner then
		add(ui.text({ tostring(item.owner), size = "xs", color = "muted" }))
	end
	if item.due then
		add(ui.text({ "due " .. tostring(item.due), size = "xs", color = "muted" }))
	end
	local labels = item.labels
	local title
	if item.url then
		title = ui.link({ label = tostring(item.title), url = item.url })
	else
		title = ui.text({ tostring(item.title), weight = 500 })
	end
	local detail = item.detail and tostring(item.detail):match("[^\n]+") or nil
	return build(
		ui.column,
		{ gap = 2 },
		title,
		#meta > 0 and ui.row({ gap = 8, wrap = true, meta }) or nil,
		type(labels) == "table" and #labels > 0 and ui.text({ table.concat(labels, " · "), size = "xs", color = "faint" })
			or nil,
		detail and ui.text({ detail, size = "sm", color = "muted" }) or nil,
		item.agentsWorking and ui.text({ item.agentsWorking .. " agent" .. (item.agentsWorking == 1 and "" or "s") .. " working", size = "xs", color = "accent" })
			or nil
	)
end

define("Board", { "items", "columns" }, function(p)
	local items = list_of(p, "items", "{ title, status, ... }")
	local names, order = {}, {}
	local function column(key, label)
		if names[key] == nil then
			names[key] = { label = label, items = {} }
			table.insert(order, key)
		end
	end
	if p.columns ~= nil then
		for _, name in ipairs(p.columns) do
			column(status_key(name), tostring(name))
		end
		if #order == 0 then
			fail("has empty columns: leave it out or list the status names")
		end
	end
	for i, item in ipairs(items) do
		item_table(item, i, "title, status, ...")
		if item.title == nil or item.status == nil then
			fail("item " .. i .. " needs title and status")
		end
		local key = status_key(item.status)
		if p.columns == nil then
			column(key, pretty(item.status))
		elseif names[key] == nil then
			fail("item '" .. tostring(item.title) .. "' has status '" .. tostring(item.status) .. "', which is not in columns")
		end
		table.insert(names[key].items, item)
	end
	if p.columns == nil then
		local rank = {}
		for i, key in ipairs(BOARD_ORDER) do
			rank[key] = i
		end
		local seen = {}
		for i, key in ipairs(order) do
			seen[key] = i
		end
		table.sort(order, function(a, b)
			local ra, rb = rank[a] or 100 + seen[a], rank[b] or 100 + seen[b]
			return ra < rb
		end)
	end
	local columns = {}
	for _, key in ipairs(order) do
		local col = names[key]
		local done = key == "done" or key == "merged"
		local blocked = key == "blocked" or key == "closed"
		table.insert(
			columns,
			build(
				ui.column,
				{ gap = 10, width = 220 },
				ui.row({
					gap = 8,
					ui.text({ col.label, weight = 600, color = blocked and "danger" or (done and "success" or nil) }),
					ui.text({ tostring(#col.items), color = "faint" }),
				}),
				#col.items == 0 and ui.text({ "—", color = "faint" }) or nil,
				ui.map(col.items, board_item)
			)
		)
	end
	return ui.row({ gap = 28, wrap = true, align = "start", columns })
end)

-- Roadmap -----------------------------------------------------------------------

local ROAD = {
	done = "success",
	active = "accent",
	["on-track"] = "success",
	["at-risk"] = "warning",
	["off-track"] = "danger",
	blocked = "danger",
	planned = "muted",
}

define("Roadmap", { "items" }, function(p)
	local items = list_of(p, "items", "{ title, start, end, lane?, status? }")
	local lane_order, lanes = {}, {}
	local lo, hi
	for i, item in ipairs(items) do
		item_table(item, i, "title, start, end")
		if item.title == nil then
			fail("item " .. i .. " needs a title")
		end
		local first, last, milestone
		if item.milestone ~= nil then
			first = parse_date(item.milestone, "milestone date")
			last = first
			milestone = true
		else
			if item.start == nil or (item["end"] == nil and item.days == nil) then
				fail("item '" .. tostring(item.title) .. "' needs start and end (or days), or milestone")
			end
			first = parse_date(item.start, "start")
			if item["end"] ~= nil then
				last = parse_date(item["end"], "end")
			else
				last = first + math.max(1, tonumber(item.days) or 1) - 1
			end
			if last < first then
				fail("item '" .. tostring(item.title) .. "' ends before it starts")
			end
		end
		local status = item.status
		if status ~= nil and ROAD[status] == nil then
			fail(
				"has an unknown status '"
					.. tostring(status)
					.. "': use planned, active, done, at-risk, off-track, on-track, or blocked"
			)
		end
		local lane = tostring(item.lane or item.group or "")
		if lanes[lane] == nil then
			lanes[lane] = {}
			table.insert(lane_order, lane)
		end
		local row = {
			title = tostring(item.title),
			first = first,
			last = last,
			milestone = milestone,
			status = status,
			progress = tonumber(item.progress),
			after = item.after,
			detail = item.detail,
		}
		table.insert(lanes[lane], row)
		lo = lo and math.min(lo, first) or first
		hi = hi and math.max(hi, last + 1) or (last + 1)
	end
	-- Rows top to bottom: a heading per lane (when lanes are named), then its items.
	local rows, by_title, details = {}, {}, {}
	for _, lane in ipairs(lane_order) do
		if lane ~= "" then
			table.insert(rows, { heading = lane })
		end
		for _, row in ipairs(lanes[lane]) do
			table.insert(rows, row)
			by_title[row.title] = #rows
			if row.detail then
				table.insert(details, ui.text({ row.title .. ": " .. tostring(row.detail), size = "sm", color = "muted" }))
			end
		end
	end
	for _, row in ipairs(rows) do
		if row.after ~= nil and by_title[tostring(row.after)] == nil then
			fail("item '" .. row.title .. "' comes after '" .. tostring(row.after) .. "', which is not an item")
		end
	end
	local span = math.max(hi - lo, 1)
	local head, row_h = 22, 24
	local height = head + #rows * row_h + 6
	local canvas = ui.canvas({
		height = height,
		draw = function(g, w)
			local label_w = ui.clamp(w * 0.3, 90, 170)
			local x0 = label_w + 8
			local plot = math.max(w - x0 - 6, 40)
			local function sx(day)
				return x0 + (day - lo) / span * plot
			end
			-- Time scale: months when long, Mondays when short.
			local ticks = {}
			if span > 45 then
				local y, m = civil_from_days(lo)
				local z
				repeat
					m = m + 1
					if m > 12 then
						m, y = 1, y + 1
					end
					z = month_start(y, m)
					if z < hi then
						table.insert(ticks, { z, m == 1 and (MONTHS[m] .. " " .. y) or MONTHS[m] })
					end
				until z >= hi
			else
				for z = lo, hi - 1 do
					if (z + 3) % 7 == 0 then
						table.insert(ticks, { z, short_date(z) })
					end
				end
			end
			local last_x = -100
			for _, tick in ipairs(ticks) do
				local x = sx(tick[1])
				g:line(x, head - 4, x, height, { color = ui.color("text", 0.08), width = 1 })
				if x - last_x >= 44 and x < w - 20 then
					g:text(x + 3, 2, tick[2], { size = "xs", color = "faint" })
					last_x = x
				end
			end
			local chars = math.floor(label_w / 6.5)
			local mid = function(i)
				return head + (i - 1) * row_h + row_h / 2
			end
			-- Dependencies first, under the bars.
			for i, row in ipairs(rows) do
				if row.after ~= nil then
					local j = by_title[tostring(row.after)]
					local pred = rows[j]
					local xe, xs = sx(pred.last + 1), sx(row.first)
					local bend = math.min(xe + 5, xs - 3)
					g:polyline({ { xe, mid(j) }, { bend, mid(j) }, { bend, mid(i) }, { xs, mid(i) } }, { color = ui.color("text", 0.3), width = 1 })
				end
			end
			for i, row in ipairs(rows) do
				local y = mid(i)
				if row.heading then
					g:text(0, y, trunc(row.heading, 40), { size = "sm", weight = 600, color = "muted", baseline = "center" })
				else
					g:text(8, y, trunc(row.title, chars), { size = "sm", baseline = "center" })
					local token = row.status and ROAD[row.status] or "muted"
					if row.milestone then
						local x, r = sx(row.first + 0.5), 7
						g:polygon({ { x, y - r }, { x + r, y }, { x, y + r }, { x - r, y } }, { fill = token == "muted" and "text" or token })
					else
						local x1, x2 = sx(row.first), sx(row.last + 1)
						local bw = math.max(x2 - x1, 4)
						local bar_y, bar_h = y - row_h / 2 + 6, row_h - 12
						if row.progress ~= nil then
							g:rect(x1, bar_y, bw, bar_h, { fill = ui.color(token, 0.3), radius = 3 })
							local share = ui.clamp(row.progress / 100, 0, 1)
							if share > 0 then
								g:rect(x1, bar_y, math.max(4, bw * share), bar_h, { fill = token, radius = 3 })
							end
						elseif token == "muted" then
							g:rect(x1, bar_y, bw, bar_h, { fill = ui.color("text", 0.25), radius = 3 })
						else
							g:rect(x1, bar_y, bw, bar_h, { fill = token, radius = 3 })
						end
					end
				end
			end
		end,
	})
	return build(ui.column, { gap = 10 }, canvas, #details > 0 and ui.column({ gap = 2, details }) or nil)
end)

-- Plan ----------------------------------------------------------------------------

-- Steps the reader approves: a checklist (all ticked but optional steps) and
-- the answer button. The steps left ticked are what was approved.
define("Plan", { "steps", "name", "label", "submit", "answer" }, function(p)
	local steps = list_of(p, "steps", "{ title, detail?, files?, risk?, estimate? }")
	local options, value = {}, {}
	for i, step in ipairs(steps) do
		if type(step) ~= "table" then
			step = { title = step }
		end
		if step.title == nil then
			fail("step " .. i .. " needs a title")
		end
		local meta = {}
		if step.risk then
			table.insert(meta, tostring(step.risk) .. " risk")
		end
		if step.estimate then
			table.insert(meta, tostring(step.estimate))
		end
		if type(step.files) == "table" and #step.files > 0 then
			table.insert(meta, table.concat(step.files, ", "))
		end
		local parts = {}
		if step.detail then
			table.insert(parts, tostring(step.detail))
		end
		if step.optional then
			table.insert(parts, "optional")
		end
		if #meta > 0 then
			table.insert(parts, table.concat(meta, " · "))
		end
		local title = tostring(step.title)
		table.insert(options, {
			value = title,
			label = title,
			description = #parts > 0 and table.concat(parts, "  ·  ") or nil,
		})
		if not step.optional then
			table.insert(value, title)
		end
	end
	local checklist = ui.checklist({
		name = p.name or "plan",
		label = p.label or "Steps to carry out",
		options = options,
		value = value,
	})
	if p.answer == false then
		return checklist
	end
	return ui.column({ gap = 12, checklist, ui.answer({ label = p.submit or "Approve" }) })
end)

-- Decision ----------------------------------------------------------------------------

local weights_by = {}

define("Decision", { "options", "criteria", "max", "key" }, function(p)
	local options = p.options
	local criteria = p.criteria
	if type(options) ~= "table" or #options < 2 then
		fail("needs options = { { name, scores, pros?, cons? }, ... } (at least two)")
	end
	if type(criteria) ~= "table" or #criteria < 1 then
		fail("needs criteria = { { name, weight? }, ... }")
	end
	local names = {}
	for i, c in ipairs(criteria) do
		if type(c) ~= "table" then
			c = { name = c }
			criteria[i] = c
		end
		if c.name == nil then
			fail("criterion " .. i .. " needs a name")
		end
		names[i] = tostring(c.name)
	end
	local top = 0
	local rows = {}
	for i, option in ipairs(options) do
		if type(option) ~= "table" or option.name == nil or type(option.scores) ~= "table" then
			fail("option " .. i .. " needs name and scores")
		end
		local scores = {}
		for j, crit in ipairs(names) do
			local s = tonumber(option.scores[j] ~= nil and option.scores[j] or option.scores[crit])
			if s == nil then
				fail("option '" .. tostring(option.name) .. "' has no score for '" .. crit .. "'")
			end
			scores[j] = s
			top = math.max(top, s)
		end
		rows[i] = { option = option, name = tostring(option.name), scores = scores }
	end
	local max = tonumber(p.max) or (top > 5 and 10 or 5)
	local id = p.key or table.concat(names, "|")
	local weights = weights_by[id]
	if weights == nil then
		weights = {}
		for j, c in ipairs(criteria) do
			weights[j] = tonumber(c.weight) or 1
		end
		weights_by[id] = weights
	end
	local total = 0
	for _, w in ipairs(weights) do
		total = total + w
	end
	for _, row in ipairs(rows) do
		local sum = 0
		for j, s in ipairs(row.scores) do
			sum = sum + s * weights[j]
		end
		row.total = total > 0 and sum / total or 0
	end
	local ranked = table.clone(rows)
	table.sort(ranked, function(a, b)
		if a.total ~= b.total then
			return a.total > b.total
		end
		return a.name < b.name
	end)
	local wmax = 0
	for _, w in ipairs(weights) do
		wmax = math.max(wmax, w)
	end
	local sliders = {}
	for j, c in ipairs(criteria) do
		table.insert(
			sliders,
			ui.slider({
				label = names[j] .. (c.note and (" · " .. tostring(c.note)) or ""),
				value = weights[j],
				min = 0,
				max = math.max(10, wmax * 2),
				step = 1,
				decimals = 0,
				on_change = function(v)
					weights[j] = v
				end,
			})
		)
	end
	local ranking = ui.canvas({
		height = #ranked * 26,
		draw = function(g, w)
			local name_w = ui.clamp(w * 0.35, 100, 200)
			local bar_x = 22 + name_w
			local bar_w = math.max(w - bar_x - 60, 30)
			for i, row in ipairs(ranked) do
				local y = (i - 1) * 26 + 13
				g:text(0, y, tostring(i), { size = "sm", color = "faint", baseline = "center" })
				g:text(20, y, trunc(row.name, math.floor(name_w / 6.5)), { size = "sm", weight = i == 1 and 600 or 400, baseline = "center" })
				g:rect(bar_x, y - 3, bar_w, 6, { fill = ui.color("text", 0.12), radius = 3 })
				g:rect(bar_x, y - 3, math.max(4, bar_w * ui.clamp(row.total / max, 0, 1)), 6, { fill = i == 1 and "accent" or ui.color("accent", 0.55), radius = 3 })
				g:text(w, y, num(row.total) .. " / " .. num(max), { size = "sm", color = "muted", align = "end", baseline = "center" })
			end
		end,
	})
	local columns = { "Option" }
	for _, name in ipairs(names) do
		table.insert(columns, name)
	end
	local matrix = {}
	for i, row in ipairs(rows) do
		matrix[i] = { row.name }
		for _, s in ipairs(row.scores) do
			table.insert(matrix[i], s)
		end
	end
	local notes = {}
	for _, row in ipairs(rows) do
		local o = row.option
		local lines = {}
		for _, pro in ipairs(o.pros or {}) do
			table.insert(lines, ui.text({ "+ " .. tostring(pro), size = "sm", color = "success" }))
		end
		for _, con in ipairs(o.cons or {}) do
			table.insert(lines, ui.text({ "− " .. tostring(con), size = "sm", color = "danger" }))
		end
		if o.note then
			table.insert(lines, ui.text({ tostring(o.note), size = "sm", color = "muted" }))
		end
		if #lines > 0 then
			table.insert(notes, build(ui.column, { gap = 2 }, ui.text({ row.name, weight = 600 }), lines))
		end
	end
	return build(
		ui.column,
		{ gap = 16 },
		ranking,
		ui.column({ gap = 4, ui.text({ "How much each criterion counts (try other weights; the ranking follows)", size = "sm", color = "muted" }), sliders }),
		ui.table({ columns = columns, rows = matrix }),
		#notes > 0 and ui.column({ gap = 12, notes }) or nil
	)
end)

-- Estimate ----------------------------------------------------------------------------

define("Estimate", { "tasks", "unit" }, function(p)
	local tasks = list_of(p, "tasks", "{ title, low, high, likely? }")
	local unit = p.unit and (" " .. tostring(p.unit)) or ""
	local rows = {}
	local mean, var, lows, highs = 0, 0, 0, 0
	for i, task in ipairs(tasks) do
		item_table(task, i, "title, low, high")
		local low, high = tonumber(task.low), tonumber(task.high)
		if task.title == nil or low == nil or high == nil then
			fail("task " .. i .. " needs title, low and high")
		end
		if low > high then
			low, high = high, low
		end
		local likely = tonumber(task.likely) or (low + high) / 2
		likely = ui.clamp(likely, low, high)
		local expected = (low + 4 * likely + high) / 6
		local sd = (high - low) / 6
		mean, var, lows, highs = mean + expected, var + sd * sd, lows + low, highs + high
		table.insert(rows, { tostring(task.title), num(low), num(likely), num(high), num(expected) })
	end
	local p90 = mean + 1.2816 * math.sqrt(var)
	table.insert(rows, {
		ui.text({ "Total", weight = 600 }),
		ui.text({ num(lows), weight = 600 }),
		"",
		ui.text({ num(highs), weight = 600 }),
		ui.text({ num(mean), weight = 600 }),
	})
	return ui.column({
		gap = 14,
		ui.table({
			columns = {
				"Task",
				{ label = "Low", align = "end" },
				{ label = "Likely", align = "end" },
				{ label = "High", align = "end" },
				{ label = "Expected", align = "end" },
			},
			rows = rows,
		}),
		ui.row({
			gap = 40,
			ui.stat({ value = num(mean) .. unit, label = "P50, even odds" }),
			ui.stat({ value = num(p90) .. unit, label = "P90, nine in ten" }),
			ui.stat({ value = num(lows) .. "–" .. num(highs) .. unit, label = "Best to worst case" }),
		}),
		ui.text({ "Expected = (low + 4 × likely + high) / 6; P90 adds 1.28 × the spread of the sum.", size = "xs", color = "faint" }),
	})
end)

-- Table ------------------------------------------------------------------------------

define("Table", { "columns", "rows", "hover" }, function(p)
	if type(p.columns) ~= "table" or #p.columns == 0 then
		fail("needs columns = { \"...\", ... }")
	end
	if type(p.rows) ~= "table" then
		fail("needs rows = { { cell, ... }, ... }")
	end
	local rows = {}
	for i, row in ipairs(p.rows) do
		if type(row) ~= "table" or rawget(row, "__kind") ~= nil then
			fail("row " .. i .. " must be a list of cells")
		end
		if #row > #p.columns then
			fail("row " .. i .. " has " .. #row .. " cells for " .. #p.columns .. " columns")
		end
		local out = {}
		for j, value in ipairs(row) do
			if value == true then
				value = "✓"
			elseif value == false then
				value = "–"
			end
			out[j] = value
		end
		rows[i] = out
	end
	return ui.table({ columns = p.columns, rows = rows, hover = p.hover })
end)

-- Timeline ----------------------------------------------------------------------------

define("Timeline", { "items" }, function(p)
	local items = list_of(p, "items", "{ time, title, detail?, tone? }")
	return ui.column({
		gap = 12,
		ui.map(items, function(item, i)
			item_table(item, i, "time, title")
			if item.title == nil then
				fail("item " .. i .. " needs a title")
			end
			local token = tone(item.tone)
			return ui.row({
				gap = 10,
				align = "start",
				ui.text({ tostring(item.time or ""), size = "sm", color = "muted", width = 64 }),
				ui.text({ "●", color = token or "faint", width = 14 }),
				build(
					ui.column,
					{ gap = 2, grow = true },
					ui.text({ tostring(item.title), weight = 500, color = token }),
					cell(item.detail, { size = "sm", color = "muted" })
				),
			})
		end),
	})
end)

-- Calendar ----------------------------------------------------------------------------

local WEEKDAYS = { "Mon", "Tue", "Wed", "Thu", "Fri", "Sat", "Sun" }

define("Calendar", { "events", "month" }, function(p)
	local events = list_of(p, "events", "{ date, title, tone? }")
	local by_day, first, last = {}, nil, nil
	for i, event in ipairs(events) do
		item_table(event, i, "date, title")
		if event.title == nil then
			fail("event " .. i .. " needs a title")
		end
		local z = parse_date(event.date, "date")
		by_day[z] = by_day[z] or {}
		table.insert(by_day[z], { title = tostring(event.title), token = tone(event.tone) })
		first = first and math.min(first, z) or z
		last = last and math.max(last, z) or z
	end
	local months = {}
	if p.month ~= nil then
		local y, m = tostring(p.month):match("^(%d%d%d%d)-(%d%d)$")
		if y == nil or tonumber(m) < 1 or tonumber(m) > 12 then
			fail("has a bad month '" .. tostring(p.month) .. "': use yyyy-mm")
		end
		months[1] = { tonumber(y), tonumber(m) }
	else
		local y, m = civil_from_days(first)
		local ly, lm = civil_from_days(last)
		while y < ly or (y == ly and m <= lm) do
			table.insert(months, { y, m })
			m = m + 1
			if m > 12 then
				m, y = 1, y + 1
			end
			if #months > 6 then
				fail("spans more than six months: pass month = \"yyyy-mm\" for one")
			end
		end
	end
	local out = {}
	for _, ym in ipairs(months) do
		local y, m = ym[1], ym[2]
		local z0 = month_start(y, m)
		local ny, nm = y, m + 1
		if nm > 12 then
			nm, ny = 1, y + 1
		end
		local len = month_start(ny, nm) - z0
		local header = {}
		for _, name in ipairs(WEEKDAYS) do
			table.insert(header, ui.text({ name, size = "xs", color = "faint", align = "center" }))
		end
		local cells = {}
		for _ = 1, (z0 + 3) % 7 do
			table.insert(cells, ui.text({ " " }))
		end
		local agenda = {}
		for d = 1, len do
			local list = by_day[z0 + d - 1]
			local dots = {}
			if list then
				for k, e in ipairs(list) do
					if k <= 4 then
						table.insert(dots, ui.span({ "●", color = e.token or "text" }))
					end
				end
				local titles = {}
				for _, e in ipairs(list) do
					table.insert(titles, ui.span({ e.title, color = e.token }))
					table.insert(titles, ui.span({ "  ", color = "faint" }))
				end
				table.remove(titles)
				table.insert(agenda, ui.row({
					gap = 10,
					align = "start",
					ui.text({ MONTHS[m] .. " " .. d, size = "sm", color = "muted", width = 56 }),
					ui.text(titles),
				}))
			end
			table.insert(
				cells,
				build(
					ui.column,
					{ gap = 1 },
					ui.text({ tostring(d), size = "sm", weight = list and 600 or 400, color = (not list) and "muted" or nil, align = "center" }),
					list and ui.text({ size = "xs", align = "center", table.unpack(dots) }) or ui.text({ " ", size = "xs" })
				)
			)
		end
		table.insert(
			out,
			ui.column({
				gap = 10,
				ui.text({ MONTH_NAMES[m] .. " " .. y, weight = 600 }),
				ui.grid({ columns = 7, gap = 6, header }),
				ui.grid({ columns = 7, gap = 6, cells }),
				#agenda > 0 and ui.column({ gap = 4, agenda }) or nil,
			})
		)
	end
	return ui.column({ gap = 22, out })
end)

-- CalendarHeatmap ------------------------------------------------------------------------

define("CalendarHeatmap", { "days", "label", "tone" }, function(p)
	local days = p.days
	if type(days) ~= "table" then
		fail("needs days = { [\"yyyy-mm-dd\"] = count } or { { date, value }, ... }")
	end
	local values, lo, hi, top = {}, nil, nil, 0
	local function put(date, value)
		local z = parse_date(date, "date")
		value = tonumber(value)
		if value == nil then
			fail("has a day without a number: " .. tostring(date))
		end
		values[z] = (values[z] or 0) + value
		lo = lo and math.min(lo, z) or z
		hi = hi and math.max(hi, z) or z
	end
	if #days > 0 then
		for i, day in ipairs(days) do
			item_table(day, i, "date, value")
			put(day.date, day.value or 1)
		end
	else
		for date, value in pairs(days) do
			put(date, value)
		end
	end
	if lo == nil then
		fail("has no days")
	end
	for _, v in pairs(values) do
		top = math.max(top, v)
	end
	if hi - lo > 370 then
		lo = hi - 370
	end
	local token = tone(p.tone) or "success"
	local sunday = lo - (lo + 4) % 7
	local weeks = math.floor((hi - sunday) / 7) + 1
	local label = p.label
	return ui.canvas({
		height = 14 + 7 * 13 + 24,
		draw = function(g, w)
			local x0 = 26
			local pitch = ui.clamp((w - x0) / weeks, 4, 13)
			local size = pitch - 2
			local function level(v)
				if v == nil or v <= 0 or top <= 0 then
					return ui.color("text", 0.08)
				end
				local steps = { 0.3, 0.5, 0.75, 1 }
				return ui.color(token, steps[math.max(1, math.min(4, math.ceil(4 * v / top)))])
			end
			for i, name in ipairs({ "Mon", "Wed", "Fri" }) do
				g:text(0, 14 + (i * 2 - 1) * 13 + 1, name, { size = "xs", color = "faint" })
			end
			local last_label = -100
			for week = 0, weeks - 1 do
				local x = x0 + week * pitch
				local _, m, d = civil_from_days(sunday + week * 7)
				if d <= 7 and x - last_label >= 30 then
					g:text(x, 0, MONTHS[m], { size = "xs", color = "faint" })
					last_label = x
				end
				for row = 0, 6 do
					local z = sunday + week * 7 + row
					if z >= lo and z <= hi then
						g:rect(x, 14 + row * 13, size, size, { fill = level(values[z]), radius = 2 })
					end
				end
			end
			local read = ""
			local pointer = g.pointer
			if pointer then
				local week = math.floor((pointer.x - x0) / pitch)
				local row = math.floor((pointer.y - 14) / 13)
				local z = sunday + week * 7 + row
				if week >= 0 and week < weeks and row >= 0 and row <= 6 and z >= lo and z <= hi then
					read = short_date(z) .. ": " .. num(values[z] or 0) .. (label and (" " .. tostring(label)) or "")
				end
			end
			local y = 14 + 7 * 13 + 6
			g:text(0, y, read ~= "" and read or tostring(label or ""), { size = "sm", color = read ~= "" and "text" or "muted" })
			g:text(w - 4 * 13 - 36, y + 2, "Less", { size = "xs", color = "faint", align = "end" })
			for i, alpha in ipairs({ 0.08, 0.3, 0.5, 0.75, 1 }) do
				local fill = i == 1 and ui.color("text", 0.08) or ui.color(token, alpha)
				g:rect(w - 5 * 13 - 28 + (i - 1) * 13 + 4, y + 3, 11, 11, { fill = fill, radius = 2 })
			end
			g:text(w, y + 2, "More", { size = "xs", color = "faint", align = "end" })
		end,
	})
end)

-- Provenance ----------------------------------------------------------------------------

local STAGE = {
	done = { "✓", "success", "Done" },
	failed = { "✕", "danger", "Failed" },
	running = { "●", "accent", "Running" },
	pending = { "○", "faint", "Not started" },
	skipped = { "–", "faint", "Skipped" },
	unknown = { "?", "warning", "No receipt" },
}
local STAGE_ALIAS = {
	success = "done",
	succeeded = "done",
	ok = "done",
	passed = "done",
	error = "failed",
	failure = "failed",
	["in-progress"] = "running",
	active = "running",
	waiting = "pending",
	queued = "pending",
}
local IDENTITY = { branch = true, sha = true, commit = true, revision = true, build = true, version = true, digest = true }

local function fact_short(key, value)
	value = tostring(value)
	if (key == "sha" or key == "commit" or key == "revision") and #value >= 40 and value:match("^%x+$") then
		return value:sub(1, 7)
	end
	if key == "digest" and #value > 19 then
		return value:sub(1, 19) .. "…"
	end
	return value
end

-- Equal, or one is the start of the other (a short sha).
local function same_fact(a, b)
	a, b = tostring(a), tostring(b)
	return a == b or (#a >= 7 and #b >= 7 and (a:sub(1, #b) == b or b:sub(1, #a) == a))
end

define("Provenance", { "stages", "expected" }, function(p)
	local stages = list_of(p, "stages", "{ name, status, facts? }")
	local expected = p.expected or {}
	local seen = {}
	local mismatches, last_receipt = {}, 0
	local blocks = {}
	for i, stage in ipairs(stages) do
		item_table(stage, i, "name, status, facts?")
		if stage.name == nil then
			fail("stage " .. i .. " needs a name")
		end
		local status = stage.status and tostring(stage.status):lower() or "unknown"
		status = STAGE_ALIAS[status] or status
		local mark = STAGE[status]
		if mark == nil then
			fail("has an unknown status '" .. tostring(stage.status) .. "': use done, failed, running, pending, skipped, or unknown")
		end
		if status == "done" or status == "failed" or status == "running" then
			last_receipt = i
		end
		local keys = {}
		for key in pairs(stage.facts or {}) do
			table.insert(keys, key)
		end
		table.sort(keys)
		local facts = {}
		for _, key in ipairs(keys) do
			local value = stage.facts[key]
			local shown = fact_short(key, value)
			local note, token
			if IDENTITY[key] and (status == "done" or status == "running") then
				if expected[key] ~= nil and not same_fact(value, expected[key]) then
					note, token = "  expected " .. fact_short(key, expected[key]), "danger"
					table.insert(mismatches, tostring(stage.name) .. " has " .. key .. " " .. shown .. ", expected " .. fact_short(key, expected[key]))
				elseif seen[key] ~= nil and not same_fact(value, seen[key].value) then
					note, token = "  was " .. fact_short(key, seen[key].value) .. " in " .. seen[key].stage, "danger"
					table.insert(mismatches, key .. " changes from " .. fact_short(key, seen[key].value) .. " (" .. seen[key].stage .. ") to " .. shown .. " (" .. tostring(stage.name) .. ")")
				end
				seen[key] = { value = value, stage = tostring(stage.name) }
			end
			table.insert(facts, ui.row({
				gap = 10,
				ui.text({ key, size = "sm", color = "muted", width = 80 }),
				ui.text({ size = "sm", mono = true, ui.span({ shown, color = token }), ui.span({ note or "", color = "danger" }) }),
			}))
		end
		local meta = {}
		for _, v in ipairs({ stage.time, stage.duration }) do
			table.insert(meta, tostring(v))
		end
		local evidence = {}
		local items = stage.evidence
		if type(items) ~= "table" or rawget(items, "__kind") ~= nil or items.image ~= nil then
			items = items and { items } or {}
		end
		for _, e in ipairs(items) do
			if type(e) == "string" and e:match("^https?://") then
				table.insert(evidence, ui.link({ label = e, url = e }))
			elseif type(e) == "table" and e.image ~= nil then
				table.insert(evidence, ui.text({ "image: " .. tostring(e.label or e.image), size = "sm", color = "muted" }))
			else
				table.insert(evidence, ui.text({ tostring(e), size = "sm", color = "muted", mono = true }))
			end
		end
		blocks[i] = ui.row({
			gap = 10,
			align = "start",
			ui.text({ mark[1], color = mark[2], weight = 700, width = 16 }),
			build(
				ui.column,
				{ gap = 3, grow = true },
				ui.row({ gap = 8, ui.text({ tostring(stage.name), weight = 600, grow = true }), ui.text({ mark[3], size = "sm", weight = 600, color = mark[2] }) }),
				#meta > 0 and ui.text({ table.concat(meta, " · "), size = "sm", color = "muted" }) or nil,
				#facts > 0 and ui.column({ gap = 1, facts }) or nil,
				#evidence > 0 and ui.column({ gap = 1, evidence }) or nil
			),
		})
	end
	local verdict
	if #mismatches > 0 then
		verdict = kit.Callout({ tone = "bad", title = "Mismatch", text = mismatches[1] .. (#mismatches > 1 and (" (and " .. (#mismatches - 1) .. " more)") or "") })
	elseif last_receipt == 0 then
		verdict = kit.Callout({ tone = "warn", title = "No receipt", text = "No stage reported an outcome." })
	elseif last_receipt < #stages then
		verdict = kit.Callout({ tone = "warn", title = "No receipt after " .. tostring(stages[last_receipt].name), text = "Later stages did not report, so they are unproven." })
	else
		verdict = kit.Callout({ tone = "good", title = "Consistent", text = "Every stage reported, and the facts agree." })
	end
	return ui.column({ gap = 14, verdict, blocks })
end)

-- Sources ----------------------------------------------------------------------------------

define("Sources", { "links" }, function(p)
	local links = list_of(p, "links", "\"https://...\" or { url, title?, note? }")
	return ui.column({
		gap = 8,
		ui.map(links, function(link, i)
			if type(link) ~= "table" then
				link = { url = link }
			end
			local url = tostring(link.url or "")
			local host = url:match("^https?://([^/]+)")
			if host == nil then
				fail("source " .. i .. " needs an http(s) url")
			end
			local label = link.title and tostring(link.title) or (url:gsub("^https?://", ""):gsub("/$", ""))
			return ui.row({
				gap = 10,
				align = "start",
				ui.text({ "[" .. i .. "]", size = "sm", color = "faint", width = 26 }),
				build(
					ui.column,
					{ gap = 1, grow = true },
					ui.link({ label = label, url = url }),
					link.title and ui.text({ host, size = "xs", color = "faint" }) or nil,
					cell(link.note, { size = "sm", color = "muted" })
				),
			})
		end),
	})
end)

-- Entity --------------------------------------------------------------------------------------

local ENTITY_KINDS = { product = true, place = true, person = true, org = true, link = true, other = true }

define("Entity", { "name", "kind", "facts", "url", "note" }, function(p)
	local name = p.name or p[1]
	if name == nil then
		fail("needs name = \"...\"")
	end
	if p.kind ~= nil and not ENTITY_KINDS[p.kind] then
		fail("has an unknown kind '" .. tostring(p.kind) .. "': use product, place, person, org, link, or other")
	end
	local has_facts = type(p.facts) == "table" and next(p.facts) ~= nil
	return build(
		ui.column,
		{ gap = 6 },
		ui.row({ gap = 8, align = "end", ui.text({ tostring(name), size = "lg", weight = 600 }), p.kind and ui.text({ p.kind, size = "xs", color = "faint" }) or nil }),
		cell(p.note, { color = "muted" }),
		has_facts and kit.KeyValue({ pairs = p.facts }) or nil,
		p.url and ui.link({ label = tostring(p.url):gsub("^https?://", ""), url = p.url }) or nil
	)
end)

-- Transcript ------------------------------------------------------------------------------------

local MAX_WORDS = 600

local function words_of(track, index)
	local list = {}
	if type(track.words) == "table" then
		for _, w in ipairs(track.words) do
			table.insert(list, type(w) == "table" and tostring(w.text) or tostring(w))
		end
	elseif track.text ~= nil then
		for w in tostring(track.text):gmatch("%S+") do
			table.insert(list, w)
		end
	else
		fail("track " .. index .. " needs words = { ... } or text = \"...\"")
	end
	if #list > MAX_WORDS then
		fail("track " .. index .. " has more than " .. MAX_WORDS .. " words; show an excerpt")
	end
	local tokens = {}
	for _, w in ipairs(list) do
		local norm = w:lower():gsub("%p", "")
		if norm ~= "" then
			table.insert(tokens, { shown = w, norm = norm })
		end
	end
	return tokens
end

-- Word by word against the reference: ok, sub (wrong word), ins (extra),
-- del (missed: the reference word, in brackets).
local function align(ref, hyp)
	local n, m = #ref, #hyp
	local d = {}
	for i = 0, n do
		d[i] = { [0] = i }
	end
	for j = 1, m do
		d[0][j] = j
	end
	for i = 1, n do
		local row, prev = d[i], d[i - 1]
		for j = 1, m do
			local cost = ref[i].norm == hyp[j].norm and 0 or 1
			row[j] = math.min(prev[j - 1] + cost, prev[j] + 1, row[j - 1] + 1)
		end
	end
	local ops, i, j = {}, n, m
	local counts = { sub = 0, ins = 0, del = 0 }
	while i > 0 or j > 0 do
		if i > 0 and j > 0 and d[i][j] == d[i - 1][j - 1] + (ref[i].norm == hyp[j].norm and 0 or 1) then
			local same = ref[i].norm == hyp[j].norm
			if not same then
				counts.sub = counts.sub + 1
			end
			table.insert(ops, { same and "ok" or "sub", hyp[j].shown })
			i, j = i - 1, j - 1
		elseif i > 0 and d[i][j] == d[i - 1][j] + 1 then
			counts.del = counts.del + 1
			table.insert(ops, { "del", ref[i].shown })
			i = i - 1
		else
			counts.ins = counts.ins + 1
			table.insert(ops, { "ins", hyp[j].shown })
			j = j - 1
		end
	end
	local out = {}
	for k = #ops, 1, -1 do
		table.insert(out, ops[k])
	end
	return out, counts
end

define("Transcript", { "tracks" }, function(p)
	local tracks = list_of(p, "tracks", "{ name, words | text }")
	if #tracks < 1 then
		fail("needs tracks")
	end
	local parsed = {}
	for i, track in ipairs(tracks) do
		item_table(track, i, "name, words | text")
		parsed[i] = words_of(track, i)
	end
	local ref = parsed[1]
	if #ref == 0 then
		fail("has an empty reference (the first track)")
	end
	local blocks = {
		ui.text({
			size = "xs",
			color = "faint",
			"Against the first track: ",
			ui.span({ "wrong word", color = "warning", weight = 600 }),
			"  ",
			ui.span({ "extra word", color = "danger", weight = 600 }),
			"  ",
			ui.span({ "[missed word]", color = "danger" }),
		}),
	}
	for i, track in ipairs(tracks) do
		local name = tostring(track.name or ("Track " .. i))
		local spans, summary, token
		if i == 1 then
			spans = {}
			for _, w in ipairs(ref) do
				table.insert(spans, w.shown)
				table.insert(spans, " ")
			end
			summary, token = "reference · " .. #ref .. " words", "muted"
		else
			local ops, counts = align(ref, parsed[i])
			spans = {}
			for _, op in ipairs(ops) do
				local kind, word = op[1], op[2]
				if kind == "ok" then
					table.insert(spans, word)
				elseif kind == "sub" then
					table.insert(spans, ui.span({ word, color = "warning", weight = 600 }))
				elseif kind == "ins" then
					table.insert(spans, ui.span({ word, color = "danger", weight = 600 }))
				else
					table.insert(spans, ui.span({ "[" .. word .. "]", color = "danger" }))
				end
				table.insert(spans, " ")
			end
			local wer = (counts.sub + counts.ins + counts.del) / #ref
			token = wer < 0.1 and "success" or (wer < 0.25 and "warning" or "danger")
			summary = string.format("WER %d%% · %d wrong, %d extra, %d missed", math.floor(wer * 100 + 0.5), counts.sub, counts.ins, counts.del)
		end
		table.insert(blocks, ui.column({
			gap = 3,
			ui.row({ gap = 10, ui.text({ name, weight = 600 }), ui.text({ summary, size = "sm", color = token }) }),
			ui.text(spans),
		}))
	end
	return ui.column({ gap = 14, blocks })
end)

-- Walkthrough --------------------------------------------------------------------------------------

local walk_at = {}

define("Walkthrough", { "steps", "key" }, function(p)
	local steps = list_of(p, "steps", "{ title, body, code?, language? }")
	for i, step in ipairs(steps) do
		item_table(step, i, "title, body")
		if step.title == nil then
			fail("step " .. i .. " needs a title")
		end
	end
	local id = p.key or (tostring(steps[1].title) .. #steps)
	local at = ui.clamp(walk_at[id] or 1, 1, #steps)
	local step = steps[at]
	local buttons = {}
	if at > 1 then
		table.insert(buttons, ui.button({ label = "Back", on_click = function() walk_at[id] = at - 1 end }))
	end
	if at < #steps then
		table.insert(buttons, ui.button({ label = "Next", primary = true, on_click = function() walk_at[id] = at + 1 end }))
	end
	return build(
		ui.column,
		{ gap = 10 },
		ui.text({ "Step " .. at .. " of " .. #steps, size = "xs", color = "faint" }),
		ui.text({ tostring(step.title), size = "lg", weight = 600 }),
		cell(step.body),
		step.code and ui.column({ gap = 2, step.language and ui.text({ tostring(step.language), size = "xs", color = "faint" }) or nil, ui.text({ tostring(step.code), mono = true, size = "sm" }) }) or nil,
		ui.row({ gap = 8, buttons })
	)
end)

-- Flashcards ---------------------------------------------------------------------------------------

local card_at = {}

define("Flashcards", { "cards", "key" }, function(p)
	local cards = list_of(p, "cards", "{ front, back }")
	for i, card in ipairs(cards) do
		item_table(card, i, "front, back")
		if card.front == nil or card.back == nil then
			fail("card " .. i .. " needs front and back")
		end
	end
	local id = p.key or (tostring(cards[1].front) .. #cards)
	local state = card_at[id]
	if state == nil then
		state = { at = 1, flipped = false }
		card_at[id] = state
	end
	local at = ui.clamp(state.at, 1, #cards)
	local card = cards[at]
	local buttons = {}
	if at > 1 then
		table.insert(buttons, ui.button({ label = "Previous", on_click = function() state.at, state.flipped = at - 1, false end }))
	end
	if at < #cards then
		table.insert(buttons, ui.button({ label = "Next", primary = true, on_click = function() state.at, state.flipped = at + 1, false end }))
	end
	return ui.column({
		gap = 12,
		ui.text({ "Card " .. at .. " of " .. #cards, size = "xs", color = "faint" }),
		ui.column({
			gap = 6,
			hover = true,
			on_click = function()
				state.flipped = not state.flipped
			end,
			ui.text({ state.flipped and "Back" or "Front", size = "xs", color = "faint" }),
			ui.text({ tostring(state.flipped and card.back or card.front), size = "lg", weight = 600 }),
			ui.text({ "Click the card to turn it", size = "xs", color = "faint" }),
		}),
		ui.row({ gap = 8, buttons }),
	})
end)
