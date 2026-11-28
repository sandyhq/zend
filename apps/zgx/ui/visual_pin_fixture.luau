-- ds/visual_pin's pick, run on made-up transcripts (ui/tests/visual_pin
-- .test.luau reads what it found from the state `out`). Mounted, so
-- the module resolves as it does in the app.

local VisualPin = require("@ds/visual_pin")

local function visual(id: string, extra: { [string]: any }?): any
	local b: any = { kind = "visual", id = id }
	for k, v in extra or {} do
		b[k] = v
	end
	return b
end

-- n messages, each text and a work group of 12 tool rows.
local function long_chat(n: number): { any }
	local messages = {}
	for i = 1, n do
		local rows = {}
		for j = 1, 12 do
			rows[j] = { kind = "tool", id = "t", blocks = { { kind = "text", text = "x" } } }
		end
		messages[i] = { id = "m", blocks = { { kind = "text", text = "a" }, { kind = "group", blocks = rows } } }
	end
	return messages
end

local out: { string } = {}
local function say(name: string, b: any)
	table.insert(out, name .. "=" .. (if b then b.id else "nil"))
end

-- A long chat with no open visual: 20 picks (a scroll's worth).
local long = long_chat(3000)
table.insert(long, 10, { id = "old", blocks = { visual("v-sent") } })
local start = os.clock()
local none: any = nil
for _ = 1, 20 do
	none = VisualPin.pick(long, { ["v-sent"] = { open = false, sent = true } }, {}, {})
end
local ms = (os.clock() - start) * 1000
say("long", none)
table.insert(out, "fast=" .. tostring(ms < 2))

local messages = {
	{ id = "a", blocks = { visual("v1") } },
	{ id = "b", blocks = { { kind = "group", blocks = { visual("v2"), visual("v3", { sent = true }) } } } },
	{ id = "c", blocks = { visual("v4", { dismissed = true }), visual("v5") } },
}
local revs = {
	v1 = { open = true }, v2 = { open = true }, v3 = { open = true },
	v4 = { open = true }, v5 = { open = true },
}
say("newest", VisualPin.pick(messages, revs, {}, {}))
say("v5in", VisualPin.pick(messages, revs, { v5 = false }, {}))
say("v5v2in", VisualPin.pick(messages, revs, { v5 = false, v2 = false }, {}))
say("allin", VisualPin.pick(messages, revs, { v5 = false, v2 = false, v1 = false }, {}))
say("closed", VisualPin.pick(messages, { v5 = { open = false } }, {}, {}))
say("noaway", VisualPin.pick(messages, revs, nil, {}))
local sent = { { id = "a", blocks = { visual("v1", { sent = true }) } } }
say("listed", VisualPin.pick(sent, { v1 = { open = true }, v9 = { open = true } }, {}, { visual("v8"), visual("v9") }))
say("metlisted", VisualPin.pick(sent, { v1 = { open = true } }, {}, { visual("v1") }))
say("listdismissed", VisualPin.pick(sent, { v9 = { open = true } }, {}, { visual("v9", { dismissed = true }) }))
local far = long_chat(3000)
table.insert(far, 2, { id = "far", blocks = { { kind = "group", blocks = { visual("vf") } } } })
say("far", VisualPin.pick(far, { vf = { open = true } }, {}, {}))

ui.state("out", table.concat(out, " "))
return ui.text { text = "visual pin" }
