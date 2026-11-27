-- Where a files pane or a terminal works, and a path's name: plain
-- functions of what they're given (screens/files/api is the host's calls
-- alone, which fixtures stand in for wholesale).

local control = require("@ds/control")
local api = require("@app/screens/files/api")
local projects = require("@account/projects")

type Target = api.Target

local M = {}

-- Where a pane or a terminal works: the session's chat on its device (in
-- its folder), or a project's folder with none open. Each may be a value,
-- a function or a state of one, or an id alone.
function M.target_of(project: unknown, session: unknown): Target?
	local s: any, p: any = control.value(session), control.value(project)
	if type(s) == "string" then
		s = { id = s }
	end
	if type(p) == "string" then
		p = { id = p }
	end
	if type(s) == "table" and s.id then
		return {
			chat = s.id,
			device = s.device or (if type(p) == "table" then p.device else nil),
			root = s.cwd or (if type(p) == "table" then p.path else nil),
		}
	elseif type(p) == "table" and p.id then
		return { space = p.id, device = p.device, root = p.path }
	end
	return nil
end

-- A paired server that doesn't grant the area a call is in: the host sends
-- no call then and says "not_granted|<area>|<server>|<this device>"
-- (src/server_route.rs). Where an error is one: { area, server, device }.
function M.not_granted(err: unknown): { area: string, server: string, device: string }?
	local m = if type(err) == "table" and (err :: any).message then tostring((err :: any).message) else tostring(err)
	local area, server, device = string.match(m, "not_granted|([^|]*)|([^|]*)|([^|]*)$")
	if area == nil or server == nil or device == nil then
		return nil
	end
	return { area = area, server = server, device = device }
end

-- What such a page says, plainly and with the way on: the server doesn't
-- grant the area, and the command to run on the server. Nil for any other
-- error. (Words of the moment they're asked for.)
function M.not_granted_words(err: unknown): { title: string, body: string }?
	local g = M.not_granted(err)
	if not g then
		return nil
	end
	-- (The command is an argument, not words to translate.)
	local p = { server = g.server, area = g.area, command = `zend peer grant {projects.shell_word(g.device)} {g.area}` }
	return { title = ui.t("files.not_granted.title", p):get(), body = ui.t("files.not_granted.body", p):get() }
end

-- A new chat a paired server won't run in its project (src/sessions.rs
-- `create`: the refusal for that project's run, `sessions.run:<project>`,
-- or `sessions.run` held back here before asking): the server doesn't let
-- this device run chats in the project, and the command to run on the
-- server. `project` is the project's id, `name` its name. Nil for any
-- other error.
function M.run_not_granted_words(err: unknown, project: string?, name: string?): string?
	local g = M.not_granted(err)
	if not g or (g.area ~= "sessions.run" and string.sub(g.area, 1, 13) ~= "sessions.run:") then
		return nil
	end
	local area = if g.area == "sessions.run" and project then `sessions.run:{project}` else g.area
	local id = string.match(area, "^sessions%.run:(.+)$") or project or ""
	-- (The command is an argument, not words to translate.)
	local p = { server = g.server, project = name or id, command = `zend peer grant {projects.shell_word(g.device)} {area}` }
	return ui.t("chat.composer.not_granted", p):get()
end

-- A path's last part (a file's name).
function M.base_name(path: string): string
	return string.match(path, "[^/]+$") or path
end

return M
