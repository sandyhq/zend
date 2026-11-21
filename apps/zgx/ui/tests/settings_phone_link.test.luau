-- Settings → Devices, connecting a phone (src/phone_link.rs): on a
-- computer, Show code draws the QR code the host answers, says how long it
-- stays, and hides after that time or when Hide is pressed; with no relay
-- to share it says so, and a failure keeps a way to ask again. On the
-- phone, Scan code and a pasted code each reach phone.connect: "Saved,
-- connecting" until the engine's room is joined, then Connected; Stop
-- ends a scan (its late answer dropped); a refused camera says so; with a
-- session running here, Connect asks first; the pasted code is masked and
-- emptied once sent.
-- The QR's own pixels are checked in Rust (phone_link.rs's test). On the
-- shell in ui/shell_fixture.luau, Settings' calls answered by
-- ui/settings_stub.luau (a fake relay and key).

local PAGE = "#settings-page"
local NAV = "#settings-nav"
local CODE = `{PAGE} >> #phone-code`
local SAVED = `{PAGE} >> "Saved. Connecting to the relay at http://…:27640…"`
local JOINED = `{PAGE} >> "Connected to the relay at http://…:27640."`

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

test("with no relay to share, it says so instead of a code", function()
	local app = open({ "settings_stub.phone_no_edge" })
	app:click(`{PAGE} >> role:button "Show code"`)
	expect(app:wait(`{PAGE} >> "No relay to share"`, { timeout_ms = 5000 })).to_be(true)
	expect(app:has(CODE)).to_be(false)
end)

test("on the phone, a scanned code connects: saved, then connected once the relay answers", function()
	local app = open({ "settings_stub.on_android" })
	expect(app:has(`{PAGE} >> role:button "Show code"`)).to_be(false)
	app:click(`{PAGE} >> role:button "Scan code"`)
	expect(app:wait(SAVED, { timeout_ms = 5000 })).to_be(true)
	expect(asked(app, "phone.scan")).to_be(true)
	expect(asked(app, "phone.connect")).to_be(true)
	expect(app:has(JOINED)).to_be(false)
	app:command("settings_stub.relay_up")
	expect(app:wait(JOINED, { timeout_ms = 12000 })).to_be(true)
end)

-- The engine started again reads "connected" until 4 s of failing have
-- passed: that early word is no proof the relay answered.
test("on the phone, a relay that reads connected at once is not taken at its word", function()
	local app = open({ "settings_stub.on_android" })
	app:click(`{PAGE} >> role:button "Scan code"`)
	expect(app:wait(SAVED, { timeout_ms = 5000 })).to_be(true)
	-- (Down before, as the engine before had no relay; up at once after.)
	app:command("settings_stub.relay_up")
	app:frames(10)
	expect(app:has(JOINED)).to_be(false)
	expect(app:has(SAVED)).to_be(true)
	expect(app:wait(JOINED, { timeout_ms = 12000 })).to_be(true)
end)

test("on the phone, Stop ends the scan, and its late answer is dropped", function()
	local app = open({ "settings_stub.on_android", "settings_stub.phone_scan_waits" })
	app:click(`{PAGE} >> role:button "Scan code"`)
	expect(app:wait(`{PAGE} >> role:button "Stop"`, { timeout_ms = 5000 })).to_be(true)
	app:click(`{PAGE} >> role:button "Stop"`)
	expect(app:wait(`{PAGE} >> role:button "Scan code"`, { timeout_ms = 5000 })).to_be(true)
	expect(asked(app, "phone.scan_stop")).to_be(true)
	app:command("settings_stub.phone_scan_lands")
	app:frames(20)
	expect(asked(app, "phone.connect")).to_be(false)
end)

test("on the phone, leaving the page ends the scan", function()
	local app = open({ "settings_stub.on_android", "settings_stub.phone_scan_waits" })
	app:click(`{PAGE} >> role:button "Scan code"`)
	expect(app:wait(`{PAGE} >> role:button "Stop"`, { timeout_ms = 5000 })).to_be(true)
	app:click(`{NAV} >> role:tab "General"`)
	app:frames(30)
	expect(asked(app, "phone.scan_stop")).to_be(true)
	app:command("settings_stub.phone_scan_lands")
	app:frames(20)
	expect(asked(app, "phone.connect")).to_be(false)
end)

test("on the phone, a refused camera says so at once", function()
	local app = open({ "settings_stub.on_android", "settings_stub.phone_denied" })
	app:click(`{PAGE} >> role:button "Scan code"`)
	expect(app:wait(`{PAGE} >> "Camera not allowed. Allow it in Android's settings, or paste the code."`, { timeout_ms = 5000 })).to_be(true)
end)

test("on the phone, a pasted code connects, and other text is refused", function()
	local app = open({ "settings_stub.on_android" })
	app:fill(`{PAGE} >> "Connect code"`, "hello")
	app:click(`{PAGE} >> role:button "Connect"`)
	expect(app:wait(`{PAGE} >> "That's not a Zend connect code."`, { timeout_ms = 5000 })).to_be(true)
	app:fill(`{PAGE} >> "Connect code"`, " zend://connect?v=1&edge=http%3A%2F%2F100.64.0.1%3A27640&token=u-fake%40invalid ")
	app:click(`{PAGE} >> role:button "Connect"`)
	expect(app:wait(SAVED, { timeout_ms = 5000 })).to_be(true)
	-- (Emptied once sent, and never shown: the code is the computer's key.)
	expect(app:tree(PAGE):find("u-fake", 1, true) == nil).to_be(true)
end)

test("on the phone, with a session running here, Connect asks first", function()
	local app = open({ "settings_stub.on_android", "settings_stub.phone_busy" })
	app:click(`{PAGE} >> role:button "Scan code"`)
	expect(app:wait(`{PAGE} >> "Sessions are running on this phone"`, { timeout_ms = 5000 })).to_be(true)
	expect(asked(app, "phone.connect")).to_be(false)
	app:click(`{PAGE} >> role:button "Connect anyway"`)
	expect(app:wait(SAVED, { timeout_ms = 5000 })).to_be(true)
	expect(asked(app, "phone.connect")).to_be(true)
end)

test("on a computer, a failed code keeps a way to ask again", function()
	local app = open({ "settings_stub.phone_no_edge" })
	app:click(`{PAGE} >> role:button "Show code"`)
	expect(app:wait(`{PAGE} >> role:button "Try again"`, { timeout_ms = 5000 })).to_be(true)
end)
