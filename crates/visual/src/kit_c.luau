--!nocheck
-- ui.kit, batch c: charts, drawing, media and 3D kinds. Loaded after kit.luau
-- with its helpers, before the kit is frozen. Same rule as there (see
-- visuals/README.md, "The kit"): one call with data, built from the nodes
-- every renderer knows, Zend's style, errors that name the kind.
local ui, kit, h = ...
local define, fail, tone, build, children, cell = h.define, h.fail, h.tone, h.build, h.children, h.cell

-- Small helpers ---------------------------------------------------------------

-- Columns a character takes: two for East Asian wide characters, else one.
local function wide(cp)
	return (cp >= 0x1100 and cp <= 0x115F) or (cp >= 0x2E80 and cp <= 0xA4CF) or (cp >= 0xAC00 and cp <= 0xD7A3) or (cp >= 0xF900 and cp <= 0xFAFF) or (cp >= 0xFE30 and cp <= 0xFE4F) or (cp >= 0xFF00 and cp <= 0xFF60) or (cp >= 0xFFE0 and cp <= 0xFFE6) or cp >= 0x1F300
end

-- Columns a text takes (bytes, for text that is not UTF-8).
local function cols(s)
	s = tostring(s)
	if not utf8.len(s) then
		return #s
	end
	local n = 0
	for _, cp in utf8.codes(s) do
		n += wide(cp) and 2 or 1
	end
	return n
end

-- The longest start of 's' within 'max' columns, cut between characters.
local function head(s, max)
	if not utf8.len(s) then
		return string.sub(s, 1, max)
	end
	local n, last = 0, 0
	for p, cp in utf8.codes(s) do
		n += wide(cp) and 2 or 1
		if n > max then
			break
		end
		last = (utf8.offset(s, 2, p) or (#s + 1)) - 1
	end
	return string.sub(s, 1, last)
end

-- Width of a text in pixels, estimated (a canvas cannot measure text).
local function est(s, size)
	return cols(s) * (size == "xs" and 5.8 or 6.6)
end

-- At most as many characters as fit in 'width' pixels, with an ellipsis.
local function fit(s, width, size)
	s = tostring(s)
	local per = size == "xs" and 5.8 or 6.6
	local max = math.floor(width / per)
	if cols(s) <= max then
		return s
	end
	if max <= 1 then
		return ""
	end
	return head(s, max - 1) .. "…"
end

local function number(value, what)
	local n = tonumber(value)
	if n == nil then
		fail(what .. " must be a number, not " .. tostring(value))
	end
	return n
end

local function sorted_keys(map)
	local keys = {}
	for k in pairs(map) do
		table.insert(keys, k)
	end
	table.sort(keys, function(a, b)
		return tostring(a) < tostring(b)
	end)
	return keys
end

-- A byte count in KB, MB, ... when the unit says bytes; else value and unit.
local function with_unit(value, unit)
	if unit == "B" or unit == "bytes" then
		local steps = { "B", "KB", "MB", "GB", "TB" }
		local i = 1
		while math.abs(value) >= 1024 and i < #steps do
			value /= 1024
			i += 1
		end
		return ui.fmt(value, { decimals = i == 1 and 0 or 1 }) .. " " .. steps[i]
	end
	if unit then
		return ui.fmt(value, { unit = unit })
	end
	return ui.fmt(value)
end

local function legend(entries)
	return ui.row({
		gap = 14,
		wrap = true,
		ui.map(entries, function(e)
			return ui.row({
				gap = 5,
				ui.text({ "●", size = "xs", color = e.color }),
				ui.text({ e.name, size = "sm", color = "muted" }),
			})
		end),
	})
end

-- Series, however the agent wrote them: a list of numbers, a list of
-- { name, values, tone? }, or { name = values } (sorted by name; use the
-- list form for a fixed order). Each comes back as { name, values, color }.
local function series_of(raw, kind)
	if type(raw) ~= "table" or next(raw) == nil then
		fail("needs series = { ... } (numbers, { name = numbers }, or { { name, values, tone? } })")
	end
	local out = {}
	local function one(name, v)
		if type(v) == "table" and v.values ~= nil then
			table.insert(out, { name = v.name or name, values = v.values, tone = v.tone })
		elseif type(v) == "table" then
			table.insert(out, { name = name, values = v })
		else
			fail("series '" .. name .. "' needs a list of numbers")
		end
	end
	local first = raw[1]
	if type(first) == "number" or first == false then
		table.insert(out, { name = "", values = raw })
	elseif first ~= nil then
		for i, v in ipairs(raw) do
			one(type(v) == "table" and v.name or tostring(i), v)
		end
	else
		for _, k in ipairs(sorted_keys(raw)) do
			one(tostring(k), raw[k])
		end
	end
	for i, s in ipairs(out) do
		s.color = tone(s.tone) or ui.series(i)
		if #s.values == 0 then
			fail("series '" .. s.name .. "' has no values")
		end
	end
	return out
end

local function max_of(list)
	local m
	for _, v in ipairs(list) do
		if type(v) == "number" and (m == nil or v > m) then
			m = v
		end
	end
	return m
end

local function min_of(list)
	local m
	for _, v in ipairs(list) do
		if type(v) == "number" and (m == nil or v < m) then
			m = v
		end
	end
	return m
end

-- Chart -------------------------------------------------------------------------

local function plain_chart(p, kind, series)
	local labels = p.labels or {}
	local nodes = {}
	for i, s in ipairs(series) do
		local item = { name = s.name, color = s.color }
		if kind == "scatter" then
			local points = {}
			for j, y in ipairs(s.values) do
				local x = labels[j]
				if type(x) ~= "number" then
					fail("of type scatter needs labels = { x values } as numbers, one per value")
				end
				table.insert(points, { x, y })
			end
			item.points = points
		else
			item.values = s.values
		end
		table.insert(nodes, item)
	end
	return ui.chart({
		type = kind,
		x = kind ~= "scatter" and labels or nil,
		series = nodes,
		unit = p.unit,
		stacked = p.stacked,
		height = p.height,
		legend = #nodes > 1,
	})
end

local function pie_chart(p, series)
	local s = series[1]
	local labels = p.labels or {}
	local total = 0
	for _, v in ipairs(s.values) do
		total += math.max(v, 0)
	end
	if total <= 0 then
		fail("of type pie needs values above zero")
	end
	local n = #s.values
	local height = p.height or math.max(150, math.min(240, 40 + n * 24))
	return ui.canvas({
		height = height,
		draw = function(g, w, hh)
			local r = math.min(hh / 2 - 4, w * 0.32)
			local cx, cy = r + 4, hh / 2
			local a = -math.pi / 2
			for i, v in ipairs(s.values) do
				local sweep = math.max(v, 0) / total * 2 * math.pi
				if sweep > 0 then
					g:arc(cx, cy, r, a, a + math.min(sweep, 2 * math.pi - 1e-4), { fill = ui.series(i), inner = r * 0.58 })
				end
				a += sweep
			end
			local x0 = 2 * r + 28
			local step = math.min(24, (hh - 8) / n)
			local top = hh / 2 - step * n / 2
			for i, v in ipairs(s.values) do
				local y = top + (i - 1) * step
				g:text(x0, y, "●", { size = "xs", color = ui.series(i) })
				g:text(x0 + 16, y, fit(labels[i] or ("#" .. i), w - x0 - 130), { size = "sm" })
				local share = math.floor(math.max(v, 0) / total * 100 + 0.5)
				g:text(w, y, with_unit(v, p.unit) .. "  " .. share .. "%", { size = "sm", color = "muted", align = "end" })
			end
		end,
	})
end

local function radar_chart(p, series)
	local labels = p.labels or {}
	local n = #labels
	if n < 3 then
		fail("of type radar needs labels = { at least three axes }")
	end
	local top = p.max
	if top == nil then
		top = 0
		for _, s in ipairs(series) do
			top = math.max(top, max_of(s.values) or 0)
		end
	end
	if top <= 0 then
		top = 1
	end
	return ui.column({
		gap = 8,
		#series > 1 and legend(series) or nil,
		ui.canvas({
			height = p.height or 300,
			draw = function(g, w, hh)
				local cx, cy = w / 2, hh / 2
				local r = math.max(30, math.min(w / 2 - 80, hh / 2 - 22))
				local function at(i, frac)
					local a = -math.pi / 2 + (i - 1) / n * 2 * math.pi
					return cx + math.cos(a) * r * frac, cy + math.sin(a) * r * frac
				end
				for ring = 1, 4 do
					local pts = {}
					for i = 1, n do
						local x, y = at(i, ring / 4)
						table.insert(pts, { x, y })
					end
					table.insert(pts, pts[1])
					g:polyline(pts, { color = ui.color("text", ring == 4 and 0.22 or 0.1) })
				end
				for i = 1, n do
					local x, y = at(i, 1)
					g:line(cx, cy, x, y, { color = ui.color("text", 0.1) })
					local lx, ly = at(i, 1.12)
					local align = math.abs(lx - cx) < 8 and "center" or (lx > cx and "start" or "end")
					g:text(lx, ly, labels[i], { size = "sm", color = "muted", align = align, baseline = "center" })
				end
				for si, s in ipairs(series) do
					local pts = {}
					for i = 1, n do
						local x, y = at(i, ui.clamp((s.values[i] or 0) / top, 0, 1))
						table.insert(pts, { x, y })
					end
					g:polygon(pts, { fill = ui.color(s.color, 0.16) })
					table.insert(pts, pts[1])
					g:polyline(pts, { color = s.color, width = 2 })
				end
			end,
		}),
	})
end

local function funnel_chart(p, series)
	local s = series[1]
	local labels = p.labels or {}
	local top = max_of(s.values) or 1
	local first = s.values[1] or 1
	local row = 42
	return ui.canvas({
		height = p.height or (#s.values * row + 4),
		draw = function(g, w)
			for i, v in ipairs(s.values) do
				local y = (i - 1) * row
				g:text(0, y, labels[i] or ("#" .. i), { size = "sm" })
				local share = first > 0 and math.floor(v / first * 100 + 0.5) or 0
				g:text(w, y, with_unit(v, p.unit) .. (i > 1 and ("  " .. share .. "%") or ""), { size = "sm", color = "muted", align = "end" })
				local bw = math.max(4, w * v / top)
				g:rect((w - bw) / 2, y + 20, bw, 14, { fill = ui.color(s.color == "chart1" and "accent" or s.color, 1 - 0.55 * (i - 1) / math.max(#s.values, 2)), radius = 3 })
			end
		end,
	})
end

local function heatmap_chart(p, series)
	local labels = p.labels or {}
	local cols = #labels
	if cols == 0 then
		fail("of type heatmap needs labels = { column names }")
	end
	local top = 0
	local lw = 0
	for _, s in ipairs(series) do
		top = math.max(top, max_of(s.values) or 0)
		lw = math.max(lw, est(s.name))
	end
	lw += 12
	local rowh = 28
	return ui.canvas({
		height = p.height or (22 + #series * rowh),
		draw = function(g, w)
			local cw = (w - lw) / cols
			local skip = math.max(1, math.ceil(est("00") * 1.4 / cw))
			for c = 1, cols do
				if (c - 1) % skip == 0 then
					g:text(lw + (c - 0.5) * cw, 0, labels[c], { size = "xs", color = "muted", align = "center" })
				end
			end
			for r, s in ipairs(series) do
				local y = 20 + (r - 1) * rowh
				g:text(0, y + rowh / 2 - 1, s.name, { size = "sm", color = "muted", baseline = "center" })
				for c = 1, cols do
					local v = s.values[c]
					if type(v) == "number" then
						local alpha = top > 0 and (0.07 + 0.8 * v / top) or 0.07
						g:rect(lw + (c - 1) * cw + 1, y + 1, cw - 2, rowh - 2, { fill = ui.color("accent", alpha), radius = 3 })
						if cw >= 30 then
							g:text(lw + (c - 0.5) * cw, y + rowh / 2 - 1, ui.fmt(v), { size = "xs", align = "center", baseline = "center", color = alpha > 0.55 and "on-accent" or "text" })
						end
					end
				end
			end
		end,
	})
end

local function histogram_chart(p, series)
	local lo, hi, count = math.huge, -math.huge, 0
	for _, s in ipairs(series) do
		lo = math.min(lo, min_of(s.values) or lo)
		hi = math.max(hi, max_of(s.values) or hi)
		count = math.max(count, #s.values)
	end
	local edges = {}
	local labels = p.labels or {}
	if #labels >= 2 then
		edges = labels
	else
		local bins = math.floor(ui.clamp(math.ceil(math.log(count, 2)) + 1, 5, 16))
		local width = (hi - lo) / bins
		if width <= 0 then
			width = 1
		end
		for i = 0, bins do
			table.insert(edges, lo + i * width)
		end
	end
	local bins = #edges - 1
	local names, nodes = {}, {}
	for i = 1, bins do
		names[i] = ui.fmt(edges[i], { decimals = 0 }) .. "–" .. ui.fmt(edges[i + 1], { decimals = 0 })
	end
	for _, s in ipairs(series) do
		local counts = {}
		for i = 1, bins do
			counts[i] = 0
		end
		for _, v in ipairs(s.values) do
			for i = 1, bins do
				if v >= edges[i] and (v < edges[i + 1] or (i == bins and v <= edges[i + 1])) then
					counts[i] += 1
					break
				end
			end
		end
		table.insert(nodes, { name = s.name, values = counts, color = s.color })
	end
	return ui.column({
		gap = 4,
		ui.chart({ type = "bar", x = names, series = nodes, height = p.height, legend = #nodes > 1 }),
		ui.text({ "Count per " .. (p.unit or "value") .. " range", size = "xs", color = "faint" }),
	})
end

local function quantile(sorted, q)
	local pos = (#sorted - 1) * q + 1
	local lo = math.floor(pos)
	local hi = math.ceil(pos)
	return sorted[lo] + (sorted[hi] - sorted[lo]) * (pos - lo)
end

local function box_chart(p, series)
	local stats = {}
	local lo, hi = math.huge, -math.huge
	local lw = 0
	for _, s in ipairs(series) do
		local v = table.clone(s.values)
		table.sort(v)
		local q1, med, q3 = quantile(v, 0.25), quantile(v, 0.5), quantile(v, 0.75)
		local fence = (q3 - q1) * 1.5
		local wlo, whi = q1, q3
		local outliers = {}
		for _, x in ipairs(v) do
			if x < q1 - fence or x > q3 + fence then
				table.insert(outliers, x)
			else
				wlo = math.min(wlo, x)
				whi = math.max(whi, x)
			end
		end
		lo = math.min(lo, v[1])
		hi = math.max(hi, v[#v])
		lw = math.max(lw, est(s.name))
		table.insert(stats, { s = s, q1 = q1, med = med, q3 = q3, wlo = wlo, whi = whi, out = outliers })
	end
	lw = math.min(lw + 14, 200)
	if hi == lo then
		hi = lo + 1
	end
	local ticks = ui.ticks(lo, hi, 5)
	local rowh = 46
	return ui.canvas({
		height = p.height or (#stats * rowh + 24),
		draw = function(g, w, hh)
			local x0, x1 = lw, w - 16
			local scale = ui.scale(math.min(lo, ticks[1] or lo), math.max(hi, ticks[#ticks] or hi), x0, x1)
			for _, t in ipairs(ticks) do
				local x = scale(t)
				g:line(x, 0, x, #stats * rowh, { color = ui.color("text", 0.08) })
				g:text(x, #stats * rowh + 6, with_unit(t, p.unit), { size = "xs", color = "faint", align = "center" })
			end
			for i, st in ipairs(stats) do
				local y = (i - 1) * rowh + rowh / 2
				g:text(0, y, fit(st.s.name, lw - 10), { size = "sm", color = "muted", baseline = "center" })
				g:line(scale(st.wlo), y, scale(st.q1), y, { color = st.s.color, width = 1.5 })
				g:line(scale(st.q3), y, scale(st.whi), y, { color = st.s.color, width = 1.5 })
				g:rect(scale(st.q1), y - 11, math.max(2, scale(st.q3) - scale(st.q1)), 22, { fill = ui.color(st.s.color, 0.3), radius = 3 })
				g:line(scale(st.med), y - 11, scale(st.med), y + 11, { color = st.s.color, width = 2.5 })
				for _, o in ipairs(st.out) do
					g:circle(scale(o), y, 2.5, { color = st.s.color })
				end
			end
		end,
	})
end

local CHART_TYPES = { "bar", "line", "area", "scatter", "pie", "radar", "funnel", "heatmap", "histogram", "box" }

define("Chart", { "type", "labels", "series", "unit", "stacked", "height", "max" }, function(p)
	local kind = p.type or p[1] or "bar"
	if not table.find(CHART_TYPES, kind) then
		fail("has unknown type '" .. tostring(kind) .. "': use " .. table.concat(CHART_TYPES, ", "))
	end
	local series = series_of(p.series)
	if kind == "bar" or kind == "line" or kind == "area" or kind == "scatter" then
		return plain_chart(p, kind, series)
	elseif kind == "pie" then
		return pie_chart(p, series)
	elseif kind == "radar" then
		return radar_chart(p, series)
	elseif kind == "funnel" then
		return funnel_chart(p, series)
	elseif kind == "heatmap" then
		return heatmap_chart(p, series)
	elseif kind == "histogram" then
		return histogram_chart(p, series)
	end
	return box_chart(p, series)
end)

-- Sparkline, SegmentedBar -------------------------------------------------------

define("Sparkline", { "values", "tone", "width", "height" }, function(p)
	local values = p.values or children(p)
	if #values < 2 then
		fail("needs values = { at least two numbers }")
	end
	local color = tone(p.tone) or "accent"
	local lo, hi = min_of(values), max_of(values)
	return ui.canvas({
		height = p.height or 22,
		width = p.width or 96,
		draw = function(g, w, hh)
			local pad = 3
			local x = ui.scale(1, #values, pad, w - pad)
			local y = ui.scale(lo, hi == lo and lo + 1 or hi, hh - pad, pad)
			local pts = {}
			for i, v in ipairs(values) do
				table.insert(pts, { x(i), hi == lo and hh / 2 or y(v) })
			end
			g:polyline(pts, { color = color, width = 1.5 })
			local last = pts[#pts]
			g:circle(last[1], last[2], 2.5, { fill = color })
		end,
	})
end)

define("SegmentedBar", { "parts", "unit" }, function(p)
	local parts = p.parts or children(p)
	if #parts == 0 then
		fail("needs parts = { { label, value, tone? }, ... }")
	end
	local total = 0
	for i, part in ipairs(parts) do
		part.value = number(part.value, "part " .. i .. " value")
		part.color = tone(part.tone) or ui.series(i)
		total += math.max(part.value, 0)
	end
	if total <= 0 then
		fail("needs values above zero")
	end
	local rows = ui.map(parts, function(part)
		return ui.row({
			gap = 8,
			ui.text({ "●", size = "xs", color = part.color, width = 14 }),
			ui.text({ part.label or "", grow = true }),
			ui.text({
				with_unit(part.value, p.unit) .. "  " .. math.floor(math.max(part.value, 0) / total * 100 + 0.5) .. "%",
				size = "sm",
				color = "muted",
			}),
		})
	end)
	return ui.column({
		gap = 10,
		ui.canvas({
			height = 14,
			draw = function(g, w)
				local x = 0
				for _, part in ipairs(parts) do
					local pw = math.max(part.value, 0) / total * w
					if pw > 0 then
						g:rect(x, 0, math.max(pw - 2, 1), 14, { fill = part.color, radius = 3 })
					end
					x += pw
				end
			end,
		}),
		ui.column({ gap = 6, rows }),
	})
end)

-- Waterfall: spans on a shared time axis ------------------------------------------

define("Waterfall", { "spans", "unit" }, function(p)
	local spans = p.spans or children(p)
	if #spans == 0 or #spans > 300 then
		fail("needs spans = { { name, start, duration, depth?, tone?, detail? }, ... } (1 to 300)")
	end
	local total, lw = 0, 0
	for i, s in ipairs(spans) do
		s.start = number(s.start or 0, "span " .. i .. " start")
		s.duration = number(s.duration, "span " .. i .. " duration")
		s.depth = s.depth or 0
		s.color = tone(s.tone) or "accent"
		total = math.max(total, s.start + s.duration)
		lw = math.max(lw, est(s.name) + s.depth * 12)
	end
	if total <= 0 then
		total = 1
	end
	local rowh = 24
	local notes = {}
	for _, s in ipairs(spans) do
		if s.detail then
			table.insert(notes, ui.row({
				gap = 8,
				ui.text({ "●", size = "xs", color = s.color, width = 14 }),
				ui.text({ s.name .. ": " .. s.detail, size = "sm", color = "muted", grow = true }),
			}))
		end
	end
	return build(
		ui.column,
		{ gap = 10 },
		ui.canvas({
			height = 22 + #spans * rowh,
			draw = function(g, w)
				local left = math.min(lw + 12, w * 0.45)
				local right = 64
				local x = ui.scale(0, total, left, w - right)
				for _, t in ipairs(ui.ticks(0, total, 4)) do
					if t <= total then
						g:line(x(t), 16, x(t), 22 + #spans * rowh, { color = ui.color("text", 0.08) })
						g:text(x(t), 0, with_unit(t, p.unit), { size = "xs", color = "faint", align = "center" })
					end
				end
				for i, s in ipairs(spans) do
					local y = 22 + (i - 1) * rowh
					g:text(s.depth * 12, y + rowh / 2, fit(s.name, left - 12 - s.depth * 12), { size = "sm", baseline = "center", color = s.depth == 0 and "text" or "muted" })
					local bx = x(s.start)
					g:rect(bx, y + 6, math.max(2, x(s.start + s.duration) - bx), rowh - 12, { fill = s.color, radius = 2 })
					g:text(math.max(bx, x(s.start + s.duration)) + 5, y + rowh / 2, with_unit(s.duration, p.unit), { size = "xs", color = "muted", baseline = "center" })
				end
			end,
		}),
		#notes > 0 and ui.column({ gap = 4, notes }) or nil
	)
end)

-- Treemap: sizes by path, one rectangle each -----------------------------------------

local function squarify(nodes, x, y, w, h, place)
	local total = 0
	for _, n in ipairs(nodes) do
		total += n.value
	end
	if total <= 0 or w <= 0 or h <= 0 then
		return
	end
	local scale = (w * h) / total
	local i = 1
	while i <= #nodes do
		local short = math.min(w, h)
		local row, sum, mx, mn, best = {}, 0, 0, math.huge, math.huge
		local j = i
		while j <= #nodes do
			local a = nodes[j].value * scale
			local s2 = sum + a
			local nmx, nmn = math.max(mx, a), math.min(mn, a)
			local worst = math.max(short * short * nmx / (s2 * s2), s2 * s2 / (short * short * nmn))
			if #row > 0 and worst > best then
				break
			end
			best, sum, mx, mn = worst, s2, nmx, nmn
			table.insert(row, nodes[j])
			j += 1
		end
		local thick = sum / short
		if w >= h then
			local cy = y
			for _, n in ipairs(row) do
				local ch = n.value * scale / thick
				place(n, x, cy, thick, ch)
				cy += ch
			end
			x += thick
			w -= thick
		else
			local cx = x
			for _, n in ipairs(row) do
				local cw = n.value * scale / thick
				place(n, cx, y, cw, thick)
				cx += cw
			end
			y += thick
			h -= thick
		end
		i = j
	end
end

define("Treemap", { "items", "unit", "height" }, function(p)
	local items = p.items or children(p)
	if #items == 0 then
		fail("needs items = { { path = \"src/a.ts\", size = 1234 }, ... }")
	end
	local root = { name = "", kids = {}, map = {}, value = 0 }
	for i, item in ipairs(items) do
		local size = number(item.size or item.bytes or item.value, "item " .. i .. " size")
		local path = item.path or item.name
		if type(path) ~= "string" or path == "" then
			fail("item " .. i .. " needs path = \"...\"")
		end
		local at = root
		at.value += size
		for part in string.gmatch(path, "[^/]+") do
			local nxt = at.map[part]
			if nxt == nil then
				nxt = { name = part, kids = {}, map = {}, value = 0 }
				at.map[part] = nxt
				table.insert(at.kids, nxt)
			end
			nxt.value += size
			at = nxt
		end
	end
	-- A single top directory holding everything is the root itself.
	while #root.kids == 1 and #root.kids[1].kids > 0 do
		root = root.kids[1]
	end
	local function sort(node)
		table.sort(node.kids, function(a, b)
			return a.value > b.value
		end)
		for _, k in ipairs(node.kids) do
			sort(k)
		end
	end
	sort(root)
	for i, k in ipairs(root.kids) do
		k.color = ui.series(i)
	end
	local keys = ui.map(root.kids, function(k)
		return { name = k.name .. "  " .. with_unit(k.value, p.unit), color = k.color }
	end)
	return ui.column({
		gap = 8,
		#keys > 1 and legend(keys) or nil,
		ui.canvas({
			height = ui.clamp(p.height or 300, 160, 420),
			draw = function(g, w, hh)
				local function draw_node(n, x, y, cw, ch, color)
					color = n.color or color
					if #n.kids > 0 then
						squarify(n.kids, x, y, cw, ch, function(k, kx, ky, kw, kh)
							draw_node(k, kx, ky, kw, kh, color)
						end)
						return
					end
					if cw < 2 or ch < 2 then
						return
					end
					g:rect(x + 1, y + 1, cw - 2, ch - 2, { fill = ui.color(color or "accent", 0.35), radius = 2 })
					if cw > 46 and ch > 18 then
						g:text(x + 5, y + 4, fit(n.name, cw - 10, "xs"), { size = "xs" })
						if ch > 32 then
							g:text(x + 5, y + 18, with_unit(n.value, p.unit), { size = "xs", color = "muted" })
						end
					end
				end
				if #root.kids == 0 then
					draw_node(root, 0, 0, w, hh, "accent")
				else
					squarify(root.kids, 0, 0, w, hh, function(k, kx, ky, kw, kh)
						draw_node(k, kx, ky, kw, kh, k.color)
					end)
				end
			end,
		}),
	})
end)

-- Tree: a file or outline tree -----------------------------------------------------------

define("Tree", { "items" }, function(p)
	local items = p.items or children(p)
	if #items == 0 then
		fail("needs items = { \"path/to/file\", { label, children?, note? }, ... }")
	end
	local root = { kids = {}, map = {} }
	local function add(parent, label, note)
		local node = { label = label, note = note, kids = {}, map = {} }
		table.insert(parent.kids, node)
		return node
	end
	local function convert(parent, item)
		if type(item) == "table" then
			local node = add(parent, tostring(item.label or item.name or "?"), item.note)
			for _, c in ipairs(item.children or {}) do
				convert(node, c)
			end
		else
			local at = parent
			for part in string.gmatch(tostring(item), "[^/]+") do
				local nxt = at.map[part]
				if nxt == nil then
					nxt = add(at, part)
					at.map[part] = nxt
				end
				at = nxt
			end
		end
	end
	for _, item in ipairs(items) do
		convert(root, item)
	end
	local rows = {}
	local function walk(node, prefix)
		for i, k in ipairs(node.kids) do
			local last = i == #node.kids
			table.insert(rows, ui.row({
				gap = 0,
				ui.text({ prefix .. (last and "└─ " or "├─ "), mono = true, color = "faint" }),
				ui.text({ k.label .. (#k.kids > 0 and "/" or ""), mono = true, weight = #k.kids > 0 and 600 or 400 }),
				k.note and ui.text({ "   " .. k.note, size = "sm", color = "accent" }) or nil,
			}))
			walk(k, prefix .. (last and "   " or "│  "))
		end
	end
	walk(root, "")
	return ui.column({ gap = 2, rows })
end)

-- Carousel: slides, one at a time ----------------------------------------------------------

local slide_at = {}
define("Carousel", { "key" }, function(p)
	local slides = children(p)
	if #slides == 0 then
		fail("needs its slides as children: ui.kit.Carousel { ui.kit.Card { ... }, ... }")
	end
	local id = p.key or ("carousel" .. #slides)
	local at = ui.clamp(slide_at[id] or 1, 1, #slides)
	local function go(to)
		return function()
			slide_at[id] = ui.clamp(to, 1, #slides)
		end
	end
	return ui.column({
		gap = 12,
		slides[at],
		ui.row({
			gap = 14,
			align = "center",
			ui.row({ hover = true, on_click = go(at - 1), ui.text({ "‹", size = "lg", color = at > 1 and "text" or "faint", weight = 600 }) }),
			ui.text({ at .. " / " .. #slides, size = "sm", color = "muted" }),
			ui.row({ hover = true, on_click = go(at + 1), ui.text({ "›", size = "lg", color = at < #slides and "text" or "faint", weight = 600 }) }),
		}),
	})
end)

-- Icon: a glyph for a lucide name ----------------------------------------------------------------

local GLYPHS = {
	Check = "✓", CheckCircle = "✓", X = "✕", XCircle = "✕", Plus = "+", Minus = "−",
	AlertTriangle = "⚠", AlertCircle = "!", Info = "ℹ", HelpCircle = "?",
	Star = "★", Heart = "♥", Cloud = "☁", Sun = "☀", Moon = "☾", Zap = "⚡",
	Mail = "✉", Phone = "☎", Home = "⌂", Settings = "⚙", Edit = "✎", Pencil = "✎",
	ArrowRight = "→", ArrowLeft = "←", ArrowUp = "↑", ArrowDown = "↓",
	ChevronRight = "›", ChevronLeft = "‹", ChevronDown = "▾", ChevronUp = "▴",
	Play = "▶", Pause = "‖", Square = "■", Circle = "●", Flag = "⚑",
	Key = "⚷", Scissors = "✂", Command = "⌘", Refresh = "↻", RefreshCw = "↻",
	Clock = "◷", Package = "▣", GitBranch = "⑂",
}

define("Icon", { "name", "tone" }, function(p)
	local name = p.name or p[1]
	local glyph = GLYPHS[name]
	if glyph == nil then
		fail("has no icon '" .. tostring(name) .. "' (it knows " .. table.concat(sorted_keys(GLYPHS), ", ") .. ")")
	end
	return ui.text({ glyph, weight = 700, color = tone(p.tone) or "text" })
end)

-- Scene and Shader: the 3D nodes with a list of objects / the Shadertoy props -------------------

local SHAPES = { "box", "sphere", "cylinder", "surface", "mesh", "model", "environment", "grid", "axes", "line", "path", "point", "label", "sprite" }

local function place(s, obj)
	if obj == nil or obj == false then
		return
	end
	if type(obj) ~= "table" then
		error("ui.kit.Scene objects are tables like { shape = \"box\", at = { 0, 0, 0 } }, not a " .. type(obj), 0)
	end
	if obj.shape == nil then
		for _, o in ipairs(obj) do
			place(s, o)
		end
		return
	end
	local shape = obj.shape
	if not table.find(SHAPES, shape) then
		error("ui.kit.Scene has unknown shape '" .. tostring(shape) .. "': use " .. table.concat(SHAPES, ", "), 0)
	end
	if shape == "path" then
		s:path(obj.points or {}, obj)
	else
		s[shape](s, obj)
	end
end

define("Scene", { "objects", "height", "camera", "orbit", "spin", "animate" }, function(p)
	local objects = p.objects or children(p)
	if type(objects) ~= "function" and type(objects) ~= "table" then
		fail("needs objects = { { shape, at, ... }, ... } or function(t) returning them")
	end
	local live = type(objects) == "function"
	return ui.scene({
		height = p.height or 280,
		camera = p.camera,
		orbit = p.orbit,
		spin = p.spin,
		animate = p.animate == nil and live or p.animate,
		draw = function(s, t)
			place(s, live and objects(t) or objects)
		end,
	})
end)

define("Shader", { "code", "height", "buffers", "inputs", "textures", "blend", "bloom", "orbit", "spin", "camera", "animate" }, function(p)
	if type(p.code) ~= "string" then
		fail("needs code = [[ void mainImage(out vec4 fragColor, in vec2 fragCoord) { ... } ]]")
	end
	local props = {}
	for k, v in pairs(p) do
		props[k] = v
	end
	props.height = props.height or 260
	return ui.shader(props)
end)


-- Map: places on a plot of their coordinates (no tiles: Zend draws no basemap) -------------------

define("Map", { "places", "route", "height" }, function(p)
	local places = p.places or children(p)
	if #places == 0 then
		fail("needs places = { { name, lat, lng, note?, day? }, ... }")
	end
	local latlo, lathi, lnglo, lnghi = math.huge, -math.huge, math.huge, -math.huge
	for i, place in ipairs(places) do
		local name = tostring(place.name or i)
		if type(place.lat) ~= "number" or type(place.lng) ~= "number" then
			fail("place '" .. name .. "' needs lat and lng as numbers (Zend does not look places up)")
		end
		latlo, lathi = math.min(latlo, place.lat), math.max(lathi, place.lat)
		lnglo, lnghi = math.min(lnglo, place.lng), math.max(lnghi, place.lng)
	end
	local kx = math.cos(math.rad((latlo + lathi) / 2))
	local spanx = math.max((lnghi - lnglo) * kx, 0.01)
	local spany = math.max(lathi - latlo, 0.01)
	local rows = {}
	for i, place in ipairs(places) do
		table.insert(rows, ui.row({
			gap = 10,
			align = "start",
			ui.text({ tostring(i), color = "accent", weight = 600, width = 20 }),
			build(
				ui.column,
				{ gap = 2, grow = true },
				ui.text({ tostring(place.name or i), weight = 500 }),
				cell(place.note, { size = "sm", color = "muted" })
			),
		}))
	end
	return ui.column({
		gap = 12,
		ui.canvas({
			height = p.height or 300,
			draw = function(g, w, hh)
				local pad = 36
				local scale = math.min((w - 2 * pad) / spanx, (hh - 2 * pad) / spany)
				local ox, oy = (w - spanx * scale) / 2, (hh - spany * scale) / 2
				local function at(lat, lng)
					return ox + (lng - lnglo) * kx * scale, oy + (lathi - lat) * scale
				end
				for _, lat in ipairs(ui.ticks(latlo, lathi, 4)) do
					local _, y = at(lat, lnglo)
					if y >= 0 and y <= hh then
						g:line(0, y, w, y, { color = ui.color("text", 0.07) })
						g:text(0, y - 12, ui.fmt(lat, { decimals = 2 }) .. "°", { size = "xs", color = "faint" })
					end
				end
				for _, lng in ipairs(ui.ticks(lnglo, lnghi, 4)) do
					local x = at(latlo, lng)
					if x >= 0 and x <= w then
						g:line(x, 0, x, hh, { color = ui.color("text", 0.07) })
						g:text(x + 3, hh - 14, ui.fmt(lng, { decimals = 2 }) .. "°", { size = "xs", color = "faint" })
					end
				end
				local pts = {}
				for i, place in ipairs(places) do
					local x, y = at(place.lat, place.lng)
					pts[i] = { x, y }
				end
				if p.route then
					local run = {}
					for i, place in ipairs(places) do
						if #run > 0 and place.day ~= places[i - 1].day then
							g:polyline(run, { color = "accent", width = 1.5, dash = 5 })
							run = {}
						end
						table.insert(run, pts[i])
					end
					g:polyline(run, { color = "accent", width = 1.5, dash = 5 })
				end
				for i, place in ipairs(places) do
					local x, y = pts[i][1], pts[i][2]
					local color = place.day and ui.series(place.day) or "accent"
					g:circle(x, y, 5, { fill = color })
					local label = i .. "  " .. tostring(place.name or i)
					if x + 10 + est(label) > w then
						g:text(x - 10, y - 7, label, { size = "sm", align = "end" })
					else
						g:text(x + 10, y - 7, label, { size = "sm" })
					end
				end
			end,
		}),
		ui.column({ gap = 8, rows }),
	})
end)

-- Diagram: Mermaid flowcharts and sequence diagrams ---------------------------------------------------

local MERMAID_SHAPES = {
	{ "([", "])", "round" }, { "((", "))", "circle" }, { "[[", "]]", "rect" }, { "[(", ")]", "rect" },
	{ "{{", "}}", "diamond" }, { "[", "]", "rect" }, { "(", ")", "round" }, { "{", "}", "diamond" }, { ">", "]", "rect" },
}

local function unquote(label)
	label = string.gsub(label, "<br%s*/?>", " ")
	label = string.gsub(label, '^%s*"(.*)"%s*$', "%1")
	return (string.gsub(label, "^%s+", ""):gsub("%s+$", ""))
end

local function parse_node(s, pos)
	local id = string.match(s, "^[%w_]+", pos)
	if id == nil then
		return nil
	end
	local np = pos + #id
	for _, shape in ipairs(MERMAID_SHAPES) do
		local open, close = shape[1], shape[2]
		if string.sub(s, np, np + #open - 1) == open then
			local at = string.find(s, close, np + #open, true)
			if at == nil then
				fail("has an unclosed '" .. open .. "' in: " .. s)
			end
			return id, unquote(string.sub(s, np + #open, at - 1)), shape[3], at + #close
		end
	end
	return id, nil, nil, np
end

local ARROWS = {
	{ "^%-%-%s+(.-)%s+%-%->", "solid", true }, { "^%-%.%s+(.-)%s+%.%->", "dotted", true },
	{ "^==%s+(.-)%s+==>", "thick", true }, { "^%-%-+>", "solid", true }, { "^%-%-%-+", "solid", false },
	{ "^%-%.+%->", "dotted", true }, { "^==+>", "thick", true }, { "^===+", "thick", false }, { "^%-%.+%-", "dotted", false },
}

local function parse_arrow(s, pos)
	for _, a in ipairs(ARROWS) do
		local from, to, text = string.find(s, a[1], pos)
		if from then
			local label = text and unquote(text) or nil
			local np = to + 1
			local bar = string.match(s, "^%s*|([^|]*)|", np)
			if bar then
				label = unquote(bar)
				np = select(2, string.find(s, "^%s*|[^|]*|", np)) + 1
			end
			return { style = a[2], arrow = a[3], label = label }, np
		end
	end
	return nil
end

local function parse_flowchart(lines, direction)
	local order, nodes, edges = {}, {}, {}
	local function node(id, label, shape)
		local n = nodes[id]
		if n == nil then
			n = { id = id, label = id, shape = "rect" }
			nodes[id] = n
			table.insert(order, n)
		end
		n.label = label or n.label
		n.shape = shape or n.shape
		return n
	end
	for _, line in ipairs(lines) do
		local word = string.match(line, "^(%a+)")
		if word == "subgraph" or word == "end" then
			fail("does not support subgraph: draw one flat flowchart")
		elseif word == "classDef" or word == "class" or word == "style" or word == "linkStyle" or word == "click" then
			continue
		end
		local pos, prev, pending = 1, nil, nil
		while true do
			pos = string.find(line, "%S", pos)
			if pos == nil then
				break
			end
			local id, label, shape, np = parse_node(line, pos)
			if id == nil then
				fail("cannot read '" .. string.sub(line, pos) .. "' in: " .. line)
			end
			local n = node(id, label, shape)
			if prev and pending then
				table.insert(edges, { from = prev, to = n, style = pending.style, arrow = pending.arrow, label = pending.label })
			end
			prev, pending = n, nil
			pos = string.find(line, "%S", np)
			if pos == nil then
				break
			end
			if string.sub(line, pos, pos) == "&" then
				fail("does not support '&': write one edge per line")
			end
			pending, np = parse_arrow(line, pos)
			if pending == nil then
				fail("cannot read '" .. string.sub(line, pos) .. "' in: " .. line)
			end
			pos = np
		end
	end
	if #order == 0 then
		fail("has no nodes")
	end
	return { kind = "flow", direction = direction, nodes = order, edges = edges }
end

-- Ranks along the edges (longest path, back edges of cycles left out) and
-- each rank's order (barycentre of the rank before), once per diagram.
local function rank_flowchart(d)
	local state, back = {}, {}
	local out_edges = {}
	for _, e in ipairs(d.edges) do
		out_edges[e.from.id] = out_edges[e.from.id] or {}
		table.insert(out_edges[e.from.id], e)
	end
	local function dfs(n)
		state[n.id] = 1
		for _, e in ipairs(out_edges[n.id] or {}) do
			if state[e.to.id] == 1 then
				back[e] = true
			elseif not state[e.to.id] then
				dfs(e.to)
			end
		end
		state[n.id] = 2
	end
	for _, n in ipairs(d.nodes) do
		if not state[n.id] then
			dfs(n)
		end
	end
	for _, n in ipairs(d.nodes) do
		n.rank = 0
	end
	for _ = 1, #d.nodes do
		for _, e in ipairs(d.edges) do
			if not back[e] and e.to.rank < e.from.rank + 1 then
				e.to.rank = e.from.rank + 1
			end
		end
	end
	local ranks = {}
	for _, n in ipairs(d.nodes) do
		ranks[n.rank + 1] = ranks[n.rank + 1] or {}
		table.insert(ranks[n.rank + 1], n)
	end
	for r = 1, #ranks do
		ranks[r] = ranks[r] or {}
	end
	for r = 2, #ranks do
		local index = {}
		for i, n in ipairs(ranks[r - 1]) do
			index[n.id] = i
		end
		for i, n in ipairs(ranks[r]) do
			local sum, count = 0, 0
			for _, e in ipairs(d.edges) do
				if e.to == n and index[e.from.id] then
					sum += index[e.from.id]
					count += 1
				end
			end
			n.bary = count > 0 and sum / count or i
			n.seq = i
		end
		table.sort(ranks[r], function(a, b)
			if a.bary ~= b.bary then
				return a.bary < b.bary
			end
			return a.seq < b.seq
		end)
	end
	d.ranks = ranks
end

-- A label in lines of at most 'max' columns, broken between words (a
-- longer word is broken between its characters).
local function wrap_words(label, max)
	local lines, line = {}, ""
	local words = {}
	for word in string.gmatch(label, "%S+") do
		while cols(word) > max do
			local part = head(word, max)
			if part == "" then
				break
			end
			table.insert(words, part)
			word = string.sub(word, #part + 1)
		end
		table.insert(words, word)
	end
	for _, word in ipairs(words) do
		if line == "" then
			line = word
		elseif cols(line) + 1 + cols(word) <= max then
			line ..= " " .. word
		else
			table.insert(lines, line)
			line = word
		end
	end
	if line ~= "" or #lines == 0 then
		table.insert(lines, line)
	end
	return lines
end

local LINE = 16

-- One layout of a ranked flowchart: labels wrapped at 'wrap' characters,
-- across the page when 'flip' turns LR into TD (RL into BT). Boxes are by
-- node id, centres in layout pixels from the top left; 'gap' is the middle
-- of the gap before each rank, where an edge into it turns.
local function layout_flowchart(d, wrap, flip, tight)
	local horizontal = d.direction == "LR" or d.direction == "RL"
	if flip then
		horizontal = not horizontal
	end
	local reverse = d.direction == "RL" or d.direction == "BT"
	local box = {}
	for _, n in ipairs(d.nodes) do
		local lines = wrap_words(n.label, wrap)
		local text = 0
		for _, line in ipairs(lines) do
			text = math.max(text, est(line))
		end
		local w, h = math.max(64, text + 26), #lines * LINE + 18
		if n.shape == "diamond" then
			-- Each line within the diamond's width at its own height: the
			-- narrowest diamond that holds them, about as wide as tall.
			local best = math.huge
			for k = 0, 12 do
				local hy = #lines * LINE / 2 + 12 + k * 6
				local hx = 32
				for i, line in ipairs(lines) do
					local dy = math.abs(i - (#lines + 1) / 2) * LINE + 7
					hx = math.max(hx, (est(line) / 2 + 6) / (1 - dy / hy))
				end
				if hx + hy < best then
					best, w, h = hx + hy, math.ceil(hx * 2), math.ceil(hy * 2)
				end
			end
		elseif n.shape == "circle" then
			w = math.max(text + 20, h)
			h = w
		end
		box[n.id] = { lines = lines, w = w, h = h }
	end
	-- The widest label of an edge into each node from the rank before.
	local into = {}
	for _, e in ipairs(d.edges) do
		if e.label and e.to.rank == e.from.rank + 1 then
			into[e.to.id] = math.max(into[e.to.id] or 0, est(e.label, "xs"))
		end
	end
	-- Across: the nodes of a rank side by side. Top to bottom, the label
	-- beside an edge's last run clears the next node's edge.
	local across, sizes = 0, {}
	local space = horizontal and 20 or 28
	for r, list in ipairs(d.ranks) do
		local size, c, prev = 0, 0, nil
		for _, n in ipairs(list) do
			local b = box[n.id]
			size = math.max(size, horizontal and b.w or b.h)
			local cross = horizontal and b.h or b.w
			if prev and not horizontal and into[prev.id] then
				c = math.max(c, prev.c + into[prev.id] + 16 - cross / 2)
			end
			b.c = c + cross / 2
			c += cross + space
			prev = b
			prev.id = n.id
		end
		local far = c - space
		if prev and not horizontal and into[prev.id] then
			far = math.max(far, prev.c + 6 + into[prev.id] + 2)
		end
		across = math.max(across, far)
		sizes[r] = size
	end
	-- Between ranks: room for the arrow, and left to right for the label of
	-- an edge into the next rank (after its turn, when it turns).
	-- (A gap with such a label turns early, near its start.)
	local gaps, early = {}, {}
	for r = 1, #d.ranks - 1 do
		gaps[r] = tight and 24 or (horizontal and 48 or 40)
	end
	for _, e in ipairs(d.edges) do
		if e.label and e.to.rank == e.from.rank + 1 and horizontal then
			local r = e.from.rank + 1
			local lw = est(e.label, "xs")
			local straight = math.abs(box[e.from.id].c - box[e.to.id].c) < 1
			gaps[r] = math.max(gaps[r], lw + (straight and 24 or 30))
			early[r] = early[r] or not straight
		end
	end
	local along = 0
	local starts = {}
	for r, list in ipairs(d.ranks) do
		starts[r] = along
		for _, n in ipairs(list) do
			box[n.id].a = along + sizes[r] / 2
		end
		along += sizes[r] + (gaps[r] or 0)
	end
	-- An edge that skips ranks runs along its source's row; where a node
	-- between stands in that row, it goes round by a lane past the rest.
	local lanes, lane = {}, across + 16
	for _, e in ipairs(d.edges) do
		if e.to.rank > e.from.rank + 1 then
			local c = box[e.from.id].c
			local blocked = false
			for r = e.from.rank + 2, e.to.rank do
				for _, n in ipairs(d.ranks[r]) do
					local b = box[n.id]
					local half = (horizontal and b.h or b.w) / 2 + 6
					if math.abs(b.c - c) < half then
						blocked = true
					end
				end
			end
			if blocked then
				lanes[e] = lane
				across = lane + 6
				lane += 12
			end
		end
	end
	local gap = {}
	for r = 2, #d.ranks do
		gap[r] = early[r - 1] and (starts[r] - gaps[r - 1] + 12) or (starts[r] - gaps[r - 1] / 2)
	end
	local function flip_along(a)
		return reverse and (along - a) or a
	end
	for _, b in pairs(box) do
		local a = flip_along(b.a)
		if horizontal then
			b.x, b.y = a, b.c
		else
			b.x, b.y = b.c, a
		end
	end
	for r, a in pairs(gap) do
		gap[r] = flip_along(a)
	end
	return {
		box = box,
		gap = gap,
		lanes = lanes,
		horizontal = horizontal,
		width = horizontal and along or across,
		height = horizontal and across or along,
	}
end

-- The first layout that fits 'maxw' by 'maxh': the diagram's own direction,
-- then narrower labels, then (a left-to-right diagram) top to bottom. When
-- none fits, the one that overflows least.
local function fit_flowchart(d, maxw, maxh)
	local turns = d.direction == "LR" or d.direction == "RL"
	local tries = { { 24, false }, { 14, false } }
	if turns then
		table.insert(tries, { 24, true })
		table.insert(tries, { 14, true })
	end
	-- (Last, shorter arrows.)
	table.insert(tries, { 14, false, true })
	if turns then
		table.insert(tries, { 14, true, true })
	end
	local best, over = nil, math.huge
	for _, t in ipairs(tries) do
		local l = layout_flowchart(d, t[1], t[2], t[3])
		if l.width <= maxw and l.height <= maxh then
			return l
		end
		local o = math.max(l.width / math.max(maxw, 1), l.height / math.max(maxh, 1))
		if o < over then
			best, over = l, o
		end
	end
	return best
end

local function arrow_head(g, x, y, dx, dy, color)
	local len = math.sqrt(dx * dx + dy * dy)
	if len < 1e-6 then
		return
	end
	dx, dy = dx / len, dy / len
	g:polygon({ { x, y }, { x - dx * 8 - dy * 4, y - dy * 8 + dx * 4 }, { x - dx * 8 + dy * 4, y - dy * 8 - dx * 4 } }, { fill = color })
end

local function draw_flowchart(g, w, hh, d, l, selected, boxes)
	-- A layout that still overflows shrinks whole: boxes with their places,
	-- each line cut to its box, so nothing overlaps.
	local sx = math.min(1, math.max(w - 8, 10) / math.max(l.width, 1))
	local sy = math.min(1, math.max(hh - 8, 10) / math.max(l.height, 1))
	local ox = (w - l.width * sx) / 2
	local function place(n)
		local b = l.box[n.id]
		return ox + b.x * sx, 4 + b.y * sy, b.w * sx, b.h * sy, b
	end
	local function turn(r)
		local a = l.gap[r]
		return a and (l.horizontal and ox + a * sx or 4 + a * sy) or nil
	end
	-- (Labelled edges into each node: one keeps its label past its turn.)
	local labelled = {}
	for _, e in ipairs(d.edges) do
		if e.label then
			labelled[e.to.id] = (labelled[e.to.id] or 0) + 1
		end
	end
	for _, e in ipairs(d.edges) do
		local x1, y1, w1, h1 = place(e.from)
		local x2, y2, w2, h2 = place(e.to)
		local color = ui.color("text", 0.45)
		local width = e.style == "thick" and 3 or 1.5
		local dash = e.style == "dotted" and 4 or nil
		-- (A forward edge turns in the gap before its target, so one that
		-- skips ranks runs along its own row, clear of the nodes between.)
		local forward = e.to.rank > e.from.rank
		local at = forward and turn(e.to.rank + 1) or nil
		-- (A back edge in line with a forward one runs beside it.)
		if not forward then
			if l.horizontal and math.abs(y2 - y1) < 1 then
				y1, y2 = y1 + 8, y2 + 8
			elseif not l.horizontal and math.abs(x2 - x1) < 1 then
				x1, x2 = x1 + 12, x2 + 12
			end
		end
		local pts
		local lane = l.lanes[e]
		local out = lane and turn(e.from.rank + 2)
		if lane and out and at then
			if l.horizontal then
				local s = x2 >= x1 and 1 or -1
				local ly = 4 + lane * sy
				pts = { { x1 + s * w1 / 2, y1 }, { out, y1 }, { out, ly }, { at, ly }, { at, y2 }, { x2 - s * w2 / 2, y2 } }
			else
				local s = y2 >= y1 and 1 or -1
				local lx = ox + lane * sx
				pts = { { x1, y1 + s * h1 / 2 }, { x1, out }, { lx, out }, { lx, at }, { x2, at }, { x2, y2 - s * h2 / 2 } }
			end
		elseif l.horizontal then
			local s = x2 >= x1 and 1 or -1
			local ax, bx = x1 + s * w1 / 2, x2 - s * w2 / 2
			local tx = at or (ax + bx) / 2
			pts = (math.abs(y2 - y1) < 1) and { { ax, y1 }, { bx, y2 } } or { { ax, y1 }, { tx, y1 }, { tx, y2 }, { bx, y2 } }
		else
			local s = y2 >= y1 and 1 or -1
			local ay, by = y1 + s * h1 / 2, y2 - s * h2 / 2
			local ty = at or (ay + by) / 2
			pts = (math.abs(x2 - x1) < 1) and { { x1, ay }, { x2, by } } or { { x1, ay }, { x1, ty }, { x2, ty }, { x2, by } }
		end
		g:polyline(pts, { color = color, width = width, dash = dash })
		local last, before = pts[#pts], pts[#pts - 1]
		if e.arrow then
			arrow_head(g, last[1], last[2], last[1] - before[1], last[2] - before[2], color)
		end
		if e.label then
			-- A forward edge's on its last run, its own past any turn it shares;
			-- another's on its longest. Above a level run, beside an upright
			-- one in a top-down layout (where the gap is the rank's height).
			local pick, long = #pts - 1, -1
			if not forward or lane or labelled[e.to.id] > 1 then
				for i = 1, #pts - 1 do
					local p, q = pts[i], pts[i + 1]
					local len = math.abs(q[1] - p[1]) + math.abs(q[2] - p[2])
					if len > long then
						pick, long = i, len
					end
				end
			end
			local p, q = pts[pick], pts[pick + 1]
			local mx, my = (p[1] + q[1]) / 2, (p[2] + q[2]) / 2
			-- (Always inside the canvas: beside on the side with room, or cut.)
			local lw = est(e.label, "xs")
			if l.horizontal and forward and not lane and #pts == 4 and math.abs(q[2] - p[2]) < 1 then
				-- (After the turn, clear of the upright run it shares.)
				local s = q[1] >= p[1] and 1 or -1
				local room = math.abs(q[1] - p[1]) - 6
				g:text(p[1] + s * 4, my - 8, fit(e.label, room, "xs"), { size = "xs", color = "muted", align = s > 0 and "start" or "end", baseline = "center" })
			elseif math.abs(q[2] - p[2]) >= 1 and not l.horizontal then
				local right, left = w - 2 - (mx + 6), mx - 6 - 2
				if lw <= right or right >= left then
					g:text(mx + 6, my, fit(e.label, right, "xs"), { size = "xs", color = "muted", baseline = "center" })
				else
					g:text(mx - 6, my, fit(e.label, left, "xs"), { size = "xs", color = "muted", align = "end", baseline = "center" })
				end
			else
				local text = fit(e.label, w - 4, "xs")
				local half = est(text, "xs") / 2
				local cx = math.max(2 + half, math.min(w - 2 - half, mx))
				g:text(cx, my - 8, text, { size = "xs", color = "muted", align = "center", baseline = "center" })
			end
		end
	end
	for _, n in ipairs(d.nodes) do
		local x, y, nw, nh, b = place(n)
		local fill = ui.color("text", 0.08)
		local edge = selected == n.id and "accent" or ui.color("text", 0.25)
		if n.shape == "diamond" then
			local pts = { { x, y - nh / 2 }, { x + nw / 2, y }, { x, y + nh / 2 }, { x - nw / 2, y } }
			g:polygon(pts, { fill = fill })
			table.insert(pts, pts[1])
			g:polyline(pts, { color = edge, width = selected == n.id and 2 or 1 })
		elseif n.shape == "circle" then
			g:circle(x, y, math.min(nw, nh) / 2, { fill = fill })
			g:circle(x, y, math.min(nw, nh) / 2, { color = edge, width = selected == n.id and 2 or 1 })
		else
			local r = n.shape == "round" and math.min(nh / 2, 17) or 6
			g:rect(x - nw / 2, y - nh / 2, nw, nh, { fill = fill, radius = r })
			if selected == n.id then
				g:rect(x - nw / 2, y - nh / 2, nw, nh, { color = "accent", width = 2, radius = r })
			end
		end
		local lines = b.lines
		local room = math.max(1, math.floor((nh - 10) / LINE))
		if #lines > room then
			local cut = table.move(lines, 1, room, 1, {})
			cut[room] = cut[room] .. " …"
			lines = cut
		end
		for i, line in ipairs(lines) do
			local dy = (i - (#lines + 1) / 2) * LINE
			-- (A diamond narrows away from its middle.)
			local inner = n.shape == "diamond" and nw * (1 - (math.abs(dy) + 7) / math.max(nh / 2, 1)) - 12 or nw - 12
			local text = (sx < 1 or sy < 1 or n.shape == "diamond") and fit(line, inner) or line
			g:text(x, y + dy, text, { size = "sm", align = "center", baseline = "center" })
		end
		table.insert(boxes, { id = n.id, x0 = x - nw / 2, x1 = x + nw / 2, y0 = y - nh / 2, y1 = y + nh / 2 })
	end
end

local function parse_sequence(lines)
	local order, by, rows = {}, {}, {}
	local function who(id, label)
		local x = by[id]
		if x == nil then
			x = { id = id, label = id }
			by[id] = x
			table.insert(order, x)
		end
		x.label = label or x.label
		return x
	end
	for _, line in ipairs(lines) do
		local word = string.match(line, "^(%a+)")
		if word == "participant" or word == "actor" then
			local id, label = string.match(line, "^%a+%s+([%w_]+)%s+as%s+(.+)$")
			if id then
				who(id, unquote(label))
			else
				who(string.match(line, "^%a+%s+([%w_]+)") or fail("cannot read: " .. line))
			end
		elseif word == "autonumber" or word == "activate" or word == "deactivate" or word == "title" then
			continue
		elseif word == "loop" or word == "alt" or word == "opt" or word == "par" or word == "critical" or word == "break" or word == "rect" or word == "else" or word == "end" then
			fail("does not support '" .. word .. "' blocks in a sequence diagram")
		elseif word == "Note" or word == "note" then
			local a, b, text = string.match(line, "^%a+%s+over%s+([%w_]+)%s*,?%s*([%w_]*)%s*:%s*(.*)$")
			if a then
				table.insert(rows, { note = true, a = who(a), b = b ~= "" and who(b) or who(a), text = unquote(text) })
			else
				local side, id, t = string.match(line, "^%a+%s+(%a+)%s+of%s+([%w_]+)%s*:%s*(.*)$")
				if not id then
					fail("cannot read: " .. line)
				end
				table.insert(rows, { note = true, a = who(id), b = who(id), text = unquote(t) })
			end
		else
			local a, arrow, b, text = string.match(line, "^([%w_]+)%s*(%-%-?[>x%)]>?)%s*[%+%-]?([%w_]+)%s*:%s*(.*)$")
			if not a then
				fail("cannot read '" .. line .. "' (a message looks like A->>B: text)")
			end
			table.insert(rows, { a = who(a), b = who(b), dashed = string.sub(arrow, 1, 2) == "--", text = unquote(text) })
		end
	end
	if #order == 0 then
		fail("has no participants")
	end
	return { kind = "sequence", parts = order, rows = rows }
end

local function draw_sequence(g, w, d)
	local n = #d.parts
	local colw = w / n
	local x = {}
	for i, part in ipairs(d.parts) do
		x[part.id] = (i - 0.5) * colw
	end
	local bottom = 44 + #d.rows * 34
	for _, part in ipairs(d.parts) do
		g:text(x[part.id], 0, fit(part.label, colw - 8), { weight = 600, align = "center" })
		g:line(x[part.id], 22, x[part.id], bottom, { color = ui.color("text", 0.2), dash = 4 })
	end
	local color = ui.color("text", 0.55)
	for i, r in ipairs(d.rows) do
		local y = 44 + (i - 1) * 34 + 14
		local xa, xb = x[r.a.id], x[r.b.id]
		if r.note then
			g:text((xa + xb) / 2, y - 6, fit(r.text, math.max(colw, math.abs(xb - xa) + colw) - 8), { size = "sm", color = "muted", align = "center" })
		elseif r.a == r.b then
			g:polyline({ { xa, y - 6 }, { xa + 26, y - 6 }, { xa + 26, y + 6 }, { xa + 2, y + 6 } }, { color = color, dash = r.dashed and 4 or nil })
			arrow_head(g, xa + 2, y + 6, -1, 0, color)
			g:text(xa + 32, y - 7, r.text, { size = "sm" })
		else
			local s = xb > xa and 1 or -1
			g:line(xa, y, xb - s * 1, y, { color = color, dash = r.dashed and 4 or nil })
			arrow_head(g, xb, y, s, 0, color)
			g:text((xa + xb) / 2, y - 18, fit(r.text, math.abs(xb - xa) - 8), { size = "sm", align = "center" })
		end
	end
end

local diagram_sel = {}

define("Diagram", { "mermaid", "details", "key", "height" }, function(p)
	local source = p.mermaid or p[1]
	if type(source) ~= "string" then
		fail("needs mermaid = \"flowchart LR\\n  A --> B\" (a Mermaid flowchart or sequenceDiagram)")
	end
	local lines = {}
	for raw in string.gmatch(source, "[^\n;]+") do
		local line = string.gsub(string.gsub(raw, "%%%%.*$", ""), "^%s+", "")
		line = string.gsub(line, "%s+$", "")
		if line ~= "" then
			table.insert(lines, line)
		end
	end
	local head = table.remove(lines, 1) or ""
	local kind, dir = string.match(head, "^(%a+)%s*(%a*)")
	local d
	if kind == "flowchart" or kind == "graph" then
		dir = dir ~= "" and string.upper(dir) or "TD"
		if dir == "TB" then
			dir = "TD"
		end
		if not table.find({ "LR", "RL", "TD", "BT" }, dir) then
			fail("has unknown direction '" .. dir .. "': use LR, RL, TD, or BT")
		end
		d = parse_flowchart(lines, dir)
		rank_flowchart(d)
	elseif kind == "sequenceDiagram" then
		if p.details then
			fail("takes details only on a flowchart")
		end
		d = parse_sequence(lines)
	else
		fail("draws flowcharts and sequence diagrams: start with 'flowchart LR' or 'sequenceDiagram'")
	end
	local id = p.key or source
	local height
	-- A flowchart is laid out for the reply column (about 720 px) to size
	-- its canvas, then again for the width it's drawn at.
	local layouts = {}
	if d.kind == "flow" then
		height = p.height or math.min(fit_flowchart(d, 680, math.huge).height + 12, 640)
	else
		height = p.height or (52 + #d.rows * 34)
	end
	local boxes = {}
	local canvas = ui.canvas({
		height = height,
		draw = function(g, w, hh)
			if d.kind == "flow" then
				boxes = {}
				local key = math.floor(w) .. "x" .. math.floor(hh)
				layouts[key] = layouts[key] or fit_flowchart(d, w - 8, hh - 8)
				draw_flowchart(g, w, hh, d, layouts[key], diagram_sel[id], boxes)
			else
				draw_sequence(g, w, d)
			end
		end,
		on_click = (p.details and d.kind == "flow") and function(x, y)
			for _, b in ipairs(boxes) do
				if x >= b.x0 and x <= b.x1 and y >= b.y0 and y <= b.y1 then
					diagram_sel[id] = diagram_sel[id] ~= b.id and b.id or nil
					return
				end
			end
		end or nil,
	})
	if not p.details then
		return canvas
	end
	local chosen = diagram_sel[id]
	local detail = chosen and p.details[chosen]
	local body
	if chosen == nil then
		body = ui.text({ "Click a node for its details.", size = "xs", color = "faint" })
	else
		local label = chosen
		for _, n in ipairs(d.nodes) do
			if n.id == chosen then
				label = n.label
			end
		end
		local content = type(detail) == "table" and (detail.__kind and detail or ui.column({ gap = 6, ui.map(detail, function(c) return cell(c) end) })) or cell(detail or "No details for this node.", { color = "muted" })
		body = ui.column({ gap = 4, ui.text({ label, weight = 600 }), content })
	end
	return ui.column({ gap = 8, canvas, body })
end)

