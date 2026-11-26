-- Settings → Providers on a paired server: one that grants logins shows its
-- agents; one that doesn't says so with the command and is asked nothing
-- (no error); this computer's agents come back. On the page in
-- ui/providers_devices_fixture.luau.

local function open(): any
	local app = mount("providers_devices_fixture.luau")
	app:resize(1000, 900)
	app:frames(20)
	return app
end

local function pick(app: any, name: string)
	app:click('role:button "Device"')
	app:frames(10)
	app:click(`role:menuitem "{name}"`)
	app:frames(30)
end

test("providers: a server that doesn't grant logins says so, with the command, and shows no agents", function()
	local app = open()
	expect(app:tree()).to_contain("Local Claude")
	pick(app, "AI")
	local tree = app:tree()
	expect(tree).to_contain("AI doesn't grant logins")
	expect(tree).to_contain("zend peer grant")
	expect(tree).to_contain("MacBook Pro")
	expect(tree).never.to_contain("Local Claude")
	expect(tree).never.to_contain("Server Codex")
	-- Never as an error: no "can't be checked" page, no toast.
	expect(tree).never.to_contain("Couldn't")
	pick(app, "Granting")
	expect(app:tree()).to_contain("Server Codex")
	expect(app:tree()).never.to_contain("doesn't grant")
	pick(app, "MacBook Pro")
	expect(app:tree()).to_contain("Local Claude")
end)
