-- Fillet the top rim of a shaft by clicking its edge in the view (real pointer events).
fixture("shaft")
wait_until(function() return diagnostic("app").volumes.shaft ~= nil end)
local base = diagnostic("app").volumes.shaft

widget("ribbon.feature.fillet"):click()
wait_until(function() return diagnostic("app").draft ~= nil end)
-- The rim is a circle: click it somewhere on the view (the app picks the edge nearest the click).
-- Aim at the centre-front of the viewport, where the top rim of the iso view lies.
local vp = widget("viewport"):state().rect
local function click_at(x, y)
    root():raw_pointer_move({ x = x, y = y })
    root():raw_pointer_button({ x = x, y = y }, "primary", "press")
    root():raw_pointer_button({ x = x, y = y }, "primary", "release")
    wait_for_frames(2)
end
-- Let the view finish fitting the shaft first (the fit runs over the first frames).
wait_for_frames(20)
local found = false
for _, dx in ipairs({ 0, -30, 30, -60, 60 }) do
    for dy = -200, 200, 5 do
        click_at(vp.min.x + (vp.max.x - vp.min.x) / 2 + dx, vp.min.y + (vp.max.y - vp.min.y) / 2 + dy)
        if #diagnostic("app").draft.edges > 0 then found = true break end
    end
    if found then break end
end
assert(found, "an edge of the shaft was picked by clicking in the view")

widget("page.ok"):click()
wait_until(function() return diagnostic("app").draft == nil end)
local after = diagnostic("app")
assert(after.volumes.fillet1 ~= nil and after.volumes.fillet1 < base, "the fillet removed material")
return { before = base, after = after.volumes.fillet1 }
