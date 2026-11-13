-- Account rotate's window: says what its service did, in a toast. The work
-- is the service's (service.luau), which runs with no window open; this
-- only tells you while Zend is open.

local zend = require("./zend")("account-rotate")

local service = zend.service()

service:on("rotating", function(e)
	if e.status == "waiting" then
		zend.toast(`Claude Code is at {e.remaining}% ({e.window}). Switching to {e.to} when its chats finish.`)
	end
end)

service:on("rotated", function(e)
	zend.toast(`Claude Code switched to {e.to}.`)
end)

service:on("stuck", function(e)
	zend.toast(`Claude Code is at {e.remaining}% ({e.window}), and no other saved account has more left.`)
end)
