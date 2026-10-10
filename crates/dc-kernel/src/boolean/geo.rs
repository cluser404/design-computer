//! Curves for the boolean: lines, circles and ellipses as parametric geometry, directed arcs of
//! them, polylines for decisions, and curve/curve intersection by refinement.
//!
//! Exact geometry is what ends up in the result; polylines (sagitta at most [`SAG`]) are only
//! used to decide inside/outside, so a decision can differ from the exact answer only for points
//! within about `SAG` of a boundary, which the callers treat as "on the boundary" anyway.

use dc_math::{det, Vec3};

use super::prep::TOL;

/// Largest distance between a curve and the polyline used for decisions.
pub const SAG: f64 = 2e-7;
const MAX_POINTS: usize = 20_000;
const TAU: f64 = 2.0 * core::f64::consts::PI;

#[derive(Clone, Debug, PartialEq)]
pub enum Geo {
    /// `o + t d`, `d` a unit vector; `t` is length.
    Line { o: Vec3, d: Vec3 },
    /// `c + r (cos t u + sin t w)`, `w = n × u`.
    Circle { c: Vec3, n: Vec3, u: Vec3, r: f64 },
    /// `c + a cos t u + b sin t w`, `w = n × u`.
    Ellipse {
        c: Vec3,
        n: Vec3,
        u: Vec3,
        a: f64,
        b: f64,
    },
}

impl Geo {
    pub fn point(&self, t: f64) -> Vec3 {
        match self {
            Geo::Line { o, d } => *o + *d * t,
            Geo::Circle { c, n, u, r } => {
                *c + *u * (r * det::cos(t)) + n.cross(*u) * (r * det::sin(t))
            }
            Geo::Ellipse { c, n, u, a, b } => {
                *c + *u * (a * det::cos(t)) + n.cross(*u) * (b * det::sin(t))
            }
        }
    }

    /// Derivative of `point` with respect to `t`.
    pub fn tangent(&self, t: f64) -> Vec3 {
        match self {
            Geo::Line { d, .. } => *d,
            Geo::Circle { n, u, r, .. } => {
                *u * (-r * det::sin(t)) + n.cross(*u) * (r * det::cos(t))
            }
            Geo::Ellipse { n, u, a, b, .. } => {
                *u * (-a * det::sin(t)) + n.cross(*u) * (b * det::cos(t))
            }
        }
    }

    /// The parameter of the point of the curve nearest `p` in angle (exact for points on the
    /// curve): a length along a line, an angle in `(-π, π]` otherwise.
    pub fn param_of(&self, p: Vec3) -> f64 {
        match self {
            Geo::Line { o, d } => (p - *o).dot(*d),
            Geo::Circle { c, n, u, .. } => {
                let q = p - *c;
                det::atan2(q.dot(n.cross(*u)), q.dot(*u))
            }
            Geo::Ellipse { c, n, u, a, b } => {
                let q = p - *c;
                det::atan2(q.dot(n.cross(*u)) / b, q.dot(*u) / a)
            }
        }
    }

    pub fn period(&self) -> Option<f64> {
        match self {
            Geo::Line { .. } => None,
            _ => Some(TAU),
        }
    }

    /// Largest distance of the curve's points from its centre (0 for a line).
    pub fn radius(&self) -> f64 {
        match self {
            Geo::Line { .. } => 0.0,
            Geo::Circle { r, .. } => *r,
            Geo::Ellipse { a, b, .. } => a.max(*b),
        }
    }

    fn max_curvature(&self) -> f64 {
        match self {
            Geo::Line { .. } => 0.0,
            Geo::Circle { r, .. } => 1.0 / r,
            Geo::Ellipse { a, b, .. } => a.max(*b) / (a.min(*b) * a.min(*b)),
        }
    }

    /// The plane's normal for a circle or ellipse.
    pub fn normal(&self) -> Option<Vec3> {
        match self {
            Geo::Line { .. } => None,
            Geo::Circle { n, .. } | Geo::Ellipse { n, .. } => Some(*n),
        }
    }
}

/// A directed piece of a curve from parameter `t0` to `t1` (`t1 < t0` runs it backwards).
#[derive(Clone, Debug, PartialEq)]
pub struct Arc {
    pub geo: Geo,
    pub t0: f64,
    pub t1: f64,
}

impl Arc {
    pub fn line(a: Vec3, b: Vec3) -> Option<Arc> {
        let len = (b - a).length();
        if len <= TOL {
            return None;
        }
        Some(Arc {
            geo: Geo::Line {
                o: a,
                d: (b - a) / len,
            },
            t0: 0.0,
            t1: len,
        })
    }

    pub fn point(&self, t: f64) -> Vec3 {
        self.geo.point(t)
    }

    pub fn start(&self) -> Vec3 {
        self.geo.point(self.t0)
    }

    pub fn end(&self) -> Vec3 {
        self.geo.point(self.t1)
    }

    pub fn mid(&self) -> Vec3 {
        self.geo.point(0.5 * (self.t0 + self.t1))
    }

    pub fn reversed(&self) -> Arc {
        Arc {
            geo: self.geo.clone(),
            t0: self.t1,
            t1: self.t0,
        }
    }

    pub fn span(&self) -> f64 {
        (self.t1 - self.t0).abs()
    }

    pub fn lo(&self) -> f64 {
        self.t0.min(self.t1)
    }

    pub fn hi(&self) -> f64 {
        self.t0.max(self.t1)
    }

    /// A full period of a closed curve.
    pub fn is_full(&self) -> bool {
        self.geo
            .period()
            .is_some_and(|p| (self.span() - p).abs() <= 1e-9)
    }

    /// The sub-arc between parameters `a` and `b` (in this arc's direction if `a` precedes `b`).
    pub fn between(&self, a: f64, b: f64) -> Arc {
        Arc {
            geo: self.geo.clone(),
            t0: a,
            t1: b,
        }
    }

    /// The parameter, unwrapped into this arc's range, of `p` if `p` lies on the arc (within
    /// tolerance, endpoints included).
    pub fn param_of_point(&self, p: Vec3) -> Option<f64> {
        let mut t = self.geo.param_of(p);
        let (lo, hi) = (self.lo(), self.hi());
        if let Some(per) = self.geo.period() {
            let slack = TOL / self.geo.radius().max(TOL);
            t -= per * ((t - lo + slack) / per).floor();
            if t > hi + slack {
                // The point may sit at the very start of the next period.
                if (self.geo.point(lo) - p).length() <= TOL {
                    t = lo;
                } else if (self.geo.point(hi) - p).length() <= TOL {
                    t = hi;
                } else {
                    return None;
                }
            }
        } else if t < lo - TOL || t > hi + TOL {
            return None;
        }
        ((self.geo.point(t) - p).length() <= TOL).then_some(t.clamp(lo, hi))
    }

    /// Distance from `p` to the arc.
    pub fn dist(&self, p: Vec3) -> f64 {
        let (lo, hi) = (self.lo(), self.hi());
        let t = self.geo.param_of(p);
        let mut best = match self.geo.period() {
            None => t.clamp(lo, hi),
            Some(per) => {
                let tt = t - per * ((t - lo) / per).floor();
                if tt <= hi {
                    tt
                } else if (p - self.geo.point(lo)).length() <= (p - self.geo.point(hi)).length() {
                    lo
                } else {
                    hi
                }
            }
        };
        // An ellipse's nearest point is not at the same parameter: refine by Newton on the
        // squared distance (a few steps; for circles and lines this converges at once).
        if matches!(self.geo, Geo::Ellipse { .. }) {
            for _ in 0..30 {
                let (q, d1) = (self.geo.point(best), self.geo.tangent(best));
                let h = 1e-6;
                let (qp, qm) = (self.geo.point(best + h), self.geo.point(best - h));
                let d2 = (qp - q * 2.0 + qm) / (h * h);
                let g = (q - p).dot(d1);
                let hess = d1.dot(d1) + (q - p).dot(d2);
                if hess.abs() < 1e-14 {
                    break;
                }
                let next = (best - g / hess).clamp(lo, hi);
                if (next - best).abs() < 1e-14 {
                    best = next;
                    break;
                }
                best = next;
            }
        }
        (self.geo.point(best) - p).length()
    }

    /// Points along the arc (both ends included) with sagitta at most `sag` (and at most
    /// `MAX_POINTS` points).
    pub fn polyline_sag(&self, sag: f64) -> Vec<Vec3> {
        let n = match self.geo {
            Geo::Line { .. } => 1,
            _ => {
                let k = self.geo.max_curvature();
                let speed = self.geo.radius();
                let dt = det::sqrt(8.0 * sag / k) / speed;
                ((self.span() / dt).ceil() as usize).clamp(2, MAX_POINTS)
            }
        };
        (0..=n)
            .map(|i| {
                if i == n {
                    self.end()
                } else {
                    self.point(self.t0 + (self.t1 - self.t0) * (i as f64 / n as f64))
                }
            })
            .collect()
    }

    pub fn polyline(&self) -> Vec<Vec3> {
        self.polyline_sag(SAG)
    }

    fn coarse(&self) -> Vec<(f64, Vec3)> {
        let n = if matches!(self.geo, Geo::Line { .. }) {
            1
        } else {
            192
        };
        (0..=n)
            .map(|i| {
                let t = self.t0 + (self.t1 - self.t0) * (i as f64 / n as f64);
                (t, self.point(t))
            })
            .collect()
    }

    fn coarse_sag(&self) -> f64 {
        match self.geo {
            Geo::Line { .. } => 0.0,
            _ => {
                let step = self.span() / 192.0 * self.geo.radius();
                self.geo.max_curvature() * step * step / 8.0
            }
        }
    }
}

/// Closest approach of segments `p0 p1` and `q0 q1`.
fn segment_distance(p0: Vec3, p1: Vec3, q0: Vec3, q1: Vec3) -> f64 {
    let (u, v, w) = (p1 - p0, q1 - q0, p0 - q0);
    let (a, b, c, d, e) = (u.dot(u), u.dot(v), v.dot(v), u.dot(w), v.dot(w));
    let den = a * c - b * b;
    // Closest points of the infinite lines, then clamped back onto the segments.
    let mut s = if den > 1e-14 * a * c {
        ((b * e - c * d) / den).clamp(0.0, 1.0)
    } else {
        0.0
    };
    let mut t = if c > 0.0 {
        ((b * s + e) / c).clamp(0.0, 1.0)
    } else {
        0.0
    };
    s = if a > 0.0 {
        ((b * t - d) / a).clamp(0.0, 1.0)
    } else {
        0.0
    };
    t = if c > 0.0 {
        ((b * s + e) / c).clamp(0.0, 1.0)
    } else {
        0.0
    };
    ((p0 + u * s) - (q0 + v * t)).length()
}

/// Parameters `(ta, tb)` where the arcs cross (touch within tolerance), each refined to a point
/// common to both curves. Overlapping stretches are not reported here (see the callers).
pub fn arc_arc(a: &Arc, b: &Arc) -> Vec<(f64, f64)> {
    let (pa, pb) = (a.coarse(), b.coarse());
    let thresh = 2.0 * (a.coarse_sag() + b.coarse_sag()) + 1e-5;
    let mut found: Vec<(f64, f64)> = Vec::new();
    for i in 0..pa.len() - 1 {
        for j in 0..pb.len() - 1 {
            if segment_distance(pa[i].1, pa[i + 1].1, pb[j].1, pb[j + 1].1) > thresh {
                continue;
            }
            // Start from the middle of both segments (and the ends) and refine.
            for (fa, fb) in [(0.5, 0.5), (0.0, 0.0), (1.0, 1.0), (0.0, 1.0), (1.0, 0.0)] {
                let mut ta = pa[i].0 + (pa[i + 1].0 - pa[i].0) * fa;
                let mut tb = pb[j].0 + (pb[j + 1].0 - pb[j].0) * fb;
                let mut ok = false;
                for _ in 0..40 {
                    let (x, y) = (a.point(ta), b.point(tb));
                    let r = x - y;
                    if r.length() < 1e-11 {
                        ok = true;
                        break;
                    }
                    let (da, db) = (a.geo.tangent(ta), b.geo.tangent(tb));
                    // Solve min |r + da dta - db dtb|^2.
                    let (m11, m12, m22) = (da.dot(da), -da.dot(db), db.dot(db));
                    let (g1, g2) = (-da.dot(r), db.dot(r));
                    let det_ = m11 * m22 - m12 * m12;
                    if det_.abs() < 1e-18 * m11 * m22 {
                        break;
                    }
                    let (dta, dtb) = ((g1 * m22 - g2 * m12) / det_, (m11 * g2 - m12 * g1) / det_);
                    ta += dta;
                    tb += dtb;
                    if dta.abs() < 1e-15 && dtb.abs() < 1e-15 {
                        ok = (a.point(ta) - b.point(tb)).length() < 1e-9;
                        break;
                    }
                }
                if !ok {
                    continue;
                }
                let (ta, tb) = (wrap_into(a, ta), wrap_into(b, tb));
                let (Some(ta), Some(tb)) = (ta, tb) else {
                    continue;
                };
                if !found.iter().any(|&(x, y)| {
                    (a.point(x) - a.point(ta)).length() < 1e-7
                        && (b.point(y) - b.point(tb)).length() < 1e-7
                }) {
                    found.push((ta, tb));
                }
                break;
            }
        }
    }
    found.sort_by(|x, y| x.0.total_cmp(&y.0).then(x.1.total_cmp(&y.1)));
    found
}

/// `t` brought into the arc's parameter range (by whole periods), or `None` if it is outside.
fn wrap_into(a: &Arc, t: f64) -> Option<f64> {
    let (lo, hi) = (a.lo(), a.hi());
    let slack = 1e-9;
    match a.geo.period() {
        None => (t >= lo - slack && t <= hi + slack).then_some(t.clamp(lo, hi)),
        Some(per) => {
            let t = t - per * ((t - lo + slack) / per).floor();
            (t <= hi + slack).then_some(t.min(hi))
        }
    }
}

/// The curve in which a plane cuts a right circular cylinder (axis `a` through `o`, reference
/// direction `e1` across the axis, radius `r`): a circle if the plane is across the axis, an
/// ellipse if it is oblique; `None` if the plane is parallel to the axis (lines, or nothing).
pub fn plane_cylinder_section(
    plane_origin: Vec3,
    n: Vec3,
    o: Vec3,
    a: Vec3,
    e1: Vec3,
    r: f64,
) -> Option<Geo> {
    let na = n.dot(a);
    if na.abs() <= 1e-9 {
        return None;
    }
    let e2 = a.cross(e1);
    let t = n.dot(plane_origin - o) / na;
    let c = o + a * t;
    if n.cross(a).length() <= 1e-9 {
        return Some(Geo::Circle { c, n: a, u: e1, r });
    }
    // Conjugate semi-diameters U, V of the ellipse, turned to its principal axes.
    let (ka, kb) = (n.dot(e1) / na, n.dot(e2) / na);
    let (uu, vv) = ((e1 - a * ka) * r, (e2 - a * kb) * r);
    let phi0 = 0.5 * det::atan2(2.0 * uu.dot(vv), uu.dot(uu) - vv.dot(vv));
    let (cs, sn) = (det::cos(phi0), det::sin(phi0));
    let (up, wp) = (uu * cs + vv * sn, uu * (-sn) + vv * cs);
    let (ea, eb) = (up.length(), wp.length());
    let (uh, wh) = (up / ea, wp / eb);
    Some(Geo::Ellipse {
        c,
        n: uh.cross(wh),
        u: uh,
        a: ea,
        b: eb,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn v(x: f64, y: f64, z: f64) -> Vec3 {
        Vec3::new(x, y, z)
    }

    fn circle(c: Vec3, r: f64) -> Arc {
        Arc {
            geo: Geo::Circle {
                c,
                n: v(0.0, 0.0, 1.0),
                u: v(1.0, 0.0, 0.0),
                r,
            },
            t0: 0.0,
            t1: TAU,
        }
    }

    #[test]
    fn a_line_crosses_a_circle_twice() {
        let c = circle(v(0.0, 0.0, 0.0), 3.0);
        let l = Arc::line(v(-5.0, 1.0, 0.0), v(5.0, 1.0, 0.0)).unwrap();
        let hits = arc_arc(&c, &l);
        assert_eq!(hits.len(), 2, "{hits:?}");
        for (tc, tl) in hits {
            let (p, q) = (c.point(tc), l.point(tl));
            assert!((p - q).length() < 1e-9);
            assert!((p.length() - 3.0).abs() < 1e-9 && (p.y - 1.0).abs() < 1e-9);
        }
    }

    #[test]
    fn two_circles_cross_twice_and_a_far_one_not_at_all() {
        let a = circle(v(0.0, 0.0, 0.0), 3.0);
        let b = circle(v(4.0, 0.0, 0.0), 2.0);
        let hits = arc_arc(&a, &b);
        assert_eq!(hits.len(), 2, "{hits:?}");
        assert!(arc_arc(&a, &circle(v(10.0, 0.0, 0.0), 2.0)).is_empty());
    }

    #[test]
    fn an_ellipse_meets_a_line_where_expected() {
        let e = Arc {
            geo: Geo::Ellipse {
                c: v(0.0, 0.0, 0.0),
                n: v(0.0, 0.0, 1.0),
                u: v(1.0, 0.0, 0.0),
                a: 5.0,
                b: 2.0,
            },
            t0: 0.0,
            t1: TAU,
        };
        let l = Arc::line(v(3.0, -4.0, 0.0), v(3.0, 4.0, 0.0)).unwrap();
        let hits = arc_arc(&e, &l);
        assert_eq!(hits.len(), 2);
        let want = 2.0 * (1.0 - 9.0 / 25.0f64).sqrt();
        for (te, _) in hits {
            assert!((e.point(te).y.abs() - want).abs() < 1e-9);
        }
    }

    #[test]
    fn points_on_an_arc_are_found_with_their_unwrapped_parameter() {
        // A quarter circle running backwards from 3π/2 down to π.
        let a = Arc {
            geo: Geo::Circle {
                c: v(0.0, 0.0, 0.0),
                n: v(0.0, 0.0, 1.0),
                u: v(1.0, 0.0, 0.0),
                r: 2.0,
            },
            t0: 1.5 * core::f64::consts::PI,
            t1: core::f64::consts::PI,
        };
        let p = a.point(1.25 * core::f64::consts::PI);
        let t = a.param_of_point(p).expect("on the arc");
        assert!((t - 1.25 * core::f64::consts::PI).abs() < 1e-12);
        assert!(a.param_of_point(v(2.0, 0.0, 0.0)).is_none(), "off the arc");
        assert!(
            a.param_of_point(v(2.0, 0.1, 0.0)).is_none(),
            "off the curve"
        );
    }

    #[test]
    fn the_distance_to_an_ellipse_is_the_true_distance() {
        let e = Arc {
            geo: Geo::Ellipse {
                c: v(0.0, 0.0, 0.0),
                n: v(0.0, 0.0, 1.0),
                u: v(1.0, 0.0, 0.0),
                a: 5.0,
                b: 2.0,
            },
            t0: 0.0,
            t1: TAU,
        };
        // The nearest point of the ellipse to (0, 5) is the top (0, 2).
        assert!((e.dist(v(0.0, 5.0, 0.0)) - 3.0).abs() < 1e-9);
        assert!(e.dist(v(5.0, 0.0, 0.0)) < 1e-12);
    }

    #[test]
    fn polylines_stay_within_the_sagitta_and_close_the_circle() {
        let c = circle(v(0.0, 0.0, 0.0), 3.0);
        let pts = c.polyline();
        assert!((pts[0] - pts[pts.len() - 1]).length() < 1e-12);
        for w in pts.windows(2) {
            let mid = (w[0] + w[1]) * 0.5;
            assert!(
                3.0 - mid.length() <= SAG * 1.001,
                "sagitta {}",
                3.0 - mid.length()
            );
        }
    }
}
