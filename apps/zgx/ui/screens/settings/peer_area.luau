--!strict
-- A paired server's grants, for Settings' per-device pages (Plugins,
-- Providers): whether the server shown opens an area (`settings`, `logins`,
-- `updates`), and the notice a page shows in place of its list when it
-- doesn't: "<server> doesn't grant <area>" and the command to run on the
-- server. Never an error toast. The host asks nothing of a server that
-- doesn't grant (src/sessions.rs `server_refuses`); this is what the page
-- says. A device's `peer_areas` is what the server said its grants open
-- (nil: it didn't say, so only sessions.* is open); `full` opens every area.

local ds = require("@ds")
local sessions = require("@account/sessions")

local M = {}

-- Whether device `d` (a row of `sessions.devices`) opens `area`: any device
-- that isn't a paired server does.
function M.granted(d: any, area: string): boolean
	if d == nil or not d.paired then
		return true
	end
	local areas = d.peer_areas
	if areas == nil then
		return string.sub(area, 1, 9) == "sessions."
	end
	for _, a in areas do
		if a == area or a == "full" then
			return true
		end
	end
	return false
end

local function device(id: string?): any
	for _, d in sessions.devices:get() or {} do
		if id and d.id == id then
			return d
		end
	end
	return nil
end

-- Whether the device `id` (nil: this one) is a paired server that doesn't
-- grant `area` (followed live: read inside a function or computed).
function M.refused(id: string?, area: string): boolean
	if id == nil or id == "" then
		return false
	end
	return not M.granted(device(id), area)
end

-- The command to run on the server to open `area` to this device.
function M.command(area: string): string
	local here = "this-device"
	for _, d in sessions.devices:get() or {} do
		if d.here then
			here = d.name
		end
	end
	return `zend peer grant "{here}" {area}`
end

-- The notice for the server `id()` and `area` (inside a function: it follows
-- the devices).
function M.Notice(id: () -> string?, area: string): Node
	local function name(): string
		local d = device(id())
		return if d then d.name else ""
	end
	return ui.column {
		w_full = true,
		gap = "$space.2",
		ds.Callout {
			tone = "info",
			icon = "lock-keyhole-unlocked",
			title = function(): string
				return ui.t("settings.peer.not_granted_title", { device = name(), area = area }):get()
			end,
			body = function(): string
				return ui.t("settings.peer.not_granted_body", { device = name() }):get()
			end,
		},
		ui.text {
			M.command(area),
			font = "$font.mono",
			text_size = "$font.size.sm",
			color = "$text.secondary",
			selectable = true,
			px = "$space.4",
		},
	}
end

return M
