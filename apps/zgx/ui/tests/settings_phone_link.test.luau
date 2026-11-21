-- Settings → Devices, connecting a phone (src/phone_link.rs): on a
-- computer, Show code draws the QR code the host answers, says how long it
-- stays, and hides after that time, when Hide is pressed, or once a new
-- device joins; with no relay to share it says so. On the phone, Scan
-- code and a pasted code each reach phone.connect and say what came of it.
-- The QR's own pixels are checked in Rust (phone_link.rs's test). On the
-- shell in ui/shell_fixture.luau, Settings' calls answered by
-- ui/settings_stub.luau (a fake relay and key).

local PAGE = "#settings-page"
local NAV = "#settings-nav"
local CODE = `{PAGE} >> #phone-code`

local function open(before: { string }?): any
	local app = mount("shell_fixture.luau")
	app:resize(1200, 900)
	app:frames(30)
	app:command("settings_stub.sign_in")
	for _, c in before or {} do
		app:command(c)
	end
	app:command("shell.settings")
	app:frames(20)
	app:click(`{NAV} >> role:tab "Devices"`)
	app:frames(30)
	return app
end

local function asked(app: any, entry: string): boolean
	for _, c in (app:state("settings_stub_calls") :: any) or {} do
		if c == entry then
			return true
		end
	end
	return false
end

local function show(app: any)
	app:click(`{PAGE} >> role:button "Show code"`)
	expect(app:wait(CODE, { timeout_ms = 5000 })).to_be(true)
end

test("Show code draws the host's QR code for a minute, and Hide takes it away", function()
	local app = open()
	expect(app:has(CODE)).to_be(false)
	show(app)
	expect(asked(app, "phone.connect_code")).to_be(true)
	expect(app:tree(PAGE):find("Connect code for your phone", 1, true) ~= nil).to_be(true)
	expect(app:text(`{PAGE} >> #phone-code-left`)).to_be("Hides in 60 s")
	expect(app:wait(`{PAGE} >> role:button "Hide"`, { timeout_ms = 5000 })).to_be(true)
	app:click(`{PAGE} >> role:button "Hide"`)
	expect(app:wait(`{PAGE} >> role:button "Show code"`, { timeout_ms = 5000 })).to_be(true)
	expect(app:has(CODE)).to_be(false)
end)

test("the code hides by itself once its time is up", function()
	local app = open({ "settings_stub.phone_quick" })
	show(app)
	-- (1 s here; 60 s in the app.)
	expect(app:wait(`{PAGE} >> role:button "Show code"`, { timeout_ms = 5000 })).to_be(true)
	expect(app:has(CODE)).to_be(false)
end)

test("the code hides once a new device joins, and says which", function()
	local app = open()
	show(app)
	app:command("fixture.more_devices")
	expect(app:wait('"Mac Studio joined"', { timeout_ms = 5000 })).to_be(true)
	expect(app:wait(`{PAGE} >> role:button "Show code"`, { timeout_ms = 5000 })).to_be(true)
	expect(app:has(CODE)).to_be(false)
end)

test("with no relay to share, it says so instead of a code", function()
	local app = open({ "settings_stub.phone_no_edge" })
	app:click(`{PAGE} >> role:button "Show code"`)
	expect(app:wait(`{PAGE} >> "No relay to share"`, { timeout_ms = 5000 })).to_be(true)
	expect(app:has(CODE)).to_be(false)
end)

test("on the phone, a scanned code connects", function()
	local app = open({ "settings_stub.on_android" })
	expect(app:has(`{PAGE} >> role:button "Show code"`)).to_be(false)
	app:click(`{PAGE} >> role:button "Scan code"`)
	expect(app:wait(`{PAGE} >> "Joined the relay at http://…:27640. Your sessions show once it answers."`, { timeout_ms = 5000 })).to_be(true)
	expect(asked(app, "phone.scan")).to_be(true)
	expect(asked(app, "phone.connect")).to_be(true)
end)

test("on the phone, a pasted code connects, and other text is refused", function()
	local app = open({ "settings_stub.on_android" })
	app:fill(`{PAGE} >> "Connect code"`, "hello")
	app:click(`{PAGE} >> role:button "Connect"`)
	expect(app:wait(`{PAGE} >> "That's not a Zend connect code."`, { timeout_ms = 5000 })).to_be(true)
	app:fill(`{PAGE} >> "Connect code"`, " zend://connect?v=1&edge=http%3A%2F%2F100.64.0.1%3A27640&token=u-fake%40invalid ")
	app:click(`{PAGE} >> role:button "Connect"`)
	expect(app:wait(`{PAGE} >> "Joined the relay at http://…:27640. Your sessions show once it answers."`, { timeout_ms = 5000 })).to_be(true)
end)
