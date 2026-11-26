-- Settings → About & updates with paired servers, mounted on its own (no host
-- in a test): what tests/about_devices.test.luau drives. Three computers:
-- this one, a server that grants updates ("Granting", at 0.2.130 with 0.2.131
-- out) and one that doesn't ("AI"). The host's calls are stubbed to note
-- what was asked (`calls`) and answer as src/peer_updates.rs does.

ui.theme.strict("warn")
local ds = require("@ds")
ds.setup()
local sessions = require("@account/sessions")
local host = require("@app/screens/settings/host") :: any
local about = require("@app/screens/settings/about")

sessions.now:set({
	state = "ready",
	devices = {
		{ id = "mac", name = "MacBook Pro", platform = "macos", here = true, online = true },
		{ id = "ai", name = "AI", platform = "linux", here = false, online = true, paired = true, peer_areas = { "sessions.read", "settings" } },
		{ id = "granting", name = "Granting", platform = "linux", here = false, online = true, paired = true, peer_areas = { "updates" } },
	},
	projects = {},
	chats = {},
	archived = {},
	places = { pinned = {}, sections = {} },
	places_ready = true,
} :: any)

about.now:set({ app_version = "0.2.130", engine_version = "0.2.130", available = false } :: any)

local said: { string } = {}
local calls = ui.state("calls", "")
local function note(what: string)
	table.insert(said, what)
	calls:set(table.concat(said, ","))
end

local function settled(value: any): any
	return {
		after = function(_self: any, done: (any) -> (), _failed: ((any) -> ())?)
			done(value)
		end,
	}
end

-- What the host says of a server shown: what its engine reported, or nothing
-- for one that doesn't grant updates (it's asked nothing).
host.invoke = function(name: string, arg: any?)
	note(`{name}:{arg}`)
	if name == "peer_updates.show" then
		local d = arg :: string
		if d == "granting" then
			about.device_now:set({ device = d, state = "ready", current = "0.2.130", latest = "0.2.131", available = true } :: any)
		elseif d == "ai" then
			about.device_now:set({ device = d, state = "refused", available = false } :: any)
		else
			about.device_now:set({ device = "", state = "idle", available = false } :: any)
		end
	end
end
host.call = function(name: string, arg: any?): any
	note(`{name}:{if arg then arg.device else ""}`)
	return settled({ version = "0.2.131" })
end

return ui.scroll {
	w_full = true,
	h_full = true,
	bg = "$background",
	ui.column {
		w_full = true,
		px = "$space.6",
		pt = "$space.4",
		pb = "$space.8",
		items = "center",
		about.view(),
	},
}
