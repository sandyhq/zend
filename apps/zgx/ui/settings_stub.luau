-- Settings' host calls, answered here as the host would (no engine or edge
-- in a test host): your profile and card, your organization, its members
-- and invitations, this computer's engine settings and banners. The shell's
-- fixture requires it, so Settings' pages build headless; the tests drive
-- it (tests/settings_in_place.test.luau).
--
-- What was asked, in order, is `settings_stub_calls` (a state, "name" or
-- "name:detail" each). `ui.command`s change the data as the edge would on
-- its own, or make the next change fail:
--   settings_stub.sign_in          signed in as Ada, in org_1 (else signed out)
--   settings_stub.restoring        the engine hasn't answered yet
--   settings_stub.signed_out       the engine says signed out
--   settings_stub.open_account     a way in to `settings/account` (a link)
--   settings_stub.member_joins     someone accepts an invitation
--   settings_stub.refuse_next      the next change is refused ("refused")
--   settings_stub.not_admin        you're a member, not an admin, from now on
--   settings_stub.signed_in_again  the browser sign-in a delete waits on lands
--   settings_stub.phone_no_tailnet this computer isn't on a tailnet
--   settings_stub.phone_quick      a phone code shows for 1 s, not 60
--   settings_stub.on_android       the page as the phone shows it (no session
--                                  runs on it: here is a phone of its own)
--   settings_stub.phone_busy       a session runs on this phone
--   settings_stub.phone_scan_waits the next scan looks until scan_lands
--   settings_stub.phone_scan_lands that scan finds the code (late, if stopped)
--   settings_stub.phone_scan_frame the camera's newest frame, as the host shares it
--   settings_stub.linked_phone     the phone, a window onto a computer (a direct code kept)
--   settings_stub.phone_denied     the camera isn't allowed
--   settings_stub.relay_up         the engine's registry room is joined
--   settings_stub.desktop_answers  the computer's engine answers (no relay)

local host = require("@app/screens/settings/host")
local api = require("@account/api")
local auth = require("@account/auth")

local M = {}

local calls = ui.state("settings_stub_calls", {} :: { string })
local refuse = ui.state("settings_stub_refuse", false)
local role = ui.state("settings_stub_role", "admin")
local about_next = ui.state("settings_stub_about_next", "ok")
local delete_word = ui.state("settings_stub_delete_word", "ok")
-- What a delete after signing in again answers: "ok", "wait" (until
-- `settings_stub.signed_in_again`, or cancelled), or a refusal.
local step_up_word = ui.state("settings_stub_step_up_word", "ok")
-- The account's signing keys: "some" (this computer's, one new on another
-- device, one revoked; listed out of order) or "none".
local keys_mode = ui.state("settings_stub_keys", "some")
local signing_keys = {
	{ key = "k_dev", fingerprint = "9f3a 21c0 77be 04d1 aa10 bb20 cc30 dd40", device_name = "dev", device_id = "d_dev", this_device = false, created_at = 1793600000 },
	{ key = "k_old", fingerprint = "0b0b 1c1c 2d2d 3e3e 4f4f 5a5a 6b6b 7c7c", device_name = "old-laptop", device_id = "d_old", this_device = false, created_at = 1780000000, revoked_at = 1785000000 },
	{ key = "k_mac", fingerprint = "a1b2 c3d4 e5f6 0718 2930 4a5b 6c7d 8e9f", device_name = "Ada's MacBook Pro", device_id = "d_mac", this_device = true, created_at = 1790000000 },
}
-- That wait, while it's open.
local signing_in: { ok: any, bad: any }? = nil
local function waiting(): any
	local p = {}
	function p.after(_self: any, ok: ((any) -> ())?, bad: ((any) -> ())?)
		signing_in = { ok = ok, bad = bad }
	end
	return p
end
local function signed_in_again(word: string?)
	local w = signing_in
	signing_in = nil
	if not w then
		return
	end
	if word then
		if w.bad then
			w.bad({ message = word })
		end
	elseif w.ok then
		w.ok(true)
	end
end
-- What the next photo upload answers ("ok", or a refusal like "too_large").
local photo_word = ui.state("settings_stub_photo_word", "ok")
-- The last square a photo was cut at, for a test to read.
local last_crop = ui.state("settings_stub_last_crop", {} :: any)
local photo_n = 0

local function log(entry: string)
	calls:push(entry)
end

-- A promise already settled, as the host's settle: `:after` at once.
local function settled(value: any, word: string?): any
	local p = { done = true, value = if word then nil else value, error = if word then { message = word } else nil }
	function p.after(self: any, ok: ((any) -> ())?, bad: ((any) -> ())?)
		if word then
			if bad then
				bad({ message = word })
			end
		elseif ok then
			ok(value)
		end
	end
	return p
end

-- A change: refused if asked to be, else `value`.
local function change(value: any): any
	if refuse:peek() then
		refuse:set(false)
		return settled(nil, "refused")
	end
	return settled(value)
end

local me = { first_name = "Ada", last_name = "Lovelace", name = "Ada Lovelace", email = "ada@example.net" }
local card: api.Card = {
	username = "ada",
	first_name = "Ada",
	number = 1815,
	joined_on = "2026-10-31",
	badges = {},
	scene = 0,
	holo = "glyph",
	sky = 0,
}
local members = {
	{ id = "om_ada", name = "Ada Lovelace", email = "ada@example.net", role = "admin", you = true, username = "ada" },
	{ id = "om_grace", name = "Grace Hopper", email = "grace@example.net", role = "member" },
	{ id = "om_alan", name = "Alan Turing", email = "alan@example.net", role = "member" },
}
local invitations = {
	{ id = "inv_1", email = "edsger@example.net", state = "pending", created_at = "2026-10-30T10:00:00Z" },
	{ id = "inv_2", email = "barbara@example.net", state = "expired", created_at = "2026-10-01T10:00:00Z" },
}
local engine: { [string]: any } = {
	GetCommitCredit = { enabled = false, cloud = false },
	GetWorktreeSettings = {
		settings = { useCustomDirectory = false },
		defaultDirectory = "~/.zend/worktrees",
		effectiveDirectory = "~/.zend/worktrees",
	},
	GetTitleSettings = {},
	GetNativeSubagents = { switchable = { "claude-code" }, off = {} },
}
local banners = { desktop = true, background_only = true, agent_updates = true }

local function find(list: { any }, id: string): number?
	for i, item in list do
		if item.id == id then
			return i
		end
	end
	return nil
end

-- A phone's connect code (src/phone_link.rs), with a fake relay and key.
local phone_tailnet = ui.state("settings_stub_phone_tailnet", true)
-- The phone link as the engine keeps it (crates/engine/src/phone_link.rs).
local link = { on = false, use = "name", key = 1 }
local function link_status(): any
	return {
		on = link.on,
		use = link.use,
		name = "omarchy.tail0.ts.net",
		ip = "100.64.0.1",
		host = if link.use == "ip" then "100.64.0.1" else "omarchy.tail0.ts.net",
		port = 27872,
	}
end
-- What the next scan answers: "found", "wait" (until phone_scan_lands), "denied".
local scan_word = ui.state("settings_stub_scan_word", "found")
local scan_waiting: ((any) -> ())? = nil
local FAKE_CODE = "zend://connect?v=1&edge=http%3A%2F%2F100.64.0.1%3A27640&token=u-fake%40invalid"
local FAKE_QR = '<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 29 29"><rect width="29" height="29" fill="#fff"/><path d="M4 4h7v7h-7z" fill="#000"/></svg>'

local answers: { [string]: (arg: any) -> any } = {
	["phone.connect_code"] = function(): any
		if not phone_tailnet:peek() then
			return settled(nil, "no_tailnet")
		end
		link.on = true
		local answer = link_status()
		answer.svg = FAKE_QR
		answer.modules = 29
		return settled(answer)
	end,
	["phone.link_status"] = function(): any
		return settled(link_status())
	end,
	["phone.link_set"] = function(arg: any): any
		if arg.on == true and not phone_tailnet:peek() then
			return settled(nil, "no_tailnet")
		end
		if arg.on ~= nil then
			link.on = arg.on == true
		end
		if arg.use == "name" or arg.use == "ip" then
			link.use = arg.use
		end
		return settled(link_status())
	end,
	["phone.link_new_key"] = function(): any
		link.key += 1
		return settled(link_status())
	end,
	["phone.scan"] = function(): any
		local word = scan_word:peek()
		if word == "wait" then
			local p = {}
			function p.after(_self: any, ok: ((any) -> ())?)
				scan_waiting = ok
			end
			return p
		end
		return if word == "found" then settled(FAKE_CODE) else settled(nil, word)
	end,
	["phone.connect"] = function(arg: any): any
		local code = string.match(arg.code or "", "^%s*(.-)%s*$") or ""
		if string.sub(code, 1, 15) ~= "zend://connect?" then
			return settled(nil, "bad_code")
		end
		return if string.find(code, "v=2", 1, true) then settled({ desktop = "omarchy.tail0.ts.net" }) else settled({ edge = "http://…:27640" })
	end,
	["account.me"] = function()
		return settled(table.clone(me))
	end,
	["dictation.call"] = function(arg: any)
		return settled(if arg.op == "status" then { installed = false, size = 670479942 } elseif arg.op == "inputs" then {} else nil)
	end,
	["account.rename"] = function(name: string)
		local r = change(nil)
		if r.error then
			return r
		end
		local first, rest = string.match(name, "^(%S+)%s*(.*)$")
		me.name, me.first_name, me.last_name = name, first or name, rest or ""
		return settled(table.clone(me))
	end,
	-- Profile's title, photo and deleting the account, as the edge answers
	-- (src/settings.rs). `settings_stub_delete_word`: what the next delete
	-- answers ("ok", or a refusal like "last_admin");
	-- `settings_stub_photo_word`: the next photo upload's.
	["account.set_title"] = function(title: string)
		local r = change(nil)
		if r.error then
			return r
		end
		me.title = if title ~= "" then title else nil
		return settled(table.clone(me))
	end,
	["account.photo_size"] = function()
		return settled({ w = 1200, h = 800 })
	end,
	["account.set_photo"] = function(arg: any)
		local r = change(nil)
		if r.error then
			return r
		end
		if photo_word:peek() ~= "ok" then
			local word = photo_word:peek()
			photo_word:set("ok")
			return settled(nil, word)
		end
		photo_n += 1
		me.avatar = string.format("https://staging.zend.invalid/auth/avatars/%032x", photo_n)
		last_crop:set({ x = arg.x, y = arg.y, side = arg.side })
		return settled(table.clone(me))
	end,
	-- A photo's file: the app's own PNG, whatever the address.
	["account.photo_file"] = function()
		return settled("assets/favicons/zend.png")
	end,
	["account.remove_photo"] = function()
		me.avatar = nil
		return settled(table.clone(me))
	end,
	["account.delete"] = function(arg: any)
		local word = delete_word:peek()
		if arg.confirm ~= me.email then
			return settled(nil, "invalid")
		elseif word ~= "ok" then
			return settled(nil, word)
		end
		return settled(true)
	end,
	["account.delete_signed_in"] = function(arg: any)
		local word = step_up_word:peek()
		if arg.confirm ~= me.email then
			return settled(nil, "invalid")
		elseif word == "wait" then
			return waiting()
		elseif word ~= "ok" then
			return settled(nil, word)
		end
		return settled(true)
	end,
	["account.signing_keys"] = function()
		if keys_mode:peek() == "none" then
			return settled({ keys = {}, new_keys = {} })
		end
		local out = {}
		for _, k in signing_keys do
			table.insert(out, table.clone(k))
		end
		return settled({ keys = out, new_keys = { "k_dev" } })
	end,
	["account.revoke_key"] = function(key: string)
		for _, k in signing_keys do
			if k.key == key then
				k.revoked_at = 1793700000
			end
		end
		return settled(true)
	end,
	["account.members"] = function()
		local out = {}
		for _, m in members do
			table.insert(out, table.clone(m))
		end
		return settled(out)
	end,
	["account.invitations"] = function()
		if role:peek() ~= "admin" then
			return settled(nil, "refused")
		end
		local out = {}
		for _, i in invitations do
			table.insert(out, table.clone(i))
		end
		return settled(out)
	end,
	["account.remove_member"] = function(arg: any)
		local r = change(nil)
		if not r.error then
			local at = find(members, arg.id)
			if at then
				table.remove(members, at)
			end
		end
		return r
	end,
	["account.set_role"] = function(arg: any)
		local r = change(nil)
		if not r.error then
			local at = find(members, arg.id)
			if at then
				members[at].role = arg.role
			end
		end
		return r
	end,
	["account.revoke_invite"] = function(arg: any)
		local r = change(nil)
		if not r.error then
			local at = find(invitations, arg.id)
			if at then
				table.remove(invitations, at)
			end
		end
		return r
	end,
	["account.resend_invite"] = function()
		return change(nil)
	end,
	["account.rename_org"] = function()
		return change(nil)
	end,
	["settings.engine"] = function(arg: any)
		local m = arg.method
		if string.sub(m, 1, 3) == "Set" then
			local r = change(nil)
			if r.error then
				return r
			end
			local get = "Get" .. string.sub(m, 4)
			if m == "SetCommitCredit" then
				engine[get] = { enabled = arg.params.enabled, cloud = false }
			elseif m == "SetWorktreeSettings" then
				local w = table.clone(engine[get])
				w.settings = arg.params
				engine[get] = w
			elseif m == "SetTitleSettings" then
				engine[get] = arg.params
			end
			return settled(engine[get])
		end
		return settled(engine[m])
	end,
	["notifications.get"] = function()
		if refuse:peek() then
			refuse:set(false)
			return settled(nil, "failed")
		end
		return settled(table.clone(banners))
	end,
	["notifications.set"] = function(arg: any)
		local r = change(nil)
		if not r.error then
			for k, v in arg do
				banners[k] = v
			end
		end
		return r
	end,
	["agents.update"] = function()
		return settled(nil)
	end,
	["agents.set_update_policy"] = function()
		return settled(nil)
	end,
	-- The release feed and the engine's updater (src/about.rs): what the
	-- next check and update answer is `settings_stub_about_next`
	-- ("ok", "pending", or an outcome word).
	["about.check"] = function()
		local next = about_next:peek()
		if next == "pending" then
			-- (Still on its way: never settles.)
			return { done = false, after = function() end }
		elseif next ~= "ok" then
			return settled(nil, next)
		end
		return settled({ latest = "0.2.140", available = true })
	end,
	["about.apply"] = function()
		local next = about_next:peek()
		if next == "pending" then
			-- (Still on its way: never settles.)
			return { done = false, after = function() end }
		elseif next ~= "ok" then
			return settled(nil, next)
		end
		return settled({ version = "0.2.140" })
	end,
	-- This computer's fonts (src/fonts.rs), a few of each kind.
	["fonts.list"] = function()
		return settled({
			{ family = "Fira Code", mono = true },
			{ family = "Inter", mono = false },
			{ family = "JetBrains Mono", mono = true },
			{ family = "Noto Serif", mono = false },
		})
	end,
	-- A background image kept: the picked file itself (the app's own PNG,
	-- so a headless host can draw it), 512 square.
	["wallpaper.keep"] = function(arg: any)
		local name = string.match(arg.path, "[^/]+$") or arg.path
		return settled({ path = arg.path, name = name, w = 512, h = 512 })
	end,
}

local function detail(arg: any): string
	if type(arg) == "string" then
		return arg
	elseif type(arg) == "table" then
		if arg.on ~= nil then
			return if arg.on then "on" else "off"
		end
		return arg.id or arg.method or arg.name or arg.path or arg.use or ""
	end
	return ""
end

local stub = host :: any
stub.call = function(name: string, arg: any?): any
	log(if detail(arg) ~= "" then `{name}:{detail(arg)}` else name)
	local answer = answers[name]
	return if answer then answer(arg) else settled(nil)
end
stub.invoke = function(name: string, arg: any?)
	log(if detail(arg) ~= "" then `{name}:{detail(arg)}` else name)
	if name == "account.cancel_step_up" then
		signed_in_again("cancelled")
	end
end
-- A photo picked: the app's own PNG, so a headless host can draw it.
stub.open_files = function(_opts: any): any
	log("open_files")
	return settled({ { path = "assets/favicons/zend.png", name = "zend.png", size = 1, type = "image/png" } })
end

-- The account's own calls (account/api), the same way.
local apis = api :: any
-- (No username yet: a card's art is WebP, which a headless host can't
-- draw, so Profile shows the way to one.)
apis.username = function(): any
	log("account.username")
	return settled(nil)
end
apis.check = function(name: string): any
	log(`account.check_username:{name}`)
	return settled({ available = true })
end
apis.card = function(name: string): any
	log(`account.card:{name}`)
	return settled(table.clone(card))
end
apis.orgs = function(): any
	log("account.orgs")
	return settled({ { id = "om_ada", organization_id = "org_1", name = "Analytical Engines", role = role:peek() } })
end
apis.invite = function(_org: string, emails: { string }): any
	log(`account.invite:{#emails}`)
	if refuse:peek() then
		refuse:set(false)
		return settled(nil, "refused")
	end
	local sent, failed = {}, {}
	for _, e in emails do
		if string.find(e, "taken", 1, true) then
			table.insert(failed, { email = e, error = "exists" })
		else
			table.insert(sent, e)
			table.insert(invitations, 1, { id = `inv_{e}`, email = e, state = "pending", created_at = "2026-10-31T10:00:00Z" })
		end
	end
	return settled({ sent = sent, failed = failed })
end
-- This computer's agents and another device's (account/agents): one each, so
-- a switch of device shows whose.
local agents = require("@account/agents")
local function agents_of(target: string?): any
	local name = if target then "Codex" else "Claude Code"
	local id = if target then "codex" else "claude-code"
	return {
		state = "ready",
		target = target,
		list = {
			{
				id = id,
				name = name,
				mark = if target then "openai" else "claude",
				installed = true,
				can_install = true,
				enabled = true,
				version = "1.0.0",
				installing = false,
				login = "idle",
				accounts = {},
			},
		},
	}
end
local agent_stub = agents :: any
agent_stub.load = function()
	log("agents.load")
	agents.now:set(agents_of(agents.now:peek().target))
end
agent_stub.target = function(device: string?)
	log(`agents.target:{device or ""}`)
	agents.now:set(agents_of(device))
end
-- A sign-in here is Claude's that asks for a code pasted back (no loopback
-- port free); the code finishes it with an account, or Cancel gives it up.
local function set_login(id: string, login: string, account: any?)
	local now = table.clone(agents.now:peek())
	local list = {}
	for _, a in now.list do
		local one = table.clone(a)
		if one.id == id then
			one.login = login
			if account then
				one.accounts = { account }
			end
		end
		table.insert(list, one)
	end
	now.list = list
	agents.now:set(now)
end
agent_stub.sign_in = function(id: string)
	log(`agents.sign_in:{id}`)
	set_login(id, "code")
end
agent_stub.cancel = function(id: string)
	log(`agents.cancel:{id}`)
	set_login(id, "idle")
end
agent_stub.complete = function(id: string, code: string)
	log(`agents.complete:{id}:{code}`)
	set_login(id, "idle", { id = "acc_1", email = "ada@example.net", plan = "Max", session = 0.2, week = 0.4, active = true })
end

-- Renaming a device (account/sessions), as the engine would answer.
local sessions_stub = require("@account/sessions") :: any
sessions_stub.rename_device = function(id: string, name: string): any
	log(`sessions.rename_device:{id}:{name}`)
	return change(nil)
end

local auths = auth :: any
auths.sign_out = function(done: (() -> ())?)
	log("account.sign_out")
	auth.account:set({ state = "signed_out", env = "staging" } :: any)
	if done then
		done()
	end
end

-- The launch before the engine has answered, and the engine saying signed out.
ui.command {
	id = "settings_stub.phone_no_tailnet",
	title = "Not on a tailnet",
	run = function()
		phone_tailnet:set(false)
	end,
}
ui.command {
	id = "settings_stub.phone_quick",
	title = "A phone code shows for a second",
	run = function()
		require("@app/screens/settings/devices").SHOWN_FOR = 1
	end,
}
local function here_is(id: string)
	local sessions = require("@account/sessions")
	local now = table.clone(sessions.now:peek()) :: any
	now.here = id
	sessions.now:set(now)
end
ui.command {
	id = "settings_stub.on_android",
	title = "The page as the phone shows it",
	run = function()
		ui.shared("platform", ""):set("android")
		here_is("pixel")
	end,
}
ui.command {
	id = "settings_stub.linked_phone",
	title = "The phone, a window onto a computer",
	run = function()
		ui.shared("platform", ""):set("android")
		ui.shared("phone.desktop", ""):set("omarchy.tail0.ts.net")
		here_is("pixel")
	end,
}
ui.command {
	id = "settings_stub.phone_busy",
	title = "A session runs on this phone",
	run = function()
		here_is("mac")
	end,
}
ui.command {
	id = "settings_stub.phone_scan_waits",
	title = "The next scan looks until it lands",
	run = function()
		scan_word:set("wait")
	end,
}
ui.command {
	id = "settings_stub.phone_scan_lands",
	title = "The waiting scan finds the code",
	run = function()
		local ok = scan_waiting
		scan_waiting = nil
		-- (The host lets the scan's last frame go as it ends.)
		ui.shared("phone.scan_preview", nil :: any):set(nil)
		if ok then
			ok(FAKE_CODE)
		end
	end,
}
ui.command {
	id = "settings_stub.phone_scan_frame",
	title = "The camera's newest frame",
	run = function()
		ui.shared("phone.scan_preview", nil :: any):set(ui.bitmap(3, 4, function(x: number, y: number): number
			return (x + y) / 5
		end))
	end,
}
ui.command {
	id = "settings_stub.phone_denied",
	title = "The camera isn't allowed",
	run = function()
		scan_word:set("denied")
	end,
}
ui.command {
	id = "settings_stub.relay_up",
	title = "The engine's registry room is joined",
	run = function()
		local sessions = require("@account/sessions")
		local now = table.clone(sessions.now:peek()) :: any
		now.relay = "connected"
		sessions.now:set(now)
	end,
}
ui.command {
	id = "settings_stub.desktop_answers",
	title = "The computer's engine answers, with no relay of its own",
	run = function()
		local sessions = require("@account/sessions")
		local now = table.clone(sessions.now:peek()) :: any
		now.here = "omarchy"
		now.relay = "offline"
		now.connection = "offline"
		sessions.now:set(now)
	end,
}
ui.command {
	id = "settings_stub.restoring",
	title = "The engine hasn't answered yet",
	run = function()
		auth.account:set({ state = "restoring", env = "staging" } :: any)
	end,
}
ui.command {
	id = "settings_stub.signed_out",
	title = "The engine says signed out",
	run = function()
		auth.account:set({ state = "signed_out", env = "staging" } :: any)
	end,
}

ui.command {
	id = "settings_stub.member_joins",
	title = "A member joins",
	run = function()
		table.insert(members, { id = "om_edsger", name = "Edsger Dijkstra", email = "edsger@example.net", role = "member" })
		local at = find(invitations, "inv_1")
		if at then
			table.remove(invitations, at)
		end
		-- (The page asks again: the edge doesn't tell.)
		local store = require("@app/screens/settings/store")
		store.load_members()
		store.load_invitations()
	end,
}
ui.command {
	id = "settings_stub.signed_in_again",
	title = "Signed in again in the browser",
	run = function()
		signed_in_again(nil)
	end,
}
ui.command {
	id = "settings_stub.sign_in",
	title = "Signed in",
	run = function()
		auth.account:set({
			state = "signed_in",
			env = "staging",
			user = { id = "user_ada", email = "ada@example.net", first_name = "Ada" },
			organization_id = "org_1",
		} :: any)
	end,
}
ui.command {
	id = "settings_stub.open_account",
	title = "Open Settings' old Account page",
	run = function()
		require("@app/screens/shell/route").settings("account")
	end,
}
-- What the main window's theme overrides now (ds.look): its accent, a
-- syntax colour and its fonts, for a test to read.
local window_theme = ui.state("settings_stub_window_theme", {} :: { [string]: any })
ui.command {
	id = "settings_stub.read_theme",
	title = "Read the window's theme",
	run = function()
		local t = require("@ds/look").window_theme() or {}
		window_theme:set({
			accent = t.accent,
			keyword = t["zend.syntax.keyword"],
			body = t["font.size.md"],
			window = t["zend.window"],
			sans = t["font.sans"],
			mono = t["font.mono"],
			secondary = t["text.secondary"],
			tertiary = t["text.tertiary"],
			-- "Accent colour throughout": a status dot, a find's highlight,
			-- an @file token, a list marker, and a link's ink (nil: the
			-- text's own).
			working_dot = t["zend.status.dot.working"],
			working = t["zend.status.working"],
			find = t["zend.editor.match"],
			file_token = t["zend.composer.token.file"],
			marker = t["zend.prose.marker"],
			link = require("@ds/look").accented_ink(nil)(),
			-- What the accent's own token is, and what the theme's says.
			accented_ink = ui.theme.get("zend.accented.ink"),
			accent_ink = ui.theme.get("accent"),
		})
	end,
}
ui.command {
	id = "settings_stub.refuse_next",
	title = "Refuse the next change",
	run = function()
		refuse:set(true)
	end,
}
ui.command {
	id = "settings_stub.not_admin",
	title = "Not an admin",
	run = function()
		role:set("member")
	end,
}

-- Cloud, Billing and GitHub for Cloud (Settings → Cloud and → Billing),
-- the same way: none until `cloud_stub.offer`.
require("./cloud_stub")
-- The ACP registry's agents (Settings → Providers → Add an agent).
require("./registry_stub")

return M
