-- Settings → About & updates: which Zend this is, and its updates, as the
-- shipping app has them (its About panel and "Check for Updates…", crates/ui
-- app_menus.rs, app_update.rs; the engine's updater, crates/update).
--
-- As the Zend app (the beta: `self_updating`), it updates itself as
-- Zend.app does (src/about.rs): a new version downloads in the background,
-- "Restart to update" installs it, and one nobody restarted for installs on
-- quit. "Get beta updates" chooses the release channel; leaving beta goes
-- back to Stable even when that's an older version.
--
-- A development build isn't packaged: what updates is the engine's updater,
-- the engine on this Mac, which knows its version and the newest release
-- and, on an install it manages, updates itself and starts again on it.
--
-- The host's `about` (shared): { app_version, app_build, engine_version?,
-- engine_build?, latest?, available, checked_at?, error?, self_updating,
-- channels, channel?, channel_note?, beta_minimum?, held?, switch, flow,
-- flow_version?, flow_error?, blocker? }.

local ds = require("@ds")
local host = require("@app/screens/settings/host")
local search = require("@app/screens/settings/search")
local locale = require("@ds/locale")
local peer_area = require("@app/screens/settings/peer_area")
local devices_page = require("@app/screens/settings/devices")

local M = {}

M.ID = "about"
M.TITLE = "About & updates"

M.RELEASES = "https://github.com/sandyhq/comet/releases"
M.WEBSITE = "https://zend.invalid"

local rows: { search.Row } = {
	{ key = "version", title = ui.t("settings.about.version.label"), terms = ui.t("settings.about.version.terms") },
	{
		key = "updates",
		title = ui.t("settings.about.updates.label"),
		section = ui.t("settings.about.updates_section"),
		terms = ui.t("settings.about.updates.terms"),
	},
	{
		key = "release_notes",
		title = ui.t("settings.about.release_notes.label"),
		section = ui.t("settings.about.links_section"),
		terms = ui.t("settings.about.release_notes.terms"),
	},
	{
		key = "beta",
		title = ui.t("settings.about.beta.label"),
		section = ui.t("settings.about.updates_section"),
		terms = ui.t("settings.about.beta.terms"),
	},
	{ key = "website", title = ui.t("settings.about.website.label"), section = ui.t("settings.about.links_section"), terms = ui.t("settings.about.website.terms") },
	{ key = "copy_version", title = ui.t("settings.about.copy_version.label"), terms = ui.t("settings.about.copy_version.terms") },
}
-- (Android: no updater, so not its rows: the store updates the app.)
if not require("@ds/platform").has("updates") then
	local kept: { search.Row } = {}
	for _, row in rows do
		if row.key ~= "updates" and row.key ~= "beta" then
			table.insert(kept, row)
		end
	end
	rows = kept
end
search.index(M.ID, ui.t("settings.about.title"), ui.t("settings.about.terms"), rows)

export type About = {
	app_version: string,
	app_build: string?,
	engine_version: string?,
	engine_build: string?,
	latest: string?,
	available: boolean,
	checked_at: number?,
	error: string?,
	self_updating: boolean?,
	channels: boolean?,
	channel: string?,
	channel_note: string?,
	beta_minimum: string?,
	held: string?,
	switch: boolean?,
	flow: string?,
	flow_version: string?,
	flow_error: string?,
	blocker: string?,
}

M.now = ui.shared("about", { app_version = "", available = false } :: About)

-- A paired server's update (src/peer_updates.rs): the server shown ("" this
-- computer: the page as it is), what its engine said (shared `about_device`),
-- and what this window is doing there. Asking a server that doesn't grant
-- `updates` sends nothing: the page says so, with the command to run there.
local chosen = ui.state("about_device_chosen", "")
M.device_now = ui.shared("about_device", { device = "", state = "idle", available = false } :: any)
M.device_doing = ui.state("about_device_doing", "")

function M.device(): string?
	local d = chosen:get()
	return if d ~= "" then d else nil
end

local function here(): string?
	for _, d in require("@account/sessions").devices:get() or {} do
		if d.here then
			return d.id
		end
	end
	return nil
end

function M.choose(id: string)
	local next = if id == here() then "" else id
	if next == chosen:peek() then
		return
	end
	M.device_doing:set("")
	chosen:set(next)
	host.invoke("peer_updates.show", next)
end

local function server_refused(): boolean
	return peer_area.refused(M.device(), "updates")
end

-- What this window is doing about an update ("" nothing): "checking",
-- "updating", "updated" (the engine starting again on `M.version`), or how
-- the last try ended: "current", "unmanaged", "offline", "failed".
M.doing = ui.state("about_doing", "")
-- The version an update went to.
local updated_to = ui.state("about_updated_to", "")

-- An engine build ("<commit>.<seconds>") or this app's ("<seconds>"), as
-- people read it: the commit's first seven, the day it was built (in the
-- locale shown: read where it's drawn).
function M.build(b: string?): string?
	if not b or b == "" then
		return nil
	end
	local commit, secs = string.match(b, "^(%x+)%.(%d+)$")
	if not secs then
		secs = string.match(b, "^(%d+)$")
	end
	local n = tonumber(secs)
	local day: string? = nil
	if n and n > 0 then
		day = locale.date(n)
	end
	if commit and day then
		return ui.t("settings.about.build", { commit = string.sub(commit, 1, 7), day = day }):get()
	end
	return day or (if commit then string.sub(commit, 1, 7) else b)
end

-- When the newest release was last asked, said shortly.
local function ago(ms: number?): string?
	if not ms then
		return nil
	end
	local s = math.max(0, os.time() - math.floor(ms / 1000))
	if s < 60 then
		return ui.t("settings.about.ago.just_now"):get()
	elseif s < 3600 then
		return ui.t("settings.about.ago.minutes", { n = math.floor(s / 60) }):get()
	elseif s < 86400 then
		return ui.t("settings.about.ago.hours", { n = math.floor(s / 3600) }):get()
	end
	return ui.t("settings.about.ago.days", { n = math.floor(s / 86400) }):get()
end

function M.check()
	if M.doing:peek() == "checking" or M.doing:peek() == "updating" then
		return
	end
	M.doing:set("checking")
	host.call("about.check"):after(function()
		M.doing:set("")
	end, function(err: any)
		M.doing:set(if err.message == "offline" then "offline" else "failed")
	end)
end

function M.update()
	if M.doing:peek() == "updating" then
		return
	end
	M.doing:set("updating")
	host.call("about.apply"):after(function(answer: any)
		updated_to:set(answer and answer.version or "")
		M.doing:set("updated")
	end, function(err: any)
		local word = err.message
		-- (Held: the page says why from `about.held`.)
		M.doing:set(if word == "held" then "" elseif word == "unmanaged" or word == "current" or word == "offline" then word else "failed")
	end)
end

-- "Get beta updates": the host answers with the channel it chose (or why
-- not, `channel_note`), and the switch follows that.
local beta = ui.state("about_beta", false)
ui.effect(function()
	beta:set(M.now:get().channel == "beta")
end)
function M.set_beta(on: boolean)
	host.invoke("about.channel", on)
end

-- What the update row says now: its title, its line, and its action (in
-- the locale shown: read where it's drawn).
export type Update = { title: string, line: string, action: string?, accent: boolean? }

function M.describe(a: About, doing: string): Update
	local current = a.engine_version
	local checked = ago(a.checked_at)
	local function s(key: string, args: { [string]: any }?): string
		return ui.t(`settings.about.state.{key}`, args):get()
	end
	local retry = ui.t("common.retry"):get()
	if a.self_updating then
		-- The Zend app's own updates (Zend.app's lifecycle).
		local mine = a.app_version
		if doing == "checking" then
			return { title = s("checking"), line = s("checking_line") }
		elseif a.flow == "downloading" then
			return { title = s("downloading", { version = a.flow_version or "" }), line = s("downloading_line") }
		elseif a.flow == "ready" then
			return { title = s("ready", { version = a.flow_version or "" }), line = s("ready_line"), action = s("restart"), accent = true }
		elseif a.flow == "failed" then
			return {
				title = s("download_failed", { version = a.flow_version or "" }),
				line = a.flow_error or s("failed_line"),
				action = retry,
				accent = true,
			}
		elseif a.held and a.latest then
			return { title = s("held", { version = a.latest }), line = a.held, action = s("check_again") }
		elseif doing == "offline" then
			return { title = s("couldnt_check"), line = s("offline_line"), action = retry }
		elseif doing == "failed" then
			return { title = s("couldnt_update"), line = s("failed_line"), action = retry }
		elseif a.available and a.latest then
			local title = if a.switch then s("available_stable", { version = a.latest }) else s("available", { version = a.latest })
			return { title = title, line = s("you_have", { version = mine }), action = s("update"), accent = true }
		elseif a.error and not checked then
			return { title = s("couldnt_check"), line = s("unreachable_line"), action = retry }
		end
		return {
			title = s("up_to_date"),
			line = if checked then s("newest_checked", { version = mine, ago = checked }) else s("newest", { version = mine }),
			action = s("check"),
		}
	end
	if doing == "checking" then
		return { title = s("checking"), line = s("checking_line") }
	elseif doing == "updating" then
		return { title = s("updating"), line = s("updating_line", { version = a.latest or "" }) }
	elseif doing == "updated" then
		local v = updated_to:peek()
		if current and v ~= "" and current == v then
			return { title = s("updated", { version = v }), line = s("updated_running") }
		end
		return { title = s("updated", { version = v }), line = s("updated_starting") }
	elseif doing == "unmanaged" then
		return {
			title = s("unmanaged"),
			line = s("unmanaged_line"),
			action = s("check_again"),
		}
	elseif doing == "offline" then
		return { title = s("couldnt_check"), line = s("offline_line"), action = retry }
	elseif doing == "failed" then
		return { title = s("couldnt_update"), line = s("failed_line"), action = retry }
	end
	if not current then
		return { title = s("not_connected"), line = s("not_connected_line"), action = s("check") }
	elseif a.available and a.latest then
		return { title = s("available", { version = a.latest }), line = s("you_have", { version = current }), action = s("update_engine"), accent = true }
	elseif a.error and not checked then
		return { title = s("couldnt_check"), line = s("unreachable_line"), action = retry }
	end
	return {
		title = s("up_to_date"),
		line = if checked then s("newest_checked", { version = current, ago = checked }) else s("newest", { version = current }),
		action = s("check"),
	}
end

-- What the update row says for the server shown: the engine's own words
-- (`describe`) over what that server said.
local function server_update(): Update
	local s: any = M.device_now:get()
	return M.describe({
		app_version = "",
		engine_version = s.current,
		latest = s.latest,
		available = s.available == true,
		checked_at = s.checked_at,
		error = s.error,
	} :: About, M.device_doing:get())
end

function M.check_server()
	local d = M.device()
	if not d or M.device_doing:peek() == "updating" then
		return
	end
	M.device_doing:set("")
	host.invoke("peer_updates.show", d)
end

function M.update_server()
	local d = M.device()
	if not d or M.device_doing:peek() == "updating" then
		return
	end
	M.device_doing:set("updating")
	host.call("peer_updates.apply", { device = d }):after(function(answer: any)
		updated_to:set(answer and answer.version or "")
		M.device_doing:set("updated")
	end, function(err: any)
		local word = err.message
		M.device_doing:set(if word == "unmanaged" or word == "current" or word == "offline" then word else "failed")
	end)
end

-- The version facts as text, for a bug report (in English, as support
-- reads them).
function M.summary(a: About): string
	local lines = {
		`Zend engine: {a.engine_version or "not connected"}`,
	}
	if a.engine_build then
		table.insert(lines, `Engine build: {a.engine_build}`)
	end
	table.insert(lines, `App: zgx {a.app_version}{if a.app_build and a.app_build ~= "" then ` (built {a.app_build})` else ""}`)
	if a.latest then
		table.insert(lines, `Newest release: {a.latest}`)
	end
	return table.concat(lines, "\n")
end

local function row(key: string, props: any): Node
	return search.anchor(key, ds.Settings.Row(props))
end

local function fact(text: () -> string): Node
	return ui.text {
		text,
		text_size = "$font.size.md",
		line_height = "$font.line.md",
		color = "$text.secondary",
		tabular = true,
	}
end

local function now_update(): Update
	return M.describe(M.now:get(), M.doing:get())
end

function M.view(): Node
	return ds.Settings.Page {
		title = ui.t("settings.about.title"),
		description = ui.t("settings.about.description"),
		-- Updates are per device: whose (a choice once there's more than this
		-- computer; the same one as Plugins' and Providers').
		trail = devices_page.picker(function(): string?
			return M.device()
		end, M.choose, "updates"),
		trail_top = true,
		ui.show(server_refused, peer_area.Notice(M.device, "updates")),
		ui.show(function(): boolean
			return M.device() ~= nil and not server_refused()
		end, ds.Settings.Section {
			title = ui.t("settings.about.updates_section"),
			ds.Settings.Group {
				row("updates", {
					label = function(): string
						return server_update().title
					end,
					description = function(): string
						return server_update().line
					end,
					control = ui.row {
						items = "center",
						ui.show(function(): boolean
							return server_update().accent == true
						end, ds.Button {
							function(): string
								return server_update().action or ""
							end,
							tone = "accent",
							size = "sm",
							on_click = M.update_server,
						}),
						ui.show(function(): boolean
							local u = server_update()
							return u.action ~= nil and not u.accent
						end, ds.Button {
							function(): string
								return server_update().action or ""
							end,
							tone = "secondary",
							size = "sm",
							on_click = M.check_server,
						}),
					},
				}),
			},
		}),
		ui.show(function(): boolean
			return M.device() == nil
		end, ds.Settings.Section {
			ds.Settings.Group {
				row("version", {
					label = "Zend",
					description = function(): string
						return M.build(M.now:get().engine_build) or ui.t("settings.about.version.description"):get()
					end,
					control = fact(function(): string
						return M.now:get().engine_version or "—"
					end),
				}),
				row("app_version", {
					label = ui.t("settings.about.app.label"),
					description = function(): string
						local built = M.build(M.now:get().app_build)
						if M.now:get().self_updating and built then
							return ui.t("settings.about.app.packaged", { built = built }):get()
						end
						return if built
							then ui.t("settings.about.app.built", { built = built }):get()
							else ui.t("settings.about.app.not_packaged"):get()
					end,
					control = fact(function(): string
						local v = M.now:get().app_version
						return if v ~= "" then v else "—"
					end),
				}),
				row("copy_version", {
					label = ui.t("settings.about.version_info.label"),
					description = ui.t("settings.about.version_info.description"),
					control = ds.Button {
						ui.t("common.copy"),
						tone = "secondary",
						size = "sm",
						on_click = function()
							ui.clipboard.write(M.summary(M.now:peek()))
							ds.toast.show { title = ui.t("settings.about.version_info.copied"):get(), tone = "success" }
						end,
					},
				}),
			},
		}),
		-- (Android: the store updates the app; no updater here.)
		ui.show(function(): boolean
			return M.device() == nil and require("@ds/platform").has("updates")
		end, ds.Settings.Section {
			title = ui.t("settings.about.updates_section"),
			-- (Whether the app updates itself is fixed for the process.)
			description = if M.now:peek().self_updating
				then ui.t("settings.about.updates_self_description")
				else ui.t("settings.about.updates_description"),
			ds.Settings.Group {
				ui.column {
					w_full = true,
					row("updates", {
						label = function(): string
							return now_update().title
						end,
						description = function(): string
							return now_update().line
						end,
						-- (Updating is the one accented action; the rest ask again.)
						control = ui.row {
							items = "center",
							ui.show(function(): boolean
								return now_update().accent == true
							end, ds.Button {
								function(): string
									return now_update().action or ""
								end,
								tone = "accent",
								size = "sm",
								on_click = M.update,
							}),
							ui.show(function(): boolean
								local u = now_update()
								return u.action ~= nil and not u.accent
							end, ds.Button {
								function(): string
									return now_update().action or ""
								end,
								tone = "secondary",
								size = "sm",
								on_click = M.check,
							}),
						},
					}),
					-- The release channel (an install that follows channels).
					-- (A row that comes and goes brings its own hairline: none under the
					-- one above while it's away.)
					ui.show(function(): boolean
						return M.now:get().channels == true
					end, ui.column {
						w_full = true,
						ds.Settings.Rule(),
						row("beta", {
						label = ui.t("settings.about.beta.label"),
						description = function(): string
							local a = M.now:get()
							if a.channel_note then
								return a.channel_note
							end
							if a.channel == "beta" then
								return ui.t("settings.about.beta.on"):get()
							end
							return if a.beta_minimum
								then ui.t("settings.about.beta.off", { version = a.beta_minimum }):get()
								else ui.t("settings.about.beta.off_any"):get()
						end,
						control = ds.Switch { value = beta, label = ui.t("settings.about.beta.label"), on_change = M.set_beta },
						}),
					}),
				},
			},
		}),
		ds.Settings.Section {
			title = ui.t("settings.about.links_section"),
			ds.Settings.Group {
				row("release_notes", {
					label = ui.t("settings.about.release_notes.label"),
					description = ui.t("settings.about.release_notes.description"),
					on_click = function()
						local v = M.now:peek().latest or M.now:peek().engine_version
						host.invoke("open_url", if v then `{M.RELEASES}/tag/v{v}` else M.RELEASES)
					end,
					href = function(): string
						local v = M.now:peek().latest or M.now:peek().engine_version
						return if v then `{M.RELEASES}/tag/v{v}` else M.RELEASES
					end,
				}),
				row("website", {
					label = ui.t("settings.about.website.label"),
					description = "zend.invalid",
					on_click = function()
						host.invoke("open_url", M.WEBSITE)
					end,
					href = M.WEBSITE,
				}),
			},
		},
	}
end

return M
