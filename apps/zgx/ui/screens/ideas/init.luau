-- Ideas & Intents: a page of its own from the sidebar (under Search), for
-- what comes to mind before it belongs anywhere. Typed ideas need no title,
-- project or chat; a voice memo is recorded between Record and Stop, kept
-- as it was said (it plays after a restart) and transcribed on this device
-- with the speech model Settings → Voice downloads. Newest first.
--
-- The host keeps them (src/ideas.rs): `ideas.call { op, … }`, and what it
-- holds in the shared `ideas`. The composer's dictation is another thing:
-- it keeps no audio, and this page changes nothing about it.

local ds = require("@ds")
local locale = require("@ds/locale")
local toast = require("@ds/toast")
local prefs = require("@app/screens/settings/prefs")
local route = require("@app/screens/shell/route")

local M = {}

-- Where the sidebar opens it: a page as a plugin's are, under Zend's own
-- reserved id (no plugin can take "zend"), so the shell treats it as one.
M.PLUGIN = "zend"
M.PAGE = "ideas"
M.KEY = `{M.PLUGIN}/{M.PAGE}`
M.ICON = "lightbulb-minimalistic"

M.state = ui.shared("ideas", nil :: any)
local draft = ui.state("ideas_draft", "")

function M.title(): string
	return locale.s("ideas.title")
end

local function call(op: string, args: { [string]: any }?, quiet: boolean?, done: ((ok: boolean) -> ())?)
	local a: { [string]: any } = args or {}
	a.op = op
	local function failed(err: any)
		if done then
			done(false)
		end
		-- (What the page shows says it already: a recording's, a memo's.)
		if quiet then
			return
		end
		toast.show {
			title = locale.s("ideas.couldnt"),
			body = if type(err) == "table" and err.message then tostring(err.message) else tostring(err),
			tone = "warning",
		}
	end
	-- (A window without the host's side, such as a test's, has no
	-- `ideas.call`: a failure like any other, not the shell's error.)
	local ok, pending = pcall(ui.call, "ideas.call", a)
	if not ok then
		failed(pending)
		return
	end
	pending:after(function()
		if done then
			done(true)
		end
	end, failed)
end

-- The draft stays until the idea is on disk: a save that fails leaves it
-- (and what was typed meanwhile) where it was. Only a draft still as it
-- was sent is cleared; one edited while it saved stays for the person.
local saving = ui.state("ideas_saving", false)
local function save()
	local text = draft:peek()
	if saving:peek() or string.match(text, "^%s*$") then
		return
	end
	saving:set(true)
	call("add", { text = text }, false, function(ok: boolean)
		saving:set(false)
		if ok and draft:peek() == text then
			draft:set("")
		end
	end)
end

-- Leaving the page (Settings, a chat, another page, a template's switch)
-- tells the host: a recording is stopped and saved there, one still
-- starting is let go, and none starts until the page shows again. (The page
-- can stay built under Settings, so it's the route that says, not the view.)
local function shown(): boolean
	local c = route.current:get()
	return c.name == "plugin" and c.plugin == M.PLUGIN and c.page == M.PAGE
end
ui.effect(function()
	if shown() then
		call("here", nil, true)
	else
		call("leave", nil, true)
	end
end)

local function clock(seconds: number): string
	local s = math.max(0, math.floor(seconds + 0.5))
	return string.format("%d:%02d", s // 60, s % 60)
end

local function recording(): any
	local s = M.state:get()
	return if s and s.recording then s.recording else { phase = "idle" }
end
local function phase(): string
	return recording().phase or "idle"
end
local function listening(): boolean
	local p = phase()
	return p == "starting" or p == "recording"
end

-- Record, or Stop and Discard while it records; the line beside them says
-- where it stands (and why not, when the microphone wouldn't).
local function recorder(): Node
	return ui.row {
		w_full = true,
		gap = "$space.2",
		items = "center",
		wrap = true,
		ui.show(function(): boolean
			return not listening() and phase() ~= "saving"
		end, ds.Button {
			id = "ideas-record",
			tone = "secondary",
			ds.Icon { name = "microphone", size = 14 },
			ui.t("ideas.record"),
			on_click = function()
				local device = prefs.dictation_input:peek()
				call("record_start", { device = device }, true)
			end,
		}),
		ui.show(listening, ds.Button {
			id = "ideas-stop",
			tone = "primary",
			ds.Icon { name = "stop", size = 14 },
			ui.t("ideas.stop"),
			on_click = function()
				call("record_stop", nil, true)
			end,
		}),
		ui.show(listening, ds.Button {
			id = "ideas-discard",
			tone = "ghost",
			ui.t("ideas.discard"),
			on_click = function()
				call("record_cancel", nil, true)
			end,
		}),
		ui.text {
			id = "ideas-recording-status",
			text_size = "$font.size.sm",
			line_height = "$font.line.sm",
			wrap = true,
			grow = 1,
			min_w = 0,
			color = function(): string
				return if phase() == "failed" then "$danger" else "$text.secondary"
			end,
			function(): string
				local r = recording()
				local p = r.phase
				if p == "starting" then
					return locale.s("ideas.recording.starting")
				elseif p == "recording" then
					return locale.s("ideas.recording.on", { time = clock(r.seconds or 0), max = clock(60) })
				elseif p == "saving" then
					return locale.s("ideas.recording.saving")
				elseif p == "saved" then
					return locale.s("ideas.recording.saved")
				elseif p == "failed" then
					if r.kind == "microphone" then
						return locale.s("ideas.recording.microphone", { error = tostring(r.error or "") })
					elseif r.kind == "empty" then
						return locale.s("ideas.recording.empty")
					end
					return locale.s("ideas.recording.save_failed", { error = tostring(r.error or "") })
				end
				return locale.s("ideas.recording.hint")
			end,
		},
	}
end

-- A memo's words, or why there are none yet (the recording kept either way),
-- with what to do about it.
local function transcript(item: any, model: boolean): Node
	local t = item.transcript
	if t == "done" then
		if item.text == "" then
			return ds.Text { class = "idea-transcript", ui.t("ideas.memo.silent"), style = "caption", tone = "tertiary" }
		end
		return ds.Text { class = "idea-text idea-transcript", item.text, wrap = true, literal = true }
	end
	local words: string
	if t == "transcribing" then
		words = locale.s("ideas.memo.transcribing")
	elseif t == "no_model" and not model then
		words = locale.s("ideas.memo.no_model")
	elseif t == "failed" then
		words = locale.s("ideas.memo.failed", { error = tostring(item.error or "") })
	else
		words = locale.s("ideas.memo.not_yet")
	end
	local actions: { Node } = {}
	if t ~= "transcribing" then
		if not model then
			table.insert(actions, ds.Button {
				class = "idea-get-model",
				tone = "secondary",
				size = "sm",
				ui.t("ideas.memo.get_model"),
				on_click = function()
					route.settings("voice")
				end,
			})
		else
			table.insert(actions, ds.Button {
				class = "idea-transcribe",
				tone = "secondary",
				size = "sm",
				if t == "failed" then ui.t("ideas.memo.retry") else ui.t("ideas.memo.transcribe"),
				on_click = function()
					call("transcribe", { id = item.id })
				end,
			})
		end
	end
	return ui.column {
		w_full = true,
		gap = "$space.2",
		ds.Text {
			class = `idea-status idea-{t}`,
			words,
			literal = true,
			style = "caption",
			wrap = true,
			tone = if t == "failed" then "danger" else "secondary",
		},
		if #actions > 0 then ui.row { gap = "$space.2", table.unpack(actions) } else nil,
	}
end

local function idea(item: any, model: boolean): Node
	local when = locale.ago(os.time() - (item.created or 0) / 1000)
	local meta = if item.memo then `{when} · {clock(item.seconds or 0)}` else when
	local body: { Node } = {}
	if item.memo then
		local playing = item.playing == true
		table.insert(body, ui.row {
			w_full = true,
			gap = "$space.2",
			items = "start",
			ds.IconButton {
				class = "idea-play",
				icon = if playing then "stop" else "action-play",
				label = if playing then ui.t("ideas.memo.stop_playing") else ui.t("ideas.memo.play"),
				size = "sm",
				on_click = function()
					if playing then
						call("play_stop", nil, true)
					else
						call("play", { id = item.id }, true)
					end
				end,
			},
			ui.column { grow = 1, min_w = 0, gap = "$space.1", transcript(item, model) },
		})
		if item.play_error then
			table.insert(body, ds.Text {
				class = "idea-play-error",
				locale.s("ideas.memo.play_failed", { error = tostring(item.play_error) }),
				literal = true,
				style = "caption",
				tone = "danger",
				wrap = true,
			})
		end
	else
		table.insert(body, ds.Text { class = "idea-text", item.text, wrap = true, literal = true })
	end
	return ui.column {
		class = if item.memo then "idea idea-memo" else "idea idea-typed",
		w_full = true,
		gap = "$space.2",
		pt = "$space.3",
		ui.row {
			w_full = true,
			gap = "$space.2",
			items = "center",
			ui.div { color = "$text.tertiary", ds.Icon { name = if item.memo then "microphone" else "pen", size = 14 } },
			ds.Text { meta, literal = true, style = "caption", tone = "tertiary", grow = 1 },
			ds.IconButton {
				class = "idea-delete",
				icon = "trash-bin-minimalistic",
				label = ui.t("ideas.delete"),
				size = "sm",
				tone = "quiet",
				on_click = function()
					call("delete", { id = item.id })
				end,
			},
		},
		ui.column { w_full = true, gap = "$space.2", table.unpack(body) },
		ui.div { w_full = true, pt = "$space.1", ds.Divider {} },
	}
end

-- The list, with what each row shows folded into it (playing, a playback's
-- error), so a row is built again only when what it shows changes.
local items = ui.computed(function(): { any }
	local s = M.state:get()
	if not s or not s.items then
		return {}
	end
	local out = {}
	for _, it in s.items do
		local row = table.clone(it)
		row.playing = s.playing == it.id
		row.play_error = if s.play_error and s.play_error.id == it.id then s.play_error.error else nil
		table.insert(out, row)
	end
	return out
end)

function M.view(): Node
	-- Fresh from disk (another window's, the model downloaded since).
	call("list", nil, true)
	-- (Gone without the route changing, e.g. the window's interface swapped.)
	ui.on_cleanup(function()
		call("leave", nil, true)
	end)
	local function model(): boolean
		local s = M.state:peek()
		return s ~= nil and s.model == true
	end
	return ui.column {
		id = "ideas-page",
		w_full = true,
		h_full = true,
		min_h = 0,
		ui.scroll {
			w_full = true,
			grow = 1,
			min_h = 0,
			fade = { 12, 12 },
			fade_overflow = true,
			ui.column {
				w_full = true,
				items = "center",
				px = "$space.6",
				pt = "$space.6",
				pb = "$space.8",
				ui.column {
					w_full = true,
					max_w = 680,
					gap = "$space.4",
					ui.column {
						w_full = true,
						gap = "$space.1",
						ds.Text { ui.t("ideas.title"), style = "title", heading = 1 },
						ds.Text { ui.t("ideas.description"), tone = "secondary", wrap = true },
					},
					ui.show(function(): boolean
						local s = M.state:get()
						return s == nil or s.ready ~= true
					end, ds.Text {
						id = "ideas-unavailable",
						style = "caption",
						tone = "danger",
						wrap = true,
						function(): string
							local s = M.state:get()
							return locale.s("ideas.unavailable", { error = if s and s.error then tostring(s.error) else "" })
						end,
					}),
					ds.TextArea {
						value = draft,
						w_full = true,
						name = ui.t("ideas.new"),
						placeholder = ui.t("ideas.placeholder"),
						rows = 2,
						max_rows = 10,
						on_submit = function()
							save()
						end,
					},
					ui.row {
						w_full = true,
						gap = "$space.2",
						items = "center",
						ds.Button {
							id = "ideas-save",
							ui.t("ideas.save"),
							disabled = function(): boolean
								return saving:get() or string.match(draft:get(), "^%s*$") ~= nil
							end,
							on_click = save,
						},
						ds.Text { ui.t("ideas.save_hint", { keys = ui.shortcut_text("cmd+enter") }), style = "caption", tone = "tertiary" },
					},
					recorder(),
					ui.column {
						id = "ideas-list",
						w_full = true,
						pt = "$space.2",
						ui.show(function(): boolean
							return #items:get() == 0
						end, ds.Text { id = "ideas-empty", ui.t("ideas.empty"), style = "caption", tone = "tertiary", wrap = true }),
						ui.each(items, function(it: any): Node
							return idea(it, model())
						end, {
							direction = "column",
							w_full = true,
							key = function(it: any): string
								-- (What's shown changing builds the row again.)
								return `{it.id}:{it.transcript or ""}:{it.playing}:{it.play_error or ""}:{#(it.text or "")}:{tostring(model())}`
							end,
						}),
					},
				},
			},
		},
	}
end

return M
