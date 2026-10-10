//! Boolean operations on solids bounded by planes and cylinders (design note:
//! `docs/kernel/boolean.md`). `boolean(a, b, op)` is pure: it reads both solids and returns a new
//! one, or says why it cannot, and never returns a solid that fails [`Solid::validate`].
//!
//! Method (boundary merge): cut every face along its intersection with the other solid's faces,
//! node all curves against each other so neighbouring faces agree on their shared edges, trace
//! the regions of each face's arrangement in the face's own flat picture (see `prep`), classify
//! each region against the other solid (inside, outside, or coincident with one of its faces,
//! same or opposite sense), keep what the operation selects, merge coplanar neighbours, and
//! stitch the result.
//!
//! Complexity: face pairs are pruned by bounding box; noding compares each curve with the nodes
//! inside its x range (sorted), so the cost follows nearby candidates, not the whole model
//! squared. Nothing here is parallel, so the result does not depend on scheduling.

mod build;
mod cuts;
pub mod geo;
pub mod planar;
mod prep;

use crate::solid::Solid;
use crate::validate::Issue;
use cuts::pair_cuts;
use dc_math::{Tolerance, Vec3};
use geo::{arc_arc, Arc};
use planar::{cycle_points, interior_point, regions, AEdge, HalfEdge, P2};
use prep::{prepare, BFace, Loc, Surf, TOL};
use std::collections::BTreeMap;

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Side {
    A,
    B,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BoolOp {
    Union,
    Subtract,
    Intersect,
}

#[derive(Clone, Debug, PartialEq)]
pub enum BoolError {
    /// The inputs contain something this phase cannot do, named.
    Unsupported(String),
    /// The result is empty (nothing is left).
    Empty,
    /// A classification or tracing step hit a case it cannot decide; nothing was changed.
    Degenerate(String),
    /// The constructed result failed validation; the issues say how.
    Invalid(Vec<Issue>),
}

impl core::fmt::Display for BoolError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            BoolError::Unsupported(s) => write!(f, "unsupported: {s}"),
            BoolError::Empty => write!(f, "the result is empty"),
            BoolError::Degenerate(s) => write!(f, "degenerate case: {s}"),
            BoolError::Invalid(i) => write!(f, "the result is not a valid solid: {i:?}"),
        }
    }
}

/// Names the faces that come from solid B in the result.
pub type Namer = std::sync::Arc<dyn Fn(&str) -> String + Send + Sync>;

#[derive(Clone)]
pub struct BoolOptions {
    /// Prefix put on the names of faces that come from solid B (when `b_namer` is not set).
    pub b_prefix: String,
    /// Maps the name of a face of B to its name in the result.
    pub b_namer: Option<Namer>,
}

impl Default for BoolOptions {
    fn default() -> Self {
        BoolOptions {
            b_prefix: "tool:".into(),
            b_namer: None,
        }
    }
}

impl core::fmt::Debug for BoolOptions {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("BoolOptions")
            .field("b_prefix", &self.b_prefix)
            .field("b_namer", &self.b_namer.is_some())
            .finish()
    }
}

/// Which input faces an output face comes from.
#[derive(Clone, Debug, PartialEq)]
pub struct FaceSource {
    pub face: String,
    pub from: Vec<(Side, String)>,
}

#[derive(Clone, Debug)]
pub struct BoolResult {
    pub solid: Solid,
    pub sources: Vec<FaceSource>,
}

// ---- node pool ---------------------------------------------------------------------------------

const CELL: f64 = 8.0 * TOL;

#[derive(Default)]
pub(crate) struct Pool {
    pub pts: Vec<Vec3>,
    grid: BTreeMap<(i64, i64, i64), Vec<u32>>,
}

impl Pool {
    fn cell(p: Vec3) -> (i64, i64, i64) {
        let c = |v: f64| (v / CELL).floor() as i64;
        (c(p.x), c(p.y), c(p.z))
    }

    /// The node at `p`: an existing one within tolerance (the closest, then the lowest ID), or a
    /// new one.
    pub fn id(&mut self, p: Vec3) -> u32 {
        let (cx, cy, cz) = Pool::cell(p);
        let mut best: Option<(f64, u32)> = None;
        for dx in -1..=1 {
            for dy in -1..=1 {
                for dz in -1..=1 {
                    if let Some(ids) = self.grid.get(&(cx + dx, cy + dy, cz + dz)) {
                        for &i in ids {
                            let d = (self.pts[i as usize] - p).length();
                            if d <= TOL && best.is_none_or(|(bd, bi)| d < bd || (d == bd && i < bi))
                            {
                                best = Some((d, i));
                            }
                        }
                    }
                }
            }
        }
        if let Some((_, i)) = best {
            return i;
        }
        let i = self.pts.len() as u32;
        self.pts.push(p);
        self.grid.entry((cx, cy, cz)).or_default().push(i);
        i
    }

    pub fn p(&self, i: u32) -> Vec3 {
        self.pts[i as usize]
    }
}

// ---- directed pieces ---------------------------------------------------------------------------

/// A directed piece of curve between two nodes.
#[derive(Clone, Debug)]
pub(crate) struct Seg {
    pub a: u32,
    pub b: u32,
    pub arc: Arc,
}

impl Seg {
    pub fn reversed(&self) -> Seg {
        Seg {
            a: self.b,
            b: self.a,
            arc: self.arc.reversed(),
        }
    }
}

/// Identity of a piece of curve independent of direction: its two end nodes and its middle.
#[derive(Default)]
pub(crate) struct PieceTable {
    entries: Vec<(u32, u32, Vec3, Vec3)>,
}

impl PieceTable {
    fn travel(s: &Seg) -> Vec3 {
        let t = 0.5 * (s.arc.t0 + s.arc.t1);
        s.arc.geo.tangent(t) * (s.arc.t1 - s.arc.t0).signum()
    }

    pub fn find(&self, s: &Seg) -> Option<(usize, bool)> {
        let mid = s.arc.mid();
        let (lo, hi) = (s.a.min(s.b), s.a.max(s.b));
        let dir = PieceTable::travel(s);
        self.entries
            .iter()
            .position(|e| e.0 == lo && e.1 == hi && (e.2 - mid).length() <= 1e-6)
            .map(|i| (i, self.entries[i].3.dot(dir) < 0.0))
    }

    /// `(index, reversed)`: the piece of `s`, added if it is new. `reversed` is true if `s` runs
    /// the other way from the first one stored.
    pub fn id(&mut self, s: &Seg) -> (usize, bool) {
        if let Some(found) = self.find(s) {
            return found;
        }
        let (lo, hi) = (s.a.min(s.b), s.a.max(s.b));
        self.entries
            .push((lo, hi, s.arc.mid(), PieceTable::travel(s)));
        (self.entries.len() - 1, false)
    }
}

// ---- noding ------------------------------------------------------------------------------------

struct Noder {
    by_x: Vec<(f64, u32)>,
}

impl Noder {
    fn new(pool: &Pool) -> Noder {
        let mut by_x: Vec<(f64, u32)> = pool
            .pts
            .iter()
            .enumerate()
            .map(|(i, p)| (p.x, i as u32))
            .collect();
        by_x.sort_by(|a, b| a.0.total_cmp(&b.0).then(a.1.cmp(&b.1)));
        Noder { by_x }
    }

    /// The pieces of `seg` after splitting at every node that lies on it.
    fn split(&self, pool: &Pool, seg: &Seg) -> Vec<Seg> {
        let pl = seg.arc.polyline_sag(1e-3);
        let (mut lo, mut hi) = (f64::MAX, f64::MIN);
        for p in &pl {
            lo = lo.min(p.x);
            hi = hi.max(p.x);
        }
        let (lo, hi) = (lo - 2e-3, hi + 2e-3);
        let start = self.by_x.partition_point(|e| e.0 < lo);
        let mut on: Vec<(f64, f64, u32)> = Vec::new();
        for &(x, i) in &self.by_x[start..] {
            if x > hi {
                break;
            }
            let p = pool.p(i);
            let Some(t) = seg.arc.param_of_point(p) else {
                continue;
            };
            let f = (t - seg.arc.t0) / (seg.arc.t1 - seg.arc.t0);
            let at_end = (i == seg.a && f < 0.5) || (i == seg.b && f > 0.5);
            if at_end || f <= 0.0 || f >= 1.0 {
                // The ends themselves (a closed curve's start is also its end).
                if i != seg.a && i != seg.b {
                    continue;
                }
                continue;
            }
            on.push((f, t, i));
        }
        on.sort_by(|x, y| x.0.total_cmp(&y.0).then(x.2.cmp(&y.2)));
        on.dedup_by(|x, y| x.2 == y.2);
        let mut out = Vec::new();
        let (mut cur_n, mut cur_t) = (seg.a, seg.arc.t0);
        for (_, t, n) in on {
            if n == cur_n {
                continue;
            }
            out.push(Seg {
                a: cur_n,
                b: n,
                arc: seg.arc.between(cur_t, t),
            });
            cur_n = n;
            cur_t = t;
        }
        out.push(Seg {
            a: cur_n,
            b: seg.b,
            arc: seg.arc.between(cur_t, seg.arc.t1),
        });
        out
    }
}

// ---- classification ----------------------------------------------------------------------------

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Class {
    In,
    Out,
    OnSame,
    OnOpposite,
}

const DIRS: [(f64, f64, f64); 8] = [
    (
        0.371_390_676_354_103_7,
        0.557_086_014_531_155_6,
        0.742_781_352_708_207_4,
    ),
    (
        -0.267_261_241_912_424_4,
        0.534_522_483_824_848_8,
        0.801_783_725_737_273_2,
    ),
    (
        0.801_783_725_737_273_2,
        -0.534_522_483_824_848_8,
        0.267_261_241_912_424_4,
    ),
    (
        -0.617_213_399_848_368_2,
        -0.308_606_699_924_184_1,
        0.723_189_473_228_359_1,
    ),
    (
        0.151_585_927_420_364_3,
        -0.788_246_821_385_894_4,
        -0.597_424_503_398_219,
    ),
    (-0.707_106_781_186_547_5, 0.0, 0.707_106_781_186_547_5),
    (
        0.123_456_789_012_345_67,
        0.987_654_321_098_765_4,
        -0.098_765_432_109_876_5,
    ),
    (
        0.577_350_269_189_625_8,
        0.577_350_269_189_625_8,
        -0.577_350_269_189_625_8,
    ),
];

/// Where the ray `q + t d` meets the surface of `g` for `t > 0`, as points on that surface.
fn ray_hits(g: &BFace, q: Vec3, d: Vec3) -> Vec<Vec3> {
    match &g.surf {
        Surf::Plane { origin, normal, .. } => {
            let den = d.dot(*normal);
            if den.abs() < 1e-9 {
                return Vec::new();
            }
            let t = -((q - *origin).dot(*normal)) / den;
            if t <= 0.0 {
                Vec::new()
            } else {
                vec![q + d * t]
            }
        }
        Surf::Cyl {
            origin, axis, r, ..
        } => {
            let dp = d - *axis * d.dot(*axis);
            let w = q - *origin;
            let wp = w - *axis * w.dot(*axis);
            let a = dp.dot(dp);
            if a < 1e-12 {
                return Vec::new();
            }
            let b = 2.0 * wp.dot(dp);
            let c = wp.dot(wp) - r * r;
            let disc = b * b - 4.0 * a * c;
            if disc < 0.0 {
                return Vec::new();
            }
            let sq = disc.sqrt();
            [(-b - sq) / (2.0 * a), (-b + sq) / (2.0 * a)]
                .into_iter()
                .filter(|t| *t > 1e-9)
                .map(|t| q + d * t)
                .collect()
        }
    }
}

fn classify(q: Vec3, n: Vec3, others: &[BFace], tol: Tolerance) -> Result<Class, BoolError> {
    for g in others {
        if g.surf.dist(q) <= TOL
            && q.x >= g.lo.x - TOL
            && q.x <= g.hi.x + TOL
            && q.y >= g.lo.y - TOL
            && q.y <= g.hi.y + TOL
            && q.z >= g.lo.z - TOL
            && q.z <= g.hi.z + TOL
            && g.locate(q) != Loc::Outside
        {
            return Ok(if n.dot(g.surf.normal_at(q)) > 0.0 {
                Class::OnSame
            } else {
                Class::OnOpposite
            });
        }
    }
    'dir: for &(x, y, z) in &DIRS {
        let d = Vec3::new(x, y, z).normalized(tol).expect("fixed direction");
        let mut crossings = 0;
        for g in others {
            // Skip faces the ray cannot reach (a ray's own box test would need a slab test; the
            // hit test below is cheap enough for the face counts of one part).
            for hit in ray_hits(g, q, d) {
                match g.locate(hit) {
                    Loc::Inside => crossings += 1,
                    Loc::Boundary => continue 'dir,
                    Loc::Outside => {}
                }
            }
            if let Surf::Cyl {
                axis, origin, r, ..
            } = &g.surf
            {
                // A ray that just grazes the cylinder is not a clear answer.
                let dp = d - *axis * d.dot(*axis);
                let w = q - *origin;
                let wp = w - *axis * w.dot(*axis);
                let a = dp.dot(dp);
                if a >= 1e-12 {
                    let b = 2.0 * wp.dot(dp);
                    let c = wp.dot(wp) - r * r;
                    let disc = b * b - 4.0 * a * c;
                    if disc.abs() < 1e-9 * (b * b).max(1.0) {
                        continue 'dir;
                    }
                }
            }
        }
        return Ok(if crossings % 2 == 1 {
            Class::In
        } else {
            Class::Out
        });
    }
    Err(BoolError::Degenerate(
        "no ray direction avoids the edges of the other solid".into(),
    ))
}

fn keeps(op: BoolOp, side: Side, class: Class) -> Option<bool> {
    match (op, side, class) {
        (BoolOp::Union, Side::A, Class::Out | Class::OnSame) => Some(false),
        (BoolOp::Union, Side::B, Class::Out) => Some(false),
        (BoolOp::Subtract, Side::A, Class::Out | Class::OnOpposite) => Some(false),
        (BoolOp::Subtract, Side::B, Class::In) => Some(true),
        (BoolOp::Intersect, Side::A, Class::In | Class::OnSame) => Some(false),
        (BoolOp::Intersect, Side::B, Class::In) => Some(false),
        _ => None,
    }
}

// ---- the operation -----------------------------------------------------------------------------

/// A face piece that passed selection: its surface (oriented outward), its boundary loops as
/// directed pieces (outer first), and the input faces it comes from.
pub(crate) struct OutFace {
    pub sources: Vec<(Side, String)>,
    pub surf: Surf,
    pub loops: Vec<Vec<Seg>>,
}

impl OutFace {
    fn flipped(mut self) -> OutFace {
        for lp in &mut self.loops {
            *lp = lp.iter().rev().map(Seg::reversed).collect();
        }
        self.surf = match self.surf {
            Surf::Plane {
                origin, normal, x, ..
            } => Surf::Plane {
                origin,
                normal: -normal,
                x,
                y: (-normal).cross(x),
            },
            Surf::Cyl {
                origin,
                axis,
                e1,
                e2,
                r,
                out,
                winding,
                x0,
            } => Surf::Cyl {
                origin,
                axis,
                e1,
                e2,
                r,
                out: -out,
                winding,
                x0,
            },
        };
        self
    }
}

/// `a op b`. See the module documentation for what is supported.
pub fn boolean(
    a: &Solid,
    b: &Solid,
    op: BoolOp,
    opts: &BoolOptions,
) -> Result<BoolResult, BoolError> {
    let tol = Tolerance::DEFAULT;
    let fa = prepare(a, Side::A)?;
    let fb = prepare(b, Side::B)?;
    let faces: Vec<&BFace> = fa.iter().chain(fb.iter()).collect();
    let slot = |f: &BFace| -> usize {
        if f.side == Side::A {
            f.idx
        } else {
            fa.len() + f.idx
        }
    };

    let mut pool = Pool::default();
    // Boundary pieces of every face, with end nodes.
    let mut boundary: Vec<Vec<Seg>> = Vec::with_capacity(faces.len());
    for f in &faces {
        let mut v = Vec::new();
        for arc in f.loops.iter().flatten() {
            let (na, nb) = (pool.id(arc.start()), pool.id(arc.end()));
            v.push(Seg {
                a: na,
                b: nb,
                arc: arc.clone(),
            });
        }
        boundary.push(v);
    }

    // Where the two solids' faces meet.
    let mut cut_segs: Vec<Vec<Seg>> = vec![Vec::new(); faces.len()];
    for x in &fa {
        for y in &fb {
            if !x.overlaps(y) {
                continue;
            }
            for c in pair_cuts(x, y)? {
                if c.arc.span() * c.arc.geo.radius().max(1.0) <= TOL {
                    continue;
                }
                let (na, nb) = (pool.id(c.arc.start()), pool.id(c.arc.end()));
                let seg = Seg {
                    a: na,
                    b: nb,
                    arc: c.arc,
                };
                if c.into_a {
                    cut_segs[slot(x)].push(seg.clone());
                }
                if c.into_b {
                    cut_segs[slot(y)].push(seg);
                }
            }
        }
    }

    // Cuts that cross another curve of the same face in the interior of both need a node there.
    // (For valid inputs cuts meet other curves at their ends; the seam of a cylinder is the
    // usual exception, and this also catches degenerate cases.)
    for (fi, _) in faces.iter().enumerate() {
        let mut found: Vec<Vec3> = Vec::new();
        let all: Vec<&Seg> = cut_segs[fi].iter().chain(boundary[fi].iter()).collect();
        for (i, s) in cut_segs[fi].iter().enumerate() {
            for (j, o) in all.iter().enumerate() {
                if j < cut_segs[fi].len() && j <= i {
                    continue;
                }
                for (ta, tb) in arc_arc(&s.arc, &o.arc) {
                    let (pa, pb) = (s.arc.point(ta), o.arc.point(tb));
                    let interior = |seg: &Seg, p: Vec3| {
                        (p - seg.arc.start()).length() > TOL && (p - seg.arc.end()).length() > TOL
                    };
                    if interior(s, pa) && interior(o, pb) {
                        found.push((pa + pb) * 0.5);
                    }
                }
            }
        }
        for p in found {
            pool.id(p);
        }
    }

    // Node every piece against every node on it; the faces' edge sets then agree.
    let noder = Noder::new(&pool);
    let mut face_segs: Vec<Vec<Seg>> = Vec::with_capacity(faces.len());
    for fi in 0..faces.len() {
        // Boundary pieces first; a seam piece occurs twice (up and down) and both stay.
        let mut result: Vec<Seg> = Vec::new();
        let mut table = PieceTable::default();
        for s in &boundary[fi] {
            for piece in noder.split(&pool, s) {
                table.id(&piece);
                result.push(piece);
            }
        }
        // Cuts are added unless the face already has that piece.
        for s in &cut_segs[fi] {
            for piece in noder.split(&pool, s) {
                if table.find(&piece).is_none() {
                    table.id(&piece);
                    result.push(piece);
                }
            }
        }
        face_segs.push(result);
    }

    // Regions of every face, kept if they lie inside the original face.
    let mut pieces: Vec<(Side, Class, OutFace)> = Vec::new();
    for (fi, f) in faces.iter().enumerate() {
        let segs = &face_segs[fi];
        let others: &[BFace] = if f.side == Side::A { &fb } else { &fa };
        let (placed, kpos) = build::place_all(f, segs);
        let edges: Vec<AEdge> = placed.iter().map(|p| p.edge.clone()).collect();
        let pos = |k: u32| kpos[&k];
        for r in regions(&pos, &edges) {
            let outer_pts = cycle_points(&edges, &pos, &r.outer);
            let mut loops2: Vec<Vec<P2>> = vec![outer_pts];
            for h in &r.holes {
                loops2.push(cycle_points(&edges, &pos, h));
            }
            let Some(s) = interior_point(&loops2) else {
                continue;
            };
            let q = f.from2(s);
            if f.locate(q) != Loc::Inside {
                continue;
            }
            let n = f.surf.normal_at(q);
            let class = classify(q, n, others, tol)?;
            let to_loop = |hs: &[HalfEdge]| -> Vec<Seg> {
                hs.iter()
                    .map(|&(i, fwd)| {
                        if fwd {
                            placed[i].seg.clone()
                        } else {
                            placed[i].seg.reversed()
                        }
                    })
                    .collect()
            };
            let mut loops = vec![to_loop(&r.outer)];
            loops.extend(r.holes.iter().map(|h| to_loop(h)));
            pieces.push((
                f.side,
                class,
                OutFace {
                    sources: vec![(f.side, f.name.clone())],
                    surf: f.surf.clone(),
                    loops,
                },
            ));
        }
    }

    // Select what the operation keeps.
    let mut kept: Vec<OutFace> = Vec::new();
    for (side, class, piece) in pieces {
        match keeps(op, side, class) {
            Some(false) => kept.push(piece),
            Some(true) => kept.push(piece.flipped()),
            None => {}
        }
    }
    if kept.is_empty() {
        return Err(BoolError::Empty);
    }
    let merged = build::merge_coplanar(kept, &pool)?;
    let fused = build::fuse(merged, &pool)?;
    build::finish(fused, &pool, opts)
}

impl Solid {
    /// [`Solid::boolean`] naming the faces that come from `other` `<owner>:<its name>`.
    pub fn boolean_named(
        &self,
        other: &Solid,
        op: BoolOp,
        owner: &str,
    ) -> Result<BoolResult, BoolError> {
        let owner = owner.to_owned();
        let opts = BoolOptions {
            b_prefix: String::new(),
            b_namer: Some(std::sync::Arc::new(move |n: &str| format!("{owner}:{n}"))),
        };
        boolean(self, other, op, &opts)
    }

    /// `self op other` for solids with planar and cylindrical faces (see [`boolean`]).
    pub fn boolean(&self, other: &Solid, op: BoolOp) -> Result<BoolResult, BoolError> {
        boolean(self, other, op, &BoolOptions::default())
    }
}
