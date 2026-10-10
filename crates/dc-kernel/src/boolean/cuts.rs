//! Where two faces meet: the intersection curves of their surfaces, clipped to both faces.

use super::geo::{plane_cylinder_section, Arc, Geo};
use super::prep::{BFace, Surf, TOL};
use super::BoolError;
use dc_math::{det, Tolerance, Vec3};

const TAU: f64 = 2.0 * core::f64::consts::PI;

/// A curve on both faces' surfaces, as a parameter interval, and which faces it cuts.
pub struct Cut {
    pub arc: Arc,
    pub into_a: bool,
    pub into_b: bool,
}

fn intersect_intervals(a: &[(f64, f64)], b: &[(f64, f64)]) -> Vec<(f64, f64)> {
    let mut out = Vec::new();
    for &(a0, a1) in a {
        for &(b0, b1) in b {
            let (lo, hi) = (a0.max(b0), a1.min(b1));
            if hi - lo > 1e-12 {
                out.push((lo, hi));
            }
        }
    }
    out.sort_by(|x, y| x.0.total_cmp(&y.0));
    out
}

/// For a closed curve scanned over `0..period`: an interval that ends at the period and one that
/// starts at zero are one piece, so join them (the joined one runs past the period).
fn join_wrap(mut v: Vec<(f64, f64)>, period: f64) -> Vec<(f64, f64)> {
    if v.len() > 1 && v[0].0 <= 1e-12 && v[v.len() - 1].1 >= period - 1e-12 {
        let first = v.remove(0);
        let last = v.pop().expect("two or more");
        v.push((last.0, first.1 + period));
    }
    v
}

fn clip_both(
    fa: &BFace,
    fb: &BFace,
    geo: &Geo,
    t0: f64,
    t1: f64,
    period: Option<f64>,
) -> Vec<(f64, f64)> {
    let ia = fa.clip_curve(geo, t0, t1);
    let ib = fb.clip_curve(geo, t0, t1);
    let both = intersect_intervals(&ia, &ib);
    match period {
        Some(p) => join_wrap(both, p),
        None => both,
    }
}

fn line_range(fa: &BFace, fb: &BFace, o: Vec3, d: Vec3) -> (f64, f64) {
    let (mut t0, mut t1) = (f64::MAX, f64::MIN);
    for face in [fa, fb] {
        for p in [face.lo, face.hi] {
            let t = (p - o).dot(d);
            t0 = t0.min(t);
            t1 = t1.max(t);
        }
    }
    let pad = (fa.hi - fa.lo).length() + (fb.hi - fb.lo).length() + 1.0;
    (t0 - pad, t1 + pad)
}

/// The curves along which the faces meet (or, for coincident surfaces, each face's boundary
/// clipped to the other).
pub fn pair_cuts(fa: &BFace, fb: &BFace) -> Result<Vec<Cut>, BoolError> {
    let tol = Tolerance::DEFAULT;
    match (&fa.surf, &fb.surf) {
        (
            Surf::Plane {
                origin: oa,
                normal: na,
                ..
            },
            Surf::Plane {
                origin: ob,
                normal: nb,
                ..
            },
        ) => {
            if tol.parallel(*na, *nb) {
                if (*oa - *ob).dot(*na).abs() > TOL {
                    return Ok(Vec::new());
                }
                return Ok(coincident(fa, fb));
            }
            let u = na.cross(*nb);
            let (da, db) = (na.dot(*oa), nb.dot(*ob));
            let p0 = (nb.cross(u) * da + u.cross(*na) * db) / u.dot(u);
            let Some(d) = u.normalized(tol) else {
                return Ok(Vec::new());
            };
            let (t0, t1) = line_range(fa, fb, p0, d);
            let geo = Geo::Line { o: p0, d };
            Ok(clip_both(fa, fb, &geo, t0, t1, None)
                .into_iter()
                .map(|(lo, hi)| Cut {
                    arc: Arc {
                        geo: geo.clone(),
                        t0: lo,
                        t1: hi,
                    },
                    into_a: true,
                    into_b: true,
                })
                .collect())
        }
        (Surf::Plane { .. }, Surf::Cyl { .. }) => plane_cyl(fa, fb),
        (Surf::Cyl { .. }, Surf::Plane { .. }) => plane_cyl(fb, fa),
        (
            Surf::Cyl {
                origin: oa,
                axis: aa,
                r: ra,
                ..
            },
            Surf::Cyl {
                origin: ob,
                axis: ab,
                r: rb,
                ..
            },
        ) => {
            if tol.parallel(*aa, *ab) {
                // Distance between the two axes, measured across them.
                let w = *ob - *oa;
                let wp = w - *aa * w.dot(*aa);
                let across = wp.length();
                if across <= TOL {
                    return Ok(if (ra - rb).abs() <= TOL {
                        coincident(fa, fb)
                    } else {
                        Vec::new()
                    });
                }
                if across > ra + rb + TOL || across < (ra - rb).abs() - TOL {
                    return Ok(Vec::new());
                }
                return parallel_cylinders(fa, fb, *oa, *aa, *ra, *rb, wp, across);
            }
            Err(BoolError::Unsupported(format!(
                "faces '{}' and '{}' are cylinders that cross each other (only cylinders with parallel axes are supported so far)",
                fa.name, fb.name
            )))
        }
    }
}

/// Two cylinders with parallel axes that overlap: they meet along two straight lines (or one,
/// if they only touch, which is refused when the contact meets both faces).
#[allow(clippy::too_many_arguments)]
fn parallel_cylinders(
    fa: &BFace,
    fb: &BFace,
    oa: Vec3,
    axis: Vec3,
    ra: f64,
    rb: f64,
    wp: Vec3,
    d: f64,
) -> Result<Vec<Cut>, BoolError> {
    let u = wp / d;
    let v = axis.cross(u);
    let x = (d * d + ra * ra - rb * rb) / (2.0 * d);
    let y2 = ra * ra - x * x;
    let tangent = y2 <= 0.0 || y2.sqrt() <= TOL;
    let y = if tangent { 0.0 } else { y2.sqrt() };
    let ys: Vec<f64> = if tangent { vec![0.0] } else { vec![y, -y] };
    let mut out = Vec::new();
    for yy in ys {
        let base = oa + u * x + v * yy;
        let (mut lo, mut hi) = (f64::MAX, f64::MIN);
        for f in [fa, fb] {
            for p in [f.lo, f.hi] {
                let t = (p - base).dot(axis);
                lo = lo.min(t);
                hi = hi.max(t);
            }
        }
        let pad = (fa.hi - fa.lo).length() + (fb.hi - fb.lo).length() + 1.0;
        let geo = Geo::Line { o: base, d: axis };
        let spans = clip_both(fa, fb, &geo, lo - pad, hi + pad, None);
        if tangent && !spans.is_empty() {
            return Err(BoolError::Unsupported(format!(
                "the cylinders '{}' and '{}' touch along a line (tangent contact)",
                fa.name, fb.name
            )));
        }
        for (t0, t1) in spans {
            out.push(Cut {
                arc: Arc {
                    geo: geo.clone(),
                    t0,
                    t1,
                },
                into_a: true,
                into_b: true,
            });
        }
    }
    Ok(out)
}

/// Coincident surfaces: each face's boundary, clipped to the other face, cuts the other face.
fn coincident(fa: &BFace, fb: &BFace) -> Vec<Cut> {
    let mut out = Vec::new();
    for (src, dst, into_a) in [(fb, fa, true), (fa, fb, false)] {
        for arc in src.loops.iter().flatten() {
            let (lo, hi) = (arc.lo(), arc.hi());
            for (t0, t1) in dst.clip_curve(&arc.geo, lo, hi) {
                out.push(Cut {
                    arc: Arc {
                        geo: arc.geo.clone(),
                        t0,
                        t1,
                    },
                    into_a,
                    into_b: !into_a,
                });
            }
        }
    }
    out
}

fn plane_cyl(plane: &BFace, cyl: &BFace) -> Result<Vec<Cut>, BoolError> {
    let tol = Tolerance::DEFAULT;
    let Surf::Plane {
        origin: op,
        normal: n,
        ..
    } = &plane.surf
    else {
        unreachable!()
    };
    let Surf::Cyl {
        origin: o,
        axis: a,
        e1,
        e2,
        r,
        ..
    } = &cyl.surf
    else {
        unreachable!()
    };
    let (n, a) = (*n, *a);
    let na = n.dot(a);
    let mut geos: Vec<(Geo, f64, f64, Option<f64>)> = Vec::new();
    if na.abs() > tol.angular {
        // Across the axis (a circle) or oblique (an ellipse).
        if let Some(geo) = plane_cylinder_section(*op, n, *o, a, *e1, *r) {
            geos.push((geo, 0.0, TAU, Some(TAU)));
        }
    } else if na.abs() <= tol.angular {
        // Plane along the axis: two straight lines, one (tangent) or none.
        let dprime = n.dot(*o - *op);
        let ratio = -dprime / r;
        if ratio.abs() > 1.0 + TOL / r {
            return Ok(Vec::new());
        }
        let tangent = (ratio.abs() - 1.0).abs() * r <= TOL;
        let psi = det::atan2(n.dot(*e2), n.dot(*e1));
        let off = det::acos(ratio.clamp(-1.0, 1.0));
        let phis: Vec<f64> = if tangent {
            vec![psi + off]
        } else {
            vec![psi + off, psi - off]
        };
        for phi in phis {
            let base = *o + (*e1 * det::cos(phi) + *e2 * det::sin(phi)) * *r;
            let (t0, t1) = {
                let (mut lo, mut hi) = (f64::MAX, f64::MIN);
                for f in [plane, cyl] {
                    for p in [f.lo, f.hi] {
                        let t = (p - base).dot(a);
                        lo = lo.min(t);
                        hi = hi.max(t);
                    }
                }
                let pad = (plane.hi - plane.lo).length() + (cyl.hi - cyl.lo).length() + 1.0;
                (lo - pad, hi + pad)
            };
            if tangent {
                // Touching along a line: only a problem if the line meets both faces.
                let geo = Geo::Line { o: base, d: a };
                if !clip_both(plane, cyl, &geo, t0, t1, None).is_empty() {
                    return Err(BoolError::Unsupported(format!(
                        "the plane of '{}' touches the cylinder '{}' along a line (tangent contact)",
                        plane.name, cyl.name
                    )));
                }
                return Ok(Vec::new());
            }
            geos.push((Geo::Line { o: base, d: a }, t0, t1, None));
        }
    }
    let mut out = Vec::new();
    for (geo, t0, t1, period) in geos {
        for (lo, hi) in clip_both(plane, cyl, &geo, t0, t1, period) {
            out.push(Cut {
                arc: Arc {
                    geo: geo.clone(),
                    t0: lo,
                    t1: hi,
                },
                into_a: true,
                into_b: true,
            });
        }
    }
    Ok(out)
}
