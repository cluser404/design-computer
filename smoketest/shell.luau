-- The ribbon shell: tabs switch, each tab shows its commands, the tree lists the planes.
fixture("empty")
for _, id in { "tab.features", "tab.sketch", "tab.evaluate", "tab.view" } do
    expect(id)
end
expect("tree.origin")
expect("viewport")

widget("tab.evaluate"):click()
wait_until(function() return diagnostic("app").ui.ribbon_tab == "Evaluate" end)
expect("ribbon.evaluate.mass")

widget("tab.view"):click()
wait_until(function() return diagnostic("app").ui.ribbon_tab == "View" end)
expect("ribbon.view.fit")

widget("tab.features"):click()
wait_until(function() return diagnostic("app").ui.ribbon_tab == "Features" end)
expect("ribbon.feature.extrude")
return { ok = true }
