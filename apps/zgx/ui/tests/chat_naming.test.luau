-- Naming where a chat works (screens/chat/naming, plans/branch-naming): a
-- new chat's new branch in a local checkout and a new worktree's name, both
-- carried by the send; an open chat's chip renaming its branch or worktree
-- and switching its branch, saying which other chats work there and doing
-- nothing while a turn runs. ui/chat_fixture.luau notes each call
-- (newbranch:, renamebranch:, renameworktree:, switch:, wtname:, made:, run:).

local function settle(app: any)
	app:frames(30)
end

local function showing(app: any, sel: string): boolean
	if not app:has(sel) then
		return false
	end
	local first = string.match(app:tree(sel), "[^\n]+") or ""
	return string.find(first, " hidden", 1, true) == nil
end

local function home(app: any)
	app:resize(960, 760)
	app:command("go_home")
	settle(app)
end

local function panel(app: any)
	if not showing(app, 'role:menuitem "Device"') then
		app:click('role:button "Branch"')
		settle(app)
	end
end

local function pick_in(app: any, row: string, option: string)
	panel(app)
	app:click(`role:menuitem "{row}"`)
	settle(app)
	app:click(`role:option ~"{option}"`)
	settle(app)
end

local function send(app: any, words: string)
	app:key("Escape")
	app:key("Escape")
	settle(app)
	app:type('role:textbox "Message"', words)
	app:key("Enter")
	settle(app)
end

-- K8: a local checkout's new branch is made off the ref chosen, in the
-- project's folder, before the chat; the chat and its run are on it.
test("a new chat in a local checkout makes the branch it's given, off the ref chosen", function()
	local app = mount("chat_fixture.luau")
	home(app)
	pick_in(app, "Checkout", "Local checkout Work in")
	panel(app)
	app:click('role:option "release/2.4"')
	settle(app)
	panel(app)
	app:click('~"New branch…"')
	settle(app)
	app:type('role:textbox "Branch"', "fix/login")
	app:click('role:button "Use"')
	settle(app)
	send(app, "Fix the login")
	local calls = app:state("calls")
	expect(calls).to_contain("newbranch:/tmp/atlas|fix/login|release/2.4|here")
	expect(calls).to_contain("made:atlas|-|fix/login|/tmp/atlas")
	expect(calls).to_contain("run:/tmp/atlas|local|fix/login")
	expect(app:error()).to_be(nil)
end)

test("a new branch git refuses keeps the draft and makes no chat", function()
	local app = mount("chat_fixture.luau")
	home(app)
	pick_in(app, "Checkout", "Local checkout Work in")
	panel(app)
	app:click('~"New branch…"')
	settle(app)
	app:type('role:textbox "Branch"', "taken")
	app:click('role:button "Use"')
	settle(app)
	app:command("git_refuses")
	send(app, "Fix the login")
	local calls = app:state("calls")
	expect(calls).to_contain("newbranch:/tmp/atlas|taken|main|here")
	expect((string.find(calls, "made:atlas", 1, true))).to_be(nil)
	expect(app:wait("~\"a branch named 'taken' already exists\"", { timeout_ms = 2000 })).to_be(true)
end)

test("a new worktree takes the name it's given, its branch apart", function()
	local app = mount("chat_fixture.luau")
	home(app)
	pick_in(app, "Checkout", "Worktree An isolated")
	panel(app)
	app:click('~"Name the worktree…"')
	settle(app)
	app:type('role:textbox "Name"', "fix-login")
	app:type('role:textbox "Branch"', "feat/login")
	app:click('role:button "Use"')
	settle(app)
	send(app, "Fix the login")
	local calls = app:state("calls")
	expect(calls).to_contain("run:/tmp/atlas|worktree|main")
	expect(calls).to_contain("wtname:fix-login|feat/login")
	expect((string.find(calls, "newbranch:", 1, true))).to_be(nil)
end)

test("with no name given, a send is as before", function()
	local app = mount("chat_fixture.luau")
	home(app)
	pick_in(app, "Checkout", "Worktree An isolated")
	send(app, "Cut the release")
	local calls = app:state("calls")
	expect(calls).to_contain("run:/tmp/atlas|worktree|main")
	expect((string.find(calls, "wtname:", 1, true))).to_be(nil)
	expect((string.find(calls, "newbranch:", 1, true))).to_be(nil)
end)

-- K9: an open chat's chip opens its menu.
local function open_in_checkout(app: any)
	app:resize(900, 700)
	app:command("go_s2")
	settle(app)
	app:command("session_in_checkout")
	settle(app)
	app:wait('~"Branch main"', { timeout_ms = 2000 })
end

test("an open chat's branch is renamed from its chip, the other chats there named", function()
	local app = mount("chat_fixture.luau")
	open_in_checkout(app)
	app:click('~"Branch main"')
	settle(app)
	app:click('role:menuitem ~"Rename branch"')
	settle(app)
	-- The other chat in the folder moves with it: said.
	expect(app:wait('~"Long chat"', { timeout_ms = 2000 })).to_be(true)
	app:fill('role:textbox "Branch"', "trunk")
	app:click('role:button "Rename"')
	settle(app)
	expect(app:state("calls")).to_contain("renamebranch:/tmp/atlas|trunk|here")
end)

test("an open chat in a local checkout switches branch from its chip", function()
	local app = mount("chat_fixture.luau")
	open_in_checkout(app)
	app:click('~"Branch main"')
	settle(app)
	app:click('role:menuitem ~"Switch branch"')
	settle(app)
	app:click('role:menuitem ~"release/2.4"')
	settle(app)
	app:click('role:button "Switch"')
	settle(app)
	expect(app:state("calls")).to_contain("switch:/tmp/atlas|release/2.4|here")
end)

test("nothing is renamed while a turn runs in the folder", function()
	local app = mount("chat_fixture.luau")
	open_in_checkout(app)
	app:command("turn_running")
	settle(app)
	app:click('~"Branch main"')
	settle(app)
	expect(app:wait('~"A turn is running"', { timeout_ms = 2000 })).to_be(true)
	app:click('role:menuitem ~"Rename branch"')
	settle(app)
	expect(app:has('role:textbox "Branch"')).to_be(false)
	expect((string.find(app:state("calls"), "renamebranch:", 1, true))).to_be(nil)
end)

test("a worktree's chat renames its worktree from its chip", function()
	local app = mount("chat_fixture.luau")
	app:resize(900, 700)
	app:command("go_s2")
	settle(app)
	app:command("session_in_worktree")
	settle(app)
	app:wait('~"zend/readme-pr"', { timeout_ms = 2000 })
	app:click('~"Branch zend/readme-pr"')
	settle(app)
	app:click('role:menuitem ~"Rename worktree"')
	settle(app)
	app:fill('role:textbox "Name"', "readme")
	app:click('role:button "Rename"')
	settle(app)
	expect(app:state("calls")).to_contain("renameworktree:/tmp/atlas-wt/pr|readme|-|here")
end)
