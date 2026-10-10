//! The back half of the boolean: placing pieces of curve in a face's flat picture, merging
//! coplanar faces, fusing pieces that only a spurious vertex separates, and stitching the result
//! into a [`Solid`].

use super::geo::{Arc, Geo};
use super::planar::{inside_even_odd, signed_area2, AEdge, P2};
use super::prep::{BFace, Surf, TOL};
use super::{BoolError, BoolOptions, BoolResult, FaceSource, OutFace, PieceTable, Pool, Seg, Side};
use crate::solid::{CoEdge, Curve, Edge, Face, Id, Solid, Surface};
use dc_math::{det, Tolerance, Vec3};
use std::collections::{BTreeMap, BTreeSet};

const TAU: f64 = 2.0 * core::f64::consts::PI;

pub(crate) struct Placed {
    pub seg: Seg,
    pub edge: AEdge,
}

/// Places every piece in the face's picture. Returns the pieces (with arrangement edges) and the
/// position of every node key (`2 * node + copy`; copy 1 is the right edge of a winding
/// cylinder's strip, where seam nodes appear a second time).
pub(crate) fn place_all(face: &BFace, segs: &[Seg]) -> (Vec<Placed>, BTreeMap<u32, P2>) {
    let mut placed = Vec::with_capacity(segs.len());
    let mut kpos: BTreeMap<u32, P2> = BTreeMap::new();
    for s in segs {
        let pts = s.arc.polyline();
        let pic: Vec<P2> = match &face.surf {
            Surf::Plane { .. } => pts.iter().map(|&p| face.to2(p)).collect(),
            Surf::Cyl {
                axis,
                origin,
                e1,
                e2,
                r,
                out,
                winding,
                x0,
                ..
            } => {
                let w = TAU * r;
                let h = |p: Vec3| (p - *origin).dot(*axis);
                let on_seam = *winding
                    && matches!(s.arc.geo, Geo::Line { d, .. } if d.cross(*axis).length() < 1e-9)
                    && (face.to2(s.arc.mid()).0 - x0).abs() < 1e-7 * w;
                if on_seam {
                    // A seam piece: the right edge of the strip if it runs up, the left if down.
                    let x = if h(s.arc.end()) > h(s.arc.start()) {
                        x0 + w
                    } else {
                        *x0
                    };
                    pts.iter().map(|&p| (x, h(p))).collect()
                } else {
                    // Unwrap outwards from the middle, which is inside the strip.
                    let m = pts.len() / 2;
                    let xm = face.to2(pts[m]).0;
                    let raw = |p: Vec3| {
                        let d = p - *origin;
                        out * r * det::atan2(d.dot(*e2), d.dot(*e1))
                    };
                    let mut xs = vec![0.0; pts.len()];
                    xs[m] = xm;
                    for i in (m + 1)..pts.len() {
                        let x = raw(pts[i]);
                        xs[i] = x - w * ((x - xs[i - 1]) / w).round();
                    }
                    for i in (0..m).rev() {
                        let x = raw(pts[i]);
                        xs[i] = x - w * ((x - xs[i + 1]) / w).round();
                    }
                    pts.iter()
                        .enumerate()
                        .map(|(i, &p)| (xs[i], h(p)))
                        .collect()
                }
            }
        };
        let copy_of = |x: f64| -> u32 {
            match &face.surf {
                Surf::Cyl {
                    winding: true,
                    x0,
                    r,
                    ..
                } if (x - (x0 + TAU * r)).abs() < 1e-7 * TAU * r => 1,
                _ => 0,
            }
        };
        let (pa, pb) = (pic[0], pic[pic.len() - 1]);
        let (ka, kb) = (s.a * 2 + copy_of(pa.0), s.b * 2 + copy_of(pb.0));
        kpos.entry(ka).or_insert(pa);
        kpos.entry(kb).or_insert(pb);
        placed.push(Placed {
            seg: s.clone(),
            edge: AEdge {
                a: ka,
                b: kb,
                mid: pic[1..pic.len() - 1].to_vec(),
            },
        });
    }
    (placed, kpos)
}

// ---- merging coplanar faces --------------------------------------------------------------------

/// How a group of faces on one surface is drawn flat, for merging.
trait Picture {
    /// The direction (angle in the picture) of `arc` at its start, or at its end if `at_end`.
    fn dir(&self, arc: &Arc, at_end: bool) -> f64;
    /// The loop as a polygon in the picture; `None` if it winds round the surface (it would
    /// need a seam).
    fn poly(&self, lp: &[Seg]) -> Option<Vec<P2>>;
    /// Is the 3D point `p` of the surface inside the polygon?
    fn contains(&self, poly: &[P2], p: Vec3) -> bool;
}

struct PlanePic {
    origin: Vec3,
    x: Vec3,
    y: Vec3,
}

impl PlanePic {
    fn to2(&self, p: Vec3) -> P2 {
        let d = p - self.origin;
        (d.dot(self.x), d.dot(self.y))
    }
}

impl Picture for PlanePic {
    fn dir(&self, arc: &Arc, at_end: bool) -> f64 {
        let pl = arc.polyline_sag(1e-4);
        let (p, q) = if at_end {
            (pl[pl.len() - 1], pl[pl.len() - 2])
        } else {
            (pl[0], pl[1])
        };
        let (a, b) = (self.to2(p), self.to2(q));
        det::atan2(b.1 - a.1, b.0 - a.0)
    }

    fn poly(&self, lp: &[Seg]) -> Option<Vec<P2>> {
        let mut v = Vec::new();
        for s in lp {
            let pl = s.arc.polyline();
            v.extend(pl[..pl.len() - 1].iter().map(|&p| self.to2(p)));
        }
        Some(v)
    }

    fn contains(&self, poly: &[P2], p: Vec3) -> bool {
        inside_even_odd(std::slice::from_ref(&poly.to_vec()), self.to2(p))
    }
}

struct CylPic {
    origin: Vec3,
    axis: Vec3,
    e1: Vec3,
    e2: Vec3,
    r: f64,
    out: f64,
}

impl CylPic {
    fn raw(&self, p: Vec3) -> P2 {
        let d = p - self.origin;
        (
            self.out * self.r * det::atan2(d.dot(self.e2), d.dot(self.e1)),
            d.dot(self.axis),
        )
    }
}

impl Picture for CylPic {
    fn dir(&self, arc: &Arc, at_end: bool) -> f64 {
        let pl = arc.polyline_sag(1e-4);
        let (p, q) = if at_end {
            (pl[pl.len() - 1], pl[pl.len() - 2])
        } else {
            (pl[0], pl[1])
        };
        let (a, b) = (self.raw(p), self.raw(q));
        let w = TAU * self.r;
        let dx = (b.0 - a.0) - w * ((b.0 - a.0) / w).round();
        det::atan2(b.1 - a.1, dx)
    }

    fn poly(&self, lp: &[Seg]) -> Option<Vec<P2>> {
        let w = TAU * self.r;
        let mut v: Vec<P2> = Vec::new();
        for s in lp {
            let pl = s.arc.polyline();
            for &p in &pl[..pl.len() - 1] {
                let mut q = self.raw(p);
                if let Some(last) = v.last() {
                    q.0 -= w * ((q.0 - last.0) / w).round();
                }
                v.push(q);
            }
        }
        // Does it close, going once more to the start?
        let last = *v.last()?;
        let mut first = self.raw(lp[0].arc.start());
        first.0 -= w * ((first.0 - last.0) / w).round();
        ((first.0 - v[0].0).abs() <= 1e-6 * w).then_some(v)
    }

    fn contains(&self, poly: &[P2], p: Vec3) -> bool {
        let w = TAU * self.r;
        let centre = poly.iter().map(|q| q.0).sum::<f64>() / poly.len() as f64;
        let mut q = self.raw(p);
        q.0 -= w * ((q.0 - centre) / w).round();
        inside_even_odd(std::slice::from_ref(&poly.to_vec()), q)
    }
}

fn surface_key_matches(a: &Surf, b: &Surf) -> bool {
    let tol = Tolerance::DEFAULT;
    match (a, b) {
        (
            Surf::Plane {
                origin: o1,
                normal: n1,
                ..
            },
            Surf::Plane {
                origin: o2,
                normal: n2,
                ..
            },
        ) => n1.dot(*n2) > 0.0 && tol.parallel(*n1, *n2) && (*o2 - *o1).dot(*n1).abs() <= TOL,
        (
            Surf::Cyl {
                origin: o1,
                axis: a1,
                r: r1,
                out: s1,
                ..
            },
            Surf::Cyl {
                origin: o2,
                axis: a2,
                r: r2,
                out: s2,
                ..
            },
        ) => {
            let w = *o2 - *o1;
            tol.parallel(*a1, *a2)
                && (r1 - r2).abs() <= 1e-9
                && s1 == s2
                && (w - *a1 * w.dot(*a1)).length() <= TOL
        }
        _ => false,
    }
}

fn picture_of(surf: &Surf) -> Box<dyn Picture> {
    match surf {
        Surf::Plane {
            origin, x, normal, ..
        } => Box::new(PlanePic {
            origin: *origin,
            x: *x,
            y: normal.cross(*x),
        }),
        Surf::Cyl {
            origin,
            axis,
            e1,
            e2,
            r,
            out,
            ..
        } => Box::new(CylPic {
            origin: *origin,
            axis: *axis,
            e1: *e1,
            e2: *e2,
            r: *r,
            out: *out,
        }),
    }
}

/// Joins faces on one surface (a plane with the same orientation, or a cylinder): a piece of
/// boundary that two *different* faces use in opposite directions cancels, and the remaining
/// boundary is traced again. A piece a face uses twice itself (a seam) stays. If the merged
/// boundary would wind round a cylinder, the group is left as it was.
fn merge_group(g: Vec<OutFace>) -> Result<Vec<OutFace>, BoolError> {
    let pic = picture_of(&g[0].surf);
    let mut sources: Vec<(Side, String)> = Vec::new();
    for p in &g {
        for s in &p.sources {
            if !sources.contains(s) {
                sources.push(s.clone());
            }
        }
    }
    // Half-edges by piece identity; opposite ones from different faces cancel.
    let mut table = PieceTable::default();
    let mut stored: BTreeMap<usize, Seg> = BTreeMap::new();
    let mut occ: BTreeMap<(usize, bool), Vec<usize>> = BTreeMap::new();
    for (fi, p) in g.iter().enumerate() {
        for s in p.loops.iter().flatten() {
            let (id, rev) = table.id(s);
            stored
                .entry(id)
                .or_insert_with(|| if rev { s.reversed() } else { s.clone() });
            let opposite = occ
                .get_mut(&(id, !rev))
                .and_then(|v| v.iter().position(|&f| f != fi).map(|at| v.remove(at)));
            if opposite.is_none() {
                occ.entry((id, rev)).or_default().push(fi);
            }
        }
    }
    let directed = |id: usize, rev: bool| -> Seg {
        let s = &stored[&id];
        if rev {
            s.reversed()
        } else {
            s.clone()
        }
    };
    let mut out_edges: BTreeMap<u32, Vec<(usize, bool)>> = BTreeMap::new();
    for (&(id, rev), faces) in &occ {
        for _ in faces {
            out_edges
                .entry(directed(id, rev).a)
                .or_default()
                .push((id, rev));
        }
    }
    // Trace the boundary loops.
    let mut loops: Vec<Vec<Seg>> = Vec::new();
    while let Some((&u0, hs)) = out_edges.iter().find(|(_, v)| !v.is_empty()) {
        let first = hs[0];
        out_edges.get_mut(&u0).expect("present").remove(0);
        let mut lp = vec![directed(first.0, first.1)];
        loop {
            let last = lp[lp.len() - 1].clone();
            if last.b == lp[0].a {
                break;
            }
            let v = last.b;
            let cands = out_edges.get(&v).cloned().unwrap_or_default();
            if cands.is_empty() {
                return Err(BoolError::Degenerate(
                    "a merged boundary does not close".into(),
                ));
            }
            let next = if cands.len() == 1 {
                cands[0]
            } else {
                // The first edge clockwise from the way we came.
                let back = pic.dir(&last.arc, true);
                let cw = |h: (usize, bool)| {
                    let d = pic.dir(&directed(h.0, h.1).arc, false);
                    (back - d).rem_euclid(TAU)
                };
                *cands
                    .iter()
                    .min_by(|&&a, &&b| cw(a).total_cmp(&cw(b)).then(a.cmp(&b)))
                    .expect("non-empty")
            };
            let list = out_edges.get_mut(&v).expect("present");
            let at = list.iter().position(|&c| c == next).expect("candidate");
            list.remove(at);
            lp.push(directed(next.0, next.1));
        }
        loops.push(lp);
    }
    // Outer boundaries and the holes they contain.
    type Outer = (f64, Vec<Seg>, Vec<P2>, Vec<Vec<Seg>>);
    let mut outers: Vec<Outer> = Vec::new();
    let mut holes: Vec<Vec<Seg>> = Vec::new();
    for lp in loops {
        let Some(poly) = pic.poly(&lp) else {
            return Ok(g); // winds round the surface: keep the faces as they are
        };
        let a = signed_area2(&poly);
        if a > 0.0 {
            outers.push((a, lp, poly, Vec::new()));
        } else {
            holes.push(lp);
        }
    }
    for h in holes {
        let probe = h[0].arc.mid();
        let best = outers
            .iter()
            .enumerate()
            .filter(|(_, o)| pic.contains(&o.2, probe))
            .min_by(|a, b| a.1 .0.total_cmp(&b.1 .0))
            .map(|(i, _)| i);
        match best {
            Some(i) => outers[i].3.push(h),
            None => {
                return Err(BoolError::Degenerate(
                    "a hole boundary lies outside every face boundary".into(),
                ))
            }
        }
    }
    Ok(outers
        .into_iter()
        .map(|(_, o, _, hs)| {
            let mut loops = vec![o];
            loops.extend(hs);
            OutFace {
                sources: sources.clone(),
                surf: g[0].surf.clone(),
                loops,
            }
        })
        .collect())
}

/// Merges faces that lie on one plane or cylinder and share boundary (see [`merge_group`]).
pub(crate) fn merge_coplanar(faces: Vec<OutFace>, pool: &Pool) -> Result<Vec<OutFace>, BoolError> {
    let _ = pool;
    let mut groups: Vec<Vec<OutFace>> = Vec::new();
    for f in faces {
        let at = groups
            .iter()
            .position(|g| surface_key_matches(&g[0].surf, &f.surf));
        match at {
            Some(i) => groups[i].push(f),
            None => groups.push(vec![f]),
        }
    }
    let mut out = Vec::new();
    for g in groups {
        out.extend(merge_group(g)?);
    }
    Ok(out)
}

// ---- fusing --------------------------------------------------------------------------------------

fn same_geo(a: &Geo, b: &Geo) -> bool {
    let close = |p: Vec3, q: Vec3| (p - q).length() <= 1e-9;
    match (a, b) {
        (Geo::Line { o: o1, d: d1 }, Geo::Line { o: o2, d: d2 }) => {
            d1.cross(*d2).length() <= 1e-9 && (*o2 - *o1).cross(*d1).length() <= TOL
        }
        (
            Geo::Circle {
                c: c1,
                n: n1,
                u: u1,
                r: r1,
            },
            Geo::Circle {
                c: c2,
                n: n2,
                u: u2,
                r: r2,
            },
        ) => close(*c1, *c2) && close(*n1, *n2) && close(*u1, *u2) && (r1 - r2).abs() <= 1e-9,
        (
            Geo::Ellipse {
                c: c1,
                n: n1,
                u: u1,
                a: a1,
                b: b1,
            },
            Geo::Ellipse {
                c: c2,
                n: n2,
                u: u2,
                a: a2,
                b: b2,
            },
        ) => {
            close(*c1, *c2)
                && close(*n1, *n2)
                && close(*u1, *u2)
                && (a1 - a2).abs() <= 1e-9
                && (b1 - b2).abs() <= 1e-9
        }
        _ => false,
    }
}

/// The arc running through `first` then `second` (consecutive pieces of one curve), if they are.
fn join_arcs(first: &Arc, second: &Arc) -> Option<Arc> {
    if !same_geo(&first.geo, &second.geo) {
        return None;
    }
    match &first.geo {
        Geo::Line { d, .. } => {
            // Same direction of travel.
            let (d1, d2) = (
                first.geo.tangent(0.0) * (first.t1 - first.t0).signum(),
                second.geo.tangent(0.0) * (second.t1 - second.t0).signum(),
            );
            let _ = d;
            if d1.dot(d2) <= 0.0 {
                return None;
            }
            Arc::line(first.start(), second.end())
        }
        geo => {
            let per = geo.period().expect("closed curve");
            if (first.t1 - first.t0).signum() != (second.t1 - second.t0).signum() {
                return None;
            }
            let gap = (second.t0 - first.t1).rem_euclid(per);
            if gap > 1e-9 && per - gap > 1e-9 {
                return None;
            }
            let total = (first.t1 - first.t0) + (second.t1 - second.t0);
            if total.abs() > per + 1e-9 {
                return None;
            }
            Some(Arc {
                geo: geo.clone(),
                t0: first.t0,
                t1: first.t0 + total,
            })
        }
    }
}

/// Removes a vertex that only joins two pieces of one curve and that no other piece needs.
pub(crate) fn fuse(mut faces: Vec<OutFace>, pool: &Pool) -> Result<Vec<OutFace>, BoolError> {
    let mut skip: BTreeSet<u32> = BTreeSet::new();
    loop {
        let mut table = PieceTable::default();
        let mut inc: BTreeMap<u32, BTreeSet<usize>> = BTreeMap::new();
        for f in &faces {
            for s in f.loops.iter().flatten() {
                let (id, _) = table.id(s);
                inc.entry(s.a).or_default().insert(id);
                inc.entry(s.b).or_default().insert(id);
            }
        }
        let mut done = false;
        for (&v, ids) in &inc {
            if ids.len() != 2 || skip.contains(&v) {
                continue;
            }
            // Try every occurrence of the pair at v in every loop.
            let mut new_faces: Vec<OutFace> = Vec::new();
            let mut ok = true;
            let mut replaced = 0;
            for f in &faces {
                let mut loops: Vec<Vec<Seg>> = Vec::new();
                for lp in &f.loops {
                    let n = lp.len();
                    let mut out: Vec<Seg> = Vec::new();
                    // Start where no join at `v` can span the wrap-around.
                    let start = (0..n).find(|&k| lp[k].a != v).unwrap_or(0);
                    let rot: Vec<Seg> = (0..n).map(|k| lp[(start + k) % n].clone()).collect();
                    let mut i = 0;
                    while i < rot.len() {
                        if rot[i].b == v && i + 1 < rot.len() && rot[i + 1].a == v {
                            match join_arcs(&rot[i].arc, &rot[i + 1].arc) {
                                Some(arc) => {
                                    out.push(Seg {
                                        a: rot[i].a,
                                        b: rot[i + 1].b,
                                        arc,
                                    });
                                    replaced += 1;
                                    i += 2;
                                    continue;
                                }
                                None => ok = false,
                            }
                        }
                        out.push(rot[i].clone());
                        i += 1;
                    }
                    loops.push(out);
                }
                new_faces.push(OutFace {
                    sources: f.sources.clone(),
                    surf: f.surf.clone(),
                    loops,
                });
            }
            if ok && replaced > 0 {
                faces = new_faces;
                done = true;
                break;
            }
            skip.insert(v);
        }
        if !done {
            let _ = pool;
            return Ok(faces);
        }
    }
}

// ---- building the solid ------------------------------------------------------------------------

fn curve_of(arc: &Arc) -> Curve {
    match &arc.geo {
        Geo::Line { .. } => Curve::Line,
        Geo::Circle { c, n, u, r } => {
            if arc.t1 >= arc.t0 {
                Curve::Circle {
                    center: *c,
                    axis: *n,
                    x_dir: *u,
                    radius: *r,
                    t0: arc.t0,
                    sweep: arc.t1 - arc.t0,
                }
            } else {
                Curve::Circle {
                    center: *c,
                    axis: -*n,
                    x_dir: *u,
                    radius: *r,
                    t0: -arc.t0,
                    sweep: arc.t0 - arc.t1,
                }
            }
        }
        Geo::Ellipse { c, n, u, a, b } => {
            if arc.t1 >= arc.t0 {
                Curve::Ellipse {
                    center: *c,
                    axis: *n,
                    x_dir: *u,
                    a: *a,
                    b: *b,
                    t0: arc.t0,
                    sweep: arc.t1 - arc.t0,
                }
            } else {
                Curve::Ellipse {
                    center: *c,
                    axis: -*n,
                    x_dir: *u,
                    a: *a,
                    b: *b,
                    t0: -arc.t0,
                    sweep: arc.t0 - arc.t1,
                }
            }
        }
    }
}

/// The same solid with faces ordered by name, edges by name and vertices by first use, so its
/// bytes (and hash) do not depend on the order the construction happened to visit things in.
fn canonical_order(solid: Solid) -> Solid {
    let mut face_order: Vec<usize> = (0..solid.faces.len()).collect();
    face_order.sort_by(|&a, &b| solid.faces[a].name.cmp(&solid.faces[b].name));
    let mut edge_order: Vec<usize> = (0..solid.edges.len()).collect();
    edge_order.sort_by(|&a, &b| solid.edges[a].name.cmp(&solid.edges[b].name));
    let mut edge_new: Vec<Id> = vec![0; solid.edges.len()];
    for (new, &old) in edge_order.iter().enumerate() {
        edge_new[old] = new as Id;
    }
    let mut vert_new: BTreeMap<Id, Id> = BTreeMap::new();
    let mut vertices: Vec<Vec3> = Vec::new();
    let mut take = |v: Id, vertices: &mut Vec<Vec3>| {
        *vert_new.entry(v).or_insert_with(|| {
            vertices.push(solid.vertices[v as usize]);
            (vertices.len() - 1) as Id
        })
    };
    let mut edges: Vec<Edge> = Vec::with_capacity(solid.edges.len());
    for &old in &edge_order {
        let e = &solid.edges[old];
        let (v0, v1) = (take(e.v0, &mut vertices), take(e.v1, &mut vertices));
        edges.push(Edge {
            name: e.name.clone(),
            v0,
            v1,
            curve: e.curve.clone(),
        });
    }
    let faces: Vec<Face> = face_order
        .iter()
        .map(|&i| {
            let f = &solid.faces[i];
            Face {
                name: f.name.clone(),
                surface: f.surface.clone(),
                same_sense: f.same_sense,
                loops: f
                    .loops
                    .iter()
                    .map(|lp| {
                        lp.iter()
                            .map(|c| CoEdge {
                                edge: edge_new[c.edge as usize],
                                reversed: c.reversed,
                            })
                            .collect()
                    })
                    .collect(),
            }
        })
        .collect();
    Solid {
        vertices,
        edges,
        faces,
    }
}

pub(crate) fn finish(
    faces: Vec<OutFace>,
    pool: &Pool,
    opts: &BoolOptions,
) -> Result<BoolResult, BoolError> {
    let mut vid: BTreeMap<u32, Id> = BTreeMap::new();
    let mut vertices: Vec<Vec3> = Vec::new();
    let mut vertex = |n: u32, vertices: &mut Vec<Vec3>| -> Id {
        *vid.entry(n).or_insert_with(|| {
            vertices.push(pool.p(n));
            (vertices.len() - 1) as Id
        })
    };
    let mut table = PieceTable::default();
    let mut edges: Vec<Edge> = Vec::new();
    let mut edge_of: BTreeMap<usize, Id> = BTreeMap::new();
    let mut out_faces: Vec<Face> = Vec::new();
    let mut bases: Vec<String> = Vec::new();
    for f in &faces {
        let (side, first) = &f.sources[0];
        let base = match (side, &opts.b_namer) {
            (Side::B, Some(namer)) => namer(first),
            (Side::B, None) => format!("{}{first}", opts.b_prefix),
            _ => first.clone(),
        };
        bases.push(base);
        let mut loops: Vec<Vec<CoEdge>> = Vec::new();
        let (mut hmin, mut hmax) = (f64::MAX, f64::MIN);
        for lp in &f.loops {
            let mut cl = Vec::new();
            for s in lp {
                let (id, rev) = table.id(s);
                let eid = *edge_of.entry(id).or_insert_with(|| {
                    let (a, b) = (vertex(s.a, &mut vertices), vertex(s.b, &mut vertices));
                    edges.push(Edge {
                        name: String::new(),
                        v0: a,
                        v1: b,
                        curve: curve_of(&s.arc),
                    });
                    (edges.len() - 1) as Id
                });
                vertex(s.a, &mut vertices);
                vertex(s.b, &mut vertices);
                cl.push(CoEdge {
                    edge: eid,
                    reversed: rev,
                });
                if let Surf::Cyl { origin, axis, .. } = &f.surf {
                    for p in s.arc.polyline_sag(1e-3) {
                        let h = (p - *origin).dot(*axis);
                        hmin = hmin.min(h);
                        hmax = hmax.max(h);
                    }
                }
            }
            loops.push(cl);
        }
        let (surface, same_sense) = match &f.surf {
            Surf::Plane {
                origin, normal, x, ..
            } => (
                Surface::Plane {
                    origin: *origin,
                    normal: *normal,
                    x_dir: *x,
                },
                true,
            ),
            Surf::Cyl {
                origin,
                axis,
                e1,
                r,
                out,
                ..
            } => (
                Surface::Cylinder {
                    origin: *origin,
                    axis: *axis,
                    ref_dir: *e1,
                    radius: *r,
                    phi0: 0.0,
                    phi1: TAU,
                    h0: hmin,
                    h1: hmax,
                },
                *out > 0.0,
            ),
        };
        out_faces.push(Face {
            name: String::new(),
            surface,
            same_sense,
            loops,
        });
    }
    let mut solid = Solid {
        vertices,
        edges,
        faces: out_faces,
    };
    let (face_names, edge_names) = crate::naming::structural_names(&solid, &bases);
    for (f, n) in solid.faces.iter_mut().zip(&face_names) {
        f.name = n.clone();
    }
    for (e, n) in solid.edges.iter_mut().zip(&edge_names) {
        e.name = n.clone();
    }
    // Sources, in the final (canonical) order of the faces.
    let mut by_name: BTreeMap<String, Vec<(Side, String)>> = BTreeMap::new();
    for (f, n) in faces.iter().zip(&face_names) {
        by_name.insert(n.clone(), f.sources.clone());
    }
    let solid = canonical_order(solid);
    let sources = solid
        .faces
        .iter()
        .map(|f| FaceSource {
            face: f.name.clone(),
            from: by_name[&f.name].clone(),
        })
        .collect();
    let issues = solid.validate();
    if !issues.is_empty() {
        return Err(BoolError::Invalid(issues));
    }
    Ok(BoolResult { solid, sources })
}
