-- Words that must not be cut off (src/wrap_tests.rs): a field's label and a
-- visual form's question longer than a narrow column, and a group of tool
-- calls, one short, one a command that runs to two lines and one to more.
-- A look in a window: `zend-zgx fixture=wrap_fixture.luau --inspect`.

ui.theme.strict("warn")
local ds = require("@ds")
ds.setup()
local Activity = require("@ds/activity")
local Field = require("@ds/field")
local Visual = require("@ds/visual")

local QUESTION = "Welche der Projekte gov-sess, chatgpt-intercom, zend-wrap und pi-setup sind für die Migration vorgesehen?"
local SOURCE = [[
return function()
  return ui.column { gap = 12,
    ui.checklist { name = "which", label = "Welche der Projekte gov-sess, chatgpt-intercom, zend-wrap und pi-setup sind für die Migration vorgesehen?", options = { "gov-sess", "zend-wrap" } },
    ui.field { name = "why", label = "Warum gerade diese Projekte jetzt und nicht erst nach dem Release im Herbst?" },
    ui.choice { name = "inbox", label = "Ein Eingang für alles. Jeder Gedanke geht als eine Nachricht an eine Stelle, egal zu welchem Projekt. Der Agent ordnet ihn zu.", options = { "Ja", "Nein", "Anders" } },
    ui.kit.Form { title = "Flow-Thesen", submit = "Add answers",
      ui.kit.Radio { name = "capture", label = "Einfangen unterbricht nicht. Ein Gedanke zu Projekt B, während A läuft, wird nur festgehalten. A läuft weiter.", options = { "Ja", "Nein", "Anders" } },
    },
  }
end
]]

local TWO = "cd ~/Projects/zend-wrap && git log --oneline -5 && git status"
local MANY = "cd ~/Projects/zend-wrap && git log --oneline -5 && git status && cargo check -p zend-engine && cargo nextest run --no-fail-fast -p zend-engine && cargo test --doc -p zend-engine && git diff --stat"

local tools_list = {
	{ id = "t1", kind = "read", verb = "Read", target = "chat.rs" },
	{ id = "t2", kind = "exec", verb = "Ran", target = "cargo test", call = "cargo test" },
	{ id = "t3", kind = "exec", verb = "Ran", target = TWO, call = TWO },
	{ id = "t4", kind = "exec", verb = "Ran", target = MANY, call = MANY },
}

return ui.scroll {
	w_full = true,
	h_full = true,
	ui.column {
		id = "narrow",
		w = 360,
		p = "$space.4",
		gap = "$space.6",
		ui.column {
			id = "wrap-field",
			w_full = true,
			Field.wrap {
				label = QUESTION,
				box = ui.div { w_full = true, h = 32 },
			},
		},
		ui.column {
			id = "wrap-visual",
			w_full = true,
			Visual { id = "v:fixture-wrap", title = "Wrap", source = SOURCE },
		},
		ui.column {
			id = "wrap-activity",
			w_full = true,
			Activity {
				tools = function()
					return tools_list
				end,
				live = function()
					return true
				end,
				summary = function()
					return "Ran commands"
				end,
			},
		},
	},
}
