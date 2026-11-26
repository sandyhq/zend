-- Settings → Devices: the computers your sessions sync across, the
-- shipping app's Devices (crates/ui settings/devices.rs): this one first,
-- each with its system, the Zend it runs, and whether it's there now or
-- when it last was. Each row's ⋯ renames it in place (every device then
-- shows the name) and removes another one from your account: gone from the
-- list everywhere and never dialed again, its sessions and projects kept
-- (the engine's `removeDevice`). Not this device (its own Zend writes it
-- back as it starts), and not one online now (its Zend is running, maybe
-- with sessions working, and they'd drop out from under it): Remove says
-- why instead. Cloud's machines aren't listed (they come and go with their
-- sessions).
--
-- The list is the engine's (account/sessions `now.devices`), a keyed
-- `Settings.Each`, so a device coming online, renamed, or arriving changes
-- its row alone. Before the engine answers, skeleton rows hold its place;
-- with no engine, a callout says so.
--
-- Under it, connecting a phone to this computer (src/phone_link.rs): on a
-- computer, "Show code" turns on its phone link (its engine listening on
-- its tailnet address too, crates/engine/src/phone_link.rs) and draws a QR
-- code with that address (its name or its IP, as chosen here), the port
-- and the link's key, for a minute or until Hide (no guess at which device
-- used it: a join can't be tied to the code). A switch turns the link off
-- again (every phone cut), New key cuts the phones holding the old one;
-- with no tailnet it says so and nothing listens. On the phone (Android), the view
-- of phone_join.luau: "Scan code" reads it with the camera, or its text is
-- pasted, and the phone's engine starts again on it.

local ds = require("@ds")
local Menu = require("@ds/menu")
local locale = require("@ds/locale")
local toast = require("@ds/toast")
local status = require("@ds/status")
local sessions = require("@account/sessions")
local peer_area = require("@app/screens/settings/peer_area")
local search = require("@app/screens/settings/search")
local rename = require("@app/screens/settings/rename_row")
local host = require("@app/screens/settings/host")
local phone_join = require("@app/screens/settings/phone_join")

local M = {}

M.ID = "devices"
M.TITLE = "Devices"

search.index(M.ID, ui.t("settings.devices.title"), ui.t("settings.devices.terms"), {
	{ key = "devices", title = ui.t("settings.devices.yours.label"), terms = ui.t("settings.devices.yours.terms") },
	{ key = "devices", title = ui.t("settings.devices.rename_one.label"), terms = ui.t("settings.devices.rename_one.terms") },
	{ key = "devices", title = ui.t("settings.devices.remove_one.label"), terms = ui.t("settings.devices.remove_one.terms") },
	{ key = "phone", title = ui.t("settings.devices.phone.title"), terms = ui.t("settings.devices.phone.terms") },
})

local PLATFORMS: { [string]: string } = {
	macos = "macOS",
	linux = "Linux",
	windows = "Windows",
	ios = "iOS",
	android = "Android",
}

-- What the line under a device's name says: its system, its Zend, and
-- whether it's there (in the locale shown: read where it's drawn).
function M.describe(d: sessions.Device, now: number): string
	local parts = { PLATFORMS[d.platform] or d.platform }
	if d.version then
		table.insert(parts, `v{d.version}`)
	end
	-- (This device says so in its badge, as the gallery's device rows do.)
	if d.here then
		return table.concat(parts, " · ")
	elseif d.online then
		table.insert(parts, ui.t("settings.devices.online"):get())
	elseif d.seen then
		table.insert(parts, ui.t("settings.devices.last_seen", { ago = status.ago(math.max(0, now - d.seen)) }):get())
	else
		table.insert(parts, ui.t("settings.devices.offline"):get())
	end
	return table.concat(parts, " · ")
end


-- Renaming: which device (its id; "" none) and the name being typed.
-- Removing: which one is asked about, and whether that's on its way.
local renaming = ui.state("settings_device_renaming", "")
local new_name = ui.state("settings_device_name", "")
local removing = ui.state("settings_device_removing", "")
-- (Its name as it was asked about: the dialog keeps it while it leaves, the
-- device gone by then.)
local removing_name = ui.state("settings_device_removing_name", "")
local removing_open = ui.state("settings_device_removing_open", false)
local removing_busy = ui.state("settings_device_removing_busy", false)
-- Names saved and not yet back from the engine, by device: shown meanwhile,
-- so a rename never flickers back to the old name.
local pending = ui.state("settings_device_pending_names", {} :: { [string]: string })

local function device(id: string): sessions.Device?
	for _, d in sessions.devices:get() or {} do
		if d.id == id then
			return d
		end
	end
	return nil
end

local function trimmed(s: string): string
	return string.match(s, "^%s*(.-)%s*$") or ""
end

local function name_of(d: sessions.Device): string
	return pending:get()[d.id] or d.name
end

local function set_pending(id: string, name: string?)
	pending:set_key(id, name)
end

-- What the engine sends back settles a pending name.
ui.effect(function()
	local waiting = pending:get()
	for id, name in waiting do
		local d = device(id)
		if not d or d.name == name then
			set_pending(id, nil)
		end
	end
end)

-- Why a change didn't go, in words (an outcome word from the host).
local function why(err: any): string
	local word = if type(err) == "string" then err elseif type(err) == "table" or type(err) == "userdata" then err.message else nil
	if word == "offline" then
		return ui.t("settings.engine.offline"):get()
	elseif word == "online" then
		return ui.t("settings.devices.online_now"):get()
	elseif word == "outdated" then
		return ui.t("settings.engine.outdated"):get()
	end
	return ui.t("settings.devices.try_later"):get()
end

local function save_rename(id: string)
	local name = trimmed(new_name:peek())
	renaming:set("")
	local d = device(id)
	if not d or name == "" or name == name_of(d) then
		return
	end
	set_pending(id, name)
	sessions.rename_device(id, name):after(function() end, function(err: any)
		set_pending(id, nil)
		toast.show { title = ui.t("settings.devices.couldnt_rename"):get(), body = why(err), tone = "danger" }
	end)
end

local function remove()
	local id = removing:peek()
	if id == "" or removing_busy:peek() then
		return
	end
	removing_busy:set(true)
	sessions.remove_device(id):after(function()
		ui.batch(function()
			removing_busy:set(false)
			removing_open:set(false)
		end)
	end, function(err: any)
		ui.batch(function()
			removing_busy:set(false)
			removing_open:set(false)
		end)
		toast.show { title = ui.t("settings.devices.couldnt_remove"):get(), body = why(err), tone = "danger" }
	end)
end

-- A device's glyph, by its system (Settings' pages draw devices with it).
function M.icon(platform: string): any
	return if platform == "linux" then "remote-server" elseif platform == "ios" or platform == "android" then "smartphone" else "laptop"
end
local icon_of = M.icon

-- Which computer a per-device page shows (Providers, Plugins): a choice at
-- its title's end once there's more than this one, this one first, one
-- that's away saying so. `shown` is the device shown (nil: this one);
-- choosing one calls `choose` with its id. Built once: its options follow
-- the devices as they come, go and are renamed, and the choice follows
-- `shown`.
-- `area`: the paired-server grant the page needs (settings, logins,
-- updates); a server that doesn't grant it is listed too, as "Not granted"
-- (choosing it says so on the page, with the command to run there).
function M.picker(shown: () -> string?, choose: (id: string) -> (), area: string?): Node
	local function computers(): { sessions.Device }
		return sessions.computers(sessions.devices:get(), true)
	end
	local function here(): string
		for _, d in sessions.devices:get() or {} do
			if d.here then
				return d.id
			end
		end
		return ""
	end
	return ui.show(
		function(): boolean
			return #computers() > 1
		end,
		ui.component(function(): Node
			local chosen = ui.use_state(shown() or here())
			ui.effect(function()
				local id = shown() or here()
				if id ~= chosen:peek() then
					chosen:set(id)
				end
			end)
			return ds.Select {
				value = chosen,
				name = ui.t("settings.providers.device"),
				size = "sm",
				w = 200,
				options = function(): { any }
					local options = {}
					for _, d in computers() do
						table.insert(options, {
							value = d.id,
							label = d.name,
							icon = icon_of(d.platform),
							description = if area and not peer_area.granted(d, area)
								then ui.t("settings.peer.not_granted_option")
								elseif not d.here and d.online == false then ui.t("settings.devices.offline")
								else nil,
						})
					end
					return options
				end,
				on_change = choose,
			}
		end)({})
	)
end

-- The page's clock, which ticks the "last seen" lines over once a minute
-- while the page shows (M.view makes it; its rows read it).
local clock: State<number> = ui.state("settings_devices_clock", 0)

-- A device's row: its name (a field in its place while renaming), its
-- system and whether it's there, and its ⋯.
local function device_row(_d: sessions.Device, _i: number, d: State<sessions.Device>): Node
	local id = d:peek().id
	local menu_open = ui.use_state(false)
	local trigger = ui.ref()
	local function editing(): boolean
		return renaming:get() == id
	end
	-- (Back on the ⋯ once a rename is done, so the keyboard carries on.)
	local function refocus()
		ui.after(0, function()
			local el = trigger:peek()
			if el then
				el:focus()
			end
		end)
	end
	local more = ds.IconButton {
		icon = "more-horizontal",
		label = ui.t("settings.devices.more_for", { name = d:peek().name }),
		size = "md",
		tone = "quiet",
		on_click = function()
			menu_open:set(not menu_open:peek())
		end,
	}
	local function online(): boolean
		return d:get().online == true
	end
	local remove_row = Menu.Row {
		icon = "trash-bin-minimalistic",
		label = locale.s("settings.devices.remove"),
		danger = true,
		on_click = function()
			menu_open:set(false)
			removing:set(id)
			removing_name:set(name_of(d:peek()))
			removing_open:set(true)
		end,
	}
	-- (Online now: there, saying why it can't go yet.)
	local remove_later = Menu.Row {
		icon = "trash-bin-minimalistic",
		label = locale.s("settings.devices.remove"),
		description = locale.s("settings.devices.remove_online"),
		disabled = true,
	}
	local menu = ui.menu(Menu.panel {
		open = menu_open,
		anchor = more,
		placement = { side = "bottom", align = "end", offset = 4 },
		w = 232,
		Menu.Group {
			Menu.Row {
				icon = "pen",
				label = locale.s("settings.devices.rename"),
				on_click = function()
					menu_open:set(false)
					new_name:set(name_of(d:peek()))
					renaming:set(id)
				end,
			},
		},
		-- (Never this device: its own Zend writes it back as it starts.)
		if d:peek().here
			then nil
			else ui.column {
				w_full = true,
				Menu.Rule {},
				Menu.Group {
					ui.show(function(): boolean
						return not online()
					end, remove_row),
					ui.show(online, remove_later),
				},
			},
	})
	local icon = icon_of(d:peek().platform)
	return rename.Row {
		row = ds.Settings.Row {
			label = function(): string
				return name_of(d:get())
			end,
			description = function(): string
				return M.describe(d:get(), clock:get())
			end,
			lead = rename.lead(icon),
			badge = if d:peek().here then ds.Badge { ui.t("settings.devices.this_device"), size = "sm" } else nil,
			-- one line: a list row: its words fade at the end, its controls stay beside them
			truncate = true,
			control = ui.div { ref = trigger, menu },
		},
		lead = function(): Node
			return rename.lead(icon)
		end,
		editing = editing,
		value = new_name,
		name = ui.t("settings.devices.name_of", { name = d:peek().name }),
		can_save = function(): boolean
			return trimmed(new_name:get()) ~= ""
		end,
		on_save = function()
			save_rename(id)
			refocus()
		end,
		on_cancel = function()
			renaming:set("")
			refocus()
		end,
	}
end

-- What the list shows: the devices, or (none yet) whether they're on their
-- way, there's no engine to ask, or there are none.
local function list_kind(list: State<{ sessions.Device }>): string
	if #list:get() > 0 then
		return "devices"
	end
	local s = sessions.state:get()
	return if s == "offline" then "offline" elseif s == "ready" then "none" else "loading"
end


-- Connecting a phone ------------------------------------------------------

-- How long the code shows, in seconds (a fixture shortens it).
M.SHOWN_FOR = 60

-- The code shown (its QR's SVG; nil: none), why it can't be ("" none), the
-- seconds it has left, and which showing this is (a hide meant for an
-- earlier one does nothing).
local code_svg = ui.state("settings_phone_code", nil :: string?)
local code_error = ui.state("settings_phone_code_error", "")
local code_left = ui.state("settings_phone_code_left", 0)
local code_turn = ui.state("settings_phone_code_turn", 0)
-- The phone link as the host last said: on, what the code holds ("name"
-- or "ip"), this computer's tailnet name and IP, and the port.
local link_on = ui.state("settings_phone_link_on", false)
local link_use = ui.state("settings_phone_link_use", "name")
local link_name = ui.state("settings_phone_link_name", "")
local link_ip = ui.state("settings_phone_link_ip", "")
local link_port = ui.state("settings_phone_link_port", 0)
-- The choice the host has (a change of link_use not yet sent is the
-- person's).
local link_sent_use = "name"

local function took_status(answer: any)
	if type(answer) ~= "table" and type(answer) ~= "userdata" then
		return
	end
	ui.batch(function()
		link_on:set(answer.on == true)
		local use = if answer.use == "ip" then "ip" else "name"
		link_sent_use = use
		link_use:set(use)
		link_name:set(answer.name or "")
		link_ip:set(answer.ip or "")
		link_port:set(answer.port or 0)
	end)
end

local function error_word(err: any): string
	return if type(err) == "table" or type(err) == "userdata" then err.message or "failed" else tostring(err)
end

local function hide_code()
	ui.batch(function()
		code_svg:set(nil)
		code_left:set(0)
		code_turn:set(code_turn:peek() + 1)
	end)
end

local function show_code()
	local turn = code_turn:peek() + 1
	code_turn:set(turn)
	code_error:set("")
	host.call("phone.connect_code", {}):after(function(answer: any)
		if code_turn:peek() ~= turn then
			return
		end
		took_status(answer)
		ui.batch(function()
			code_left:set(M.SHOWN_FOR)
			code_svg:set(answer.svg)
		end)
		ui.after(M.SHOWN_FOR * 1000, function()
			if code_turn:peek() == turn then
				hide_code()
			end
		end)
	end, function(err: any)
		if code_turn:peek() == turn then
			code_error:set(error_word(err))
		end
	end)
end

-- The link's switch, what the code holds, and a new key: each the host's
-- answer taken as the link's state (a code shown hides as the link goes
-- off or its key changes, and is drawn again for a new choice).
local function set_link(params: any)
	host.call("phone.link_set", params):after(function(answer: any)
		took_status(answer)
		if not link_on:peek() then
			hide_code()
		elseif params.use ~= nil and code_svg:peek() ~= nil then
			show_code()
		end
	end, function(err: any)
		link_on:set(false)
		code_error:set(error_word(err))
	end)
end

local function new_key()
	host.call("phone.link_new_key", {}):after(function(answer: any)
		took_status(answer)
		hide_code()
		toast.show { title = ui.t("settings.devices.phone.new_key_done"):get(), tone = "success" }
	end, function(err: any)
		code_error:set(error_word(err))
	end)
end

local function link_rows(): Node
	host.call("phone.link_status", {}):after(took_status, function() end)
	ui.effect(function()
		local use = link_use:get()
		if use ~= link_sent_use then
			link_sent_use = use
			set_link({ use = use })
		end
	end)
	return ds.Settings.Group {
		ds.Settings.Row {
			label = ui.t("settings.devices.phone.link_label"),
			description = function(): string
				if not link_on:get() then
					return ui.t("settings.devices.phone.link_off"):get()
				end
				local at = if link_use:get() == "ip" then link_ip:get() else link_name:get()
				return ui.t("settings.devices.phone.link_on", { at = if at ~= "" then at else link_ip:get(), port = link_port:get() }):get()
			end,
			control = ds.Switch {
				value = link_on,
				label = ui.t("settings.devices.phone.link_label"),
				on_change = function(on: boolean)
					set_link({ on = on })
				end,
			},
		},
		ds.Settings.Row {
			label = ui.t("settings.devices.phone.use_label"),
			description = function(): string
				local name, ip = link_name:get(), link_ip:get()
				if name == "" and ip == "" then
					return ui.t("settings.devices.phone.use_unknown"):get()
				end
				return ui.t("settings.devices.phone.use_hint", { name = if name ~= "" then name else "–", ip = if ip ~= "" then ip else "–" }):get()
			end,
			control = ds.Segmented {
				value = link_use,
				id = "phone-link-use",
				label = ui.t("settings.devices.phone.use_label"),
				options = {
					{ value = "name", label = ui.t("settings.devices.phone.use_name") },
					{ value = "ip", label = ui.t("settings.devices.phone.use_ip") },
				} :: any,
			},
		},
		ds.Settings.Row {
			label = ui.t("settings.devices.phone.new_key_label"),
			description = ui.t("settings.devices.phone.new_key_hint"),
			control = ds.Button {
				ui.t("settings.devices.phone.new_key"),
				tone = "secondary",
				size = "sm",
				on_click = new_key,
			},
		},
	}
end

local function code_view(): Node
	ui.every(1000, function()
		if code_svg:peek() == nil then
			return false
		end
		code_left:set(math.max(0, code_left:peek() - 1))
		return true
	end)
	return ui.column {
		w_full = true,
		items = "center",
		gap = "$space.3",
		py = "$space.4",
		ui.svg {
			code = code_svg:peek() :: string,
			id = "phone-code",
			label = ui.t("settings.devices.phone.code_name"),
			w = 240,
			h = 240,
		},
		ui.text {
			ui.t("settings.devices.phone.scan_steps"),
			text_size = "$font.size.md",
			color = "$text.secondary",
			wrap = true,
			align = "center",
		},
		ui.row {
			gap = "$space.2",
			items = "center",
			ui.text {
				function(): string
					return ui.t("settings.devices.phone.hides_in", { n = code_left:get() }):get()
				end,
				id = "phone-code-left",
				text_size = "$font.size.sm",
				color = "$text.tertiary",
			},
			ds.Button {
				ui.t("settings.devices.phone.hide"),
				tone = "secondary",
				size = "sm",
				on_click = hide_code,
			},
		},
	}
end

local function computer_section(): Node
	return search.anchor(
		"phone",
		ds.Settings.Section {
			title = ui.t("settings.devices.phone.title"),
			description = ui.t("settings.devices.phone.description"),
			ui.presence(function(): string
				if code_svg:get() ~= nil then
					-- (Each showing its own: a new code is drawn anew.)
					return `code{code_turn:get()}`
				elseif code_error:get() ~= "" then
					return "error"
				end
				return "ask"
			end, function(kind: string): Node
				if string.sub(kind, 1, 4) == "code" then
					return ds.Settings.Group { ui.component(code_view)({}) }
				elseif kind == "error" then
					local no_tailnet = code_error:peek() == "no_tailnet"
					return ds.Callout {
						tone = if no_tailnet then "neutral" else "danger",
						title = (if no_tailnet then ui.t("settings.devices.phone.no_tailnet_title") else ui.t("settings.devices.phone.couldnt")) :: any,
						body = (if no_tailnet then ui.t("settings.devices.phone.no_tailnet_body") else ui.t("settings.devices.try_later")) :: any,
						-- (Start Tailscale, or the moment passes: ask again.)
						action = { label = ui.t("settings.devices.phone.retry"), on_click = show_code },
					}
				end
				return ds.Settings.Group {
					ds.Settings.Row {
						label = ui.t("settings.devices.phone.show_label"),
						description = ui.t("settings.devices.phone.show_hint"),
						control = ds.Button {
							ui.t("settings.devices.phone.show"),
							tone = "secondary",
							size = "sm",
							on_click = show_code,
						},
					},
				}
			end, { w_full = true, enter = { opacity = { 0, 1 }, duration = 120 }, exit = { opacity = 0, duration = 60 }, mode = "wait" }),
			ui.component(link_rows)({}),
		}
	)
end

-- On the phone: scanning or pasting the computer's code (phone_join.luau,
-- as onboarding's Connect to a computer has it).
local function phone_section(): Node
	return search.anchor(
		"phone",
		ds.Settings.Section {
			title = ui.t("settings.devices.phone.join_title"),
			description = ui.t("settings.devices.phone.join_description"),
			ui.component(phone_join.view)({}),
		}
	)
end

-- The page's part about phones: joining on the phone, showing the code on
-- a computer.
function M.phone(): Node
	return if ui.shared("platform", ""):peek() == "android" then phone_section() else computer_section()
end

function M.view(): Node
	local list = ui.computed(function(): { sessions.Device }
		return sessions.computers(sessions.devices:get())
	end)
	clock = ui.use_state(os.time())
	ui.every(60000, function()
		clock:set(os.time())
		return true
	end)
	return ui.column {
		w_full = true,
		items = "center",
		ds.Settings.Page {
			title = ui.t("settings.devices.title"),
			description = ui.t("settings.devices.description"),
			search.anchor(
				"devices",
				ds.Settings.Section {
					ui.presence(function(): string
						return list_kind(list)
					end, function(kind: string): Node
						if kind == "loading" then
							-- (Said, not only drawn: the bars are hidden from assistive
							-- technology.)
							return ui.column {
								w_full = true,
								label = ui.t("common.loading"),
								live = "polite",
								ds.Settings.Group {
									ds.SkeletonRow { trail = 28 },
									ds.SkeletonRow { title = 110, line = 180, trail = 28 },
								},
							}
						elseif kind == "offline" then
							return ds.Callout {
								tone = "danger",
								title = ui.t("settings.devices.offline_title") :: any,
								body = ui.t("settings.engine.offline") :: any,
							}
						elseif kind == "none" then
							return ds.EmptyState {
								icon = "laptop",
								title = ui.t("settings.devices.empty_title"),
								body = ui.t("settings.devices.empty_body"),
								size = "sm",
							}
						end
						return ds.Settings.Each(list, device_row, {
							key = function(d: sessions.Device): string
								return d.id
							end,
						})
					end, { w_full = true, enter = { opacity = { 0, 1 }, duration = 120 }, exit = { opacity = 0, duration = 60 }, mode = "wait" }),
				}
			),
			M.phone(),
		},
		ds.Dialog {
			open = removing_open,
			title = (function(): string
				return ui.t("settings.devices.remove_title", { name = removing_name:get() }):get()
			end) :: any,
			body = ui.t("settings.devices.remove_body"),
			tone = "danger",
			size = "sm",
			confirm = {
				label = ui.t("settings.devices.remove_confirm"),
				keep_open = true,
				busy = function(): boolean
					return removing_busy:get()
				end,
				on_click = remove,
			},
		},
	}
end

return M
