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
