-- A tray from the Shell page (open top, 2.5 mm walls; a decimal because the box is a float box).
fixture("plate_with_four_holes")
wait_for_frames(3)
local before = diagnostic("app").volumes["plate"]
widget("ribbon.feature.shell"):click(); wait_for_frames(5)
local d = diagnostic("app").draft
assert(d ~= nil and d.kind == "Shell", "the shell page is open")
widget("page.face.cap_end"):click(); wait_for_frames(3)
widget("page.thickness"):set_value(2.5); wait_for_frames(3)
d = diagnostic("app").draft
assert(d.problem == nil, "the shell builds: " .. tostring(d.problem))
widget("page.ok"):click(); wait_for_frames(5)
local a = diagnostic("app")
assert(#a.features == 2, "the shell was made")
local shell = a.volumes["shell1"]
-- The plate is 40 x 20 x 10; walls 2.5 thick, open top: 8000 - 35 * 15 * 7.5.
assert(math.abs(shell - 4062.5) < 1e-6, "the volume is exact: " .. tostring(shell))
return a.volumes
