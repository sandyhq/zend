-- title: What fills the bundle
-- category: charts
-- description: File sizes as rectangles, grouped by folder.
-- tags: treemap, size, bundle, disk
return function()
	return ui.kit.Treemap {
		unit = "bytes",
		items = {
			{ path = "core/markdown.ts", size = 74222 },
			{ path = "core/evaluate.ts", size = 29468 },
			{ path = "core/validate.ts", size = 24407 },
			{ path = "core/catalog.ts", size = 21582 },
			{ path = "core/project.ts", size = 17762 },
			{ path = "core/data.ts", size = 13375 },
			{ path = "ui/components/chartTypes.tsx", size = 30271 },
			{ path = "ui/components/references.tsx", size = 29659 },
			{ path = "ui/components/Tests.tsx", size = 27940 },
			{ path = "ui/components/Map.tsx", size = 26134 },
			{ path = "ui/components/Chart.tsx", size = 23272 },
			{ path = "server.ts", size = 15774 },
			{ path = "web/lookups.ts", size = 12767 },
		},
	}
end
