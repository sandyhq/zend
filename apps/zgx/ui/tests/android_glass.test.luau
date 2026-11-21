-- G6: Android's window has no glass to see through, so Settings →
-- Appearance has no Frosted glass switch there and setup's appearance step
-- no Frosted/Opaque choice; the desktop keeps both. On the shell in
-- ui/shell_fixture.luau (settings_stub.on_android: the host's `platform`).

local PAGE = "#settings-page"
local NAV = "#settings-nav"

local function appearance(before: { string }?): any
	local app = mount("shell_fixture.luau")
	app:resize(1200, 900)
	app:frames(30)
	app:command("settings_stub.sign_in")
	for _, c in before or {} do
		app:command(c)
	end
	app:command("shell.settings")
	app:frames(20)
	app:click(`{NAV} >> role:tab "Appearance"`)
	app:frames(30)
	return app
end

test("the desktop's Appearance has the Frosted glass switch", function()
	local app = appearance()
	expect(app:has(`{PAGE} >> "Frosted glass"`)).to_be(true)
	expect(app:has(`{PAGE} >> "Accent color throughout"`)).to_be(true)
end)

test("on Android, Appearance has no Frosted glass switch, and the rows after it stay", function()
	local app = appearance({ "settings_stub.on_android" })
	expect(app:has(`{PAGE} >> "Frosted glass"`)).to_be(false)
	expect(app:has(`{PAGE} >> "Accent color throughout"`)).to_be(true)
end)

-- Setup's picker, as ds.AppearancePicker draws it alone
-- (ui/appearance_picker_fixture.luau).
local function picker(before: { string }?): any
	local app = mount("appearance_picker_fixture.luau")
	app:resize(700, 600)
	app:frames(20)
	for _, c in before or {} do
		app:command(c)
	end
	app:frames(20)
	return app
end

test("setup's appearance step offers Frosted or Opaque on the desktop", function()
	local app = picker()
	expect(app:has('"Frosted"')).to_be(true)
	expect(app:has('"Opaque"')).to_be(true)
end)

test("on Android, setup's appearance step has no Frosted or Opaque, and keeps the accent", function()
	local app = picker({ "on_android" })
	expect(app:has('"Frosted"')).to_be(false)
	expect(app:has('"Opaque"')).to_be(false)
	expect(app:has('"Accent"')).to_be(true)
end)
