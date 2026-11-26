-- Settings → About & updates on a paired server: one that grants updates
-- shows its engine's update and updates it there; one that doesn't says so
-- with the command and is asked nothing (no error); this computer's own
-- page comes back. On the page in ui/about_devices_fixture.luau.

local function open(): any
	local app = mount("about_devices_fixture.luau")
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

test("about: a server that grants updates shows its update, and Update asks that server", function()
	local app = open()
	expect(app:tree()).to_contain("Updates go to the engine on this computer")
	pick(app, "Granting")
	local tree = app:tree()
	expect(tree).to_contain("0.2.131")
	expect(tree).never.to_contain("doesn't grant")
	expect(app:state("calls")).to_contain("peer_updates.show:granting")
	app:click('role:button "Update engine"')
	app:frames(20)
	expect(app:state("calls")).to_contain("peer_updates.apply:granting")
end)

test("about: a server that doesn't grant updates says so, with the command, and Update asks nothing", function()
	local app = open()
	pick(app, "AI")
	local tree = app:tree()
	expect(tree).to_contain("AI doesn't grant updates")
	expect(tree).to_contain("zend peer grant")
	expect(app:has('role:button "Update engine"')).to_be(false)
	expect(app:state("calls")).never.to_contain("peer_updates.apply")
	-- Back on this computer: its own update.
	pick(app, "MacBook Pro")
	expect(app:tree()).never.to_contain("doesn't grant")
end)
