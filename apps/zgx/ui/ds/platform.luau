--!strict
-- What the device the window runs on can do, where a desktop and a phone
-- differ. Android (the app's own build, apps/zgx/android) has no terminals
-- (their bridge is a process of its own), no web view for the panel's
-- browser, no microphone service for dictation, no updater (the store
-- updates it), no file picker or Finder to show a file in (both need Java
-- the app doesn't have), and no window capture for appshots. Everything
-- else is the same.
--
--   local platform = require("@ds/platform")
--   if platform.has("terminal") then … end     -- a control only where it works
--   platform.android()                         -- the window runs on Android

local M = {}

export type Feature = "terminal" | "browser" | "dictation" | "updates" | "reveal" | "file_picker" | "appshots"

-- Each feature Android doesn't have, and why (the report and tests read it).
M.ANDROID_LACKS = {
	terminal = "terminals run in a bridge process the app can't start",
	browser = "no web view in the app",
	dictation = "no microphone service",
	updates = "the store updates the app",
	reveal = "other apps can't read the app's files",
	file_picker = "the system picker answers through Java",
	appshots = "no capture of other apps' windows",
} :: { [string]: string }

-- Whether the window runs on Android (the host's `platform`, its OS).
function M.android(): boolean
	return ui.shared("platform", ""):peek() == "android"
end

-- Whether this device has `feature`.
function M.has(feature: Feature): boolean
	return not (M.android() and M.ANDROID_LACKS[feature] ~= nil)
end

-- For a control that stays where Android lacks what it needs (a page's
-- button among others): whether `feature` is here; if not, a toast says
-- why, so the press isn't lost.
function M.check(feature: Feature): boolean
	if M.has(feature) then
		return true
	end
	require("@self/toast").show {
		title = "Not on Android",
		body = M.ANDROID_LACKS[feature],
		tone = "warning",
	}
	print(`zend-zgx: {feature} isn't on Android: {M.ANDROID_LACKS[feature]}`)
	return false
end

return M
