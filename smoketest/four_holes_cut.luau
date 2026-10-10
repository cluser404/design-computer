-- A cut with a sketch of four circles removes all four holes (it once removed only one).
fixture("plate_with_four_holes")
wait_until(function() return diagnostic("app").volumes.plate ~= nil end)
local before = diagnostic("app")
local plate = before.volumes.plate
assert(math.abs(plate - 8000) < 1e-6, "plate volume " .. tostring(plate))

-- Select the sketch in the tree, start the cut: the page lists the four regions, all used.
widget("tree.sketch:holes"):click()
widget("ribbon.feature.cut"):click()
wait_until(function() return diagnostic("app").draft ~= nil end)
local draft = diagnostic("app").draft
assert(draft.kind == "Cut" and #draft.profiles == 0, "every region is used by default")
expect("page.ok")
local boxes = 0
for _, w in widget("page.ok"):viewport():widget_list({ id_prefix = "page.profile." }) do boxes += 1 end
assert(boxes == 4, "four regions are listed, got " .. tostring(boxes))

widget("page.ok"):click()
wait_until(function() return diagnostic("app").draft == nil end)
local after = diagnostic("app")
local removed = plate - after.volumes.cut1
local want = 4 * math.pi * 4 * 10
assert(math.abs(removed - want) < 1e-6, "removed " .. tostring(removed) .. ", wanted " .. tostring(want))
return { removed = removed }
