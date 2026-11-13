-- Account rotate's service: run by Zend's engine on each device that
-- allowed it, with no window open. When the active Claude Code account
-- drops below 10% remaining in any of its windows (its session, its week),
-- it switches to the next saved account that has more left. The switch
-- waits for that provider's running chats to finish (`when_idle`), so a
-- turn is never cut off; Zend paces switches, so this can't spin.

local zend = require("./zend")("account-rotate")

local PROVIDER = "claude-code"
local BELOW = 10

-- The least any of an account's windows has left, in percent.
local function least_left(row: any): number
	local least = 100
	for _, w in row.windows or {} do
		least = math.min(least, w.remaining)
	end
	return least
end

-- The next saved account (after the active one, in Zend's order) with
-- more than BELOW left, or nil.
local function next_account(): (string?, string?)
	local accounts = zend.accounts({ provider = PROVIDER }):await()
	local usage = zend.usage({ provider = PROVIDER }):await()
	local left: { [string]: number } = {}
	for _, row in usage do
		left[row.account] = least_left(row)
	end
	local start = 1
	for i, a in accounts do
		if a.active then
			start = i
		end
	end
	for step = 1, #accounts - 1 do
		local a = accounts[(start - 1 + step) % #accounts + 1]
		if a.switchable and not a.active and (left[a.account] or 100) >= BELOW then
			return a.account, a.email
		end
	end
	return nil, nil
end

zend.on("usage.threshold", function(e)
	local account, email = next_account()
	if not account then
		zend.emit("stuck", { remaining = e.remaining, window = e.window })
		return
	end
	local done = zend.switch_account({ provider = PROVIDER, account = account, when_idle = true }):await()
	zend.emit("rotating", { status = done.status, to = email or account, window = e.window, remaining = e.remaining })
end, { below = BELOW, provider = PROVIDER })

-- Told again once the switch happened (now, or when the chats finished).
zend.on("account.switched", function(e)
	if e.by and e.by.plugin == "account-rotate" then
		zend.emit("rotated", { to = e.email or e.account })
	end
end, { provider = PROVIDER })
