-- title: Migration details
-- category: layout
-- description: A section the reader opens and closes.
-- tags: details, collapsible, disclosure
return function()
	return ui.kit.Card {
		title = "Migration plan",
		ui.text { "Rewrites the orders table in place; about 4 minutes on 2 GB." },
		ui.kit.Details { title = "What it changes",
			ui.kit.List { items = { "orders.total becomes a decimal", "orders.state gets an index", "the legacy_orders view is dropped" } },
		},
		ui.kit.Details { title = "How to roll back", open = true,
			ui.text { "Restore the snapshot taken before the run; the migration keeps it for 7 days." },
		},
	}
end
