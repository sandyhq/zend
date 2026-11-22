-- Connect to a computer, the phone's (Android) whole setup: reaching a
-- computer's engine by its direct code, scanned or pasted, as Settings →
-- Devices does it (settings/phone_join.luau, the same view and state).
-- Continue waits until that engine answers, then opens the main window.

local ds = require("@ds")
local Onboarding = ds.Onboarding
local state = require("./state")
local phone_join = require("@app/screens/settings/phone_join")

export type Props = { on_back: () -> (), on_continue: () -> () }

return function(props: Props): Node
	return Onboarding.Step {
		-- (Back is Sign in, which a signed-in run has passed; a phone's setup
		-- has nothing before it.)
		back = if state.path:peek() == "signed_in" or state.on_phone() then nil else Onboarding.Back { on_click = props.on_back },
		continue = {
			label = ui.t("common.continue") :: any,
			disabled = function(): boolean
				return not phone_join.connected:get()
			end,
			on_click = props.on_continue,
		},
		Onboarding.Form {
			wide = true,
			title = ui.t("onboarding.connect.title") :: any,
			body = ui.t("onboarding.connect.body") :: any,
			ui.component(phone_join.view)({}),
			-- Linked to a computer: the account's steps are the computer's
			-- (state.on_computer), and the next steps pass over them.
			ui.show(function(): boolean
				return phone_join.connected:get() and state.on_computer()
			end, ui.text {
				id = "onboarding-account-on-computer",
				ui.t("onboarding.connect.account_on_computer"),
				text_size = "$font.size.sm",
				line_height = "$font.line.sm",
				color = "$text.secondary",
				wrap = true,
			}),
		},
	}
end
