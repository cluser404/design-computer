# Design note: features built on the B-rep (phase F)

Status: implemented 2026-10-10. Companion to `boolean.md`. Each section says what is built, how,
what it is checked by, and what is refused.

## 1. Fillet and chamfer of straight edges (`dc-kernel::blend`)

Method: local surgery, no boolean. An edge E between flat faces F1 and F2 is replaced by two edges
a distance `t` away on each face (`t = r / tan(φ/2)` for a fillet of radius `r`, where `φ` is the
angle between the directions into F1 and F2 away from the edge; `t = d` for a chamfer) and a new face
between them: part of a cylinder (axis along E through the point `p0 + bisector * r / sin(φ/2)`) or
a plane. Convex and concave edges are the same construction: the cylinder is tangent to both faces
inside the wedge between them; a convex edge loses material and the new face's normal points away from
the axis (`same_sense = true`), a concave one gains material (`same_sense = false`).

At each end the edge must end at a corner of exactly three flat faces. The third face F3 loses the
corner: the neighbouring edges `g1` (F1∩F3) and `g2` (F2∩F3) are shortened to the points where the
two new edges meet F3, and an arc is inserted between them: the cylinder cut by F3's plane (a circle
if F3 is square to the edge, an ellipse otherwise, `plane_cylinder_section`), or a straight line for a
chamfer. Which of the two arcs is the right one is decided by which one lies on the side of the
original corner. The old edge is dropped, unused vertices and edges are compacted, edge names are
recomputed structurally, and the result must pass `validate()` and change the volume the right way
(convex: smaller, concave: larger) or the size is refused as too large.

Checked by 9 kernel tests (exact volumes for a block edge, a chamfer, a top edge, an inside corner that
gains material, a 60 degree corner against `r²(cot(φ/2) − (π−φ)/2)`, a slanted prism whose ends cut an
ellipse, two edges at once, the refusals, and a rounded part going on through the boolean), 1 `dc-doc`
test through commands (a top edge, then another), and the existing rounding tests.

Refused with the reason: curved edges (a circular rim is rounded by `round_rim`), edges that share a
vertex (a corner patch needs a sphere), an end with more or fewer than three faces, an end on a curved
face, a radius that reaches past a neighbouring edge, flat or folded-back edges.

Routing in `dc-doc`: an edge along the part's axis on a part that still has a layered model is rounded
in the layered model as before (keeps its names and rim handling); anything else is rounded here. A
part rounded here has no layered model.

## 2. Partial revolve (`dc-features::revolve_partial`)

A profile turned by an angle in `(0, 360)` degrees about an axis of its sketch, built directly: the two
caps (`cap_start` in the sketch plane, `cap_end`), a cylinder patch for every edge parallel to the axis,
a flat sector for every edge square to it, no face for an edge on the axis (the caps share it). Vertices
on the axis are single. The orientation of every face follows from one choice (which way the start cap
faces) and the shared-edge rule, and the sense of each wall from which side of the edge the material is.
`FeatureSpec::Revolve` gained `angle: Option<f64>` (degrees, omitted for a full turn; a full turn still
uses the layered model). Refused: profile across the axis, slanted edges (cones), curves.

Checked by 4 kernel-level tests (quarter, half, three-quarter and 30 degree sectors against
`θ/2 (R²−r²) h`, a profile touching the axis, opposite axis direction, refusals; watertight meshes) and a
`dc-doc` test (exact, follows a dragged sketch, undo/redo/replay hash, refusals).

## 3. Shell (`dc-features::shell`)

Only prism-shaped parts: two flat end faces facing away from each other, between them only flat faces
square to them and cylinders along the axis. The cavity is the end face's outline moved inwards by the
thickness (lines sideways, circle arcs change radius, corners at the intersections of the moved
neighbours, found with the boolean's curve intersection), extruded between the ends (kept `thickness`
from a closed end, going out through an open one) and subtracted with the boolean. Open: none, one or
both ends; with none the part gets an inner shell. New faces are `side:shell:<feature>:<edge>`,
`cap_start:<feature>`, `cap_end:<feature>`. Refused: a part with steps, side holes or edges rounded across
the axis; faces that are not ends; a wall too thick for the part; a corner that cannot be moved sharply.

Checked by 6 tests (tray, hollow, tube, round part, profile with a hole, refusals), 3 through commands
(follows its sketch, thickness through `set_depth`, goes on through the boolean) and a smoke script on
the real window.

## 4. Loft and sweep (command layer)

* **Loft** joins two closed outlines of straight lines with the same number of corners by flat faces
  (`Solid::loft`): both are turned counter-clockwise about the direction from one to the other, the top
  is rotated to the start that keeps matching corners closest, and every side must be flat or the loft
  is refused as twisted (corresponding edges parallel or meeting). It reads two sketches, so
  `FeatureSpec::sketches()` lists both and editing either rebuilds it.
* **Sweep** pushes one profile along a straight line of another sketch. Straight along the profile's
  normal it is an extrusion (any profile); at an angle it is an oblique prism, and only for straight
  edges (a circle swept at an angle is an oblique cylinder, which the kernel does not have).

Checked by 2 kernel tests (a frustum with exact volume in either winding, a twisted loft refused) and 3
`dc-doc` tests (follows both sketches, undo/replay, refusals, exact oblique volume by Cavalieri).

Not built: curved lofts and sweeps along arcs or splines (they need ruled, swept or free-form surfaces,
tori and oblique cylinders) and a page for either in the app (they are made with commands for now).

## 5. Prerequisites added to the boolean on the way

* Cylinders with parallel axes: intersection lines (two, or one when they touch).
* Faces on one plane or cylinder that share boundary are merged (also across a cylinder's seam, which
  keeps the number of faces canonical); a piece a face uses twice itself (a seam) is never cancelled.
