//! Stepped prisms: bodies built as a stack of slabs along one axis, each slab a set of 2D regions.
//!
//! This model is closed under extrudes, cuts and bosses whose axis is the stack's axis, which
//! covers through-cuts, blind pockets, counterbores, steps and stacked bosses without a general
//! 3D boolean. Any stack can be turned into a validated B-rep by [`build_solid`].
//!
//! ```text
//!  level 3 ─────────  w
//!   slab 2  [ M2 ]
//!  level 2 ─────────
//!   slab 1  [ M1 ]      M_k = the material cross-section (profiles with holes) of slab k
//!  level 1 ─────────
//!   slab 0  [ M0 ]
//!  level 0 ─────────
//! ```

use dc_kernel::{CoEdge, Curve, Edge, Face, Id, Solid, Surface};
use dc_math::{det, Plane, Tolerance, Vec2, Vec3};
use dc_sketch::{refine, subtract, union, Curve2, Loop, LoopEdge, Profile, RegionError};
use std::collections::BTreeMap;
use std::fmt;

const TWO_PI: f64 = 2.0 * core::f64::consts::PI;
/// Points closer than this are the same point.
const MERGE: f64 = 1e-7;

/// Whether a level is where a feature's extent begins (at its sketch plane) or ends.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Role {
    Start,
    End,
}

/// A plane perpendicular to the stack's axis, at height `w` above the model plane.
#[derive(Clone, Debug, PartialEq)]
pub struct Level {
    pub w: f64,
    /// The feature that introduced this level ("" for the base extrusion). Names the caps.
    pub owner: String,
    pub role: Role,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Stepped {
    /// The model plane: origin at `w = 0`, axis = its normal.
    pub plane: Plane,
    pub levels: Vec<Level>,
    /// `slabs[k]` is the material between `levels[k]` and `levels[k + 1]`.
    pub slabs: Vec<Vec<Profile>>,
    /// Circular rims rounded across the axis, applied to the built solid in this order. They
    /// are kept here, not baked in, so later cuts and bosses can still be built on the layers
    /// and the rounding is redone on the result.
    pub rims: Vec<RimSpec>,
}

/// A rounded circular rim: the two faces that meet there, what to put in its place, and the
/// name of the face it makes.
#[derive(Clone, Debug, PartialEq)]
pub struct RimSpec {
    pub face_a: String,
    pub face_b: String,
    pub kind: dc_kernel::RimRound,
    pub size: f64,
    pub name: String,
}

#[derive(Debug, Clone, PartialEq)]
pub enum StepError {
    Region(RegionError),
    /// The operation would change nothing, or has nothing to act on.
    Nothing(String),
    /// The request is valid but needs surfaces or cuts the kernel does not have yet.
    Unsupported(String),
    /// A boss that does not touch the part.
    NoContact(String),
    /// The stack could not be turned into a valid solid (a kernel-level problem).
    Invalid(Vec<String>),
}

impl fmt::Display for StepError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            StepError::Region(e) => write!(f, "{e}"),
            StepError::Nothing(m) | StepError::Unsupported(m) | StepError::NoContact(m) => {
                write!(f, "{m}")
            }
            StepError::Invalid(v) => write!(f, "the result is not a valid solid: {}", v.join("; ")),
        }
    }
}

impl std::error::Error for StepError {}

impl From<RegionError> for StepError {
    fn from(e: RegionError) -> Self {
        StepError::Region(e)
    }
}

fn near(a: f64, b: f64) -> bool {
    (a - b).abs() <= Tolerance::DEFAULT.linear
}

impl Stepped {
    /// A single slab: `profile` extruded by `depth` from the plane, along its normal or against it.
    pub fn extrusion(plane: Plane, profile: Profile, depth: f64, reverse: bool) -> Stepped {
        let (lo, hi) = if reverse {
            (
                Level {
                    w: -depth,
                    owner: String::new(),
                    role: Role::End,
                },
                Level {
                    w: 0.0,
                    owner: String::new(),
                    role: Role::Start,
                },
            )
        } else {
            (
                Level {
                    w: 0.0,
                    owner: String::new(),
                    role: Role::Start,
                },
                Level {
                    w: depth,
                    owner: String::new(),
                    role: Role::End,
                },
            )
        };
        Stepped {
            plane,
            levels: vec![lo, hi],
            slabs: vec![vec![profile]],
            rims: Vec::new(),
        }
    }

    /// One slab holding several disjoint profiles.
    pub fn extrusion_all(
        plane: Plane,
        profiles: Vec<Profile>,
        depth: f64,
        reverse: bool,
    ) -> Stepped {
        let mut s = Stepped::extrusion(plane, profiles[0].clone(), depth, reverse);
        s.slabs = vec![profiles];
        s
    }

    /// Like [`extrusion_span`](Self::extrusion_span) for several disjoint profiles.
    pub fn extrusion_span_all(plane: Plane, profiles: Vec<Profile>, lo: f64, hi: f64) -> Stepped {
        let mut s = Stepped::extrusion_span(plane, profiles[0].clone(), lo, hi);
        s.slabs = vec![profiles];
        s
    }

    /// A single slab from `lo` to `hi` (offsets from the plane along its normal; `lo < hi`): an
    /// extrusion that goes both ways, or sits symmetric about the plane.
    pub fn extrusion_span(plane: Plane, profile: Profile, lo: f64, hi: f64) -> Stepped {
        Stepped {
            plane,
            levels: vec![
                Level {
                    w: lo,
                    owner: String::new(),
                    role: Role::Start,
                },
                Level {
                    w: hi,
                    owner: String::new(),
                    role: Role::End,
                },
            ],
            slabs: vec![vec![profile]],
            rims: Vec::new(),
        }
    }

    pub fn min_w(&self) -> f64 {
        self.levels[0].w
    }

    pub fn max_w(&self) -> f64 {
        self.levels[self.levels.len() - 1].w
    }

    /// Splits the slab containing `w` (or extends the stack with an empty slab if `w` lies beyond
    /// it). Does nothing if a level is already there.
    fn insert_level(&mut self, w: f64, owner: &str, role: Role) {
        if self.levels.iter().any(|l| near(l.w, w)) {
            return;
        }
        let level = Level {
            w,
            owner: owner.to_owned(),
            role,
        };
        if w < self.min_w() {
            self.levels.insert(0, level);
            self.slabs.insert(0, Vec::new());
        } else if w > self.max_w() {
            self.levels.push(level);
            self.slabs.push(Vec::new());
        } else {
            let k = self
                .levels
                .iter()
                .position(|l| l.w > w)
                .expect("inside the range")
                - 1;
            self.levels.insert(k + 1, level);
            let copy = self.slabs[k].clone();
            self.slabs.insert(k + 1, copy);
        }
    }

    /// Merges neighbouring slabs with identical material and drops the levels between them.
    /// Merges neighbouring layers that are identical (for builders outside this module).
    pub fn simplify_public(&mut self) {
        self.simplify();
    }

    fn simplify(&mut self) {
        let mut k = 0;
        while k + 1 < self.slabs.len() {
            if self.slabs[k] == self.slabs[k + 1] {
                self.slabs.remove(k + 1);
                self.levels.remove(k + 1);
            } else {
                k += 1;
            }
        }
    }

    fn slab_range(&self, k: usize) -> (f64, f64) {
        (self.levels[k].w, self.levels[k + 1].w)
    }

    /// Removes `tool` (already in this model's 2D frame) over the span from `start` to `end`
    /// along the axis; `end` may be infinite for "through all" (clipped to the part).
    pub fn cut(
        &self,
        tool: &Profile,
        start: f64,
        end: f64,
        owner: &str,
    ) -> Result<Stepped, StepError> {
        self.cut_with(tool, start, end, owner, false)
    }

    /// [`cut`](Self::cut), or with `outside` set the opposite: everything *outside* the tool's
    /// outline (within the span) is removed and only what is inside it stays.
    pub fn cut_with(
        &self,
        tool: &Profile,
        start: f64,
        end: f64,
        owner: &str,
        outside: bool,
    ) -> Result<Stepped, StepError> {
        let (lo, hi) = (start.min(end), start.max(end));
        let (clip_lo, clip_hi) = (lo.max(self.min_w()), hi.min(self.max_w()));
        if clip_hi - clip_lo <= Tolerance::DEFAULT.linear {
            return Err(StepError::Nothing(format!(
                "the cut spans {} to {} along the axis, but the part spans {} to {}: they do not overlap",
                fmt_w(lo),
                fmt_w(hi),
                fmt_w(self.min_w()),
                fmt_w(self.max_w())
            )));
        }
        let mut out = self.clone();
        // New levels only where the tool's own ends fall inside the part.
        if lo.is_finite() && lo > self.min_w() + Tolerance::DEFAULT.linear {
            out.insert_level(
                lo,
                owner,
                if start <= end { Role::Start } else { Role::End },
            );
        }
        if hi.is_finite() && hi < self.max_w() - Tolerance::DEFAULT.linear {
            out.insert_level(
                hi,
                owner,
                if start <= end { Role::End } else { Role::Start },
            );
        }
        let prefix = format!("cut:{owner}:");
        let mut touched = false;
        for k in 0..out.slabs.len() {
            let (a, b) = out.slab_range(k);
            if a < clip_lo - Tolerance::DEFAULT.linear || b > clip_hi + Tolerance::DEFAULT.linear {
                continue;
            }
            let mut next = Vec::new();
            for p in &out.slabs[k] {
                let r = subtract(p, tool, &prefix)?;
                if !outside {
                    touched |= r.changed;
                    next.extend(r.profiles);
                    continue;
                }
                // Keeping what is inside the tool: p minus (p minus tool).
                let mut kept = vec![p.clone()];
                for gone in &r.profiles {
                    let mut again = Vec::new();
                    for q in &kept {
                        again.extend(subtract(q, gone, "")?.profiles);
                    }
                    kept = again;
                }
                touched |= kept.len() != 1 || kept[0] != *p;
                next.extend(kept);
            }
            out.slabs[k] = next;
        }
        if !touched {
            return Err(StepError::Nothing(
                "the cut profile does not overlap any material in that depth range, so it would remove nothing".into(),
            ));
        }
        if out.slabs.iter().all(Vec::is_empty) {
            return Err(StepError::Nothing(
                "the cut would remove the entire part: nothing would be left".into(),
            ));
        }
        out.simplify();
        Ok(out)
    }

    /// Adds `tool` over the span from `start` to `end` (both finite). It must touch the part.
    pub fn boss(
        &self,
        tool: &Profile,
        start: f64,
        end: f64,
        owner: &str,
    ) -> Result<Stepped, StepError> {
        let (lo, hi) = (start.min(end), start.max(end));
        if !(lo.is_finite() && hi.is_finite()) || hi - lo <= Tolerance::DEFAULT.linear {
            return Err(StepError::Nothing(
                "a boss needs a finite, positive depth".into(),
            ));
        }
        let (r_lo, r_hi) = if start <= end {
            (Role::Start, Role::End)
        } else {
            (Role::End, Role::Start)
        };
        let mut out = self.clone();
        out.insert_level(lo, owner, r_lo);
        out.insert_level(hi, owner, r_hi);

        // Contact: the tool must share area with material inside its span, or sit directly on
        // the face just below or just above it.
        let mut contact = false;
        for k in 0..out.slabs.len() {
            let (a, b) = out.slab_range(k);
            let inside = a >= lo - Tolerance::DEFAULT.linear && b <= hi + Tolerance::DEFAULT.linear;
            let below = near(b, lo);
            let above = near(a, hi);
            if inside || below || above {
                for p in &out.slabs[k] {
                    contact |= dc_sketch::overlaps(tool, p)? || dc_sketch::overlaps(p, tool)?;
                }
            }
        }
        if !contact {
            return Err(StepError::NoContact(
                "the boss does not touch the part: its region shares no area with the material in its depth range or on the faces it starts and ends at".into(),
            ));
        }
        // Name the boss's edges after the boss once, up front. (Prefixing inside the union would
        // miss a boss that lands in an empty slab, where there is nothing to union with, and its
        // walls would then collide with the part's own wall names.)
        let tool = prefixed(tool, &format!("boss:{owner}:"));
        let before = out.slabs.clone();
        for k in 0..out.slabs.len() {
            let (a, b) = out.slab_range(k);
            if a >= lo - Tolerance::DEFAULT.linear && b <= hi + Tolerance::DEFAULT.linear {
                out.slabs[k] = union_all(&out.slabs[k], &tool, "")?;
            }
        }
        if out.slabs == before {
            return Err(StepError::Nothing(
                "the boss lies entirely inside existing material, so it adds nothing".into(),
            ));
        }
        out.simplify();
        Ok(out)
    }
}

/// A copy of `p` with `prefix` in front of every edge ID.
fn prefixed(p: &Profile, prefix: &str) -> Profile {
    let mut q = p.clone();
    for l in std::iter::once(&mut q.outer).chain(q.holes.iter_mut()) {
        for e in &mut l.edges {
            e.id = format!("{prefix}{}", e.id);
        }
    }
    q.id = format!("{prefix}{}", q.id);
    q
}

fn fmt_w(w: f64) -> String {
    if w.is_infinite() {
        if w > 0.0 {
            "+inf".into()
        } else {
            "-inf".into()
        }
    } else {
        format!("{w:.4}")
    }
}

/// `list ∪ tool`: merges the tool into every profile it touches, repeating until stable.
fn union_all(list: &[Profile], tool: &Profile, prefix: &str) -> Result<Vec<Profile>, StepError> {
    let mut merged = tool.clone();
    let mut rest: Vec<Profile> = list.to_vec();
    loop {
        let mut progressed = false;
        let mut keep = Vec::new();
        for p in rest {
            let r = union(&p, &merged, prefix)?;
            if r.changed {
                if r.profiles.len() != 1 {
                    return Err(StepError::Region(RegionError::Unsupported(
                        "adding the boss would split the material into several regions".into(),
                    )));
                }
                merged = r.profiles.into_iter().next().expect("one profile");
                progressed = true;
            } else {
                keep.push(p);
            }
        }
        rest = keep;
        if !progressed {
            break;
        }
    }
    rest.push(merged);
    rest.sort_by(|a, b| a.id.cmp(&b.id));
    Ok(rest)
}

// ---------------------------------------------------------------------------------------------
// Building the B-rep
// ---------------------------------------------------------------------------------------------

/// A directed boundary curve in the model plane.
#[derive(Clone, Debug)]
enum G2 {
    Line {
        a: Vec2,
        b: Vec2,
    },
    /// Signed sweep: positive is counter-clockwise.
    Arc {
        c: Vec2,
        r: f64,
        a0: f64,
        sweep: f64,
    },
}

fn v2(p: [f64; 2]) -> Vec2 {
    Vec2::new(p[0], p[1])
}

fn polar(c: Vec2, r: f64, t: f64) -> Vec2 {
    Vec2::new(c.x + r * det::cos(t), c.y + r * det::sin(t))
}

impl G2 {
    fn of(e: &LoopEdge) -> G2 {
        match &e.curve {
            Curve2::Line => G2::Line {
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
                    G2::Arc {
                        c: v2(*center),
                        r: *radius,
                        a0: start_angle + sweep,
                        sweep: -sweep,
                    }
                } else {
                    G2::Arc {
                        c: v2(*center),
                        r: *radius,
                        a0: *start_angle,
                        sweep: *sweep,
                    }
                }
            }
            Curve2::Circle { center, radius } => G2::Arc {
                c: v2(*center),
                r: *radius,
                a0: 0.0,
                sweep: TWO_PI,
            },
        }
    }

    fn reversed(&self) -> G2 {
        match self {
            G2::Line { a, b } => G2::Line { a: *b, b: *a },
            G2::Arc { c, r, a0, sweep } => G2::Arc {
                c: *c,
                r: *r,
                a0: a0 + sweep,
                sweep: -sweep,
            },
        }
    }

    fn start(&self) -> Vec2 {
        match self {
            G2::Line { a, .. } => *a,
            G2::Arc { c, r, a0, .. } => polar(*c, *r, *a0),
        }
    }

    fn end(&self) -> Vec2 {
        match self {
            G2::Line { b, .. } => *b,
            G2::Arc { c, r, a0, sweep } => polar(*c, *r, a0 + sweep),
        }
    }

    fn mid(&self) -> Vec2 {
        match self {
            G2::Line { a, b } => (*a + *b) * 0.5,
            G2::Arc { c, r, a0, sweep } => polar(*c, *r, a0 + sweep * 0.5),
        }
    }

    /// −1, 0 (line) or +1: the direction of travel around an arc's centre.
    fn turn(&self) -> i8 {
        match self {
            G2::Line { .. } => 0,
            G2::Arc { sweep, .. } => {
                if *sweep >= 0.0 {
                    1
                } else {
                    -1
                }
            }
        }
    }
}

/// Clusters points so that equal points get one index.
#[derive(Default)]
struct Cluster {
    pts: Vec<Vec2>,
}

impl Cluster {
    fn id(&mut self, p: Vec2) -> usize {
        match self.pts.iter().position(|q| (*q - p).length() <= MERGE) {
            Some(i) => i,
            None => {
                self.pts.push(p);
                self.pts.len() - 1
            }
        }
    }
}

/// Identity of a boundary piece regardless of direction: its two end nodes and its midpoint.
type PieceKey = (usize, usize, usize);

/// A boundary piece as one slab sees it, oriented so material lies on its left.
struct Inst {
    slab: usize,
    name: String,
    geo: G2,
    from: usize,
    to: usize,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Side {
    Left,
    Right,
}

struct Canon {
    geo: G2,
    from: usize,
    to: usize,
    turn: i8,
}

struct Builder<'a> {
    st: &'a Stepped,
    nodes: Cluster,
    mids: Cluster,
    solid: Solid,
    vertices: BTreeMap<(usize, usize), Id>,
    hedges: BTreeMap<(usize, PieceKey), Id>,
    rises: BTreeMap<(usize, usize), Id>,
    canon: BTreeMap<PieceKey, Canon>,
}

fn diff_lists(a: &[Profile], b: &[Profile]) -> Result<Vec<Profile>, RegionError> {
    let mut out = Vec::new();
    for p in a {
        let mut cur = vec![p.clone()];
        for q in b {
            let mut next = Vec::new();
            for c in &cur {
                next.extend(subtract(c, q, "")?.profiles);
            }
            cur = next;
        }
        out.extend(cur);
    }
    Ok(out)
}

impl<'a> Builder<'a> {
    fn node(&mut self, p: Vec2) -> usize {
        self.nodes.id(p)
    }

    fn key_of(&mut self, geo: &G2) -> (PieceKey, usize, usize) {
        let (f, t) = (self.node(geo.start()), self.node(geo.end()));
        let m = self.mids.id(geo.mid());
        ((f.min(t), f.max(t), m), f, t)
    }

    fn pos3(&self, level: usize, p: Vec2) -> Vec3 {
        self.st.plane.to_3d(p) + self.st.plane.normal() * self.st.levels[level].w
    }

    fn vertex(&mut self, level: usize, node: usize) -> Id {
        if let Some(v) = self.vertices.get(&(level, node)) {
            return *v;
        }
        let id = self.solid.vertices.len() as Id;
        let p = self.pos3(level, self.nodes.pts[node]);
        self.solid.vertices.push(p);
        self.vertices.insert((level, node), id);
        id
    }

    fn rise(&mut self, slab: usize, node: usize) -> Id {
        if let Some(e) = self.rises.get(&(slab, node)) {
            return *e;
        }
        let (v0, v1) = (self.vertex(slab, node), self.vertex(slab + 1, node));
        let id = self.solid.edges.len() as Id;
        self.solid.edges.push(Edge {
            name: format!("rise:{slab}:{node}"),
            v0,
            v1,
            curve: Curve::Line,
        });
        self.rises.insert((slab, node), id);
        id
    }

    /// The horizontal edge at `level` along a piece, in the piece's canonical direction.
    fn hedge(&mut self, level: usize, key: PieceKey, name: &str) -> Id {
        if let Some(e) = self.hedges.get(&(level, key)) {
            return *e;
        }
        let c = self.canon.get(&key).expect("piece registered");
        let (geo, from, to) = (c.geo.clone(), c.from, c.to);
        let (v0, v1) = (self.vertex(level, from), self.vertex(level, to));
        let n = self.st.plane.normal();
        let shift = n * self.st.levels[level].w;
        let curve = match &geo {
            G2::Line { .. } => Curve::Line,
            G2::Arc { c, r, a0, sweep } => {
                let centre = self.st.plane.to_3d(*c) + shift;
                // Counter-clockwise about +n, or about −n for clockwise travel.
                if *sweep >= 0.0 {
                    Curve::Circle {
                        center: centre,
                        axis: n,
                        x_dir: self.st.plane.x_axis,
                        radius: *r,
                        t0: *a0,
                        sweep: *sweep,
                    }
                } else {
                    Curve::Circle {
                        center: centre,
                        axis: -n,
                        x_dir: self.st.plane.x_axis,
                        radius: *r,
                        t0: -a0,
                        sweep: -sweep,
                    }
                }
            }
        };
        let id = self.solid.edges.len() as Id;
        self.solid.edges.push(Edge {
            name: format!("h{level}:{name}"),
            v0,
            v1,
            curve,
        });
        self.hedges.insert((level, key), id);
        id
    }

    /// Is travelling `geo` (from node `f` to node `t`) the same way as the piece's canonical direction?
    fn forward(&self, key: PieceKey, f: usize, t: usize, geo: &G2) -> bool {
        let c = &self.canon[&key];
        c.from == f && c.to == t && c.turn == geo.turn()
    }
}

/// Boundary pieces of a profile, oriented so the material is on the left (holes run clockwise).
fn material_left(p: &Profile) -> Vec<(String, G2)> {
    let mut out = Vec::new();
    for e in &p.outer.edges {
        out.push((e.id.clone(), G2::of(e)));
    }
    for h in &p.holes {
        for e in &h.edges {
            out.push((e.id.clone(), G2::of(e).reversed()));
        }
    }
    out
}

/// How an edge is rounded.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Round {
    /// A circular arc of the given radius, tangent to both faces.
    Fillet,
    /// A flat bevel, the given distance from the edge along each face.
    Chamfer,
}

impl Round {
    /// The word for it, also used in the names of the faces it makes.
    pub fn label(self) -> &'static str {
        match self {
            Round::Fillet => "fillet",
            Round::Chamfer => "chamfer",
        }
    }
}

/// Area of a loop from its edges (counter-clockwise loops are positive).
fn loop_area(edges: &[LoopEdge]) -> f64 {
    let mut total = 0.0;
    for e in edges {
        let term = match &e.curve {
            Curve2::Line => 0.5 * (e.from[0] * e.to[1] - e.to[0] * e.from[1]),
            Curve2::Circle { radius, .. } => core::f64::consts::PI * radius * radius,
            Curve2::Arc {
                center,
                radius,
                start_angle,
                sweep,
            } => {
                // The contribution of the arc in its natural direction, ½∮(x dy − y dx).
                let t1 = start_angle + sweep;
                let natural = 0.5
                    * (radius * radius * sweep
                        + radius
                            * (center[0] * (det::sin(t1) - det::sin(*start_angle))
                                - center[1] * (det::cos(t1) - det::cos(*start_angle))));
                if e.reversed {
                    -natural
                } else {
                    natural
                }
            }
        };
        total += term;
    }
    total
}

impl Stepped {
    /// Rounds the edge that stands on `corner` (in this model's 2D frame) and runs parallel to
    /// the stack's axis from `lo` to `hi`: in every layer in that range, the corner of the
    /// outline at that point is replaced by an arc or a bevel `size` from it. This is exact
    /// for such edges because they are corners of the 2D outlines; edges across the axis would
    /// need other surfaces than planes and cylinders.
    ///
    /// Rounding works on corners between two straight edges, convex or concave. The new edge is
    /// named `name`, the same in every layer, so the faces it makes join into one tall face.
    pub fn round_edge(
        &self,
        corner: Vec2,
        lo: f64,
        hi: f64,
        size: f64,
        kind: Round,
        name: &str,
    ) -> Result<Stepped, StepError> {
        if !size.is_finite() || size <= 0.0 {
            return Err(StepError::Nothing(format!(
                "the size must be greater than zero, got {size}"
            )));
        }
        let lin = Tolerance::DEFAULT.linear;
        let mut out = self.clone();
        let mut layers = 0;
        for k in 0..out.slabs.len() {
            let (a, b) = out.slab_range(k);
            if a < lo - lin || b > hi + lin {
                continue;
            }
            layers += 1;
            let mut found = 0;
            for p in &mut out.slabs[k] {
                let mut loops: Vec<&mut Loop> = vec![&mut p.outer];
                loops.extend(p.holes.iter_mut());
                for l in loops {
                    if round_in_loop(l, corner, size, kind, name)? {
                        found += 1;
                    }
                }
            }
            if found == 0 {
                return Err(StepError::Nothing(format!(
                    "the edge does not stand in every layer it should: there is no corner at ({:.4}, {:.4}) between {} and {} along the axis",
                    corner.x,
                    corner.y,
                    fmt_w(a),
                    fmt_w(b)
                )));
            }
            for p in &mut out.slabs[k] {
                for l in core::iter::once(&mut p.outer).chain(p.holes.iter_mut()) {
                    l.area = loop_area(&l.edges);
                }
                p.area = p.outer.area - p.holes.iter().map(|h| h.area).sum::<f64>();
            }
        }
        if layers == 0 {
            return Err(StepError::Nothing(format!(
                "no layer of the part lies between {} and {} along the axis",
                fmt_w(lo),
                fmt_w(hi)
            )));
        }
        out.simplify();
        Ok(out)
    }
}

/// Rounds the corner of `l` at `corner`, if it has one; whether it did.
fn round_in_loop(
    l: &mut Loop,
    corner: Vec2,
    size: f64,
    kind: Round,
    name: &str,
) -> Result<bool, StepError> {
    let n = l.edges.len();
    let lin = 1e-6;
    let Some(i) = (0..n).find(|&i| {
        let (e1, e2) = (&l.edges[i], &l.edges[(i + 1) % n]);
        (v2(e1.to) - corner).length() < lin && (v2(e2.from) - corner).length() < lin
    }) else {
        return Ok(false);
    };
    let j = (i + 1) % n;
    let (e1, e2) = (l.edges[i].clone(), l.edges[j].clone());
    for e in [&e1, &e2] {
        if !matches!(e.curve, Curve2::Line) {
            return Err(StepError::Nothing(format!(
                "edge '{}' next to the corner is not straight: only corners between two straight edges can be rounded for now",
                e.id
            )));
        }
    }
    let (l1, l2) = (
        (corner - v2(e1.from)).length(),
        (v2(e2.to) - corner).length(),
    );
    // Directions away from the corner along each edge.
    let u1 = (v2(e1.from) - corner) / l1;
    let u2 = (v2(e2.to) - corner) / l2;
    let turn = (-u1).cross(u2); // > 0: the loop turns left here
    let cos_t = u1.dot(u2).clamp(-1.0, 1.0);
    let theta = det::acos(cos_t); // the angle between the two edges, 0..pi
    if turn.abs() < 1e-9 || theta < 1e-6 || core::f64::consts::PI - theta < 1e-6 {
        return Err(StepError::Nothing(format!(
            "the edges '{}' and '{}' run straight on, so there is no corner to round",
            e1.id, e2.id
        )));
    }
    let t = match kind {
        Round::Fillet => size / det::tan(theta / 2.0),
        Round::Chamfer => size,
    };
    for (len, e) in [(l1, &e1), (l2, &e2)] {
        if t >= len - 1e-6 {
            return Err(StepError::Nothing(format!(
                "{} {} is too big here: it takes {:.4} along '{}', which is only {:.4} long",
                if kind == Round::Fillet {
                    "a radius of"
                } else {
                    "a distance of"
                },
                size,
                t,
                e.id,
                len
            )));
        }
    }
    let a = corner + u1 * t;
    let b = corner + u2 * t;
    let middle = match kind {
        Round::Chamfer => LoopEdge {
            id: name.to_owned(),
            reversed: false,
            from: [a.x, a.y],
            to: [b.x, b.y],
            curve: Curve2::Line,
        },
        Round::Fillet => {
            let bisector = (u1 + u2) / (u1 + u2).length();
            let centre = corner + bisector * (size / det::sin(theta / 2.0));
            let sweep = core::f64::consts::PI - theta;
            let left = turn > 0.0;
            // Counter-clockwise from A to B round the centre on a left turn; on a right turn
            // the loop goes clockwise, which is the arc's natural direction run backwards.
            let start = if left {
                det::atan2(a.y - centre.y, a.x - centre.x)
            } else {
                det::atan2(b.y - centre.y, b.x - centre.x)
            };
            LoopEdge {
                id: name.to_owned(),
                reversed: !left,
                from: [a.x, a.y],
                to: [b.x, b.y],
                curve: Curve2::Arc {
                    center: [centre.x, centre.y],
                    radius: size,
                    start_angle: start,
                    sweep,
                },
            }
        }
    };
    l.edges[i].to = [a.x, a.y];
    l.edges[j].from = [b.x, b.y];
    l.edges.insert(i + 1, middle);
    Ok(true)
}

/// Builds the B-rep of a stack and validates it. A stack that does not give a valid solid is an
/// error with the reasons, never a solid that looks right but is not.
pub fn build_solid(st: &Stepped) -> Result<Solid, StepError> {
    let n_slabs = st.slabs.len();
    // 1. Refine every region against every other slab's regions, so coincident boundary pieces
    //    line up exactly between slabs.
    let mut refined: Vec<Vec<Profile>> = Vec::with_capacity(n_slabs);
    for (k, slab) in st.slabs.iter().enumerate() {
        let others: Vec<&Profile> = st
            .slabs
            .iter()
            .enumerate()
            .filter(|(j, _)| *j != k)
            .flat_map(|(_, s)| s.iter())
            .collect();
        let mut r = Vec::new();
        for p in slab {
            r.push(refine(p, &others)?);
        }
        refined.push(r);
    }

    let mut b = Builder {
        st,
        nodes: Cluster::default(),
        mids: Cluster::default(),
        solid: Solid::default(),
        vertices: BTreeMap::new(),
        hedges: BTreeMap::new(),
        rises: BTreeMap::new(),
        canon: BTreeMap::new(),
    };

    // 2. Boundary pieces per slab, grouped by geometric identity.
    let mut by_key: BTreeMap<PieceKey, Vec<Inst>> = BTreeMap::new();
    for (k, slab) in refined.iter().enumerate() {
        for p in slab {
            for (name, geo) in material_left(p) {
                let (key, from, to) = b.key_of(&geo);
                b.canon.entry(key).or_insert_with(|| Canon {
                    geo: geo.clone(),
                    from,
                    to,
                    turn: geo.turn(),
                });
                by_key.entry(key).or_default().push(Inst {
                    slab: k,
                    name,
                    geo,
                    from,
                    to,
                });
            }
        }
    }

    // 3. Walls: runs of consecutive slabs where a piece has material on the same side.
    let n = st.plane.normal();
    let mut walls: Vec<Face> = Vec::new();
    for (key, insts) in &by_key {
        let mut insts: Vec<&Inst> = insts.iter().collect();
        insts.sort_by_key(|i| i.slab);
        let sides: Vec<(usize, Side, &str)> = insts
            .iter()
            .map(|i| {
                let s = if b.forward(*key, i.from, i.to, &i.geo) {
                    Side::Left
                } else {
                    Side::Right
                };
                (i.slab, s, i.name.as_str())
            })
            .collect();
        let mut runs: Vec<(usize, usize, Side, String)> = Vec::new();
        for (slab, side, name) in sides {
            match runs.last_mut() {
                Some(r) if r.1 + 1 == slab && r.2 == side => r.1 = slab,
                _ => runs.push((slab, slab, side, name.to_owned())),
            }
        }
        let many = runs.len() > 1;
        for (ri, (ka, kb, side, name)) in runs.iter().enumerate() {
            let (ka, kb) = (*ka, *kb);
            let cg = b.canon[key].geo.clone();
            let (a, bn) = (b.canon[key].from, b.canon[key].to);
            let lo = b.hedge(ka, *key, name);
            let hi = b.hedge(kb + 1, *key, name);
            let mut seq: Vec<CoEdge> = Vec::new();
            match side {
                Side::Left => {
                    seq.push(CoEdge {
                        edge: lo,
                        reversed: false,
                    });
                    for s in ka..=kb {
                        seq.push(CoEdge {
                            edge: b.rise(s, bn),
                            reversed: false,
                        });
                    }
                    seq.push(CoEdge {
                        edge: hi,
                        reversed: true,
                    });
                    for s in (ka..=kb).rev() {
                        seq.push(CoEdge {
                            edge: b.rise(s, a),
                            reversed: true,
                        });
                    }
                }
                Side::Right => {
                    seq.push(CoEdge {
                        edge: hi,
                        reversed: false,
                    });
                    for s in (ka..=kb).rev() {
                        seq.push(CoEdge {
                            edge: b.rise(s, bn),
                            reversed: true,
                        });
                    }
                    seq.push(CoEdge {
                        edge: lo,
                        reversed: true,
                    });
                    for s in ka..=kb {
                        seq.push(CoEdge {
                            edge: b.rise(s, a),
                            reversed: false,
                        });
                    }
                }
            }
            let (w0, w1) = (st.levels[ka].w, st.levels[kb + 1].w);
            let _ = (many, ri);
            let face_name = format!("side:{name}");
            let (surface, same_sense) = match &cg {
                G2::Line { a: pa, b: pb } => {
                    let d = *pb - *pa;
                    let t = (st.plane.x_axis * d.x + st.plane.y_axis * d.y) / d.length();
                    let normal = if *side == Side::Left {
                        t.cross(n)
                    } else {
                        n.cross(t)
                    };
                    (
                        Surface::Plane {
                            origin: b.pos3(ka, *pa),
                            normal,
                            x_dir: t,
                        },
                        true,
                    )
                }
                G2::Arc { c, r, a0, sweep } => {
                    let (phi0, phi1) = if *sweep >= 0.0 {
                        (*a0, a0 + sweep)
                    } else {
                        (a0 + sweep, *a0)
                    };
                    (
                        Surface::Cylinder {
                            origin: st.plane.to_3d(*c),
                            axis: n,
                            ref_dir: st.plane.x_axis,
                            radius: *r,
                            phi0,
                            phi1,
                            h0: w0,
                            h1: w1,
                        },
                        (*sweep >= 0.0) == (*side == Side::Left),
                    )
                }
            };
            walls.push(Face {
                name: face_name,
                surface,
                same_sense,
                loops: vec![seq],
            });
        }
    }

    // 3b. Caps: where material ends above a level (a top cap) or begins above it (a bottom cap).
    let mut caps: Vec<Face> = Vec::new();
    for j in 0..st.levels.len() {
        let below: &[Profile] = if j > 0 { &refined[j - 1] } else { &[] };
        let above: &[Profile] = if j < n_slabs { &refined[j] } else { &[] };
        for (outward_up, regions) in [
            (true, diff_lists(below, above)?),
            (false, diff_lists(above, below)?),
        ] {
            for p in regions {
                let lvl = &st.levels[j];
                let role = match lvl.role {
                    Role::Start => "cap_start",
                    Role::End => "cap_end",
                };
                let base = if lvl.owner.is_empty() {
                    role.to_owned()
                } else {
                    format!("{role}:{}", lvl.owner)
                };
                let mut loops: Vec<Vec<CoEdge>> = Vec::new();
                let push_loop = |edges: &[LoopEdge], hole: bool, b: &mut Builder<'_>| {
                    // Outward +n: outer loop counter-clockwise (as stored), holes clockwise.
                    let reverse_order = hole == outward_up;
                    let list: Vec<&LoopEdge> = if reverse_order {
                        edges.iter().rev().collect()
                    } else {
                        edges.iter().collect()
                    };
                    let mut seq = Vec::new();
                    for e in list {
                        let geo = if reverse_order {
                            G2::of(e).reversed()
                        } else {
                            G2::of(e)
                        };
                        let (key, f, t) = b.key_of(&geo);
                        b.canon.entry(key).or_insert_with(|| Canon {
                            geo: geo.clone(),
                            from: f,
                            to: t,
                            turn: geo.turn(),
                        });
                        let edge = b.hedge(j, key, &e.id);
                        seq.push(CoEdge {
                            edge,
                            reversed: !b.forward(key, f, t, &geo),
                        });
                    }
                    seq
                };
                loops.push(push_loop(&p.outer.edges, false, &mut b));
                for h in &p.holes {
                    loops.push(push_loop(&h.edges, true, &mut b));
                }
                caps.push(Face {
                    name: base,
                    surface: Surface::Plane {
                        origin: st.plane.origin + n * lvl.w,
                        normal: n,
                        x_dir: st.plane.x_axis,
                    },
                    same_sense: outward_up,
                    loops,
                });
            }
        }
    }

    // 4. Assemble. Faces that share a base name (a wall or a floor in several pieces) are told
    // apart by their neighbours, not by the order they were built in.
    let mut faces = caps;
    faces.extend(walls);
    b.solid.faces = faces;
    let bases: Vec<String> = b.solid.faces.iter().map(|f| f.name.clone()).collect();
    let (face_names, _) = dc_kernel::naming::structural_names(&b.solid, &bases);
    for (f, n) in b.solid.faces.iter_mut().zip(face_names) {
        f.name = n;
    }
    let issues = b.solid.validate();
    if issues.is_empty() {
        let mut solid = b.solid;
        for r in &st.rims {
            solid = solid
                .round_rim(&r.face_a, &r.face_b, r.kind, r.size, &r.name)
                .map_err(|e| match e {
                    dc_kernel::RimError::NotFound(n) => StepError::Nothing(format!(
                        "the rounded rim '{}' needs the face '{n}', which the part no longer has",
                        r.name
                    )),
                    dc_kernel::RimError::Unsupported(m) => StepError::Unsupported(m),
                    dc_kernel::RimError::TooLarge(m) => StepError::Nothing(m),
                    dc_kernel::RimError::Invalid(v) => StepError::Invalid(v),
                })?;
        }
        Ok(solid)
    } else {
        Err(StepError::Invalid(
            issues.into_iter().map(|i| i.message).collect(),
        ))
    }
}
