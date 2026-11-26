-- Settings → Themes and → Templates on a paired server: its list, Use and
-- Delete name that server (src/peer_looks.rs); one that doesn't grant
-- settings says so, with the command, and is asked nothing; this computer's
-- own pages are as they were. On the page in ui/peer_looks_fixture.luau.

local function open(page: string): any
	local app = mount("peer_looks_fixture.luau")
	app:resize(1000, 900)
	app:set_state("page", page)
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
	return (app:state("calls") :: any) or ""
end

test("themes: a server that grants settings lists, uses and deletes its own", function()
	local app = open("themes")
	expect(calls(app)).never.to_contain("list:")
	pick(app, "Granting")
	expect(calls(app)).to_contain("list:themes@granting")
	local tree = app:tree()
	expect(tree).to_contain("Server Ink")
	expect(tree).to_contain("Nord")
	-- Use, not for the one in use there.
	app:click('role:button "Use"')
	app:frames(20)
	expect(calls(app)).to_contain("use:themes:nord@granting")
	app:click('role:button "Delete Server Ink"')
	app:frames(10)
	app:click('role:button "Delete"')
	app:frames(30)
	expect(calls(app)).to_contain("delete:themes:custom-ink@granting")
	-- (This computer's again: its own page.)
	pick(app, "MacBook Pro")
	expect(app:tree()).never.to_contain("Server Ink")
end)

test("templates: a server's are listed and used there, with no install or consent", function()
	local app = open("templates")
	pick(app, "Granting")
	expect(calls(app)).to_contain("list:templates@granting")
	local tree = app:tree()
	expect(tree).to_contain("Ops board")
	expect(app:has('role:button "Install"')).to_be(false)
	app:click('role:button "Use"')
	app:frames(20)
	expect(calls(app)).to_contain("use:templates:zend@granting")
	-- (Zend's own can't be deleted: no Delete beside it.)
	expect(app:has('role:button "Delete Zend"')).to_be(false)
end)

test("a server that doesn't grant settings says so, with the command, and is asked nothing", function()
	for _, page in { "themes", "templates" } do
		local app = open(page)
		pick(app, "AI")
		local tree = app:tree()
		expect(tree).to_contain("AI doesn't grant settings")
		expect(tree).to_contain("zend peer grant")
		expect(calls(app)).never.to_contain("@ai")
		expect(tree).never.to_contain("Ops board")
	end
end)
