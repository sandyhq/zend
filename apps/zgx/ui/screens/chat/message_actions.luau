-- A chat's message actions for its transcript (ds/transcript's `on_fork`,
-- `on_edit`, `edit_blocked`): Fork from here opens a side chat starting at
-- that message (screens/chat/lanes); an edited prompt goes again, the
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
	on_edit: (id: string, text: string, attachments: { any }) -> (),
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
			local h = if home then home() else nil
			if h and c then
				lanes.fork(h, id, c)
			else
				lanes.fork(c, id)
			end
		end,
		edit_blocked = blocked,
		on_edit = function(id: string, text: string, attachments: { any })
			local c = chat()
			if not c or blocked() then
				return
			end
			-- (What it carried, sent again from where it is on the host.)
			local carried = {}
			for _, a in attachments do
				if type(a.path) == "string" then
					table.insert(carried, { path = a.path, name = a.name, kind = a.kind, on_host = true })
				end
			end
			-- (Failing, the edited words wait in the chat's message box: the
			-- editor has closed, and they're not lost.)
			local function failed(err: any)
				Shared.set_draft_of(c, "text", text)
				local why = if type(err) == "table" and err.message then tostring(err.message) else tostring(err)
				ds.toast.show { title = locale.s("chat.message_actions.couldnt_edit"), body = why, tone = "warning" }
			end
			sessions.rewind(c, id):after(function()
				local sent = store.send({ session = c, text = text, attachments = carried } :: any)
				if type(sent) == "table" and sent.after then
					sent:after(function() end, failed)
				end
			end, failed)
		end,
	}
end

return M
