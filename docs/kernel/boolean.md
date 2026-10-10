# Design note: general B-rep boolean (Phase A)

Status: **APPROVED 2026-10-10** (D1-D5 as recommended). Phase B (planes only) is implemented, see section 12. Items marked `[OPEN]` need a decision
before Phase B starts. Written 2026-10-10.

## 1. Why

Everything solid today is a *stepped prism*: a stack of slabs along one axis (`dc-features::stepped`).
It is exact and validated, but closed only under same-axis extrudes, cuts and bosses. Hence the
known refusals: a hole in a side face, any angled cut, partial revolve, sweep, loft, shell,
rounding across an axis, cuts on rounded parts. A solid-solid boolean is the missing primitive.

Goal: `boolean(a, b, op) -> Result<Solid, BoolError>` with `op` in {union, subtract, intersect},
pure (immutable inputs, new solid out), deterministic (AGENTS.md 4.5), validated (section 11
"never corrupt, always explain"), and returning `Unsupported` with a reason for anything outside
the supported surface pairs. It never approximates silently.

## 2. Scope by phase

| Phase | Surface pairs supported | New curves | Notes |
|---|---|---|---|
| B | plane / plane | lines | Includes arbitrary orientation, so angled cuts on prismatic parts. |
| C | plane / cylinder, coaxial cylinder / cylinder, parallel-axis cylinder / cylinder only if easy | ellipse arcs, circles, lines | Side holes, cross holes. Skew cylinders (quartic) stay `Unsupported`. |
| later | cone, torus, sphere, NURBS | intersection curves as NURBS | Out of scope here. |

Explicitly **not** attempted: tangent surface contact between non-coincident surfaces that is
not exactly representable (refused as `Unsupported::TangentContact`, with entity names), and
self-intersecting inputs (refused by the validator before the boolean runs).

## 3. Data-model changes (the real cost)

The current kernel (`solid.rs`) has two properties the boolean cannot live with:

1. **Cylinder, cone and torus faces are rectangular parameter patches** (`phi0..phi1`, `h0..h1`).
   A boolean produces faces whose boundary is an arbitrary loop on the surface (e.g. a cylinder
   wall cut by an oblique plane has an elliptical edge). Faces must be bounded by their **loops**,
   with the surface treated as unbounded (periodic in phi for cylinders). The patch bounds become
   a derived bounding box, not the definition.
2. **Curves are `Line` and `Circle` only.** Plane/cylinder at an angle gives an ellipse. Add
   `Curve::Ellipse { center, axis, major_dir, a, b, t0, sweep }`.

Decision `[OPEN] D1`: change `Surface` in place (cylinder/cone/torus lose `phi/h` bounds) versus add
trimmed variants beside the old ones. **Recommendation: change in place.** The old fields are
used by tessellation, area and volume; they are rewritten to integrate over the loops (Green's
theorem on the surface, which also makes area/volume exact for trimmed faces). A compatibility
constructor keeps `extrude`, `revolve`, `round_rim` building the same shapes, and their tests act
as the regression suite for the change. Saved documents are unaffected (the kernel result is a
derived cache; state hashes include the solid digest, so the digest may change once, recorded in
the decision log; alpha, no backward compatibility, per 2026-10-10 entry).

Edges carry a `Curve` and parametrise by arc or angle; add **pcurves** (the edge's curve in each
adjacent face's (u,v) domain)? `[OPEN] D2`. Recommendation: **no pcurves in B/C.** Plane and
cylinder surfaces have closed-form inverse maps (point -> (u,v)), so loops are walked in 3D and
mapped on demand. Revisit when NURBS arrive.

## 4. Algorithm outline (boundary-representation merge)

Inputs A, B, op. Steps, each testable alone:

1. **Broad phase.** Face AABBs (from loops) pairwise; only overlapping pairs continue. Complexity
   proportional to candidate pairs, not model size (AGENTS.md 4.7), via a sorted-sweep on one axis
   in fixed order (deterministic).
2. **Face/face intersection.** For each candidate pair compute intersection curves, as trimmed
   segments of the analytic curve:
   - plane/plane: line (or *coincident* if parallel and the signed distance is within tolerance).
   - plane/cylinder: axis perpendicular to the plane: circle; axis parallel: 0 or 2 lines or
     tangent line; otherwise ellipse.
   - coaxial cylinder/cylinder: none or coincident.
   Each curve is clipped to the interior of both faces by intersecting with their loops
   (point-in-face via exact `orient2d` in the face's (u,v) map).
3. **Imprint.** Split every face by the curves that cross it (edge splitting with new shared
   vertices, then face splitting by walking the planar arrangement in (u,v)). Vertices are merged
   by tolerance in a fixed lexicographic order, never by hash iteration order.
4. **Classify** each split face of A as inside, outside, on-same, on-opposite relative to B (and
   vice versa). Method: ray cast from an interior point of the face with a deterministic
   direction chosen to avoid edges/vertices (retry with the next of a fixed direction list), using
   exact predicates for the crossing tests; coincident faces handled by the on-same / on-opposite
   rule instead of rays.
5. **Select** faces by op (union: A-out + B-out + on-same; subtract: A-out + B-in flipped +
   on-opposite; intersect: A-in + B-in + on-same).
6. **Stitch** into shells and loops; each edge must be used once per direction. Merge adjacent
   coplanar / co-surface faces that came from the same original surface (needed so a box minus a
   flush block returns six faces, not twelve).
7. **Validate** with `Solid::validate()` (manifold, Euler-Poincare, orientation, positive volume,
   boundaries on surfaces). Failure returns `BoolError::Invalid(issues)`; the caller's solid is
   unchanged.

## 5. Robustness policy

- One tolerance policy only: `dc_math::Tolerance` (1e-6 mm, 1e-9 rad). No local epsilons.
- Orientation tests use `dc_math::robust::orient2d` (exact). Add `orient3d` (exact, same
  expansion arithmetic, no FMA, no libm) for plane-side classification.
- Snap-rounding is **not** used. Coincidence decisions are made once, in step 2, and recorded as
  data (a `Coincident` relation between faces), so later steps never re-decide with different
  tolerances (the classic source of non-manifold output).
- Fixed iteration order everywhere (sorted by face/edge id, then by parameter). No `HashMap`
  iteration in results.
- Transcendentals only via `dc_math::det` (libm), enforced by `clippy.toml`.

## 6. Naming (feeds Phase E)

Every output face and edge is named from the *input* names plus the operation, deterministically:
a face piece keeps `<input face name>` when it is the only piece, else `<name>#k` with `k` by
(centroid projected on a fixed axis, then id) order. New faces created by a cut tool are named
`cut:<feature>:<tool face name>` (already the convention). New edges from an intersection are
named `<faceA>|<faceB>` (already the fillet convention); two edges between the same pair get a
`#k` suffix by order along the intersection curve. Phase E tightens this with command-id-based
IDs; this note only fixes that the boolean must **return a provenance map** (output face -> input
face(s)) so Phase E can build names from it.

## 7. Integration plan (Phase D)

- `features` builds the *tool body* for a cut/boss (extruded profile, any plane) and calls the
  boolean against the current solid. The stepped model remains as an oracle: for every existing
  stepped test case, the boolean path must produce the same volume, area, bounds and face count
  (names compared up to the `#k` suffix rule). When both agree on the whole corpus the stepped path
  is deleted. Until then a feature records which path built it (debug query only).
- `feature.cut/boss` lose the "must be parallel to the base sketch" refusal; the refusal text
  becomes `Unsupported(<surface pair>)` from the boolean.
- Rounded rims (`Stepped::rims`) are re-applied today after each cut. With a real boolean, fillets
  become ordinary features in the DAG; the special case is removed in Phase F.

## 8. Test strategy

- **Differential:** every stepped-model test shape built both ways (volume, area, bounds, face and
  edge counts, `validate()` clean).
- **Analytic:** box-minus-box, box-minus-rotated-box, cylinder-through-box (known volumes and
  areas), cylinder across a cylinder wall (exact area from closed form), flush and T-junction
  contact, tool touching only along an edge or at a point (refused, with reason).
- **Property:** random axis-aligned and rotated boxes; volume(A) + volume(B) - volume(A and B) =
  volume(A or B); subtract then union of the cut part restores A; ray parity through the *mesh*
  versus point-in-solid from the B-rep at hundreds of probe points (the method that already found
  real bugs in the stepped model).
- **Fuzz:** random small perturbations near coincidence to exercise the tolerance policy; expected
  result is either a valid solid or a named refusal, never a panic or an invalid solid.
- **Corpus metric:** success rate on a growing hard-case corpus is tracked in CI and must not
  drop (AGENTS.md section 11).
- **Determinism:** golden state hashes for the boolean results added to the cross-platform suite.

## 9. Risks

| Risk | Mitigation |
|---|---|
| Trimmed-face change breaks existing tessellation, area, volume | Do it first inside Phase B as its own step, with the full existing kernel suite as the gate. |
| Classification errors near coincidence | Decide coincidence once (step 2) and store it; exact predicates; fuzz near-coincident. |
| Cylinder tessellation of trimmed faces is not watertight | Share sampled edge points between adjacent faces (already the rule); trimmed cylinder faces triangulated in (phi,h) with loops sampled once. `Mesh::check` stays the gate. |
| Scope creep to NURBS | Hard rule: unsupported pairs return `Unsupported` with names. |
| Performance on large parts | Broad phase and per-face-pair work only; benchmark with `criterion` on 1k-face parts; budgets recorded. |

## 10. Open decisions for sign-off

- D1: trimmed surfaces in place (recommended) or beside the old patches.
- D2: no pcurves in B/C (recommended).
- D3: add `Curve::Ellipse` now (Phase C) rather than a generic parametric curve. Recommended.
- D4: the boolean lives in a new module `kernel/ops/boolean` inside `dc-kernel` (per AGENTS.md
  layout), pure and headless. Recommended.
- D5: Phase B includes the Surface change (section 3) as its first commit, gated by the existing
  suite, before any intersection code. Recommended.

## 11. References

Mantyla, *An Introduction to Solid Modeling* (1988), ch. 12, boundary merge. Hoffmann, *Geometric
and Solid Modeling* (1989), ch. 4. Shewchuk, *Adaptive Precision Floating-Point Arithmetic and Fast
Robust Geometric Predicates* (1997), already used by `dc-math::robust`. Requicha and Voelcker,
*Boolean operations in solid modeling* (1985), regularised set operations.

## 12. Phase B status (2026-10-10)

Implemented in `crates/dc-kernel/src/boolean/` (`mod.rs` operation, `planar.rs` 2D arrangement and
point location, `prep.rs` planar faces and line clipping) and `builder.rs` (`SolidBuilder`,
`Solid::prism`, `Solid::cuboid`). Public API: `Solid::boolean(&other, BoolOp) -> Result<BoolResult,
BoolError>` where `BoolResult` carries the solid and a per-face provenance list.

**Deviation from D5, recorded:** the trimmed-surface change was *not* the first commit of Phase B.
A plane-only boolean does not need it (planar faces are already bounded by loops), so doing it
first would have put the whole existing kernel suite at risk for no benefit yet. It moves to the
start of Phase C, where cylinders need it. Everything else in the note is as written.

What the plane-only implementation does: noding of all segments against all nodes (nodes sorted by
x, so the cost follows nearby candidates), per-face arrangement with exact side tests, interior
sample points by scan line, classification by coincidence test then ray parity with a fixed list of
directions (retrying a direction that grazes an edge), merge of coplanar neighbours by cancelling
opposite half-edges and retracing, removal of vertices that only join two collinear edges.

Checked by: 13 boolean tests (analytic volumes, face counts, flush and shared faces, islands,
rotated and slanted tools, chained operations, determinism, near-coincidence offsets from 1e-9 to
1e-3, refusal of a curved face with its name, 60 random grid-aligned box pairs and 80 randomly
turned box pairs checked against inclusion-exclusion and probe points through the *mesh*); 8
differential tests in `dc-features/tests/boolean_vs_stepped.rs` that rebuild the stepped model's
shapes (pocket, through hole, counterbore, boss with pocket, stacked bosses, split, step) and
compare volume, area, bounds and the area in each plane. A deliberate break of one selection rule
was caught by the tests.

Known limits: planar faces and straight edges only (anything else is `Unsupported` naming the
face); contact along an edge or at a point between the two solids is non-manifold and comes back as
`Invalid` or `Degenerate` (the random tests saw 1 refusal in 60), never as a wrong solid; the
stepped model sometimes splits a flat face the boolean keeps whole, so face counts are compared as
"not more", and the area in each plane is compared exactly; no criterion benchmark yet.

## 13. Phase C status (2026-10-10): trimmed cylinders and ellipse edges done; cylinder boolean not yet

Done (C1, C2), all in `dc-kernel`:

- `Curve::Ellipse { center, axis, x_dir, a, b, t0, sweep }`: points, tangent, length (Gauss), exact
  bounds (extremes per axis), canonical bytes (tag 2), validator checks, sampling like circles.
  Plane faces with ellipse edges get an exact area (closed form, same as for circles).
- **Cylinder faces are bounded by their loops, not by their `phi`/`h` fields.** Area and volume
  come from boundary integrals (`Solid::cylinder_integrals`: `L1 = Σ ∫ h dphi`, `L2 = Σ ∫ G(phi) dh`;
  area `-s r L1`, volume share `r/3 (L2 - r L1)`; closed form for rulings and coaxial circles,
  Gauss-Legendre for ellipses). `Surface::deviation` for a cylinder no longer limits the axial extent.
  The `phi0 phi1 h0 h1` fields remain in the type as a hint (nothing reads them for cylinders);
  they are removed when the boolean starts emitting cylinder faces. **Cone and torus patches are
  unchanged** (patch formulas): they only come from rim rounding and the boolean refuses them.
- Tessellation: a cylinder face that is one strip (two arcs and their sides) keeps the strip path,
  now also with ellipse edges; every other cylinder face goes through
  `triangulate_cylinder_general`: loops unwrapped about the axis (a loop that winds around the axis
  needs a seam edge, as strips have, else it is refused), holes shifted by whole turns into the
  outer loop, a zipper between lower and upper chain when both advance in angle, otherwise ear
  clipping with hole bridging. Triangle winding follows the orientation in the (angle, height)
  plane.

Checked by: 6 tests on a hand-built cylinder cut by a slanted plane (exact area and volume for
several slopes including negative, exact bounds, ellipse length against Ramanujan, watertight mesh,
mesh volume converging as the tolerance shrinks, zipped wall has the same triangle count as the
strip); 1 test that sends every cylinder the extrude and stepped model build (disc, plate with a
hole, D shape downwards, counterbore, boss) through the general path and compares with the strip
path (watertight, outward, volume and area within 1e-6); the full workspace (625 tests) passes.

Limits: a general face with a hole is ear-clipped, so a wide patch can get long triangles (valid
and watertight but a coarse approximation of the curved surface); no interior points are added.
Next (C3): plane/cylinder intersection curves, splitting cylinder faces in the (angle, height)
plane with seam handling, classification on cylinders, and the first cylinder boolean tests.

## 14. Phase C3 status (2026-10-10): the cylinder boolean works

The boolean core was rewritten around curves (`boolean/geo.rs`, `prep.rs`, `cuts.rs`, `build.rs`,
`planar.rs`): edges are directed arcs of lines, circles and ellipses; faces are planes or cylinders
with a flat picture (a plane's own frame; a cylinder's `(σ r φ, h)`, a strip for faces that wind
round the axis, with the seam as left and right edges and seam nodes as two copies); every face's
arrangement is traced on that picture with exact curve geometry kept alongside polylines used only
for decisions (sagitta 2e-7 mm, inside the 1e-6 mm boundary band).

Surface pairs: plane/plane, plane/cylinder (perpendicular: circle; parallel: two lines, or a
tangent line, refused only if the contact meets both faces; oblique: an ellipse from conjugate
semi-diameters), coincident coaxial cylinders, coaxial cylinders of different radius (no contact).
Cylinders that cross are refused (`Unsupported`, named). Cones and tori are refused.

Also: `Solid::cylinder`; tessellation of cylinder faces with holes uses minimum-diagonal ear
clipping (diagonals weighed by their reach round the axis) plus a capped refinement of interior
edges that still span too wide, because plain ear clipping cut chords through a wall with a window
(a 20 % volume error in the mesh, found by a probe test).

Checked by: 11 cylinder tests (hole, blind hole, boss, slab along the axis with exact segment area,
angled cut with an ellipse edge, perpendicular cut, intersection, chained operations, refusal of
crossing cylinders, 60 random block/bar pairs x 3 operations against analytic point-in-solid probes
through the mesh); 4 new differential tests against the stepped model (round hole, counterbore,
boss with pocket, two holes and a boss: volume, area per plane, area per cylinder, bounds); all
earlier planar tests still pass on the new core; a deliberate break of one selection rule fails 6
of the 11 cylinder tests.

Limits: no merging of adjacent faces on one cylinder (a stack of coaxial same-radius cylinders
keeps one wall per piece); a cut whose seam lands inside a window leaves two faces where one would
do; tangent contact between a plane and a cylinder is refused when the contact is real; circles of
the same geometry from different sources are fused only when their parameterisations agree; cost is
about 10 ms per operation in release on the test parts (no benchmark yet).

## 15. Phase D status (2026-10-10): features on the boolean, hybrid form

`dc-features::solid_ops` (`tool_solid`, `cut_with_tool`, `boss_with_tool`, `reach`) builds the tool
as the extruded profile on the sketch plane over an offset span, runs the boolean with a namer that
gives the tool's faces the layered model's naming (`side:cut:<feature>:<entity>`,
`cap_start:<feature>`), and refuses a cut that removes nothing and a boss that floats.
`dc-doc::feature` sends a cut or boss to it when the sketch plane is not parallel to the target's
layered model or the target has none; patterns run one copy at a time (`<feature>~i` names). The
result carries no layered model.

Checked by: 8 side-cut tests in `dc-features` (blind and through holes from a side face, a slot, a
boss on a side face, a floating boss, a cut that misses, a tilted plane, crossing cylinders
refused), 3 more differential tests of tools built from real profiles (annulus, D shape,
bosses) against the layered model, the rewritten dc-doc and e2e tests (a side hole through the
command layer with the right volume and face names), a smoke script that does it in the real
window (`smoketest/side_hole.luau`), and the full workspace (659 tests plus the new ones).

Not done in phase D, by decision: replacing the layered model for cuts and bosses along the axis.
It keeps stable names and the fillet/chamfer/rim code works on it; it can go only when fillet and
chamfer exist on the B-rep (phase F). `flip side to cut` is only available along the axis.

## 16. Phase E status (2026-10-10): stable names for faces and edges

`dc-kernel::naming::structural_names` names every face and edge of a solid from structure, not
from the order the construction visited things in. It is used by the boolean (`finish`) and by the
layered model (`build_solid`), so both paths follow one rule:

* a base name used by one face names it; the base comes from the input (`side:<entity>`,
  `cap_start:<feature>`, `side:cut:<feature>:<entity>`);
* a face that was split into several pieces gets `<base>#<hash>`, the hash being of the sorted base
  names of the faces across the edges of its **outer loop** (a hole inside it changes nothing), so a
  piece keeps its name when its geometry changes and its neighbours do not; clashes are numbered by
  position (`.1`);
* an edge is `<face>|<face>` from the final names of the faces on its two sides; several edges
  between the same pair get `#<hash>` of the faces around their end vertices;
* the boolean returns the solid in canonical order (faces and edges sorted by name, vertices by first
  use), so the same inputs give the same bytes whatever order the faces came in.

The old schemes (`#1` by enumeration order, `@<region>` for walls in pieces) are gone. The
resolver explains a lost reference: asking for a face that was split answers "it was split into
`a#..`, `b#..`" and lists them (`state.resolve_plane`).

Checked by: 6 naming tests in `dc-kernel` (pieces keep their names when a slot moves across the
middle, widens or the part grows; a round hole that moves or grows changes no name; names and areas
do not depend on the order of the input faces; same bytes twice; duplicate-edge names; a chain of
five holes never repeats a name), uniqueness asserted in every boolean test, a layered-model test
(`stepped_names`), and a `dc-doc` test (a sketch on a pocket's floor follows the pocket when it gets
deeper, with identical names; a split face's old name is refused with the pieces listed).

Limits: a piece is named by its outer-loop neighbours, so a later feature that cuts the *boundary*
of the piece (a notch in its edge) renames it; a split gives no piece the old name, so references
to the unsplit face are refused (by design: no guess about which piece was meant); names are hashes,
so they are stable but not readable.
