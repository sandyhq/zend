-- Connecting the phone (Android) to a computer: its direct code (the
-- computer's phone link over the tailnet) or its relay's (v=1), by the
-- camera's scan or the code's text pasted, both to the host's
-- `phone.connect` (src/phone_link.rs). Settings → Devices and
-- onboarding's Connect to a computer draw the same view, on the same
-- state. While the camera scans, what it sees shows under Scan code
-- (`phone.scan_preview`: the newest frame, small, grey and upright, from
-- src/android_camera.rs; nil when no scan runs), so the code can be aimed
-- at.
--
-- After the code is kept the engine starts again on it: "Saved,
-- connecting…" until the engine says its registry room is joined
-- (`sessions.now.relay` "connected" M.SETTLE seconds after it: the
-- engine reports a failing room only after 4 s of grace, so an earlier
-- "connected" proves nothing), then "Connected"; no answer
-- in M.WAIT_FOR seconds says so (and turns to Connected if it comes).
--
-- A scan is ended (`phone.scan_stop`, and its answer dropped) when the
-- view goes (another page, another step) or the app is sent away (not
-- while Android's camera dialog has the focus). The
-- pasted text is the view's own, masked, and emptied once submitted: the
-- code is the computer's key. With sessions running on this phone,
-- connecting asks first (the engine's restart stops them).
--
-- A direct code (v=2, the computer's phone link over the tailnet) answers
-- `desktop` instead of `edge`: the window then drives that computer's
-- engine, and "Connected" waits on it answering (its sessions read, the
-- connection healthy, M.SETTLE seconds on), not on a relay.

local ds = require("@ds")
local sessions = require("@account/sessions")
local host = require("@app/screens/settings/host")

local M = {}

M.SETTLE = 8
M.WAIT_FOR = 30

local scan = ui.shared("phone.scan", { state = "idle", frames = 0 } :: any)
local preview = ui.shared("phone.scan_preview", nil :: any)
-- The preview's size on the page, in points (the camera's 4:3, upright).
M.PREVIEW_W = 240
M.PREVIEW_H = 320
M.scanning = ui.state("phone_join_scanning", false)
M.joining = ui.state("phone_join_joining", false)
-- What came of the last try: "" nothing yet, "connecting" (kept, waiting
-- on the relay), "connected", "unreachable", or a failure's word.
M.outcome = ui.state("phone_join_outcome", "")
-- The relay's address as the host shows it (scheme and port).
M.edge = ui.state("phone_join_edge", "")
-- The computer's tailnet name or IP, for a direct code ("" for a relay's).
M.desktop = ui.state("phone_join_desktop", "")
-- Sessions running here, as a code waits on Connect anyway (0: none waits).
M.busy = ui.state("phone_join_busy", 0)
-- (The waiting code itself is no window state.)
local held: string? = nil
-- The engine this window drove when the code was kept: a direct code is
-- connected once another one (the computer's) answers.
local here_before: string? = nil
local settled = ui.state("phone_join_settled", false)
local scan_turn = 0
local watch_turn = 0

local function word_of(err: any): string
	return if type(err) == "string" then err elseif type(err) == "table" or type(err) == "userdata" then err.message or "failed" else "failed"
end

-- Sessions of this phone that a restart of its engine would stop.
local function running_here(): number
	local here = sessions.here:peek()
	local n = 0
	for _, c in (sessions.now:peek().chats or {}) :: { any } do
		if here ~= nil and c.device == here and (c.status == "working" or c.status == "waiting" or c.status == "input") then
			n += 1
		end
	end
	return n
end

local function watch()
	watch_turn += 1
	local turn = watch_turn
	settled:set(false)
	ui.after(M.SETTLE * 1000, function()
		if turn == watch_turn then
			settled:set(true)
		end
	end)
	ui.after(M.WAIT_FOR * 1000, function()
		if turn == watch_turn and M.outcome:peek() == "connecting" then
			M.outcome:set("unreachable")
		end
	end)
end

-- Kept and waiting: Connected once the new engine's room is joined.
ui.effect(function()
	local outcome = M.outcome:get()
	if outcome ~= "connecting" and outcome ~= "unreachable" then
		return
	end
	local now = sessions.now:get()
	-- (Not the computer's relay: a computer reached directly needs none.)
	local up = if M.desktop:get() ~= ""
		then now.state ~= "loading" and now.here ~= nil and now.here ~= here_before
		else now.relay == "connected"
	if up and settled:get() then
		ui.after(0, function()
			if M.outcome:peek() == outcome then
				M.outcome:set("connected")
			end
		end)
	end
end)

-- Whether this phone is on a relay now: joined in this run, or one it
-- already had answers.
M.connected = ui.computed(function(): boolean
	local outcome = M.outcome:get()
	return outcome == "connected" or (outcome == "" and sessions.now:get().relay == "connected")
end)

-- Keep `code` and start the engine on it; with sessions running here,
-- only once confirmed.
function M.join(code: string, confirmed: boolean?)
	if M.joining:peek() or string.match(code, "^%s*$") then
		return
	end
	local n = running_here()
	if n > 0 and not confirmed then
		held = code
		M.busy:set(n)
		return
	end
	held = nil
	here_before = sessions.here:peek()
	watch_turn += 1
	ui.batch(function()
		M.busy:set(0)
		M.joining:set(true)
		M.outcome:set("")
	end)
	host.call("phone.connect", { code = code }):after(function(answer: any)
		ui.batch(function()
			M.joining:set(false)
			M.edge:set(answer.edge or "")
			M.desktop:set(answer.desktop or "")
			M.outcome:set("connecting")
		end)
		watch()
	end, function(err: any)
		ui.batch(function()
			M.joining:set(false)
			M.outcome:set(word_of(err))
		end)
	end)
end

function M.confirm()
	local code = held
	if code then
		M.join(code, true)
	end
end

function M.cancel_confirm()
	held = nil
	M.busy:set(0)
end

function M.start_scan()
	if M.scanning:peek() then
		return
	end
	scan_turn += 1
	local turn = scan_turn
	ui.batch(function()
		M.scanning:set(true)
		M.outcome:set("")
	end)
	host.call("phone.scan", {}):after(function(code: any)
		if turn ~= scan_turn then
			return
		end
		M.scanning:set(false)
		M.join(code)
	end, function(err: any)
		if turn ~= scan_turn then
			return
		end
		ui.batch(function()
			M.scanning:set(false)
			local word = word_of(err)
			M.outcome:set(if word == "stopped" then "" else word)
		end)
	end)
end

-- End the scan running, if one is: its answer, if any comes, is dropped.
function M.stop_scan()
	if not M.scanning:peek() then
		return
	end
	scan_turn += 1
	M.scanning:set(false)
	host.invoke("phone.scan_stop", {})
end

local OUTCOMES: { [string]: string } = {
	bad_code = "settings.devices.phone.bad_code",
	denied = "settings.devices.phone.denied",
	no_camera = "settings.devices.phone.no_camera",
	camera = "settings.devices.phone.camera_failed",
	store = "settings.devices.phone.store_failed",
	engine = "settings.devices.phone.engine_failed",
	busy = "settings.devices.phone.camera_failed",
}

local function says(): string
	local outcome = M.outcome:get()
	local edge = M.edge:get()
	local desktop = M.desktop:get()
	if M.joining:get() then
		return ui.t("settings.devices.phone.joining"):get()
	elseif desktop ~= "" and (outcome == "connecting" or outcome == "connected" or outcome == "unreachable") then
		local key = if outcome == "connected"
			then "settings.devices.phone.desktop_connected"
			elseif outcome == "unreachable" then "settings.devices.phone.desktop_unreachable"
			else "settings.devices.phone.desktop_connecting"
		return ui.t(key, { desktop = desktop }):get()
	elseif outcome == "connecting" then
		return ui.t("settings.devices.phone.saved_connecting", { edge = edge }):get()
	elseif outcome == "connected" then
		return ui.t("settings.devices.phone.connected", { edge = edge }):get()
	elseif outcome == "unreachable" then
		return ui.t("settings.devices.phone.unreachable", { edge = edge }):get()
	elseif outcome ~= "" then
		return ui.t(OUTCOMES[outcome] or "settings.devices.try_later"):get()
	end
	return ""
end

-- Scan, paste, and what came of it.
function M.view(): Node
	local pasted = ui.use_state("")
	ui.on_cleanup(function()
		M.stop_scan()
		M.cancel_confirm()
	end)
	-- (Not while the camera is asked for: the system's dialog takes the
	-- window's focus too.)
	ui.effect(function()
		local away = ui.window.active:get() == false
		local asking = ((scan:get() or {}) :: any).state == "asking"
		if away and not asking then
			ui.after(0, M.stop_scan)
		end
	end)
	local function submit()
		local code = pasted:peek()
		pasted:set("")
		M.join(code)
	end
	return ui.column {
		w_full = true,
		gap = "$space.3",
		ds.Settings.Group {
			ds.Settings.Row {
				label = ui.t("settings.devices.phone.scan_label"),
				description = function(): string
					if not M.scanning:get() then
						return ui.t("settings.devices.phone.scan_hint"):get()
					end
					local s = (scan:get() or {}) :: any
					return if s.state == "asking"
						then ui.t("settings.devices.phone.asking"):get()
						else ui.t("settings.devices.phone.scanning"):get()
				end,
				control = ui.presence(function(): boolean
					return M.scanning:get()
				end, function(on: boolean): Node
					return if on
						then ds.Button {
							ui.t("settings.devices.phone.cancel"),
							tone = "secondary",
							size = "sm",
							on_click = M.stop_scan,
						}
						else ds.Button {
							ui.t("settings.devices.phone.scan"),
							tone = "accent",
							size = "sm",
							disabled = function(): boolean
								return M.joining:get()
							end,
							on_click = M.start_scan,
						}
				end),
			},
			ds.Settings.Rule(),
			ds.Settings.Row {
				label = ui.t("settings.devices.phone.paste_label"),
				stacked = true,
				control = ui.row {
					w_full = true,
					gap = "$space.2",
					items = "center",
					ds.TextField {
						value = pasted,
						name = ui.t("settings.devices.phone.paste_name"),
						placeholder = "zend://connect?…",
						password = true,
						grow = 1,
						on_commit = function()
							submit()
						end,
					},
					ds.Button {
						ui.t("settings.devices.phone.connect"),
						tone = "secondary",
						size = "sm",
						disabled = function(): boolean
							return M.joining:get() or string.match(pasted:get(), "^%s*$") ~= nil
						end,
						on_click = submit,
					},
				},
			},
		},
		-- What the camera sees, while it scans (a frame turned the other way,
		-- the phone on its side, keeps its own shape).
		ui.show(function(): boolean
			return M.scanning:get() and preview:get() ~= nil
		end, ui.row {
			w_full = true,
			justify = "center",
			ui.image {
				id = "phone-scan-preview",
				src = function(): any
					return preview:get()
				end,
				label = ui.t("settings.devices.phone.preview"),
				fit = "contain",
				w = M.PREVIEW_W,
				h = M.PREVIEW_H,
				rounded = "$radius.lg",
				overflow = "hidden",
			},
		}),
		ui.show(function(): boolean
			return M.busy:get() > 0
		end, ds.Callout {
				tone = "warning",
				title = ui.t("settings.devices.phone.busy_title") :: any,
				body = function(): string
					return ui.t("settings.devices.phone.busy_body", { n = M.busy:get() }):get()
				end :: any,
				actions = {
					{ label = ui.t("settings.devices.phone.busy_go"), on_click = M.confirm, primary = true },
					{ label = ui.t("common.cancel"), on_click = M.cancel_confirm },
				},
			}),
		ui.text {
			says,
			id = "phone-join-outcome",
			live = "polite",
			text_size = "$font.size.md",
			color = function(): string
				local outcome = M.outcome:get()
				return if outcome == "" or outcome == "connecting"
					then "$text.secondary"
					elseif outcome == "connected" then "$zend.tone.success.ink"
					else "$zend.tone.danger.ink"
			end,
			wrap = true,
		},
	}
end

return M
