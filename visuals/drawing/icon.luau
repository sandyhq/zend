-- title: Environment status
-- category: drawing
-- description: A glyph for a lucide icon name, toned.
-- tags: icon, status, glyph
return function()
	return ui.column { gap = 6,
		ui.row { gap = 8, ui.kit.Icon { name = "CheckCircle", tone = "good" }, ui.text { "Production: up" } },
		ui.row { gap = 8, ui.kit.Icon { name = "AlertTriangle", tone = "warn" }, ui.text { "Staging: slow" } },
		ui.row { gap = 8, ui.kit.Icon { name = "X", tone = "bad" }, ui.text { "Preview: down" } },
	}
end
