//! Planar graph work in 2D: tracing the regions of a straight-line arrangement, point location
//! and interior sample points. Nodes are global `u32` IDs; their 2D positions come from a
//! caller-supplied function so every face of a solid shares the same node numbering.
//!
//! Everything iterates in sorted order (`BTreeMap`, sorted vectors), never in hash order, so the
//! result is the same on every platform (AGENTS.md §4.5). Side tests use `dc_math::robust`.

use dc_math::robust::orient2d_sign;
use dc_math::{det, Vec2};
use std::collections::BTreeMap;

pub type P2 = (f64, f64);

fn v2(p: P2) -> Vec2 {
    Vec2::new(p.0, p.1)
}

/// Twice the signed area of a polygon given by points (positive = counter-clockwise).
pub fn signed_area2(poly: &[P2]) -> f64 {
    let mut s = 0.0;
    for k in 0..poly.len() {
        let (a, b) = (poly[k], poly[(k + 1) % poly.len()]);
        s += a.0 * b.1 - b.0 * a.1;
    }
    s
}

/// Is `p` inside the even-odd region bounded by `loops`? Exact side tests; points on the boundary
/// give an arbitrary answer (callers test for the boundary separately).
pub fn inside_even_odd(loops: &[Vec<P2>], p: P2) -> bool {
    let mut inside = false;
    for lp in loops {
        for k in 0..lp.len() {
            let (a, b) = (lp[k], lp[(k + 1) % lp.len()]);
            if (a.1 > p.1) != (b.1 > p.1) {
                let s = orient2d_sign(v2(a), v2(b), v2(p));
                if (b.1 > a.1 && s > 0) || (b.1 < a.1 && s < 0) {
                    inside = !inside;
                }
            }
        }
    }
    inside
}

/// Distance from `p` to the segment `a..b`.
pub fn dist_to_segment(p: P2, a: P2, b: P2) -> f64 {
    let (dx, dy) = (b.0 - a.0, b.1 - a.1);
    let len2 = dx * dx + dy * dy;
    let t = if len2 > 0.0 {
        (((p.0 - a.0) * dx + (p.1 - a.1) * dy) / len2).clamp(0.0, 1.0)
    } else {
        0.0
    };
    let (qx, qy) = (a.0 + t * dx - p.0, a.1 + t * dy - p.1);
    det::sqrt(qx * qx + qy * qy)
}

/// A point strictly inside the region bounded by `loops` (outer boundary plus holes, even-odd).
/// Uses a horizontal scan line through the middle of the tallest gap between vertex heights, so
/// it never passes through a vertex, and returns the middle of the widest interior interval.
pub fn interior_point(loops: &[Vec<P2>]) -> Option<P2> {
    let mut ys: Vec<f64> = loops.iter().flatten().map(|p| p.1).collect();
    ys.sort_by(f64::total_cmp);
    ys.dedup();
    let mut best_gap = (0.0, 0.0);
    for w in ys.windows(2) {
        let g = w[1] - w[0];
        if g > best_gap.0 {
            best_gap = (g, 0.5 * (w[0] + w[1]));
        }
    }
    if best_gap.0 <= 0.0 {
        return None;
    }
    let y = best_gap.1;
    let mut xs = Vec::new();
    for lp in loops {
        for k in 0..lp.len() {
            let (a, b) = (lp[k], lp[(k + 1) % lp.len()]);
            if (a.1 > y) != (b.1 > y) {
                xs.push(a.0 + (y - a.1) / (b.1 - a.1) * (b.0 - a.0));
            }
        }
    }
    xs.sort_by(f64::total_cmp);
    let mut best: Option<(f64, f64)> = None;
    for pair in xs.chunks_exact(2) {
        let w = pair[1] - pair[0];
        if best.is_none_or(|(bw, _)| w > bw) {
            best = Some((w, 0.5 * (pair[0] + pair[1])));
        }
    }
    best.filter(|(w, _)| *w > 0.0).map(|(_, x)| (x, y))
}

/// An undirected edge of an arrangement: end nodes `a` and `b` (node keys) and the points of
/// its polyline strictly between them, in order from `a` to `b`. A closed edge has `a == b`.
#[derive(Clone, Debug, PartialEq)]
pub struct AEdge {
    pub a: u32,
    pub b: u32,
    pub mid: Vec<P2>,
}

/// A directed use of an edge: `(edge index, forward)`.
pub type HalfEdge = (usize, bool);

/// A bounded region of the arrangement: its outer boundary (counter-clockwise) and the boundaries
/// of components lying inside it (clockwise), each as a cycle of directed edges.
#[derive(Clone, Debug, PartialEq)]
pub struct Region {
    pub outer: Vec<HalfEdge>,
    pub holes: Vec<Vec<HalfEdge>>,
}

struct Cycle {
    edges: Vec<HalfEdge>,
    poly: Vec<P2>,
    area2: f64,
    component: u32,
}

/// The bounded regions of the planar graph made of `edges`, whose positions are `pos(node)` and
/// whose interiors are the polylines in `mid`. The edges must not cross except at nodes.
/// Dangling edges (ending at a node with no other edge) are ignored.
pub fn regions(pos: &dyn Fn(u32) -> P2, edges: &[AEdge]) -> Vec<Region> {
    // Degree of each node, pruning dangling edges.
    let mut alive: Vec<bool> = vec![true; edges.len()];
    loop {
        let mut deg: BTreeMap<u32, u32> = BTreeMap::new();
        for (i, e) in edges.iter().enumerate() {
            if alive[i] {
                *deg.entry(e.a).or_insert(0) += 1;
                *deg.entry(e.b).or_insert(0) += 1;
            }
        }
        let mut changed = false;
        for (i, e) in edges.iter().enumerate() {
            if alive[i] && (deg[&e.a] < 2 || deg[&e.b] < 2) {
                alive[i] = false;
                changed = true;
            }
        }
        if !changed {
            break;
        }
    }
    // The polyline of each edge, from a to b.
    let polys: Vec<Vec<P2>> = edges
        .iter()
        .map(|e| {
            let mut v = vec![pos(e.a)];
            v.extend(e.mid.iter().copied());
            v.push(pos(e.b));
            v
        })
        .collect();
    let start_of = |h: HalfEdge| if h.1 { edges[h.0].a } else { edges[h.0].b };
    let end_of = |h: HalfEdge| if h.1 { edges[h.0].b } else { edges[h.0].a };
    let dir_of = |h: HalfEdge| -> f64 {
        let p = &polys[h.0];
        let (from, to) = if h.1 {
            (p[0], p[1])
        } else {
            (p[p.len() - 1], p[p.len() - 2])
        };
        det::atan2(to.1 - from.1, to.0 - from.0)
    };
    // Outgoing half-edges around each node, counter-clockwise.
    let mut around: BTreeMap<u32, Vec<HalfEdge>> = BTreeMap::new();
    for i in (0..edges.len()).filter(|&i| alive[i]) {
        around
            .entry(start_of((i, true)))
            .or_default()
            .push((i, true));
        around
            .entry(start_of((i, false)))
            .or_default()
            .push((i, false));
    }
    for hs in around.values_mut() {
        hs.sort_by(|&x, &y| dir_of(x).total_cmp(&dir_of(y)).then(x.cmp(&y)));
    }
    // Connected components by node.
    let mut comp: BTreeMap<u32, u32> = BTreeMap::new();
    for &s in around.keys() {
        if comp.contains_key(&s) {
            continue;
        }
        comp.insert(s, s);
        let mut stack = vec![s];
        while let Some(u) = stack.pop() {
            for &h in &around[&u] {
                let v = end_of(h);
                if let std::collections::btree_map::Entry::Vacant(e) = comp.entry(v) {
                    e.insert(s);
                    stack.push(v);
                }
            }
        }
    }
    // Trace the face on the left of every half-edge.
    let mut used: BTreeMap<HalfEdge, bool> = BTreeMap::new();
    for hs in around.values() {
        for &h in hs {
            used.insert(h, false);
        }
    }
    let keys: Vec<HalfEdge> = used.keys().copied().collect();
    let mut cycles: Vec<Cycle> = Vec::new();
    for start in keys {
        if used[&start] {
            continue;
        }
        let mut hs = Vec::new();
        let mut h = start;
        loop {
            used.insert(h, true);
            hs.push(h);
            let v = end_of(h);
            let twin = (h.0, !h.1);
            let list = &around[&v];
            let at = list
                .iter()
                .position(|&x| x == twin)
                .expect("twin is listed");
            h = list[(at + list.len() - 1) % list.len()];
            if h == start || used[&h] {
                break;
            }
        }
        let mut poly: Vec<P2> = Vec::new();
        for &(i, fwd) in &hs {
            let p = &polys[i];
            if fwd {
                poly.extend(p[..p.len() - 1].iter().copied());
            } else {
                poly.extend(p[1..].iter().rev().copied());
            }
        }
        let component = comp[&start_of(hs[0])];
        cycles.push(Cycle {
            area2: signed_area2(&poly),
            edges: hs,
            poly,
            component,
        });
    }
    let bounded: Vec<usize> = (0..cycles.len())
        .filter(|&i| cycles[i].area2 > 0.0)
        .collect();
    let mut out: Vec<Region> = bounded
        .iter()
        .map(|&i| Region {
            outer: cycles[i].edges.clone(),
            holes: Vec::new(),
        })
        .collect();
    // Each component's outer boundary is a hole of the smallest region of another component that
    // contains it.
    for c in cycles.iter().filter(|c| c.area2 < 0.0) {
        let probe = c.poly[0];
        let mut best: Option<(f64, usize)> = None;
        for (slot, &i) in bounded.iter().enumerate() {
            let r = &cycles[i];
            if r.component == c.component {
                continue;
            }
            if inside_even_odd(std::slice::from_ref(&r.poly), probe)
                && best.is_none_or(|(a, _)| r.area2 < a)
            {
                best = Some((r.area2, slot));
            }
        }
        if let Some((_, slot)) = best {
            out[slot].holes.push(c.edges.clone());
        }
    }
    out
}

/// The polyline of a cycle of directed edges (each edge's points once).
pub fn cycle_points(edges: &[AEdge], pos: &dyn Fn(u32) -> P2, cycle: &[HalfEdge]) -> Vec<P2> {
    let mut poly = Vec::new();
    for &(i, fwd) in cycle {
        let e = &edges[i];
        let mut p = vec![pos(e.a)];
        p.extend(e.mid.iter().copied());
        p.push(pos(e.b));
        if fwd {
            poly.extend(p[..p.len() - 1].iter().copied());
        } else {
            poly.extend(p[1..].iter().rev().copied());
        }
    }
    poly
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pos_of(pts: &[P2]) -> impl Fn(u32) -> P2 + '_ {
        move |n| pts[n as usize]
    }

    fn e(a: u32, b: u32) -> AEdge {
        AEdge { a, b, mid: vec![] }
    }

    #[test]
    fn a_square_with_a_diagonal_has_two_triangles() {
        let pts = [(0.0, 0.0), (4.0, 0.0), (4.0, 4.0), (0.0, 4.0)];
        let edges = [e(0, 1), e(1, 2), e(2, 3), e(3, 0), e(0, 2)];
        let r = regions(&pos_of(&pts), &edges);
        assert_eq!(r.len(), 2);
        assert!(r.iter().all(|x| x.outer.len() == 3 && x.holes.is_empty()));
    }

    #[test]
    fn a_curved_edge_splits_a_square_into_a_bulge_and_the_rest() {
        let pts = [(0.0, 0.0), (4.0, 0.0), (4.0, 4.0), (0.0, 4.0)];
        let curved = AEdge {
            a: 0,
            b: 2,
            mid: vec![(2.0, 0.5), (3.5, 1.5)],
        };
        let edges = [e(0, 1), e(1, 2), e(2, 3), e(3, 0), curved];
        let f = pos_of(&pts);
        let r = regions(&f, &edges);
        assert_eq!(r.len(), 2);
        let mut areas: Vec<f64> = r
            .iter()
            .map(|x| signed_area2(&cycle_points(&edges, &f, &x.outer)) / 2.0)
            .collect();
        areas.sort_by(f64::total_cmp);
        assert!(areas[0] < 8.0 && areas[1] > 8.0, "{areas:?}");
        assert!((areas[0] + areas[1] - 16.0).abs() < 1e-12);
    }

    #[test]
    fn two_edges_between_the_same_nodes_make_one_lens() {
        let pts = [(0.0, 0.0), (4.0, 0.0)];
        let up = AEdge {
            a: 0,
            b: 1,
            mid: vec![(2.0, 1.0)],
        };
        let down = AEdge {
            a: 0,
            b: 1,
            mid: vec![(2.0, -1.0)],
        };
        let r = regions(&pos_of(&pts), &[up, down]);
        assert_eq!(r.len(), 1, "one bounded lens");
        assert_eq!(r[0].outer.len(), 2);
    }

    #[test]
    fn a_closed_edge_inside_a_square_is_a_hole_of_the_outer_region() {
        let pts = [
            (0.0, 0.0),
            (10.0, 0.0),
            (10.0, 10.0),
            (0.0, 10.0),
            (5.0, 4.0),
        ];
        let ring = AEdge {
            a: 4,
            b: 4,
            mid: vec![(6.0, 5.0), (5.0, 6.0), (4.0, 5.0)],
        };
        let edges = [e(0, 1), e(1, 2), e(2, 3), e(3, 0), ring];
        let f = pos_of(&pts);
        let r = regions(&f, &edges);
        assert_eq!(r.len(), 2, "the square with a hole, and the disc");
        let outer = r
            .iter()
            .find(|x| x.outer.len() == 4)
            .expect("square region");
        assert_eq!(outer.holes.len(), 1);
        let disc = r.iter().find(|x| x.outer.len() == 1).expect("disc region");
        assert!(disc.holes.is_empty());
    }

    #[test]
    fn dangling_edges_are_ignored() {
        let pts = [(0.0, 0.0), (4.0, 0.0), (4.0, 4.0), (0.0, 4.0), (2.0, 2.0)];
        let edges = [e(0, 1), e(1, 2), e(2, 3), e(3, 0), e(0, 4)];
        let r = regions(&pos_of(&pts), &edges);
        assert_eq!(r.len(), 1);
        assert_eq!(r[0].outer.len(), 4);
    }
}
