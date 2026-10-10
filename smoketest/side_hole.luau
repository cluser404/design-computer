-- A sketch on a side face and a cut from it: built by the general boolean (the layered model
-- cannot cut across its own axis). The cut starts into the part by itself and removes a round
-- hole's worth of volume.
fixture("plate_with_four_holes")
wait_for_frames(3)
local v = root()
local function tool(name)
  -- The ribbon folds the Draw group into a menu in a narrow window.
  local b = try_widget("ribbon.sketch.tool." .. name)
  if b == nil then
    widget("ribbon.menu.Draw"):click(); wait_for_frames(2)
    b = widget("menu.sketch.tool." .. name)
  end
  b:click(); wait_for_frames(2)
end
local function click(p)
  v:raw_pointer_move(p); wait_for_frames(2)
  v:raw_pointer_button(p, "primary", "press"); v:raw_pointer_button(p, "primary", "release")
  wait_for_frames(3)
end
local before = diagnostic("app").volumes["plate"]
local r = widget("viewport"):state().rect
local w, h = r.max.x - r.min.x, r.max.y - r.min.y
-- Find a side face of the part by clicking where the isometric view shows one.
local picked = false
for _, f in ipairs({ { 0.40, 0.62 }, { 0.5, 0.65 }, { 0.35, 0.55 }, { 0.8, 0.6 }, { 0.75, 0.65 } }) do
  click({ x = r.min.x + w * f[1], y = r.min.y + h * f[2] })
  local sel = diagnostic("app").selected
  if sel.kind == "face" and string.find(sel.face, "side") ~= nil then
    picked = true
    break
  end
end
assert(picked, "a side face could be picked")
widget("tab.sketch"):click(); wait_for_frames(2)
widget("ribbon.menu.Sketch"):click(); wait_for_frames(2)
widget("menu.sketch.on_face"):click(); wait_for_frames(6)
tool("circle")
local cx, cy = (r.min.x + r.max.x) / 2, (r.min.y + r.max.y) / 2
click({ x = cx, y = cy })
v:raw_pointer_move({ x = cx + 25, y = cy }); wait_for_frames(2)
click({ x = cx + 25, y = cy })
widget("ribbon.sketch.finish"):click(); wait_for_frames(4)
widget("ribbon.feature.cut"):click(); wait_for_frames(6)
local d = diagnostic("app").draft
assert(d ~= nil and d.sketch == "sketch1", "the cut uses the new sketch")
assert(d.problem == nil, "the side cut builds as it opens")
assert(widget("page.reverse"):state().value == true, "it goes into the part by itself: problem " .. tostring(d.problem) .. ", profiles " .. #d.profiles .. ", reverse " .. tostring(d.reverse))
widget("page.ok"):click(); wait_for_frames(5)
local a = diagnostic("app")
assert(#a.features == 2, "the cut was made")
assert(a.volumes["cut1"] < before, "material was removed")
return a.volumes
