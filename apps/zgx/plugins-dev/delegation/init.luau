-- Delegation, in the window: what its setting says goes to every chat's
-- agent ("session.instructions"), whole, in its system prompt. Followed as
-- it changes in Settings → Delegation; emptied, its entry goes and your
-- agents get no rules from it. Settings shows the text they get now under
-- "What it tells your agents".

local zend = require("./zend")("delegation")

local rules = zend.setting("rules")

-- What it told them last (none yet: `false`), so it tells them only a change.
local told: string | false | nil = false

ui.effect(function()
	local text = rules:get()
	-- (Its values not here yet: nothing to say.)
	if text == nil then
		return
	end
	local words = if type(text) == "string" and string.match(text, "%S") then text else nil
	if told ~= false and told == words then
		return
	end
	told = words
	zend.set_instructions({ text = words })
end)
