-- Delegation's service: run by Zend's engine with no window open. The
-- first time it runs on a device, it tells every chat's agent the rules
-- (rules.luau, the same as its setting's default), so a device whose window
-- never opened still has them. After that it leaves them be: the window
-- changes them as you change its setting, emptied too.

local zend = require("./zend")("delegation")
local RULES = require("./rules")

local seeded = zend.store("seeded", false)

zend.task(function()
	if seeded:get() then
		return
	end
	local mine = zend.instructions():await()
	if #mine == 0 then
		zend.set_instructions({ text = RULES }):await()
	end
	seeded:set(true)
end)
