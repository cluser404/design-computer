//! Full revolves of profiles whose edges are all parallel or perpendicular to the axis.
//!
//! A line parallel to the axis sweeps a cylinder and a line across it sweeps a flat ring, the
//! two surfaces the kernel has. Such a body (a turned shaft, a bushing, a washer, a pulley
//! blank) is exactly a stack of concentric discs and rings along the axis, which is what the
//! stepped model is, so it is built there and every later cut, boss, pattern and sketch on a
//! face works on it like on any other part. A slanted line (a cone), an arc (a sphere or a
//! torus) or a partial turn would need surfaces and cuts the kernel does not have yet, and is
//! refused with that reason.

use crate::stepped::{Level, Role, StepError, Stepped};
use dc_kernel::Solid;
use dc_math::{Plane, Tolerance, Vec2};
use dc_sketch::{Curve2, Loop, LoopEdge, Profile};

/// One straight edge of the profile in axis coordinates: `z` along the axis, `r` across it.
struct Edge {
    id: String,
    z: [f64; 2],
    r: [f64; 2],
}

fn circle(id: &str, r: f64) -> Loop {
    Loop {
        edges: vec![LoopEdge {
            id: id.to_owned(),
            reversed: false,
            from: [r, 0.0],
            to: [r, 0.0],
            curve: Curve2::Circle {
                center: [0.0, 0.0],
                radius: r,
            },
        }],
        area: core::f64::consts::PI * r * r,
    }
}

/// Revolves `profile` (in `sketch`'s 2D frame) once round the axis through `origin` along
/// `dir` (also in that frame).
///
/// The result lives in a model plane perpendicular to the axis, at the axis point `origin`,
/// with `w` measured along the axis. Edges parallel to the axis become the cylinders named
/// `side:<edge id>`; the two ends are `cap_start` and `cap_end`; a step in between is
/// `cap_start:<feature>.<edge id>`.
pub fn revolve(
    sketch: &Plane,
    profile: &Profile,
    origin: Vec2,
    dir: Vec2,
    feature: &str,
) -> Result<Stepped, StepError> {
    let lin = Tolerance::DEFAULT.linear;
    let len = dir.length();
    if !len.is_finite() || len <= lin {
        return Err(StepError::Nothing(
            "the axis has no direction (its two ends coincide)".into(),
        ));
    }
    let d = dir / len;
    let perp = Vec2::new(-d.y, d.x);
    let zr = |a: [f64; 2]| {
        let v = Vec2::new(a[0], a[1]) - origin;
        (v.dot(d), v.dot(perp))
    };

    // The model plane: origin on the axis, normal along it, x across it.
    let o3 = sketch.to_3d(origin);
    let n3 = (sketch.to_3d(origin + d) - o3).normalized(Tolerance::DEFAULT);
    let x3 = (sketch.to_3d(origin + perp) - o3).normalized(Tolerance::DEFAULT);
    let (Some(n3), Some(x3)) = (n3, x3) else {
        return Err(StepError::Nothing("the sketch plane is degenerate".into()));
    };
    let model = Plane::new(o3, x3, n3.cross(x3), Tolerance::DEFAULT)
        .ok_or_else(|| StepError::Nothing("could not build the plane of the axis".into()))?;

    // The straight edges, classified.
    let mut edges = Vec::new();
    for l in core::iter::once(&profile.outer).chain(&profile.holes) {
        for e in &l.edges {
            if !matches!(e.curve, Curve2::Line) {
                return Err(StepError::Unsupported(format!(
                    "edge '{}' is a curve: revolving an arc makes a sphere or a torus, which the kernel cannot hold yet (only lines parallel or square to the axis can be revolved)",
                    e.id
                )));
            }
            let ((z0, r0), (z1, r1)) = (zr(e.from), zr(e.to));
            let parallel = (r1 - r0).abs() <= lin && (z1 - z0).abs() > lin;
            let square = (z1 - z0).abs() <= lin && (r1 - r0).abs() > lin;
            if !parallel && !square {
                return Err(StepError::Unsupported(format!(
                    "edge '{}' is slanted to the axis: revolving it makes a cone, which the kernel cannot hold yet (only lines parallel or square to the axis can be revolved)",
                    e.id
                )));
            }
            edges.push(Edge {
                id: e.id.clone(),
                z: [z0, z1],
                r: [r0, r1],
            });
        }
    }
    let (rmin, rmax) = edges
        .iter()
        .flat_map(|e| e.r)
        .fold((f64::INFINITY, f64::NEG_INFINITY), |(a, b), r| {
            (a.min(r), b.max(r))
        });
    if rmin < -lin && rmax > lin {
        return Err(StepError::Nothing(
            "the profile crosses the axis: draw it on one side of the axis only".into(),
        ));
    }

    // The distinct heights along the axis.
    let mut zs: Vec<f64> = edges.iter().flat_map(|e| e.z).collect();
    zs.sort_by(f64::total_cmp);
    let mut levels_z: Vec<f64> = Vec::new();
    for z in zs {
        if levels_z.last().is_none_or(|&l| z - l > lin) {
            levels_z.push(z);
        }
    }
    if levels_z.len() < 2 {
        return Err(StepError::Nothing(
            "the profile has no extent along the axis".into(),
        ));
    }

    // Between two heights the material is a set of rings: where the line at the middle height
    // crosses the edges parallel to the axis, taken in pairs.
    let mut slabs: Vec<Vec<Profile>> = Vec::new();
    for w in levels_z.windows(2) {
        let mid = 0.5 * (w[0] + w[1]);
        let mut hits: Vec<(f64, &str)> = edges
            .iter()
            .filter(|e| (e.r[1] - e.r[0]).abs() <= lin)
            .filter(|e| e.z[0].min(e.z[1]) < mid && mid < e.z[0].max(e.z[1]))
            .map(|e| (e.r[0].abs(), e.id.as_str()))
            .collect();
        hits.sort_by(|a, b| a.0.total_cmp(&b.0).then(a.1.cmp(b.1)));
        if hits.is_empty() {
            return Err(StepError::Nothing(format!(
                "the profile has nothing between {:.4} and {:.4} along the axis: it is in several pieces",
                w[0], w[1]
            )));
        }
        if !hits.len().is_multiple_of(2) {
            return Err(StepError::Nothing(
                "the profile is not a closed outline (an odd number of edges cross the axis direction)".into(),
            ));
        }
        let mut rings = Vec::new();
        for pair in hits.chunks(2) {
            let ((ra, ida), (rb, idb)) = (pair[0], pair[1]);
            if rb - ra <= lin {
                return Err(StepError::Nothing(format!(
                    "the profile is zero thick between '{ida}' and '{idb}'"
                )));
            }
            let outer = circle(idb, rb);
            let holes = if ra > lin {
                vec![circle(ida, ra)]
            } else {
                Vec::new()
            };
            let area = outer.area - holes.iter().map(|h| h.area).sum::<f64>();
            rings.push(Profile {
                id: format!("profile.{idb}"),
                outer,
                holes,
                area,
            });
        }
        slabs.push(rings);
    }

    let last = levels_z.len() - 1;
    let levels = levels_z
        .iter()
        .enumerate()
        .map(|(i, &z)| {
            let (owner, role) = if i == 0 {
                (String::new(), Role::Start)
            } else if i == last {
                (String::new(), Role::End)
            } else {
                let edge = edges
                    .iter()
                    .find(|e| (e.z[0] - z).abs() <= lin && (e.z[1] - z).abs() <= lin)
                    .map_or("step", |e| e.id.as_str());
                (format!("{feature}.{edge}"), Role::Start)
            };
            Level { w: z, owner, role }
        })
        .collect();
    let mut st = Stepped {
        plane: model,
        levels,
        slabs,
        rims: Vec::new(),
    };
    st.simplify_public();
    Ok(st)
}

/// A revolve by less than a full turn, built directly as a B-rep (the layered model is made of
/// whole discs and rings). The profile must lie on one side of the axis and have only straight
/// edges parallel or square to it (cylinders and flat sectors); an edge on the axis itself makes
/// no face. The turn starts in the sketch plane and goes the way the right-hand rule says about
/// the axis direction. Faces are `cap_start` (the sketch plane), `cap_end` and `side:<edge id>`.
pub fn revolve_partial(
    sketch: &Plane,
    profile: &Profile,
    origin: Vec2,
    dir: Vec2,
    angle_degrees: f64,
) -> Result<Solid, StepError> {
    use dc_kernel::{CoEdge, Curve, Edge, Face, Id, Surface};
    use dc_math::{det, Vec3};
    use std::collections::BTreeMap;

    let tol = Tolerance::DEFAULT;
    let lin = tol.linear;
    if !angle_degrees.is_finite() || angle_degrees <= 0.0 || angle_degrees >= 360.0 {
        return Err(StepError::Nothing(format!(
            "a partial revolve turns by more than 0 and less than 360 degrees, got {angle_degrees}"
        )));
    }
    let theta = angle_degrees * core::f64::consts::PI / 180.0;
    let len = dir.length();
    if !len.is_finite() || len <= lin {
        return Err(StepError::Nothing(
            "the axis has no direction (its two ends coincide)".into(),
        ));
    }
    let n = sketch.normal();
    let a0 = sketch.to_3d(origin);
    let a = (sketch.to_3d(origin + dir / len) - a0)
        .normalized(tol)
        .ok_or_else(|| StepError::Nothing("the sketch plane is degenerate".into()))?;
    let across = n.cross(a).normalized(tol).expect("axis in the plane");
    // Which side of the axis the profile is on.
    let mut side = 0.0f64;
    let loops: Vec<Vec<(Vec3, Vec3, String)>> = core::iter::once((&profile.outer, false))
        .chain(profile.holes.iter().map(|h| (h, true)))
        .map(|(l, hole)| {
            let mut v: Vec<(Vec3, Vec3, String)> = l
                .edges
                .iter()
                .map(|e| {
                    (
                        sketch.to_3d(Vec2::new(e.from[0], e.from[1])),
                        sketch.to_3d(Vec2::new(e.to[0], e.to[1])),
                        e.id.clone(),
                    )
                })
                .collect();
            if hole {
                // Holes are stored counter-clockwise; the face boundary runs the other way.
                v.reverse();
                for e in &mut v {
                    *e = (e.1, e.0, e.2.clone());
                }
            }
            v
        })
        .collect();
    for lp in &loops {
        for (p, q, _) in lp {
            for x in [p, q] {
                let s = (*x - a0).dot(across);
                if s.abs() > lin && side == 0.0 {
                    side = s.signum();
                } else if s.abs() > lin && s.signum() != side {
                    return Err(StepError::Nothing(
                        "the profile crosses the axis: draw it on one side of the axis only".into(),
                    ));
                }
            }
        }
    }
    if side == 0.0 {
        return Err(StepError::Nothing(
            "the profile lies on the axis: there is nothing to revolve".into(),
        ));
    }
    let u = across * side;
    let w = a.cross(u);
    // (height, radius) of a point.
    let hr = |p: Vec3| -> (f64, f64) {
        let d = p - a0;
        let h = d.dot(a);
        ((h), (d - a * h).length())
    };
    let at = |h: f64, r: f64, phi: f64| a0 + a * h + (u * det::cos(phi) + w * det::sin(phi)) * r;

    let mut s = Solid::default();
    // Vertices per profile point: (angle 0, angle theta); on the axis both are one vertex.
    let mut vkey: BTreeMap<(u64, u64), (Id, Id)> = BTreeMap::new();
    let mut vertex = |s: &mut Solid, p: Vec3| -> (Id, Id) {
        let (h, r) = hr(p);
        let key = (h.to_bits(), r.to_bits());
        *vkey.entry(key).or_insert_with(|| {
            let v0 = s.vertices.len() as Id;
            s.vertices.push(at(h, r.max(0.0), 0.0));
            if r <= lin {
                (v0, v0)
            } else {
                s.vertices.push(at(h, r, theta));
                (v0, v0 + 1)
            }
        })
    };
    let mut line_edges: BTreeMap<(Id, Id), Id> = BTreeMap::new();
    let mut arc_edges: BTreeMap<(Id, Id), Id> = BTreeMap::new();
    // A directed use of the straight edge a -> b.
    let line = |s: &mut Solid, lines: &mut BTreeMap<(Id, Id), Id>, va: Id, vb: Id| -> CoEdge {
        let key = (va.min(vb), va.max(vb));
        let id = *lines.entry(key).or_insert_with(|| {
            s.edges.push(Edge {
                name: String::new(),
                v0: va,
                v1: vb,
                curve: Curve::Line,
            });
            (s.edges.len() - 1) as Id
        });
        CoEdge {
            edge: id,
            reversed: s.edges[id as usize].v0 != va,
        }
    };
    // The arc at (h, r) from angle 0 to theta, used forward (`up`) or backward.
    let mut arc = |s: &mut Solid, v0: Id, v1: Id, h: f64, r: f64, up: bool| -> CoEdge {
        let id = *arc_edges.entry((v0, v1)).or_insert_with(|| {
            s.edges.push(Edge {
                name: String::new(),
                v0,
                v1,
                curve: Curve::Circle {
                    center: a0 + a * h,
                    axis: a,
                    x_dir: u,
                    radius: r,
                    t0: 0.0,
                    sweep: theta,
                },
            });
            (s.edges.len() - 1) as Id
        });
        CoEdge {
            edge: id,
            reversed: !up,
        }
    };
    // Cap orientation: the sketch-plane cap must face against the direction of the turn.
    let s0 = n.dot(-w) > 0.0;
    let mut cap0: Vec<Vec<CoEdge>> = Vec::new();
    let mut cap1: Vec<Vec<CoEdge>> = Vec::new();
    let mut walls: Vec<(String, Surface, bool, Vec<CoEdge>)> = Vec::new();
    for lp in &loops {
        let mut c0: Vec<CoEdge> = Vec::new();
        let mut c1: Vec<CoEdge> = Vec::new();
        for (p, q, id) in lp {
            let ((hp, rp), (hq, rq)) = (hr(*p), hr(*q));
            let (vp, vq) = (vertex(&mut s, *p), vertex(&mut s, *q));
            let t = (*q - *p)
                .normalized(tol)
                .ok_or_else(|| StepError::Nothing(format!("edge '{id}' has no length")))?;
            // The profile's own edges at both ends of the turn.
            c0.push(line(&mut s, &mut line_edges, vp.0, vq.0));
            c1.push(line(&mut s, &mut line_edges, vp.1, vq.1));
            if rp <= lin && rq <= lin {
                continue; // on the axis: no surface
            }
            let parallel = (rq - rp).abs() <= lin;
            let square = (hq - hp).abs() <= lin;
            if !parallel && !square {
                return Err(StepError::Unsupported(format!(
                    "edge '{id}' is slanted to the axis: revolving it makes a cone, which the kernel cannot hold yet (only lines parallel or square to the axis can be revolved)"
                )));
            }
            // The wall's boundary: P0 -> Q0, arc at Q, Qθ -> Pθ, arc back at P.
            let mut steps: Vec<CoEdge> = vec![line(&mut s, &mut line_edges, vp.0, vq.0)];
            if rq > lin {
                steps.push(arc(&mut s, vq.0, vq.1, hq, rq, true));
            }
            steps.push(line(&mut s, &mut line_edges, vq.1, vp.1));
            if rp > lin {
                steps.push(arc(&mut s, vp.0, vp.1, hp, rp, false));
            }
            let boundary = if s0 {
                // Opposite to the cap: reverse the whole cycle.
                steps
                    .into_iter()
                    .rev()
                    .map(|c| CoEdge {
                        edge: c.edge,
                        reversed: !c.reversed,
                    })
                    .collect()
            } else {
                steps
            };
            let inward = n.cross(t); // into the material, in the profile plane
            let (surface, same_sense) = if parallel {
                (
                    Surface::Cylinder {
                        origin: a0,
                        axis: a,
                        ref_dir: u,
                        radius: rp,
                        phi0: 0.0,
                        phi1: theta,
                        h0: hp.min(hq),
                        h1: hp.max(hq),
                    },
                    inward.dot(u) <= 0.0,
                )
            } else {
                let up = inward.dot(a) > 0.0;
                (
                    Surface::Plane {
                        origin: a0 + a * hp,
                        normal: if up { -a } else { a },
                        x_dir: u,
                    },
                    true,
                )
            };
            walls.push((format!("side:{id}"), surface, same_sense, boundary));
        }
        let reversed = |v: Vec<CoEdge>| -> Vec<CoEdge> {
            v.into_iter()
                .rev()
                .map(|c| CoEdge {
                    edge: c.edge,
                    reversed: !c.reversed,
                })
                .collect()
        };
        if s0 {
            cap0.push(c0);
            cap1.push(reversed(c1));
        } else {
            cap0.push(reversed(c0));
            cap1.push(c1);
        }
    }
    let w_end = w * det::cos(theta) - u * det::sin(theta);
    s.faces.push(Face {
        name: "cap_start".into(),
        surface: Surface::Plane {
            origin: a0,
            normal: -w,
            x_dir: u,
        },
        same_sense: true,
        loops: cap0,
    });
    s.faces.push(Face {
        name: "cap_end".into(),
        surface: Surface::Plane {
            origin: a0,
            normal: w_end,
            x_dir: u * det::cos(theta) + w * det::sin(theta),
        },
        same_sense: true,
        loops: cap1,
    });
    for (name, surface, same_sense, boundary) in walls {
        s.faces.push(Face {
            name,
            surface,
            same_sense,
            loops: vec![boundary],
        });
    }
    let bases: Vec<String> = s.faces.iter().map(|f| f.name.clone()).collect();
    let (faces, edges) = dc_kernel::naming::structural_names(&s, &bases);
    for (f, n) in s.faces.iter_mut().zip(faces) {
        f.name = n;
    }
    for (e, n) in s.edges.iter_mut().zip(edges) {
        e.name = n;
    }
    let issues = s.validate();
    if issues.is_empty() {
        Ok(s)
    } else {
        Err(StepError::Invalid(
            issues.into_iter().map(|i| i.message).collect(),
        ))
    }
}

/// A full turn of `profile` about an axis, built directly as a B-rep: a cylinder (with a seam) for
/// every edge parallel to the axis, a flat ring or disc for every edge square to it, nothing for
/// an edge on the axis. Edges slanted to the axis (cones) and curves (spheres, tori) are refused.
/// The two end faces are `cap_start` (facing against the axis direction) and `cap_end`; a step
/// between is `cap_start:<feature>.<edge>`; walls are `side:<edge id>`.
pub fn revolve_full(
    sketch: &Plane,
    profile: &Profile,
    origin: Vec2,
    dir: Vec2,
    feature: &str,
) -> Result<Solid, StepError> {
    use dc_kernel::{CoEdge, Curve, Edge, Face, Id, Surface};
    use dc_math::{det, Vec3};
    use std::collections::BTreeMap;

    let tol = Tolerance::DEFAULT;
    let lin = tol.linear;
    let two_pi = 2.0 * core::f64::consts::PI;
    let len = dir.length();
    if !len.is_finite() || len <= lin {
        return Err(StepError::Nothing(
            "the axis has no direction (its two ends coincide)".into(),
        ));
    }
    let n = sketch.normal();
    let a0 = sketch.to_3d(origin);
    let a = (sketch.to_3d(origin + dir / len) - a0)
        .normalized(tol)
        .ok_or_else(|| StepError::Nothing("the sketch plane is degenerate".into()))?;
    let across = n.cross(a).normalized(tol).expect("axis in the plane");
    let loops: Vec<Vec<(Vec3, Vec3, String)>> = core::iter::once((&profile.outer, false))
        .chain(profile.holes.iter().map(|h| (h, true)))
        .map(|(l, hole)| {
            let mut v: Vec<(Vec3, Vec3, String)> = l
                .edges
                .iter()
                .map(|e| {
                    (
                        sketch.to_3d(Vec2::new(e.from[0], e.from[1])),
                        sketch.to_3d(Vec2::new(e.to[0], e.to[1])),
                        e.id.clone(),
                    )
                })
                .collect();
            if hole {
                v.reverse();
                for e in &mut v {
                    *e = (e.1, e.0, e.2.clone());
                }
            }
            v
        })
        .collect();
    let mut side = 0.0f64;
    for (p, q, id) in loops.iter().flatten() {
        for x in [p, q] {
            let s = (*x - a0).dot(across);
            if s.abs() > lin && side == 0.0 {
                side = s.signum();
            } else if s.abs() > lin && s.signum() != side {
                return Err(StepError::Nothing(
                    "the profile crosses the axis: draw it on one side of the axis only".into(),
                ));
            }
        }
        let _ = id;
    }
    if side == 0.0 {
        return Err(StepError::Nothing(
            "the profile lies on the axis: there is nothing to revolve".into(),
        ));
    }
    let u = across * side;
    let hr = |p: Vec3| -> (f64, f64) {
        let d = p - a0;
        let h = d.dot(a);
        (h, (d - a * h).length())
    };
    let (mut hmin, mut hmax) = (f64::MAX, f64::MIN);
    for (p, q, _) in loops.iter().flatten() {
        for x in [p, q] {
            let (h, _) = hr(*x);
            hmin = hmin.min(h);
            hmax = hmax.max(h);
        }
    }
    let mut s = Solid::default();
    let mut vkey: BTreeMap<(u64, u64), Id> = BTreeMap::new();
    let mut vertex = |s: &mut Solid, p: Vec3| -> Id {
        let (h, r) = hr(p);
        let r = if r <= lin { 0.0 } else { r };
        *vkey.entry((h.to_bits(), r.to_bits())).or_insert_with(|| {
            s.vertices.push(a0 + a * h + u * r);
            (s.vertices.len() - 1) as Id
        })
    };
    let mut circles: BTreeMap<(u64, u64), Id> = BTreeMap::new();
    let mut seams: BTreeMap<(Id, Id), Id> = BTreeMap::new();
    let mut walls: Vec<(String, Surface, bool, Vec<CoEdge>)> = Vec::new();
    let mut rings: Vec<(String, f64, f64, f64, f64, bool)> = Vec::new(); // id, h, r_hi, r_lo, ..., faces up
    let mut ring_faces: Vec<(String, Surface, Vec<Vec<CoEdge>>)> = Vec::new();
    let _ = &mut rings;
    for (p, q, id) in loops.iter().flatten() {
        let ((hp, rp), (hq, rq)) = (hr(*p), hr(*q));
        if rp <= lin && rq <= lin {
            continue;
        }
        let t = (*q - *p)
            .normalized(tol)
            .ok_or_else(|| StepError::Nothing(format!("edge '{id}' has no length")))?;
        let inward = n.cross(t);
        let parallel = (rq - rp).abs() <= lin;
        let square = (hq - hp).abs() <= lin;
        if !parallel && !square {
            return Err(StepError::Unsupported(format!(
                "edge '{id}' is slanted to the axis: revolving it makes a cone, which the kernel cannot hold yet (only lines parallel or square to the axis can be revolved)"
            )));
        }
        // A closed circle at height `h` and radius `r`.
        let mut circle = |s: &mut Solid, vkey_v: Id, h: f64, r: f64| -> Id {
            *circles
                .entry((h.to_bits(), r.to_bits()))
                .or_insert_with(|| {
                    s.edges.push(Edge {
                        name: String::new(),
                        v0: vkey_v,
                        v1: vkey_v,
                        curve: Curve::Circle {
                            center: a0 + a * h,
                            axis: a,
                            x_dir: u,
                            radius: r,
                            t0: 0.0,
                            sweep: two_pi,
                        },
                    });
                    (s.edges.len() - 1) as Id
                })
        };
        if parallel {
            let (lo, hi) = if hp <= hq { (*p, *q) } else { (*q, *p) };
            let ((hl, rl), (hh, rh)) = (hr(lo), hr(hi));
            let (vl, vh) = (vertex(&mut s, lo), vertex(&mut s, hi));
            let (cl, ch) = (circle(&mut s, vl, hl, rl), circle(&mut s, vh, hh, rh));
            let seam = *seams.entry((vl, vh)).or_insert_with(|| {
                s.edges.push(Edge {
                    name: String::new(),
                    v0: vl,
                    v1: vh,
                    curve: Curve::Line,
                });
                (s.edges.len() - 1) as Id
            });
            let same_sense = inward.dot(u) <= 0.0;
            let c = |edge: Id, reversed: bool| CoEdge { edge, reversed };
            let boundary = if same_sense {
                vec![c(cl, false), c(seam, false), c(ch, true), c(seam, true)]
            } else {
                vec![c(cl, true), c(seam, false), c(ch, false), c(seam, true)]
            };
            walls.push((
                format!("side:{id}"),
                Surface::Cylinder {
                    origin: a0,
                    axis: a,
                    ref_dir: u,
                    radius: rl,
                    phi0: 0.0,
                    phi1: two_pi,
                    h0: hl,
                    h1: hh,
                },
                same_sense,
                boundary,
            ));
        } else {
            let up = inward.dot(a) > 0.0; // material above: the face looks down
            let normal = if up { -a } else { a };
            let ((r_hi, p_hi), (r_lo, p_lo)) = if rp >= rq {
                ((rp, *p), (rq, *q))
            } else {
                ((rq, *q), (rp, *p))
            };
            let mut ring_loops: Vec<Vec<CoEdge>> = Vec::new();
            let vh = vertex(&mut s, p_hi);
            let ch = circle(&mut s, vh, hp, r_hi);
            let outer_fwd = normal.dot(a) > 0.0;
            ring_loops.push(vec![CoEdge {
                edge: ch,
                reversed: !outer_fwd,
            }]);
            if r_lo > lin {
                let vl = vertex(&mut s, p_lo);
                let cl = circle(&mut s, vl, hp, r_lo);
                ring_loops.push(vec![CoEdge {
                    edge: cl,
                    reversed: outer_fwd,
                }]);
            }
            let name = if up && (hp - hmin).abs() <= lin {
                "cap_start".to_owned()
            } else if !up && (hp - hmax).abs() <= lin {
                "cap_end".to_owned()
            } else {
                format!("cap_start:{feature}.{id}")
            };
            ring_faces.push((
                name,
                Surface::Plane {
                    origin: a0 + a * hp,
                    normal,
                    x_dir: u,
                },
                ring_loops,
            ));
        }
    }
    for (name, surface, same_sense, boundary) in walls {
        s.faces.push(Face {
            name,
            surface,
            same_sense,
            loops: vec![boundary],
        });
    }
    for (name, surface, loops) in ring_faces {
        s.faces.push(Face {
            name,
            surface,
            same_sense: true,
            loops,
        });
    }
    if s.faces.is_empty() {
        return Err(StepError::Nothing(
            "the profile has no extent to revolve".into(),
        ));
    }
    let bases: Vec<String> = s.faces.iter().map(|f| f.name.clone()).collect();
    let (faces, edges) = dc_kernel::naming::structural_names(&s, &bases);
    for (f, n) in s.faces.iter_mut().zip(faces) {
        f.name = n;
    }
    for (e, n) in s.edges.iter_mut().zip(edges) {
        e.name = n;
    }
    let _ = det::cos;
    let issues = s.validate();
    if issues.is_empty() {
        Ok(s)
    } else {
        Err(StepError::Invalid(
            issues.into_iter().map(|i| i.message).collect(),
        ))
    }
}
