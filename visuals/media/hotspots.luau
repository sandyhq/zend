-- title: What's on the dashboard
-- category: media
-- description: Numbered regions on a picture; click one to read about it.
-- tags: hotspots, annotated, image, regions, assets
-- (Show it with assets = { ["dashboard.png"] = ... }.)
local s = ui.state { picked = 0 }
local spots = {
	{ x = 0.03, y = 0.15, w = 0.15, h = 0.42, title = "Sections", detail = "One entry per page; the blue one is open." },
	{ x = 0.23, y = 0.15, w = 0.73, h = 0.22, title = "Stat cards", detail = "Revenue, sign-ups and churn for the last 30 days." },
	{ x = 0.93, y = 0.48, title = "Latest point", detail = "Today's figure; it updates every five minutes.", tone = "warning" },
}
return function()
	return ui.kit.Hotspots {
		src = "dashboard.png",
		alt = "The dashboard",
		spots = spots,
		selected = s.picked > 0 and s.picked or nil,
		on_select = function(i)
			s.picked = i
		end,
	}
end
