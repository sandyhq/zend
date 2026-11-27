-- Naming where a chat works, and renaming it (plans/branch-naming).
--
-- A new chat: in a local checkout, a new branch made off the ref chosen
-- (`git switch -c` in the project's folder, as the chat is sent); in a
-- worktree, the worktree's name (its folder and its branch, no `zend/`), and
-- its branch apart when it should differ. Asked in a dialog from the branch
-- chip's menu; held in `names` until the chat is sent (screens/chat/where's
-- plan carries them).
--
-- An open chat: its chip's menu renames its branch, renames its worktree (the
-- folder moved, its branch renamed, every chat in it following), or, in a
-- local checkout, checks out another branch. Each says which other chats work
-- in that folder (they move with it), and none goes while a turn runs in one
-- of them. Git's own words say why one didn't (nothing is forced).

local sessions = require("@account/sessions")
local store = require("@app/screens/chat/store")
local locale = require("@ds/locale")
local ds = require("@ds")

local M = {}

-- What a new chat is to be named (empty: as Zend names it).
export type Names = { new_branch: string, worktree_name: string, worktree_branch: string }
M.names = ui.shared("chat.names", { new_branch = "", worktree_name = "", worktree_branch = "" } :: Names)

-- What the dialog asks: `kind` one of new_branch, worktree (a new chat's),
-- rename_branch, rename_worktree, switch (an open chat's). `path` the folder
-- (an open chat's own), `device` its device when it isn't this one, `chat`
-- the open chat, `ref` the branch to switch to, `current` the branch now.
export type Ask = { kind: string, path: string?, device: string?, chat: string?, ref: string?, current: string? }
M.asking = ui.shared("chat.naming.asking", nil :: Ask?)

local function trim(s: string?): string
	return string.match(s or "", "^%s*(.-)%s*$") or ""
end

-- Whether `cwd` is `folder` or inside it.
local function within(cwd: string?, folder: string): boolean
	if not cwd or folder == "" then
		return false
	end
	local f = string.gsub(folder, "/+$", "")
	return cwd == f or string.sub(cwd, 1, #f + 1) == f .. "/"
end

-- The chats working in `folder` (on `device`), but `except`.
function M.sharing(folder: string?, device: string?, except: string?): { sessions.Chat }
	local out: { sessions.Chat } = {}
	if not folder then
		return out
	end
	for _, c in sessions.now:peek().chats do
		if c.id ~= except and (device == nil or c.device == device) and within(c.cwd, folder) then
			table.insert(out, c)
		end
	end
	return out
end

-- The warning naming the other chats in the folder; nil with none.
function M.warning(folder: string?, device: string?, except: string?): string?
	local others = M.sharing(folder, device, except)
	if #others == 0 then
		return nil
	end
	local titles: { string } = {}
	for i, c in others do
		if i > 3 then
			break
		end
		table.insert(titles, `"{c.title}"`)
	end
	local more = #others - #titles
	return locale.s("chat.naming.others", {
		count = #others,
		names = table.concat(titles, ", "),
		more = more,
	})
end

-- Why nothing can change in the folder now: a turn running in a chat there
-- (the open one's own `running` too); nil when nothing runs.
function M.busy(folder: string?, device: string?, own_running: boolean?): string?
	if own_running then
		return locale.s("chat.naming.busy_here")
	end
	for _, c in M.sharing(folder, device, nil) do
		if c.status == "working" then
			return locale.s("chat.naming.busy", { title = c.title })
		end
	end
	return nil
end

-- What the dialog's name field and branch field hold, what went wrong, and
-- whether its call is on its way.
local name = ui.shared("chat.naming.name", "")
local branch = ui.shared("chat.naming.branch", "")
local failed = ui.shared("chat.naming.failed", nil :: string?)
local working = ui.shared("chat.naming.working", false)
local open = ui.shared("chat.naming.open", false)

-- Ask: the dialog opens on `a`, its fields filled as they stand.
function M.ask(a: Ask)
	ui.batch(function()
		local n = M.names:peek()
		if a.kind == "new_branch" then
			name:set(n.new_branch)
			branch:set("")
		elseif a.kind == "worktree" then
			name:set(n.worktree_name)
			branch:set(n.worktree_branch)
		elseif a.kind == "rename_branch" then
			name:set(a.current or "")
			branch:set("")
		elseif a.kind == "rename_worktree" then
			local folder = if a.path then string.match(a.path, "([^/]+)/*$") else nil
			name:set(folder or "")
			branch:set("")
		else
			name:set("")
			branch:set("")
		end
		failed:set(nil)
		working:set(false)
		M.asking:set(a)
		open:set(true)
	end)
end

-- A new chat's names, cleared (it was sent, or its place changed).
function M.clear()
	M.names:set({ new_branch = "", worktree_name = "", worktree_branch = "" })
end

-- Git's words from a refused call.
local function words(e: any): string
	if type(e) == "table" and type(e.message) == "string" then
		return e.message
	end
	return tostring(e)
end

-- Heard once an open chat's change is made: the branch it's on now (the
-- composer's chip follows it at once).
M.changed = ui.shared("chat.naming.changed", nil :: { chat: string?, branch: string, n: number }?)

local function done(a: Ask, answer: any)
	local b = if type(answer) == "table" and type(answer.branch) == "string" then answer.branch else ""
	local prev = M.changed:peek()
	ui.batch(function()
		working:set(false)
		open:set(false)
		M.asking:set(nil)
		M.changed:set({ chat = a.chat, branch = b, n = (if prev then prev.n else 0) + 1 })
	end)
end

local function confirm()
	local a = M.asking:peek()
	if not a then
		open:set(false)
		return
	end
	local n, b = trim(name:peek()), trim(branch:peek())
	if a.kind == "new_branch" then
		M.names:set({ new_branch = n, worktree_name = "", worktree_branch = "" })
		open:set(false)
		return
	elseif a.kind == "worktree" then
		M.names:set({ new_branch = "", worktree_name = n, worktree_branch = if n ~= "" then b else "" })
		open:set(false)
		return
	end
	if a.kind ~= "switch" and n == "" then
		failed:set(locale.s("chat.naming.name_needed"))
		return
	end
	local path = a.path or ""
	local call: any
	if a.kind == "rename_branch" then
		call = store.rename_branch(path, n, a.device)
	elseif a.kind == "rename_worktree" then
		call = store.rename_worktree(path, n, if b ~= "" then b else nil, a.device)
	else
		call = store.switch_ref(path, a.ref or "", a.device)
	end
	failed:set(nil)
	working:set(true)
	call:after(function(answer: any)
		done(a, answer)
	end, function(e: any)
		working:set(false)
		failed:set(words(e))
	end)
end

local TITLES = {
	new_branch = "chat.naming.new_branch_title",
	worktree = "chat.naming.worktree_title",
	rename_branch = "chat.naming.rename_branch_title",
	rename_worktree = "chat.naming.rename_worktree_title",
	switch = "chat.naming.switch_title",
}
local CONFIRMS = {
	new_branch = "chat.naming.use",
	worktree = "chat.naming.use",
	rename_branch = "common.rename",
	rename_worktree = "common.rename",
	switch = "chat.naming.switch",
}

-- What the dialog says over its fields: the other chats moving with it, and
-- (switching) which branch.
local function body(): string
	local a = M.asking:get()
	if not a then
		return ""
	end
	local lines: { string } = {}
	if a.kind == "switch" then
		table.insert(lines, locale.s("chat.naming.switch_body", { ref = a.ref or "", current = a.current or "" }))
	elseif a.kind == "worktree" then
		table.insert(lines, locale.s("chat.naming.worktree_body"))
	end
	-- (A new worktree has no one else in it yet.)
	if a.kind ~= "worktree" then
		local warn = M.warning(a.path, a.device, a.chat)
		if warn then
			table.insert(lines, warn)
		end
	end
	return table.concat(lines, "\n\n")
end

-- The dialog, once in the page.
function M.Dialog(): Node
	local function kind(): string
		local a = M.asking:get()
		return if a then a.kind else ""
	end
	return ds.Dialog {
		open = open,
		title = function(): string
			local key = TITLES[kind()]
			return if key then locale.s(key) else ""
		end,
		body = body,
		size = "sm",
		confirm = {
			label = function(): string
				local key = CONFIRMS[kind()]
				return if key then locale.s(key) else ""
			end,
			on_click = confirm,
			keep_open = true,
			busy = function(): boolean
				return working:get()
			end,
		},
		ui.show(function(): boolean
			return kind() ~= "switch"
		end, ds.TextField {
			value = name,
			label = function(): string
				return locale.s(if kind() == "new_branch" or kind() == "rename_branch" then "chat.naming.branch" else "chat.naming.name")
			end :: any,
			placeholder = function(): string
				return locale.s(if kind() == "new_branch" then "chat.naming.new_branch_placeholder" else "chat.naming.name_placeholder")
			end,
			error = function(): string?
				return if kind() ~= "switch" then failed:get() else nil
			end :: any,
			select_on_focus = true,
			autofocus = true,
			w_full = true,
		}),
		-- A worktree's branch apart from its name (empty: the name).
		ui.show(function(): boolean
			return kind() == "worktree" or kind() == "rename_worktree"
		end, ds.TextField {
			value = branch,
			label = function(): string
				return locale.s("chat.naming.branch")
			end :: any,
			placeholder = function(): string
				local n = trim(name:get())
				return if n ~= "" then n else locale.s("chat.naming.branch_follows")
			end,
			w_full = true,
		}),
		ui.show(function(): boolean
			return kind() == "switch" and failed:get() ~= nil
		end, ui.text {
			color = "$danger",
			text_size = "$font.size.sm",
			function(): string
				return failed:get() or ""
			end,
		}),
	}
end

-- The menu of an open chat's chip: `s` the chat (its id, folder, device,
-- whether a turn runs), `worktree` whether its folder is a worktree,
-- `current` its branch now, `refs` its branches (the composer's live read).
export type Open = { id: string, cwd: string?, device: string?, running: boolean?, here: string? }
function M.menu(s: Open, worktree: boolean, current: string?, refs: { store.Ref }): { any }
	if not s.cwd then
		return {}
	end
	local device = if s.device ~= s.here then s.device else nil
	local why = M.busy(s.cwd, s.device, s.running)
	local function ask(kind: string, ref: string?)
		M.ask({ kind = kind, path = s.cwd, device = device, chat = s.id, ref = ref, current = current })
	end
	local entries: { any } = {
		{
			label = locale.s("chat.naming.rename_branch"),
			icon = "pen",
			disabled = why or (current == nil),
			on_select = function()
				ask("rename_branch")
			end,
		},
	}
	if worktree then
		table.insert(entries, {
			label = locale.s("chat.naming.rename_worktree"),
			icon = "worktree",
			disabled = why,
			on_select = function()
				ask("rename_worktree")
			end,
		})
	else
		-- Another branch, checked out here: those not checked out elsewhere.
		local others: { any } = {}
		for _, ref in refs do
			if not ref.current and not ref.worktree then
				local r = ref.name
				table.insert(others, {
					label = r,
					icon = "git-branch",
					on_select = function()
						ask("switch", r)
					end,
				})
			end
		end
		table.insert(entries, {
			label = locale.s("chat.naming.switch_branch"),
			icon = "git-branch",
			disabled = why or (if #others == 0 then locale.s("chat.naming.no_other_branch") else nil),
			submenu = if why or #others == 0 then nil else others,
		})
	end
	return entries
end

return M
