-- Keep going's window: says what its service did, in a toast. The work is
-- the service's (service.luau), which runs with no window open.

local zend = require("./zend")("keep-going")

zend.service():on("continued", function(e)
	local title = e.chat
	local session = zend.sessions()[e.chat]
	if session and session.title then
		title = session.title
	end
	zend.toast(`Sent "continue" to {title}.`)
end)
