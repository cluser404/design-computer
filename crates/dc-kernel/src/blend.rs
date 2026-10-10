//! Fillet and chamfer of straight edges between flat faces, by local surgery on the B-rep.
//!
//! An edge between faces F1 and F2 is replaced by two new edges, one on each face, a distance
//! `t` from where it was (`t = r / tan(φ/2)` for a fillet of radius `r` between faces whose
//! in-face directions away from the edge make the angle `φ`; `t = d` for a chamfer), and a new
//! face between them: part of a cylinder of radius `r` (or a plane for a chamfer). At each end
//! the edge must end at a corner of exactly three flat faces; the third face loses a corner to an
//! arc (circle or ellipse, wherever the cylinder meets that plane) or a straight line. Convex
//! edges lose material, concave ones gain it. Edges that share a vertex, ends with more than
//! three faces and curved neighbours are refused with the reason (corner blends need spheres and
//! similar surfaces the kernel does not have yet).

use crate::boolean::geo::{plane_cylinder_section, Arc, Geo};
use crate::naming::structural_names;
use crate::solid::{CoEdge, Curve, Edge, Face, Id, Solid, Surface};
use crate::validate::Issue;
use dc_math::{det, Tolerance, Vec3};
use std::collections::{BTreeMap, BTreeSet};

const TAU: f64 = 2.0 * core::f64::consts::PI;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BlendKind {
    Fillet,
    Chamfer,
}

#[derive(Clone, Debug, PartialEq)]
pub enum BlendError {
    /// A named face or an edge between two named faces does not exist.
    NotFound(String),
    /// Something this version cannot do, with the reason.
    Unsupported(String),
    /// The size is too big for the part around the edge.
    TooLarge(String),
    /// The construction produced an invalid solid (a bug); the issues say how.
    Invalid(Vec<Issue>),
}

impl core::fmt::Display for BlendError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            BlendError::NotFound(s) | BlendError::Unsupported(s) | BlendError::TooLarge(s) => {
                write!(f, "{s}")
            }
            BlendError::Invalid(i) => write!(f, "the result is not a valid solid: {i:?}"),
        }
    }
}

impl std::error::Error for BlendError {}

fn plane_of(f: &Face) -> Option<(Vec3, Vec3)> {
    match &f.surface {
        Surface::Plane { origin, normal, .. } => {
            Some((*origin, if f.same_sense { *normal } else { -*normal }))
        }
        _ => None,
    }
}

fn faces_of_edge(s: &Solid, e: usize) -> Vec<usize> {
    let mut out = Vec::new();
    for (fi, f) in s.faces.iter().enumerate() {
        for c in f.loops.iter().flatten() {
            if c.edge as usize == e {
                out.push(fi);
            }
        }
    }
    out
}

fn touches(s: &Solid, e: usize, v: Id) -> bool {
    s.edges[e].v0 == v || s.edges[e].v1 == v
}

fn faces_at_vertex(s: &Solid, v: Id) -> BTreeSet<usize> {
    let mut out = BTreeSet::new();
    for (fi, f) in s.faces.iter().enumerate() {
        if f.loops
            .iter()
            .flatten()
            .any(|c| touches(s, c.edge as usize, v))
        {
            out.insert(fi);
        }
    }
    out
}

fn curve_of_arc(arc: &Arc) -> Curve {
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

/// Rounds or bevels every straight edge shared by each named pair of faces. `size` is the radius
/// of a fillet or the distance of a chamfer; `owner` names the new faces
/// (`fillet:<owner>`, `fillet:<owner>~1`, ...).
pub fn blend_edges(
    solid: &Solid,
    pairs: &[[String; 2]],
    kind: BlendKind,
    size: f64,
    owner: &str,
) -> Result<Solid, BlendError> {
    if !size.is_finite() || size <= 0.0 {
        return Err(BlendError::Unsupported(format!(
            "the size must be finite and greater than zero, got {size}"
        )));
    }
    let face_index = |name: &str| -> Result<usize, BlendError> {
        solid
            .faces
            .iter()
            .position(|f| f.name == name)
            .ok_or_else(|| {
                BlendError::NotFound(format!(
                    "the part has no face '{name}' (faces: {})",
                    solid
                        .faces
                        .iter()
                        .take(20)
                        .map(|f| f.name.as_str())
                        .collect::<Vec<_>>()
                        .join(", ")
                ))
            })
    };
    let mut targets: Vec<usize> = Vec::new();
    for pair in pairs {
        let (a, b) = (face_index(&pair[0])?, face_index(&pair[1])?);
        let in_a: BTreeSet<usize> = solid.faces[a]
            .loops
            .iter()
            .flatten()
            .map(|c| c.edge as usize)
            .collect();
        let shared: Vec<usize> = solid.faces[b]
            .loops
            .iter()
            .flatten()
            .map(|c| c.edge as usize)
            .filter(|e| in_a.contains(e))
            .collect();
        if shared.is_empty() {
            return Err(BlendError::NotFound(format!(
                "the faces '{}' and '{}' do not meet along an edge",
                pair[0], pair[1]
            )));
        }
        for e in shared {
            if solid.edges[e].curve != Curve::Line {
                return Err(BlendError::Unsupported(format!(
                    "the edge between '{}' and '{}' is curved (a circular rim is rounded by the rim rounding, not here)",
                    pair[0], pair[1]
                )));
            }
            if !targets.contains(&e) {
                targets.push(e);
            }
        }
    }
    let mut seen: BTreeSet<Id> = BTreeSet::new();
    for &e in &targets {
        for v in [solid.edges[e].v0, solid.edges[e].v1] {
            if !seen.insert(v) {
                return Err(BlendError::Unsupported(
                    "two of the edges meet at a corner: blending them needs a corner patch (a sphere or similar), which the kernel does not have yet".into(),
                ));
            }
        }
    }
    let before = solid.volume();
    let mut w = solid.clone();
    let mut convex_all = true;
    let mut concave_all = true;
    for (k, &e) in targets.iter().enumerate() {
        let name = if k == 0 {
            format!(
                "{}:{owner}",
                if kind == BlendKind::Fillet {
                    "fillet"
                } else {
                    "chamfer"
                }
            )
        } else {
            format!(
                "{}:{owner}~{k}",
                if kind == BlendKind::Fillet {
                    "fillet"
                } else {
                    "chamfer"
                }
            )
        };
        let convex = blend_one(&mut w, e, kind, size, name)?;
        convex_all &= convex;
        concave_all &= !convex;
    }
    let mut w = compact(w);
    // Edge names from the faces on either side.
    let bases: Vec<String> = w.faces.iter().map(|f| f.name.clone()).collect();
    let (_, enames) = structural_names(&w, &bases);
    for (e, n) in w.edges.iter_mut().zip(enames) {
        e.name = n;
    }
    let issues = w.validate();
    if !issues.is_empty() {
        return Err(BlendError::Invalid(issues));
    }
    let after = w.volume();
    if (convex_all && after >= before) || (concave_all && after <= before) {
        return Err(BlendError::TooLarge(format!(
            "the {} does not fit the part around the edge (volume went from {before:.6} to {after:.6})",
            if kind == BlendKind::Fillet { "radius" } else { "distance" }
        )));
    }
    Ok(w)
}

/// Blends one edge of `w` in place. Returns whether the edge was convex.
fn blend_one(
    w: &mut Solid,
    e: usize,
    kind: BlendKind,
    size: f64,
    face_name: String,
) -> Result<bool, BlendError> {
    let tol = Tolerance::DEFAULT;
    let users = faces_of_edge(w, e);
    if users.len() != 2 || users[0] == users[1] {
        return Err(BlendError::Unsupported(
            "the edge is not shared by exactly two different faces".into(),
        ));
    }
    let (f1, f2) = (users[0], users[1]);
    let (Some((o1, n1)), Some((o2, n2))) = (plane_of(&w.faces[f1]), plane_of(&w.faces[f2])) else {
        return Err(BlendError::Unsupported(format!(
            "the faces '{}' and '{}' are not both flat (only an edge between two flat faces can be blended so far)",
            w.faces[f1].name, w.faces[f2].name
        )));
    };
    let _ = (o1, o2);
    let (ev0, ev1) = (w.edges[e].v0, w.edges[e].v1);
    let (p0, p1) = (w.vertex(ev0), w.vertex(ev1));
    let d = (p1 - p0)
        .normalized(tol)
        .ok_or_else(|| BlendError::Unsupported("the edge has no length".into()))?;
    // Directions into each face, away from the edge: left of the loop's travel.
    let into = |f: usize, n: Vec3| -> Result<(Vec3, bool), BlendError> {
        for lp in &w.faces[f].loops {
            for c in lp {
                if c.edge as usize == e {
                    let (a, b) = w.coedge_ends(*c);
                    let t = (w.vertex(b) - w.vertex(a))
                        .normalized(tol)
                        .expect("edge length");
                    return Ok((n.cross(t), c.reversed));
                }
            }
        }
        Err(BlendError::Unsupported(
            "the edge is missing from a face".into(),
        ))
    };
    let ((u1, rev1), (u2, rev2)) = (into(f1, n1)?, into(f2, n2)?);
    let cosphi = u1.dot(u2).clamp(-1.0, 1.0);
    let phi = det::acos(cosphi);
    if phi < 1e-3 || phi > core::f64::consts::PI - 1e-3 {
        return Err(BlendError::Unsupported(
            "the two faces are (nearly) in one plane or fold back on each other: there is no edge to blend".into(),
        ));
    }
    let convex = n1.dot(u2) < 0.0;
    let half = 0.5 * phi;
    let (t, off) = match kind {
        BlendKind::Fillet => (size / det::tan(half), size / det::sin(half)),
        BlendKind::Chamfer => (size, 0.0),
    };
    let bis = (u1 + u2).normalized(tol).expect("faces are not opposite");
    let centre = p0 + bis * off;
    let t1_at = |p: Vec3| p + u1 * t;
    let t2_at = |p: Vec3| p + u2 * t;
    // Outward direction of the new face (seen from the part's outside), roughly.
    let out_dir = if convex { -bis } else { bis };
    let e1_dir = (t1_at(p0) - centre).normalized(tol).unwrap_or(-bis);
    let name_of = |w: &Solid, f: usize| w.faces[f].name.clone();
    let _ = name_of;

    struct End {
        p1: Id,
        p2: Id,
        arc: Id,
    }
    let mut ends: Vec<End> = Vec::new();
    for v in [ev0, ev1] {
        let pv = w.vertex(v);
        let around = faces_at_vertex(w, v);
        if around.len() != 3 {
            return Err(BlendError::Unsupported(format!(
                "the edge ends at a vertex where {} faces meet; only corners of three flat faces can be blended so far",
                around.len()
            )));
        }
        let f3 = *around
            .iter()
            .find(|&&f| f != f1 && f != f2)
            .ok_or_else(|| BlendError::Unsupported("corner without a third face".into()))?;
        let Some((o3, n3)) = plane_of(&w.faces[f3]) else {
            return Err(BlendError::Unsupported(format!(
                "the edge ends on the curved face '{}'; only flat end faces can be blended so far",
                w.faces[f3].name
            )));
        };
        let neighbour = |f: usize| -> Result<usize, BlendError> {
            let mut found = None;
            for c in w.faces[f].loops.iter().flatten() {
                let ce = c.edge as usize;
                if ce != e && touches(w, ce, v) {
                    found = Some(ce);
                }
            }
            found.ok_or_else(|| BlendError::Unsupported("a face has no edge at the corner".into()))
        };
        let (g1, g2) = (neighbour(f1)?, neighbour(f2)?);
        for g in [g1, g2] {
            if w.edges[g].curve != Curve::Line {
                return Err(BlendError::Unsupported(
                    "an edge at the corner is curved".into(),
                ));
            }
            if !faces_of_edge(w, g).contains(&f3) {
                return Err(BlendError::Unsupported(
                    "the edges at the corner do not meet the third face".into(),
                ));
            }
        }
        if g1 == g2 {
            return Err(BlendError::Unsupported("the corner is degenerate".into()));
        }
        let dn = d.dot(n3);
        if dn.abs() < 1e-9 {
            return Err(BlendError::Unsupported(
                "the edge runs along its end face".into(),
            ));
        }
        let hit = |origin: Vec3| origin + d * ((o3 - origin).dot(n3) / dn);
        let (q1, q2) = (hit(t1_at(pv)), hit(t2_at(pv)));
        for (g, q) in [(g1, q1), (g2, q2)] {
            let (ga, gb) = (w.vertex(w.edges[g].v0), w.vertex(w.edges[g].v1));
            let far = if w.edges[g].v0 == v { gb } else { ga };
            let len2 = (pv - far).dot(pv - far);
            let s = (q - far).dot(pv - far) / len2;
            if s <= 1e-9 || s >= 1.0 - 1e-9 {
                return Err(BlendError::TooLarge(format!(
                    "the {} reaches past the neighbouring edge of the part at one end",
                    if kind == BlendKind::Fillet {
                        "radius"
                    } else {
                        "distance"
                    }
                )));
            }
        }
        // New vertices; the two neighbouring edges are shortened to end at them.
        w.vertices.push(q1);
        let p1v = (w.vertices.len() - 1) as Id;
        w.vertices.push(q2);
        let p2v = (w.vertices.len() - 1) as Id;
        for (g, nv) in [(g1, p1v), (g2, p2v)] {
            if w.edges[g].v0 == v {
                w.edges[g].v0 = nv;
            } else {
                w.edges[g].v1 = nv;
            }
        }
        // The edge that closes the third face's corner.
        let curve = match kind {
            BlendKind::Chamfer => Curve::Line,
            BlendKind::Fillet => {
                let e2v = d.cross(e1_dir);
                let _ = e2v;
                let geo =
                    plane_cylinder_section(o3, n3, centre, d, e1_dir, size).ok_or_else(|| {
                        BlendError::Unsupported("the end face is parallel to the edge".into())
                    })?;
                let (ts, te) = (geo.param_of(q1), geo.param_of(q2));
                let up = te + TAU * ((ts - te) / TAU).ceil();
                let up = if up <= ts { up + TAU } else { up };
                let down = ts - (up - ts - 0.0);
                let _ = down;
                let down = te - TAU * ((te - ts) / TAU).ceil();
                let down = if down >= ts { down - TAU } else { down };
                let towards = (-bis).normalized(tol).expect("unit");
                let pick = |t1: f64| {
                    let m = geo.point(0.5 * (ts + t1));
                    (m - centre).dot(towards)
                };
                let (a_up, a_down) = (pick(up), pick(down));
                let t_end = if a_up >= a_down { up } else { down };
                curve_of_arc(&Arc {
                    geo,
                    t0: ts,
                    t1: t_end,
                })
            }
        };
        w.edges.push(Edge {
            name: String::new(),
            v0: p1v,
            v1: p2v,
            curve,
        });
        let arc_edge = (w.edges.len() - 1) as Id;
        // Third face: the arc goes where the corner was, between the two shortened edges.
        let lp_of = |w: &Solid| -> Option<(usize, usize, bool)> {
            for (li, lp) in w.faces[f3].loops.iter().enumerate() {
                for i in 0..lp.len() {
                    let (a, b) = (lp[i].edge as usize, lp[(i + 1) % lp.len()].edge as usize);
                    if a == g1 && b == g2 {
                        return Some((li, i, false));
                    }
                    if a == g2 && b == g1 {
                        return Some((li, i, true));
                    }
                }
            }
            None
        };
        let (li, i, flipped) = lp_of(w).ok_or_else(|| {
            BlendError::Unsupported("the corner is not a simple corner of the third face".into())
        })?;
        let coedge = CoEdge {
            edge: arc_edge,
            reversed: flipped,
        };
        w.faces[f3].loops[li].insert(i + 1, coedge);
        ends.push(End {
            p1: p1v,
            p2: p2v,
            arc: arc_edge,
        });
    }
    // The new edges along the faces.
    let (end_v, end_w) = (&ends[0], &ends[1]);
    let line = |a: Id, b: Id| Edge {
        name: String::new(),
        v0: a,
        v1: b,
        curve: Curve::Line,
    };
    w.edges.push(line(end_v.p1, end_w.p1));
    let e1_id = (w.edges.len() - 1) as Id;
    w.edges.push(line(end_v.p2, end_w.p2));
    let e2_id = (w.edges.len() - 1) as Id;
    for (f, new, rev) in [(f1, e1_id, rev1), (f2, e2_id, rev2)] {
        for lp in &mut w.faces[f].loops {
            for c in lp.iter_mut() {
                if c.edge as usize == e {
                    *c = CoEdge {
                        edge: new,
                        reversed: rev,
                    };
                }
            }
        }
    }
    // The new face between them.
    let mut lp = vec![
        CoEdge {
            edge: e1_id,
            reversed: false,
        },
        CoEdge {
            edge: end_w.arc,
            reversed: false,
        },
        CoEdge {
            edge: e2_id,
            reversed: true,
        },
        CoEdge {
            edge: end_v.arc,
            reversed: true,
        },
    ];
    // Counter-clockwise seen from outside: the face lies to the left of the travel.
    let a_mid = (w.vertex(end_v.p1) + w.vertex(end_w.p1)) * 0.5;
    let b_mid = (w.vertex(end_v.p2) + w.vertex(end_w.p2)) * 0.5;
    let travel = (w.vertex(end_w.p1) - w.vertex(end_v.p1))
        .normalized(tol)
        .expect("length");
    let inward = (b_mid - a_mid).normalized(tol).expect("two distinct edges");
    if out_dir.cross(travel).dot(inward) < 0.0 {
        lp.reverse();
        for c in &mut lp {
            c.reversed = !c.reversed;
        }
    }
    let (surface, same_sense) = match kind {
        BlendKind::Fillet => (
            Surface::Cylinder {
                origin: centre,
                axis: d,
                ref_dir: e1_dir,
                radius: size,
                phi0: 0.0,
                phi1: TAU,
                h0: 0.0,
                h1: (p1 - p0).length(),
            },
            convex,
        ),
        BlendKind::Chamfer => {
            let across = (t2_at(p0) - t1_at(p0)).normalized(tol).expect("distinct");
            let mut n = d.cross(across).normalized(tol).expect("not parallel");
            if n.dot(out_dir) < 0.0 {
                n = -n;
            }
            (
                Surface::Plane {
                    origin: t1_at(p0),
                    normal: n,
                    x_dir: d,
                },
                true,
            )
        }
    };
    w.faces.push(Face {
        name: face_name,
        surface,
        same_sense,
        loops: vec![lp],
    });
    // The old edge is gone: leave it unreferenced; `compact` drops it.
    Ok(convex)
}

/// Drops edges and vertices nothing refers to any more and renumbers the rest in order.
fn compact(s: Solid) -> Solid {
    let mut used_e: BTreeSet<usize> = BTreeSet::new();
    for f in &s.faces {
        for c in f.loops.iter().flatten() {
            used_e.insert(c.edge as usize);
        }
    }
    let mut e_new: BTreeMap<usize, Id> = BTreeMap::new();
    let mut edges: Vec<Edge> = Vec::new();
    let mut v_new: BTreeMap<Id, Id> = BTreeMap::new();
    let mut vertices: Vec<Vec3> = Vec::new();
    for &e in &used_e {
        let ed = &s.edges[e];
        let mut map = |v: Id| -> Id {
            *v_new.entry(v).or_insert_with(|| {
                vertices.push(s.vertices[v as usize]);
                (vertices.len() - 1) as Id
            })
        };
        let (v0, v1) = (map(ed.v0), map(ed.v1));
        e_new.insert(e, edges.len() as Id);
        edges.push(Edge {
            name: ed.name.clone(),
            v0,
            v1,
            curve: ed.curve.clone(),
        });
    }
    let faces = s
        .faces
        .iter()
        .map(|f| Face {
            name: f.name.clone(),
            surface: f.surface.clone(),
            same_sense: f.same_sense,
            loops: f
                .loops
                .iter()
                .map(|lp| {
                    lp.iter()
                        .map(|c| CoEdge {
                            edge: e_new[&(c.edge as usize)],
                            reversed: c.reversed,
                        })
                        .collect()
                })
                .collect(),
        })
        .collect();
    Solid {
        vertices,
        edges,
        faces,
    }
}
