-- Connect to a computer, on the phone (Android) the step after sign-in:
-- joining a computer's relay by its code, scanned or pasted, as Settings →
-- Devices does it (settings/phone_join.luau, the same view and state). The
-- steps after it ask the relay, so Continue waits until it answers (a
-- relay this phone had already, answering, counts).

local ds = require("@ds")
local Onboarding = ds.Onboarding
local state = require("./state")
local phone_join = require("@app/screens/settings/phone_join")

export type Props = { on_back: () -> (), on_continue: () -> () }

return function(props: Props): Node
	return Onboarding.Step {
		-- (Back is Sign in, which a signed-in run has passed.)
		back = if state.path:peek() == "signed_in" then nil else Onboarding.Back { on_click = props.on_back },
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
		},
	}
end
