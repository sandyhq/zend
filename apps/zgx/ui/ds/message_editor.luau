-- A prompt of the person's, edited where it stands (ds/transcript's Edit):
-- its words in a field as wide as its bubble could be, the files and
-- pictures it carries as chips, each with its ×, then Cancel and Save &
-- send. Sending again removes it and everything below it first (the
-- caller's `on_send`): with anything below, a dialog asks once, saying how
-- many messages go and that files the agent changed stay as they are.
-- While a turn runs it can't go (`blocked` says why, under the field).
-- Escape is Cancel, ⌘Enter Save & send. It stays open till the caller's
-- `done` says it went: a turn that started meanwhile, or an engine that
-- refused, leaves the words here with why.
--
-- `MessageEditor { id = "u3", text = "…", attachments = {…}, below = fn,
--   blocked = fn, on_send = fn(text, attachments, done), on_cancel = fn }`.

local locale = require("@self/locale")
local Button = require("@self/button")
local TextArea = require("@self/text_area")
local Icon = require("@self/icon")
local dialog = require("@self/dialog")
local UserMessage = require("@self/user_message")

type Attachment = UserMessage.Attachment

export type Props = {
	id: string,
	text: string,
	attachments: { Attachment }?,
	-- How many messages are below it now (all go with it).
	below: () -> number,
	-- Why it can't be sent now, nil when it can.
	blocked: (() -> string?)?,
	-- `done(nil)` once it went, `done(why)` when it didn't.
	on_send: (text: string, attachments: { Attachment }, done: (failure: string?) -> ()) -> (),
	on_cancel: () -> (),
}

-- What it carries that can go again: its files and pictures, by where they
-- are on the chat's host (passages and chats are in its words already).
local function sendable(list: { Attachment }?): { Attachment }
	local out = {}
	for _, a in list or {} do
		if (a.kind == "file" or a.kind == "image") and type((a :: any).path) == "string" then
			table.insert(out, a)
		end
	end
	return out
end

local function chip(a: Attachment, on_remove: () -> ()): Node
	return ui.row {
		class = "message-edit-chip",
		gap = "$space.1",
		items = "center",
		h = 24,
		pl = "$space.2",
		pr = "$space.1",
		rounded = "$radius.md",
		bg = "$zend.secondary.bg",
		max_w = ui.raw(220, "a chip's widest, its name cut short past it"),
		Icon { name = if a.kind == "image" then "gallery" else "document", size = 12, color = "$text.secondary" },
		ui.text { a.name, literal = true, truncate = true, text_size = "$font.size.sm", min_w = 0 },
		ui.button {
			label = locale.t("ds.transcript.edit_remove", { name = a.name }),
			tooltip = locale.s("ds.transcript.edit_remove", { name = a.name }),
			w = 18,
			h = 18,
			p = 0,
			items = "center",
			justify = "center",
			rounded = "$radius.sm",
			bg = "#00000000",
			color = "$text.secondary",
			hover = { bg = "$zend.secondary.hover", color = "$text.primary" },
			cursor = "pointer",
			on_click = on_remove,
			Icon { name = "close", size = 10 },
		},
	}
end

return ui.component(function(props: Props): Node
	local text = ui.use_state(props.text)
	local carried = ui.use_state(sendable(props.attachments))
	local asking = ui.use_state(false)
	local busy = ui.use_state(false)
	local failure = ui.use_state(nil :: string?)
	local function why(): string?
		local blocked = if props.blocked then props.blocked() else nil
		return blocked or failure:get()
	end
	local function has_words(s: string): boolean
		return string.find(s, "%S") ~= nil
	end
	-- (Asked again as it goes: a turn may have started while the dialog
	-- was open. Then nothing goes and the words stay here.)
	local function go()
		asking:set(false)
		if busy:peek() or (props.blocked and props.blocked()) ~= nil then
			return
		end
		busy:set(true)
		failure:set(nil)
		props.on_send(text:peek(), carried:peek(), function(err: string?)
			busy:set(false)
			failure:set(err)
		end)
	end
	local function save()
		failure:set(nil)
		if (props.blocked and props.blocked()) ~= nil or busy:peek() or not has_words(text:peek()) then
			return
		end
		if props.below() > 0 then
			asking:set(true)
		else
			go()
		end
	end
	local function remove(a: Attachment)
		local left = {}
		for _, b in carried:peek() do
			if (b :: any).path ~= (a :: any).path then
				table.insert(left, b)
			end
		end
		carried:set(left)
	end
	return ui.column {
		id = `edit-{props.id}`,
		class = "message-editor",
		role = "group",
		label = locale.t("ds.transcript.edit_label"),
		w_full = true,
		gap = "$space.2",
		items = "end",
		TextArea {
			value = text,
			label = locale.t("ds.transcript.edit_label"),
			rows = 2,
			max_rows = 12,
			autofocus = true,
			w_full = true,
			on_submit = function()
				save()
			end,
			on_key = function(e: any): boolean?
				if e.key == "Escape" then
					props.on_cancel()
					return true
				end
				return nil
			end,
		},
		ui.show(function(): boolean
			return #carried:get() > 0
		end, ui.row {
			w_full = true,
			flex_wrap = true,
			justify = "end",
			gap = "$space.1",
			ui.each(carried, function(a: Attachment): Node
				return chip(a, function()
					remove(a)
				end)
			end),
		}),
		ui.row {
			gap = "$space.2",
			items = "center",
			ui.show(function(): boolean
				return why() ~= nil
			end, ui.text {
				function(): string
					return why() or ""
				end,
				class = "message-edit-blocked",
				text_size = "$font.size.sm",
				color = "$text.secondary",
			}),
			Button {
				locale.t("ds.transcript.edit_cancel"),
				tone = "ghost",
				size = "sm",
				on_click = props.on_cancel,
			},
			Button {
				locale.t("ds.transcript.edit_save"),
				size = "sm",
				disabled = function(): boolean
					return (props.blocked and props.blocked()) ~= nil or busy:get() or not has_words(text:get())
				end,
				on_click = save,
			},
		},
		dialog.Dialog {
			open = asking,
			tone = "danger",
			title = locale.t("ds.transcript.edit_confirm_title"),
			body = function(): string
				return locale.s("ds.transcript.edit_confirm_body", { n = props.below() })
			end,
			confirm = { label = locale.t("ds.transcript.edit_confirm"), on_click = go },
		},
	}
end)
