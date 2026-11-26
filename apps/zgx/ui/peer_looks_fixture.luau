-- Settings → Themes and → Templates with paired servers, mounted on their own
-- (no host in a test): what tests/peer_looks.test.luau drives. Three
-- computers: this one, a server that grants settings ("Granting", with a
-- theme and a template of its own) and one that doesn't ("AI"). What the
-- pages ask of the host is stubbed to note the call (`calls`) and answer as
-- the engine would; this computer's own lists are src/themes.rs's and
-- plugins/live.rs's, here a few made-up ones.

ui.theme.strict("warn")
local ds = require("@ds")
ds.setup()
local sessions = require("@account/sessions")
local palettes = require("@ds/palettes")
-- (No window chrome to restyle in a test host, as shell_fixture says.)
local actions = require("@app/screens/onboarding/actions");
(actions :: any).apply_scheme = function() end
local templates = require("@app/screens/templates/state")
local themes_page = require("@app/screens/themes/page")
local templates_page = require("@app/screens/templates/page")
local looks = require("@app/screens/settings/peer_looks")

sessions.now:set({
	state = "ready",
	devices = {
		{ id = "mac", name = "MacBook Pro", platform = "macos", here = true, online = true },
		{ id = "ai", name = "AI", platform = "linux", here = false, online = true, paired = true, peer_areas = { "sessions.read" } },
		{ id = "granting", name = "Granting", platform = "linux", here = false, online = true, paired = true, peer_areas = { "settings" } },
	},
	projects = {},
	chats = {},
	archived = {},
	places = { pinned = {}, sections = {} },
	places_ready = true,
} :: any)

palettes.library():set({ state = "ready", list = {}, entries = {}, chosen = {} } :: any)
templates.now:set({
	state = "ready",
	active = "zend",
	list = { { id = "zend", name = "Zend", builtin = true, active = true, status = "ok" } },
} :: any)

-- (This computer's own sources aren't looked up in a test host.)
;(templates :: any).check_updates = function()
	return nil
end

local said: { string } = {}
local calls = ui.state("calls", "")
local function note(what: string)
	table.insert(said, what)
	calls:set(table.concat(said, ","))
end

local THEMES = {
	{ id = "nord", name = "Nord", source = "built in", editable = false, in_use = {} },
	{ id = "custom-ink", name = "Server Ink", source = "yours", editable = true, in_use = { "dark" } },
}
local TEMPLATES = {
	{ id = "zend", name = "Zend", description = "The default interface.", builtin = true, active = false },
	{ id = "ops-board", name = "Ops board", description = "Dense, for the server.", builtin = false, active = true },
}

-- A promise settled at once, as a host's does later.
local function settled(value: any): any
	return {
		after = function(_self: any, done: (any) -> (), _failed: ((any) -> ())?)
			done(value)
		end,
	}
end

local host = looks.host
host.list = function(device: string, kind: string): any
	note(`list:{kind}@{device}`)
	if kind == "themes" then
		return settled({ themes = THEMES })
	end
	return settled({ templates = TEMPLATES })
end
host.act = function(op: string, device: string, kind: string, id: string): any
	note(`{op}:{kind}:{id}@{device}`)
	if op == "use" and kind == "templates" then
		for _, t in TEMPLATES do
			t.active = t.id == id
		end
	elseif op == "delete" then
		local list: any = if kind == "themes" then THEMES else TEMPLATES
		for k, t in list do
			if t.id == id then
				table.remove(list, k)
				break
			end
		end
	end
	return settled({})
end

local page = ui.state("page", "themes")

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
		ui.presence(function(): string
			return page:get()
		end, function(which: string): Node
			if which == "templates" then
				return templates_page.View()
			end
			return themes_page.View()
		end, { w_full = true, items = "center" }),
	},
}
