--!strict
-- A plugin's page built for a picture, apart from what the window shows:
-- an agent's `plugin_status { snapshot = true, page = … }`
-- (src/plugins/stage.rs).
--
-- The host says what to build (`plugins.stage`: { plugin, page, title,
-- width, height, n }, nil for none) and draws this node alone into a
-- texture once the plugin has filled it; then it's gone. So it's built as
-- the page is (the class the plugin's SDK places by, the page it's given),
-- at the size asked, on the window's own colour (the picture has nothing
-- behind it), and stands past the window's edge: laid out, never painted
-- there, under no pointer. Nothing enters or leaves with a motion: it's
-- pictured as it stands.
--
-- `stage` tells the plugin's SDK these nodes are the picture's, so what
-- stands in for what it keeps (`state`) is read here and nowhere else.
--
-- The shell puts it in its page's pane (screens/shell/page). It's a
-- file of the app's own that no template replaces, so every template
-- whose shell has it builds the same thing.

local staged = ui.shared("plugins.stage", nil :: any)

-- (Far past any window's right edge.)
local AWAY = 100000

-- Where the shell holds it: a point at its pane's corner, out of the
-- pane's flow, so the page beside it keeps its room while a picture is
-- taken. What's pictured stands away from that point.
local ANCHOR = { absolute = true, left = 0, top = 0, w = 0, h = 0 }

local function stage(_n: any): Node
	local now = staged:peek()
	if type(now) ~= "table" then
		return ui.column {}
	end
	return ui.column {
		id = "plugin-stage",
		absolute = true,
		left = AWAY,
		top = 0,
		w = now.width,
		h = now.height,
		bg = "$background",
		provide = {
			page = { id = now.page, title = now.title },
			panel = { page = { plugin = now.plugin, id = now.page } },
			stage = { plugin = now.plugin, page = now.page },
		},
		ui.column {
			class = `plugin-page plugin-{now.plugin}`,
			w_full = true,
			h_full = true,
		},
	}
end

return function(): Node
	return ui.presence(function(): any
		local now = staged:get()
		return if type(now) == "table" then now.n else nil
	end, stage, ANCHOR)
end
