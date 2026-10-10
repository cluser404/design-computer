-- The Loft and Sweep pages, from the ribbon: two sketches in, one solid out, exact volumes.
fixture("two_sketches")
wait_for_frames(3)
widget("ribbon.feature.loft"):click(); wait_for_frames(5)
local d = diagnostic("app").draft
assert(d ~= nil and d.kind == "Loft", "the loft page is open")
assert(d.problem == nil, "the loft builds: " .. tostring(d.problem))
widget("page.ok"):click(); wait_for_frames(5)
local a = diagnostic("app")
-- Frustum with rectangular ends 800 and 200 mm2, 30 mm apart: 30/3 * (800 + 200 + 400).
assert(math.abs(a.volumes["loft1"] - 14000) < 1e-6, "loft volume: " .. tostring(a.volumes["loft1"]))
widget("ribbon.feature.sweep"):click(); wait_for_frames(5)
d = diagnostic("app").draft
assert(d ~= nil and d.kind == "Sweep", "the sweep page is open")
assert(d.problem == nil, "the sweep builds: " .. tostring(d.problem))
widget("page.ok"):click(); wait_for_frames(5)
a = diagnostic("app")
assert(#a.features == 2, "both features were made")
return a.volumes
