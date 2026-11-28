-- ds/visual_pin's pick (on ui/visual_pin_fixture.luau): the newest open
-- visual whose place is out of view; and a long transcript with no open
-- visual isn't walked (walking every block once ran out of a script call's
-- time budget in a long chat and took the composer with it).

local function out(): string
	local app = mount("visual_pin_fixture.luau")
	app:frames(2)
	return app:state("out")
end

test("visual pin: a long chat with no open visual is not walked", function()
	local o = out()
	expect(o).to_contain("long=nil")
	expect(o).to_contain("fast=true")
end)

test("visual pin: the newest open one out of view wins; sent, dismissed and in-view ones don't", function()
	local o = out()
	for _, want in { "newest=v5", "v5in=v2", "v5v2in=v1", "allin=nil", "closed=nil", "noaway=v5", "far=vf" } do
		expect(o).to_contain(want)
	end
end)

test("visual pin: one not in the loaded messages comes from the host's list", function()
	local o = out()
	for _, want in { "listed=v9", "metlisted=v1", "listdismissed=nil" } do
		expect(o).to_contain(want)
	end
end)
