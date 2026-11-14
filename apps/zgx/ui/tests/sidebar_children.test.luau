-- The sidebar nests child sessions (a session another started: `parent`,
-- child_kind "session") under their parent, one step in, the parent's row
-- holding the button that folds them; "Nest child sessions" in the view
-- menu's Show lists them flat instead, each its own row naming its parent.
-- A folded parent still lists a child in a state while "Active when
-- collapsed" is on; a child whose parent isn't in its list (archived)
-- stands on its own either way. On the shell in ui/shell_fixture.luau
-- (`child_sessions`, `child_of_archived`).

local SIDE = "#shell-sidebar"
local K = "screens/shell/sidebar.luau::"

local function open(first: string?): any
	local app = mount("shell_fixture.luau")
	app:resize(1200, 900)
	app:frames(30)
	-- (Regular density: the rows' second lines.)
	app:set_state(K .. "shell_sidebar_compact", false)
	app:frames(20)
	app:command(first or "child_sessions")
	app:frames(30)
	return app
end

local function order(app: any): string
	return table.concat((app:state("shell_sidebar_order") :: any) or {}, ",")
end

local function shows(app: any, label: string): boolean
	return app:has(`{SIDE} >> role:button "{label}"`)
end

-- Where a session's row starts its first glyph (how far in it stands).
local function lead(app: any, title: string): number
	local seen = false
	for line in app:tree(SIDE):gmatch("[^\n]+") do
		if line:find(` button "{title}"`, 1, true) then
			seen = true
		elseif seen then
			local x = line:match(" (%-?%d+),%-?%d+ %d+x%d+")
			if x and (line:find(" img ", 1, true) or line:find(' text "', 1, true)) then
				return tonumber(x) :: number
			end
		end
	end
	error(`no "{title}" in the sidebar`)
end

-- A session's second line (not its title, its corner's time, or a tile's letter).
local function second(app: any, title: string): string
	local out = {}
	for line in app:tree(`{SIDE} >> role:button "{title}"`):gmatch("[^\n]+") do
		local words = line:match(' text "([^"]*)"')
		if words and not line:find(" hidden", 1, true) and words ~= title and not words:match("^%d+[smhdw]$") and words ~= "now" and not words:match("^%u$") then
			table.insert(out, words)
		end
	end
	return table.concat(out, " ")
end

test("child sessions nest under their parent by default, one step in", function()
	local app = open()
	-- (Newest first: Profile the build is the later of the two.)
	expect(order(app)).to_be("a1,c2,c1,a2,a3,b1,b2")
	expect(lead(app, "Profile the build") > lead(app, "Fix the build")).to_be(true)
	expect(lead(app, "Chase the flake")).to_be(lead(app, "Profile the build"))
	expect(lead(app, "Write the docs")).to_be(lead(app, "Fix the build"))
	-- (Under their parent, a child's second line is what a session's is, not a parent's name.)
	expect(second(app, "Chase the flake")).to_be("MacBook Pro")
end)

test("the parent's button folds its child sessions away and shows them again", function()
	local app = open()
	-- (Only the parent has one.)
	expect(app:has(`{SIDE} >> role:button "Fold child sessions"`)).to_be(true)
	app:set_state(K .. "shell_sidebar_keep_active", false)
	app:frames(20)
	app:click(`{SIDE} >> role:button "Fold child sessions"`)
	app:frames(60)
	expect(order(app)).to_be("a1,a2,a3,b1,b2")
	expect(shows(app, "Chase the flake")).to_be(false)
	expect(shows(app, "Fix the build")).to_be(true)
	-- (Kept with the sidebar's folds, so it comes back as it was.)
	expect(((app:state("shell_sidebar_folded") :: any) or {})["kids:a1"]).to_be(true)
	app:click(`{SIDE} >> role:button "Show child sessions"`)
	app:frames(60)
	expect(order(app)).to_be("a1,c2,c1,a2,a3,b1,b2")
	expect(shows(app, "Chase the flake")).to_be(true)
end)

test("a folded parent still lists a child that is working while Active when collapsed is on", function()
	local app = open()
	app:click(`{SIDE} >> role:button "Fold child sessions"`)
	app:frames(60)
	-- Profile the build is working: it stays; Chase the flake goes.
	expect(order(app)).to_be("a1,c2,a2,a3,b1,b2")
	expect(shows(app, "Profile the build")).to_be(true)
	expect(shows(app, "Chase the flake")).to_be(false)
	-- Off, nothing stays.
	app:set_state(K .. "shell_sidebar_keep_active", false)
	app:frames(40)
	expect(order(app)).to_be("a1,a2,a3,b1,b2")
end)

test("the view menu's Nest child sessions off lists them among the sessions, each naming its parent", function()
	local app = open()
	app:click(`{SIDE} >> role:button "Sidebar view options"`)
	app:frames(20)
	expect(app:has('role:checkbox "Nest child sessions"')).to_be(true)
	app:click('role:checkbox "Nest child sessions"')
	app:frames(30)
	expect(app:state("shell_sidebar_nest_children")).to_be(false)
	app:key("Escape")
	app:frames(20)
	-- Flat: by when they were last at it, each a row like any other.
	expect(order(app)).to_be("a1,c2,a2,a3,c1,b1,b2")
	expect(lead(app, "Profile the build")).to_be(lead(app, "Fix the build"))
	expect(lead(app, "Chase the flake")).to_be(lead(app, "Fix the build"))
	expect(second(app, "Chase the flake")).to_be("Child of Fix the build")
	expect(second(app, "Profile the build")).to_be("Child of Fix the build")
	expect(app:has(`{SIDE} >> role:button "Fold child sessions"`)).to_be(false)
	-- And on again.
	app:set_state(K .. "shell_sidebar_nest_children", true)
	app:frames(30)
	expect(order(app)).to_be("a1,c2,c1,a2,a3,b1,b2")
end)

test("a child session whose parent is archived stands flat, naming it, nested or not", function()
	local app = open("child_of_archived")
	-- (After Tidy the tests: 3 hours old.)
	expect(order(app)).to_be("a1,a2,a3,c3,b1,b2")
	expect(lead(app, "Revive the experiment")).to_be(lead(app, "Fix the build"))
	expect(second(app, "Revive the experiment")).to_be("Child of Old experiment")
	app:set_state(K .. "shell_sidebar_nest_children", false)
	app:frames(30)
	expect(second(app, "Revive the experiment")).to_be("Child of Old experiment")
	expect(lead(app, "Revive the experiment")).to_be(lead(app, "Fix the build"))
end)

-- A change to the sessions as the engine would make it.
local function change(app: any, f: (any) -> ())
	local function copy(v: any): any
		if type(v) ~= "table" then
			return v
		end
		local out = {}
		for k, value in v do
			out[k] = copy(value)
		end
		return out
	end
	local now = copy(app:state("sessions"))
	f(now)
	app:set_state("sessions", now)
	app:frames(50)
end

test("a flat child names its parent in compact density too, on a row tall enough", function()
	local app = open()
	app:set_state(K .. "shell_sidebar_compact", true)
	app:set_state(K .. "shell_sidebar_nest_children", false)
	app:frames(40)
	expect(second(app, "Chase the flake")).to_be("Child of Fix the build")
	local tree = app:tree(SIDE .. ' >> role:button "Chase the flake"')
	local h = tree:match("button \"Chase the flake\" %-?%d+,%-?%d+ %d+x(%d+)")
	expect(tonumber(h) :: number > 30).to_be(true)
	-- (A session of no parent stays one line.)
	local plain = app:tree(SIDE .. ' >> role:button "Fix the build"')
	expect(tonumber(plain:match("button \"Fix the build\" %-?%d+,%-?%d+ %d+x(%d+)")) :: number <= 30).to_be(true)
end)

test("a child's fold changed from outside moves its rows, not only the arrow", function()
	local app = open()
	app:set_state(K .. "shell_sidebar_keep_active", false)
	app:set_state(K .. "shell_sidebar_folded", { ["kids:a1"] = true })
	app:frames(60)
	expect(order(app)).to_be("a1,a2,a3,b1,b2")
	app:set_state(K .. "shell_sidebar_folded", {})
	app:frames(60)
	expect(order(app)).to_be("a1,c2,c1,a2,a3,b1,b2")
end)

test("a parent put by hand in Pinned or a section lists its children flat, once each", function()
	local app = open()
	change(app, function(now)
		now.places.pinned = { "a1" }
	end)
	expect(order(app)).to_be("a1,c2,a2,a3,c1,b1,b2")
	expect(second(app, "Chase the flake")).to_be("Child of Fix the build")
	change(app, function(now)
		now.places.pinned = {}
		now.places.sections = { { id = "sec1", name = "Work", sessions = { "a1" } } }
	end)
	expect(order(app)).to_be("a1,c2,a2,a3,c1,b1,b2")
end)

test("clicking a nested child opens its own page and panel", function()
	local app = open()
	app:click(SIDE .. ' >> role:button "Chase the flake"')
	app:frames(30)
	expect(app:state("shell_selected")).to_be("c1")
	expect(app:state("fixture_open_chat")).to_be("c1")
	app:click('#titlebar >> role:button "Panel"')
	app:frames(50)
	expect(((app:state("shell_panel_open") :: any) or {}).c1).to_be(true)
	expect(((app:state("shell_panel_open") :: any) or {}).a1).to_be_nil()
	expect(app:error()).to_be_nil()
end)

-- Activity: one list; the children's parent and they quiet, so on it together.
local function activity(app: any)
	app:set_state("screens/shell/activity.luau::shell_sidebar_activity", true)
	change(app, function(now)
		for _, c in now.chats do
			if c.id == "a1" or c.id == "c2" then
				c.status = nil
			end
		end
	end)
	app:frames(40)
end

test("Activity nests a child under its parent while Nest child sessions is on, and lists it flat when off", function()
	local app = open()
	activity(app)
	expect(shows(app, "Fold child sessions")).to_be(true)
	expect(lead(app, "Chase the flake") > lead(app, "Fix the build")).to_be(true)
	expect(second(app, "Chase the flake")).to_be("Atlas · MacBook Pro")
	app:set_state(K .. "shell_sidebar_nest_children", false)
	app:frames(40)
	expect(shows(app, "Fold child sessions")).to_be(false)
	expect(second(app, "Chase the flake")).to_be("Child of Fix the build")
	expect(lead(app, "Chase the flake")).to_be(lead(app, "Fix the build"))
end)

test("a parent and its child in the same section nest there, with the fold button", function()
	local app = open()
	change(app, function(now)
		now.places.sections = { { id = "sec1", name = "Work", sessions = { "a1", "c1" } } }
	end)
	expect(shows(app, "Fold child sessions")).to_be(true)
	expect(lead(app, "Chase the flake") > lead(app, "Fix the build")).to_be(true)
	app:click(SIDE .. ' >> role:button "Fold child sessions"')
	app:frames(60)
	expect(shows(app, "Chase the flake")).to_be(false)
	app:set_state(K .. "shell_sidebar_nest_children", false)
	app:frames(40)
	expect(second(app, "Chase the flake")).to_be("Child of Fix the build")
end)

test("a child put in another section than its parent stands flat, naming it", function()
	local app = open()
	change(app, function(now)
		now.places.sections = {
			{ id = "sec1", name = "Work", sessions = { "a1" } },
			{ id = "sec2", name = "Later", sessions = { "c1" } },
		}
	end)
	expect(second(app, "Chase the flake")).to_be("Child of Fix the build")
	expect(shows(app, "Fold child sessions")).to_be(false)
end)

test("a pinned parent and child stay unnested with the toggle on", function()
	local app = open()
	change(app, function(now)
		now.places.pinned = { "a1", "c1" }
	end)
	expect(shows(app, "Fold child sessions")).to_be(false)
	expect(second(app, "Chase the flake")).to_be("Child of Fix the build")
	expect(lead(app, "Chase the flake")).to_be(lead(app, "Fix the build"))
end)
