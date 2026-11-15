-- title: Unit test run
-- category: dev
-- description: Totals, then each failure with its message; the passes fold.
-- tags: tests, junit, vitest, failures
return function()
	return ui.kit.Tests {
		title = "Unit tests",
		results = {
			{ name = "cart › adds an item", status = "pass", file = "src/cart.test.ts", ms = 4 },
			{ name = "cart › applies a coupon once", status = "fail", file = "src/cart.test.ts", line = 42, ms = 11,
				message = "expect(received).toBe(expected)\n\nExpected: 90\nReceived: 81\n\n    at src/cart.test.ts:47:28" },
			{ name = "cart › removes an item", status = "pass", file = "src/cart.test.ts", ms = 3 },
			{ name = "cart › keeps the cart across reloads", status = "todo", file = "src/cart.test.ts" },
			{ name = "checkout › charges the card", status = "pass", file = "src/checkout.test.ts", ms = 38 },
			{ name = "checkout › retries a declined card", status = "skip", file = "src/checkout.test.ts", message = "needs the payments sandbox" },
		},
	}
end
