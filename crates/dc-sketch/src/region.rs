//! 2D region boolean: `a − b` for profiles bounded by lines, arcs and circles.
//!
//! Every boundary edge of both regions is split where it meets the other region, each piece is
//! classified (inside, outside or coincident), the right pieces are kept (the tool's reversed),
//! and the pieces are stitched back into loops. Loops are then nested into profiles by the same
//! code that finds profiles in a sketch.
//!
//! Anything ambiguous is refused with [`RegionError::Unsupported`] instead of guessed: the two
//! boundaries touching at a single point, for example.

use crate::profile::{
    arc_area_term, dist_point_segment, in_sweep, nest, Curve2, Loop, LoopEdge, Prepared, Profile,
    RawLoop,
};
use dc_math::{det, Tolerance, Vec2};
use std::fmt;

const TWO_PI: f64 = 2.0 * core::f64::consts::PI;
/// Vertices closer than this are the same vertex.
const MERGE: f64 = 1e-7;

#[derive(Debug, Clone, PartialEq)]
pub enum RegionError {
    Unsupported(String),
}

impl fmt::Display for RegionError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let RegionError::Unsupported(m) = self;
        write!(f, "{m}")
    }
}
impl std::error::Error for RegionError {}

/// Result of a subtraction.
#[derive(Debug, Clone, PartialEq)]
pub struct Subtracted {
    /// What is left of `a`. Empty if `b` covers all of it.
    pub profiles: Vec<Profile>,
    /// False if `b` does not touch `a` at all (then `profiles` is just `a`).
    pub changed: bool,
}

#[derive(Clone, Debug)]
enum Geo {
    Line {
        a: Vec2,
        b: Vec2,
    },
    /// Directed arc; `sweep` is signed (positive = counter-clockwise).
    Arc {
        c: Vec2,
        r: f64,
        a0: f64,
        sweep: f64,
    },
}

#[derive(Clone, Debug)]
struct DEdge {
    id: String,
    geo: Geo,
}

fn v2(p: [f64; 2]) -> Vec2 {
    Vec2::new(p[0], p[1])
}

fn polar(c: Vec2, r: f64, t: f64) -> Vec2 {
    Vec2::new(c.x + r * det::cos(t), c.y + r * det::sin(t))
}

impl Geo {
    fn length(&self) -> f64 {
        match self {
            Geo::Line { a, b } => (*b - *a).length(),
            Geo::Arc { r, sweep, .. } => r * sweep.abs(),
        }
    }

    fn at(&self, f: f64) -> Vec2 {
        match self {
            Geo::Line { a, b } => *a + (*b - *a) * f,
            Geo::Arc { c, r, a0, sweep } => polar(*c, *r, a0 + sweep * f),
        }
    }

    fn start(&self) -> Vec2 {
        self.at(0.0)
    }

    fn end(&self) -> Vec2 {
        match self {
            Geo::Line { b, .. } => *b,
            Geo::Arc { .. } => self.at(1.0),
        }
    }

    fn tangent(&self, f: f64) -> Vec2 {
        match self {
            Geo::Line { a, b } => *b - *a,
            Geo::Arc { r, a0, sweep, .. } => {
                let t = a0 + sweep * f;
                let s = if *sweep >= 0.0 { 1.0 } else { -1.0 };
                Vec2::new(-det::sin(t), det::cos(t)) * (s * r)
            }
        }
    }

    /// Is `p` on this (bounded) curve, within `tol`?
    fn contains(&self, p: Vec2, tol: f64) -> bool {
        match self {
            Geo::Line { a, b } => dist_point_segment(p, *a, *b) <= tol,
            Geo::Arc { c, r, a0, sweep } => {
                if ((p - *c).length() - r).abs() > tol {
                    return false;
                }
                let (lo, span) = if *sweep >= 0.0 {
                    (*a0, *sweep)
                } else {
                    (*a0 + *sweep, -*sweep)
                };
                span >= TWO_PI - 1e-12
                    || in_sweep(det::atan2(p.y - c.y, p.x - c.x), lo, span, tol / r.max(tol))
            }
        }
    }

    /// Parameter in `0..=1` of a point known to be on the curve.
    fn param(&self, p: Vec2) -> f64 {
        match self {
            Geo::Line { a, b } => {
                let d = *b - *a;
                (p - *a).dot(d) / d.length_squared()
            }
            Geo::Arc { c, a0, sweep, .. } => {
                let th = det::atan2(p.y - c.y, p.x - c.x);
                let s = if *sweep >= 0.0 { 1.0 } else { -1.0 };
                let d = (th - a0) * s;
                let d = d - TWO_PI * (d / TWO_PI).floor();
                // A point at (or numerically just before) the start is the start, not a full turn.
                let d = if d >= TWO_PI - 1e-9 { 0.0 } else { d };
                d / sweep.abs()
            }
        }
    }

    /// The part of the curve between parameters `f0` and `f1`.
    fn slice(&self, f0: f64, f1: f64) -> Geo {
        match self {
            // Exact endpoints at the ends, so adjacent pieces share bit-identical coordinates.
            Geo::Line { a, b } => Geo::Line {
                a: if f0 == 0.0 { *a } else { self.at(f0) },
                b: if f1 == 1.0 { *b } else { self.at(f1) },
            },
            Geo::Arc { c, r, a0, sweep } => Geo::Arc {
                c: *c,
                r: *r,
                a0: a0 + sweep * f0,
                sweep: sweep * (f1 - f0),
            },
        }
    }

    fn reversed(&self) -> Geo {
        match self {
            Geo::Line { a, b } => Geo::Line { a: *b, b: *a },
            Geo::Arc { c, r, a0, sweep } => Geo::Arc {
                c: *c,
                r: *r,
                a0: a0 + sweep,
                sweep: -sweep,
            },
        }
    }

    fn area_term(&self) -> f64 {
        match self {
            Geo::Line { a, b } => 0.5 * (a.x * b.y - b.x * a.y),
            Geo::Arc { c, r, a0, sweep } => arc_area_term(*c, *r, *a0, *sweep),
        }
    }
}

/// Points where the *infinite* curves meet; the caller keeps those on both bounded curves.
fn candidates(g: &Geo, h: &Geo, tol: f64) -> Vec<Vec2> {
    match (g, h) {
        (Geo::Line { a: a1, b: b1 }, Geo::Line { a: a2, b: b2 }) => {
            let (d1, d2) = (*b1 - *a1, *b2 - *a2);
            let den = d1.cross(d2);
            if den.abs() <= 1e-12 * d1.length() * d2.length() {
                return vec![];
            }
            let t = (*a2 - *a1).cross(d2) / den;
            vec![*a1 + d1 * t]
        }
        (Geo::Line { a, b }, Geo::Arc { c, r, .. })
        | (Geo::Arc { c, r, .. }, Geo::Line { a, b }) => {
            let d = *b - *a;
            let l2 = d.length_squared();
            let t0 = (*c - *a).dot(d) / l2;
            let foot = *a + d * t0;
            let h = (*c - foot).length();
            if h > r + tol {
                return vec![];
            }
            let half = det::sqrt((r * r - h * h).max(0.0));
            let u = d / det::sqrt(l2);
            vec![foot - u * half, foot + u * half]
        }
        (Geo::Arc { c: c1, r: r1, .. }, Geo::Arc { c: c2, r: r2, .. }) => {
            let dv = *c2 - *c1;
            let d = dv.length();
            if d <= tol || d > r1 + r2 + tol || d < (r1 - r2).abs() - tol {
                return vec![];
            }
            let a = (r1 * r1 - r2 * r2 + d * d) / (2.0 * d);
            let h = det::sqrt((r1 * r1 - a * a).max(0.0));
            let base = *c1 + dv * (a / d);
            let perp = Vec2::new(-dv.y, dv.x) / d;
            vec![base + perp * h, base - perp * h]
        }
    }
}

fn to_dedges(profile: &Profile, prefix: &str) -> Vec<DEdge> {
    let mut out = Vec::new();
    let mut add = |lp: &Loop, hole: bool| {
        // Boundary orientation: material on the left, so holes run clockwise.
        let mut edges: Vec<LoopEdge> = lp.edges.clone();
        if hole {
            crate::profile::reverse_loop(&mut edges);
        }
        for e in &edges {
            let geo = match &e.curve {
                Curve2::Line => Geo::Line {
                    a: v2(e.from),
                    b: v2(e.to),
                },
                Curve2::Arc {
                    center,
                    radius,
                    start_angle,
                    sweep,
                } => {
                    if e.reversed {
                        Geo::Arc {
                            c: v2(*center),
                            r: *radius,
                            a0: start_angle + sweep,
                            sweep: -sweep,
                        }
                    } else {
                        Geo::Arc {
                            c: v2(*center),
                            r: *radius,
                            a0: *start_angle,
                            sweep: *sweep,
                        }
                    }
                }
                Curve2::Circle { center, radius } => Geo::Arc {
                    c: v2(*center),
                    r: *radius,
                    a0: 0.0,
                    sweep: if hole { -TWO_PI } else { TWO_PI },
                },
            };
            out.push(DEdge {
                id: format!("{prefix}{}", e.id),
                geo,
            });
        }
    };
    add(&profile.outer, false);
    for h in &profile.holes {
        add(h, true);
    }
    out
}

/// Splits `edges` wherever `others` meets them. Returns the elementary pieces, in order.
fn split(edges: &[DEdge], others: &[DEdge], tol: f64) -> Vec<DEdge> {
    let mut out = Vec::new();
    for e in edges {
        let len = e.geo.length();
        let eps = tol / len.max(tol);
        let mut params: Vec<f64> = Vec::new();
        for o in others {
            for p in candidates(&e.geo, &o.geo, tol) {
                if e.geo.contains(p, tol) && o.geo.contains(p, tol) {
                    params.push(e.geo.param(p));
                }
            }
            // Where the other curve starts or ends on this one (T-junctions, overlaps).
            for q in [o.geo.start(), o.geo.end()] {
                if e.geo.contains(q, tol) {
                    params.push(e.geo.param(q));
                }
            }
        }
        params.retain(|f| *f > eps && *f < 1.0 - eps);
        params.sort_by(f64::total_cmp);
        params.dedup_by(|a, b| (*a - *b).abs() <= eps);
        let mut cuts = vec![0.0];
        cuts.extend(params);
        cuts.push(1.0);
        let n = cuts.len() - 1;
        for k in 0..n {
            out.push(DEdge {
                id: if n > 1 {
                    format!("{}#{k}", e.id)
                } else {
                    e.id.clone()
                },
                geo: e.geo.slice(cuts[k], cuts[k + 1]),
            });
        }
    }
    out
}

#[derive(PartialEq)]
enum Rel {
    Inside,
    Outside,
    SameDirection,
    OppositeDirection,
}

/// Where a piece lies relative to the other region.
fn classify(piece: &DEdge, other_pieces: &[DEdge], other: &Prepared, tol: f64) -> Rel {
    let m = piece.geo.at(0.5);
    let t = piece.geo.tangent(0.5);
    for o in other_pieces {
        if o.geo.contains(m, tol) {
            let to = o.geo.tangent(o.geo.param(m));
            return if t.dot(to) > 0.0 {
                Rel::SameDirection
            } else {
                Rel::OppositeDirection
            };
        }
    }
    if other.contains(m) {
        Rel::Inside
    } else {
        Rel::Outside
    }
}

/// A boolean operation on two regions.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Op {
    /// `a − b`
    Subtract,
    /// `a ∪ b`
    Union,
}

/// `a − b`. Edges that come from `b` get `b_prefix` in front of their IDs so they can never
/// collide with `a`'s.
pub fn subtract(a: &Profile, b: &Profile, b_prefix: &str) -> Result<Subtracted, RegionError> {
    boolean(a, b, Op::Subtract, b_prefix)
}

/// `a ∪ b`. If the two regions neither overlap nor touch, `changed` is false and both are
/// returned unchanged.
pub fn union(a: &Profile, b: &Profile, b_prefix: &str) -> Result<Subtracted, RegionError> {
    boolean(a, b, Op::Union, b_prefix)
}

/// Do the two regions share any area? (Touching along a boundary does not count.)
pub fn overlaps(a: &Profile, b: &Profile) -> Result<bool, RegionError> {
    Ok(subtract(a, b, "")?.changed)
}

pub fn boolean(
    a: &Profile,
    b: &Profile,
    op: Op,
    b_prefix: &str,
) -> Result<Subtracted, RegionError> {
    let tol = Tolerance::DEFAULT.linear;
    let (ea, eb) = (to_dedges(a, ""), to_dedges(b, b_prefix));
    let (pa, pb) = (split(&ea, &eb, tol), split(&eb, &ea, tol));
    // Inside/outside queries are asked once per piece: build the polylines once.
    let (prep_a, prep_b) = (a.prepare(), b.prepare());

    let mut kept: Vec<DEdge> = Vec::new();
    let mut changed = false;
    for p in &pa {
        let keep = matches!(
            (op, classify(p, &pb, &prep_b, tol)),
            (_, Rel::Outside)
                | (Op::Subtract, Rel::OppositeDirection)
                | (Op::Union, Rel::SameDirection)
        );
        if keep {
            kept.push(p.clone());
        } else {
            changed = true;
        }
    }
    for p in &pb {
        let rel = classify(p, &pa, &prep_a, tol);
        match (op, rel) {
            (Op::Subtract, Rel::Inside) => {
                changed = true;
                kept.push(DEdge {
                    id: p.id.clone(),
                    geo: p.geo.reversed(),
                });
            }
            (Op::Union, Rel::Outside) => kept.push(p.clone()),
            (Op::Union, _) => changed = true,
            _ => {}
        }
    }
    if !changed {
        let profiles = match op {
            Op::Subtract => vec![a.clone()],
            Op::Union => vec![a.clone(), b.clone()],
        };
        return Ok(Subtracted {
            profiles,
            changed: false,
        });
    }
    if kept.is_empty() {
        return Ok(Subtracted {
            profiles: vec![],
            changed: true,
        });
    }
    let loops = stitch(kept)?;
    Ok(Subtracted {
        profiles: nest(loops),
        changed: true,
    })
}

/// `profile` with its boundary edges split wherever any of `others` meets them (crossings,
/// T-junctions and overlaps). The shape is unchanged; only the edge structure is refined, so two
/// regions refined against each other have identical pieces along their shared boundaries.
pub fn refine(profile: &Profile, others: &[&Profile]) -> Result<Profile, RegionError> {
    let tol = Tolerance::DEFAULT.linear;
    let mine = to_dedges(profile, "");
    let theirs: Vec<DEdge> = others.iter().flat_map(|o| to_dedges(o, "")).collect();
    let pieces = split(&mine, &theirs, tol);
    let loops = stitch(pieces)?;
    let mut profiles = nest(loops);
    if profiles.len() != 1 {
        return Err(RegionError::Unsupported(format!(
            "refining a region produced {} regions instead of 1",
            profiles.len()
        )));
    }
    Ok(profiles.remove(0))
}

fn stitch(edges: Vec<DEdge>) -> Result<Vec<RawLoop>, RegionError> {
    // Cluster endpoints into vertices.
    let mut verts: Vec<Vec2> = Vec::new();
    let vid = |p: Vec2, verts: &mut Vec<Vec2>| -> usize {
        match verts.iter().position(|q| (*q - p).length() <= MERGE) {
            Some(i) => i,
            None => {
                verts.push(p);
                verts.len() - 1
            }
        }
    };
    let ends: Vec<(usize, usize)> = edges
        .iter()
        .map(|e| (vid(e.geo.start(), &mut verts), vid(e.geo.end(), &mut verts)))
        .collect();
    let mut out_of: Vec<Vec<usize>> = vec![Vec::new(); verts.len()];
    let mut in_deg = vec![0usize; verts.len()];
    for (i, (f, t)) in ends.iter().enumerate() {
        out_of[*f].push(i);
        in_deg[*t] += 1;
    }
    for (v, outs) in out_of.iter().enumerate() {
        if outs.len() != in_deg[v] {
            return Err(RegionError::Unsupported(format!(
                "the cut leaves an open boundary near ({:.4}, {:.4})",
                verts[v].x, verts[v].y
            )));
        }
        if outs.len() > 1 {
            return Err(RegionError::Unsupported(format!(
                "the part and the cut touch at a single point near ({:.4}, {:.4}); move the cut slightly so they overlap or separate",
                verts[v].x, verts[v].y
            )));
        }
    }
    let mut used = vec![false; edges.len()];
    let mut loops = Vec::new();
    for start in 0..edges.len() {
        if used[start] {
            continue;
        }
        let mut seq = vec![start];
        used[start] = true;
        let mut cur = ends[start].1;
        while cur != ends[start].0 {
            let next = out_of[cur][0];
            if used[next] {
                return Err(RegionError::Unsupported(
                    "the boundary does not close".into(),
                ));
            }
            used[next] = true;
            seq.push(next);
            cur = ends[next].1;
        }
        loops.push(make_loop(&seq, &edges, &ends, &verts));
    }
    Ok(loops)
}

fn make_loop(seq: &[usize], edges: &[DEdge], ends: &[(usize, usize)], verts: &[Vec2]) -> RawLoop {
    let area: f64 = seq.iter().map(|&i| edges[i].geo.area_term()).sum();
    // Walk counter-clockwise: if the traced boundary is clockwise (a hole), flip it.
    let flip = area < 0.0;
    let items: Vec<(&DEdge, usize, usize)> = if flip {
        seq.iter()
            .rev()
            .map(|&i| (&edges[i], ends[i].1, ends[i].0))
            .collect()
    } else {
        seq.iter()
            .map(|&i| (&edges[i], ends[i].0, ends[i].1))
            .collect()
    };
    let mut out = Vec::new();
    for (e, f, t) in items {
        let geo = if flip {
            e.geo.reversed()
        } else {
            e.geo.clone()
        };
        let (from, to) = (verts[f], verts[t]);
        let single_circle =
            seq.len() == 1 && matches!(geo, Geo::Arc { sweep, .. } if sweep.abs() >= TWO_PI - 1e-9);
        let le = match geo {
            _ if single_circle => {
                let Geo::Arc { c, r, .. } = geo else {
                    unreachable!()
                };
                let p = [c.x + r, c.y];
                LoopEdge {
                    id: e.id.clone(),
                    reversed: false,
                    from: p,
                    to: p,
                    curve: Curve2::Circle {
                        center: [c.x, c.y],
                        radius: r,
                    },
                }
            }
            Geo::Line { .. } => LoopEdge {
                id: e.id.clone(),
                reversed: false,
                from: [from.x, from.y],
                to: [to.x, to.y],
                curve: Curve2::Line,
            },
            Geo::Arc { c, r, a0, sweep } => {
                let (start_angle, nat_sweep, reversed) = if sweep >= 0.0 {
                    (a0, sweep, false)
                } else {
                    (a0 + sweep, -sweep, true)
                };
                LoopEdge {
                    id: e.id.clone(),
                    reversed,
                    from: [from.x, from.y],
                    to: [to.x, to.y],
                    curve: Curve2::Arc {
                        center: [c.x, c.y],
                        radius: r,
                        start_angle,
                        sweep: nat_sweep,
                    },
                }
            }
        };
        out.push(le);
    }
    let min_id = out
        .iter()
        .map(|e| e.id.clone())
        .min()
        .expect("non-empty loop");
    RawLoop {
        edges: out,
        area: area.abs(),
        min_id,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Entity, Sketch};
    use core::f64::consts::PI;

    fn profile_of(build: impl FnOnce(&mut Sketch)) -> Profile {
        let mut s = Sketch::new();
        build(&mut s);
        let r = s.profiles();
        assert!(r.problems.is_empty(), "{:?}", r.problems);
        assert_eq!(
            r.profiles.len(),
            1,
            "{:?}",
            r.profiles.iter().map(|p| &p.id).collect::<Vec<_>>()
        );
        r.profiles.into_iter().next().unwrap()
    }
    fn pt(s: &mut Sketch, id: &str, x: f64, y: f64) {
        s.add_entity(id, Entity::Point { x, y }).unwrap();
    }
    fn rect(x0: f64, y0: f64, x1: f64, y1: f64, p: &'static str) -> Profile {
        profile_of(|s| {
            let c = [(x0, y0), (x1, y0), (x1, y1), (x0, y1)];
            for (k, (x, y)) in c.iter().enumerate() {
                pt(s, &format!("{p}{k}"), *x, *y);
            }
            for k in 0..4 {
                s.add_entity(
                    &format!("{p}e{k}"),
                    Entity::Line {
                        a: format!("{p}{k}"),
                        b: format!("{p}{}", (k + 1) % 4),
                    },
                )
                .unwrap();
            }
        })
    }
    fn disc(cx: f64, cy: f64, r: f64, p: &'static str) -> Profile {
        profile_of(|s| {
            pt(s, &format!("{p}c"), cx, cy);
            s.add_entity(
                &format!("{p}k"),
                Entity::Circle {
                    center: format!("{p}c"),
                    radius: r,
                },
            )
            .unwrap();
        })
    }
    fn close(a: f64, b: f64) -> bool {
        (a - b).abs() < 1e-7 * b.abs().max(1.0)
    }
    fn total(r: &Subtracted) -> f64 {
        r.profiles.iter().map(|p| p.area).sum()
    }

    /// A grid of probe points away from every boundary: the result must contain a point exactly
    /// when `a` does and `b` does not.
    fn agrees(a: &Profile, b: &Profile, r: &Subtracted) {
        agrees_with(a, b, r, |in_a, in_b| in_a && !in_b)
    }

    fn agrees_with(a: &Profile, b: &Profile, r: &Subtracted, expect: impl Fn(bool, bool) -> bool) {
        let (pa, pb) = (a.prepare(), b.prepare());
        let pr: Vec<Prepared> = r.profiles.iter().map(Profile::prepare).collect();
        // Boundary polylines, built once (they are large: sagitta 1e-5).
        let boundaries: Vec<&Vec<Vec2>> = pa.boundaries().chain(pb.boundaries()).collect();
        let (mut lo, mut hi) = (Vec2::new(f64::MAX, f64::MAX), Vec2::new(f64::MIN, f64::MIN));
        for p in boundaries.iter().flat_map(|l| l.iter()) {
            lo = Vec2::new(lo.x.min(p.x), lo.y.min(p.y));
            hi = Vec2::new(hi.x.max(p.x), hi.y.max(p.y));
        }
        // Thin the polylines for the distance test: 0.05 mm tolerance does not need 1e-5 sagitta.
        let coarse: Vec<Vec<Vec2>> = boundaries
            .iter()
            .map(|l| {
                let step = (l.len() / 400).max(1);
                l.iter().step_by(step).copied().collect()
            })
            .collect();
        let near_boundary = |q: Vec2| {
            coarse.iter().any(|pl| {
                (0..pl.len()).any(|i| dist_point_segment(q, pl[i], pl[(i + 1) % pl.len()]) < 0.05)
            })
        };
        let mut checked = 0;
        let mut y = lo.y - 1.1;
        while y < hi.y + 1.0 {
            let mut x = lo.x - 1.3;
            while x < hi.x + 1.0 {
                let q = Vec2::new(x, y);
                if !near_boundary(q) {
                    let want = expect(pa.contains(q), pb.contains(q));
                    let got = pr.iter().any(|p| p.contains(q));
                    assert_eq!(got, want, "probe ({x:.2}, {y:.2})");
                    checked += 1;
                }
                x += 0.73;
            }
            y += 0.61;
        }
        assert!(checked > 100, "only {checked} probes");
    }

    fn check(a: &Profile, b: &Profile) -> Subtracted {
        let r = subtract(a, b, "cut:").unwrap();
        agrees(a, b, &r);
        r
    }

    #[test]
    fn a_hole_inside() {
        let (a, b) = (rect(0.0, 0.0, 100.0, 60.0, "a"), disc(50.0, 30.0, 8.0, "b"));
        let r = check(&a, &b);
        assert_eq!((r.profiles.len(), r.profiles[0].holes.len()), (1, 1));
        assert!(close(total(&r), 6000.0 - 64.0 * PI), "{}", total(&r));
        assert!(r.changed);
        // The hole's edge is named after the tool, never colliding with the part's names.
        assert_eq!(r.profiles[0].holes[0].edges[0].id, "cut:bk");
    }

    #[test]
    fn a_corner_notch_makes_an_l_shape() {
        let (a, b) = (
            rect(0.0, 0.0, 100.0, 60.0, "a"),
            rect(60.0, 30.0, 120.0, 80.0, "b"),
        );
        let r = check(&a, &b);
        assert_eq!(r.profiles.len(), 1);
        assert!(close(total(&r), 6000.0 - 40.0 * 30.0), "{}", total(&r));
        assert_eq!(r.profiles[0].outer.edges.len(), 6);
        assert!(r.profiles[0].holes.is_empty());
    }

    #[test]
    fn a_slot_open_to_one_side() {
        let (a, b) = (
            rect(0.0, 0.0, 100.0, 60.0, "a"),
            rect(40.0, 40.0, 60.0, 70.0, "b"),
        );
        let r = check(&a, &b);
        assert_eq!(r.profiles.len(), 1);
        assert!(close(total(&r), 6000.0 - 20.0 * 20.0));
        assert_eq!(
            r.profiles[0].outer.edges.len(),
            8,
            "the top edge is split in two"
        );
        let ids: Vec<_> = r.profiles[0]
            .outer
            .edges
            .iter()
            .map(|e| e.id.as_str())
            .collect();
        assert!(
            ids.iter().any(|i| i.contains('#')),
            "split pieces are numbered: {ids:?}"
        );
    }

    #[test]
    fn a_cut_across_the_whole_part_splits_it_in_two() {
        let (a, b) = (
            rect(0.0, 0.0, 100.0, 60.0, "a"),
            rect(45.0, -5.0, 55.0, 65.0, "b"),
        );
        let r = check(&a, &b);
        assert_eq!(r.profiles.len(), 2);
        assert!(close(total(&r), 6000.0 - 10.0 * 60.0));
        assert!(r.profiles.iter().all(|p| p.holes.is_empty()));
    }

    #[test]
    fn a_cut_flush_with_the_boundary_is_handled() {
        // The cut's left side lies exactly on the part's left edge, and its bottom on the bottom.
        let (a, b) = (
            rect(0.0, 0.0, 100.0, 60.0, "a"),
            rect(0.0, 0.0, 30.0, 20.0, "b"),
        );
        let r = check(&a, &b);
        assert_eq!(r.profiles.len(), 1);
        assert!(close(total(&r), 6000.0 - 600.0), "{}", total(&r));
        assert_eq!(r.profiles[0].outer.edges.len(), 6);
        // A cut flush on one side only (a pocket open at the edge, sharing a boundary run).
        let b2 = rect(0.0, 20.0, 30.0, 40.0, "c");
        let r2 = check(&a, &b2);
        assert!(close(total(&r2), 6000.0 - 600.0));
    }

    #[test]
    fn a_circle_biting_an_edge_uses_line_arc_intersections() {
        let (a, b) = (
            rect(0.0, 0.0, 100.0, 60.0, "a"),
            disc(100.0, 30.0, 12.0, "b"),
        );
        let r = check(&a, &b);
        assert_eq!(r.profiles.len(), 1);
        // Half the disc is removed (the centre is on the edge).
        assert!(close(total(&r), 6000.0 - 0.5 * PI * 144.0), "{}", total(&r));
        let kinds: Vec<_> = r.profiles[0]
            .outer
            .edges
            .iter()
            .map(|e| matches!(e.curve, Curve2::Arc { .. }))
            .collect();
        assert_eq!(kinds.iter().filter(|k| **k).count(), 1, "one concave arc");
    }

    #[test]
    fn two_circles_make_a_crescent_with_exact_area() {
        let (a, b) = (disc(0.0, 0.0, 10.0, "a"), disc(10.0, 0.0, 10.0, "b"));
        let r = check(&a, &b);
        assert_eq!(r.profiles.len(), 1);
        let lens = 2.0 * 100.0 * (PI / 3.0) - 5.0 * (300.0f64).sqrt();
        assert!(
            close(total(&r), PI * 100.0 - lens),
            "{} vs {}",
            total(&r),
            PI * 100.0 - lens
        );
        assert_eq!(r.profiles[0].outer.edges.len(), 2);
    }

    #[test]
    fn the_part_has_a_hole_and_the_cut_overlaps_it() {
        let a = profile_of(|s| {
            for (k, (x, y)) in [(0.0, 0.0), (100.0, 0.0), (100.0, 60.0), (0.0, 60.0)]
                .iter()
                .enumerate()
            {
                pt(s, &format!("a{k}"), *x, *y);
            }
            for k in 0..4 {
                s.add_entity(
                    &format!("ae{k}"),
                    Entity::Line {
                        a: format!("a{k}"),
                        b: format!("a{}", (k + 1) % 4),
                    },
                )
                .unwrap();
            }
            pt(s, "m", 50.0, 30.0);
            s.add_entity(
                "hole",
                Entity::Circle {
                    center: "m".into(),
                    radius: 10.0,
                },
            )
            .unwrap();
        });
        // A bar passing through the hole and out the top: the hole merges into a slot.
        let b = rect(45.0, 25.0, 55.0, 70.0, "b");
        let r = check(&a, &b);
        assert_eq!(r.profiles.len(), 1);
        assert!(
            r.profiles[0].holes.is_empty(),
            "the hole opened into the slot"
        );
        // And a tool that only fills part of the hole changes nothing structural but the hole.
        let b2 = rect(48.0, 28.0, 52.0, 32.0, "c");
        let r2 = check(&a, &b2);
        assert!(
            !r2.changed,
            "a tool entirely inside the hole removes no material"
        );
        assert!(close(total(&r2), 6000.0 - 100.0 * PI));
    }

    #[test]
    fn a_tool_with_a_hole_leaves_an_island() {
        let a = rect(0.0, 0.0, 100.0, 100.0, "a");
        let b = profile_of(|s| {
            for (k, (x, y)) in [(10.0, 10.0), (90.0, 10.0), (90.0, 90.0), (10.0, 90.0)]
                .iter()
                .enumerate()
            {
                pt(s, &format!("b{k}"), *x, *y);
            }
            for k in 0..4 {
                s.add_entity(
                    &format!("be{k}"),
                    Entity::Line {
                        a: format!("b{k}"),
                        b: format!("b{}", (k + 1) % 4),
                    },
                )
                .unwrap();
            }
            pt(s, "m", 50.0, 50.0);
            s.add_entity(
                "island",
                Entity::Circle {
                    center: "m".into(),
                    radius: 15.0,
                },
            )
            .unwrap();
        });
        let r = check(&a, &b);
        // A frame around the border plus a disc in the middle that the tool left standing.
        assert_eq!(r.profiles.len(), 2);
        assert!(close(total(&r), 10_000.0 - 6400.0 + PI * 225.0));
    }

    #[test]
    fn disjoint_and_swallowing_tools() {
        let a = rect(0.0, 0.0, 10.0, 10.0, "a");
        let far = rect(50.0, 50.0, 60.0, 60.0, "b");
        let r = subtract(&a, &far, "cut:").unwrap();
        assert!(!r.changed);
        assert_eq!(r.profiles, vec![a.clone()]);
        let big = rect(-5.0, -5.0, 15.0, 15.0, "c");
        let r = subtract(&a, &big, "cut:").unwrap();
        assert!(
            r.changed && r.profiles.is_empty(),
            "the tool covers the whole part"
        );
    }

    #[test]
    fn touching_at_a_single_point_is_refused_not_guessed() {
        // Two squares meeting only at a corner.
        let (a, b) = (
            rect(0.0, 0.0, 10.0, 10.0, "a"),
            rect(10.0, 10.0, 20.0, 20.0, "b"),
        );
        let r = subtract(&a, &b, "cut:").unwrap();
        assert!(!r.changed || r.profiles.len() == 1);
        // A disc tangent from inside to the edge of a rectangle.
        let (a, b) = (
            rect(0.0, 0.0, 100.0, 60.0, "a"),
            disc(50.0, 30.0, 30.0, "b"),
        );
        match subtract(&a, &b, "cut:") {
            Err(RegionError::Unsupported(m)) => assert!(m.contains("touch"), "{m}"),
            Ok(r) => panic!("expected a refusal, got {} profile(s)", r.profiles.len()),
        }
    }

    #[test]
    fn the_result_is_deterministic_and_well_formed() {
        let (a, b) = (
            rect(0.0, 0.0, 100.0, 60.0, "a"),
            disc(70.0, 50.0, 20.0, "b"),
        );
        let r1 = subtract(&a, &b, "cut:").unwrap();
        let r2 = subtract(&a.clone(), &b.clone(), "cut:").unwrap();
        assert_eq!(r1, r2);
        for p in &r1.profiles {
            for l in std::iter::once(&p.outer).chain(&p.holes) {
                assert!(l.area > 0.0, "every loop is stored counter-clockwise");
                for (i, e) in l.edges.iter().enumerate() {
                    assert_eq!(
                        e.to,
                        l.edges[(i + 1) % l.edges.len()].from,
                        "adjacent edges share exact coordinates"
                    );
                }
            }
        }
        // Subtracting a tool and then checking the area identity over many positions.
        for k in 0..12 {
            let b = disc(10.0 + 7.0 * k as f64, 30.0, 9.0, "b");
            let r = subtract(&a, &b, "cut:").unwrap();
            agrees(&a, &b, &r);
        }
    }

    fn check_union(a: &Profile, b: &Profile) -> Subtracted {
        let r = union(a, b, "u:").unwrap();
        agrees_with(a, b, &r, |x, y| x || y);
        r
    }

    #[test]
    fn union_of_overlapping_regions() {
        let (a, b) = (
            rect(0.0, 0.0, 60.0, 40.0, "a"),
            rect(40.0, 20.0, 100.0, 70.0, "b"),
        );
        let r = check_union(&a, &b);
        assert!(r.changed);
        assert_eq!(r.profiles.len(), 1);
        assert!(
            close(total(&r), 2400.0 + 3000.0 - 20.0 * 20.0),
            "{}",
            total(&r)
        );
        // A circle poking out of a rectangle.
        let (a, b) = (
            rect(0.0, 0.0, 100.0, 60.0, "a"),
            disc(100.0, 30.0, 15.0, "b"),
        );
        let r = check_union(&a, &b);
        assert_eq!(r.profiles.len(), 1);
        assert!(close(total(&r), 6000.0 + 0.5 * PI * 225.0), "{}", total(&r));
    }

    #[test]
    fn union_merges_regions_that_share_an_edge() {
        // Two squares side by side sharing a full edge: the shared edge disappears.
        let (a, b) = (
            rect(0.0, 0.0, 10.0, 10.0, "a"),
            rect(10.0, 0.0, 20.0, 10.0, "b"),
        );
        let r = check_union(&a, &b);
        assert!(r.changed);
        assert_eq!(r.profiles.len(), 1);
        assert!(close(total(&r), 200.0));
        // Partially shared edge: an L made of touching rectangles.
        let (a, b) = (
            rect(0.0, 0.0, 20.0, 10.0, "a"),
            rect(10.0, 10.0, 20.0, 30.0, "b"),
        );
        let r = check_union(&a, &b);
        assert_eq!(r.profiles.len(), 1);
        assert!(close(total(&r), 200.0 + 200.0));
        // An L has six corners, but the two collinear right-hand edges (one from each rectangle)
        // are kept as separate edges on purpose: each keeps its own name for persistent faces.
        assert_eq!(r.profiles[0].outer.edges.len(), 7);
    }

    #[test]
    fn union_of_unrelated_and_nested_regions() {
        let a = rect(0.0, 0.0, 10.0, 10.0, "a");
        let far = rect(50.0, 50.0, 60.0, 60.0, "b");
        let r = union(&a, &far, "u:").unwrap();
        assert!(!r.changed);
        assert_eq!(r.profiles.len(), 2, "kept separate");
        // B entirely inside A adds nothing.
        let inner = rect(2.0, 2.0, 5.0, 5.0, "c");
        let r = check_union(&a, &inner);
        assert_eq!(r.profiles.len(), 1);
        assert!(close(total(&r), 100.0));
    }

    #[test]
    fn union_closes_a_hole_and_can_form_one() {
        // A ring plus a disc exactly filling its hole: a solid rectangle again.
        let ring = profile_of(|s| {
            for (k, (x, y)) in [(0.0, 0.0), (100.0, 0.0), (100.0, 100.0), (0.0, 100.0)]
                .iter()
                .enumerate()
            {
                pt(s, &format!("a{k}"), *x, *y);
            }
            for k in 0..4 {
                s.add_entity(
                    &format!("ae{k}"),
                    Entity::Line {
                        a: format!("a{k}"),
                        b: format!("a{}", (k + 1) % 4),
                    },
                )
                .unwrap();
            }
            pt(s, "m", 50.0, 50.0);
            s.add_entity(
                "hole",
                Entity::Circle {
                    center: "m".into(),
                    radius: 20.0,
                },
            )
            .unwrap();
        });
        let plug = disc(50.0, 50.0, 20.0, "p");
        let r = check_union(&ring, &plug);
        assert_eq!(r.profiles.len(), 1);
        assert!(r.profiles[0].holes.is_empty(), "the hole is gone");
        assert!(close(total(&r), 10_000.0));
        // A "C" closed by a bar encloses a hole.
        let c_shape = {
            let a = rect(0.0, 0.0, 60.0, 60.0, "a");
            let slot = rect(20.0, 20.0, 80.0, 40.0, "s");
            subtract(&a, &slot, "x:").unwrap().profiles.remove(0)
        };
        let bar = rect(50.0, 10.0, 60.0, 50.0, "b");
        let r = check_union(&c_shape, &bar);
        assert_eq!(r.profiles.len(), 1);
        assert_eq!(
            r.profiles[0].holes.len(),
            1,
            "the slot is now an enclosed hole"
        );
    }

    #[test]
    fn refine_splits_edges_but_does_not_change_the_shape() {
        let a = rect(0.0, 0.0, 100.0, 60.0, "a");
        let b = rect(40.0, -20.0, 70.0, 30.0, "b"); // crosses the bottom edge twice
        let r = refine(&a, &[&b]).unwrap();
        assert!(close(r.area, 6000.0));
        assert_eq!(
            r.outer.edges.len(),
            4 + 2,
            "the bottom edge is split at both crossings"
        );
        for (i, e) in r.outer.edges.iter().enumerate() {
            assert_eq!(e.to, r.outer.edges[(i + 1) % r.outer.edges.len()].from);
        }
        // Refining the other way splits the other rectangle's edges at the same two points.
        let rb = refine(&b, &[&a]).unwrap();
        assert!(close(rb.area, 750.0 * 2.0));
        assert_eq!(rb.outer.edges.len(), 4 + 2);
        // Nothing to split: the profile comes back unchanged in structure.
        let far = rect(500.0, 500.0, 510.0, 510.0, "f");
        assert_eq!(refine(&a, &[&far]).unwrap().outer.edges.len(), 4);
        // Both agree on the points where they meet.
        let xs = |p: &Profile| {
            let mut v: Vec<[i64; 2]> = p
                .outer
                .edges
                .iter()
                .map(|e| {
                    [
                        (e.from[0] * 1e6).round() as i64,
                        (e.from[1] * 1e6).round() as i64,
                    ]
                })
                .collect();
            v.sort();
            v
        };
        let (xa, xb) = (xs(&r), xs(&rb));
        for p in [[40_000_000, 0], [70_000_000, 0]] {
            assert!(xa.contains(&p) && xb.contains(&p), "{p:?}");
        }
    }
}
