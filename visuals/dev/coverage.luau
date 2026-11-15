-- title: Coverage report
-- category: dev
-- description: Total coverage, then the least covered files first.
-- tags: coverage, tests, lcov
return function()
	return ui.kit.Coverage {
		title = "Coverage",
		report = {
			files = {
				{ path = "src/cart/coupon.ts", covered = 12, total = 40 },
				{ path = "src/cart/price.ts", covered = 61, total = 80 },
				{ path = "src/checkout/pay.ts", covered = 150, total = 180 },
				{ path = "src/api/users.ts", covered = 96, total = 100 },
				{ path = "src/util/format.ts", covered = 30, total = 30 },
			},
		},
	}
end
