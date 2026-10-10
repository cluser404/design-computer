//! Triangle meshes from solids, and binary/ASCII STL output.
//!
//! The mesh is **watertight by construction**: every B-rep edge is sampled exactly once and both
//! faces that meet there use the same vertices, so each mesh edge ends up shared by exactly two
//! triangles running in opposite directions. [`Mesh::check`] verifies that independently.

use crate::solid::{Face, Id, Solid, Surface};
use dc_math::robust::{orient2d, orient2d_sign};
use dc_math::{det, Vec2, Vec3};
use std::collections::BTreeMap;

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct TessParams {
    /// Maximum distance between a true curve and its polyline approximation, in mm.
    pub chord_tol: f64,
    /// Maximum angle covered by one segment of a curve, in radians.
    pub max_angle: f64,
}

impl Default for TessParams {
    fn default() -> Self {
        TessParams {
            chord_tol: 0.01,
            max_angle: core::f64::consts::PI / 8.0,
        }
    }
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Mesh {
    pub positions: Vec<Vec3>,
    /// Counter-clockwise seen from outside the solid.
    pub triangles: Vec<[u32; 3]>,
    /// Which solid face each triangle came from (index into `Solid::faces`).
    pub face_of: Vec<u32>,
}

#[derive(Clone, Debug, PartialEq)]
pub enum MeshError {
    /// A planar face could not be triangulated (degenerate or self-touching boundary).
    Triangulation(String),
}

impl std::fmt::Display for MeshError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            MeshError::Triangulation(m) => write!(f, "triangulation failed: {m}"),
        }
    }
}
impl std::error::Error for MeshError {}

/// Number of segments for an arc of `sweep` radians and `radius`.
fn segments(sweep: f64, radius: f64, p: TessParams) -> usize {
    let by_angle = (sweep / p.max_angle).ceil();
    let half = (1.0 - p.chord_tol / radius).clamp(-1.0, 1.0);
    let step = 2.0 * det::acos(half);
    let by_chord = if step > 1e-9 {
        (sweep / step).ceil()
    } else {
        by_angle
    };
    by_angle.max(by_chord).max(1.0) as usize
}

impl Solid {
    /// How many segments each circular edge is sampled with. The two boundary arcs of a strip
    /// (a cylinder, cone or torus patch) must be sampled alike, point for point, so the arcs of
    /// one strip, and of strips that share an arc, all take the finest count among them.
    fn circle_segment_counts(&self, params: TessParams) -> Vec<usize> {
        let n = self.edges.len();
        let mut count: Vec<usize> = self
            .edges
            .iter()
            .map(|e| match e.curve.range() {
                Some((_, sweep)) => segments(sweep, e.curve.max_radius(), params),
                None => 1,
            })
            .collect();
        let mut parent: Vec<usize> = (0..n).collect();
        fn root(parent: &mut [usize], mut i: usize) -> usize {
            while parent[i] != i {
                parent[i] = parent[parent[i]];
                i = parent[i];
            }
            i
        }
        for f in &self.faces {
            if matches!(f.surface, Surface::Plane { .. }) {
                continue;
            }
            let Some(lp) = f.loops.first() else { continue };
            let arcs: Vec<usize> = lp
                .iter()
                .filter(|c| {
                    self.edges[c.edge as usize].curve.is_curved()
                        && lp.iter().filter(|d| d.edge == c.edge).count() == 1
                })
                .map(|c| c.edge as usize)
                .collect();
            for w in arcs.windows(2) {
                let (a, b) = (root(&mut parent, w[0]), root(&mut parent, w[1]));
                if a != b {
                    parent[a] = b;
                }
            }
        }
        let mut best = vec![0usize; n];
        for (i, c) in count.iter().enumerate() {
            let r = root(&mut parent, i);
            best[r] = best[r].max(*c);
        }
        for (i, c) in count.iter_mut().enumerate() {
            let r = root(&mut parent, i);
            if self.edges[i].curve.is_curved() {
                *c = best[r];
            }
        }
        count
    }

    /// Triangulates every face. Deterministic: the same solid always gives the same mesh.
    pub fn tessellate(&self, params: TessParams) -> Result<Mesh, MeshError> {
        self.tessellate_with(params, false)
    }

    /// `force_general` sends every cylinder face through the general (trimmed) path instead of
    /// the strip path; used by tests to check that the two agree.
    #[doc(hidden)]
    pub fn tessellate_with(
        &self,
        params: TessParams,
        force_general: bool,
    ) -> Result<Mesh, MeshError> {
        let mut mesh = Mesh::default();
        // Mesh vertex index of each solid vertex.
        let mut vmap: Vec<u32> = Vec::with_capacity(self.vertices.len());
        for &p in &self.vertices {
            vmap.push(mesh.positions.len() as u32);
            mesh.positions.push(p);
        }
        // Every edge is sampled exactly once, in its own direction; both faces that meet at the
        // edge use these same mesh vertices, which is what makes the mesh watertight.
        let counts = self.circle_segment_counts(params);
        let mut edge_chain: Vec<Vec<u32>> = Vec::with_capacity(self.edges.len());
        for (i, e) in self.edges.iter().enumerate() {
            let mut chain = vec![vmap[e.v0 as usize]];
            if e.curve.is_curved() {
                let n = counts[i];
                for k in 1..n {
                    chain.push(mesh.positions.len() as u32);
                    mesh.positions
                        .push(self.edge_point(i as Id, k as f64 / n as f64));
                }
            }
            chain.push(vmap[e.v1 as usize]);
            edge_chain.push(chain);
        }
        // A coedge's polyline in loop direction, without its final vertex (the next one starts there).
        let coedge_chain = |c: crate::solid::CoEdge| -> Vec<u32> {
            let mut chain = edge_chain[c.edge as usize].clone();
            if c.reversed {
                chain.reverse();
            }
            chain.pop();
            chain
        };

        for (fi, face) in self.faces.iter().enumerate() {
            let chains: Vec<Vec<u32>> = face
                .loops
                .iter()
                .map(|lp| lp.iter().flat_map(|&c| coedge_chain(c)).collect())
                .collect();
            let before = mesh.triangles.len();
            match &face.surface {
                Surface::Plane {
                    origin,
                    normal,
                    x_dir,
                } => {
                    triangulate_planar(&mut mesh, face, &chains, *origin, *normal, *x_dir)
                        .map_err(|m| {
                            MeshError::Triangulation(format!("face '{}': {m}", face.name))
                        })?;
                }
                Surface::Cylinder { .. } => {
                    // A face bounded by two arcs and their sides is a strip; any other trimmed
                    // face (holes, ellipse caps with several loops, partial patches) is
                    // triangulated in the surface's own (angle, height) plane.
                    let strip = if force_general || face.loops.len() != 1 {
                        Err("not a strip".to_owned())
                    } else {
                        triangulate_cylinder(&mut mesh, self, face, &edge_chain)
                    };
                    if strip.is_err() {
                        triangulate_cylinder_general(&mut mesh, face, &chains, params).map_err(
                            |m| MeshError::Triangulation(format!("face '{}': {m}", face.name)),
                        )?;
                    }
                }
                Surface::Cone { .. } => triangulate_cylinder(&mut mesh, self, face, &edge_chain)
                    .map_err(|m| MeshError::Triangulation(format!("face '{}': {m}", face.name)))?,
                Surface::Torus { .. } => triangulate_torus(&mut mesh, self, face, &edge_chain)
                    .map_err(|m| MeshError::Triangulation(format!("face '{}': {m}", face.name)))?,
            }
            mesh.face_of.extend(std::iter::repeat_n(
                fi as u32,
                mesh.triangles.len() - before,
            ));
        }
        Ok(mesh)
    }
}

// ---------------------------------------------------------------------------------------------
// Planar faces: ear clipping with hole bridging
// ---------------------------------------------------------------------------------------------

fn cross2(a: Vec2, b: Vec2) -> f64 {
    a.x * b.y - a.y * b.x
}

fn signed_area(poly: &[Vec2]) -> f64 {
    let mut a = 0.0;
    for i in 0..poly.len() {
        a += cross2(poly[i], poly[(i + 1) % poly.len()]);
    }
    0.5 * a
}

/// On which side of the line `a -> b` is `c`: 1 left, -1 right, 0 on it. Exact.
fn side(a: Vec2, b: Vec2, c: Vec2) -> i8 {
    orient2d_sign(a, b, c)
}

/// Is `p` inside triangle `abc` or on its boundary? (Whatever the triangle's orientation.)
fn point_in_triangle(p: Vec2, a: Vec2, b: Vec2, c: Vec2) -> bool {
    let (d1, d2, d3) = (side(a, b, p), side(b, c, p), side(c, a, p));
    let neg = d1 < 0 || d2 < 0 || d3 < 0;
    let pos = d1 > 0 || d2 > 0 || d3 > 0;
    !(neg && pos)
}

/// Does segment p-q properly cross segment a-b (excluding shared endpoints)?
fn segments_cross(p: Vec2, q: Vec2, a: Vec2, b: Vec2) -> bool {
    let (d1, d2) = (side(p, q, a), side(p, q, b));
    let (d3, d4) = (side(a, b, p), side(a, b, q));
    d1 != 0 && d2 != 0 && d3 != 0 && d4 != 0 && (d1 != d2) && (d3 != d4)
}

/// Connects each hole to the outer boundary with a bridge, producing one simple (weakly) polygon
/// as a list of indices into `pts`. Holes are processed right to left by their rightmost vertex.
fn bridge_holes(pts: &[Vec2], outer: &[usize], holes: &[Vec<usize>]) -> Result<Vec<usize>, String> {
    let mut poly: Vec<usize> = outer.to_vec();
    let mut order: Vec<usize> = (0..holes.len()).collect();
    let rightmost = |h: &Vec<usize>| {
        let mut best = 0;
        for k in 1..h.len() {
            let (a, b) = (pts[h[k]], pts[h[best]]);
            if a.x > b.x || (a.x == b.x && a.y < b.y) {
                best = k;
            }
        }
        best
    };
    order.sort_by(|&a, &b| {
        let (pa, pb) = (
            pts[holes[a][rightmost(&holes[a])]],
            pts[holes[b][rightmost(&holes[b])]],
        );
        pb.x.total_cmp(&pa.x)
            .then(pa.y.total_cmp(&pb.y))
            .then(a.cmp(&b))
    });
    // All hole edges, to reject bridges that would cross a hole not yet merged.
    let all_hole_edges: Vec<(Vec2, Vec2)> = holes
        .iter()
        .flat_map(|h| (0..h.len()).map(move |i| (pts[h[i]], pts[h[(i + 1) % h.len()]])))
        .collect();
    for &hi in &order {
        let h = &holes[hi];
        let hk = rightmost(h);
        let hp = pts[h[hk]];
        // Candidate polygon vertices, nearest first; take the first with a clear line of sight.
        let mut cands: Vec<usize> = (0..poly.len()).collect();
        cands.sort_by(|&a, &b| {
            let (da, db) = (
                (pts[poly[a]] - hp).length_squared(),
                (pts[poly[b]] - hp).length_squared(),
            );
            da.total_cmp(&db).then(a.cmp(&b))
        });
        let mut chosen = None;
        'cand: for &ci in &cands {
            let q = pts[poly[ci]];
            if q == hp {
                continue;
            }
            for i in 0..poly.len() {
                let (a, b) = (pts[poly[i]], pts[poly[(i + 1) % poly.len()]]);
                if segments_cross(hp, q, a, b) {
                    continue 'cand;
                }
            }
            for &(a, b) in &all_hole_edges {
                if segments_cross(hp, q, a, b) {
                    continue 'cand;
                }
            }
            chosen = Some(ci);
            break;
        }
        let ci = chosen.ok_or("no visible bridge from a hole to the outer boundary")?;
        // Splice: ... poly[ci], hole from hk all the way round, hole[hk] again, poly[ci], ...
        let mut spliced: Vec<usize> = poly[..=ci].to_vec();
        for k in 0..=h.len() {
            spliced.push(h[(hk + k) % h.len()]);
        }
        spliced.push(poly[ci]);
        spliced.extend_from_slice(&poly[ci + 1..]);
        poly = spliced;
    }
    Ok(poly)
}

/// Ear clipping over an index polygon (counter-clockwise, may repeat bridge vertices).
///
/// The polygon is a doubly linked ring, so clipping an ear costs O(1), and a vertex is tested
/// against the ear only if it is reflex (or flat): only those can lie inside a triangle cut off
/// a simple polygon. The scan resumes where the last ear was found instead of starting over.
/// That makes typical faces close to linear; a polygon with very many reflex vertices (a
/// star) is still quadratic. All orientation tests are exact, so a sliver or a nearly flat
/// vertex is classified the same way every time and on every platform.
fn ear_clip(pts: &[Vec2], poly: Vec<usize>) -> Result<Vec<[usize; 3]>, String> {
    ear_clip_by(pts, poly, None)
}

/// Ear clipping that, given a `weight` of a diagonal, always clips the valid ear whose diagonal
/// is lightest, which keeps triangles short in the direction the weight measures.
fn ear_clip_by(
    pts: &[Vec2],
    poly: Vec<usize>,
    weight: Option<&dyn Fn(Vec2, Vec2) -> f64>,
) -> Result<Vec<[usize; 3]>, String> {
    let m = poly.len();
    let mut tris = Vec::with_capacity(m.saturating_sub(2));
    if m < 3 {
        return Ok(tris);
    }
    let mut prev: Vec<usize> = (0..m).map(|i| (i + m - 1) % m).collect();
    let mut next: Vec<usize> = (0..m).map(|i| (i + 1) % m).collect();
    let mut alive = vec![true; m];
    let mut count = m;
    let at = |pos: usize| pts[poly[pos]];
    // > 0 convex, < 0 reflex, 0 flat; recomputed from the current neighbours.
    let turn = |pos: usize, prev: &[usize], next: &[usize]| {
        orient2d(at(prev[pos]), at(pos), at(next[pos]))
    };
    // Vertices that are not strictly convex. A vertex only ever leaves this list.
    let mut reflex: Vec<usize> = (0..m).filter(|&i| turn(i, &prev, &next) <= 0.0).collect();
    let mut cur = 0;
    // A budget that a correct run never reaches: each ear or removal uses one, and a pass that
    // finds none ends the loop.
    let mut budget = m * m + 16;
    while count > 3 {
        if budget == 0 {
            return Err("ear clipping did not terminate".into());
        }
        budget -= 1;
        reflex.retain(|&r| alive[r] && turn(r, &prev, &next) <= 0.0);
        let mut found: Option<(usize, bool)> = None; // (vertex, emit a triangle?)
        let mut best_w = f64::MAX;
        let mut v = cur;
        for _ in 0..count {
            let (pa, pc) = (prev[v], next[v]);
            let (ia, ib, ic) = (poly[pa], poly[v], poly[pc]);
            let (a, b, c) = (pts[ia], pts[ib], pts[ic]);
            let t = orient2d(a, b, c);
            if t <= 0.0 {
                // Reflex or flat: a bridge spike (the ring goes out and back along one segment)
                // is removed without a triangle; anything else is skipped for now.
                if t == 0.0 && (ia == ic || a == c) {
                    found = Some((v, false));
                    break;
                }
                v = next[v];
                continue;
            }
            let blocked = reflex.iter().any(|&r| {
                let k = poly[r];
                alive[r]
                    && r != pa
                    && r != v
                    && r != pc
                    && k != ia
                    && k != ib
                    && k != ic
                    && pts[k] != a
                    && pts[k] != b
                    && pts[k] != c
                    && point_in_triangle(pts[k], a, b, c)
            });
            if !blocked {
                match weight {
                    None => {
                        found = Some((v, true));
                        break;
                    }
                    Some(w) => {
                        let cost = w(a, c);
                        if cost < best_w {
                            best_w = cost;
                            found = Some((v, true));
                        }
                    }
                }
            }
            v = next[v];
        }
        let (v, emit) = match found {
            Some(f) => f,
            None => {
                // No ear: drop a flat vertex to make progress, else the boundary is not simple.
                match (0..m).find(|&i| alive[i] && turn(i, &prev, &next) == 0.0) {
                    Some(i) => (i, false),
                    None => return Err("no ear found (boundary is self-intersecting)".into()),
                }
            }
        };
        let (pa, pc) = (prev[v], next[v]);
        if emit {
            tris.push([poly[pa], poly[v], poly[pc]]);
        }
        alive[v] = false;
        next[pa] = pc;
        prev[pc] = pa;
        count -= 1;
        cur = pc;
    }
    // What is left is one triangle (or a flat sliver, which is dropped).
    let v = (0..m).find(|&i| alive[i]).expect("three vertices remain");
    let (b, c) = (next[v], next[next[v]]);
    if orient2d(at(v), at(b), at(c)) > 0.0 {
        tris.push([poly[v], poly[b], poly[c]]);
    }
    Ok(tris)
}

fn triangulate_planar(
    mesh: &mut Mesh,
    face: &Face,
    chains: &[Vec<u32>],
    origin: Vec3,
    normal: Vec3,
    x_dir: Vec3,
) -> Result<(), String> {
    // Work in the plane's 2D frame, oriented so the face's outward normal is +z: loops are then
    // counter-clockwise (outer) / clockwise (holes) and the triangles come out outward-facing.
    let flip = !face.same_sense;
    let n_out = if flip { -normal } else { normal };
    let y_dir = n_out.cross(x_dir);
    let mut idx: Vec<u32> = Vec::new(); // local index -> mesh vertex
    let mut pts: Vec<Vec2> = Vec::new();
    let mut loops_local: Vec<Vec<usize>> = Vec::new();
    for ch in chains {
        let mut l = Vec::new();
        for &mi in ch {
            let p = mesh.positions[mi as usize] - origin;
            l.push(pts.len());
            idx.push(mi);
            pts.push(Vec2::new(p.dot(x_dir), p.dot(y_dir)));
        }
        loops_local.push(l);
    }
    let outer = loops_local[0].clone();
    let outer_poly: Vec<Vec2> = outer.iter().map(|&i| pts[i]).collect();
    if signed_area(&outer_poly) <= 0.0 {
        return Err("outer loop is not counter-clockwise seen from outside".into());
    }
    let mut holes = Vec::new();
    for h in &loops_local[1..] {
        let hp: Vec<Vec2> = h.iter().map(|&i| pts[i]).collect();
        if signed_area(&hp) >= 0.0 {
            return Err("a hole is not clockwise seen from outside".into());
        }
        holes.push(h.clone());
    }
    let poly = bridge_holes(&pts, &outer, &holes)?;
    for t in ear_clip(&pts, poly)? {
        mesh.triangles.push([idx[t[0]], idx[t[1]], idx[t[2]]]);
    }
    Ok(())
}

// ---------------------------------------------------------------------------------------------
// Cylinder patches: a strip between the two boundary arcs
// ---------------------------------------------------------------------------------------------

/// The two boundary arcs of a strip patch and the runs of vertices up its two sides: `a` and
/// `b` correspond point for point (`b[k]` is across from `a[k]`), `right` runs from `a[n]` to
/// `b[n]` and `left` from `a[0]` to `b[0]`, each with its intermediate vertices.
struct Strip {
    a: Vec<u32>,
    b: Vec<u32>,
    right: Vec<u32>,
    left: Vec<u32>,
}

fn strip_of(solid: &Solid, face: &Face, edge_chain: &[Vec<u32>]) -> Result<Strip, String> {
    let lp = &face.loops[0];
    // A boundary arc is a circle used once by the loop; an edge used twice (back and forth) is
    // the seam, straight or curved.
    let is_arc = |c: &crate::solid::CoEdge| {
        solid.edges[c.edge as usize].curve.is_curved()
            && lp.iter().filter(|d| d.edge == c.edge).count() == 1
    };
    let first = lp.iter().position(is_arc).ok_or("no boundary arc")?;
    let rotated: Vec<&crate::solid::CoEdge> =
        lp[first..].iter().chain(lp[..first].iter()).collect();
    let chain = |c: &crate::solid::CoEdge| -> Vec<u32> {
        let mut v = edge_chain[c.edge as usize].clone();
        if c.reversed {
            v.reverse();
        }
        v
    };
    // arc1, vertical edges, arc2, vertical edges.
    let mut arcs: Vec<Vec<u32>> = Vec::new();
    let mut sides: Vec<Vec<u32>> = Vec::new();
    for c in rotated {
        if is_arc(c) {
            arcs.push(chain(c));
            sides.push(Vec::new());
        } else {
            let side = sides.last_mut().ok_or("a vertical edge before any arc")?;
            for v in chain(c) {
                if side.last() != Some(&v) {
                    side.push(v);
                }
            }
        }
    }
    let [a, b_loop] = &arcs[..] else {
        return Err(format!("expected two boundary arcs, found {}", arcs.len()));
    };
    if a.len() != b_loop.len() {
        return Err("boundary arcs are sampled differently".into());
    }
    // `b` runs the other way round the face; flip it so b[k] sits above a[k].
    let b: Vec<u32> = b_loop.iter().rev().copied().collect();
    let mut left = sides.get(1).cloned().unwrap_or_default();
    left.reverse();
    Ok(Strip {
        a: a.clone(),
        b,
        right: sides.first().cloned().unwrap_or_default(),
        left,
    })
}

fn triangulate_cylinder(
    mesh: &mut Mesh,
    solid: &Solid,
    face: &Face,
    edge_chain: &[Vec<u32>],
) -> Result<(), String> {
    let (origin, axis) = match &face.surface {
        Surface::Cylinder { origin, axis, .. } | Surface::Cone { origin, axis, .. } => {
            (origin, axis)
        }
        _ => unreachable!("called for cylinders and cones only"),
    };
    // The patch is bounded by two arcs of the same circle sampled at the same parameters, so
    // their polylines correspond point for point (a strip of quads between them), plus a vertical
    // side at each end. A wall that spans several stacked slabs has *intermediate vertices* on
    // those sides (shared with neighbouring faces), and the strip must use them or the mesh gets
    // T-junctions: the first and last columns become fans over the side vertices.
    let Strip { a, b, right, left } = strip_of(solid, face, edge_chain)?;
    let n = a.len() - 1;
    // Keep only the intermediate vertices of each side.
    let mids = |v: &[u32]| -> Vec<u32> {
        if v.len() > 2 {
            v[1..v.len() - 1].to_vec()
        } else {
            Vec::new()
        }
    };
    let right_mids = mids(&right);
    let left_mids = mids(&left);

    let mut tris: Vec<[u32; 3]> = Vec::new();
    for k in 0..n {
        // Lower triangle of the quad: (a_k, a_{k+1}, b_k), with the left edge if k == 0.
        if k == 0 {
            let mut p = vec![a[0]];
            p.extend(&left_mids);
            p.push(b[0]);
            for w in p.windows(2) {
                tris.push([a[1], w[1], w[0]]);
            }
        } else {
            tris.push([a[k], a[k + 1], b[k]]);
        }
        // Upper triangle: (a_{k+1}, b_{k+1}, b_k), with the right edge if k == n - 1.
        if k == n - 1 {
            let mut q = vec![a[n]];
            q.extend(&right_mids);
            q.push(b[n]);
            for w in q.windows(2) {
                tris.push([b[n - 1], w[0], w[1]]);
            }
        } else {
            tris.push([a[k + 1], b[k + 1], b[k]]);
        }
    }

    // Choose the winding once so every triangle faces outward.
    let pos = |i: u32| mesh.positions[i as usize];
    let sense = if face.same_sense { 1.0 } else { -1.0 };
    let t0 = tris[0];
    let centroid = (pos(t0[0]) + pos(t0[1]) + pos(t0[2])) / 3.0;
    let d = centroid - *origin;
    let outward = (d - *axis * d.dot(*axis)) * sense;
    let flip = (pos(t0[1]) - pos(t0[0]))
        .cross(pos(t0[2]) - pos(t0[0]))
        .dot(outward)
        < 0.0;
    for t in tris {
        mesh.triangles
            .push(if flip { [t[0], t[2], t[1]] } else { t });
    }
    Ok(())
}

/// A cylinder face trimmed by arbitrary loops, triangulated in the (angle, height) plane of the
/// surface. Loops are unwrapped about the axis (a loop that winds around it needs a seam edge,
/// as strips have, and is refused here). A face between a lower and an upper chain that both run
/// the same way in angle (the usual wall between two cuts) is zipped, which keeps triangles thin
/// along the axis and so close to the surface; any other face is ear-clipped, which is valid but
/// can leave long triangles across a wide patch.
fn triangulate_cylinder_general(
    mesh: &mut Mesh,
    face: &Face,
    chains: &[Vec<u32>],
    params: TessParams,
) -> Result<(), String> {
    let Surface::Cylinder {
        origin,
        axis,
        ref_dir,
        radius,
        ..
    } = &face.surface
    else {
        return Err("not a cylinder".into());
    };
    let e2 = axis.cross(*ref_dir);
    let sigma = if face.same_sense { 1.0 } else { -1.0 };
    let tau = 2.0 * core::f64::consts::PI;
    let mut pts: Vec<Vec2> = Vec::new();
    let mut idx: Vec<u32> = Vec::new();
    let mut loops: Vec<Vec<usize>> = Vec::new();
    for ch in chains {
        let mut lp = Vec::new();
        let mut prev: Option<f64> = None;
        let mut first = 0.0;
        for &mi in ch {
            let d = mesh.positions[mi as usize] - *origin;
            let mut phi = det::atan2(d.dot(e2), d.dot(*ref_dir));
            if let Some(p) = prev {
                while phi - p > core::f64::consts::PI {
                    phi -= tau;
                }
                while phi - p < -core::f64::consts::PI {
                    phi += tau;
                }
            } else {
                first = phi;
            }
            prev = Some(phi);
            lp.push(pts.len());
            idx.push(mi);
            pts.push(Vec2::new(sigma * radius * phi, d.dot(*axis)));
        }
        // Does the loop close in the unwrapped plane?
        if let (Some(p), Some(&last)) = (prev, ch.last()) {
            let d = mesh.positions[ch[0] as usize] - *origin;
            let mut back = det::atan2(d.dot(e2), d.dot(*ref_dir));
            while back - p > core::f64::consts::PI {
                back -= tau;
            }
            while back - p < -core::f64::consts::PI {
                back += tau;
            }
            let _ = last;
            if (back - first).abs() > 1e-6 {
                return Err("a loop winds around the axis without a seam".into());
            }
        }
        loops.push(lp);
    }
    let outer_poly: Vec<Vec2> = loops[0].iter().map(|&i| pts[i]).collect();
    if signed_area(&outer_poly) <= 0.0 {
        return Err("outer loop is not counter-clockwise seen from outside".into());
    }
    // Holes: shift by whole turns until the hole lies inside the outer loop.
    let mut holes: Vec<Vec<usize>> = Vec::new();
    for h in &loops[1..] {
        let probe = pts[h[0]];
        let mut placed = false;
        for k in [0.0, 1.0, -1.0] {
            let shift = sigma * radius * tau * k;
            let moved = Vec2::new(probe.x + shift, probe.y);
            if point_in_polygon(&outer_poly, moved) {
                for &i in h {
                    pts[i].x += shift;
                }
                placed = true;
                break;
            }
        }
        if !placed {
            return Err("a hole lies outside the outer loop".into());
        }
        let hp: Vec<Vec2> = h.iter().map(|&i| pts[i]).collect();
        if signed_area(&hp) >= 0.0 {
            return Err("a hole is not clockwise seen from outside".into());
        }
        holes.push(h.clone());
    }
    // Diagonals are weighed by how far they reach round the axis: a long one cuts a chord
    // through the surface. Large faces use the plain (first ear) clip, which is much cheaper.
    let across = |a: Vec2, c: Vec2| (a.x - c.x).abs() + 1e-3 * (a.y - c.y).abs();
    let tris: Vec<[usize; 3]> = if holes.is_empty() {
        match zip_strip(&pts, &loops[0]) {
            Some(t) => t,
            None => ear_clip_by(
                &pts,
                loops[0].clone(),
                (pts.len() < 2500).then_some(&across),
            )?,
        }
    } else {
        let poly = bridge_holes(&pts, &loops[0], &holes)?;
        let small = poly.len() < 2500;
        ear_clip_by(&pts, poly, small.then_some(&across))?
    };
    let mut tris = tris;
    // Interior edges that span a wide angle cut through the surface: split them (in both the
    // triangles they belong to) until each is about as short as a boundary segment. Boundary
    // edges are never split, so neighbouring faces still meet this one exactly.
    let n_seg = segments(tau, *radius, params) as f64;
    let max_dx = radius * tau / n_seg * 1.5;
    refine_wide_edges(mesh, face, &mut pts, &mut idx, &mut tris, max_dx);
    for t in tris {
        // Counter-clockwise in the plane means outward-facing on the surface (see `sigma`).
        let (a, b, c) = (pts[t[0]], pts[t[1]], pts[t[2]]);
        let tri = if orient2d(a, b, c) >= 0.0 {
            [idx[t[0]], idx[t[1]], idx[t[2]]]
        } else {
            [idx[t[0]], idx[t[2]], idx[t[1]]]
        };
        if tri[0] != tri[1] && tri[1] != tri[2] && tri[0] != tri[2] {
            mesh.triangles.push(tri);
        }
    }
    Ok(())
}

/// Splits interior edges longer than `max_dx` in the angle direction by inserting a vertex on
/// the surface at the middle of the edge and joining it to the opposite vertex of both
/// neighbouring triangles. Boundary edges (used by one triangle) are left alone.
fn refine_wide_edges(
    mesh: &mut Mesh,
    face: &Face,
    pts: &mut Vec<Vec2>,
    idx: &mut Vec<u32>,
    tris: &mut Vec<[usize; 3]>,
    max_dx: f64,
) {
    let Surface::Cylinder {
        origin,
        axis,
        ref_dir,
        radius,
        ..
    } = &face.surface
    else {
        return;
    };
    let e2 = axis.cross(*ref_dir);
    let sigma = if face.same_sense { 1.0 } else { -1.0 };
    for _ in 0..100_000 {
        // Interior edges and the triangles on each side.
        let mut edges: BTreeMap<(usize, usize), Vec<(usize, usize)>> = BTreeMap::new();
        for (ti, t) in tris.iter().enumerate() {
            for k in 0..3 {
                let (a, b) = (t[k], t[(k + 1) % 3]);
                edges
                    .entry((a.min(b), a.max(b)))
                    .or_default()
                    .push((ti, (k + 2) % 3));
            }
        }
        let mut best: Option<(f64, (usize, usize))> = None;
        for (&(a, b), users) in &edges {
            if users.len() == 2 {
                let dx = (pts[a].x - pts[b].x).abs();
                if dx > max_dx && best.is_none_or(|(w, _)| dx > w) {
                    best = Some((dx, (a, b)));
                }
            }
        }
        let Some((_, (a, b))) = best else {
            return;
        };
        let users = edges[&(a, b)].clone();
        let m = Vec2::new(0.5 * (pts[a].x + pts[b].x), 0.5 * (pts[a].y + pts[b].y));
        let phi = m.x / (sigma * radius);
        let pos = *origin + *axis * m.y + (*ref_dir * det::cos(phi) + e2 * det::sin(phi)) * *radius;
        let new_pt = pts.len();
        pts.push(m);
        mesh.positions.push(pos);
        idx.push((mesh.positions.len() - 1) as u32);
        let mut remove: Vec<usize> = Vec::new();
        for (ti, opp) in users {
            let t = tris[ti];
            let c = t[opp];
            // The triangle keeps its winding: replace the edge a-b by a-m-b.
            let (p, q) = (t[(opp + 1) % 3], t[(opp + 2) % 3]); // p -> q is the edge in winding order
            tris.push([p, new_pt, c]);
            tris.push([new_pt, q, c]);
            remove.push(ti);
        }
        remove.sort_unstable_by(|x, y| y.cmp(x));
        for ti in remove {
            tris.swap_remove(ti);
        }
    }
}

fn point_in_polygon(poly: &[Vec2], p: Vec2) -> bool {
    let mut inside = false;
    for k in 0..poly.len() {
        let (a, b) = (poly[k], poly[(k + 1) % poly.len()]);
        if (a.y > p.y) != (b.y > p.y) {
            let s = orient2d_sign(a, b, p);
            if (b.y > a.y && s > 0) || (b.y < a.y && s < 0) {
                inside = !inside;
            }
        }
    }
    inside
}

/// Triangulates a counter-clockwise polygon whose boundary is a lower chain and an upper chain
/// that both advance left to right (with vertical sides, which may carry extra vertices that
/// neighbouring faces share). `None` if the polygon is not of that shape.
fn zip_strip(pts: &[Vec2], poly: &[usize]) -> Option<Vec<[usize; 3]>> {
    let n = poly.len();
    if n < 4 {
        return None;
    }
    let x = |k: usize| pts[poly[k % n]].x;
    let (xmin, xmax) = poly.iter().fold((f64::MAX, f64::MIN), |(lo, hi), &i| {
        (lo.min(pts[i].x), hi.max(pts[i].x))
    });
    let eps = 1e-9 * (xmax - xmin).abs().max(1.0);
    let on_left = |k: usize| x(k) <= xmin + eps;
    let on_right = |k: usize| x(k) >= xmax - eps;
    // A contiguous run of loop positions satisfying `pred`: (first, length).
    let run = |pred: &dyn Fn(usize) -> bool| -> Option<(usize, usize)> {
        let starts: Vec<usize> = (0..n).filter(|&k| pred(k) && !pred(k + n - 1)).collect();
        if starts.len() != 1 {
            return None;
        }
        let s = starts[0];
        let len = (0..n).take_while(|&d| pred(s + d)).count();
        let total = (0..n).filter(|&k| pred(k)).count();
        (len == total).then_some((s, len))
    };
    let (ls, ll) = run(&on_left)?;
    let (rs, rl) = run(&on_right)?;
    // Counter-clockwise: the left side runs downward, the right side upward.
    let (tl, bl) = (ls, ls + ll - 1);
    let (br, tr) = (rs, rs + rl - 1);
    let walk = |from: usize, to: usize| -> Vec<usize> {
        let mut v = vec![poly[from % n]];
        let mut k = from;
        while k % n != to % n {
            k += 1;
            v.push(poly[k % n]);
        }
        v
    };
    let lower = walk(bl, br);
    let mut upper = walk(tr, tl);
    upper.reverse();
    let left: Vec<usize> = {
        let mut v = walk(tl, bl);
        v.reverse();
        v
    }; // bottom to top
    let right = walk(br, tr); // bottom to top
    let monotone = |c: &[usize]| c.windows(2).all(|w| pts[w[1]].x >= pts[w[0]].x - eps);
    if !monotone(&lower) || !monotone(&upper) {
        return None;
    }
    let mut tris: Vec<[usize; 3]> = Vec::new();
    let (mut i, mut j) = (0usize, 0usize);
    // Extra vertices on a side are fanned from the neighbouring chain vertex so they are used.
    if left.len() > 2 && lower.len() > 1 {
        for w in left.windows(2) {
            tris.push([lower[1], w[1], w[0]]);
        }
        i = 1;
    }
    let mut j_end = upper.len() - 1;
    if right.len() > 2 && upper.len() > 1 {
        j_end -= 1;
    }
    let i_end = lower.len() - 1;
    while i < i_end || j < j_end {
        let adv_a = if i == i_end {
            false
        } else if j == j_end {
            true
        } else {
            pts[lower[i + 1]].x <= pts[upper[j + 1]].x
        };
        if adv_a {
            tris.push([lower[i], lower[i + 1], upper[j]]);
            i += 1;
        } else {
            tris.push([lower[i], upper[j + 1], upper[j]]);
            j += 1;
        }
    }
    if right.len() > 2 && upper.len() > 1 {
        let apex = upper[upper.len() - 2];
        for w in right.windows(2) {
            tris.push([apex, w[0], w[1]]);
        }
    }
    Some(tris)
}

/// A torus patch (a round edge): rows of vertices between its two boundary circles, following the
/// samples of the tube's arc along the seam; the first and last columns are the seam itself, so
/// they are shared with the neighbouring ends of the strip.
fn triangulate_torus(
    mesh: &mut Mesh,
    solid: &Solid,
    face: &Face,
    edge_chain: &[Vec<u32>],
) -> Result<(), String> {
    let Surface::Torus {
        origin,
        axis,
        major,
        ..
    } = &face.surface
    else {
        unreachable!("called for tori only")
    };
    let Strip { a, b, right, left } = strip_of(solid, face, edge_chain)?;
    let n = a.len() - 1;
    if right.len() < 2 || left.len() != right.len() {
        return Err("the seam of a round edge is sampled differently on its two sides".into());
    }
    let m = right.len() - 1;
    let pos = |mesh: &Mesh, i: u32| mesh.positions[i as usize];
    // Each row's circle: its radius and height, read off the seam vertex of that row.
    let rows: Vec<(f64, f64)> = right
        .iter()
        .map(|&v| {
            let d = pos(mesh, v) - *origin;
            let h = d.dot(*axis);
            ((d - *axis * h).length(), h)
        })
        .collect();
    let mut ring: Vec<Vec<u32>> = Vec::with_capacity(m + 1);
    for i in 0..=m {
        let mut row = Vec::with_capacity(n + 1);
        for k in 0..=n {
            let v = if i == 0 {
                a[k]
            } else if i == m {
                b[k]
            } else if k == 0 {
                left[i]
            } else if k == n {
                right[i]
            } else {
                let d = pos(mesh, a[k]) - *origin;
                let rv = d - *axis * d.dot(*axis);
                let u = rv / rv.length();
                let p = *origin + *axis * rows[i].1 + u * rows[i].0;
                mesh.positions.push(p);
                (mesh.positions.len() - 1) as u32
            };
            row.push(v);
        }
        ring.push(row);
    }
    let mut tris: Vec<[u32; 3]> = Vec::new();
    for i in 0..m {
        for k in 0..n {
            let (v00, v01, v10, v11) = (
                ring[i][k],
                ring[i][k + 1],
                ring[i + 1][k],
                ring[i + 1][k + 1],
            );
            tris.push([v00, v01, v10]);
            tris.push([v01, v11, v10]);
        }
    }
    // One winding for all: outward is away from the tube's centre circle.
    let sense = if face.same_sense { 1.0 } else { -1.0 };
    let t0 = tris[0];
    let c = (pos(mesh, t0[0]) + pos(mesh, t0[1]) + pos(mesh, t0[2])) / 3.0;
    let d = c - *origin;
    let h = d.dot(*axis);
    let radial = d - *axis * h;
    let tube = (radial - radial / radial.length() * *major) + *axis * h;
    let flip = (pos(mesh, t0[1]) - pos(mesh, t0[0]))
        .cross(pos(mesh, t0[2]) - pos(mesh, t0[0]))
        .dot(tube * sense)
        < 0.0;
    for t in tris {
        mesh.triangles
            .push(if flip { [t[0], t[2], t[1]] } else { t });
    }
    Ok(())
}

// ---------------------------------------------------------------------------------------------
// Mesh checks and measures
// ---------------------------------------------------------------------------------------------

impl Mesh {
    /// Verifies the mesh is a closed, consistently oriented surface: every directed edge appears
    /// once and its reverse appears once. Returns the problems found (empty means watertight).
    pub fn check(&self) -> Vec<String> {
        let mut out = Vec::new();
        let mut directed: BTreeMap<(u32, u32), u32> = BTreeMap::new();
        for (t, tri) in self.triangles.iter().enumerate() {
            if tri[0] == tri[1] || tri[1] == tri[2] || tri[0] == tri[2] {
                out.push(format!("triangle {t} is degenerate"));
            }
            for k in 0..3 {
                *directed.entry((tri[k], tri[(k + 1) % 3])).or_insert(0) += 1;
            }
        }
        for (&(a, b), &n) in &directed {
            let back = directed.get(&(b, a)).copied().unwrap_or(0);
            if n != 1 || back != 1 {
                out.push(format!("edge {a}->{b} used {n}x, reverse {back}x"));
                if out.len() > 20 {
                    break;
                }
            }
        }
        out
    }

    pub fn volume(&self) -> f64 {
        self.triangles
            .iter()
            .map(|t| {
                let (a, b, c) = (
                    self.positions[t[0] as usize],
                    self.positions[t[1] as usize],
                    self.positions[t[2] as usize],
                );
                a.dot(b.cross(c)) / 6.0
            })
            .sum()
    }

    pub fn area(&self) -> f64 {
        self.triangles
            .iter()
            .map(|t| {
                let (a, b, c) = (
                    self.positions[t[0] as usize],
                    self.positions[t[1] as usize],
                    self.positions[t[2] as usize],
                );
                0.5 * (b - a).cross(c - a).length()
            })
            .sum()
    }

    /// Binary STL. Byte-for-byte deterministic: fixed header, no timestamps.
    pub fn to_binary_stl(&self) -> Vec<u8> {
        let mut out = Vec::with_capacity(84 + self.triangles.len() * 50);
        let mut header = [0u8; 80];
        let tag = b"Design Computer STL";
        header[..tag.len()].copy_from_slice(tag);
        out.extend_from_slice(&header);
        out.extend_from_slice(&(self.triangles.len() as u32).to_le_bytes());
        for t in &self.triangles {
            let (a, b, c) = (
                self.positions[t[0] as usize],
                self.positions[t[1] as usize],
                self.positions[t[2] as usize],
            );
            let n = (b - a).cross(c - a);
            let l = n.length();
            let n = if l > 0.0 { n / l } else { Vec3::ZERO };
            for v in [n, a, b, c] {
                for x in [v.x, v.y, v.z] {
                    out.extend_from_slice(&(x as f32).to_le_bytes());
                }
            }
            out.extend_from_slice(&0u16.to_le_bytes());
        }
        out
    }

    pub fn to_ascii_stl(&self, name: &str) -> String {
        let mut s = format!("solid {name}\n");
        for t in &self.triangles {
            let (a, b, c) = (
                self.positions[t[0] as usize],
                self.positions[t[1] as usize],
                self.positions[t[2] as usize],
            );
            let n = (b - a).cross(c - a);
            let l = n.length();
            let n = if l > 0.0 { n / l } else { Vec3::ZERO };
            s.push_str(&format!(
                "  facet normal {:e} {:e} {:e}\n    outer loop\n",
                n.x as f32, n.y as f32, n.z as f32
            ));
            for v in [a, b, c] {
                s.push_str(&format!(
                    "      vertex {:e} {:e} {:e}\n",
                    v.x as f32, v.y as f32, v.z as f32
                ));
            }
            s.push_str("    endloop\n  endfacet\n");
        }
        s.push_str(&format!("endsolid {name}\n"));
        s
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::f64::consts::PI;

    fn poly_area(pts: &[Vec2], poly: &[usize]) -> f64 {
        signed_area(&poly.iter().map(|&i| pts[i]).collect::<Vec<_>>())
    }

    /// Checks a triangulation of `poly`: every triangle counter-clockwise (exactly), the areas
    /// add up to the polygon's, and (for small inputs) no vertex lies strictly inside a triangle.
    fn check(pts: &[Vec2], poly: &[usize], tris: &[[usize; 3]], inside_test: bool) {
        let area = poly_area(pts, poly);
        let mut sum = 0.0;
        for t in tris {
            let (a, b, c) = (pts[t[0]], pts[t[1]], pts[t[2]]);
            assert!(
                orient2d(a, b, c) > 0.0,
                "a clockwise or flat triangle {t:?}"
            );
            sum += 0.5 * orient2d(a, b, c);
        }
        assert!(
            (sum - area).abs() <= 1e-9 * area.abs().max(1.0),
            "areas {sum} against {area}"
        );
        if inside_test {
            for t in tris {
                let (a, b, c) = (pts[t[0]], pts[t[1]], pts[t[2]]);
                for &k in poly {
                    let p = pts[k];
                    if p == a || p == b || p == c {
                        continue;
                    }
                    let strictly = side(a, b, p) > 0 && side(b, c, p) > 0 && side(c, a, p) > 0;
                    assert!(!strictly, "vertex {k} is inside triangle {t:?}");
                }
            }
        }
    }

    fn circle(n: usize, r: f64) -> Vec<Vec2> {
        (0..n)
            .map(|i| {
                let a = 2.0 * PI * i as f64 / n as f64;
                Vec2::new(r * det::cos(a), r * det::sin(a))
            })
            .collect()
    }

    #[test]
    fn convex_concave_and_star_polygons_triangulate_completely() {
        // Convex.
        let pts = circle(40, 10.0);
        let poly: Vec<usize> = (0..40).collect();
        let t = ear_clip(&pts, poly.clone()).unwrap();
        assert_eq!(t.len(), 38);
        check(&pts, &poly, &t, true);
        // A star: 12 points alternately far and near, many reflex vertices.
        let star: Vec<Vec2> = (0..24)
            .map(|i| {
                let a = 2.0 * PI * i as f64 / 24.0;
                let r = if i % 2 == 0 { 10.0 } else { 4.0 };
                Vec2::new(r * det::cos(a), r * det::sin(a))
            })
            .collect();
        let poly: Vec<usize> = (0..24).collect();
        let t = ear_clip(&star, poly.clone()).unwrap();
        assert_eq!(t.len(), 22);
        check(&star, &poly, &t, true);
        // An L shape.
        let l = [
            (0.0, 0.0),
            (4.0, 0.0),
            (4.0, 2.0),
            (2.0, 2.0),
            (2.0, 4.0),
            (0.0, 4.0),
        ]
        .map(|(x, y)| Vec2::new(x, y));
        let poly: Vec<usize> = (0..6).collect();
        let t = ear_clip(&l, poly.clone()).unwrap();
        check(&l, &poly, &t, true);
        assert_eq!(t.len(), 4);
    }

    #[test]
    fn a_hole_is_bridged_and_the_area_is_the_difference() {
        let mut pts = vec![
            Vec2::new(0.0, 0.0),
            Vec2::new(20.0, 0.0),
            Vec2::new(20.0, 20.0),
            Vec2::new(0.0, 20.0),
        ];
        // A clockwise hole (as the planar face provides them), a diamond.
        for (x, y) in [(10.0, 15.0), (15.0, 10.0), (10.0, 5.0), (5.0, 10.0)] {
            pts.push(Vec2::new(x, y));
        }
        let outer: Vec<usize> = (0..4).collect();
        let hole: Vec<usize> = (4..8).collect();
        let poly = bridge_holes(&pts, &outer, &[hole]).unwrap();
        let t = ear_clip(&pts, poly.clone()).unwrap();
        let sum: f64 = t
            .iter()
            .map(|t| 0.5 * orient2d(pts[t[0]], pts[t[1]], pts[t[2]]))
            .sum();
        assert!((sum - (400.0 - 50.0)).abs() < 1e-9, "{sum}");
        for tri in &t {
            assert!(orient2d(pts[tri[0]], pts[tri[1]], pts[tri[2]]) > 0.0);
        }
    }

    #[test]
    fn a_vertex_a_hair_off_a_straight_line_still_triangulates_validly() {
        // A square with an extra vertex on the bottom edge, pushed out or in by 1e-17: a sliver
        // ear or a sliver dent. (The exact predicate itself is tested on genuinely hard cases in
        // `dc_math::robust`; this checks the triangulation copes with such vertices.)
        let poly: Vec<usize> = (0..5).collect();
        for dy in [1e-17, -1e-17] {
            let pts = [
                Vec2::new(0.0, 0.0),
                Vec2::new(0.5, dy),
                Vec2::new(1.0, 0.0),
                Vec2::new(1.0, 1.0),
                Vec2::new(0.0, 1.0),
            ];
            let t = ear_clip(&pts, poly.clone()).unwrap();
            check(&pts, &poly, &t, true);
        }
    }

    #[test]
    fn large_polygons_are_fast_and_right() {
        // 20,000 vertices on a circle: convex, so no reflex vertices to test against.
        let n = 20_000;
        let pts = circle(n, 100.0);
        let poly: Vec<usize> = (0..n).collect();
        let started = std::time::Instant::now();
        let t = ear_clip(&pts, poly.clone()).unwrap();
        let took = started.elapsed();
        eprintln!("20000 convex vertices: {took:?}");
        assert_eq!(t.len(), n - 2);
        check(&pts, &poly, &t, false);
        assert!(
            took.as_secs_f64() < 3.0,
            "20000 convex vertices took {took:?} (debug build)"
        );
        // A comb: 400 teeth give 800 reflex vertices, the slow case for this method.
        let teeth = 400;
        let mut comb = vec![Vec2::new(0.0, 0.0), Vec2::new(teeth as f64 * 2.0, 0.0)];
        for i in (0..teeth).rev() {
            let x = i as f64 * 2.0;
            comb.push(Vec2::new(x + 2.0, 10.0));
            comb.push(Vec2::new(x + 1.5, 10.0));
            comb.push(Vec2::new(x + 1.5, 2.0));
            comb.push(Vec2::new(x + 0.5, 2.0));
            comb.push(Vec2::new(x + 0.5, 10.0));
            comb.push(Vec2::new(x, 10.0));
        }
        // (the polygon revisits x + 2.0 / x on consecutive teeth; drop exact duplicates)
        let mut cleaned: Vec<Vec2> = Vec::new();
        for p in comb {
            if cleaned.last() != Some(&p) {
                cleaned.push(p);
            }
        }
        if cleaned.first() == cleaned.last() {
            cleaned.pop();
        }
        let poly: Vec<usize> = (0..cleaned.len()).collect();
        let started = std::time::Instant::now();
        let t = ear_clip(&cleaned, poly.clone()).unwrap();
        let took = started.elapsed();
        eprintln!("comb of {} vertices: {took:?}", cleaned.len());
        check(&cleaned, &poly, &t, false);
        assert!(
            took.as_secs_f64() < 5.0,
            "the comb took {took:?} (debug build)"
        );
    }
}
