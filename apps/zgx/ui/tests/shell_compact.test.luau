-- A phone-sized window (compact, under fit_policy.COMPACT_BELOW): the
-- sidebar a drawer over the page, the panel a sheet over it, settings' list
-- and page one at a time, the title bar's controls touch targets. On the
-- shell in ui/shell_fixture.luau, 412 x 915 (a large phone), against 1200.

local function open(w: number?, h: number?): any
	local app = mount("shell_fixture.luau")
	app:resize(w or 412, h or 915)
	app:frames(30)
	return app
end

-- Whether a node shows: there, and laid out at a size (a hidden one is 0 x 0).
local function shows(app: any, target: string): boolean
	if not app:has(target) then
		return false
	end
	local w, h = app:tree(target):match("^%s*@%d+[^\n]- [%-%d.]+,[%-%d.]+ ([%d.]+)x([%d.]+)")
	return (tonumber(w) or 0) > 0 and (tonumber(h) or 0) > 0
end

-- A node's box from the tree: x, y, w, h (nil when none matches).
local function box(app: any, target: string): (number?, number?, number?, number?)
	if not app:has(target) then
		return nil, nil, nil, nil
	end
	local x, y, w, h = app:tree(target):match("^%s*@%d+[^\n]- ([%-%d.]+),([%-%d.]+) ([%d.]+)x([%d.]+)")
	return tonumber(x), tonumber(y), tonumber(w), tonumber(h)
end

test("compact: the sidebar is no column beside the page, and the toggle opens it as a drawer", function()
	local app = open()
	local _, _, pw = box(app, "#shell-page")
	expect((pw or 0) >= 400).to_be(true)
	expect(app:has("#shell-drawer-sheet")).to_be(false)
	app:click('#titlebar >> role:button "Toggle sidebar"')
	app:frames(30)
	local x, _, w = box(app, "#shell-drawer-sheet")
	expect(x).to_be(0)
	-- (A strip of the page stays beside it to tap back to.)
	expect((w or 999) <= 412 - 56).to_be(true)
	expect(app:has('#shell-drawer-sheet >> role:button "Fix the build"')).to_be(true)
	expect(app:error()).to_be_nil()
end)

test("compact: Escape, a tap beside it and choosing a chat each close the drawer", function()
	local app = open()
	app:click('#titlebar >> role:button "Toggle sidebar"')
	app:frames(30)
	app:key("Escape")
	app:frames(30)
	expect(app:has("#shell-drawer-sheet")).to_be(false)
	app:click('#titlebar >> role:button "Toggle sidebar"')
	app:frames(30)
	app:click("#shell-drawer-scrim")
	app:frames(30)
	expect(app:has("#shell-drawer-sheet")).to_be(false)
	app:click('#titlebar >> role:button "Toggle sidebar"')
	app:frames(30)
	app:click('#shell-drawer-sheet >> role:button "Fix the build"')
	app:frames(30)
	expect(app:has("#shell-drawer-sheet")).to_be(false)
	expect(app:error()).to_be_nil()
end)

test("compact: a swipe in from the left edge opens the drawer", function()
	local app = open()
	app:drag("#shell-drawer-edge", "#shell-page")
	app:frames(30)
	expect(app:has("#shell-drawer-sheet")).to_be(true)
end)

test("compact: the panel opens over the whole page, as a sheet", function()
	local app = open()
	app:command("shell.session_1")
	app:frames(30)
	app:command("shell.toggle_files")
	app:frames(60)
	local x, _, w = box(app, "#shell-files")
	expect(x ~= nil and x <= 8).to_be(true)
	expect((w or 0) >= 396).to_be(true)
	expect(app:error()).to_be_nil()
end)

test("compact: settings shows its list first, then a page with a way back, and Escape steps back", function()
	local app = open()
	app:command("shell.settings")
	app:frames(30)
	local _, _, nw = box(app, "#settings-nav")
	expect((nw or 0) >= 396).to_be(true)
	expect(shows(app, "#settings-back")).to_be(false)
	app:click('#settings-nav >> "Appearance"')
	app:frames(30)
	expect(shows(app, "#settings-back")).to_be(true)
	expect(shows(app, "#settings-nav")).to_be(false)
	app:key("Escape")
	app:frames(30)
	expect(shows(app, "#settings-back")).to_be(false)
	app:click('#settings-nav >> "Appearance"')
	app:frames(30)
	app:click("#settings-back")
	app:frames(30)
	expect(shows(app, "#settings-nav")).to_be(true)
	app:key("Escape")
	app:frames(30)
	expect(app:has("#shell-settings")).to_be(false)
end)

test("compact: the title bar's controls are 44-point targets, and there's no Forward", function()
	local app = open()
	for _, name in { "Toggle sidebar", "Back", "Panel" } do
		local _, _, w, h = box(app, `#titlebar >> role:button "{name}"`)
		expect(w).to_be(44)
		expect(h).to_be(44)
	end
	expect(app:has('#titlebar >> role:button "Forward"')).to_be(false)
end)

test("wide again: no drawer, the sidebar beside the page, the title bar as it was", function()
	local app = open()
	app:click('#titlebar >> role:button "Toggle sidebar"')
	app:frames(30)
	app:resize(1200, 800)
	app:frames(60)
	expect(app:has("#shell-drawer-sheet")).to_be(false)
	expect(app:has("#shell-drawer-edge")).to_be(false)
	expect(app:has('#titlebar >> role:button "Forward"')).to_be(true)
	local _, _, w = box(app, '#titlebar >> role:button "Back"')
	expect((w or 0) < 44).to_be(true)
	local _, _, sw = box(app, "#sidebar-content")
	expect((sw or 0) >= 242).to_be(true)
end)

test("compact: a palette scope chosen past the card's edge changes what it lists; its Tab hint left out", function()
	local app = open()
	app:command("shell.palette")
	app:frames(30)
	expect(shows(app, '#command-palette >> "Next scope"')).to_be(false)
	expect(shows(app, '#palette-list >> role:option "New chat"')).to_be(true)
	-- (Settings, the last scope, past the card's edge: the row scrolls to it.)
	app:click('#command-palette >> role:button "Settings"')
	app:frames(30)
	expect(shows(app, '#palette-list >> role:option "Settings: Appearance"')).to_be(true)
	expect(shows(app, '#palette-list >> role:option "New chat"')).to_be(false)
	expect(app:error()).to_be_nil()
end)

-- Whether the palette's scope row (SCOPES_H, 38, high) is one that scrolls.
local function scope_row_scrolls(app: any): boolean
	for line in app:tree("#command-palette"):gmatch("[^\n]+") do
		if line:match("^%s*@%d+ scroll %-?[%d.]+,%-?[%d.]+ [%d.]+x38$") then
			return true
		end
	end
	return false
end

test("desktop down to the cutoff: the palette's Tab hint stays at 650", function()
	local app = open(650, 915)
	app:command("shell.palette")
	app:frames(30)
	expect(shows(app, '#command-palette >> "Next scope"')).to_be(true)
end)

test("a palette open as the window turns compact: its scopes scroll, what's typed and chosen kept, and back", function()
	local app = open(1200, 800)
	app:command("shell.palette")
	app:frames(30)
	app:type("#palette-search", "app")
	app:click('#command-palette >> role:button "Settings"')
	app:frames(30)
	expect(shows(app, '#command-palette >> "Next scope"')).to_be(true)
	expect(scope_row_scrolls(app)).to_be(false)
	app:resize(412, 915)
	app:frames(30)
	-- (Compact: the chips in a row that scrolls, no hint.)
	expect(scope_row_scrolls(app)).to_be(true)
	expect(shows(app, '#command-palette >> "Next scope"')).to_be(false)
	expect(app:tree("#palette-search"):find('="app"', 1, true) ~= nil).to_be(true)
	expect(shows(app, '#palette-list >> role:option "Settings: Appearance"')).to_be(true)
	app:resize(1200, 800)
	app:frames(30)
	expect(shows(app, '#command-palette >> "Next scope"')).to_be(true)
	expect(app:tree("#palette-search"):find('="app"', 1, true) ~= nil).to_be(true)
	expect(app:error()).to_be_nil()
end)

test("compact: the title bar's Back in a settings page goes to the list first, then out", function()
	local app = open()
	app:command("shell.settings")
	app:frames(30)
	app:click('#settings-nav >> "Appearance"')
	app:frames(30)
	app:click('#titlebar >> role:button "Back"')
	app:frames(30)
	expect(app:has("#shell-settings")).to_be(true)
	expect(shows(app, "#settings-nav")).to_be(true)
	app:click('#titlebar >> role:button "Back"')
	app:frames(30)
	expect(app:has("#shell-settings")).to_be(false)
end)

test("compact: choosing the chat already open closes the drawer", function()
	local app = open()
	app:command("shell.session_1")
	app:frames(30)
	app:click('#titlebar >> role:button "Toggle sidebar"')
	app:frames(30)
	app:click('#shell-drawer-sheet >> role:button "Fix the build"')
	app:frames(30)
	expect(app:has("#shell-drawer-sheet")).to_be(false)
end)

test("compact: the panel's sheet has no Collapse beside the Panel toggle", function()
	local app = open()
	app:command("shell.session_1")
	app:frames(30)
	app:command("shell.toggle_files")
	app:frames(60)
	expect(app:has("#panel-expand")).to_be(false)
	app:click('#titlebar >> role:button "Panel"')
	app:frames(60)
	local _, _, w = box(app, "#shell-files")
	expect((w or 0) == 0 or not shows(app, "#shell-files")).to_be(true)
end)
