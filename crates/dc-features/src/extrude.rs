//! Extrude a sketch profile into a solid.
//!
//! The profile's outer loop and holes become the cross-section; the result has two planar caps
//! and one side face per profile edge (planar for lines, cylindrical for arcs and circles).
//! Faces and edges are named from the sketch entities they came from, so references to them
//! survive rebuilds: `cap_start`, `cap_end`, `side:<entity>`, `base:<entity>`, `top:<entity>`,
//! `rise:<entity>@from`.

use dc_kernel::{CoEdge, Curve, Edge, Face, Id, Issue, Solid, Surface};
use dc_math::{Plane, Vec2, Vec3};
use dc_sketch::{Curve2, Loop, Profile};
use std::fmt;

const TWO_PI: f64 = 2.0 * core::f64::consts::PI;

#[derive(Debug, Clone, PartialEq)]
pub enum ExtrudeError {
    InvalidDepth(f64),
    /// The construction produced an invalid solid. This is a bug, reported with details instead
    /// of being passed on.
    InvalidResult(Vec<String>),
}

impl fmt::Display for ExtrudeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ExtrudeError::InvalidDepth(d) => write!(
                f,
                "extrude depth must be finite and greater than zero, got {d}"
            ),
            ExtrudeError::InvalidResult(v) => {
                write!(f, "extrude produced an invalid solid: {}", v.join("; "))
            }
        }
    }
}

impl std::error::Error for ExtrudeError {}

struct LoopIds {
    base: Vec<Id>,
    top: Vec<Id>,
    rise: Vec<Id>,
}

fn v2(p: [f64; 2]) -> Vec2 {
    Vec2::new(p[0], p[1])
}

/// Reverses a loop: opposite order, every coedge flipped.
fn reversed(seq: Vec<(Id, bool)>) -> Vec<(Id, bool)> {
    seq.into_iter().rev().map(|(e, r)| (e, !r)).collect()
}

fn coedges(seq: Vec<(Id, bool)>) -> Vec<CoEdge> {
    seq.into_iter()
        .map(|(edge, reversed)| CoEdge { edge, reversed })
        .collect()
}

/// Extrudes `profile` (placed on `plane`) by `depth` along the plane normal, or against it if
/// `reverse`. The result is validated before it is returned.
pub fn extrude(
    plane: &Plane,
    profile: &Profile,
    depth: f64,
    reverse: bool,
) -> Result<Solid, ExtrudeError> {
    if !depth.is_finite() || depth <= 0.0 {
        return Err(ExtrudeError::InvalidDepth(depth));
    }
    let n = plane.normal();
    let sgn = if reverse { -1.0 } else { 1.0 };
    let ext = n * (sgn * depth);
    let mut s = Solid::default();

    let loops: Vec<(&Loop, bool)> = std::iter::once((&profile.outer, true))
        .chain(profile.holes.iter().map(|h| (h, false)))
        .collect();

    // Vertices and edges for every loop.
    let mut ids: Vec<LoopIds> = Vec::new();
    for (lp, _) in &loops {
        let k = lp.edges.len();
        let mut base_v = Vec::new();
        let mut top_v = Vec::new();
        for e in &lp.edges {
            let b = plane.to_3d(v2(e.from));
            base_v.push(s.vertices.len() as Id);
            s.vertices.push(b);
            top_v.push(s.vertices.len() as Id);
            s.vertices.push(b + ext);
        }
        let mut li = LoopIds {
            base: Vec::new(),
            top: Vec::new(),
            rise: Vec::new(),
        };
        for (j, e) in lp.edges.iter().enumerate() {
            li.rise.push(s.edges.len() as Id);
            s.edges.push(Edge {
                name: format!("rise:{}@from", e.id),
                v0: base_v[j],
                v1: top_v[j],
                curve: Curve::Line,
            });
        }
        for (i, e) in lp.edges.iter().enumerate() {
            let next = (i + 1) % k;
            for (at_top, list, name) in [(false, &mut li.base, "base"), (true, &mut li.top, "top")]
            {
                let (v0, v1) = if at_top {
                    (top_v[i], top_v[next])
                } else {
                    (base_v[i], base_v[next])
                };
                let shift = if at_top { ext } else { Vec3::ZERO };
                let curve = match &e.curve {
                    Curve2::Line => Curve::Line,
                    Curve2::Arc {
                        center,
                        radius,
                        start_angle,
                        sweep,
                    } => {
                        let c = plane.to_3d(v2(*center)) + shift;
                        if e.reversed {
                            // Traversed clockwise about +n = counter-clockwise about −n.
                            Curve::Circle {
                                center: c,
                                axis: -n,
                                x_dir: plane.x_axis,
                                radius: *radius,
                                t0: -(start_angle + sweep),
                                sweep: *sweep,
                            }
                        } else {
                            Curve::Circle {
                                center: c,
                                axis: n,
                                x_dir: plane.x_axis,
                                radius: *radius,
                                t0: *start_angle,
                                sweep: *sweep,
                            }
                        }
                    }
                    Curve2::Circle { center, radius } => Curve::Circle {
                        center: plane.to_3d(v2(*center)) + shift,
                        axis: n,
                        x_dir: plane.x_axis,
                        radius: *radius,
                        t0: 0.0,
                        sweep: TWO_PI,
                    },
                };
                list.push(s.edges.len() as Id);
                s.edges.push(Edge {
                    name: format!("{name}:{}", e.id),
                    v0,
                    v1,
                    curve,
                });
            }
        }
        ids.push(li);
    }

    // Caps. Outward normal of the base cap is against the extrusion direction.
    for (name, at_top) in [("cap_start", false), ("cap_end", true)] {
        let outward_plus_n = if at_top { sgn > 0.0 } else { sgn < 0.0 };
        let mut face_loops = Vec::new();
        for (li, (lp, outer)) in loops.iter().enumerate() {
            let list = if at_top { &ids[li].top } else { &ids[li].base };
            let mut seq: Vec<(Id, bool)> = (0..lp.edges.len()).map(|i| (list[i], false)).collect();
            if *outer != outward_plus_n {
                seq = reversed(seq);
            }
            face_loops.push(coedges(seq));
        }
        s.faces.push(Face {
            name: name.to_owned(),
            surface: Surface::Plane {
                origin: if at_top {
                    plane.origin + ext
                } else {
                    plane.origin
                },
                normal: n,
                x_dir: plane.x_axis,
            },
            same_sense: outward_plus_n,
            loops: face_loops,
        });
    }

    // Side faces, one per profile edge.
    for (li, (lp, outer)) in loops.iter().enumerate() {
        let k = lp.edges.len();
        let LoopIds { base, top, rise } = &ids[li];
        for (i, e) in lp.edges.iter().enumerate() {
            let next = (i + 1) % k;
            match &e.curve {
                Curve2::Line => {
                    let d = v2(e.to) - v2(e.from);
                    let tdir = (plane.x_axis * d.x + plane.y_axis * d.y) / d.length();
                    let normal = if *outer { tdir.cross(n) } else { n.cross(tdir) };
                    let ext_along_y = (sgn > 0.0) == *outer;
                    let seq = if ext_along_y {
                        vec![
                            (base[i], false),
                            (rise[next], false),
                            (top[i], true),
                            (rise[i], true),
                        ]
                    } else {
                        vec![
                            (top[i], false),
                            (rise[next], true),
                            (base[i], true),
                            (rise[i], false),
                        ]
                    };
                    s.faces.push(Face {
                        name: format!("side:{}", e.id),
                        surface: Surface::Plane {
                            origin: plane.to_3d(v2(e.from)),
                            normal,
                            x_dir: tdir,
                        },
                        same_sense: true,
                        loops: vec![coedges(seq)],
                    });
                }
                Curve2::Arc {
                    center,
                    radius,
                    start_angle,
                    sweep,
                } => {
                    s.faces.push(cylinder_face(
                        plane,
                        e.id.as_str(),
                        v2(*center),
                        *radius,
                        *start_angle,
                        *start_angle + *sweep,
                        depth * sgn,
                        sgn,
                        *outer,
                        e.reversed,
                        i,
                        next,
                        base,
                        top,
                        rise,
                    ));
                }
                Curve2::Circle { center, radius } => {
                    s.faces.push(cylinder_face(
                        plane,
                        e.id.as_str(),
                        v2(*center),
                        *radius,
                        0.0,
                        TWO_PI,
                        depth * sgn,
                        sgn,
                        *outer,
                        false,
                        i,
                        next,
                        base,
                        top,
                        rise,
                    ));
                }
            }
        }
    }

    let issues: Vec<Issue> = s.validate();
    if issues.is_empty() {
        Ok(s)
    } else {
        Err(ExtrudeError::InvalidResult(
            issues.into_iter().map(|i| i.message).collect(),
        ))
    }
}

#[allow(clippy::too_many_arguments)]
fn cylinder_face(
    plane: &Plane,
    id: &str,
    center: Vec2,
    radius: f64,
    phi0: f64,
    phi1: f64,
    signed_depth: f64,
    sgn: f64,
    outer: bool,
    nat_rev: bool,
    i: usize,
    next: usize,
    base: &[Id],
    top: &[Id],
    rise: &[Id],
) -> Face {
    // Natural direction runs counter-clockwise about the axis. If the loop runs against it, the
    // natural start is the loop's `to` corner.
    let (nat_start, nat_end) = if nat_rev { (next, i) } else { (i, next) };
    let (low, high) = if sgn > 0.0 {
        (base[i], top[i])
    } else {
        (top[i], base[i])
    };
    let up_rev = sgn < 0.0;
    let seq = vec![
        (low, nat_rev),
        (rise[nat_end], up_rev),
        (high, !nat_rev),
        (rise[nat_start], !up_rev),
    ];
    // Radially outward is the surface normal; the solid is on the inside of the circle for
    // convex outer arcs and for hole boundaries traversed against their natural direction.
    let same_sense = nat_rev != outer;
    let seq = if same_sense { seq } else { reversed(seq) };
    Face {
        name: format!("side:{id}"),
        surface: Surface::Cylinder {
            origin: plane.to_3d(center),
            axis: plane.normal(),
            ref_dir: plane.x_axis,
            radius,
            phi0,
            phi1,
            h0: signed_depth.min(0.0),
            h1: signed_depth.max(0.0),
        },
        same_sense,
        loops: vec![coedges(seq)],
    }
}
