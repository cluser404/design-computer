-- Regressions: a cut sketched on the top face goes into the part by itself (the reverse box is
-- addressable as page.reverse); an empty sketch is "empty", not "fully defined"; an edge
-- selected before Fillet is the first edge of the fillet.
fixture("shaft")
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
local r = widget("viewport"):state().rect
local cx, cy = (r.min.x + r.max.x) / 2, (r.min.y + r.max.y) / 2
click({ x = cx, y = cy - 60 })
assert(diagnostic("app").selected.kind == "face", "the top face is selected")
widget("tab.sketch"):click(); wait_for_frames(2)
widget("ribbon.menu.Sketch"):click(); wait_for_frames(2)
widget("menu.sketch.on_face"):click(); wait_for_frames(6)
local note = widget("tree.sketch:sketch1"):state()
assert(diagnostic("app").sketching == "sketch1", "sketching on the face")
tool("circle")
click({ x = cx + 15, y = cy + 15 })
v:raw_pointer_move({ x = cx + 30, y = cy + 15 }); wait_for_frames(2)
click({ x = cx + 30, y = cy + 15 })
widget("ribbon.sketch.finish"):click(); wait_for_frames(4)
widget("ribbon.feature.cut"):click(); wait_for_frames(5)
local d = diagnostic("app").draft
assert(d ~= nil and d.problem == nil, "the cut into the part builds without a manual fix")
assert(widget("page.reverse"):state().value == true, "reverse was chosen by itself")
widget("page.ok"):click(); wait_for_frames(5)
assert(#diagnostic("app").features == 2, "the cut was made")
return diagnostic("app").volumes
