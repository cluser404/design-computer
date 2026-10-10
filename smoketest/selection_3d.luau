-- Selecting in the 3D view with real pointer events: a click, Ctrl+click to add, and a Ctrl+drag
-- selection box (to the right takes what is wholly inside).
fixture("plate_with_four_holes")
wait_for_frames(25)
local vp = widget("viewport"):state().rect
local w, h = vp.max.x - vp.min.x, vp.max.y - vp.min.y
local v = root()
local ctrl = { ctrl = true }
local function at(fx, fy) return { x = vp.min.x + w * fx, y = vp.min.y + h * fy } end
local p1 = at(0.5, 0.42)
v:raw_pointer_move(p1); wait_for_frames(2)
v:raw_pointer_button(p1, "primary", "press"); v:raw_pointer_button(p1, "primary", "release"); wait_for_frames(3)
assert(#diagnostic("app").selection == 1, "a click selects one thing")
local p2 = at(0.5, 0.62)
v:raw_pointer_move(p2); wait_for_frames(2)
v:raw_pointer_button(p2, "primary", "press", ctrl); v:raw_pointer_button(p2, "primary", "release", ctrl); wait_for_frames(3)
assert(#diagnostic("app").selection == 2, "Ctrl+click adds")
-- Ctrl+click on the second one again takes it out.
v:raw_pointer_move(p2); wait_for_frames(2)
v:raw_pointer_button(p2, "primary", "press", ctrl); v:raw_pointer_button(p2, "primary", "release", ctrl); wait_for_frames(3)
assert(#diagnostic("app").selection == 1, "Ctrl+click on a selected thing takes it out")
local cam = diagnostic("app").camera_distance
local s, e = at(0.05, 0.05), at(0.95, 0.95)
v:raw_pointer_move(s); wait_for_frames(2)
v:raw_pointer_button(s, "primary", "press", ctrl)
for i = 1, 6 do
  v:raw_pointer_move({ x = s.x + (e.x - s.x) * i / 6, y = s.y + (e.y - s.y) * i / 6 }); wait_for_frames(1)
end
v:raw_pointer_button(e, "primary", "release", ctrl); wait_for_frames(4)
local sel = diagnostic("app").selection
assert(#sel >= 12, "the box took the faces and edges in view: " .. #sel)
assert(diagnostic("app").camera_distance == cam, "a Ctrl+drag does not turn the view")
local faces, edges = 0, 0
for _, s in ipairs(sel) do
  if s.kind == "face" then faces = faces + 1 end
  if s.kind == "edge" then edges = edges + 1 end
end
assert(faces == 3 and edges == 9, "three faces and nine edges: " .. faces .. " / " .. edges)
return { count = #sel }
