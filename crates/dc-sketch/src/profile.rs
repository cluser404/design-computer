//! Closed profiles: the regions of a sketch that can be turned into solids.
//!
//! A profile is a closed loop of lines and arcs (or a circle) plus the loops nested inside it,
//! which become holes. Loops are found through shared points (points joined by a `Coincident`
//! constraint count as one). Anything that is not a clean closed loop is reported as a
//! [`ProfileProblem`] naming the entities involved, never silently dropped or guessed at.
//!
//! Current restriction (by design, for now): a profile must be a *simple cycle*. A rectangle with a
//! diagonal is reported as a branch instead of being split into two triangles.

use crate::model::{Constraint, Entity, Sketch};
use dc_math::{det, Tolerance, Vec2};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};

const TWO_PI: f64 = 2.0 * core::f64::consts::PI;
/// Sagitta used only for point-in-polygon containment tests, in mm.
const CONTAINMENT_SAGITTA: f64 = 1e-5;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Curve2 {
    /// Straight segment from `from` to `to` (see [`LoopEdge`]).
    Line,
    /// Circular arc, counter-clockwise from `start_angle` by `sweep`, in its natural direction.
    Arc {
        center: [f64; 2],
        radius: f64,
        start_angle: f64,
        sweep: f64,
    },
    Circle {
        center: [f64; 2],
        radius: f64,
    },
}

/// One edge of a loop, in traversal order.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct LoopEdge {
    /// The sketch entity this edge comes from.
    pub id: String,
    /// True if the loop runs against the entity's natural direction (arc: clockwise).
    pub reversed: bool,
    /// Endpoints in traversal direction. Adjacent edges share exactly equal coordinates.
    pub from: [f64; 2],
    pub to: [f64; 2],
    pub curve: Curve2,
}

/// A closed loop, always stored **counter-clockwise** (positive area). Holes are CCW too; the
/// consumer reverses them as needed.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct Loop {
    pub edges: Vec<LoopEdge>,
    pub area: f64,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct Profile {
    /// Stable while the loop's lowest-ID edge exists: `profile.<edge id>`.
    pub id: String,
    pub outer: Loop,
    pub holes: Vec<Loop>,
    /// Outer area minus hole areas.
    pub area: f64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum ProfileProblemCode {
    /// A chain of edges that does not close: this point is an unconnected end.
    OpenEnd,
    /// Three or more edges meet at one point, so the boundary is ambiguous.
    Branch,
    /// A zero-length line, an arc with coincident ends, or a loop with no area.
    Degenerate,
    /// Two edges cross or touch without a shared point.
    Intersection,
    /// Two edges lie on top of each other.
    Overlap,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct ProfileProblem {
    pub code: ProfileProblemCode,
    pub message: String,
    /// Sketch entity IDs involved (points for open ends and branches, edges otherwise).
    pub entities: Vec<String>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct ProfileReport {
    pub profiles: Vec<Profile>,
    pub problems: Vec<ProfileProblem>,
}

// ---------------------------------------------------------------------------------------------
// Edge geometry
// ---------------------------------------------------------------------------------------------

#[derive(Clone, Debug)]
enum G {
    Line {
        a: Vec2,
        b: Vec2,
    },
    Arc {
        c: Vec2,
        r: f64,
        t0: f64,
        sweep: f64,
    },
    Circle {
        c: Vec2,
        r: f64,
    },
}

struct Edge {
    id: String,
    /// Graph nodes at the natural start and end (equal for nothing: circles are not in the graph).
    from: usize,
    to: usize,
    g: G,
}

fn v(p: (f64, f64)) -> Vec2 {
    Vec2::new(p.0, p.1)
}

fn angle(c: Vec2, p: Vec2) -> f64 {
    det::atan2(p.y - c.y, p.x - c.x)
}

/// Signed area contribution ½∮(x dy − y dx) of an edge traversed in its natural direction.
fn area_term(g: &G) -> f64 {
    match g {
        G::Line { a, b } => 0.5 * (a.x * b.y - b.x * a.y),
        G::Arc {
            c, r, t0, sweep, ..
        } => arc_area_term(*c, *r, *t0, *sweep),
        G::Circle { c, r } => arc_area_term(*c, *r, 0.0, TWO_PI),
    }
}

pub(crate) fn arc_area_term(c: Vec2, r: f64, t0: f64, sweep: f64) -> f64 {
    let t1 = t0 + sweep;
    0.5 * (r * r * sweep
        + r * (c.x * (det::sin(t1) - det::sin(t0)) - c.y * (det::cos(t1) - det::cos(t0))))
}

pub(crate) fn in_sweep(theta: f64, t0: f64, sweep: f64, eps: f64) -> bool {
    let d = theta - t0;
    let d = d - TWO_PI * (d / TWO_PI).floor();
    d <= sweep + eps || d >= TWO_PI - eps
}

pub(crate) fn dist_point_segment(p: Vec2, a: Vec2, b: Vec2) -> f64 {
    let ab = b - a;
    let l2 = ab.length_squared();
    let t = if l2 == 0.0 {
        0.0
    } else {
        ((p - a).dot(ab) / l2).clamp(0.0, 1.0)
    };
    (p - (a + ab * t)).length()
}

fn segments_touch(a: Vec2, b: Vec2, c: Vec2, d: Vec2, tol: f64) -> bool {
    let o = |p: Vec2, q: Vec2, r: Vec2| (q - p).cross(r - p);
    let (d1, d2, d3, d4) = (o(a, b, c), o(a, b, d), o(c, d, a), o(c, d, b));
    if ((d1 > 0.0) != (d2 > 0.0)) && ((d3 > 0.0) != (d4 > 0.0)) && d1 != 0.0 && d2 != 0.0 {
        return true; // proper crossing
    }
    dist_point_segment(a, c, d) <= tol
        || dist_point_segment(b, c, d) <= tol
        || dist_point_segment(c, a, b) <= tol
        || dist_point_segment(d, a, b) <= tol
}

/// Arc as (center, radius, start angle, sweep); circles use sweep 2π.
fn as_arc(g: &G) -> Option<(Vec2, f64, f64, f64)> {
    match g {
        G::Arc {
            c, r, t0, sweep, ..
        } => Some((*c, *r, *t0, *sweep)),
        G::Circle { c, r } => Some((*c, *r, 0.0, TWO_PI)),
        G::Line { .. } => None,
    }
}

fn segment_arc_touch(a: Vec2, b: Vec2, arc: (Vec2, f64, f64, f64), tol: f64) -> bool {
    let (c, r, t0, sweep) = arc;
    let ab = b - a;
    let l2 = ab.length_squared();
    if l2 == 0.0 {
        return false;
    }
    let t_foot = (c - a).dot(ab) / l2;
    let foot = a + ab * t_foot;
    let h = (c - foot).length();
    if h > r + tol {
        return false;
    }
    let half = det::sqrt((r * r - h * h).max(0.0));
    let len = det::sqrt(l2);
    let dt = half / len;
    let eps_t = tol / len;
    let eps_a = tol / r.max(tol);
    [t_foot - dt, t_foot + dt].iter().any(|&t| {
        t >= -eps_t && t <= 1.0 + eps_t && {
            let p = a + ab * t;
            in_sweep(angle(c, p), t0, sweep, eps_a)
        }
    })
}

fn arcs_touch(a1: (Vec2, f64, f64, f64), a2: (Vec2, f64, f64, f64), tol: f64) -> bool {
    let ((c1, r1, t1, s1), (c2, r2, t2, s2)) = (a1, a2);
    let d_vec = c2 - c1;
    let d = d_vec.length();
    if d <= tol && (r1 - r2).abs() <= tol {
        // Same circle: do the angular ranges overlap or touch?
        let (e1, e2) = (tol / r1.max(tol), tol / r2.max(tol));
        return in_sweep(t2, t1, s1, e1)
            || in_sweep(t2 + s2, t1, s1, e1)
            || in_sweep(t1, t2, s2, e2)
            || in_sweep(t1 + s1, t2, s2, e2);
    }
    if d > r1 + r2 + tol || d < (r1 - r2).abs() - tol {
        return false;
    }
    let a = (r1 * r1 - r2 * r2 + d * d) / (2.0 * d);
    let h = det::sqrt((r1 * r1 - a * a).max(0.0));
    let base = c1 + d_vec * (a / d);
    let perp = Vec2::new(-d_vec.y, d_vec.x) / d;
    let (ea1, ea2) = (tol / r1.max(tol), tol / r2.max(tol));
    [base + perp * h, base - perp * h]
        .iter()
        .any(|&p| in_sweep(angle(c1, p), t1, s1, ea1) && in_sweep(angle(c2, p), t2, s2, ea2))
}

fn edges_touch(a: &G, b: &G, tol: f64) -> bool {
    match (a, b) {
        (G::Line { a: p, b: q }, G::Line { a: r, b: s }) => segments_touch(*p, *q, *r, *s, tol),
        (G::Line { a: p, b: q }, other) | (other, G::Line { a: p, b: q }) => {
            segment_arc_touch(*p, *q, as_arc(other).expect("not a line"), tol)
        }
        (x, y) => arcs_touch(as_arc(x).expect("arc"), as_arc(y).expect("arc"), tol),
    }
}

/// Polyline of a loop for containment tests only (not used for geometry).
pub(crate) fn polyline(edges: &[LoopEdge]) -> Vec<Vec2> {
    let mut pts = Vec::new();
    for e in edges {
        pts.push(Vec2::new(e.from[0], e.from[1]));
        let (c, r, t0, sweep, rev) = match &e.curve {
            Curve2::Line => continue,
            Curve2::Arc {
                center,
                radius,
                start_angle,
                sweep,
            } => (
                Vec2::new(center[0], center[1]),
                *radius,
                *start_angle,
                *sweep,
                e.reversed,
            ),
            Curve2::Circle { center, radius } => (
                Vec2::new(center[0], center[1]),
                *radius,
                0.0,
                TWO_PI,
                e.reversed,
            ),
        };
        let step = 2.0 * det::acos((1.0 - CONTAINMENT_SAGITTA / r).clamp(-1.0, 1.0)).max(1e-3);
        let n = (sweep / step).ceil().max(2.0) as usize;
        for i in 1..n {
            let f = i as f64 / n as f64;
            let t = if rev {
                t0 + sweep * (1.0 - f)
            } else {
                t0 + sweep * f
            };
            pts.push(Vec2::new(c.x + r * det::cos(t), c.y + r * det::sin(t)));
        }
    }
    pts
}

fn inside(poly: &[Vec2], p: Vec2) -> bool {
    let mut odd = false;
    for i in 0..poly.len() {
        let (a, b) = (poly[i], poly[(i + 1) % poly.len()]);
        if (a.y > p.y) != (b.y > p.y) {
            let x = a.x + (p.y - a.y) * (b.x - a.x) / (b.y - a.y);
            if p.x < x {
                odd = !odd;
            }
        }
    }
    odd
}

// ---------------------------------------------------------------------------------------------
// Union-find over points joined by Coincident constraints
// ---------------------------------------------------------------------------------------------

struct Nodes {
    parent: BTreeMap<String, String>,
}

impl Nodes {
    fn new(s: &Sketch) -> Self {
        let mut parent: BTreeMap<String, String> = s
            .entities
            .iter()
            .filter(|(_, e)| matches!(e, Entity::Point { .. }))
            .map(|(k, _)| (k.clone(), k.clone()))
            .collect();
        for c in s.constraints.values() {
            if let Constraint::Coincident { a, b } = c {
                let (ra, rb) = (Self::find(&mut parent, a), Self::find(&mut parent, b));
                if ra != rb {
                    // The smaller ID is the class representative, so naming is deterministic.
                    let (lo, hi) = if ra < rb { (ra, rb) } else { (rb, ra) };
                    parent.insert(hi, lo);
                }
            }
        }
        Nodes { parent }
    }

    fn find(parent: &mut BTreeMap<String, String>, x: &str) -> String {
        let mut cur = x.to_owned();
        while parent[&cur] != cur {
            cur = parent[&cur].clone();
        }
        cur
    }

    fn rep(&self, x: &str) -> String {
        let mut cur = x.to_owned();
        while self.parent[&cur] != cur {
            cur = self.parent[&cur].clone();
        }
        cur
    }
}

// ---------------------------------------------------------------------------------------------
// Profile extraction
// ---------------------------------------------------------------------------------------------

/// A closed loop found by tracing or by a boolean, before nesting: counter-clockwise, positive area.
pub(crate) struct RawLoop {
    pub edges: Vec<LoopEdge>,
    pub area: f64,
    pub min_id: String,
}

/// Reverses a loop's direction: opposite order, every edge flipped.
pub(crate) fn reverse_loop(edges: &mut [LoopEdge]) {
    edges.reverse();
    for e in edges.iter_mut() {
        e.reversed = !e.reversed;
        core::mem::swap(&mut e.from, &mut e.to);
    }
}

/// Groups counter-clockwise loops into profiles: depth = number of loops containing a loop;
/// even depth is an outer boundary, odd depth is a hole of its innermost container.
pub(crate) fn nest(mut loops: Vec<RawLoop>) -> Vec<Profile> {
    loops.sort_by(|a, b| a.min_id.cmp(&b.min_id));
    let polys: Vec<Vec<Vec2>> = loops.iter().map(|l| polyline(&l.edges)).collect();
    let n = loops.len();
    let contains = |outer: usize, inner: usize| {
        outer != inner && {
            let f = &loops[inner].edges[0].from;
            inside(&polys[outer], Vec2::new(f[0], f[1]))
        }
    };
    let containers: Vec<Vec<usize>> = (0..n)
        .map(|i| (0..n).filter(|&o| contains(o, i)).collect())
        .collect();
    let mut profiles: Vec<Profile> = Vec::new();
    let mut profile_of_loop: BTreeMap<usize, usize> = BTreeMap::new();
    for i in 0..n {
        if containers[i].len().is_multiple_of(2) {
            profile_of_loop.insert(i, profiles.len());
            let outer = Loop {
                edges: loops[i].edges.clone(),
                area: loops[i].area,
            };
            profiles.push(Profile {
                id: format!("profile.{}", loops[i].min_id),
                area: outer.area,
                outer,
                holes: Vec::new(),
            });
        }
    }
    for i in 0..n {
        if !containers[i].len().is_multiple_of(2) {
            // Parent: the innermost container (smallest area).
            let parent = *containers[i]
                .iter()
                .min_by(|&&a, &&b| loops[a].area.total_cmp(&loops[b].area))
                .expect("odd depth has a container");
            let pi = profile_of_loop[&parent];
            profiles[pi].area -= loops[i].area;
            profiles[pi].holes.push(Loop {
                edges: loops[i].edges.clone(),
                area: loops[i].area,
            });
        }
    }
    profiles.sort_by(|a, b| a.id.cmp(&b.id));
    profiles
}

/// A profile's boundaries as fine polylines, built once for repeated inside/outside queries.
pub struct Prepared {
    outer: Vec<Vec2>,
    holes: Vec<Vec<Vec2>>,
}

impl Prepared {
    /// Is `p` inside the material (inside the outer loop, outside every hole)?
    pub fn contains(&self, p: Vec2) -> bool {
        inside(&self.outer, p) && !self.holes.iter().any(|h| inside(h, p))
    }

    /// Every boundary polyline, outer first.
    pub fn boundaries(&self) -> impl Iterator<Item = &Vec<Vec2>> {
        std::iter::once(&self.outer).chain(self.holes.iter())
    }
}

impl Profile {
    /// Builds the polylines for repeated [`Prepared::contains`] queries.
    pub fn prepare(&self) -> Prepared {
        Prepared {
            outer: polyline(&self.outer.edges),
            holes: self.holes.iter().map(|h| polyline(&h.edges)).collect(),
        }
    }

    /// Is `p` inside the material of this profile? Builds polylines on every call: for many
    /// queries use [`Profile::prepare`] once.
    pub fn contains(&self, p: Vec2) -> bool {
        self.prepare().contains(p)
    }
}

impl Sketch {
    /// Finds the closed profiles of this sketch and everything that prevents other geometry from
    /// forming one. Deterministic: profiles are ordered by their ID.
    pub fn profiles(&self) -> ProfileReport {
        let tol = Tolerance::DEFAULT.linear;
        let nodes = Nodes::new(self);
        let mut problems = Vec::new();

        // Node indices by representative point ID, and their positions.
        let mut node_index: BTreeMap<String, usize> = BTreeMap::new();
        let mut node_pos: Vec<Vec2> = Vec::new();
        let mut node_members: Vec<Vec<String>> = Vec::new();
        for id in self.entities.keys() {
            if self.point(id).is_none() {
                continue;
            }
            let rep = nodes.rep(id);
            let idx = *node_index.entry(rep.clone()).or_insert_with(|| {
                node_pos.push(v(self.point(&rep).expect("representative is a point")));
                node_members.push(Vec::new());
                node_pos.len() - 1
            });
            node_members[idx].push(id.clone());
        }
        let node_of = |p: &str| node_index[&nodes.rep(p)];

        // Edges (in ID order) and circles.
        let mut edges: Vec<Edge> = Vec::new();
        let mut circles: Vec<(String, Vec2, f64)> = Vec::new();
        for (id, e) in &self.entities {
            if self.construction.contains(id) {
                continue;
            }
            match e {
                Entity::Line { a, b } => {
                    let (pa, pb) = (
                        v(self.point(a).expect("point")),
                        v(self.point(b).expect("point")),
                    );
                    let (na, nb) = (node_of(a), node_of(b));
                    if na == nb || (pb - pa).length() <= tol {
                        problems.push(ProfileProblem {
                            code: ProfileProblemCode::Degenerate,
                            message: format!("line '{id}' has zero length"),
                            entities: vec![id.clone()],
                        });
                    } else {
                        edges.push(Edge {
                            id: id.clone(),
                            from: na,
                            to: nb,
                            g: G::Line { a: pa, b: pb },
                        });
                    }
                }
                Entity::Arc { center, start, end } => {
                    let c = v(self.point(center).expect("point"));
                    let (s, en) = (
                        v(self.point(start).expect("point")),
                        v(self.point(end).expect("point")),
                    );
                    let r = (s - c).length();
                    let (ns, ne) = (node_of(start), node_of(end));
                    if ns == ne || (en - s).length() <= tol || r <= tol {
                        problems.push(ProfileProblem {
                            code: ProfileProblemCode::Degenerate,
                            message: format!("arc '{id}' has coincident ends or no radius"),
                            entities: vec![id.clone()],
                        });
                    } else {
                        let t0 = angle(c, s);
                        let mut sweep = angle(c, en) - t0;
                        while sweep <= 0.0 {
                            sweep += TWO_PI;
                        }
                        edges.push(Edge {
                            id: id.clone(),
                            from: ns,
                            to: ne,
                            g: G::Arc { c, r, t0, sweep },
                        });
                    }
                }
                Entity::Circle { center, radius } => {
                    circles.push((id.clone(), v(self.point(center).expect("point")), *radius));
                }
                Entity::Point { .. } => {}
            }
        }

        // Graph analysis: components, degrees, open ends, branches.
        let mut incident: Vec<Vec<usize>> = vec![Vec::new(); node_pos.len()];
        for (i, e) in edges.iter().enumerate() {
            incident[e.from].push(i);
            incident[e.to].push(i);
        }
        let mut comp_of_edge = vec![usize::MAX; edges.len()];
        let mut comps: Vec<Vec<usize>> = Vec::new();
        for start in 0..edges.len() {
            if comp_of_edge[start] != usize::MAX {
                continue;
            }
            let k = comps.len();
            let mut stack = vec![start];
            let mut members = Vec::new();
            comp_of_edge[start] = k;
            while let Some(i) = stack.pop() {
                members.push(i);
                for n in [edges[i].from, edges[i].to] {
                    for &j in &incident[n] {
                        if comp_of_edge[j] == usize::MAX {
                            comp_of_edge[j] = k;
                            stack.push(j);
                        }
                    }
                }
            }
            members.sort_unstable();
            comps.push(members);
        }

        let mut cycles: Vec<Vec<usize>> = Vec::new();
        for members in &comps {
            let mut ns: BTreeSet<usize> = BTreeSet::new();
            for &i in members {
                ns.insert(edges[i].from);
                ns.insert(edges[i].to);
            }
            let mut clean = true;
            for &n in &ns {
                let deg = incident[n].len();
                if deg != 2 {
                    clean = false;
                    let (code, what) = if deg == 1 {
                        (
                            ProfileProblemCode::OpenEnd,
                            "is an open end (the chain does not close)",
                        )
                    } else {
                        (
                            ProfileProblemCode::Branch,
                            "has three or more edges meeting; the boundary is ambiguous",
                        )
                    };
                    problems.push(ProfileProblem {
                        code,
                        message: format!("point '{}' {what}", node_members[n][0]),
                        entities: node_members[n].clone(),
                    });
                }
            }
            if clean {
                cycles.push(members.clone());
            }
        }

        // Overlaps and crossings among edges that belong to clean cycles.
        let cyc_edges: Vec<usize> = cycles.iter().flatten().copied().collect();
        let mut bad_edges: BTreeSet<usize> = BTreeSet::new();
        let flag = |problems: &mut Vec<ProfileProblem>, code, msg: String, a: &str, b: &str| {
            problems.push(ProfileProblem {
                code,
                message: msg,
                entities: vec![a.to_owned(), b.to_owned()],
            });
        };
        for (x, &i) in cyc_edges.iter().enumerate() {
            for &j in &cyc_edges[x + 1..] {
                let (ei, ej) = (&edges[i], &edges[j]);
                let shared: Vec<usize> = [ei.from, ei.to]
                    .iter()
                    .filter(|n| **n == ej.from || **n == ej.to)
                    .copied()
                    .collect();
                if shared.is_empty() {
                    if edges_touch(&ei.g, &ej.g, tol) {
                        flag(&mut problems, ProfileProblemCode::Intersection,
                            format!("'{}' and '{}' cross or touch without a shared point (add a coincident constraint or move one)", ei.id, ej.id),
                            &ei.id, &ej.id);
                        bad_edges.insert(i);
                        bad_edges.insert(j);
                    }
                } else if let (G::Line { a: a1, b: b1 }, G::Line { a: a2, b: b2 }) = (&ei.g, &ej.g)
                {
                    // Adjacent lines may only meet at their shared point. Both ends shared means
                    // a duplicate line; otherwise check for folding back onto each other.
                    let dup = shared.len() == 2 && {
                        let _ = (a1, b1, a2, b2);
                        true
                    };
                    let fold = shared.len() == 1 && {
                        let n = shared[0];
                        let far = |e: &Edge| if e.from == n { e.to } else { e.from };
                        let (u, w) = (
                            node_pos[far(ei)] - node_pos[n],
                            node_pos[far(ej)] - node_pos[n],
                        );
                        let (lu, lw) = (u.length(), w.length());
                        lu > tol
                            && lw > tol
                            && (u.cross(w) / (lu * lw)).abs() <= 1e-9
                            && u.dot(w) > 0.0
                    };
                    if dup || fold {
                        flag(
                            &mut problems,
                            ProfileProblemCode::Overlap,
                            format!("lines '{}' and '{}' lie on top of each other", ei.id, ej.id),
                            &ei.id,
                            &ej.id,
                        );
                        bad_edges.insert(i);
                        bad_edges.insert(j);
                    }
                }
            }
        }
        // Circles against edges and each other.
        for (ci, (cid, cc, cr)) in circles.iter().enumerate() {
            let g = G::Circle { c: *cc, r: *cr };
            for &i in &cyc_edges {
                if edges_touch(&g, &edges[i].g, tol) {
                    flag(
                        &mut problems,
                        ProfileProblemCode::Intersection,
                        format!("circle '{cid}' crosses or touches '{}'", edges[i].id),
                        cid,
                        &edges[i].id,
                    );
                    bad_edges.insert(i);
                    bad_edges.insert(usize::MAX - ci);
                }
            }
            for (cj, (oid, oc, or)) in circles.iter().enumerate().skip(ci + 1) {
                if edges_touch(&g, &G::Circle { c: *oc, r: *or }, tol) {
                    flag(
                        &mut problems,
                        ProfileProblemCode::Intersection,
                        format!("circles '{cid}' and '{oid}' cross or touch"),
                        cid,
                        oid,
                    );
                    bad_edges.insert(usize::MAX - ci);
                    bad_edges.insert(usize::MAX - cj);
                }
            }
        }

        // Trace loops.
        let mut loops: Vec<RawLoop> = Vec::new();
        for cyc in &cycles {
            if cyc.iter().any(|i| bad_edges.contains(i)) {
                continue;
            }
            let e0 = cyc[0];
            let mut order: Vec<(usize, bool)> = vec![(e0, false)];
            let mut cur = edges[e0].to;
            let mut last = e0;
            while order.len() < cyc.len() {
                let next = *incident[cur]
                    .iter()
                    .find(|&&j| j != last)
                    .expect("degree 2");
                if edges[next].from == cur {
                    order.push((next, false));
                    cur = edges[next].to;
                } else {
                    order.push((next, true));
                    cur = edges[next].from;
                }
                last = next;
            }
            let mut area: f64 = order
                .iter()
                .map(|&(i, rev)| {
                    if rev {
                        -area_term(&edges[i].g)
                    } else {
                        area_term(&edges[i].g)
                    }
                })
                .sum();
            if area.abs() <= tol * tol {
                problems.push(ProfileProblem {
                    code: ProfileProblemCode::Degenerate,
                    message: "a closed loop encloses no area".into(),
                    entities: cyc.iter().map(|&i| edges[i].id.clone()).collect(),
                });
                continue;
            }
            if area < 0.0 {
                order.reverse();
                for o in &mut order {
                    o.1 = !o.1;
                }
                area = -area;
            }
            let le: Vec<LoopEdge> = order
                .iter()
                .map(|&(i, rev)| {
                    let e = &edges[i];
                    let (f, t) = if rev { (e.to, e.from) } else { (e.from, e.to) };
                    let curve = match &e.g {
                        G::Line { .. } => Curve2::Line,
                        G::Arc {
                            c, r, t0, sweep, ..
                        } => Curve2::Arc {
                            center: [c.x, c.y],
                            radius: *r,
                            start_angle: *t0,
                            sweep: *sweep,
                        },
                        G::Circle { .. } => unreachable!("circles are not graph edges"),
                    };
                    LoopEdge {
                        id: e.id.clone(),
                        reversed: rev,
                        from: [node_pos[f].x, node_pos[f].y],
                        to: [node_pos[t].x, node_pos[t].y],
                        curve,
                    }
                })
                .collect();
            let min_id = le.iter().map(|e| e.id.clone()).min().expect("non-empty");
            loops.push(RawLoop {
                edges: le,
                area,
                min_id,
            });
        }
        for (ci, (id, c, r)) in circles.iter().enumerate() {
            if bad_edges.contains(&(usize::MAX - ci)) {
                continue;
            }
            if *r <= tol {
                problems.push(ProfileProblem {
                    code: ProfileProblemCode::Degenerate,
                    message: format!("circle '{id}' has no radius"),
                    entities: vec![id.clone()],
                });
                continue;
            }
            let p = [c.x + r, c.y];
            loops.push(RawLoop {
                edges: vec![LoopEdge {
                    id: id.clone(),
                    reversed: false,
                    from: p,
                    to: p,
                    curve: Curve2::Circle {
                        center: [c.x, c.y],
                        radius: *r,
                    },
                }],
                area: core::f64::consts::PI * r * r,
                min_id: id.clone(),
            });
        }
        loops.sort_by(|a, b| a.min_id.cmp(&b.min_id));

        let profiles = nest(loops);
        problems.sort_by(|a, b| (a.code as u8, &a.entities).cmp(&(b.code as u8, &b.entities)));
        ProfileReport { profiles, problems }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Constraint;
    use core::f64::consts::PI;

    fn pt(s: &mut Sketch, id: &str, x: f64, y: f64) {
        s.add_entity(id, Entity::Point { x, y }).unwrap();
    }
    fn line(s: &mut Sketch, id: &str, a: &str, b: &str) {
        s.add_entity(
            id,
            Entity::Line {
                a: a.into(),
                b: b.into(),
            },
        )
        .unwrap();
    }
    fn circle(s: &mut Sketch, id: &str, c: &str, r: f64) {
        s.add_entity(
            id,
            Entity::Circle {
                center: c.into(),
                radius: r,
            },
        )
        .unwrap();
    }
    /// Axis-aligned rectangle with corners `<p>1..<p>4` and edges `<p>a..<p>d`.
    fn rect(s: &mut Sketch, p: &str, x0: f64, y0: f64, x1: f64, y1: f64, ccw: bool) {
        let c = [(x0, y0), (x1, y0), (x1, y1), (x0, y1)];
        let order: Vec<usize> = if ccw {
            vec![0, 1, 2, 3]
        } else {
            vec![0, 3, 2, 1]
        };
        for (k, &i) in order.iter().enumerate() {
            pt(s, &format!("{p}{}", k + 1), c[i].0, c[i].1);
        }
        for k in 0..4 {
            line(
                s,
                &format!("{p}{}", (b'a' + k as u8) as char),
                &format!("{p}{}", k + 1),
                &format!("{p}{}", (k + 1) % 4 + 1),
            );
        }
    }
    fn close(a: f64, b: f64) -> bool {
        (a - b).abs() < 1e-9
    }

    #[test]
    fn a_rectangle_is_one_ccw_profile_whichever_way_it_was_drawn() {
        for ccw in [true, false] {
            let mut s = Sketch::new();
            rect(&mut s, "r", 0.0, 0.0, 80.0, 40.0, ccw);
            let r = s.profiles();
            assert!(r.problems.is_empty(), "{:?}", r.problems);
            assert_eq!(r.profiles.len(), 1);
            let p = &r.profiles[0];
            assert!(
                close(p.outer.area, 3200.0) && close(p.area, 3200.0),
                "{}",
                p.outer.area
            );
            assert_eq!(p.outer.edges.len(), 4);
            assert!(p.holes.is_empty());
            assert_eq!(p.id, "profile.ra");
            // Adjacent edges share exactly equal endpoint coordinates.
            let e = &p.outer.edges;
            for i in 0..4 {
                assert_eq!(e[i].to, e[(i + 1) % 4].from);
            }
        }
    }

    #[test]
    fn holes_and_islands_nest_by_depth() {
        let mut s = Sketch::new();
        rect(&mut s, "r", 0.0, 0.0, 100.0, 100.0, true);
        pt(&mut s, "c1", 50.0, 50.0);
        circle(&mut s, "hole", "c1", 30.0); // hole in the rectangle
        circle(&mut s, "island", "c1", 10.0); // island inside the hole
        pt(&mut s, "c2", 90.0, 50.0);
        circle(&mut s, "hole2", "c2", 5.0); // second hole
        let r = s.profiles();
        assert!(r.problems.is_empty(), "{:?}", r.problems);
        assert_eq!(
            r.profiles.len(),
            2,
            "{:#?}",
            r.profiles.iter().map(|p| &p.id).collect::<Vec<_>>()
        );
        let outer = r.profiles.iter().find(|p| p.id == "profile.ra").unwrap();
        assert_eq!(outer.holes.len(), 2);
        assert!(
            close(outer.area, 10_000.0 - PI * 900.0 - PI * 25.0),
            "{}",
            outer.area
        );
        let island = r
            .profiles
            .iter()
            .find(|p| p.id == "profile.island")
            .unwrap();
        assert!(island.holes.is_empty() && close(island.area, PI * 100.0));
    }

    #[test]
    fn separate_shapes_are_separate_profiles_in_stable_order() {
        let mut s = Sketch::new();
        rect(&mut s, "b", 0.0, 0.0, 10.0, 10.0, true);
        rect(&mut s, "a", 50.0, 50.0, 60.0, 70.0, true);
        let ids: Vec<_> = s.profiles().profiles.iter().map(|p| p.id.clone()).collect();
        assert_eq!(ids, ["profile.aa", "profile.ba"]);
    }

    #[test]
    fn a_d_shape_of_a_line_and_an_arc_has_exact_area() {
        let mut s = Sketch::new();
        pt(&mut s, "c", 0.0, 0.0);
        pt(&mut s, "l", -10.0, 0.0);
        pt(&mut s, "rr", 10.0, 0.0);
        // Counter-clockwise from (10,0) to (-10,0): the upper half circle.
        s.add_entity(
            "arc",
            Entity::Arc {
                center: "c".into(),
                start: "rr".into(),
                end: "l".into(),
            },
        )
        .unwrap();
        line(&mut s, "base", "l", "rr");
        let r = s.profiles();
        assert!(r.problems.is_empty(), "{:?}", r.problems);
        assert!(
            close(r.profiles[0].area, PI * 50.0),
            "{}",
            r.profiles[0].area
        );
        // The same shape from the lower half circle: also exact, and still positive.
        let mut s2 = Sketch::new();
        pt(&mut s2, "c", 0.0, 0.0);
        pt(&mut s2, "l", -10.0, 0.0);
        pt(&mut s2, "rr", 10.0, 0.0);
        s2.add_entity(
            "arc",
            Entity::Arc {
                center: "c".into(),
                start: "l".into(),
                end: "rr".into(),
            },
        )
        .unwrap();
        line(&mut s2, "base", "l", "rr");
        assert!(close(s2.profiles().profiles[0].area, PI * 50.0));
    }

    #[test]
    fn off_center_arcs_have_exact_area_too() {
        // Quarter disc of radius 6 centered at (20, 30): corners (26,30) -> (20,36) via CCW arc.
        let mut s = Sketch::new();
        pt(&mut s, "c", 20.0, 30.0);
        pt(&mut s, "a", 26.0, 30.0);
        pt(&mut s, "b", 20.0, 36.0);
        s.add_entity(
            "arc",
            Entity::Arc {
                center: "c".into(),
                start: "a".into(),
                end: "b".into(),
            },
        )
        .unwrap();
        line(&mut s, "l1", "b", "c");
        line(&mut s, "l2", "c", "a");
        let r = s.profiles();
        assert!(r.problems.is_empty(), "{:?}", r.problems);
        assert!(
            close(r.profiles[0].area, PI * 36.0 / 4.0),
            "{}",
            r.profiles[0].area
        );
    }

    #[test]
    fn coincident_constraints_join_separate_points() {
        let mut s = Sketch::new();
        for (id, x, y) in [
            ("a", 0.0, 0.0),
            ("b", 10.0, 0.0),
            ("b2", 10.0, 0.0),
            ("c", 10.0, 10.0),
            ("c2", 10.0, 10.0),
            ("d", 0.0, 10.0),
            ("d2", 0.0, 10.0),
            ("a2", 0.0, 0.0),
        ] {
            pt(&mut s, id, x, y);
        }
        line(&mut s, "l1", "a", "b");
        line(&mut s, "l2", "b2", "c");
        line(&mut s, "l3", "c2", "d");
        line(&mut s, "l4", "d2", "a2");
        assert!(s.profiles().profiles.is_empty(), "not joined yet");
        for (i, (p, q)) in [("b", "b2"), ("c", "c2"), ("d", "d2"), ("a", "a2")]
            .iter()
            .enumerate()
        {
            s.add_constraint(
                &format!("co{i}"),
                Constraint::Coincident {
                    a: (*p).into(),
                    b: (*q).into(),
                },
            )
            .unwrap();
        }
        let r = s.profiles();
        assert!(r.problems.is_empty(), "{:?}", r.problems);
        assert!(close(r.profiles[0].area, 100.0));
    }

    #[test]
    fn open_chains_and_branches_are_named_not_guessed() {
        let mut open = Sketch::new();
        pt(&mut open, "a", 0.0, 0.0);
        pt(&mut open, "b", 10.0, 0.0);
        pt(&mut open, "c", 10.0, 10.0);
        line(&mut open, "l1", "a", "b");
        line(&mut open, "l2", "b", "c");
        let r = open.profiles();
        assert!(r.profiles.is_empty());
        let ends: Vec<_> = r
            .problems
            .iter()
            .filter(|p| p.code == ProfileProblemCode::OpenEnd)
            .flat_map(|p| p.entities.clone())
            .collect();
        assert_eq!(ends, ["a", "c"]);

        let mut branched = Sketch::new();
        rect(&mut branched, "r", 0.0, 0.0, 10.0, 10.0, true);
        line(&mut branched, "diag", "r1", "r3");
        let r = branched.profiles();
        assert!(r.profiles.is_empty(), "no silent splitting");
        let at: Vec<_> = r
            .problems
            .iter()
            .filter(|p| p.code == ProfileProblemCode::Branch)
            .flat_map(|p| p.entities.clone())
            .collect();
        assert_eq!(at, ["r1", "r3"]);
    }

    #[test]
    fn crossing_and_touching_shapes_are_rejected_with_both_names() {
        let mut s = Sketch::new();
        rect(&mut s, "a", 0.0, 0.0, 20.0, 20.0, true);
        rect(&mut s, "b", 10.0, 10.0, 30.0, 30.0, true); // overlaps a
        let r = s.profiles();
        assert!(r.profiles.is_empty());
        let ix: Vec<_> = r
            .problems
            .iter()
            .filter(|p| p.code == ProfileProblemCode::Intersection)
            .collect();
        assert!(!ix.is_empty());
        assert!(ix.iter().any(|p| p.entities.contains(&"ab".to_string())
            && p.entities.contains(&"bd".to_string())
            || p.entities.contains(&"ab".to_string()) && p.entities.contains(&"ba".to_string())
            || p.entities.iter().any(|e| e.starts_with('a'))
                && p.entities.iter().any(|e| e.starts_with('b'))));

        // A circle poking through a rectangle edge.
        let mut s = Sketch::new();
        rect(&mut s, "r", 0.0, 0.0, 20.0, 20.0, true);
        pt(&mut s, "c", 20.0, 10.0);
        circle(&mut s, "bite", "c", 5.0);
        assert!(s.profiles().profiles.is_empty());

        // Two rectangles sharing a coordinate but not a point: touching = rejected.
        let mut s = Sketch::new();
        rect(&mut s, "a", 0.0, 0.0, 10.0, 10.0, true);
        rect(&mut s, "b", 10.0, 0.0, 20.0, 10.0, true);
        let r = s.profiles();
        assert!(r.profiles.is_empty());
        assert!(r
            .problems
            .iter()
            .any(|p| p.code == ProfileProblemCode::Intersection));
    }

    #[test]
    fn tangent_circle_inside_is_fine_but_touching_is_not() {
        let mut s = Sketch::new();
        rect(&mut s, "r", 0.0, 0.0, 100.0, 100.0, true);
        pt(&mut s, "c", 50.0, 50.0);
        circle(&mut s, "ok", "c", 49.0);
        let r = s.profiles();
        assert!(r.problems.is_empty(), "{:?}", r.problems);
        assert_eq!(
            r.profiles.len(),
            1,
            "an inner circle is a hole, not a second profile"
        );
        assert_eq!(r.profiles[0].holes.len(), 1);
        let mut t = Sketch::new();
        rect(&mut t, "r", 0.0, 0.0, 100.0, 100.0, true);
        pt(&mut t, "c", 50.0, 50.0);
        circle(&mut t, "touch", "c", 50.0); // tangent to all four sides
        assert!(t.profiles().profiles.is_empty());
    }

    #[test]
    fn degenerate_and_doubled_geometry_is_reported() {
        let mut s = Sketch::new();
        pt(&mut s, "a", 0.0, 0.0);
        pt(&mut s, "b", 0.0, 0.0);
        line(&mut s, "zero", "a", "b");
        s.add_constraint(
            "co",
            Constraint::Coincident {
                a: "a".into(),
                b: "b".into(),
            },
        )
        .unwrap();
        assert!(s
            .profiles()
            .problems
            .iter()
            .any(|p| p.code == ProfileProblemCode::Degenerate));

        let mut d = Sketch::new();
        rect(&mut d, "r", 0.0, 0.0, 10.0, 10.0, true);
        line(&mut d, "dup", "r1", "r2"); // second line between the same two points
        let r = d.profiles();
        assert!(r.profiles.is_empty());
        assert!(
            r.problems
                .iter()
                .any(|p| p.code == ProfileProblemCode::Overlap
                    || p.code == ProfileProblemCode::Branch)
        );
    }

    #[test]
    fn profiles_are_deterministic_and_serializable() {
        let mut s = Sketch::new();
        rect(&mut s, "r", 0.0, 0.0, 100.0, 100.0, true);
        pt(&mut s, "c", 50.0, 50.0);
        circle(&mut s, "h", "c", 20.0);
        let a = serde_json::to_string(&s.profiles()).unwrap();
        let b = serde_json::to_string(&s.clone().profiles()).unwrap();
        assert_eq!(a, b);
        let back: ProfileReport = serde_json::from_str(&a).unwrap();
        assert_eq!(back, s.profiles());
    }
}
