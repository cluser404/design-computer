//! Shelling a prism-shaped part: hollowing it out with a wall of uniform thickness, optionally
//! leaving one or both end faces open.
//!
//! The part must be a prism: two parallel flat end faces and, between them, only flat faces
//! square to the ends and cylinders along the axis (a plate, a tube, a rounded-corner block, an
//! extruded profile with holes). The cavity is the end face's outline moved inwards by the
//! thickness (lines move sideways, circle arcs change radius, corners are the intersections of
//! the moved neighbours), extruded between the ends (kept `thickness` away from a closed end,
//! going out through an open one), and taken out of the part by the boolean. Parts with steps,
//! side holes or rounded edges across the axis are refused with the reason.

use crate::extrude::extrude;
use dc_kernel::boolean::geo::{arc_arc, Arc, Geo};
use dc_kernel::boolean::{boolean, BoolError, BoolOp, BoolOptions};
use dc_kernel::{Solid, Surface};
use dc_math::{det, Plane, Tolerance, Vec2, Vec3};
use dc_sketch::{Curve2, Loop, LoopEdge, Profile};
use std::sync::Arc as Rc;

#[derive(Clone, Debug, PartialEq)]
pub enum ShellError {
    NotFound(String),
    Unsupported(String),
    TooThick(String),
    Bool(BoolError),
}

impl core::fmt::Display for ShellError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            ShellError::NotFound(s) | ShellError::Unsupported(s) | ShellError::TooThick(s) => {
                write!(f, "{s}")
            }
            ShellError::Bool(e) => write!(f, "{e}"),
        }
    }
}

impl std::error::Error for ShellError {}

fn plane_normal(f: &dc_kernel::Face) -> Option<(Vec3, Vec3)> {
    match &f.surface {
        Surface::Plane { origin, normal, .. } => {
            Some((*origin, if f.same_sense { *normal } else { -*normal }))
        }
        _ => None,
    }
}

/// The directed arcs of one loop of a face.
fn loop_arcs(s: &Solid, lp: &[dc_kernel::CoEdge]) -> Option<Vec<Arc>> {
    let mut out = Vec::new();
    for &c in lp {
        let e = &s.edges[c.edge as usize];
        let arc = match &e.curve {
            dc_kernel::Curve::Line => Arc::line(s.vertex(e.v0), s.vertex(e.v1))?,
            dc_kernel::Curve::Circle {
                center,
                axis,
                x_dir,
                radius,
                t0,
                sweep,
            } => Arc {
                geo: Geo::Circle {
                    c: *center,
                    n: *axis,
                    u: *x_dir,
                    r: *radius,
                },
                t0: *t0,
                t1: t0 + sweep,
            },
            dc_kernel::Curve::Ellipse { .. } => return None,
        };
        out.push(if c.reversed { arc.reversed() } else { arc });
    }
    Some(out)
}

/// The loop moved by `t` to its left (into the face), with sharp corners.
fn offset_loop(lp: &[Arc], n: Vec3, t: f64) -> Result<Vec<Arc>, ShellError> {
    let tol = Tolerance::DEFAULT;
    let far = 1.0e6;
    // The moved curves, extended.
    let moved: Vec<Arc> = lp
        .iter()
        .map(|a| match &a.geo {
            Geo::Line { o, d } => {
                // Left of the way the arc is travelled (its parameter may run backwards).
                let travel = *d * (a.t1 - a.t0).signum();
                let left = n.cross(travel) * t;
                Ok(Arc {
                    geo: Geo::Line {
                        o: *o + left,
                        d: *d,
                    },
                    t0: a.t0,
                    t1: a.t1,
                })
            }
            Geo::Circle { c, n: cn, u, r } => {
                // Counter-clockwise about `n` as travelled?
                let ccw = (cn.dot(n) > 0.0) == (a.t1 > a.t0);
                let r2 = if ccw { r - t } else { r + t };
                if r2 <= tol.linear {
                    return Err(ShellError::TooThick(format!(
                        "the wall is as thick as an arc of radius {r} is wide"
                    )));
                }
                Ok(Arc {
                    geo: Geo::Circle {
                        c: *c,
                        n: *cn,
                        u: *u,
                        r: r2,
                    },
                    t0: a.t0,
                    t1: a.t1,
                })
            }
            Geo::Ellipse { .. } => Err(ShellError::Unsupported("an elliptical edge".into())),
        })
        .collect::<Result<_, _>>()?;
    let full = |a: &Arc| -> Arc {
        match &a.geo {
            Geo::Line { .. } => Arc {
                geo: a.geo.clone(),
                t0: -far,
                t1: far,
            },
            g => Arc {
                geo: g.clone(),
                t0: 0.0,
                t1: 2.0 * core::f64::consts::PI,
            },
        }
    };
    // Corner i joins moved arc i-1 and i, near the original vertex.
    let k = lp.len();
    if k == 1 {
        // A lone circle (or a closed single edge): just the moved curve.
        return Ok(moved);
    }
    let mut corners: Vec<Vec3> = Vec::with_capacity(k);
    for i in 0..k {
        let (a, b) = (&moved[(i + k - 1) % k], &moved[i]);
        let v = lp[i].start();
        let mut best: Option<(f64, Vec3)> = None;
        for (ta, tb) in arc_arc(&full(a), &full(b)) {
            let (pa, pb) = (full(a).point(ta), full(b).point(tb));
            let p = (pa + pb) * 0.5;
            let d = (p - v).length();
            if best.is_none_or(|(bd, _)| d < bd) {
                best = Some((d, p));
            }
        }
        let p = match best {
            Some((d, p)) if d < 20.0 * t.abs() + 1.0 => p,
            _ => {
                // Collinear neighbours: the corner just moves sideways with the line.
                let (d1, d2) = (a.geo.tangent(a.t0), b.geo.tangent(b.t0));
                if matches!((&a.geo, &b.geo), (Geo::Line { .. }, Geo::Line { .. }))
                    && d1.cross(d2).length() < 1e-9
                {
                    v + n.cross(d1.normalized(tol).expect("line")) * t
                } else {
                    return Err(ShellError::Unsupported(
                        "a corner of the outline cannot be moved inwards with a sharp corner (it would need a round)".into(),
                    ));
                }
            }
        };
        corners.push(p);
    }
    // Trim every moved arc between its two corners, keeping its direction of travel.
    let mut out = Vec::with_capacity(k);
    for i in 0..k {
        let (p, q) = (corners[i], corners[(i + 1) % k]);
        let a = &moved[i];
        let (ta, tb) = (a.geo.param_of(p), a.geo.param_of(q));
        let arc = match &a.geo {
            Geo::Line { .. } => {
                let fwd = a.t1 > a.t0;
                if (tb > ta) != fwd || (tb - ta).abs() <= tol.linear {
                    return Err(ShellError::TooThick(
                        "the wall is too thick for the part: an edge of the outline vanishes"
                            .into(),
                    ));
                }
                Arc {
                    geo: a.geo.clone(),
                    t0: ta,
                    t1: tb,
                }
            }
            g => {
                let per = 2.0 * core::f64::consts::PI;
                let fwd = a.t1 > a.t0;
                let end = if fwd {
                    tb + per * ((ta - tb) / per).ceil()
                } else {
                    tb - per * ((tb - ta) / per).ceil()
                };
                let end = if fwd && end <= ta {
                    end + per
                } else if !fwd && end >= ta {
                    end - per
                } else {
                    end
                };
                Arc {
                    geo: g.clone(),
                    t0: ta,
                    t1: end,
                }
            }
        };
        out.push(arc);
    }
    Ok(out)
}

/// A sketch-style profile in `plane` from loops of 3D arcs lying in it (outer loop first).
fn profile_of(plane: &Plane, loops: &[Vec<Arc>]) -> Profile {
    let to2 = |p: Vec3| -> [f64; 2] {
        let q = plane.to_2d(p);
        [q.x, q.y]
    };
    let mut made: Vec<Loop> = Vec::new();
    for (li, lp) in loops.iter().enumerate() {
        let mut edges: Vec<LoopEdge> = Vec::new();
        let mut poly: Vec<Vec2> = Vec::new();
        for (k, a) in lp.iter().enumerate() {
            let (from, to) = (to2(a.start()), to2(a.end()));
            let curve_edge = |curve: Curve2, reversed: bool| LoopEdge {
                id: format!("l{li}e{k}"),
                reversed,
                from,
                to,
                curve,
            };
            for p in a.polyline() {
                let q = plane.to_2d(p);
                poly.push(q);
            }
            poly.pop();
            let e = match &a.geo {
                Geo::Line { .. } => curve_edge(Curve2::Line, false),
                Geo::Circle { c, n, r, .. } => {
                    // Counter-clockwise in the plane frame as travelled?
                    let ccw_world = (n.dot(plane.normal()) > 0.0) == (a.t1 > a.t0);
                    let c2 = to2(*c);
                    let ang = |p: [f64; 2]| det::atan2(p[1] - c2[1], p[0] - c2[0]);
                    if a.is_full() {
                        curve_edge(
                            Curve2::Circle {
                                center: c2,
                                radius: *r,
                            },
                            !ccw_world,
                        )
                    } else {
                        let (start, end) = if ccw_world { (from, to) } else { (to, from) };
                        let tau = 2.0 * core::f64::consts::PI;
                        let (sa, ea) = (ang(start), ang(end));
                        let sweep = (ea - sa).rem_euclid(tau);
                        curve_edge(
                            Curve2::Arc {
                                center: c2,
                                radius: *r,
                                start_angle: sa,
                                sweep,
                            },
                            !ccw_world,
                        )
                    }
                }
                Geo::Ellipse { .. } => unreachable!("refused earlier"),
            };
            edges.push(e);
        }
        let mut area = 0.0;
        for k in 0..poly.len() {
            let (p, q) = (poly[k], poly[(k + 1) % poly.len()]);
            area += p.x * q.y - q.x * p.y;
        }
        let area = area / 2.0;
        // Loops are stored counter-clockwise in the profile's own frame.
        if area < 0.0 {
            edges.reverse();
            for e in &mut edges {
                core::mem::swap(&mut e.from, &mut e.to);
                e.reversed = !e.reversed;
            }
        }
        made.push(Loop {
            edges,
            area: area.abs(),
        });
    }
    let outer = made.remove(0);
    let area = outer.area - made.iter().map(|h| h.area).sum::<f64>();
    Profile {
        id: "profile.shell".into(),
        outer,
        holes: made,
        area,
    }
}

fn find_ends(target: &Solid) -> Option<(usize, usize, Vec3)> {
    // Find the two end faces: flat, parallel, facing away from each other, with only square flat
    // faces and axial cylinders between.
    let candidates: Vec<usize> = (0..target.faces.len())
        .filter(|&i| plane_normal(&target.faces[i]).is_some())
        .collect();
    'cand: for &i in &candidates {
        let (_, n) = plane_normal(&target.faces[i]).expect("flat");
        let Some(j) = candidates.iter().copied().find(|&j| {
            j != i && plane_normal(&target.faces[j]).is_some_and(|(_, m)| m.dot(n) < -1.0 + 1e-9)
        }) else {
            continue;
        };
        for (k, f) in target.faces.iter().enumerate() {
            if k == i || k == j {
                continue;
            }
            let ok = match &f.surface {
                Surface::Plane { .. } => {
                    let (_, m) = plane_normal(f).expect("flat");
                    m.dot(n).abs() < 1e-9
                }
                Surface::Cylinder { axis, .. } => axis.cross(n).length() < 1e-9,
                _ => false,
            };
            if !ok {
                continue 'cand;
            }
        }
        return Some((i, j, n));
    }
    None
}

/// The names of the two end faces of a prism-shaped part (the faces a shell can leave open), or
/// `None` if the part is not a prism.
pub fn shell_ends(target: &Solid) -> Option<(String, String)> {
    find_ends(target).map(|(i, j, _)| (target.faces[i].name.clone(), target.faces[j].name.clone()))
}

/// Hollows `target` (a prism) with walls `thickness` thick, leaving the end faces named in `open`
/// (none, one, or both of its two flat ends) open. `owner` names the new faces
/// (`side:shell:<owner>:<edge>`, `cap_start:<owner>`, `cap_end:<owner>`).
pub fn shell_prism(
    target: &Solid,
    open: &[String],
    thickness: f64,
    owner: &str,
) -> Result<Solid, ShellError> {
    let tol = Tolerance::DEFAULT;
    if !thickness.is_finite() || thickness <= tol.linear {
        return Err(ShellError::Unsupported(format!(
            "the thickness must be more than zero, got {thickness}"
        )));
    }
    let ends = find_ends(target);
    let Some((ia, ib, n_a)) = ends else {
        return Err(ShellError::Unsupported(
            "only prism-shaped parts can be shelled so far: two flat end faces with flat faces square to them and cylinders along the axis between (no steps, side holes or rounded edges across the axis)".into(),
        ));
    };
    let (fa, fb) = (&target.faces[ia], &target.faces[ib]);
    for name in open {
        if *name != fa.name && *name != fb.name {
            return Err(ShellError::Unsupported(format!(
                "'{name}' cannot be left open: only the two end faces '{}' and '{}' can",
                fa.name, fb.name
            )));
        }
    }
    let (oa, _) = plane_normal(fa).expect("flat");
    let (ob, _) = plane_normal(fb).expect("flat");
    // Axis coordinate from end A into the part (s = 0 at A).
    let s_dir = -n_a;
    let height = (ob - oa).dot(s_dir);
    let a_open = open.contains(&fa.name);
    let b_open = open.contains(&fb.name);
    let closed = usize::from(!a_open) + usize::from(!b_open);
    if height - thickness * closed as f64 <= tol.linear {
        return Err(ShellError::TooThick(format!(
            "the part is {height:.4} thick between its ends: a wall of {thickness} leaves no room inside"
        )));
    }
    // The end face's outline, moved inwards.
    let mut loops: Vec<Vec<Arc>> = Vec::new();
    for lp in &fa.loops {
        let arcs = loop_arcs(target, lp).ok_or_else(|| {
            ShellError::Unsupported("an edge of the end face is elliptical".into())
        })?;
        loops.push(offset_loop(&arcs, n_a, thickness)?);
    }
    let x_dir = match &fa.surface {
        Surface::Plane { x_dir, .. } => *x_dir,
        _ => unreachable!(),
    };
    let plane = Plane::new(oa, x_dir, s_dir.cross(x_dir), tol)
        .ok_or_else(|| ShellError::Unsupported("the end face has no usable frame".into()))?;
    let profile = profile_of(&plane, &loops);
    let lo = if a_open { -1.0 } else { thickness };
    let hi = if b_open {
        height + 1.0
    } else {
        height - thickness
    };
    let tool = extrude(&plane.offset(lo), &profile, hi - lo, false)
        .map_err(|e| ShellError::Unsupported(format!("the cavity could not be built: {e}")))?;
    let owner_s = owner.to_owned();
    let opts = BoolOptions {
        b_prefix: String::new(),
        b_namer: Some(Rc::new(move |name: &str| match name {
            "cap_start" | "cap_end" => format!("{name}:{owner_s}"),
            other => match other.strip_prefix("side:") {
                Some(e) => format!("side:shell:{owner_s}:{e}"),
                None => format!("shell:{owner_s}:{other}"),
            },
        })),
    };
    let r = boolean(target, &tool, BoolOp::Subtract, &opts).map_err(ShellError::Bool)?;
    Ok(r.solid)
}
