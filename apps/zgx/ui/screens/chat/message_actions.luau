-- A chat's message actions for its transcript (ds/transcript's `on_fork`,
-- `on_edit`, `edit_blocked`): Fork from here starts a copy at that message
-- (screens/chat/lanes): in the main session a session of its own, opened
-- as a page (`lanes.fork_chat`); in a side chat's tab in the panel
-- (`home`) a side chat beside it (`lanes.fork`); an edited prompt goes again, the
-- engine first removing it and everything below it (`RewindChat`), then
-- the new words sent as a prompt. Not while a turn runs: that's why not.
--
--   local acts = MessageActions.of(function() return chat_id end, function() return session end)
--   T.Transcript { on_fork = acts.on_fork, on_edit = acts.on_edit, edit_blocked = acts.edit_blocked, … }

local ds = require("@ds")
local locale = require("@ds/locale")
local sessions = require("@account/sessions")
local store = require("@app/screens/chat/store")
local lanes = require("@app/screens/chat/lanes")
local Shared = require("@app/screens/chat/shared")

local M = {}

export type Acts = {
	on_fork: (id: string) -> (),
	-- `done(nil)` once the old prompt is gone, `done(why)` when it stays
	-- (the editor keeps the words and says why).
	on_edit: (id: string, text: string, attachments: { any }, done: (string?) -> ()) -> (),
	edit_blocked: () -> string?,
}

-- `chat`: the chat the transcript shows; `session`: its session row now
-- (tracked; `running` while a turn runs); `home`: for a side chat shown
-- as a lane, the chat whose panel it's in (a fork opens beside it there).
function M.of(chat: () -> string?, session: () -> any, home: (() -> string?)?): Acts
	local function blocked(): string?
		local s = session()
		if s ~= nil and s.running == true then
			return locale.s("chat.message_actions.edit_running")
		end
		return nil
	end
	return {
		on_fork = function(id: string)
			local c = chat()
			if not home then
				lanes.fork_chat(c, id)
				return
			end
			local h = home()
			if h and c then
				lanes.fork(h, id, c)
			else
				lanes.fork(c, id)
			end
		end,
		edit_blocked = blocked,
		on_edit = function(id: string, text: string, attachments: { any }, done: (string?) -> ())
			local c = chat()
			if not c then
				done(locale.s("chat.message_actions.couldnt_edit"))
				return
			end
			local busy = blocked()
			if busy then
				done(busy)
				return
			end
			-- (What it carried, sent again from where it is on the host.)
			local carried = {}
			for _, a in attachments do
				if type(a.path) == "string" then
					table.insert(carried, { path = a.path, name = a.name, kind = a.kind, on_host = true })
				end
			end
			local function reason(err: any): string
				return if type(err) == "table" and err.message then tostring(err.message) else tostring(err)
			end
			-- (The old prompt is gone, so the editor with it: words that then
			-- don't go wait in the chat's message box, after whatever it
			-- holds, never in its place.)
			local function unsent(err: any)
				local now = Shared.draft_of(c).text
				Shared.set_draft_of(c, "text", if string.find(now, "%S") then `{now}\n\n{text}` else text)
				ds.toast.show { title = locale.s("chat.message_actions.couldnt_edit"), body = reason(err), tone = "warning" }
			end
			sessions.rewind(c, id):after(function()
				done(nil)
				local sent = store.send({ session = c, text = text, attachments = carried } :: any)
				if type(sent) == "table" and sent.after then
					sent:after(function() end, unsent)
				end
			end, function(err: any)
				done(reason(err))
			end)
		end,
	}
end

return M
