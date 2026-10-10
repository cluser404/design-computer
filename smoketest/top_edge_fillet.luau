-- Round an edge across the part's axis (the top edge of a plate): built by the B-rep surgery,
-- which the layered model could not do.
fixture("plate_with_four_holes")
wait_for_frames(20)
local before = diagnostic("app").volumes["plate"]
widget("ribbon.feature.fillet"):click()
wait_until(function() return diagnostic("app").draft ~= nil end)
wait_for_frames(10)
local vp = widget("viewport"):state().rect
local function click_at(x, y)
  root():raw_pointer_move({ x = x, y = y }); wait_for_frames(1)
  root():raw_pointer_button({ x = x, y = y }, "primary", "press")
  root():raw_pointer_button({ x = x, y = y }, "primary", "release")
  wait_for_frames(2)
end
local found = false
local w, h = vp.max.x - vp.min.x, vp.max.y - vp.min.y
-- The plate's top front edge runs across the middle of the isometric view: scan for it.
for fy = 0.40, 0.75, 0.01 do
  for fx = 0.25, 0.75, 0.04 do
    click_at(vp.min.x + w * fx, vp.min.y + h * fy)
    local es = diagnostic("app").draft.edges
    if #es > 0 and (es[1][1] == "cap_end" or es[1][2] == "cap_end") then found = true break end
    if #es > 0 then click_at(vp.min.x + w * fx, vp.min.y + h * fy) end -- take a wrong edge back out
  end
  if found then break end
end
assert(found, "a top edge could be picked")
widget("page.ok"):click(); wait_for_frames(5)
local a = diagnostic("app")
assert(#a.features == 2, "the fillet was made")
assert(a.volumes["fillet1"] < before, "material was removed")
return a.volumes
