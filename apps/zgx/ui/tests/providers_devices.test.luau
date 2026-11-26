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

local function calls(app: any): string
	return table.concat((app:state("settings_stub_calls") :: any) or {}, ",")
end

test("providers: a grant taken back while the page is open says so, and given again shows the agents", function()
	local app = open()
	pick(app, "Granting")
	expect(app:tree()).to_contain("Server Codex")
	app:command("revoke")
	app:frames(20)
	local tree = app:tree()
	expect(tree).to_contain("Granting doesn't grant logins")
	expect(tree).never.to_contain("Server Codex")
	app:command("regrant")
	app:frames(30)
	expect(app:tree()).to_contain("Server Codex")
	expect(app:tree()).never.to_contain("doesn't grant")
end)

test("providers: an agent's settings on a server follow its grants: native subagents need settings, the update policy updates", function()
	local app = open()
	-- Logins only: both are said, and the engine is asked of neither.
	pick(app, "Granting")
	app:click('role:button "Server Codex settings"')
	app:frames(30)
	local tree = app:tree()
	expect(tree).to_contain("Granting doesn't grant settings")
	expect(tree).to_contain("Granting doesn't grant updates")
	expect(calls(app)).never.to_contain("GetNativeSubagents")
	app:click('role:button "Done"')
	app:frames(20)
	-- All three: no notice, native subagents asked.
	pick(app, "Everything")
	app:click('role:button "Server Codex settings"')
	app:frames(30)
	expect(app:tree()).never.to_contain("doesn't grant")
	expect(calls(app)).to_contain("GetNativeSubagents")
end)

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
