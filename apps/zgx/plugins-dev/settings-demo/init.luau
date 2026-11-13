-- Greeter: a greeting in the title bar ("ui.add", at #titlebar-actions),
-- set up on its own page in Settings (zend-plugin.json's "settings":
-- Zend draws the page and keeps what you set, for this plugin alone).
-- It reads its settings as they change (`zend.setting(id):get()`), and
-- hears each change (`zend.on_setting`), here said in a toast.

local zend = require("./zend")("settings-demo")

local show = zend.setting("show")
local words = zend.setting("words")
local tone = zend.setting("tone")

zend.on_setting(function(id: string, value: any, _was: any)
	zend.toast(`Greeter: {id} is now {tostring(value)}`)
end)

-- What Zend says beside the Greeting: a warning while it says nothing.
local function check_words()
	local said = words:peek() or ""
	if string.match(said, "^%s*$") then
		zend.set_status("words", { badge = "Empty", tone = "warning", text = "Say something, or it greets with nothing." })
	else
		zend.set_status("words", nil)
	end
end
check_words()
zend.on_setting(function(id: string)
	if id == "words" then
		check_words()
	end
end)

zend.place {
	at = "#titlebar-actions",
	after = function(): Node
		return ui.text {
			id = "greeter",
			function(): string
				if not show:get() then
					return ""
				end
				local said = words:get() or ""
				return if tone:get() == "excited" then `{said}!` else said
			end,
			text_size = "$font.size.sm",
			color = "$text.secondary",
		}
	end,
}
