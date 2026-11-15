--!nocheck
-- `ui.kit`, batch f (plans/live-visuals G10): pictures. Each builds on
-- `ui.image`, which shows one of the visual's own files (show_visual's
-- `assets`) by name. Loaded after kit.luau with the same `ui`; returns its
-- kinds, which Zend joins into the one frozen `ui.kit`. The helpers below
-- are kit.luau's, repeated so the batches stay apart.
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

-- A picture's own props, passed to ui.image as they are.
local function picture(p, extra)
	if type(p.src) ~= "string" or p.src == "" then
		fail("needs src = \"<an asset's name>\" (a file passed in show_visual's assets)")
	end
	local node = { src = p.src, alt = p.alt, height = p.height, fit = p.fit }
	for k, v in pairs(extra or {}) do
		node[k] = v
	end
	return ui.image(node)
end

local function caption(words)
	if words == nil then
		return nil
	end
	return ui.text { tostring(words), size = "sm", color = "muted" }
end

-- One picture, its caption under it.
define("Image", { "src", "alt", "caption", "height", "fit", "key" }, function(p)
	return ui.column { key = p.key, gap = 6, picture(p), caption(p.caption) }
end)

-- Pictures side by side in a grid, each cut to the same height (cover),
-- its caption under it.
define("Gallery", { "items", "columns", "height", "fit", "key" }, function(p)
	local items = p.items
	if type(items) ~= "table" or #items == 0 then
		fail("needs items = { { src = \"a.png\", caption? = \"...\", alt? = \"...\" }, ... }")
	end
	if #items > 24 then
		fail("takes at most 24 items (has " .. #items .. ")")
	end
	local columns = p.columns or math.min(#items, 3)
	if type(columns) ~= "number" or columns < 1 or columns > 6 then
		fail("needs columns between 1 and 6")
	end
	local cells = {}
	for i, item in ipairs(items) do
		if type(item) == "string" then
			item = { src = item }
		end
		if type(item) ~= "table" then
			fail("item " .. i .. " must be { src = \"...\", caption? }")
		end
		local ok, node = pcall(picture, {
			src = item.src,
			alt = item.alt or item.caption,
			height = p.height or 140,
			fit = p.fit or "cover",
		})
		if not ok then
			fail("item " .. i .. " " .. tostring(node))
		end
		table.insert(cells, ui.column { gap = 6, node, caption(item.caption) })
	end
	local grid = { key = p.key, columns = columns, gap = 12 }
	for _, cell in ipairs(cells) do
		table.insert(grid, cell)
	end
	return ui.grid(grid)
end)

-- Before and after: one picture over the other, split where the reader
-- drags. `before` shows on the left, `after` on the right.
define("Compare", { "before", "after", "alt", "split", "height", "fit", "labels", "caption", "key" }, function(p)
	if type(p.before) ~= "string" or type(p.after) ~= "string" then
		fail("needs before = \"<asset>\" and after = \"<asset>\" (files passed in show_visual's assets)")
	end
	local labels = p.labels or { "Before", "After" }
	if type(labels) ~= "table" or #labels ~= 2 then
		fail("needs labels = { \"<left>\", \"<right>\" } (two)")
	end
	return ui.column {
		key = p.key,
		gap = 6,
		picture({ src = p.after, alt = p.alt, height = p.height, fit = p.fit }, { before = p.before, split = p.split }),
		ui.row {
			justify = "between",
			ui.text { "◀ " .. tostring(labels[1]), size = "sm", color = "muted" },
			ui.text { tostring(labels[2]) .. " ▶", size = "sm", color = "muted" },
		},
		caption(p.caption),
	}
end)

-- Regions on a picture, numbered: their titles named as the pointer finds
-- them, and their details listed under it. With `selected` and
-- `on_select` (a ui.state the script keeps), a click picks one and only
-- its details show.
define("Hotspots", { "src", "alt", "spots", "height", "fit", "selected", "on_select", "key" }, function(p)
	local spots = p.spots
	if type(spots) ~= "table" or #spots == 0 then
		fail(
			"needs spots = { { x = 0.4, y = 0.3, title = \"...\", detail? = \"...\", w?, h?, tone? }, ... } "
				.. "(x, y, w, h: shares of the picture, 0 to 1)"
		)
	end
	local marks, legend = {}, {}
	for i, s in ipairs(spots) do
		if type(s) ~= "table" or s.title == nil then
			fail("spot " .. i .. " needs title = \"...\"")
		end
		table.insert(marks, { x = s.x, y = s.y, w = s.w, h = s.h, label = tostring(s.title), tone = s.tone })
		if p.selected == nil or p.selected == i then
			table.insert(
				legend,
				ui.row {
					gap = 8,
					ui.text { tostring(i), size = "sm", weight = "semibold", color = s.tone or "accent" },
					ui.column {
						gap = 2,
						grow = true,
						ui.text { tostring(s.title), size = "sm", weight = "medium" },
						s.detail and ui.text { tostring(s.detail), size = "sm", color = "muted" } or nil,
					},
				}
			)
		end
	end
	if p.selected ~= nil and (type(p.selected) ~= "number" or p.selected < 1 or p.selected > #spots) then
		fail("needs selected = a spot's number, 1 to " .. #spots .. " (or nil)")
	end
	if p.on_select ~= nil and type(p.on_select) ~= "function" then
		fail("needs on_select = function(i) ... end")
	end
	local column = {
		key = p.key,
		gap = 10,
		picture(p, { marks = marks, selected = p.selected, on_click = p.on_select }),
	}
	if p.selected == nil and p.on_select ~= nil then
		table.insert(column, ui.text { "Click a number to read about it.", size = "sm", color = "faint" })
	else
		for _, row in ipairs(legend) do
			table.insert(column, row)
		end
	end
	return ui.column(column)
end)

return kit
