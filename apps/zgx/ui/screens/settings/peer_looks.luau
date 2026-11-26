--!strict
-- Settings → Themes and → Templates on a paired server: its list, with Use
-- and Delete, asked of it (src/peer_looks.rs) and never worn or installed on
-- this computer. One choice of device for both pages (this computer's own
-- pages, screens/themes and screens/templates, are what shows when it's
-- this one: unchanged). A server that doesn't grant settings says so, with
-- the command to run there, and is asked nothing (settings/peer_area).
-- Installing, allowing and signing a template stay on this computer, so
-- the page has none of those for a server.
--
--   M.device()           the server shown (nil: this computer), followed live
--   M.shown()            the same, for the picker's `shown`
--   M.choose(id)         shows that device (this computer's id: this computer)
--   M.View(kind)         the page for kind "themes" or "templates"

local ds = require("@ds")
local sessions = require("@account/sessions")
local peer_area = require("@app/screens/settings/peer_area")
local devices_page = require("@app/screens/settings/devices")

local M = {}

type Item = { id: string, name: string, about: string, active: boolean, removable: boolean }

local chosen = ui.state("peer_looks_device", "")
local listing = ui.state("peer_looks_listing", { device = "", kind = "", state = "idle", list = {} :: { Item }, said = "" })
local query = ui.state("peer_looks_query", "")
local tab = ui.state("peer_looks_tab", "all")
local deleting = ui.state("peer_looks_deleting", nil :: { id: string, name: string, device: string, kind: string }?)
local deleting_open = ui.state("peer_looks_deleting_open", false)
-- Answers to an older ask go.
local asked = 0

local AREA = "settings"

-- What the host does, where a test may stand in for it: each a promise
-- (`:after(ok, err)`).
M.host = {
	list = function(device: string, kind: string): any
		return ui.call("peer_looks.list", { device = device, kind = kind })
	end,
	act = function(op: string, device: string, kind: string, id: string): any
		return ui.call("peer_looks." .. op, { device = device, kind = kind, id = id })
	end,
}

function M.device(): string?
	local d = chosen:get()
	return if d ~= "" then d else nil
end

function M.shown(): string?
	local d = chosen:get()
	if d ~= "" then
		return d
	end
	for _, x in sessions.devices:get() or {} do
		if x.here then
			return x.id
		end
	end
	return nil
end

local function device_name(): string
	local d = chosen:get()
	for _, x in sessions.devices:get() or {} do
		if x.id == d then
			return x.name
		end
	end
	return d
end

-- The server's list as items.
local function items(kind: string, reply: any): { Item }
	local out: { Item } = {}
	if kind == "themes" then
		for _, t in (reply and reply.themes) or {} do
			table.insert(out, {
				id = tostring(t.id),
				name = tostring(t.name or t.id),
				about = tostring(t.source or ""),
				active = t.in_use ~= nil and #t.in_use > 0,
				removable = t.editable == true,
			})
		end
	else
		for _, t in (reply and reply.templates) or {} do
			table.insert(out, {
				id = tostring(t.id),
				name = tostring(t.name or t.id),
				about = tostring(t.description or ""),
				active = t.active == true,
				removable = t.builtin ~= true,
			})
		end
	end
	return out
end

local function said(err: any): string
	if type(err) == "table" and err.message then
		return tostring(err.message)
	end
	return tostring(err)
end

local function load(kind: string)
	local d = chosen:peek()
	if d == "" then
		return
	end
	asked += 1
	local mine = asked
	if peer_area.refused(d, AREA) then
		listing:set({ device = d, kind = kind, state = "refused", list = {}, said = "" })
		return
	end
	local before = listing:peek()
	listing:set({ device = d, kind = kind, state = "loading", list = if before.device == d and before.kind == kind then before.list else {}, said = "" })
	local p: any = M.host.list(d, kind)
	if p == nil then
		return
	end
	p:after(function(reply: any)
		if mine == asked then
			listing:set({ device = d, kind = kind, state = "ready", list = items(kind, reply), said = "" })
		end
	end, function(err: any)
		if mine == asked then
			local words = said(err)
			listing:set({ device = d, kind = kind, state = if string.find(words, "offline", 1, true) then "offline" else "failed", list = {}, said = words })
		end
	end)
end

function M.choose(id: string)
	local here = false
	for _, d in sessions.devices:peek() or {} do
		if d.id == id then
			here = d.here
		end
	end
	chosen:set(if here then "" else id)
end

local function act(op: string, kind: string, device: string, id: string, title: string | Message)
	local p: any = M.host.act(op, device, kind, id)
	if p == nil then
		return
	end
	p:after(function(_: any)
		if chosen:peek() == device then
			load(kind)
		end
	end, function(err: any)
		ds.toast.show { title = title, body = said(err), tone = "warning" }
	end)
end

local function row(kind: string): (item: Item, i: number, now: State<any>) -> Node
	return function(item: Item, _i: number, now: State<any>): Node
		return ui.column {
			id = `peer-{kind}-{item.id}`,
			w_full = true,
			ds.Settings.Row {
				label = function(): string
					return now:get().name
				end,
				description = function(): string
					return now:get().about
				end,
				badge = ui.show(function(): boolean
					return now:get().active == true
				end, ds.Badge { ui.t("settings.peer.in_use"), tone = "accent" }),
				truncate = true,
				control = ui.row {
					gap = "$space.2",
					items = "center",
					ui.show(function(): boolean
						return now:get().active ~= true
					end, ds.Button {
						tone = "secondary",
						size = "sm",
						on_click = function()
							act("use", kind, chosen:peek(), item.id, ui.t("settings.peer.couldnt_use", { device = device_name() }))
						end,
						ui.t("settings.peer.use"),
					}),
					if item.removable
						then ds.IconButton {
							icon = "trash-bin-minimalistic",
							label = ui.t("settings.peer.delete_label", { name = item.name }),
							size = "sm",
							tone = "quiet",
							on_click = function()
								deleting:set({ id = item.id, name = item.name, device = chosen:peek(), kind = kind })
								deleting_open:set(true)
							end,
						}
						else nil,
				},
			},
		}
	end
end

-- The page's own words, by kind.
local function words(kind: string): { title: Message, description: Message, search: Message, hint: Message }
	if kind == "themes" then
		return {
			title = ui.t("themes.page.title"),
			description = ui.t("settings.peer.themes_description"),
			search = ui.t("settings.peer.search_themes"),
			hint = ui.t("settings.peer.themes_hint"),
		}
	end
	return {
		title = ui.t("templates.page.title"),
		description = ui.t("settings.peer.templates_description"),
		search = ui.t("settings.peer.search_templates"),
		hint = ui.t("settings.peer.templates_hint"),
	}
end

function M.View(kind: string): Node
	local say = words(kind)
	-- The server's list, again when the device, or its grant, changes (and
	-- as this opens).
	local last_device, last_refused = nil :: string?, false
	ui.effect(function()
		local d = chosen:get()
		local refused = d ~= "" and peer_area.refused(d, AREA)
		if d ~= "" and (d ~= last_device or refused ~= last_refused or listing:peek().kind ~= kind) then
			load(kind)
		end
		last_device, last_refused = d, refused
	end)
	local found = ui.computed(function(): { Item }
		local q = string.lower(query:get())
		local out: { Item } = {}
		for _, it in listing:get().list do
			if q == "" or string.find(string.lower(it.name), q, 1, true) ~= nil then
				table.insert(out, it)
			end
		end
		return out
	end)
	return ds.Settings.Catalog {
		title = say.title,
		description = say.description,
		trail = devices_page.picker(M.shown, M.choose, AREA),
		trail_top = true,
		search = { value = query, placeholder = say.search },
		tabs = {
			value = tab,
			label = say.title,
			options = { {
				value = "all",
				label = ui.t("settings.peer.on_device", { device = device_name() }),
				count = function(): number
					return #listing:get().list
				end,
			} },
		},
		status = function(): string
			local d = chosen:get()
			if d ~= "" and peer_area.refused(d, AREA) then
				return "refused"
			end
			local l = listing:get()
			if l.device ~= d or l.kind ~= kind then
				return "loading"
			end
			return l.state
		end,
		states = {
			refused = function(): Node
				return peer_area.Notice(M.device, AREA)
			end,
			offline = function(): Node
				return ds.Callout {
					tone = "neutral",
					icon = "wifi-off",
					title = function(): string
						return ui.t("plugins.page.device_offline_title", { device = device_name() }):get()
					end,
					body = ui.t("plugins.page.device_offline_body"),
					action = { label = ui.t("plugins.page.try_again"), on_click = function()
						load(kind)
					end },
				}
			end,
			failed = function(): Node
				return ds.Callout {
					tone = "warning",
					title = function(): string
						return ui.t("settings.peer.failed_title", { device = device_name() }):get()
					end,
					body = function(): string
						return listing:get().said
					end,
					action = { label = ui.t("plugins.page.try_again"), on_click = function()
						load(kind)
					end },
				}
			end,
		},
		found = found,
		row = row(kind),
		key = function(it: Item): string
			return it.id
		end,
		none = function(_tab: string): Node
			return ds.EmptyState {
				icon = "gallery",
				title = ui.t("settings.peer.none_title"),
				body = ui.t("settings.peer.none_body"),
				size = "sm",
			}
		end,
		hint = { id = "peer-looks-hint", text = say.hint },
		overlays = {
			ds.Dialog {
				open = deleting_open,
				title = ui.t("settings.peer.delete_title"),
				body = function(): string
					local d = deleting:get()
					return ui.t("settings.peer.delete_body", { name = if d then d.name else "", device = device_name() }):get()
				end,
				tone = "danger",
				confirm = {
					label = ui.t("common.delete"),
					on_click = function()
						local d = deleting:peek()
						deleting:set(nil)
						if d then
							act("delete", d.kind, d.device, d.id, ui.t("settings.peer.couldnt_delete", { device = device_name() }))
						end
					end,
				},
			},
		},
	}
end

return M
