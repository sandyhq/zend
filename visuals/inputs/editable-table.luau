-- title: Q3 hardware budget
-- category: inputs
-- description: A table the reader edits, with text, number and yes/no columns, here line items of a budget.
-- tags: table, editable, rows, budget
return function()
	return ui.kit.Form {
		title = "Q3 hardware budget",
		description = "Change amounts, tick what you approve, add rows for anything missing.",
		ui.kit.EditableTable {
			name = "budget",
			label = "Line items",
			columns = {
				{ key = "item", label = "Item" },
				{ key = "qty", label = "Qty", type = "number" },
				{ key = "cost", label = "Cost (€)", type = "number" },
				{ key = "ok", label = "Approved", type = "boolean" },
			},
			value = {
				{ item = "Laptops for new hires", qty = 4, cost = 7600, ok = true },
				{ item = "Monitors", qty = 6, cost = 1800, ok = false },
				{ item = "CI build machine", qty = 1, cost = 3200, ok = false },
			},
		},
	}
end
