-- What any tool's row opens to beyond a command's output, rendered for its
-- kind: the tool's whole call and result as the device that ran it keeps
-- them (the host's chat.tool_exchange, src/tool_open.rs, from its run
-- journal). The chat itself keeps only a call's identifiers and a line of
-- its result, so this is asked for as the row opens, once, and kept by the
-- tool's id.
--
--   a read of a picture     the picture, as a reply's is (ds/reply_picture:
--                           its frame, the lightbox, Copy), its bytes read
--                           from that device (files.tool_image)
--   a read of text          its lines, coloured for the file's language,
--                           numbered from where the read started
--   an edit, a write, a     the diff, as the git pages draw it (ds/tool_detail)
--   patch
--   a script (codemode's    its code, coloured, then its result
--   code), a command key
--   an MCP or other tool    its input key by key (a long or structured value
--                           as a block of its own, JSON coloured), then its
--                           result: JSON coloured, else as an output
--   a web search or fetch   its answer, as Markdown
--   anything else           its result as a command's output shows
--
-- Long code shows its first lines, fading, with "Show all N lines" under
-- them (the fold kept by the tool's id). While it comes, a quiet line says
-- so; when it can't (the device offline, too old, nothing kept), the line
-- says why and a press asks again, and what the chat kept shows under it.
--
--   ToolOpen.ToolOpen { tool = function() return tool end }
--   ToolOpen.wants(tool)    whether a tool's row opens to this

local locale = require("@self/locale")
local FoldButton = require("@self/fold_button")
local ToolDetail = require("@self/tool_detail")
local ReplyPicture = require("@self/reply_picture")
local Markdown = require("@self/markdown")
local Syntax = require("@ds/syntax")

local M = {}

-- What of a tool this reads (ds/activity's Tool has these).
export type Tool = {
	id: string,
	kind: string?,
	call_kind: string?,
	chat: string?,
	live: boolean?,
	failed: boolean?,
	output: string?,
	exit_code: number?,
}
type Exchange = { state: "loading" | "ready" | "failed", value: any?, error: string?, version: number? }

-- The lines a block of code shows folded, before "Show all N lines".
M.FOLD_LINES = 16
-- Kinds of row this never opens: a command (ds/tool_detail's output), a
-- subagent (its lane), a thought, a Cloud machine's steps.
local NOT: { [string]: boolean } = { thought = true, agent = true, cloud = true, git = true }

local EXCHANGES = ui.state("tool_exchanges", {} :: { [string]: Exchange })
local PICTURES = ui.state("tool_pictures", {} :: { [string]: any })
local CODE_OPEN = ui.state("tool_open_code", {} :: { [string]: boolean })

-- How a tool's whole call and result are read from its host, and the
-- picture a read read (a test or the gallery brings its own).
M.fetch = function(chat: string, id: string): any
	return ui.call("chat.tool_exchange", { chat = chat, id = id })
end
M.fetch_image = function(chat: string, id: string): any
	return ui.call("files.tool_image", { chat = chat, id = id })
end

-- `chat`: the doc the tool is in, when the tool doesn't name it (its
-- transcript's chat; a cut command's output names its own).
function M.wants(t: Tool, chat: string?): boolean
	return (t.chat ~= nil or chat ~= nil) and t.call_kind ~= nil and t.call_kind ~= "exec" and not NOT[t.kind or ""]
end

local function key_of(chat: string, id: string): string
	return chat .. "\0" .. id
end

local function word(err: any): string
	return if type(err) == "table" and err.message then tostring(err.message) else tostring(err)
end

local function put(state: any, key: string, value: any)
	local now = table.clone(state:peek())
	now[key] = value
	state:set(now)
end

local versions = 0

local function request(chat: string, id: string)
	local key = key_of(chat, id)
	local was = EXCHANGES:peek()[key]
	if was and was.state == "loading" then
		return
	end
	put(EXCHANGES, key, { state = "loading", value = if was then was.value else nil })
	local ok, promise = pcall(M.fetch, chat, id)
	if not ok or not promise then
		put(EXCHANGES, key, { state = "failed", error = if ok then "failed" else word(promise) })
		return
	end
	promise:after(function(got: any)
		if type(got) == "table" and type(got.sections) == "table" then
			versions += 1
			put(EXCHANGES, key, { state = "ready", value = got, version = versions })
		else
			put(EXCHANGES, key, { state = "failed", error = "failed" })
		end
	end, function(err: any)
		put(EXCHANGES, key, { state = "failed", error = word(err) })
	end)
end

-- The pictures tools read, as ds/reply_picture asks for them: by
-- "chat\0id", read once.
local pictures: ReplyPicture.Pictures = {
	resolve = function(src: string): any
		return PICTURES:get()[src] or { state = "loading" }
	end,
	fetch = function(src: string)
		if PICTURES:peek()[src] then
			return
		end
		local chat, id = string.match(src, "^(.-)%z(.*)$")
		if not chat or not id then
			return
		end
		put(PICTURES, src, { state = "loading" })
		local ok, promise = pcall(M.fetch_image, chat, id)
		if not ok or not promise then
			put(PICTURES, src, { state = "failed", error = if ok then nil else word(promise) })
			return
		end
		promise:after(function(got: any)
			if type(got) == "table" and type(got.src) == "string" then
				put(PICTURES, src, { state = "ready", src = got.src })
			else
				put(PICTURES, src, { state = "failed" })
			end
		end, function(err: any)
			put(PICTURES, src, { state = "failed", error = word(err) })
		end)
	end,
}

local function caption(text: string): Node
	return ui.text {
		text,
		literal = true,
		class = "tool-open-caption",
		text_size = "$font.size.xs",
		line_height = "$font.line.xs",
		color = "$text.tertiary",
	}
end

local function quiet(text: string | () -> string, class: string): Node
	return ui.text {
		text,
		literal = true,
		class = class,
		text_size = "$font.size.xs",
		line_height = "$font.line.xs",
		color = "$text.tertiary",
	}
end

-- Lines of code as one text, each run in its colour (ds/syntax), with
-- their numbers in a gutter when they have them.
local function lines_view(lines: { string }, lang: string?, start: number?): Node
	local source = table.concat(lines, "\n")
	local spans: { [any]: any } = {
		class = "tool-open-code-text",
		literal = true,
		shrink = 0,
		pr = "$space.3",
		font = "$font.mono",
		text_size = "$font.size.xs",
		line_height = "$font.line.xs",
		color = "$text.primary",
	}
	if lang and Syntax.highlighter(lang) then
		for i, line in Syntax.lines(source, lang) do
			if i > 1 then
				table.insert(spans, ui.span { "\n", literal = true })
			end
			for _, run in line do
				table.insert(spans, ui.span { (string.gsub(run.text, "\t", "    ")), color = run.color, literal = true })
			end
		end
	end
	local body: Node
	if #spans > 0 then
		body = ui.rich(spans)
	else
		spans[1] = (string.gsub(source, "\t", "    "))
		body = ui.text(spans)
	end
	local gutter: Node? = nil
	if start then
		local nums = table.create(#lines)
		for i = 1, #lines do
			nums[i] = tostring(start + i - 1)
		end
		gutter = ui.text {
			table.concat(nums, "\n"),
			literal = true,
			shrink = 0,
			pl = "$space.3",
			pr = "$space.3",
			align = "right",
			tabular = true,
			font = "$font.mono",
			text_size = "$font.size.xs",
			line_height = "$font.line.xs",
			color = "$text.tertiary",
		}
	end
	return ui.row {
		w_full = true,
		min_w = 0,
		items = "start",
		gutter,
		ui.scroll_x {
			grow = 1,
			min_w = 0,
			fade_x = { 16, 32 },
			fade_overflow = true,
			ui.div { pl = if gutter then nil else "$space.3", body },
		},
	}
end

local function split(text: string): { string }
	local out = string.split(text, "\n")
	if #out > 1 and out[#out] == "" then
		table.remove(out)
	end
	return out
end

-- A block of code (a script, a read's lines, a JSON value), folded past
-- FOLD_LINES; kept open by fold.
local function code_block(props: { fold: string, text: string, lang: string?, start: number?, label: string?, more: number?, class: string }): Node
	local lines = split(props.text)
	local long = #lines > M.FOLD_LINES
	local line = (ui.theme.get("font.line.xs") :: number?) or 14
	local function is_open(): boolean
		return CODE_OPEN:get()[props.fold] == true
	end
	local head = table.move(lines, 1, math.min(#lines, M.FOLD_LINES), 1, {})
	return ui.column {
		w_full = true,
		gap = "$space.1",
		class = props.class,
		label = props.lang,
		if props.label then caption(props.label) else nil,
		ui.column {
			w_full = true,
			min_w = 0,
			py = "$space.2",
			rounded = "$radius.md",
			bg = "$zend.code.wash",
			overflow = "hidden",
			fade = function(): { number }
				return if long and not is_open() then { 0, line * 1.5 } else { 0, 0 }
			end,
			if long
				then ui.show(is_open, ui.component(function(): Node
					return lines_view(lines, props.lang, props.start)
				end)({}), lines_view(head, props.lang, props.start))
				else lines_view(lines, props.lang, props.start),
		},
		if props.more and props.more > 0
			then quiet(locale.s("ds.tool_detail.more_lines", { n = props.more }), "tool-open-more")
			else nil,
		if long
			then FoldButton.FoldButton {
				label = function(): string
					return if is_open()
						then locale.s("ds.activity.show_fewer_lines")
						else locale.s("ds.activity.show_all_lines", { n = tostring(#lines) })
				end,
				open = is_open,
				class = "tool-open-fold",
				on_click = function()
					put(CODE_OPEN, props.fold, if is_open() then nil else true)
				end,
				place = { self = "start" },
			}
			else nil,
	}
end

-- A structured input, key by key.
local function fields_view(id: string, n: number, rows: { any }): Node
	local kids: { Node } = {}
	for i, r in rows do
		if r.block then
			table.insert(kids, code_block {
				fold = `{id}:f{n}:{i}`,
				text = r.text,
				lang = if r.json then "json" else nil,
				label = r.key,
				class = "tool-open-field",
			})
		else
			table.insert(kids, ui.row {
				w_full = true,
				min_w = 0,
				gap = "$space.2",
				items = "start",
				class = "tool-open-field",
				label = r.key,
				ui.text { r.key, literal = true, shrink = 0, font = "$font.mono", text_size = "$font.size.xs", line_height = "$font.line.xs", color = "$text.tertiary" },
				ui.text { r.text, literal = true, wrap = true, shrink = 1, min_w = 0, font = "$font.mono", text_size = "$font.size.xs", line_height = "$font.line.xs", color = "$text.primary" },
			})
		end
	end
	return ui.column { w_full = true, gap = "$space.1", class = "tool-open-fields", table.unpack(kids) }
end

local NOTES = { running = "ds.tool_open.running", withheld = "ds.tool_open.withheld" }

-- One section of what the host sent (src/tool_open.rs), as it shows.
local function section_view(t: Tool, n: number, s: any): Node?
	local id = t.id
	if s.kind == "script" then
		return code_block {
			fold = `{id}:s{n}`,
			text = s.text or "",
			lang = s.lang,
			label = if s.label == "result" then locale.s("ds.tool_open.result") else s.label,
			class = "tool-open-script",
		}
	elseif s.kind == "json" then
		return code_block { fold = `{id}:j{n}`, text = s.text or "", lang = "json", class = "tool-open-script" }
	elseif s.kind == "fields" then
		return fields_view(id, n, s.rows or {})
	elseif s.kind == "code" then
		return code_block {
			fold = `{id}:c{n}`,
			text = s.text or "",
			lang = if s.path then Syntax.language(s.path) else nil,
			start = s.start or 1,
			more = s.more,
			class = "tool-open-code",
		}
	elseif s.kind == "image" then
		local src = key_of(s.chat or t.chat or "", s.id or id)
		local alt = string.match(s.path or "", "([^/\\]+)$") or s.path or ""
		return ui.div {
			w_full = true,
			class = "tool-open-image",
			label = alt,
			ReplyPicture.Single {
				pictures = pictures,
				src = src,
				alt = alt,
				group = `tool:{id}`,
				set = function(): { ReplyPicture.Entry }
					return { { src = src, alt = alt } }
				end,
			},
		}
	elseif s.kind == "diff" then
		return ToolDetail.ToolDetail {
			tool = function(): any
				return { id = `{id}:d{n}`, diff = s }
			end,
		}
	elseif s.kind == "output" then
		return ToolDetail.ToolDetail {
			tool = function(): any
				return { id = id, output = s.text, exit_code = s.exit_code, failed = t.failed }
			end,
		}
	elseif s.kind == "prose" then
		return ui.div {
			w_full = true,
			min_w = 0,
			class = "tool-open-prose",
			Markdown { text = s.text or "", small = true, ink = "$text.secondary" },
		}
	elseif s.kind == "note" then
		return quiet(locale.s(NOTES[s.text] or "ds.tool_open.nothing"), "tool-open-note")
	end
	return nil
end

local WHY = {
	offline = "ds.tool_open.offline",
	update = "ds.tool_open.update",
	unavailable = "ds.tool_open.unavailable",
	running = "ds.tool_open.running",
}

M.ToolOpen = ui.component(function(props: { tool: () -> Tool, chat: (() -> string?)? }): Node
	local function now(): Tool
		local t = props.tool()
		if t.chat == nil and props.chat then
			local chat = props.chat()
			if chat then
				t = table.clone(t)
				t.chat = chat
			end
		end
		return t
	end
	local function key(): string
		local t = now()
		return key_of(t.chat or "", t.id)
	end
	local function got(): Exchange?
		return EXCHANGES:get()[key()]
	end
	-- Asked for as it opens; asked again once a tool that was running when
	-- it was asked has finished.
	ui.effect(function()
		local t = now()
		local was = EXCHANGES:peek()[key()]
		if t.chat and (was == nil or (not t.live and was.state == "ready" and was.value.done == false)) then
			request(t.chat, t.id)
		end
	end)
	local shown = ui.computed(function(): any
		local e = got()
		if e and e.value then
			return e.value
		end
		return nil
	end)
	return ui.column {
		w_full = true,
		gap = "$space.2",
		class = "tool-open",
		ui.presence(function(): string
			local e = got()
			if e and e.value then
				return `ready:{e.version or 0}`
			end
			return if e and e.state == "failed" then "failed" else "loading"
		end, function(state: string): Node
			if state == "loading" then
				return quiet(locale.s("ds.tool_open.loading"), "tool-open-loading")
			elseif state == "failed" then
				return ui.column {
					w_full = true,
					gap = "$space.2",
					ui.button {
						p = 0,
						bg = "#00000000",
						hover = { bg = "#00000000" },
						active = { bg = "#00000000" },
						cursor = "pointer",
						class = "tool-open-trouble",
						on_click = function()
							local t = now()
							if t.chat then
								request(t.chat, t.id)
							end
						end,
						quiet(function(): string
							local e = got()
							return locale.s(WHY[if e then e.error or "" else ""] or "ds.tool_open.failed")
						end, "tool-open-why"),
					},
					-- (What the chat kept, meanwhile.)
					ui.show(function(): boolean
						return ToolDetail.has(now() :: any)
					end, ToolDetail.ToolDetail { tool = now :: any }),
				}
			end
			-- (Each answer that came, made again: a finished tool's second.)
			return ui.component(function(): Node
				local v = shown:get() or {}
				local kids: { Node } = {}
				for n, s in v.sections or {} do
					local node = section_view(now(), n, s)
					if node then
						table.insert(kids, node)
					end
				end
				if #kids == 0 then
					table.insert(kids, quiet(locale.s("ds.tool_open.nothing"), "tool-open-note"))
				end
				return ui.column { w_full = true, gap = "$space.2", table.unpack(kids) }
			end)({})
		end),
	}
end)

-- Forgets what was read (a test's fresh start).
function M.forget()
	EXCHANGES:set({})
	PICTURES:set({})
	CODE_OPEN:set({})
end

return M
