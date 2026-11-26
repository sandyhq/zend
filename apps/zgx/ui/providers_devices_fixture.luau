-- Settings → Providers with paired servers, mounted on its own (no engine in
-- a test host): what tests/providers_devices.test.luau drives. Three
-- computers: this one, a server that grants logins ("Granting", with one
-- agent of its own) and one that doesn't ("AI"). account/agents' target is
-- stubbed as the host answers it (src/agents.rs): a server that doesn't
-- grant logins is not asked and publishes `refused`; the others, their
-- agents. What was asked, in order, is `calls`.

ui.theme.strict("warn")
local ds = require("@ds")
ds.setup()
ds.PrivateText.hidden:set(false)
-- Settings' host calls (the page's settings dialog, its toasts).
require("./settings_stub")
local agents = require("@account/agents")
local sessions = require("@account/sessions")
local registry = require("@account/registry") :: any
registry.load = function() end
local providers = require("@app/screens/settings/providers")

sessions.now:set({
	state = "ready",
	devices = {
		{ id = "mac", name = "MacBook Pro", platform = "macos", here = true, online = true },
		{ id = "ai", name = "AI", platform = "linux", here = false, online = true, paired = true, peer_areas = { "sessions.read" } },
		{ id = "granting", name = "Granting", platform = "linux", here = false, online = true, paired = true, peer_areas = { "logins" } },
	},
	projects = {},
	chats = {},
	archived = {},
	places = { pinned = {}, sections = {} },
	places_ready = true,
} :: any)

local function agent(id: string, name: string, mark: string): any
	return {
		id = id,
		name = name,
		mark = mark,
		installed = true,
		can_install = true,
		enabled = true,
		version = "1.0.0",
		installing = false,
		login = "idle",
		accounts = {},
	}
end

local function publish(device: string?)
	if device == "ai" then
		-- (Nothing asked of a server that doesn't grant logins.)
		agents.now:set({ state = "unavailable", list = {}, target = device, refused = true } :: any)
	elseif device == "granting" then
		agents.now:set({ state = "ready", list = { agent("codex", "Server Codex", "openai") }, target = device } :: any)
	else
		agents.now:set({ state = "ready", list = { agent("claude-code", "Local Claude", "claude") } } :: any)
	end
end
publish(nil)

local said: { string } = {}
local calls = ui.state("calls", "")
local function note(what: string)
	table.insert(said, what)
	calls:set(table.concat(said, ","))
end

local stub = agents :: any
stub.load = function() end
stub.target = function(device: string?)
	note(`target:{device or ""}`)
	publish(device)
end

return ui.scroll {
	w_full = true,
	h_full = true,
	bg = "$background",
	ui.column { w_full = true, px = "$space.6", pt = "$space.4", pb = "$space.8", items = "center", providers.view() },
}
