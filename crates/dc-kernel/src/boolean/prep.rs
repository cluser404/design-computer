//! Faces prepared for the boolean: planes and cylinders with their boundary as directed arcs, a
//! flat (x, y) picture of the surface for decisions, point location, and clipping of a curve to
//! the face.
//!
//! A plane's picture is its own frame. A cylinder's is `(x, y) = (σ r φ, h)`: the angle about the
//! axis (unwrapped) times the radius, and the height along the axis, with `σ = +1` if the face's
//! outward normal points away from the axis and `-1` otherwise, so that loops run
//! counter-clockwise in the picture. A cylinder face that winds all the way round has a seam in
//! its loops and its picture is the strip `x0 <= x <= x0 + 2π r`; a patch that does not wind
//! lives near its own loops.

use super::geo::{arc_arc, Arc, Geo};
use super::planar::{inside_even_odd, P2};
use super::{BoolError, Side};
use crate::solid::{Curve, Solid, Surface};
use dc_math::{det, Tolerance, Vec3};

pub const TOL: f64 = Tolerance::DEFAULT.linear;
const TAU: f64 = 2.0 * core::f64::consts::PI;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Loc {
    Inside,
    Boundary,
    Outside,
}

#[derive(Clone, Debug)]
pub enum Surf {
    Plane {
        origin: Vec3,
        /// Unit outward normal.
        normal: Vec3,
        x: Vec3,
        /// `normal × x`.
        y: Vec3,
    },
    Cyl {
        origin: Vec3,
        axis: Vec3,
        e1: Vec3,
        e2: Vec3,
        r: f64,
        /// +1 if the outward normal points away from the axis, -1 if towards it.
        out: f64,
        winding: bool,
        /// Left edge of the picture (winding faces) or its centre (others).
        x0: f64,
    },
}

pub struct BFace {
    pub side: Side,
    pub idx: usize,
    pub name: String,
    pub surf: Surf,
    /// Boundary loops as directed arcs: the outer loop first (counter-clockwise seen from outside).
    pub loops: Vec<Vec<Arc>>,
    pub loops2: Vec<Vec<P2>>,
    pub lo: Vec3,
    pub hi: Vec3,
}

impl Surf {
    /// The outward unit normal at `p` (a point of the surface).
    pub fn normal_at(&self, p: Vec3) -> Vec3 {
        match self {
            Surf::Plane { normal, .. } => *normal,
            Surf::Cyl {
                origin, axis, out, ..
            } => {
                let d = p - *origin;
                let radial = d - *axis * d.dot(*axis);
                radial.normalized(Tolerance::DEFAULT).unwrap_or(*axis) * *out
            }
        }
    }

    /// Distance from the (unbounded) surface.
    pub fn dist(&self, p: Vec3) -> f64 {
        match self {
            Surf::Plane { origin, normal, .. } => (p - *origin).dot(*normal).abs(),
            Surf::Cyl {
                origin, axis, r, ..
            } => {
                let d = p - *origin;
                ((d - *axis * d.dot(*axis)).length() - r).abs()
            }
        }
    }
}

impl BFace {
    /// The picture of a point of the surface. For a cylinder the angle is brought into the
    /// strip (winding faces; a point on the seam goes to the left edge) or next to the face.
    pub fn to2(&self, p: Vec3) -> P2 {
        match &self.surf {
            Surf::Plane { origin, x, y, .. } => {
                let d = p - *origin;
                (d.dot(*x), d.dot(*y))
            }
            Surf::Cyl {
                origin,
                axis,
                e1,
                e2,
                r,
                out,
                winding,
                x0,
            } => {
                let d = p - *origin;
                let phi = det::atan2(d.dot(*e2), d.dot(*e1));
                let w = TAU * r;
                let raw = out * r * phi;
                let x = if *winding {
                    // Left edge inclusive, with a little slack so seam points stay on it.
                    raw - w * ((raw - *x0 + 1e-9 * w) / w).floor()
                } else {
                    raw - w * ((raw - *x0) / w).round()
                };
                (x, d.dot(*axis))
            }
        }
    }

    /// The point of the surface with picture `(x, y)`.
    pub fn from2(&self, q: P2) -> Vec3 {
        match &self.surf {
            Surf::Plane { origin, x, y, .. } => *origin + *x * q.0 + *y * q.1,
            Surf::Cyl {
                origin,
                axis,
                e1,
                e2,
                r,
                out,
                ..
            } => {
                let phi = q.0 / (out * r);
                *origin + *axis * q.1 + (*e1 * det::cos(phi) + *e2 * det::sin(phi)) * *r
            }
        }
    }

    /// Where a point of the surface lies relative to the face.
    pub fn locate(&self, p: Vec3) -> Loc {
        for lp in &self.loops {
            for arc in lp {
                if arc.dist(p) <= TOL {
                    return Loc::Boundary;
                }
            }
        }
        if inside_even_odd(&self.loops2, self.to2(p)) {
            Loc::Inside
        } else {
            Loc::Outside
        }
    }

    pub fn overlaps(&self, other: &BFace) -> bool {
        self.lo.x <= other.hi.x + TOL
            && other.lo.x <= self.hi.x + TOL
            && self.lo.y <= other.hi.y + TOL
            && other.lo.y <= self.hi.y + TOL
            && self.lo.z <= other.hi.z + TOL
            && other.lo.z <= self.hi.z + TOL
    }

    /// The parts of the curve `geo` over `t0..t1` that are inside the face or on its boundary,
    /// as sorted parameter intervals. The curve must lie on the face's surface.
    pub fn clip_curve(&self, geo: &Geo, t0: f64, t1: f64) -> Vec<(f64, f64)> {
        let arc = Arc {
            geo: geo.clone(),
            t0,
            t1,
        };
        let mut crit = vec![t0, t1];
        for b in self.loops.iter().flatten() {
            for (ta, _) in arc_arc(&arc, b) {
                crit.push(ta);
            }
            for q in [b.start(), b.end()] {
                if let Some(t) = arc.param_of_point(q) {
                    crit.push(t);
                }
            }
        }
        crit.retain(|t| *t >= t0 - 1e-12 && *t <= t1 + 1e-12);
        crit.sort_by(f64::total_cmp);
        let scale = geo.radius().max(1.0);
        let mut cuts: Vec<f64> = Vec::new();
        for t in crit {
            if cuts
                .last()
                .is_none_or(|l| (t - l) * scale.max(1.0) > TOL * 0.5)
            {
                cuts.push(t);
            }
        }
        let mut out: Vec<(f64, f64)> = Vec::new();
        for w in cuts.windows(2) {
            let m = 0.5 * (w[0] + w[1]);
            if self.locate(geo.point(m)) != Loc::Outside {
                match out.last_mut() {
                    Some(l) if (w[0] - l.1).abs() <= 1e-12 => l.1 = w[1],
                    _ => out.push((w[0], w[1])),
                }
            }
        }
        out
    }
}

fn unwrap_chain(face: &Surf, pts: &[Vec3], start: Option<f64>) -> Option<Vec<P2>> {
    let Surf::Cyl {
        origin,
        axis,
        e1,
        e2,
        r,
        out,
        ..
    } = face
    else {
        return None;
    };
    let w = TAU * r;
    let mut v: Vec<P2> = Vec::with_capacity(pts.len());
    let mut prev = start;
    for p in pts {
        let d = *p - *origin;
        let mut x = out * r * det::atan2(d.dot(*e2), d.dot(*e1));
        if let Some(q) = prev {
            x -= w * ((x - q) / w).round();
        }
        prev = Some(x);
        v.push((x, d.dot(*axis)));
    }
    Some(v)
}

/// Faces of `solid` as boolean faces, or why it cannot be handled yet.
pub fn prepare(solid: &Solid, side: Side) -> Result<Vec<BFace>, BoolError> {
    let mut out = Vec::new();
    for (idx, f) in solid.faces.iter().enumerate() {
        let mut loops: Vec<Vec<Arc>> = Vec::new();
        for lp in &f.loops {
            let mut arcs = Vec::new();
            for &c in lp {
                let e = &solid.edges[c.edge as usize];
                let arc = match &e.curve {
                    Curve::Line => Arc::line(solid.vertex(e.v0), solid.vertex(e.v1)),
                    Curve::Circle {
                        center,
                        axis,
                        x_dir,
                        radius,
                        t0,
                        sweep,
                    } => Some(Arc {
                        geo: Geo::Circle {
                            c: *center,
                            n: *axis,
                            u: *x_dir,
                            r: *radius,
                        },
                        t0: *t0,
                        t1: t0 + sweep,
                    }),
                    Curve::Ellipse {
                        center,
                        axis,
                        x_dir,
                        a,
                        b,
                        t0,
                        sweep,
                    } => Some(Arc {
                        geo: Geo::Ellipse {
                            c: *center,
                            n: *axis,
                            u: *x_dir,
                            a: *a,
                            b: *b,
                        },
                        t0: *t0,
                        t1: t0 + sweep,
                    }),
                };
                let arc = arc.ok_or_else(|| {
                    BoolError::Degenerate(format!("face '{}' has an edge of no length", f.name))
                })?;
                arcs.push(if c.reversed { arc.reversed() } else { arc });
            }
            loops.push(arcs);
        }
        let mut surf = match &f.surface {
            Surface::Plane {
                origin,
                normal,
                x_dir,
            } => {
                let n = if f.same_sense { *normal } else { -*normal };
                Surf::Plane {
                    origin: *origin,
                    normal: n,
                    x: *x_dir,
                    y: n.cross(*x_dir),
                }
            }
            Surface::Cylinder {
                origin,
                axis,
                ref_dir,
                radius,
                ..
            } => Surf::Cyl {
                origin: *origin,
                axis: *axis,
                e1: *ref_dir,
                e2: axis.cross(*ref_dir),
                r: *radius,
                out: if f.same_sense { 1.0 } else { -1.0 },
                winding: false,
                x0: 0.0,
            },
            Surface::Cone { .. } | Surface::Torus { .. } => {
                return Err(BoolError::Unsupported(format!(
                    "face '{}' is a cone or torus (only planes and cylinders are supported so far)",
                    f.name
                )))
            }
        };
        // The picture of every loop.
        let mut loops2: Vec<Vec<P2>> = Vec::new();
        let mut lo = Vec3::new(f64::MAX, f64::MAX, f64::MAX);
        let mut hi = Vec3::new(f64::MIN, f64::MIN, f64::MIN);
        let mut grow = |p: Vec3| {
            lo = Vec3::new(lo.x.min(p.x), lo.y.min(p.y), lo.z.min(p.z));
            hi = Vec3::new(hi.x.max(p.x), hi.y.max(p.y), hi.z.max(p.z));
        };
        let mut raw_loops: Vec<Vec<Vec3>> = Vec::new();
        for lp in &loops {
            let mut pts: Vec<Vec3> = Vec::new();
            for arc in lp {
                let pl = arc.polyline();
                for &p in &pl {
                    grow(p);
                }
                pts.extend(pl[..pl.len() - 1].iter().copied());
            }
            raw_loops.push(pts);
        }
        match &mut surf {
            Surf::Plane { origin, x, y, .. } => {
                for pts in &raw_loops {
                    loops2.push(
                        pts.iter()
                            .map(|&p| {
                                let d = p - *origin;
                                (d.dot(*x), d.dot(*y))
                            })
                            .collect(),
                    );
                }
            }
            Surf::Cyl { .. } => {
                let snapshot = surf.clone();
                let Surf::Cyl { r, .. } = &snapshot else {
                    unreachable!()
                };
                let w = TAU * r;
                // Outer loop first, continuously unwrapped; it closes or it winds (seam).
                let outer = unwrap_chain(&snapshot, &raw_loops[0], None).expect("cylinder");
                let first = outer[0].0;
                let (mut xmin, mut xmax) = (f64::MAX, f64::MIN);
                for q in &outer {
                    xmin = xmin.min(q.0);
                    xmax = xmax.max(q.0);
                }
                // Does the chain close in the picture (going once more round to the start)?
                let last = *outer.last().expect("non-empty");
                let first_pt = raw_loops[0][0];
                let closing = unwrap_chain(&snapshot, &[first_pt], Some(last.0)).expect("cyl")[0];
                if (closing.0 - first).abs() > 1e-6 * w.max(1.0) {
                    return Err(BoolError::Unsupported(format!(
                        "cylinder face '{}' winds round its axis without a seam edge",
                        f.name
                    )));
                }
                let winding = xmax - xmin > w - 1e-6 * w.max(1.0);
                if let Surf::Cyl {
                    winding: wf, x0, ..
                } = &mut surf
                {
                    *wf = winding;
                    *x0 = if winding { xmin } else { 0.5 * (xmin + xmax) };
                }
                loops2.push(outer);
                let snapshot = surf.clone();
                for pts in &raw_loops[1..] {
                    // A hole is placed in the strip; one that crosses the seam is refused.
                    let probe = BFace {
                        side,
                        idx,
                        name: String::new(),
                        surf: snapshot.clone(),
                        loops: Vec::new(),
                        loops2: Vec::new(),
                        lo,
                        hi,
                    };
                    let x_start = probe.to2(pts[0]).0;
                    let chain = unwrap_chain(&snapshot, pts, Some(x_start)).expect("cyl");
                    let (cmin, cmax) = chain
                        .iter()
                        .fold((f64::MAX, f64::MIN), |(a, b), q| (a.min(q.0), b.max(q.0)));
                    let Surf::Cyl { x0, winding, .. } = &snapshot else {
                        unreachable!()
                    };
                    if *winding && (cmin < *x0 - 1e-9 || cmax > *x0 + w + 1e-9) {
                        return Err(BoolError::Unsupported(format!(
                            "a hole of cylinder face '{}' crosses the seam",
                            f.name
                        )));
                    }
                    loops2.push(chain);
                }
            }
        }
        out.push(BFace {
            side,
            idx,
            name: f.name.clone(),
            surf,
            loops,
            loops2,
            lo,
            hi,
        });
    }
    Ok(out)
}
