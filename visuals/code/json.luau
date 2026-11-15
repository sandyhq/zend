-- title: A JSON value
-- category: code
-- description: A value as a tree you open and close; the first level is open.
-- tags: json, tree, explorer, api
return function()
	return ui.kit.Json {
		label = "webhook event",
		value = {
			id = "evt_3PqK2v", type = "invoice.paid", livemode = false, created = 1791580000,
			data = { amount_paid = 4900, currency = "eur", customer = "cus_Q8x1", lines = { { description = "Pro plan × 1", amount = 4900 } } },
			["request id"] = "req_7Ab9",
		},
	}
end
