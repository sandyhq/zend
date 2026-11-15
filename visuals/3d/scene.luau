-- title: Monthly rainfall
-- category: 3d
-- description: A 3D bar chart from a list of objects; hover a bar, drag to turn.
-- tags: scene, 3d, bars
local months = { "Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec" }
local cities = {
	{ name = "Lisbon", color = "chart1", mm = { 100, 87, 59, 64, 53, 17, 4, 6, 33, 99, 111, 127 } },
	{ name = "London", color = "chart2", mm = { 55, 41, 41, 44, 49, 45, 45, 50, 49, 69, 59, 55 } },
}
local objects = {}
for row, city in ipairs(cities) do
	for month, mm in ipairs(city.mm) do
		local x, z = month * 0.5 - 3.25, row * 0.9 - 1.35
		local height = math.max(mm / 100, 0.02)
		table.insert(objects, { shape = "box", at = { x, height / 2, z }, size = { 0.34, height, 0.5 }, color = city.color })
		table.insert(objects, { shape = "point", at = { x, height + 0.1, z }, radius = 3, color = city.color, hint = city.name .. ", " .. months[month] .. ": " .. mm .. " mm" })
	end
	table.insert(objects, { shape = "label", at = { -3.9, 0.05, row * 0.9 - 1.35 }, text = city.name, color = city.color })
end
table.insert(objects, { shape = "grid", size = 4, step = 1 })
return function()
	return ui.kit.Scene { height = 320, camera = { yaw = 0.45, pitch = 0.42 }, objects = objects }
end
