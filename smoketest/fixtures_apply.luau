-- Fixtures replace the document: the demo part shows up in the design tree and the snapshot.
fixture("empty")
local empty = diagnostic("app")
assert(#empty.features == 0, "empty has no features")

fixture("demo")
wait_for_frames(3)
local demo = diagnostic("app")
assert(#demo.features > 0, "demo has features")
expect("tree.root")
return { features = demo.features, revision = demo.revision }
