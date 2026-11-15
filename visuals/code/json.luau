-- title: A JSON value
-- category: code
-- description: A value as a tree you open and close; the first level is open. ui.null shows null; text = "<json>" reads a JSON text in its own key order.
-- tags: json, tree, explorer, api, null
return function()
	return ui.column {
		gap = 16,
		ui.kit.Json {
			label = "webhook event",
			value = {
				id = "evt_3PqK2v", type = "invoice.paid", livemode = false, created = 1791580000,
				data = { amount_paid = 4900, currency = "eur", customer = "cus_Q8x1", coupon = ui.null, lines = { { description = "Pro plan × 1", amount = 4900 } } },
				["request id"] = "req_7Ab9",
			},
		},
		ui.kit.Json {
			label = "the API's reply, as text",
			open = 2,
			text = [[{"status": "ok", "user": {"id": 42, "email": null, "roles": ["admin", null]}, "next": null}]],
		},
	}
end
