//! Trim, extend and split: editing existing sketch geometry at a place the user points at.
//!
//! Each operation is exact (no meshes): shapes are lines, circles and arcs, intersections are
//! computed in closed form, and new points land exactly on the shapes they cut. The operations
//! change entities and constraints only; solving afterwards is the caller's job (the authority
//! does it when committing). New ids are deterministic: the first free `p1`, `p2`, ... .
//!
//! What happens to relations: anything that still holds after the shape changed is kept
//! (horizontal, parallel, tangent, ...); dimensions and relations that depended on the old length
//! or end points (length, equal length, midpoint, anything on a removed point) are dropped, as
//! SolidWorks does. A new end point that lies on another shape is related to it (on line / on
//! curve), or simply reuses that shape's end point when they meet exactly.

use crate::model::{Constraint, Entity, Sketch, SketchError, SketchErrorCode};
use dc_math::det;
use std::collections::BTreeSet;

type P = (f64, f64);
/// Several open polylines (a preview of what an edit removes or adds).
pub type Polylines = Vec<Vec<P>>;

/// Parameter tolerance (fractions of a segment, radians on a circle).
const EPS: f64 = 1e-9;
/// Two positions closer than this (mm) are the same place.
const POS_TOL: f64 = 1e-7;

fn err(msg: impl Into<String>, entities: Vec<String>) -> SketchError {
    SketchError {
        code: SketchErrorCode::InvalidValue,
        message: msg.into(),
        entities,
    }
}

fn dist(a: P, b: P) -> f64 {
    det::hypot(b.0 - a.0, b.1 - a.1)
}

fn angle(c: P, p: P) -> f64 {
    det::atan2(p.1 - c.1, p.0 - c.0)
}

/// `a - b` wrapped into `[0, tau)`.
fn ccw_from(b: f64, a: f64) -> f64 {
    let t = core::f64::consts::TAU;
    let d = (a - b) % t;
    if d < 0.0 {
        d + t
    } else {
        d
    }
}

/// The curve a shape lies on, with the part of it the shape covers.
#[derive(Clone, Debug)]
enum Carrier {
    Line {
        a: P,
        b: P,
        /// An infinite line covers the whole line, not just the segment between a and b.
        inf: bool,
    },
    /// A circle or an arc. `start` is the angle where the shape begins and `sweep` how far it
    /// goes counter-clockwise (tau for a whole circle).
    Round {
        c: P,
        r: f64,
        start: f64,
        sweep: f64,
        full: bool,
    },
}

impl Carrier {
    /// Position along the shape: a fraction of the segment, or an angle from its start.
    fn param(&self, p: P) -> f64 {
        match self {
            Carrier::Line { a, b, .. } => {
                let (dx, dy) = (b.0 - a.0, b.1 - a.1);
                ((p.0 - a.0) * dx + (p.1 - a.1) * dy) / (dx * dx + dy * dy).max(1e-30)
            }
            Carrier::Round { c, start, .. } => ccw_from(*start, angle(*c, p)),
        }
    }

    fn at(&self, s: f64) -> P {
        match self {
            Carrier::Line { a, b, .. } => (a.0 + s * (b.0 - a.0), a.1 + s * (b.1 - a.1)),
            Carrier::Round { c, r, start, .. } => {
                let t = start + s;
                (c.0 + r * det::cos(t), c.1 + r * det::sin(t))
            }
        }
    }

    /// The length of the covered part in parameter units (1 for a segment).
    fn extent(&self) -> f64 {
        match self {
            Carrier::Line { .. } => 1.0,
            Carrier::Round { sweep, .. } => *sweep,
        }
    }

    fn is_circle(&self) -> bool {
        matches!(self, Carrier::Round { full: true, .. })
    }

    /// Is this point (known to be on the curve) on the part the shape covers?
    fn covers(&self, p: P) -> bool {
        match self {
            Carrier::Line { inf: true, .. } => true,
            Carrier::Line { .. } => {
                let s = self.param(p);
                (-EPS..=1.0 + EPS).contains(&s)
            }
            Carrier::Round { full, sweep, .. } => {
                if *full {
                    return true;
                }
                let s = self.param(p);
                s <= sweep + EPS || s >= core::f64::consts::TAU - EPS
            }
        }
    }

    /// Distance from a point to the covered part.
    fn distance(&self, p: P) -> f64 {
        match self {
            Carrier::Line { a, b, .. } => {
                let s = self.param(p).clamp(0.0, 1.0);
                let _ = (a, b);
                dist(p, self.at(s))
            }
            Carrier::Round {
                c, r, sweep, full, ..
            } => {
                let s = self.param(p);
                if *full || s <= *sweep {
                    (dist(*c, p) - r).abs()
                } else {
                    // Off the end of the arc: nearest end point.
                    dist(p, self.at(0.0)).min(dist(p, self.at(*sweep)))
                }
            }
        }
    }
}

/// Where the full curve of `e` meets the covered part of `o`: `(parameter on e's curve, point)`.
/// Parameters on a circle are in `[0, tau)`; on a line they are unrestricted.
fn hits(e: &Carrier, o: &Carrier, whole_other: bool) -> Vec<(f64, P)> {
    let mut pts: Vec<P> = Vec::new();
    match (e, o) {
        (Carrier::Line { a, b, .. }, Carrier::Line { a: a2, b: b2, .. }) => {
            let d1 = (b.0 - a.0, b.1 - a.1);
            let d2 = (b2.0 - a2.0, b2.1 - a2.1);
            let denom = d1.0 * d2.1 - d1.1 * d2.0;
            let scale = det::hypot(d1.0, d1.1) * det::hypot(d2.0, d2.1);
            if denom.abs() > 1e-12 * scale.max(1e-30) {
                let t = ((a2.0 - a.0) * d2.1 - (a2.1 - a.1) * d2.0) / denom;
                pts.push((a.0 + t * d1.0, a.1 + t * d1.1));
            }
        }
        (Carrier::Line { a, b, .. }, Carrier::Round { c, r, .. }) => {
            for t in line_circle(*a, *b, *c, *r) {
                pts.push((a.0 + t * (b.0 - a.0), a.1 + t * (b.1 - a.1)));
            }
        }
        (Carrier::Round { c, r, .. }, Carrier::Line { a, b, .. }) => {
            for u in line_circle(*a, *b, *c, *r) {
                pts.push((a.0 + u * (b.0 - a.0), a.1 + u * (b.1 - a.1)));
            }
        }
        (Carrier::Round { c: c1, r: r1, .. }, Carrier::Round { c: c2, r: r2, .. }) => {
            let d = dist(*c1, *c2);
            if d > 1e-12 && d <= r1 + r2 + POS_TOL && d >= (r1 - r2).abs() - POS_TOL {
                let a = (r1 * r1 - r2 * r2 + d * d) / (2.0 * d);
                let h = det::sqrt((r1 * r1 - a * a).max(0.0));
                let (ux, uy) = ((c2.0 - c1.0) / d, (c2.1 - c1.1) / d);
                let m = (c1.0 + a * ux, c1.1 + a * uy);
                pts.push((m.0 - h * uy, m.1 + h * ux));
                if h > 1e-12 {
                    pts.push((m.0 + h * uy, m.1 - h * ux));
                }
            }
        }
    }
    let mut out: Vec<(f64, P)> = Vec::new();
    for p in pts {
        if !whole_other && !o.covers(p) {
            continue;
        }
        let s = e.param(p);
        if !out.iter().any(|(t, _)| (t - s).abs() < EPS) {
            out.push((s, p));
        }
    }
    out
}

/// Parameters `t` where `a + t (b - a)` is on the circle (centre `c`, radius `r`).
fn line_circle(a: P, b: P, c: P, r: f64) -> Vec<f64> {
    let d = (b.0 - a.0, b.1 - a.1);
    let f = (a.0 - c.0, a.1 - c.1);
    let qa = d.0 * d.0 + d.1 * d.1;
    if qa < 1e-30 {
        return Vec::new();
    }
    let qb = 2.0 * (f.0 * d.0 + f.1 * d.1);
    let qc = f.0 * f.0 + f.1 * f.1 - r * r;
    let disc = qb * qb - 4.0 * qa * qc;
    let tol = 1e-12 * (qb * qb).max(4.0 * qa * qc.abs()).max(1e-30);
    if disc < -tol {
        return Vec::new();
    }
    let root = det::sqrt(disc.max(0.0));
    let mut out = vec![(-qb - root) / (2.0 * qa)];
    if root > 1e-12 * qa.max(1.0) {
        out.push((-qb + root) / (2.0 * qa));
    }
    out
}

/// A place a shape is cut at by another shape.
#[derive(Clone, Debug)]
struct Cut {
    s: f64,
    p: P,
    /// The shape that cuts here.
    other: String,
    /// An end point of that shape that is exactly here (reused instead of making a new point).
    reuse: Option<String>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum End {
    Start,
    Finish,
}

/// What a trim will do, worked out from the shape, its cuts and where the user pointed.
#[derive(Clone, Debug)]
enum TrimKind {
    /// Nothing cuts it: the whole shape goes.
    DeleteAll,
    /// The piece before `hi` goes; the end point moves to the cut.
    DropFirst { hi: Cut },
    /// The piece after `lo` goes.
    DropLast { lo: Cut },
    /// The piece between two cuts goes; the rest is two shapes.
    DropMiddle { lo: Cut, hi: Cut },
    /// A circle loses the part between two cuts and becomes an arc (`lo` is where the removed
    /// part starts going counter-clockwise, `hi` where it ends).
    CircleToArc { lo: Cut, hi: Cut },
}

struct TrimPlan {
    kind: TrimKind,
    /// Parameter range of the removed part (`from` may exceed `to` on a circle: it wraps).
    from: f64,
    to: f64,
}

/// How a trim or extend decides what cuts a shape.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct TrimOptions {
    /// Construction geometry cuts too (by default only real geometry does, so a centre line
    /// does not chop up the profile it guides).
    pub construction: bool,
    /// If not empty, only these shapes cut: trim "between bounds" ignores everything else.
    pub bounds: Vec<String>,
}

/// What an edit created and removed, so a caller can select the result.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct EditReport {
    pub created: Vec<String>,
    pub removed: Vec<String>,
    /// Relations and dimensions that stopped making sense and went (merging points, say).
    pub dropped: Vec<String>,
}

impl Sketch {
    fn carrier(&self, id: &str) -> Option<Carrier> {
        Some(match self.entity(id)? {
            Entity::Line { a, b } => Carrier::Line {
                a: self.point(a)?,
                b: self.point(b)?,
                inf: self.infinite.contains(id),
            },
            Entity::Circle { center, radius } => Carrier::Round {
                c: self.point(center)?,
                r: *radius,
                start: 0.0,
                sweep: core::f64::consts::TAU,
                full: true,
            },
            Entity::Arc { center, start, end } => {
                let (c, s, e) = (self.point(center)?, self.point(start)?, self.point(end)?);
                Carrier::Round {
                    c,
                    r: dist(c, s),
                    start: angle(c, s),
                    sweep: ccw_from(angle(c, s), angle(c, e)),
                    full: false,
                }
            }
            Entity::Point { .. } => return None,
        })
    }

    /// The nearest line, circle or arc to a place, if one is within `tol` millimetres.
    pub fn entity_near(&self, at: P, tol: f64) -> Option<String> {
        self.entities
            .keys()
            .filter_map(|id| Some((self.carrier(id)?.distance(at), id)))
            .filter(|(d, _)| *d <= tol)
            .min_by(|a, b| a.0.total_cmp(&b.0).then_with(|| a.1.cmp(b.1)))
            .map(|(_, id)| id.clone())
    }

    fn shape_carrier(&self, id: &str) -> Result<Carrier, SketchError> {
        self.carrier(id).ok_or_else(|| {
            err(
                format!("'{id}' is not a line, circle or arc"),
                vec![id.to_owned()],
            )
        })
    }

    /// Every place other shapes cut `id` strictly inside it, ordered along it.
    fn cuts(&self, id: &str, e: &Carrier, opts: &TrimOptions) -> Vec<Cut> {
        let mut out: Vec<Cut> = Vec::new();
        for (oid, ent) in &self.entities {
            if oid == id || !self.may_cut(oid, opts) {
                continue;
            }
            let Some(o) = self.carrier(oid) else { continue };
            let ends: Vec<&String> = match ent {
                Entity::Line { a, b } => vec![a, b],
                Entity::Arc { start, end, .. } => vec![start, end],
                _ => Vec::new(),
            };
            for (s, p) in hits(e, &o, false) {
                let interior = if e.is_circle() {
                    true
                } else {
                    s > EPS && s < e.extent() - EPS
                };
                if !interior {
                    continue;
                }
                let s = if e.is_circle() && s >= core::f64::consts::TAU - EPS {
                    0.0
                } else {
                    s
                };
                let reuse = ends
                    .iter()
                    .find(|q| self.point(q).is_some_and(|qp| dist(qp, p) < POS_TOL))
                    .map(|q| (*q).clone());
                if let Some(c) = out.iter_mut().find(|c| (c.s - s).abs() < EPS) {
                    if c.reuse.is_none() {
                        c.reuse = reuse;
                    }
                    continue;
                }
                out.push(Cut {
                    s,
                    p,
                    other: oid.clone(),
                    reuse,
                });
            }
        }
        out.sort_by(|a, b| a.s.total_cmp(&b.s));
        out
    }

    /// Whether shape `oid` takes part as a cutter under these options.
    fn may_cut(&self, oid: &str, opts: &TrimOptions) -> bool {
        if !opts.bounds.is_empty() {
            return opts.bounds.iter().any(|b| b == oid);
        }
        opts.construction || !self.is_construction(oid)
    }

    fn plan_trim(
        &self,
        id: &str,
        at: P,
        opts: &TrimOptions,
    ) -> Result<(Carrier, TrimPlan), SketchError> {
        self.refuse_infinite(id, "trim")?;
        let e = self.shape_carrier(id)?;
        let cuts = self.cuts(id, &e, opts);
        let extent = e.extent();
        let s = match &e {
            Carrier::Line { .. } => e.param(at).clamp(0.0, 1.0),
            Carrier::Round { sweep, full, .. } => {
                let s = e.param(at);
                if *full || s <= *sweep {
                    s
                } else if s - sweep < core::f64::consts::TAU - s {
                    *sweep
                } else {
                    0.0
                }
            }
        };
        if cuts.is_empty() {
            return Ok((
                e,
                TrimPlan {
                    kind: TrimKind::DeleteAll,
                    from: 0.0,
                    to: extent,
                },
            ));
        }
        if e.is_circle() {
            if cuts.len() < 2 {
                return Err(err(
                    "a circle needs two other shapes crossing it to be trimmed",
                    vec![id.to_owned()],
                ));
            }
            // The cut at or before the click (wrapping to the last), and the one after it.
            let i = cuts
                .iter()
                .rposition(|c| c.s <= s)
                .unwrap_or(cuts.len() - 1);
            let (lo, hi) = (cuts[i].clone(), cuts[(i + 1) % cuts.len()].clone());
            let (from, to) = (lo.s, hi.s);
            return Ok((
                e,
                TrimPlan {
                    kind: TrimKind::CircleToArc { lo, hi },
                    from,
                    to,
                },
            ));
        }
        let before = cuts.iter().rposition(|c| c.s <= s);
        let after = cuts.iter().position(|c| c.s > s);
        let (lo, hi) = (
            before.map(|i| cuts[i].clone()),
            after.map(|i| cuts[i].clone()),
        );
        let plan = match (lo, hi) {
            (None, Some(hi)) => TrimPlan {
                from: 0.0,
                to: hi.s,
                kind: TrimKind::DropFirst { hi },
            },
            (Some(lo), None) => TrimPlan {
                from: lo.s,
                to: extent,
                kind: TrimKind::DropLast { lo },
            },
            (Some(lo), Some(hi)) => TrimPlan {
                from: lo.s,
                to: hi.s,
                kind: TrimKind::DropMiddle { lo, hi },
            },
            (None, None) => unreachable!("there is at least one cut"),
        };
        Ok((e, plan))
    }

    /// The points of the part a trim at `at` would remove, for drawing a preview.
    pub fn trim_preview(&self, id: &str, at: P) -> Result<Vec<P>, SketchError> {
        self.trim_preview_with(id, at, &TrimOptions::default())
    }

    /// [`trim_preview`](Self::trim_preview) with options.
    pub fn trim_preview_with(
        &self,
        id: &str,
        at: P,
        opts: &TrimOptions,
    ) -> Result<Vec<P>, SketchError> {
        let (e, plan) = self.plan_trim(id, at, opts)?;
        let (from, mut to) = (plan.from, plan.to);
        if to < from {
            to += core::f64::consts::TAU;
        }
        let n = if matches!(e, Carrier::Line { .. }) {
            1
        } else {
            48
        };
        Ok((0..=n)
            .map(|i| e.at(from + (to - from) * f64::from(i) / f64::from(n)))
            .collect())
    }

    /// Removes the part of a line, arc or circle between the cuts around `at`, as in SolidWorks'
    /// "trim to closest". With nothing cutting it, the whole shape is deleted.
    pub fn trim(&mut self, id: &str, at: P) -> Result<EditReport, SketchError> {
        self.trim_with(id, at, &TrimOptions::default())
    }

    /// [`trim`](Self::trim) with options (construction cutting, trimming between chosen bounds).
    pub fn trim_with(
        &mut self,
        id: &str,
        at: P,
        opts: &TrimOptions,
    ) -> Result<EditReport, SketchError> {
        let (_, plan) = self.plan_trim(id, at, opts)?;
        let mut rep = EditReport::default();
        match plan.kind {
            TrimKind::DeleteAll => {
                self.delete_shape(id, &mut rep);
            }
            TrimKind::DropFirst { hi } => {
                let p = self.cut_point(&hi, &mut rep);
                self.replace_end(id, End::Start, &p);
                self.after_shorten(id);
            }
            TrimKind::DropLast { lo } => {
                let p = self.cut_point(&lo, &mut rep);
                self.replace_end(id, End::Finish, &p);
                self.after_shorten(id);
            }
            TrimKind::DropMiddle { lo, hi } => {
                let (pl, ph) = (self.cut_point(&lo, &mut rep), self.cut_point(&hi, &mut rep));
                let second = self.split_off(id, &ph, &mut rep);
                self.replace_end(id, End::Finish, &pl);
                self.after_shorten(id);
                self.tie_halves(id, &second);
            }
            TrimKind::CircleToArc { lo, hi } => {
                let (pl, ph) = (self.cut_point(&lo, &mut rep), self.cut_point(&hi, &mut rep));
                let Some(Entity::Circle { center, .. }) = self.entity(id).cloned() else {
                    return Err(err("not a circle", vec![id.to_owned()]));
                };
                // The part that stays runs from where the removed part ends round to where it starts.
                self.entities.insert(
                    id.to_owned(),
                    Entity::Arc {
                        center,
                        start: ph,
                        end: pl,
                    },
                );
                self.after_shorten(id);
            }
        }
        Ok(rep)
    }

    fn end_choice(&self, id: &str, e: &Carrier, at: P) -> End {
        let (s, f) = (e.at(0.0), e.at(e.extent()));
        let _ = id;
        if dist(at, s) <= dist(at, f) {
            End::Start
        } else {
            End::Finish
        }
    }

    /// Where an extension of the end of `id` nearest `at` would stop: the first shape it meets.
    fn plan_extend(
        &self,
        id: &str,
        at: P,
        construction: bool,
    ) -> Result<(Carrier, End, Cut), SketchError> {
        self.refuse_infinite(id, "extend")?;
        let e = self.shape_carrier(id)?;
        if e.is_circle() {
            return Err(err("a circle cannot be extended", vec![id.to_owned()]));
        }
        let end = self.end_choice(id, &e, at);
        let extent = e.extent();
        let tau = core::f64::consts::TAU;
        let mut best: Option<Cut> = None;
        for (oid, ent) in &self.entities {
            if oid == id || (!construction && self.is_construction(oid)) {
                continue;
            }
            let Some(o) = self.carrier(oid) else { continue };
            let ends: Vec<&String> = match ent {
                Entity::Line { a, b } => vec![a, b],
                Entity::Arc { start, end, .. } => vec![start, end],
                _ => Vec::new(),
            };
            for (s, p) in hits(&e, &o, false) {
                // How far past the end it lies (0 = at the end).
                let past = match (&e, end) {
                    (Carrier::Line { .. }, End::Finish) => s - 1.0,
                    (Carrier::Line { .. }, End::Start) => -s,
                    (_, End::Finish) => {
                        if s > extent + EPS && s < tau - EPS {
                            s - extent
                        } else {
                            continue;
                        }
                    }
                    (_, End::Start) => {
                        if s > extent + EPS && s < tau - EPS {
                            tau - s
                        } else {
                            continue;
                        }
                    }
                };
                if past <= EPS {
                    continue;
                }
                let reuse = ends
                    .iter()
                    .find(|q| self.point(q).is_some_and(|qp| dist(qp, p) < POS_TOL))
                    .map(|q| (*q).clone());
                let better = best.as_ref().is_none_or(|b| {
                    let bp = match (&e, end) {
                        (Carrier::Line { .. }, End::Finish) => b.s - 1.0,
                        (Carrier::Line { .. }, End::Start) => -b.s,
                        (_, End::Finish) => b.s - extent,
                        (_, End::Start) => tau - b.s,
                    };
                    past < bp
                });
                if better {
                    best = Some(Cut {
                        s,
                        p,
                        other: oid.clone(),
                        reuse,
                    });
                }
            }
        }
        let cut = best.ok_or_else(|| {
            err(
                "there is nothing in that direction to extend to",
                vec![id.to_owned()],
            )
        })?;
        Ok((e, end, cut))
    }

    /// The part an extension at `at` would add, for drawing a preview.
    pub fn extend_preview(&self, id: &str, at: P) -> Result<Vec<P>, SketchError> {
        self.extend_preview_with(id, at, false)
    }

    /// [`extend_preview`](Self::extend_preview); `construction` lets construction geometry stop it.
    pub fn extend_preview_with(
        &self,
        id: &str,
        at: P,
        construction: bool,
    ) -> Result<Vec<P>, SketchError> {
        let (e, end, cut) = self.plan_extend(id, at, construction)?;
        let extent = e.extent();
        let (from, to) = match (&e, end) {
            (Carrier::Line { .. }, End::Finish) => (1.0, cut.s),
            (Carrier::Line { .. }, End::Start) => (0.0, cut.s),
            (_, End::Finish) => (extent, cut.s),
            (_, End::Start) => (0.0, cut.s - core::f64::consts::TAU),
        };
        let n = if matches!(e, Carrier::Line { .. }) {
            1
        } else {
            48
        };
        Ok((0..=n)
            .map(|i| e.at(from + (to - from) * f64::from(i) / f64::from(n)))
            .collect())
    }

    /// Lengthens a line or arc at the end nearest `at` until it meets the next shape.
    pub fn extend(&mut self, id: &str, at: P) -> Result<EditReport, SketchError> {
        self.extend_with(id, at, false)
    }

    /// [`extend`](Self::extend); `construction` lets construction geometry stop it.
    pub fn extend_with(
        &mut self,
        id: &str,
        at: P,
        construction: bool,
    ) -> Result<EditReport, SketchError> {
        let (_, end, cut) = self.plan_extend(id, at, construction)?;
        let mut rep = EditReport::default();
        let p = self.cut_point(&cut, &mut rep);
        self.replace_end(id, end, &p);
        self.after_shorten(id);
        Ok(rep)
    }

    fn plan_split(&self, id: &str, at: P) -> Result<(Carrier, f64), SketchError> {
        self.refuse_infinite(id, "split")?;
        let e = self.shape_carrier(id)?;
        if e.is_circle() {
            // A circle needs two cuts to become arcs: the clicked place and its opposite.
            return Ok((e.clone(), e.param(at)));
        }
        let s = e.param(at);
        let extent = e.extent();
        let s = match &e {
            Carrier::Line { .. } => s,
            Carrier::Round { sweep, .. } => {
                if s <= *sweep {
                    s
                } else if s - sweep < core::f64::consts::TAU - s {
                    *sweep
                } else {
                    0.0
                }
            }
        };
        if s <= EPS || s >= extent - EPS {
            return Err(err(
                "that is at an end of the shape: split it somewhere along its length",
                vec![id.to_owned()],
            ));
        }
        Ok((e, s))
    }

    /// The place a split at `at` would happen (the nearest point on the shape).
    pub fn split_preview(&self, id: &str, at: P) -> Result<P, SketchError> {
        let (e, s) = self.plan_split(id, at)?;
        Ok(e.at(s))
    }

    /// Every place a split at `at` would cut: one for a line or arc, two (opposite each other)
    /// for a circle.
    pub fn split_markers(&self, id: &str, at: P) -> Result<Vec<P>, SketchError> {
        let (e, s) = self.plan_split(id, at)?;
        let mut out = vec![e.at(s)];
        if e.is_circle() {
            out.push(e.at(s + core::f64::consts::PI));
        }
        Ok(out)
    }

    /// Divides a line or arc in two at the point nearest `at`.
    pub fn split(&mut self, id: &str, at: P) -> Result<EditReport, SketchError> {
        let (e, s) = self.plan_split(id, at)?;
        let mut rep = EditReport::default();
        if e.is_circle() {
            // Two arcs from the clicked place to the opposite one and back, about the same centre.
            let Some(Entity::Circle { center, .. }) = self.entity(id).cloned() else {
                return Err(err("not a circle", vec![id.to_owned()]));
            };
            let p1 = self.new_point(e.at(s));
            let p2 = self.new_point(e.at(s + core::f64::consts::PI));
            rep.created.extend([p1.clone(), p2.clone()]);
            let second = self.fresh_id("a");
            self.entities.insert(
                id.to_owned(),
                Entity::Arc {
                    center: center.clone(),
                    start: p1.clone(),
                    end: p2.clone(),
                },
            );
            self.entities.insert(
                second.clone(),
                Entity::Arc {
                    center,
                    start: p2,
                    end: p1,
                },
            );
            if self.construction.contains(id) {
                self.construction.insert(second.clone());
            }
            rep.created.push(second.clone());
            self.tie_halves(id, &second);
            return Ok(rep);
        }
        let pos = e.at(s);
        let p = self.new_point(pos);
        rep.created.push(p.clone());
        let second = self.split_off(id, &p, &mut rep);
        self.replace_end(id, End::Finish, &p);
        self.after_shorten(id);
        self.tie_halves(id, &second);
        Ok(rep)
    }

    // ---- editing helpers --------------------------------------------------------------------

    pub(crate) fn fresh_id(&self, prefix: &str) -> String {
        (1..)
            .map(|n| format!("{prefix}{n}"))
            .find(|id| !self.entities.contains_key(id) && !self.constraints.contains_key(id))
            .expect("some id is free")
    }

    pub(crate) fn new_point(&mut self, pos: P) -> String {
        let id = self.fresh_id("p");
        self.entities
            .insert(id.clone(), Entity::Point { x: pos.0, y: pos.1 });
        id
    }

    /// The point at a cut: an existing end point if the shapes meet exactly there, else a new
    /// one related to the shape that cuts.
    fn cut_point(&mut self, cut: &Cut, rep: &mut EditReport) -> String {
        if let Some(q) = &cut.reuse {
            return q.clone();
        }
        let p = self.new_point(cut.p);
        rep.created.push(p.clone());
        let rel = self.fresh_id("c");
        let c = match self.entities.get(&cut.other) {
            Some(Entity::Line { .. }) => Constraint::PointOnLine {
                point: p.clone(),
                line: cut.other.clone(),
            },
            _ => Constraint::PointOnCurve {
                point: p.clone(),
                curve: cut.other.clone(),
            },
        };
        self.constraints.insert(rel, c);
        p
    }

    /// Makes the shape end (or start) at `new` instead, tidying the point it left.
    pub(crate) fn replace_end(&mut self, id: &str, end: End, new: &str) {
        let old = match self.entities.get_mut(id) {
            Some(Entity::Line { a, b }) => {
                let slot = if end == End::Start { a } else { b };
                std::mem::replace(slot, new.to_owned())
            }
            Some(Entity::Arc { start, end: e, .. }) => {
                let slot = if end == End::Start { start } else { e };
                std::mem::replace(slot, new.to_owned())
            }
            _ => return,
        };
        self.discard_point_if_unused(&old);
    }

    /// Creates a copy of the shape that runs from `from` to the old finish, and returns its id.
    /// The caller then shortens the original.
    fn split_off(&mut self, id: &str, from: &str, rep: &mut EditReport) -> String {
        let (second_id, entity) = match self.entities.get(id).cloned() {
            Some(Entity::Line { b, .. }) => (
                self.fresh_id("l"),
                Entity::Line {
                    a: from.to_owned(),
                    b,
                },
            ),
            Some(Entity::Arc { center, end, .. }) => (
                self.fresh_id("a"),
                Entity::Arc {
                    center,
                    start: from.to_owned(),
                    end,
                },
            ),
            _ => return String::new(),
        };
        self.entities.insert(second_id.clone(), entity);
        if self.construction.contains(id) {
            self.construction.insert(second_id.clone());
        }
        // Direction relations carry over to the new half.
        let is_line = matches!(self.entities.get(id), Some(Entity::Line { .. }));
        let copies: Vec<Constraint> = self
            .constraints
            .values()
            .filter_map(|c| match c {
                Constraint::Horizontal { line } if line == id => Some(Constraint::Horizontal {
                    line: second_id.clone(),
                }),
                Constraint::Vertical { line } if line == id => Some(Constraint::Vertical {
                    line: second_id.clone(),
                }),
                // The new half lies on the same line, so what the whole line was parallel,
                // perpendicular or tangent to, the half is too.
                Constraint::Parallel { a, b } if is_line && a == id => Some(Constraint::Parallel {
                    a: second_id.clone(),
                    b: b.clone(),
                }),
                Constraint::Parallel { a, b } if is_line && b == id => Some(Constraint::Parallel {
                    a: a.clone(),
                    b: second_id.clone(),
                }),
                Constraint::Perpendicular { a, b } if is_line && a == id => {
                    Some(Constraint::Perpendicular {
                        a: second_id.clone(),
                        b: b.clone(),
                    })
                }
                Constraint::Perpendicular { a, b } if is_line && b == id => {
                    Some(Constraint::Perpendicular {
                        a: a.clone(),
                        b: second_id.clone(),
                    })
                }
                Constraint::Tangent { line, curve } if is_line && line == id => {
                    Some(Constraint::Tangent {
                        line: second_id.clone(),
                        curve: curve.clone(),
                    })
                }
                _ => None,
            })
            .collect();
        for c in copies {
            let cid = self.fresh_id("c");
            self.constraints.insert(cid, c);
        }
        rep.created.push(second_id.clone());
        second_id
    }

    /// Two arcs cut from one keep one radius.
    fn tie_halves(&mut self, first: &str, second: &str) {
        if matches!(self.entities.get(first), Some(Entity::Arc { .. }))
            && matches!(self.entities.get(second), Some(Entity::Arc { .. }))
        {
            let cid = self.fresh_id("c");
            self.constraints.insert(
                cid,
                Constraint::EqualRadius {
                    a: first.to_owned(),
                    b: second.to_owned(),
                },
            );
        }
    }

    /// A shape changed length: dimensions and relations that depended on it no longer mean what
    /// they said, so they go.
    pub(crate) fn after_shorten(&mut self, id: &str) {
        self.constraints.retain(|_, c| {
            !matches!(c,
                Constraint::Length { line, .. } if line == id)
                && !matches!(c, Constraint::EqualLength { a, b } if a == id || b == id)
                && !matches!(c, Constraint::Midpoint { line, .. } if line == id)
        });
    }

    /// Removes a point nothing draws any more, with everything that referred to it.
    pub(crate) fn discard_point_if_unused(&mut self, pt: &str) {
        let used = self.entities.values().any(|e| e.refs().contains(&pt));
        if used || !matches!(self.entities.get(pt), Some(Entity::Point { .. })) {
            return;
        }
        self.constraints.retain(|_, c| !c.refs().contains(&pt));
        self.entities.remove(pt);
        self.construction.remove(pt);
        self.infinite.remove(pt);
    }

    pub(crate) fn delete_shape(&mut self, id: &str, rep: &mut EditReport) {
        let pts: Vec<String> = self
            .entities
            .get(id)
            .map(|e| e.refs().into_iter().map(str::to_owned).collect())
            .unwrap_or_default();
        self.constraints.retain(|_, c| !c.refs().contains(&id));
        self.entities.remove(id);
        self.construction.remove(id);
        self.infinite.remove(id);
        rep.removed.push(id.to_owned());
        let mut seen = BTreeSet::new();
        for p in pts {
            if seen.insert(p.clone()) {
                self.discard_point_if_unused(&p);
            }
        }
    }
}

impl Sketch {
    /// Trim away inside / outside two bounding shapes: every other line or arc that crosses both
    /// bounds loses the part between them (`inside`) or the parts beyond them (`!inside`).
    /// Returns what was created; refuses if nothing crosses both.
    pub fn trim_between(
        &mut self,
        bounds: &[String],
        inside: bool,
        construction: bool,
    ) -> Result<EditReport, SketchError> {
        if bounds.len() != 2 || bounds[0] == bounds[1] {
            return Err(err("pick two different bounding shapes", bounds.to_vec()));
        }
        for b in bounds {
            self.shape_carrier(b)?;
        }
        let opts = TrimOptions {
            construction: true,
            bounds: bounds.to_vec(),
        };
        let mut rep = EditReport::default();
        let ids: Vec<String> = self
            .entities
            .keys()
            .filter(|id| !bounds.contains(id))
            .cloned()
            .collect();
        for id in ids {
            if self.is_construction(&id) && !construction {
                continue;
            }
            let Some(e) = self.carrier(&id) else { continue };
            if e.is_circle() {
                continue;
            }
            // Only shapes that both bounds actually cross.
            let mut cuts = self.cuts(&id, &e, &opts);
            if cuts.len() < 2 {
                continue;
            }
            cuts.sort_by(|a, b| a.s.total_cmp(&b.s));
            let (lo, hi) = (cuts[0].s, cuts[cuts.len() - 1].s);
            let extent = e.extent();
            let pick = |s: f64| e.at(s);
            if inside {
                let r = self.trim_with(&id, pick((lo + hi) / 2.0), &opts)?;
                rep.created.extend(r.created);
                rep.removed.extend(r.removed);
            } else {
                // Beyond the far cut first (it leaves the entity's id on the near part), then
                // beyond the near cut.
                let r = self.trim_with(&id, pick((hi + extent) / 2.0), &opts)?;
                rep.created.extend(r.created);
                let r = self.trim_with(&id, pick(lo / 2.0), &opts)?;
                rep.created.extend(r.created);
            }
        }
        if rep.created.is_empty() && rep.removed.is_empty() {
            return Err(err(
                "nothing crosses both bounding shapes, so there is nothing to trim between them",
                bounds.to_vec(),
            ));
        }
        Ok(rep)
    }

    /// Where two shapes' full curves meet nearest the two places clicked: the corner.
    fn corner_of(
        &self,
        a: &str,
        at_a: P,
        b: &str,
        at_b: P,
    ) -> Result<(Carrier, Carrier, P), SketchError> {
        if self.infinite.contains(a) && self.infinite.contains(b) {
            return Err(err(
                "two infinite lines have no ends to trim to a corner",
                vec![a.to_owned(), b.to_owned()],
            ));
        }
        let (ca, cb) = (self.shape_carrier(a)?, self.shape_carrier(b)?);
        if a == b {
            return Err(err("pick two different shapes", vec![a.to_owned()]));
        }
        if ca.is_circle() || cb.is_circle() {
            return Err(err(
                "a circle has no corner: trim it instead",
                vec![a.to_owned(), b.to_owned()],
            ));
        }
        let mid = ((at_a.0 + at_b.0) / 2.0, (at_a.1 + at_b.1) / 2.0);
        let corner = hits(&ca, &cb, true)
            .into_iter()
            .map(|(_, p)| p)
            .min_by(|p, q| dist(*p, mid).total_cmp(&dist(*q, mid)))
            .ok_or_else(|| {
                err(
                    "those two shapes never meet, so there is no corner to trim to",
                    vec![a.to_owned(), b.to_owned()],
                )
            })?;
        Ok((ca, cb, corner))
    }

    /// Which end of a shape a corner trim replaces: the end on the far side of the corner from
    /// where the user clicked (the clicked side is what is kept).
    fn corner_end(c: &Carrier, corner: P, at: P) -> End {
        let (s, t) = (c.param(corner), c.param(at));
        match c {
            Carrier::Line { .. } => {
                if t <= s {
                    End::Finish
                } else {
                    End::Start
                }
            }
            Carrier::Round { sweep, .. } => {
                let tau = core::f64::consts::TAU;
                if s <= *sweep {
                    if t <= s {
                        End::Finish
                    } else {
                        End::Start
                    }
                } else if s - sweep <= tau - s {
                    End::Finish
                } else {
                    End::Start
                }
            }
        }
    }

    /// What a corner trim would remove and add, as polylines, for a preview.
    pub fn corner_preview(
        &self,
        a: &str,
        at_a: P,
        b: &str,
        at_b: P,
    ) -> Result<(Polylines, Polylines), SketchError> {
        let (ca, cb, corner) = self.corner_of(a, at_a, b, at_b)?;
        let (mut removed, mut added) = (Vec::new(), Vec::new());
        for (id, c, at) in [(a, &ca, at_a), (b, &cb, at_b)] {
            // An infinite line has no end to move: only the other shape adjusts to it.
            if self.infinite.contains(id) {
                continue;
            }
            let end = Self::corner_end(c, corner, at);
            let s = c.param(corner);
            let extent = c.extent();
            let tau = core::f64::consts::TAU;
            // The span between the shape's end and the corner, on the replaced side.
            let (from, to) = match (c, end) {
                (Carrier::Line { .. }, End::Finish) => (1.0, s),
                (Carrier::Line { .. }, End::Start) => (0.0, s),
                (_, End::Finish) => (extent, s),
                (_, End::Start) => (0.0, if s > extent { s - tau } else { s }),
            };
            let n = if matches!(c, Carrier::Line { .. }) {
                1
            } else {
                48
            };
            let line: Vec<P> = (0..=n)
                .map(|i| c.at(from + (to - from) * f64::from(i) / f64::from(n)))
                .collect();
            // Past the end is an extension; short of it is a trim.
            let beyond = match (c, end) {
                (Carrier::Line { .. }, End::Finish) => s > 1.0,
                (Carrier::Line { .. }, End::Start) => s < 0.0,
                (_, End::Finish) => s > extent,
                (_, End::Start) => s > extent,
            };
            if beyond {
                added.push(line);
            } else {
                removed.push(line);
            }
        }
        Ok((removed, added))
    }

    /// Trims or extends two lines / arcs to the corner where they meet, keeping the sides that
    /// were clicked (SolidWorks' "corner trim").
    pub fn corner_trim(
        &mut self,
        a: &str,
        at_a: P,
        b: &str,
        at_b: P,
    ) -> Result<EditReport, SketchError> {
        let (ca, cb, corner) = self.corner_of(a, at_a, b, at_b)?;
        let (ea, eb) = (
            Self::corner_end(&ca, corner, at_a),
            Self::corner_end(&cb, corner, at_b),
        );
        let mut rep = EditReport::default();
        let p = self.new_point(corner);
        rep.created.push(p.clone());
        for (id, end) in [(a, ea), (b, eb)] {
            if self.infinite.contains(id) {
                // The corner lies on the infinite line: say so, since it shares no end with it.
                let rel = self.fresh_id("c");
                self.constraints.insert(
                    rel,
                    Constraint::PointOnLine {
                        point: p.clone(),
                        line: id.to_owned(),
                    },
                );
            } else {
                self.replace_end(id, end, &p);
                self.after_shorten(id);
            }
        }
        Ok(rep)
    }

    /// Merges two points into one: everything that used either now uses the survivor (the one
    /// that is fixed, if only one is; otherwise `a`). Refuses if that would collapse a shape
    /// (both ends of a line, say).
    pub fn merge_points(&mut self, a: &str, b: &str) -> Result<EditReport, SketchError> {
        for id in [a, b] {
            if !matches!(self.entities.get(id), Some(Entity::Point { .. })) {
                return Err(err(format!("'{id}' is not a point"), vec![id.to_owned()]));
            }
        }
        if a == b {
            return Err(err("pick two different points", vec![a.to_owned()]));
        }
        let fixed = |p: &str| {
            self.constraints
                .values()
                .any(|c| matches!(c, Constraint::Fixed { point, .. } if point == p))
        };
        let (keep, gone) = if fixed(b) && !fixed(a) {
            (b, a)
        } else {
            (a, b)
        };
        for (id, e) in &self.entities {
            let refs = e.refs();
            if refs.contains(&keep) && refs.contains(&gone) {
                return Err(err(
                    format!("merging would collapse '{id}', which uses both points"),
                    vec![id.clone(), keep.to_owned(), gone.to_owned()],
                ));
            }
        }
        let (keep, gone) = (keep.to_owned(), gone.to_owned());
        for e in self.entities.values_mut() {
            match e {
                Entity::Line { a, b } => {
                    for p in [a, b] {
                        if *p == gone {
                            p.clone_from(&keep);
                        }
                    }
                }
                Entity::Circle { center, .. } => {
                    if *center == gone {
                        center.clone_from(&keep);
                    }
                }
                Entity::Arc { center, start, end } => {
                    for p in [center, start, end] {
                        if *p == gone {
                            p.clone_from(&keep);
                        }
                    }
                }
                Entity::Point { .. } => {}
            }
        }
        self.entities.remove(&gone);
        self.construction.remove(&gone);
        // Relations follow, except those that no longer make sense (a coincident between the
        // two, a distance from a point to itself, a point on a line it is now the end of).
        let old: Vec<(String, Constraint)> =
            std::mem::take(&mut self.constraints).into_iter().collect();
        let mut rep = EditReport::default();
        for (id, c) in old {
            let c = c.renamed(&gone, &keep);
            let degenerate = matches!(&c, Constraint::Coincident { a, b } if a == b);
            if degenerate || self.add_constraint(&id, c).is_err() {
                rep.dropped.push(id);
            }
        }
        rep.removed.push(gone);
        Ok(rep)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::SolveStatus;
    use Constraint::*;

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

    /// A segment drawn from `(x0, y0)` to `(x1, y1)` with its own end points.
    fn seg(s: &mut Sketch, id: &str, (x0, y0): P, (x1, y1): P) {
        pt(s, &format!("{id}a"), x0, y0);
        pt(s, &format!("{id}b"), x1, y1);
        line(s, id, &format!("{id}a"), &format!("{id}b"));
    }

    fn solved(s: &mut Sketch) {
        let r = s.solve();
        assert_eq!(r.status, SolveStatus::Solved, "{r:?}");
        assert!(r.applied);
    }

    fn near(a: P, b: P) -> bool {
        dist(a, b) < 1e-6
    }

    fn ends(s: &Sketch, id: &str) -> (P, P) {
        match s.entity(id) {
            Some(Entity::Line { a, b }) => (s.point(a).unwrap(), s.point(b).unwrap()),
            other => panic!("{id}: {other:?}"),
        }
    }

    /// Every constraint and entity still refers to things that exist.
    fn consistent(s: &Sketch) {
        for (id, e) in s.entities() {
            for r in e.refs() {
                assert!(s.entity(r).is_some(), "{id} refers to missing {r}");
            }
        }
        for (id, c) in s.constraints() {
            for r in c.refs() {
                assert!(
                    s.entity(r).is_some(),
                    "constraint {id} refers to missing {r}"
                );
            }
        }
    }

    /// A 40 mm horizontal line crossed by verticals at x = 10 and x = 30.
    fn ladder() -> Sketch {
        let mut s = Sketch::new();
        seg(&mut s, "h", (0.0, 0.0), (40.0, 0.0));
        seg(&mut s, "v1", (10.0, -5.0), (10.0, 5.0));
        seg(&mut s, "v2", (30.0, -5.0), (30.0, 5.0));
        s.add_constraint("hz", Horizontal { line: "h".into() })
            .unwrap();
        s.add_constraint(
            "len",
            Length {
                line: "h".into(),
                value: 40.0,
            },
        )
        .unwrap();
        s
    }

    #[test]
    fn trimming_the_end_of_a_line_back_to_the_next_crossing() {
        let mut s = ladder();
        let rep = s.trim("h", (36.0, 0.0)).unwrap();
        let (a, b) = ends(&s, "h");
        assert!(near(a, (0.0, 0.0)) && near(b, (30.0, 0.0)), "{a:?} {b:?}");
        assert_eq!(rep.created.len(), 1, "one new point at the cut");
        // The new end lies on the cutting line, by a relation; the dimension went, the rest stayed.
        assert!(s
            .constraints()
            .values()
            .any(|c| matches!(c, PointOnLine { line, .. } if line == "v2")));
        assert!(!s.constraints().contains_key("len"));
        assert!(s.constraints().contains_key("hz"));
        // The old end point is gone because nothing used it.
        assert!(s.entity("hb").is_none());
        consistent(&s);
        solved(&mut s);
        // The trimmed end is still exactly on the vertical.
        assert!((ends(&s, "h").1 .0 - 30.0).abs() < 1e-6);
    }

    #[test]
    fn trimming_the_start_and_the_middle() {
        let mut s = ladder();
        s.trim("h", (4.0, 0.0)).unwrap();
        let (a, b) = ends(&s, "h");
        assert!(near(a, (10.0, 0.0)) && near(b, (40.0, 0.0)), "{a:?} {b:?}");
        consistent(&s);
        solved(&mut s);

        // The part between the two verticals goes: two pieces, both still horizontal.
        let mut s = ladder();
        let rep = s.trim("h", (20.0, 0.0)).unwrap();
        let second = rep
            .created
            .iter()
            .find(|c| matches!(s.entity(c), Some(Entity::Line { .. })))
            .expect("a second line")
            .clone();
        let (a, b) = ends(&s, "h");
        assert!(near(a, (0.0, 0.0)) && near(b, (10.0, 0.0)), "{a:?} {b:?}");
        let (c, d) = ends(&s, &second);
        assert!(near(c, (30.0, 0.0)) && near(d, (40.0, 0.0)), "{c:?} {d:?}");
        let horizontals = s
            .constraints()
            .values()
            .filter(|c| matches!(c, Horizontal { .. }))
            .count();
        assert_eq!(horizontals, 2, "the direction carried over to the new half");
        consistent(&s);
        solved(&mut s);
        let (c, d) = ends(&s, &second);
        assert!((c.1 - d.1).abs() < 1e-9, "level");
    }

    #[test]
    fn a_cut_exactly_at_the_end_of_another_shape_reuses_that_point() {
        let mut s = Sketch::new();
        seg(&mut s, "h", (0.0, 0.0), (40.0, 0.0));
        // A line that ends exactly on h, at x = 25.
        pt(&mut s, "t", 25.0, 0.0);
        pt(&mut s, "u", 25.0, 15.0);
        line(&mut s, "v", "t", "u");
        let before = s.entities().len();
        s.trim("h", (35.0, 0.0)).unwrap();
        let (_, b) = ends(&s, "h");
        assert!(near(b, (25.0, 0.0)));
        match s.entity("h") {
            Some(Entity::Line { b, .. }) => assert_eq!(b, "t", "shares the existing point"),
            _ => panic!(),
        }
        // No new point, and no relation needed: the old end point just went.
        assert_eq!(s.entities().len(), before - 1);
        assert!(s.constraints().is_empty());
        consistent(&s);
    }

    #[test]
    fn a_shape_nothing_crosses_is_deleted_with_its_loose_points() {
        let mut s = Sketch::new();
        seg(&mut s, "h", (0.0, 0.0), (40.0, 0.0));
        s.add_constraint("hz", Horizontal { line: "h".into() })
            .unwrap();
        let rep = s.trim("h", (20.0, 0.0)).unwrap();
        assert_eq!(rep.removed, ["h"]);
        assert!(s.entities().is_empty() && s.constraints().is_empty());
        // A shared end point stays when something else still uses it.
        let mut s = Sketch::new();
        pt(&mut s, "p", 0.0, 0.0);
        pt(&mut s, "q", 10.0, 0.0);
        pt(&mut s, "r", 10.0, 10.0);
        line(&mut s, "l1", "p", "q");
        line(&mut s, "l2", "q", "r");
        s.trim("l1", (5.0, 0.0)).unwrap();
        assert!(s.entity("l1").is_none() && s.entity("p").is_none());
        assert!(s.entity("q").is_some() && s.entity("l2").is_some());
        consistent(&s);
    }

    #[test]
    fn a_circle_becomes_an_arc_and_arcs_trim_like_lines() {
        let mut s = Sketch::new();
        pt(&mut s, "c", 0.0, 0.0);
        s.add_entity(
            "k",
            Entity::Circle {
                center: "c".into(),
                radius: 10.0,
            },
        )
        .unwrap();
        s.add_constraint(
            "r",
            Radius {
                target: "k".into(),
                value: 10.0,
            },
        )
        .unwrap();
        seg(&mut s, "v", (0.0, -20.0), (0.0, 20.0));
        seg(&mut s, "h", (-20.0, 0.0), (20.0, 0.0));
        // Trim the quarter between the +x axis and the +y axis (click at 45 degrees).
        let q = 10.0 * core::f64::consts::FRAC_1_SQRT_2;
        let pre = s.trim_preview("k", (q, q)).unwrap();
        assert!(near(pre[0], (10.0, 0.0)) && near(*pre.last().unwrap(), (0.0, 10.0)));
        s.trim("k", (q, q)).unwrap();
        let Some(Entity::Arc { center, start, end }) = s.entity("k").cloned() else {
            panic!("a circle trimmed becomes an arc");
        };
        assert_eq!(center, "c");
        // What is left runs counter-clockwise from the top (90 degrees) round to the right (0).
        assert!(near(s.point(&start).unwrap(), (0.0, 10.0)));
        assert!(near(s.point(&end).unwrap(), (10.0, 0.0)));
        assert!(
            s.constraints().contains_key("r"),
            "its radius dimension survives"
        );
        consistent(&s);
        solved(&mut s);

        // Now the arc itself: trim its first piece back to the next cut (the -y axis crossing
        // is at 270 degrees, the -x axis crossing at 180).
        s.trim("k", (-q, q)).unwrap();
        let Some(Entity::Arc { start, end, .. }) = s.entity("k").cloned() else {
            panic!()
        };
        assert!(
            near(s.point(&start).unwrap(), (-10.0, 0.0)),
            "start moved to the -x crossing"
        );
        assert!(near(s.point(&end).unwrap(), (10.0, 0.0)));
        consistent(&s);
        solved(&mut s);

        // Middle of an arc: add a second crossing (a diagonal through 315 degrees) so the piece
        // between the +x... crossing at 270 and this one can go. Two arcs remain, one radius.
        seg(&mut s, "d", (3.0, -3.0), (12.0, -12.0));
        let rep = s.trim("k", (3.8, -9.2)).unwrap();
        let arcs = s
            .entities()
            .values()
            .filter(|e| matches!(e, Entity::Arc { .. }))
            .count();
        assert_eq!(arcs, 2, "{rep:?}");
        assert!(s
            .constraints()
            .values()
            .any(|c| matches!(c, EqualRadius { .. })));
        consistent(&s);
        solved(&mut s);
    }

    #[test]
    fn circles_with_too_few_crossings_are_deleted_or_refused() {
        let mut s = Sketch::new();
        pt(&mut s, "c", 0.0, 0.0);
        s.add_entity(
            "k",
            Entity::Circle {
                center: "c".into(),
                radius: 10.0,
            },
        )
        .unwrap();
        // Nothing crosses: gone.
        let mut gone = s.clone();
        gone.trim("k", (10.0, 0.0)).unwrap();
        assert!(gone.entities().is_empty());
        // One crossing only: refused with a reason.
        seg(&mut s, "v", (10.0, -20.0), (10.0, 20.0));
        let e = s.trim("k", (0.0, 10.0));
        // A vertical at x = 10 touches the circle at one point (tangent).
        assert!(e.is_err(), "{e:?}");
        assert!(e.unwrap_err().message.contains("two other shapes"));
    }

    #[test]
    fn extending_to_the_next_shape() {
        let mut s = Sketch::new();
        seg(&mut s, "h", (0.0, 0.0), (10.0, 0.0));
        seg(&mut s, "w1", (25.0, -5.0), (25.0, 5.0));
        seg(&mut s, "w2", (40.0, -5.0), (40.0, 5.0));
        s.add_constraint("hz", Horizontal { line: "h".into() })
            .unwrap();
        s.add_constraint(
            "len",
            Length {
                line: "h".into(),
                value: 10.0,
            },
        )
        .unwrap();
        let pre = s.extend_preview("h", (9.0, 0.0)).unwrap();
        assert!(
            near(pre[0], (10.0, 0.0)) && near(pre[1], (25.0, 0.0)),
            "{pre:?}"
        );
        s.extend("h", (9.0, 0.0)).unwrap();
        let (a, b) = ends(&s, "h");
        assert!(near(a, (0.0, 0.0)) && near(b, (25.0, 0.0)), "{a:?} {b:?}");
        assert!(!s.constraints().contains_key("len") && s.constraints().contains_key("hz"));
        assert!(s
            .constraints()
            .values()
            .any(|c| matches!(c, PointOnLine { line, .. } if line == "w1")));
        consistent(&s);
        solved(&mut s);
        // The start end looks the other way and finds nothing.
        let e = s.extend("h", (1.0, 0.0)).unwrap_err();
        assert!(e.message.contains("nothing in that direction"), "{e:?}");
        // Extend the same line again: it passes the first wall (now an end) and reaches the second.
        s.extend("h", (24.0, 0.0)).unwrap();
        assert!(near(ends(&s, "h").1, (40.0, 0.0)));
        consistent(&s);
    }

    #[test]
    fn extending_a_line_to_a_circle_and_an_arc_to_a_line() {
        let mut s = Sketch::new();
        seg(&mut s, "h", (0.0, 0.0), (5.0, 0.0));
        pt(&mut s, "c", 20.0, 0.0);
        s.add_entity(
            "k",
            Entity::Circle {
                center: "c".into(),
                radius: 6.0,
            },
        )
        .unwrap();
        s.extend("h", (5.0, 0.0)).unwrap();
        assert!(
            near(ends(&s, "h").1, (14.0, 0.0)),
            "the near side of the circle"
        );
        assert!(s
            .constraints()
            .values()
            .any(|c| matches!(c, PointOnCurve { curve, .. } if curve == "k")));
        consistent(&s);
        solved(&mut s);

        // An arc from (10,0) to (0,10) about the origin, extended past (0,10) to the -x axis.
        let mut t = Sketch::new();
        pt(&mut t, "c", 0.0, 0.0);
        pt(&mut t, "s", 10.0, 0.0);
        pt(&mut t, "e", 0.0, 10.0);
        t.add_entity(
            "a",
            Entity::Arc {
                center: "c".into(),
                start: "s".into(),
                end: "e".into(),
            },
        )
        .unwrap();
        seg(&mut t, "w", (-20.0, 0.0), (20.0, -0.0));
        t.extend("a", (0.0, 10.0)).unwrap();
        let Some(Entity::Arc { end, .. }) = t.entity("a").cloned() else {
            panic!()
        };
        assert!(
            near(t.point(&end).unwrap(), (-10.0, 0.0)),
            "{:?}",
            t.point(&end)
        );
        consistent(&t);
        solved(&mut t);
        // A circle cannot be extended.
        assert!(s.extend("k", (14.0, 0.0)).is_err());
    }

    #[test]
    fn splitting_a_line_and_an_arc() {
        let mut s = Sketch::new();
        seg(&mut s, "h", (0.0, 0.0), (20.0, 0.0));
        s.add_constraint("hz", Horizontal { line: "h".into() })
            .unwrap();
        s.add_constraint(
            "len",
            Length {
                line: "h".into(),
                value: 20.0,
            },
        )
        .unwrap();
        assert!(
            near(s.split_preview("h", (7.0, 3.0)).unwrap(), (7.0, 0.0)),
            "projected onto the line"
        );
        let rep = s.split("h", (7.0, 3.0)).unwrap();
        let second = rep
            .created
            .iter()
            .find(|c| matches!(s.entity(c), Some(Entity::Line { .. })))
            .unwrap()
            .clone();
        let ((a, m), (m2, b)) = (ends(&s, "h"), ends(&s, &second));
        assert!(near(a, (0.0, 0.0)) && near(m, (7.0, 0.0)) && near(m2, m) && near(b, (20.0, 0.0)));
        // One shared point, direction on both, the length dimension gone.
        match (s.entity("h"), s.entity(&second)) {
            (Some(Entity::Line { b: p, .. }), Some(Entity::Line { a: q, .. })) => assert_eq!(p, q),
            _ => panic!(),
        }
        assert!(!s.constraints().contains_key("len"));
        consistent(&s);
        solved(&mut s);

        let mut t = Sketch::new();
        pt(&mut t, "c", 0.0, 0.0);
        pt(&mut t, "s", 10.0, 0.0);
        pt(&mut t, "e", -10.0, 0.0);
        t.add_entity(
            "a",
            Entity::Arc {
                center: "c".into(),
                start: "s".into(),
                end: "e".into(),
            },
        )
        .unwrap();
        t.split("a", (0.0, 12.0)).unwrap();
        let arcs: Vec<&Entity> = t
            .entities()
            .values()
            .filter(|e| matches!(e, Entity::Arc { .. }))
            .collect();
        assert_eq!(arcs.len(), 2);
        assert!(t
            .constraints()
            .values()
            .any(|c| matches!(c, EqualRadius { .. })));
        consistent(&t);
        solved(&mut t);

        // Refusals: at an end, and something that is not a shape.
        assert!(t.split("a", (10.0, 0.0)).is_err());
        let mut u = Sketch::new();
        pt(&mut u, "c", 0.0, 0.0);
        u.add_entity(
            "k",
            Entity::Circle {
                center: "c".into(),
                radius: 3.0,
            },
        )
        .unwrap();
        u.split("k", (3.0, 0.0)).unwrap();
        assert_eq!(
            u.entities()
                .values()
                .filter(|e| matches!(e, Entity::Arc { .. }))
                .count(),
            2,
            "a circle splits into two arcs"
        );
        assert!(u.split("c", (0.0, 0.0)).is_err(), "a point is not a shape");
    }

    #[test]
    fn finding_what_is_under_the_pointer() {
        let s = ladder();
        assert_eq!(s.entity_near((20.0, 0.4), 0.5).as_deref(), Some("h"));
        assert_eq!(s.entity_near((10.2, 3.0), 0.5).as_deref(), Some("v1"));
        assert_eq!(s.entity_near((20.0, 3.0), 0.5), None);
        // The corner between two shapes goes to the nearer one, deterministically.
        assert_eq!(s.entity_near((10.0, 0.0), 0.5).as_deref(), Some("h"));
    }

    #[test]
    fn edits_are_deterministic() {
        let (mut a, mut b) = (ladder(), ladder());
        a.trim("h", (20.0, 0.0)).unwrap();
        b.trim("h", (20.0, 0.0)).unwrap();
        assert_eq!(a, b);
        assert_eq!(
            serde_json::to_string(&a).unwrap(),
            serde_json::to_string(&b).unwrap()
        );
    }

    #[test]
    fn corner_trim_joins_two_lines_at_their_corner_keeping_the_clicked_sides() {
        // Two lines that cross like a plus sign; keep the left and the bottom.
        let mut s = Sketch::new();
        seg(&mut s, "h", (0.0, 0.0), (20.0, 0.0));
        seg(&mut s, "v", (10.0, -10.0), (10.0, 10.0));
        let (removed, added) = s
            .corner_preview("h", (3.0, 0.0), "v", (10.0, -6.0))
            .unwrap();
        assert_eq!((removed.len(), added.len()), (2, 0), "both trimmed");
        s.corner_trim("h", (3.0, 0.0), "v", (10.0, -6.0)).unwrap();
        let (a, b) = ends(&s, "h");
        assert!(near(a, (0.0, 0.0)) && near(b, (10.0, 0.0)), "{a:?} {b:?}");
        let (c, d) = ends(&s, "v");
        assert!(
            near(c, (10.0, -10.0)) && near(d, (10.0, 0.0)),
            "{c:?} {d:?}"
        );
        // They now share the corner point.
        match (s.entity("h"), s.entity("v")) {
            (Some(Entity::Line { b: p, .. }), Some(Entity::Line { b: q, .. })) => assert_eq!(p, q),
            _ => panic!(),
        }
        consistent(&s);
        solved(&mut s);

        // Two lines that stop short of each other are extended to meet.
        let mut t = Sketch::new();
        seg(&mut t, "h", (0.0, 0.0), (6.0, 0.0));
        seg(&mut t, "v", (10.0, 4.0), (10.0, 12.0));
        let (removed, added) = t.corner_preview("h", (2.0, 0.0), "v", (10.0, 8.0)).unwrap();
        assert_eq!((removed.len(), added.len()), (0, 2), "both extended");
        t.corner_trim("h", (2.0, 0.0), "v", (10.0, 8.0)).unwrap();
        assert!(near(ends(&t, "h").1, (10.0, 0.0)) && near(ends(&t, "v").0, (10.0, 0.0)));
        consistent(&t);
        solved(&mut t);

        // Refusals: parallel lines never meet; a shape with itself; a circle.
        let mut u = Sketch::new();
        seg(&mut u, "a", (0.0, 0.0), (10.0, 0.0));
        seg(&mut u, "b", (0.0, 5.0), (10.0, 5.0));
        assert!(u
            .corner_trim("a", (1.0, 0.0), "b", (1.0, 5.0))
            .unwrap_err()
            .message
            .contains("never meet"));
        assert!(u.corner_trim("a", (1.0, 0.0), "a", (2.0, 0.0)).is_err());
        pt(&mut u, "c", 0.0, 20.0);
        u.add_entity(
            "k",
            Entity::Circle {
                center: "c".into(),
                radius: 3.0,
            },
        )
        .unwrap();
        assert!(u.corner_trim("a", (1.0, 0.0), "k", (3.0, 20.0)).is_err());
    }

    #[test]
    fn trim_between_chosen_bounds_ignores_everything_else() {
        // A line with three crossings; trim only against the two outer ones.
        let mut s = Sketch::new();
        seg(&mut s, "h", (0.0, 0.0), (40.0, 0.0));
        for (id, x) in [("w1", 8.0), ("w2", 20.0), ("w3", 32.0)] {
            seg(&mut s, id, (x, -4.0), (x, 4.0));
        }
        let opts = TrimOptions {
            bounds: vec!["w1".into(), "w3".into()],
            ..TrimOptions::default()
        };
        // Clicking at 20 normally removes 8..20 (the middle bar w2 is the other cut)...
        let normal = s.trim_preview("h", (14.0, 0.0)).unwrap();
        assert!(near(normal[0], (8.0, 0.0)) && near(normal[1], (20.0, 0.0)));
        // ...between the two bounds it removes everything from 8 to 32.
        let pre = s.trim_preview_with("h", (14.0, 0.0), &opts).unwrap();
        assert!(
            near(pre[0], (8.0, 0.0)) && near(pre[1], (32.0, 0.0)),
            "{pre:?}"
        );
        s.trim_with("h", (14.0, 0.0), &opts).unwrap();
        let (a, b) = ends(&s, "h");
        assert!(
            near(a, (0.0, 0.0)) && near(b, (8.0, 0.0)),
            "the first piece"
        );
        // The second piece (32..40) is a new line.
        assert!(s.entities().iter().any(|(id, e)| id != "h"
            && matches!(e, Entity::Line { .. })
            && {
                let (p, q) = ends(&s, id);
                near(p, (32.0, 0.0)) && near(q, (40.0, 0.0))
            }));
        consistent(&s);
        solved(&mut s);
    }

    #[test]
    fn construction_geometry_cuts_only_when_asked() {
        let mut s = Sketch::new();
        seg(&mut s, "h", (0.0, 0.0), (40.0, 0.0));
        seg(&mut s, "guide", (30.0, -5.0), (30.0, 5.0));
        s.set_construction("guide", true).unwrap();
        // Ignored: nothing cuts h, so trimming deletes it whole.
        let mut a = s.clone();
        a.trim("h", (35.0, 0.0)).unwrap();
        assert!(a.entity("h").is_none());
        // Asked to count it: the line is cut at the guide.
        let opts = TrimOptions {
            construction: true,
            ..TrimOptions::default()
        };
        let mut b = s.clone();
        b.trim_with("h", (35.0, 0.0), &opts).unwrap();
        assert!(near(ends(&b, "h").1, (30.0, 0.0)));
        // The same choice for extend.
        let mut t = Sketch::new();
        seg(&mut t, "h", (0.0, 0.0), (10.0, 0.0));
        seg(&mut t, "guide", (25.0, -5.0), (25.0, 5.0));
        t.set_construction("guide", true).unwrap();
        assert!(t.extend("h", (9.0, 0.0)).is_err());
        t.extend_with("h", (9.0, 0.0), true).unwrap();
        assert!(near(ends(&t, "h").1, (25.0, 0.0)));
    }

    #[test]
    fn an_infinite_line_cuts_and_extends_along_its_whole_length() {
        let opts = TrimOptions {
            construction: true,
            ..TrimOptions::default()
        };
        // A short guide at x = 25, y from -5 to 5, cutting a line that reaches x = 40.
        let mut s = Sketch::new();
        seg(&mut s, "h", (0.0, 20.0), (40.0, 20.0));
        seg(&mut s, "guide", (25.0, -5.0), (25.0, 5.0));
        s.set_construction("guide", true).unwrap();
        // Finite, the guide does not reach y = 20 at all: nothing cuts h.
        let mut a = s.clone();
        assert!(a.trim_with("h", (35.0, 20.0), &opts).is_ok());
        assert!(a.entity("h").is_none(), "deleted whole: nothing cut it");
        // Infinite, it cuts h at (25, 20) even though its own points stop at y = 5.
        s.set_infinite("guide", true).unwrap();
        let mut b = s.clone();
        b.trim_with("h", (35.0, 20.0), &opts).unwrap();
        assert!(near(ends(&b, "h").1, (25.0, 20.0)), "{:?}", ends(&b, "h"));
        // And a short line is extended all the way to it.
        let mut t = Sketch::new();
        seg(&mut t, "short", (0.0, 20.0), (10.0, 20.0));
        seg(&mut t, "guide", (25.0, -5.0), (25.0, 5.0));
        t.set_construction("guide", true).unwrap();
        assert!(t.extend_with("short", (9.0, 20.0), true).is_err());
        t.set_infinite("guide", true).unwrap();
        t.extend_with("short", (9.0, 20.0), true).unwrap();
        assert!(near(ends(&t, "short").1, (25.0, 20.0)));
    }

    #[test]
    fn corner_trim_to_an_infinite_line_moves_only_the_finite_one_and_lies_on_it() {
        let mut s = Sketch::new();
        seg(&mut s, "h", (0.0, 20.0), (40.0, 20.0));
        seg(&mut s, "guide", (25.0, -5.0), (25.0, 5.0));
        s.set_construction("guide", true).unwrap();
        s.set_infinite("guide", true).unwrap();
        let before = ends(&s, "guide");
        s.corner_trim("h", (10.0, 20.0), "guide", (25.0, 0.0))
            .unwrap();
        assert!(near(ends(&s, "h").1, (25.0, 20.0)), "{:?}", ends(&s, "h"));
        assert_eq!(
            ends(&s, "guide"),
            before,
            "the infinite line has no end to move"
        );
        assert!(
            s.constraints()
                .values()
                .any(|c| matches!(c, Constraint::PointOnLine { line, .. } if line == "guide")),
            "the new corner is on the infinite line"
        );
        // Two infinite lines have no ends to trim.
        let mut t = Sketch::new();
        seg(&mut t, "a", (0.0, 0.0), (1.0, 0.0));
        seg(&mut t, "b", (5.0, -1.0), (5.0, 1.0));
        for id in ["a", "b"] {
            t.set_construction(id, true).unwrap();
            t.set_infinite(id, true).unwrap();
        }
        assert!(t.corner_trim("a", (0.0, 0.0), "b", (5.0, 0.0)).is_err());
    }

    #[test]
    fn an_infinite_line_cannot_itself_be_trimmed_extended_or_split() {
        let mut s = Sketch::new();
        seg(&mut s, "g", (0.0, 0.0), (10.0, 0.0));
        s.set_construction("g", true).unwrap();
        s.set_infinite("g", true).unwrap();
        for e in [
            s.clone().trim("g", (5.0, 0.0)).unwrap_err(),
            s.clone().extend("g", (5.0, 0.0)).unwrap_err(),
            s.clone().split("g", (5.0, 0.0)).unwrap_err(),
        ] {
            assert!(e.message.contains("infinite"), "{}", e.message);
        }
    }

    #[test]
    fn splitting_a_circle_makes_two_arcs_and_a_split_keeps_parallel_relations() {
        let mut s = Sketch::new();
        pt(&mut s, "c", 0.0, 0.0);
        s.add_entity(
            "k",
            Entity::Circle {
                center: "c".into(),
                radius: 10.0,
            },
        )
        .unwrap();
        s.add_constraint(
            "r",
            Radius {
                target: "k".into(),
                value: 10.0,
            },
        )
        .unwrap();
        let marks = s.split_markers("k", (10.0, 1.0)).unwrap();
        assert_eq!(marks.len(), 2);
        assert!(
            near(marks[1], (-marks[0].0, -marks[0].1)),
            "opposite each other"
        );
        s.split("k", (10.0, 1.0)).unwrap();
        let arcs: Vec<&Entity> = s
            .entities()
            .values()
            .filter(|e| matches!(e, Entity::Arc { .. }))
            .collect();
        assert_eq!(arcs.len(), 2);
        assert!(
            s.entity("k")
                .is_some_and(|e| matches!(e, Entity::Arc { .. })),
            "the id lives on"
        );
        assert!(s.constraints().contains_key("r"));
        consistent(&s);
        solved(&mut s);

        // Parallel / perpendicular / tangent carry over to the new half of a split line.
        let mut t = Sketch::new();
        seg(&mut t, "h", (0.0, 0.0), (20.0, 0.0));
        seg(&mut t, "p", (0.0, 5.0), (20.0, 5.0));
        seg(&mut t, "q", (5.0, 10.0), (5.0, 20.0));
        t.add_constraint(
            "par",
            Parallel {
                a: "h".into(),
                b: "p".into(),
            },
        )
        .unwrap();
        t.add_constraint(
            "perp",
            Perpendicular {
                a: "q".into(),
                b: "h".into(),
            },
        )
        .unwrap();
        let rep = t.split("h", (8.0, 0.0)).unwrap();
        let second = rep
            .created
            .iter()
            .find(|c| matches!(t.entity(c), Some(Entity::Line { .. })))
            .unwrap()
            .clone();
        assert!(t
            .constraints()
            .values()
            .any(|c| matches!(c, Parallel { a, b } if a == &second && b == "p")));
        assert!(t
            .constraints()
            .values()
            .any(|c| matches!(c, Perpendicular { a, b } if a == "q" && b == &second)));
        consistent(&t);
        solved(&mut t);
    }

    #[test]
    fn extending_stops_at_a_shape_that_only_touches_tangentially() {
        let mut s = Sketch::new();
        seg(&mut s, "h", (0.0, 0.0), (5.0, 0.0));
        pt(&mut s, "c", 20.0, 6.0);
        s.add_entity(
            "k",
            Entity::Circle {
                center: "c".into(),
                radius: 6.0,
            },
        )
        .unwrap();
        // The circle's bottom touches y = 0 at x = 20 and nowhere else.
        s.extend("h", (5.0, 0.0)).unwrap();
        assert!(near(ends(&s, "h").1, (20.0, 0.0)), "{:?}", ends(&s, "h"));
        consistent(&s);
    }

    #[test]
    fn merging_points_rewires_everything_and_refuses_to_collapse_a_shape() {
        let mut s = Sketch::new();
        // Two lines whose ends are separate points at the same place.
        seg(&mut s, "l1", (0.0, 0.0), (10.0, 0.0));
        seg(&mut s, "l2", (10.0, 0.0), (10.0, 10.0));
        s.add_constraint("hz", Horizontal { line: "l1".into() })
            .unwrap();
        s.add_constraint(
            "co",
            Coincident {
                a: "l1b".into(),
                b: "l2a".into(),
            },
        )
        .unwrap();
        s.add_constraint(
            "fx",
            Fixed {
                point: "l2a".into(),
                x: 10.0,
                y: 0.0,
            },
        )
        .unwrap();
        let rep = s.merge_points("l1b", "l2a").unwrap();
        // The fixed point is the one that survives, even though `l1b` was named first.
        assert_eq!(rep.removed, ["l1b"]);
        match (s.entity("l1"), s.entity("l2")) {
            (Some(Entity::Line { b, .. }), Some(Entity::Line { a, .. })) => {
                assert_eq!(b, "l2a");
                assert_eq!(a, "l2a");
            }
            _ => panic!(),
        }
        assert!(
            !s.constraints().contains_key("co"),
            "the coincident became meaningless"
        );
        assert!(s.constraints().contains_key("hz") && s.constraints().contains_key("fx"));
        consistent(&s);
        solved(&mut s);
        // Both ends of one line cannot be merged, nor a point with itself or a non-point.
        let e = s.merge_points("l1a", "l2a").unwrap_err();
        assert!(e.message.contains("collapse"), "{e:?}");
        assert!(s.merge_points("l1a", "l1a").is_err());
        assert!(s.merge_points("l1", "l1a").is_err());
    }

    #[test]
    fn trim_away_inside_and_outside_work_on_everything_the_two_bounds_cross() {
        let build = || {
            let mut s = Sketch::new();
            // Two vertical bounds, and three horizontals (one crossing only one bound).
            seg(&mut s, "w1", (10.0, -20.0), (10.0, 20.0));
            seg(&mut s, "w2", (30.0, -20.0), (30.0, 20.0));
            seg(&mut s, "a", (0.0, 0.0), (40.0, 0.0));
            seg(&mut s, "b", (0.0, 8.0), (40.0, 8.0));
            seg(&mut s, "c", (0.0, 14.0), (20.0, 14.0));
            s
        };
        let bounds = ["w1".to_owned(), "w2".to_owned()];
        // Inside: the part of a and b between x = 10 and 30 goes; c crosses only one bound.
        let mut s = build();
        let rep = s.trim_between(&bounds, true, false).unwrap();
        assert_eq!(
            rep.created
                .iter()
                .filter(|c| matches!(s.entity(c), Some(Entity::Line { .. })))
                .count(),
            2
        );
        assert!(near(ends(&s, "a").1, (10.0, 0.0)), "{:?}", ends(&s, "a"));
        assert!(near(ends(&s, "b").1, (10.0, 8.0)));
        assert!(near(ends(&s, "c").1, (20.0, 14.0)), "c is untouched");
        consistent(&s);
        solved(&mut s);
        // Outside: what is beyond the bounds goes; the middle stays.
        let mut t = build();
        t.trim_between(&bounds, false, false).unwrap();
        let (p, q) = ends(&t, "a");
        assert!(near(p, (10.0, 0.0)) && near(q, (30.0, 0.0)), "{p:?} {q:?}");
        let (p, q) = ends(&t, "b");
        assert!(near(p, (10.0, 8.0)) && near(q, (30.0, 8.0)));
        assert!(near(ends(&t, "c").0, (0.0, 14.0)));
        consistent(&t);
        solved(&mut t);
        // Nothing to do, or the wrong bounds: refused with a reason.
        let mut u = Sketch::new();
        seg(&mut u, "w1", (10.0, -20.0), (10.0, 20.0));
        seg(&mut u, "w2", (30.0, -20.0), (30.0, 20.0));
        seg(&mut u, "far", (0.0, 50.0), (5.0, 50.0));
        assert!(u
            .trim_between(&bounds, true, false)
            .unwrap_err()
            .message
            .contains("nothing crosses"));
        assert!(u.trim_between(&["w1".to_owned()], true, false).is_err());
        assert!(u
            .trim_between(&["w1".to_owned(), "w1".to_owned()], true, false)
            .is_err());
    }

    #[test]
    fn merging_says_which_relations_it_had_to_drop() {
        let mut s = Sketch::new();
        seg(&mut s, "l", (0.0, 0.0), (10.0, 0.0));
        pt(&mut s, "x", 10.0, 0.0);
        pt(&mut s, "y", 20.0, 0.0);
        s.add_constraint(
            "co",
            Coincident {
                a: "lb".into(),
                b: "x".into(),
            },
        )
        .unwrap();
        s.add_constraint(
            "dd",
            Distance {
                a: "lb".into(),
                b: "x".into(),
                value: 0.5,
            },
        )
        .unwrap();
        s.add_constraint(
            "keep",
            Distance {
                a: "x".into(),
                b: "y".into(),
                value: 10.0,
            },
        )
        .unwrap();
        let rep = s.merge_points("lb", "x").unwrap();
        assert_eq!(rep.dropped, ["co", "dd"], "{rep:?}");
        assert!(s.constraints().contains_key("keep"));
    }
}
