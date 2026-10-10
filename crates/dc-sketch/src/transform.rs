//! Modifying sketch geometry as a whole: move, copy, rotate, scale, mirror, patterns, offset,
//! sketch fillet and chamfer, repair, and the "fully define" assistant. Every operation is exact
//! and deterministic (new ids are the first free `p1`, `l1`, ...), changes entities and
//! relations only, and leaves solving to the caller.

use crate::edit::{EditReport, End};
use crate::model::{Constraint, Entity, Sketch, SketchError, SketchErrorCode};
use dc_math::det;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};

type P = (f64, f64);

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

/// A rigid or similarity change applied to a selection (about a point for turns and scaling).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Transform {
    Move { dx: f64, dy: f64 },
    Rotate { cx: f64, cy: f64, degrees: f64 },
    Scale { cx: f64, cy: f64, factor: f64 },
}

/// What to mirror about: a line of the sketch, or one of its axes.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum MirrorAbout {
    Line { entity: String },
    AxisX,
    AxisY,
}

/// Repeated copies of a selection.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Pattern {
    /// `count` instances in all (the original and `count - 1` copies), each `(dx, dy)` further.
    Linear { dx: f64, dy: f64, count: u32 },
    /// `count` instances spread over `degrees` about a centre (the whole turn if 360 or more).
    Circular {
        cx: f64,
        cy: f64,
        degrees: f64,
        count: u32,
    },
}

/// What a copy does to relations that mention a line's direction.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Axes {
    /// Horizontal stays horizontal.
    Keep,
    /// Horizontal becomes vertical and back (a quarter turn).
    Swap,
    /// The copy is turned some other way: direction relations do not carry over.
    Lose,
}

/// How points map in a copy.
struct Xf<'a> {
    map: &'a dyn Fn(P) -> P,
    /// Circle radii scale by this.
    k: f64,
    /// An orientation-reversing copy (a mirror) swaps an arc's start and end.
    flips: bool,
    axes: Axes,
    /// A point that does not move is the same point (on a mirror line): reuse it.
    reuse_fixed: bool,
}

impl Sketch {
    /// The shapes asked for plus every point they stand on, or an error naming what is missing.
    fn closure(&self, ids: &[String]) -> Result<BTreeSet<String>, SketchError> {
        if ids.is_empty() {
            return Err(err("select something first", vec![]));
        }
        let mut out = BTreeSet::new();
        for id in ids {
            let Some(e) = self.entity(id) else {
                return Err(SketchError {
                    code: SketchErrorCode::NotFound,
                    message: format!("entity '{id}' does not exist"),
                    entities: vec![id.clone()],
                });
            };
            out.insert(id.clone());
            out.extend(e.refs().into_iter().map(str::to_owned));
        }
        Ok(out)
    }

    /// Copies `set` through `xf`; returns old id -> new id.
    fn duplicate(
        &mut self,
        set: &BTreeSet<String>,
        xf: &Xf,
        rep: &mut EditReport,
    ) -> BTreeMap<String, String> {
        let mut map: BTreeMap<String, String> = BTreeMap::new();
        for id in set {
            if let Some(Entity::Point { x, y }) = self.entity(id).cloned() {
                let to = (xf.map)((x, y));
                if xf.reuse_fixed && dist(to, (x, y)) < 1e-9 {
                    map.insert(id.clone(), id.clone());
                } else {
                    let n = self.new_point(to);
                    rep.created.push(n.clone());
                    map.insert(id.clone(), n);
                }
            }
        }
        for id in set {
            let Some(e) = self.entity(id).cloned() else {
                continue;
            };
            let m = |p: &String| map.get(p).cloned().unwrap_or_else(|| p.clone());
            let (prefix, copy) = match &e {
                Entity::Point { .. } => continue,
                Entity::Line { a, b } => ("l", Entity::Line { a: m(a), b: m(b) }),
                Entity::Circle { center, radius } => (
                    "k",
                    Entity::Circle {
                        center: m(center),
                        radius: radius * xf.k,
                    },
                ),
                Entity::Arc { center, start, end } => {
                    let (s, en) = if xf.flips {
                        (m(end), m(start))
                    } else {
                        (m(start), m(end))
                    };
                    (
                        "a",
                        Entity::Arc {
                            center: m(center),
                            start: s,
                            end: en,
                        },
                    )
                }
            };
            let nid = self.fresh_id(prefix);
            self.entities.insert(nid.clone(), copy);
            if self.construction.contains(id) {
                self.construction.insert(nid.clone());
            }
            if self.infinite.contains(id) {
                self.infinite.insert(nid.clone());
            }
            rep.created.push(nid.clone());
            map.insert(id.clone(), nid);
        }
        // Relations among the copied things come along (dimensions and anchors do not).
        let carried: Vec<Constraint> = self
            .constraints
            .values()
            .filter(|c| c.value().is_none())
            .filter(|c| {
                !matches!(
                    c,
                    Constraint::Fixed { .. }
                        | Constraint::OnAxis { .. }
                        | Constraint::SymmetricAxis { .. }
                )
            })
            .filter(|c| c.refs().iter().all(|r| set.contains(*r)))
            .filter_map(|c| match (c, xf.axes) {
                (Constraint::Horizontal { .. } | Constraint::Vertical { .. }, Axes::Lose) => None,
                (Constraint::Horizontal { line }, Axes::Swap) => {
                    Some(Constraint::Vertical { line: line.clone() })
                }
                (Constraint::Vertical { line }, Axes::Swap) => {
                    Some(Constraint::Horizontal { line: line.clone() })
                }
                (c, _) => Some(c.clone()),
            })
            .collect();
        for c in carried {
            let mut c = c;
            for (from, to) in &map {
                c = c.renamed(from, to);
            }
            let cid = self.fresh_id("c");
            if self.add_constraint(&cid, c).is_err() {
                continue;
            }
        }
        map
    }

    /// Moves, turns or scales the selection in place, or does it to copies (`copy`).
    pub fn transform(
        &mut self,
        ids: &[String],
        t: &Transform,
        copy: bool,
    ) -> Result<EditReport, SketchError> {
        let set = self.closure(ids)?;
        let (map, k, axes): (Box<dyn Fn(P) -> P>, f64, Axes) = match t {
            Transform::Move { dx, dy } => {
                if !dx.is_finite() || !dy.is_finite() {
                    return Err(err("the distance must be finite numbers", vec![]));
                }
                let (dx, dy) = (*dx, *dy);
                (Box::new(move |p: P| (p.0 + dx, p.1 + dy)), 1.0, Axes::Keep)
            }
            Transform::Rotate { cx, cy, degrees } => {
                if ![cx, cy, degrees].iter().all(|v| v.is_finite()) {
                    return Err(err("the turn must be finite numbers", vec![]));
                }
                let (cx, cy) = (*cx, *cy);
                let a = degrees.to_radians();
                let (c, s) = (det::cos(a), det::sin(a));
                // A quarter turn swaps horizontal and vertical; a half turn keeps them.
                let quarter = (degrees / 90.0).round();
                let axes = if (degrees / 90.0 - quarter).abs() < 1e-9 {
                    if (quarter as i64).rem_euclid(2) == 0 {
                        Axes::Keep
                    } else {
                        Axes::Swap
                    }
                } else {
                    Axes::Lose
                };
                (
                    Box::new(move |p: P| {
                        let (x, y) = (p.0 - cx, p.1 - cy);
                        (cx + c * x - s * y, cy + s * x + c * y)
                    }),
                    1.0,
                    axes,
                )
            }
            Transform::Scale { cx, cy, factor } => {
                if !factor.is_finite() || *factor <= 0.0 || !cx.is_finite() || !cy.is_finite() {
                    return Err(err("the scale factor must be a positive number", vec![]));
                }
                let (cx, cy, f) = (*cx, *cy, *factor);
                (
                    Box::new(move |p: P| (cx + f * (p.0 - cx), cy + f * (p.1 - cy))),
                    f,
                    Axes::Keep,
                )
            }
        };
        let mut rep = EditReport::default();
        if copy {
            let xf = Xf {
                map: &*map,
                k,
                flips: false,
                axes,
                reuse_fixed: false,
            };
            self.duplicate(&set, &xf, &mut rep);
            return Ok(rep);
        }
        for id in &set {
            match self.entity(id).cloned() {
                Some(Entity::Point { x, y }) => {
                    let (nx, ny) = map((x, y));
                    self.set_point(id, nx, ny)?;
                }
                Some(Entity::Circle { radius, .. }) => {
                    self.set_circle_radius(id, radius * k)?;
                }
                _ => {}
            }
        }
        // A turn takes the direction of lines with it: a quarter turn swaps level and upright,
        // another angle leaves nothing for "horizontal" to mean, so it goes (and is reported).
        if axes != Axes::Keep {
            let affected: Vec<(String, Constraint)> = self
                .constraints
                .iter()
                .filter(|(_, c)| {
                    matches!(c, Constraint::Horizontal { line } | Constraint::Vertical { line } if set.contains(line))
                })
                .map(|(id, c)| (id.clone(), c.clone()))
                .collect();
            for (id, c) in affected {
                match (axes, c) {
                    (Axes::Swap, Constraint::Horizontal { line }) => {
                        self.constraints.insert(id, Constraint::Vertical { line });
                    }
                    (Axes::Swap, Constraint::Vertical { line }) => {
                        self.constraints.insert(id, Constraint::Horizontal { line });
                    }
                    _ => {
                        self.constraints.remove(&id);
                        rep.dropped.push(id);
                    }
                }
            }
        }
        Ok(rep)
    }

    /// Mirrored copies of the selection about a line or an axis, tied to the originals by symmetry.
    pub fn mirror(
        &mut self,
        ids: &[String],
        about: &MirrorAbout,
    ) -> Result<EditReport, SketchError> {
        let mut ids: Vec<String> = ids.to_vec();
        // Where is the mirror: a point on it and its direction.
        let (origin, dir, axes) = match about {
            MirrorAbout::AxisX => ((0.0, 0.0), (1.0, 0.0), Axes::Keep),
            MirrorAbout::AxisY => ((0.0, 0.0), (0.0, 1.0), Axes::Keep),
            MirrorAbout::Line { entity } => {
                let Some(Entity::Line { a, b }) = self.entity(entity).cloned() else {
                    return Err(err(
                        "mirror about a line of the sketch",
                        vec![entity.clone()],
                    ));
                };
                let (pa, pb) = (
                    self.point(&a)
                        .ok_or_else(|| err("missing end point", vec![a.clone()]))?,
                    self.point(&b)
                        .ok_or_else(|| err("missing end point", vec![b.clone()]))?,
                );
                let len = dist(pa, pb);
                if len < 1e-9 {
                    return Err(err("that line has no length", vec![entity.clone()]));
                }
                ids.retain(|i| i != entity);
                let d = ((pb.0 - pa.0) / len, (pb.1 - pa.1) / len);
                // Direction relations survive only if the mirror line is level or upright.
                let level = d.1.abs() < 1e-9 || d.0.abs() < 1e-9;
                (pa, d, if level { Axes::Keep } else { Axes::Lose })
            }
        };
        let set = self.closure(&ids)?;
        let reflect = move |p: P| {
            let (x, y) = (p.0 - origin.0, p.1 - origin.1);
            let along = x * dir.0 + y * dir.1;
            // The mirror image is twice the foot of the perpendicular, minus the point.
            (
                origin.0 + 2.0 * along * dir.0 - x,
                origin.1 + 2.0 * along * dir.1 - y,
            )
        };
        let mut rep = EditReport::default();
        let xf = Xf {
            map: &reflect,
            k: 1.0,
            flips: true,
            axes,
            reuse_fixed: true,
        };
        let map = self.duplicate(&set, &xf, &mut rep);
        // The copy follows the original: pairs of points are symmetric about the mirror.
        for (old, new) in &map {
            if old == new || !matches!(self.entity(old), Some(Entity::Point { .. })) {
                continue;
            }
            let cid = self.fresh_id("c");
            let c = match about {
                MirrorAbout::AxisX => Constraint::SymmetricAxis {
                    a: old.clone(),
                    b: new.clone(),
                    axis: crate::SketchAxis::X,
                },
                MirrorAbout::AxisY => Constraint::SymmetricAxis {
                    a: old.clone(),
                    b: new.clone(),
                    axis: crate::SketchAxis::Y,
                },
                MirrorAbout::Line { entity } => Constraint::Symmetric {
                    a: old.clone(),
                    b: new.clone(),
                    line: entity.clone(),
                },
            };
            let _ = self.add_constraint(&cid, c);
        }
        Ok(rep)
    }

    /// Linear or circular copies of the selection.
    pub fn pattern(&mut self, ids: &[String], p: &Pattern) -> Result<EditReport, SketchError> {
        let set = self.closure(ids)?;
        let count = match p {
            Pattern::Linear { count, .. } | Pattern::Circular { count, .. } => *count,
        };
        if !(2..=200).contains(&count) {
            return Err(err("a pattern needs between 2 and 200 instances", vec![]));
        }
        let mut rep = EditReport::default();
        for i in 1..count {
            let (map, axes): (Box<dyn Fn(P) -> P>, Axes) = match p {
                Pattern::Linear { dx, dy, .. } => {
                    if !dx.is_finite() || !dy.is_finite() || (dx.abs() < 1e-9 && dy.abs() < 1e-9) {
                        return Err(err("the spacing must be a real distance", vec![]));
                    }
                    let (ox, oy) = (dx * f64::from(i), dy * f64::from(i));
                    (Box::new(move |q: P| (q.0 + ox, q.1 + oy)), Axes::Keep)
                }
                Pattern::Circular {
                    cx, cy, degrees, ..
                } => {
                    if !degrees.is_finite()
                        || degrees.abs() < 1e-9
                        || !cx.is_finite()
                        || !cy.is_finite()
                    {
                        return Err(err("the angle must be a real turn", vec![]));
                    }
                    // A full turn is shared among all instances; a part turn runs first to last.
                    let step = if degrees.abs() >= 360.0 - 1e-9 {
                        degrees / f64::from(count)
                    } else {
                        degrees / f64::from(count - 1)
                    };
                    let a = (step * f64::from(i)).to_radians();
                    let (c, s) = (det::cos(a), det::sin(a));
                    let (cx, cy) = (*cx, *cy);
                    (
                        Box::new(move |q: P| {
                            let (x, y) = (q.0 - cx, q.1 - cy);
                            (cx + c * x - s * y, cy + s * x + c * y)
                        }),
                        Axes::Lose,
                    )
                }
            };
            let xf = Xf {
                map: &*map,
                k: 1.0,
                flips: false,
                axes,
                reuse_fixed: false,
            };
            self.duplicate(&set, &xf, &mut rep);
        }
        Ok(rep)
    }
}

impl Sketch {
    /// Parallel copies of lines (a single continuous chain, joined with sharp corners) at
    /// `distance`, or of circles and arcs (about the same centre) with the radius grown by it.
    /// A chain's positive side is the left of its walking direction (and the outside of a closed
    /// outline); for circles and arcs positive is outward.
    pub fn offset(&mut self, ids: &[String], distance: f64) -> Result<EditReport, SketchError> {
        if !distance.is_finite() || distance.abs() < 1e-9 {
            return Err(err(
                "the offset distance must be a real, non-zero distance",
                vec![],
            ));
        }
        if ids.is_empty() {
            return Err(err("select the shapes to offset first", vec![]));
        }
        let mut rep = EditReport::default();
        let kinds: Vec<(String, Entity)> = ids
            .iter()
            .map(|i| {
                self.entity(i)
                    .cloned()
                    .map(|e| (i.clone(), e))
                    .ok_or_else(|| err(format!("entity '{i}' does not exist"), vec![i.clone()]))
            })
            .collect::<Result<_, _>>()?;
        if kinds
            .iter()
            .all(|(_, e)| matches!(e, Entity::Circle { .. } | Entity::Arc { .. }))
        {
            for (id, e) in kinds {
                match e {
                    Entity::Circle { center, radius } => {
                        let r = radius + distance;
                        if r <= 1e-9 {
                            return Err(err("that offset would shrink the circle away", vec![id]));
                        }
                        let nid = self.fresh_id("k");
                        self.entities
                            .insert(nid.clone(), Entity::Circle { center, radius: r });
                        rep.created.push(nid);
                    }
                    Entity::Arc { center, start, end } => {
                        let c = self
                            .point(&center)
                            .ok_or_else(|| err("missing centre", vec![center.clone()]))?;
                        let r0 = dist(
                            c,
                            self.point(&start)
                                .ok_or_else(|| err("missing start", vec![start.clone()]))?,
                        );
                        let r = r0 + distance;
                        if r <= 1e-9 {
                            return Err(err("that offset would shrink the arc away", vec![id]));
                        }
                        let grown = |p: &str, me: &mut Self| -> Result<String, SketchError> {
                            let q = me
                                .point(p)
                                .ok_or_else(|| err("missing end point", vec![p.to_owned()]))?;
                            let k = r / r0.max(1e-12);
                            Ok(me.new_point((c.0 + (q.0 - c.0) * k, c.1 + (q.1 - c.1) * k)))
                        };
                        let (ns, ne) = (grown(&start, self)?, grown(&end, self)?);
                        rep.created.extend([ns.clone(), ne.clone()]);
                        let nid = self.fresh_id("a");
                        self.entities.insert(
                            nid.clone(),
                            Entity::Arc {
                                center,
                                start: ns,
                                end: ne,
                            },
                        );
                        rep.created.push(nid);
                    }
                    _ => {}
                }
            }
            return Ok(rep);
        }
        if !kinds.iter().all(|(_, e)| matches!(e, Entity::Line { .. })) {
            return Err(err(
                "offset lines (one continuous chain) or circles and arcs, not a mixture",
                ids.to_vec(),
            ));
        }
        // Walk the chain of lines through shared end points.
        let lines: Vec<(String, String, String)> = kinds
            .iter()
            .filter_map(|(id, e)| match e {
                Entity::Line { a, b } => Some((id.clone(), a.clone(), b.clone())),
                _ => None,
            })
            .collect();
        let mut degree: BTreeMap<&str, usize> = BTreeMap::new();
        for (_, a, b) in &lines {
            *degree.entry(a).or_default() += 1;
            *degree.entry(b).or_default() += 1;
        }
        if degree.values().any(|d| *d > 2) {
            return Err(err(
                "those lines branch: select one continuous chain",
                ids.to_vec(),
            ));
        }
        let closed = degree.values().all(|d| *d == 2);
        let start_pt = if closed {
            degree.keys().next().map(|s| (*s).to_owned())
        } else {
            degree
                .iter()
                .find(|(_, d)| **d == 1)
                .map(|(p, _)| (*p).to_owned())
        }
        .ok_or_else(|| err("no chain to offset", ids.to_vec()))?;
        let mut order: Vec<(String, String, String)> = Vec::new(); // (line, from point, to point)
        let mut at = start_pt.clone();
        let mut left: Vec<&(String, String, String)> = lines.iter().collect();
        while let Some(i) = left.iter().position(|(_, a, b)| *a == at || *b == at) {
            let (id, a, b) = left.remove(i).clone();
            let (from, to) = if a == at { (a, b) } else { (b, a) };
            order.push((id, from, to.clone()));
            at = to;
        }
        if !left.is_empty() {
            return Err(err("those lines are not one connected chain", ids.to_vec()));
        }
        let pos = |me: &Self, p: &str| {
            me.point(p)
                .ok_or_else(|| err("missing point", vec![p.to_owned()]))
        };
        let mut pts: Vec<P> = Vec::new();
        for (_, from, _) in &order {
            pts.push(pos(self, from)?);
        }
        pts.push(pos(self, &order[order.len() - 1].2)?);
        // Outside of a closed outline is positive: flip when it runs counter-clockwise.
        let mut d = distance;
        if closed {
            let area: f64 = (0..order.len())
                .map(|i| pts[i].0 * pts[i + 1].1 - pts[i + 1].0 * pts[i].1)
                .sum();
            if area > 0.0 {
                d = -d;
            }
        }
        let n = order.len();
        let dir = |i: usize| {
            let (a, b) = (pts[i], pts[i + 1]);
            let l = dist(a, b).max(1e-12);
            ((b.0 - a.0) / l, (b.1 - a.1) / l)
        };
        // Offset line i passes through pts[i] + normal_i * d.
        let shifted = |i: usize| {
            let u = dir(i);
            ((pts[i].0 - u.1 * d, pts[i].1 + u.0 * d), u)
        };
        let corner = |i: usize, j: usize| -> P {
            // Meeting of offset line i and offset line j at pts[j].
            let ((p, u), (q, v)) = (shifted(i), shifted(j));
            let cross = u.0 * v.1 - u.1 * v.0;
            if cross.abs() < 1e-9 {
                q
            } else {
                let w = (q.0 - p.0, q.1 - p.1);
                let t = (w.0 * v.1 - w.1 * v.0) / cross;
                (p.0 + u.0 * t, p.1 + u.1 * t)
            }
        };
        let mut verts: Vec<P> = Vec::new();
        if closed {
            for j in 0..n {
                verts.push(corner((j + n - 1) % n, j));
            }
        } else {
            verts.push(shifted(0).0);
            for j in 1..n {
                verts.push(corner(j - 1, j));
            }
            let (p, u) = shifted(n - 1);
            let l = dist(pts[n - 1], pts[n]);
            verts.push((p.0 + u.0 * l, p.1 + u.1 * l));
        }
        let new_pts: Vec<String> = verts.iter().map(|v| self.new_point(*v)).collect();
        rep.created.extend(new_pts.iter().cloned());
        for (i, (orig, _, _)) in order.iter().enumerate() {
            let (a, b) = if closed {
                (new_pts[i].clone(), new_pts[(i + 1) % n].clone())
            } else {
                (new_pts[i].clone(), new_pts[i + 1].clone())
            };
            let nid = self.fresh_id("l");
            self.entities.insert(nid.clone(), Entity::Line { a, b });
            rep.created.push(nid.clone());
            let cid = self.fresh_id("c");
            self.constraints.insert(
                cid,
                Constraint::Parallel {
                    a: orig.clone(),
                    b: nid,
                },
            );
        }
        Ok(rep)
    }

    /// The two lines' shared end point, the far ends, and unit vectors from the corner along each.
    fn corner_geometry(
        &self,
        a: &str,
        b: &str,
    ) -> Result<(String, P, P, P, f64, f64), SketchError> {
        let (Some(Entity::Line { a: a1, b: a2 }), Some(Entity::Line { a: b1, b: b2 })) =
            (self.entity(a).cloned(), self.entity(b).cloned())
        else {
            return Err(err("pick two lines", vec![a.to_owned(), b.to_owned()]));
        };
        if a == b {
            return Err(err("pick two different lines", vec![a.to_owned()]));
        }
        let shared = [&a1, &a2]
            .into_iter()
            .find(|p| **p == b1 || **p == b2)
            .cloned()
            .ok_or_else(|| {
                err(
                    "the two lines must meet at a shared end point (merge their ends first)",
                    vec![a.to_owned(), b.to_owned()],
                )
            })?;
        let far = |x: &String, y: &String| if *x == shared { y.clone() } else { x.clone() };
        let p = self
            .point(&shared)
            .ok_or_else(|| err("missing corner", vec![shared.clone()]))?;
        let pa = self
            .point(&far(&a1, &a2))
            .ok_or_else(|| err("missing end", vec![a.to_owned()]))?;
        let pb = self
            .point(&far(&b1, &b2))
            .ok_or_else(|| err("missing end", vec![b.to_owned()]))?;
        let (la, lb) = (dist(p, pa), dist(p, pb));
        if la < 1e-9 || lb < 1e-9 {
            return Err(err(
                "a line has no length",
                vec![a.to_owned(), b.to_owned()],
            ));
        }
        let (u, v) = (
            ((pa.0 - p.0) / la, (pa.1 - p.1) / la),
            ((pb.0 - p.0) / lb, (pb.1 - p.1) / lb),
        );
        let cross = u.0 * v.1 - u.1 * v.0;
        if cross.abs() < 1e-9 {
            return Err(err(
                "those lines run along one another: there is no corner",
                vec![a.to_owned(), b.to_owned()],
            ));
        }
        Ok((shared, p, u, v, la, lb))
    }

    /// Replaces the corner point of line `line` by `new`.
    fn move_line_end(&mut self, line: &str, corner: &str, new: &str) {
        let end = match self.entity(line) {
            Some(Entity::Line { a, .. }) if a == corner => End::Start,
            _ => End::Finish,
        };
        self.replace_end(line, end, new);
        self.after_shorten(line);
    }

    /// Rounds the corner where two lines meet with an arc of the given radius, tangent to both.
    pub fn fillet(&mut self, a: &str, b: &str, radius: f64) -> Result<EditReport, SketchError> {
        if !radius.is_finite() || radius <= 0.0 {
            return Err(err("the fillet radius must be a positive number", vec![]));
        }
        let (shared, p, u, v, la, lb) = self.corner_geometry(a, b)?;
        let theta = det::atan2((u.0 * v.1 - u.1 * v.0).abs(), u.0 * v.0 + u.1 * v.1);
        let half = theta / 2.0;
        let t = radius * det::cos(half) / det::sin(half);
        if t >= la - 1e-9 || t >= lb - 1e-9 {
            return Err(err(
                format!("the corner is too short for a fillet of radius {radius}: it needs {t:.3} mm of each line"),
                vec![a.to_owned(), b.to_owned()],
            ));
        }
        let bis = {
            let s = (u.0 + v.0, u.1 + v.1);
            let l = det::hypot(s.0, s.1);
            (s.0 / l, s.1 / l)
        };
        let reach = radius / det::sin(half);
        let centre = (p.0 + bis.0 * reach, p.1 + bis.1 * reach);
        let (t1, t2) = (
            (p.0 + u.0 * t, p.1 + u.1 * t),
            (p.0 + v.0 * t, p.1 + v.1 * t),
        );
        let mut rep = EditReport::default();
        let (n1, n2, nc) = (
            self.new_point(t1),
            self.new_point(t2),
            self.new_point(centre),
        );
        rep.created.extend([n1.clone(), n2.clone(), nc.clone()]);
        // Counter-clockwise from the first to the second tangent point, the short way round.
        let ccw =
            (t1.0 - centre.0) * (t2.1 - centre.1) - (t1.1 - centre.1) * (t2.0 - centre.0) > 0.0;
        let (start, end) = if ccw {
            (n1.clone(), n2.clone())
        } else {
            (n2.clone(), n1.clone())
        };
        let arc = self.fresh_id("a");
        self.entities.insert(
            arc.clone(),
            Entity::Arc {
                center: nc,
                start,
                end,
            },
        );
        rep.created.push(arc.clone());
        self.move_line_end(a, &shared, &n1);
        self.move_line_end(b, &shared, &n2);
        for (line, tag) in [(a, "t"), (b, "t")] {
            let cid = self.fresh_id(tag);
            self.constraints.insert(
                cid,
                Constraint::Tangent {
                    line: line.to_owned(),
                    curve: arc.clone(),
                },
            );
        }
        let cid = self.fresh_id("d");
        self.constraints.insert(
            cid,
            Constraint::Radius {
                target: arc,
                value: radius,
            },
        );
        Ok(rep)
    }

    /// Cuts the corner where two lines meet with a straight line, `d1` back along the first line
    /// and `d2` along the second.
    pub fn chamfer(
        &mut self,
        a: &str,
        b: &str,
        d1: f64,
        d2: f64,
    ) -> Result<EditReport, SketchError> {
        if !d1.is_finite() || !d2.is_finite() || d1 <= 0.0 || d2 <= 0.0 {
            return Err(err(
                "the chamfer distances must be positive numbers",
                vec![],
            ));
        }
        let (shared, p, u, v, la, lb) = self.corner_geometry(a, b)?;
        if d1 >= la - 1e-9 || d2 >= lb - 1e-9 {
            return Err(err(
                "the corner is too short for that chamfer",
                vec![a.to_owned(), b.to_owned()],
            ));
        }
        let mut rep = EditReport::default();
        let n1 = self.new_point((p.0 + u.0 * d1, p.1 + u.1 * d1));
        let n2 = self.new_point((p.0 + v.0 * d2, p.1 + v.1 * d2));
        rep.created.extend([n1.clone(), n2.clone()]);
        let line = self.fresh_id("l");
        self.entities.insert(
            line.clone(),
            Entity::Line {
                a: n1.clone(),
                b: n2.clone(),
            },
        );
        rep.created.push(line);
        self.move_line_end(a, &shared, &n1);
        self.move_line_end(b, &shared, &n2);
        Ok(rep)
    }

    /// Tidies a sketch: merges points that sit in the same place, removes lines with no length and
    /// repeated lines, and drops relations that are exact repeats of another.
    pub fn repair(&mut self) -> EditReport {
        let mut rep = EditReport::default();
        // Points in the same place become one (never two anchors pinned apart).
        loop {
            let pts: Vec<(String, P)> = self
                .entities
                .iter()
                .filter_map(|(id, e)| match e {
                    Entity::Point { x, y } => Some((id.clone(), (*x, *y))),
                    _ => None,
                })
                .collect();
            let mut found: Option<(String, String)> = None;
            'outer: for i in 0..pts.len() {
                for j in i + 1..pts.len() {
                    if dist(pts[i].1, pts[j].1) < 1e-7 {
                        found = Some((pts[i].0.clone(), pts[j].0.clone()));
                        break 'outer;
                    }
                }
            }
            let Some((a, b)) = found else { break };
            match self.merge_points(&a, &b) {
                Ok(r) => {
                    rep.removed.extend(r.removed);
                    rep.dropped.extend(r.dropped);
                }
                Err(_) => break, // would collapse a shape: leave it to the next pass below
            }
        }
        // Lines with no length, and lines repeated on the same two end points (the one with
        // the most relations on it is kept).
        let lines: Vec<(String, String, String)> = self
            .entities
            .iter()
            .filter_map(|(id, e)| match e {
                Entity::Line { a, b } => Some((id.clone(), a.clone(), b.clone())),
                _ => None,
            })
            .collect();
        let mut groups: BTreeMap<(String, String), Vec<String>> = BTreeMap::new();
        for (id, a, b) in lines {
            let zero = a == b
                || self
                    .point(&a)
                    .zip(self.point(&b))
                    .is_some_and(|(p, q)| dist(p, q) < 1e-9);
            if zero {
                self.delete_shape(&id, &mut rep);
                continue;
            }
            let key = if a <= b { (a, b) } else { (b, a) };
            groups.entry(key).or_default().push(id);
        }
        for (_, mut group) in groups {
            if group.len() < 2 {
                continue;
            }
            let uses = |me: &Self, id: &String| {
                me.constraints
                    .values()
                    .filter(|c| c.refs().contains(&id.as_str()))
                    .count()
            };
            group.sort_by(|x, y| uses(self, y).cmp(&uses(self, x)).then_with(|| x.cmp(y)));
            for extra in group.into_iter().skip(1) {
                self.delete_shape(&extra, &mut rep);
            }
        }
        // Relations that say exactly what another already says.
        let mut seen: Vec<Constraint> = Vec::new();
        let ids: Vec<String> = self.constraints.keys().cloned().collect();
        for id in ids {
            let c = self.constraints[&id].clone();
            if seen.contains(&c) {
                self.constraints.remove(&id);
                self.prune_placements();
                rep.dropped.push(id);
            } else {
                seen.push(c);
            }
        }
        rep
    }
}

impl Sketch {
    /// SolidWorks' "fully define sketch" assistant: adds the relations and dimensions that are
    /// still missing, using what the geometry is now (an almost level line is made level, a free
    /// length gets its current length, ...), one at a time and only if each really removes a
    /// degree of freedom. Returns the constraints it added; whatever could not be pinned down is
    /// left (look at the solve report).
    pub fn fully_define(&mut self) -> Result<EditReport, SketchError> {
        let mut rep = EditReport::default();
        if self.entities.is_empty() {
            return Err(err("there is nothing in the sketch to define", vec![]));
        }
        let mut report = self.solve();
        if !report.applied {
            return Err(err(
                "the sketch cannot be solved as it is: fix the conflict first",
                report.conflicting.clone(),
            ));
        }
        // Level and upright lines first: they are free and obvious.
        let aligned: Vec<(String, bool)> = self
            .entities
            .iter()
            .filter_map(|(id, e)| match e {
                Entity::Line { a, b } => {
                    let (p, q) = (self.point(a)?, self.point(b)?);
                    let l = dist(p, q);
                    if l < 1e-9 {
                        return None;
                    }
                    let (dx, dy) = ((q.0 - p.0).abs() / l, (q.1 - p.1).abs() / l);
                    if dy < 0.026 {
                        Some((id.clone(), true))
                    } else if dx < 0.026 {
                        Some((id.clone(), false))
                    } else {
                        None
                    }
                }
                _ => None,
            })
            .filter(|(id, _)| {
                !self.constraints.values().any(|c| {
                    matches!(c, Constraint::Horizontal { line } | Constraint::Vertical { line } if line == id)
                })
            })
            .collect();
        for (line, horizontal) in aligned {
            let c = if horizontal {
                Constraint::Horizontal { line }
            } else {
                Constraint::Vertical { line }
            };
            self.try_add(c, "c", &mut report, &mut rep);
        }
        // Something must hold the whole sketch in place.
        let anchored = self
            .constraints
            .values()
            .any(|c| matches!(c, Constraint::Fixed { .. } | Constraint::OnAxis { .. }));
        let mut anchor: Option<String> = self.constraints.values().find_map(|c| match c {
            Constraint::Fixed { point, .. } => Some(point.clone()),
            _ => None,
        });
        if !anchored {
            let nearest = self
                .entities
                .iter()
                .filter_map(|(id, e)| match e {
                    Entity::Point { x, y } => Some((id.clone(), det::hypot(*x, *y))),
                    _ => None,
                })
                .min_by(|a, b| a.1.total_cmp(&b.1).then_with(|| a.0.cmp(&b.0)));
            if let Some((id, _)) = nearest {
                let (x, y) = self.point(&id).unwrap_or((0.0, 0.0));
                if self.try_add(
                    Constraint::Fixed {
                        point: id.clone(),
                        x,
                        y,
                    },
                    "c",
                    &mut report,
                    &mut rep,
                ) {
                    anchor = Some(id);
                }
            }
        }
        // Then dimensions, one degree of freedom at a time.
        for _ in 0..400 {
            if report.dof == 0 {
                break;
            }
            let mut tried = false;
            for (c, prefix) in self.definition_candidates(&report, anchor.as_deref()) {
                if self.try_add(c, prefix, &mut report, &mut rep) {
                    tried = true;
                    break;
                }
            }
            if !tried {
                break;
            }
        }
        Ok(rep)
    }

    /// Adds the constraint if it solves and removes at least one degree of freedom.
    fn try_add(
        &mut self,
        c: Constraint,
        prefix: &str,
        report: &mut crate::SolveReport,
        rep: &mut EditReport,
    ) -> bool {
        let mut trial = self.clone();
        let id = trial.fresh_id(prefix);
        if trial.add_constraint(&id, c).is_err() {
            return false;
        }
        let r = trial.solve();
        if !r.applied || r.dof >= report.dof || !r.conflicting.is_empty() {
            return false;
        }
        *self = trial;
        *report = r;
        rep.created.push(id);
        true
    }

    /// Dimensions that could take away what is still free, most natural first.
    fn definition_candidates(
        &self,
        report: &crate::SolveReport,
        anchor: Option<&str>,
    ) -> Vec<(Constraint, &'static str)> {
        let mut out: Vec<(Constraint, &'static str)> = Vec::new();
        for (id, e) in &self.entities {
            match e {
                Entity::Circle { radius, .. } if report.radius_free.contains(id) => out.push((
                    Constraint::Radius {
                        target: id.clone(),
                        value: *radius,
                    },
                    "d",
                )),
                Entity::Arc { center, start, .. } if report.radius_free.contains(id) => {
                    if let Some((c, s)) = self.point(center).zip(self.point(start)) {
                        out.push((
                            Constraint::Radius {
                                target: id.clone(),
                                value: dist(c, s),
                            },
                            "d",
                        ));
                    }
                }
                _ => {}
            }
        }
        for (id, e) in &self.entities {
            if let Entity::Line { a, b } = e {
                let free = report.motion.contains_key(a) || report.motion.contains_key(b);
                if let (true, Some((p, q))) = (free, self.point(a).zip(self.point(b))) {
                    out.push((
                        Constraint::Length {
                            line: id.clone(),
                            value: dist(p, q),
                        },
                        "d",
                    ));
                }
            }
        }
        for (id, e) in &self.entities {
            if let Entity::Arc { center, start, end } = e {
                if report.sweep_free.contains(id) {
                    let (Some(c), Some(s), Some(en)) =
                        (self.point(center), self.point(start), self.point(end))
                    else {
                        continue;
                    };
                    let tau = core::f64::consts::TAU;
                    let mut sweep = (det::atan2(en.1 - c.1, en.0 - c.0)
                        - det::atan2(s.1 - c.1, s.0 - c.0))
                        % tau;
                    if sweep < 0.0 {
                        sweep += tau;
                    }
                    out.push((
                        Constraint::ArcLength {
                            arc: id.clone(),
                            value: dist(c, s) * sweep,
                        },
                        "d",
                    ));
                }
            }
        }
        if let Some(anchor) = anchor {
            if let Some(at) = self.point(anchor) {
                for id in report.motion.keys() {
                    if id == anchor {
                        continue;
                    }
                    let Some(p) = self.point(id) else { continue };
                    out.push((
                        Constraint::HorizontalDistance {
                            a: anchor.to_owned(),
                            b: id.clone(),
                            value: p.0 - at.0,
                        },
                        "d",
                    ));
                    out.push((
                        Constraint::VerticalDistance {
                            a: anchor.to_owned(),
                            b: id.clone(),
                            value: p.1 - at.1,
                        },
                        "d",
                    ));
                }
            }
        }
        out
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

    fn near(a: P, b: P) -> bool {
        dist(a, b) < 1e-6
    }

    fn solved(s: &mut Sketch) -> crate::SolveReport {
        let r = s.solve();
        assert_eq!(r.status, SolveStatus::Solved, "{r:?}");
        assert!(r.applied);
        r
    }

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

    /// A 40 x 20 rectangle p1..p4 with edges l1..l4, level / upright.
    fn rect() -> Sketch {
        let mut s = Sketch::new();
        for (id, x, y) in [
            ("p1", 0.0, 0.0),
            ("p2", 40.0, 0.0),
            ("p3", 40.0, 20.0),
            ("p4", 0.0, 20.0),
        ] {
            pt(&mut s, id, x, y);
        }
        for (id, a, b) in [
            ("l1", "p1", "p2"),
            ("l2", "p2", "p3"),
            ("l3", "p3", "p4"),
            ("l4", "p4", "p1"),
        ] {
            line(&mut s, id, a, b);
        }
        for (id, l) in [("h1", "l1"), ("h3", "l3")] {
            s.add_constraint(id, Horizontal { line: l.into() }).unwrap();
        }
        for (id, l) in [("v2", "l2"), ("v4", "l4")] {
            s.add_constraint(id, Vertical { line: l.into() }).unwrap();
        }
        s
    }

    fn all(ids: &[&str]) -> Vec<String> {
        ids.iter().map(|s| (*s).to_owned()).collect()
    }

    #[test]
    fn move_rotate_and_scale_work_in_place_or_on_copies() {
        let mut s = rect();
        s.transform(
            &all(&["l1", "l2", "l3", "l4"]),
            &Transform::Move { dx: 5.0, dy: -3.0 },
            false,
        )
        .unwrap();
        assert!(
            near(s.point("p1").unwrap(), (5.0, -3.0)) && near(s.point("p3").unwrap(), (45.0, 17.0))
        );
        solved(&mut s);
        // A copy: the original stays, relations carry (but an anchor would not).
        let mut t = rect();
        t.add_constraint(
            "fx",
            Fixed {
                point: "p1".into(),
                x: 0.0,
                y: 0.0,
            },
        )
        .unwrap();
        let rep = t
            .transform(
                &all(&["l1", "l2", "l3", "l4"]),
                &Transform::Move { dx: 100.0, dy: 0.0 },
                true,
            )
            .unwrap();
        assert!(near(t.point("p1").unwrap(), (0.0, 0.0)));
        assert_eq!(
            rep.created
                .iter()
                .filter(|c| matches!(t.entity(c), Some(Entity::Line { .. })))
                .count(),
            4
        );
        assert_eq!(
            t.constraints()
                .values()
                .filter(|c| matches!(c, Horizontal { .. }))
                .count(),
            4,
            "2 + 2"
        );
        assert_eq!(
            t.constraints()
                .values()
                .filter(|c| matches!(c, Fixed { .. }))
                .count(),
            1,
            "the anchor stays on the original"
        );
        consistent(&t);
        solved(&mut t);
        // Rotate a quarter turn about the origin: level becomes upright.
        let mut r = rect();
        r.transform(
            &all(&["l1", "l2", "l3", "l4"]),
            &Transform::Rotate {
                cx: 0.0,
                cy: 0.0,
                degrees: 90.0,
            },
            true,
        )
        .unwrap();
        assert_eq!(
            r.constraints()
                .values()
                .filter(|c| matches!(c, Horizontal { .. }))
                .count(),
            2 + 2,
            "2 original + the 2 edges that were upright"
        );
        assert_eq!(
            r.constraints()
                .values()
                .filter(|c| matches!(c, Vertical { .. }))
                .count(),
            2 + 2
        );
        solved(&mut r);
        // Another angle: direction relations are not carried.
        let mut q = rect();
        q.transform(
            &all(&["l1", "l2", "l3", "l4"]),
            &Transform::Rotate {
                cx: 0.0,
                cy: 0.0,
                degrees: 30.0,
            },
            true,
        )
        .unwrap();
        assert_eq!(
            q.constraints()
                .values()
                .filter(|c| matches!(c, Horizontal { .. }))
                .count(),
            2
        );
        // Scale in place doubles lengths and circle radii.
        let mut u = rect();
        pt(&mut u, "c", 10.0, 10.0);
        u.add_entity(
            "k",
            Entity::Circle {
                center: "c".into(),
                radius: 3.0,
            },
        )
        .unwrap();
        u.transform(
            &all(&["l1", "l2", "l3", "l4", "k"]),
            &Transform::Scale {
                cx: 0.0,
                cy: 0.0,
                factor: 2.0,
            },
            false,
        )
        .unwrap();
        assert!(near(u.point("p3").unwrap(), (80.0, 40.0)));
        assert!(
            matches!(u.entity("k"), Some(Entity::Circle { radius, .. }) if (radius - 6.0).abs() < 1e-9)
        );
        // Refusals.
        assert!(u
            .transform(&[], &Transform::Move { dx: 1.0, dy: 0.0 }, false)
            .is_err());
        assert!(u
            .transform(
                &all(&["nope"]),
                &Transform::Move { dx: 1.0, dy: 0.0 },
                false
            )
            .is_err());
        assert!(u
            .transform(
                &all(&["l1"]),
                &Transform::Scale {
                    cx: 0.0,
                    cy: 0.0,
                    factor: 0.0
                },
                false
            )
            .is_err());
        assert!(u
            .transform(
                &all(&["l1"]),
                &Transform::Move {
                    dx: f64::NAN,
                    dy: 0.0
                },
                false
            )
            .is_err());
    }

    #[test]
    fn a_pinned_sketch_refuses_to_be_moved_with_a_clash() {
        let mut s = rect();
        s.add_constraint(
            "fx",
            Fixed {
                point: "p1".into(),
                x: 0.0,
                y: 0.0,
            },
        )
        .unwrap();
        s.transform(&all(&["l1"]), &Transform::Move { dx: 5.0, dy: 0.0 }, false)
            .unwrap();
        let r = s.solve();
        assert!(r.applied, "the solver puts the anchored point back: {r:?}");
        assert!(near(s.point("p1").unwrap(), (0.0, 0.0)));
    }

    #[test]
    fn mirroring_makes_symmetric_copies_that_follow_the_original() {
        let mut s = rect();
        let rep = s
            .mirror(&all(&["l1", "l2", "l3", "l4"]), &MirrorAbout::AxisY)
            .unwrap();
        assert!(rep.created.len() >= 6, "{rep:?}");
        // Mirrored about x = 0: the new x are -40..0 (the edge on the axis is shared).
        let xs: Vec<f64> = s
            .entities()
            .values()
            .filter_map(|e| match e {
                Entity::Point { x, .. } => Some(*x),
                _ => None,
            })
            .collect();
        assert!(xs.iter().any(|x| (x + 40.0).abs() < 1e-9), "{xs:?}");
        // Points on the axis are reused, not duplicated: p1 and p4 are at x = 0.
        assert_eq!(xs.iter().filter(|x| x.abs() < 1e-9).count(), 2, "{xs:?}");
        assert!(s
            .constraints()
            .values()
            .any(|c| matches!(c, SymmetricAxis { .. })));
        consistent(&s);
        solved(&mut s);
        // Moving an original point moves its mirror image.
        let mut t = rect();
        t.mirror(&all(&["l1", "l2", "l3", "l4"]), &MirrorAbout::AxisY)
            .unwrap();
        t.add_constraint(
            "fx",
            Fixed {
                point: "p1".into(),
                x: 0.0,
                y: 0.0,
            },
        )
        .unwrap();
        t.set_point("p3", 55.0, 30.0).unwrap();
        solved(&mut t);
        // (The solver may nudge the whole rectangle to keep it square; whatever p3 ended up
        // as, the mirror image of that is also a point of the sketch.)
        let (px, py) = t.point("p3").unwrap();
        assert!(px > 41.0, "it did move: {px}");
        assert!(
            t.entities()
                .iter()
                .any(|(_, e)| matches!(e, Entity::Point { x, y } if near((*x, *y), (-px, py)))),
            "the mirror image of p3 moved with it"
        );

        // About a line of the sketch, and an arc keeps counter-clockwise order.
        let mut u = Sketch::new();
        pt(&mut u, "m1", 0.0, 0.0);
        pt(&mut u, "m2", 0.0, 10.0);
        line(&mut u, "mirror", "m1", "m2");
        pt(&mut u, "c", 10.0, 0.0);
        pt(&mut u, "s", 15.0, 0.0);
        pt(&mut u, "e", 10.0, 5.0);
        u.add_entity(
            "arc",
            Entity::Arc {
                center: "c".into(),
                start: "s".into(),
                end: "e".into(),
            },
        )
        .unwrap();
        u.mirror(
            &all(&["arc", "mirror"]),
            &MirrorAbout::Line {
                entity: "mirror".into(),
            },
        )
        .unwrap();
        let (mc, ms, me) = u
            .entities()
            .iter()
            .filter(|(id, _)| *id != "arc")
            .find_map(|(_, e)| match e {
                Entity::Arc { center, start, end } => {
                    Some((center.clone(), start.clone(), end.clone()))
                }
                _ => None,
            })
            .unwrap();
        let (c, st, en) = (
            u.point(&mc).unwrap(),
            u.point(&ms).unwrap(),
            u.point(&me).unwrap(),
        );
        assert!(near(c, (-10.0, 0.0)));
        let cross = (st.0 - c.0) * (en.1 - c.1) - (st.1 - c.1) * (en.0 - c.0);
        assert!(cross > 0.0, "still counter-clockwise: {st:?} -> {en:?}");
        consistent(&u);
        solved(&mut u);
        // Refusals.
        assert!(u
            .mirror(
                &all(&["arc"]),
                &MirrorAbout::Line {
                    entity: "arc".into()
                }
            )
            .is_err());
        assert!(u.mirror(&[], &MirrorAbout::AxisX).is_err());
    }

    #[test]
    fn patterns_make_the_right_number_of_copies_in_the_right_places() {
        let mut s = Sketch::new();
        pt(&mut s, "c", 10.0, 0.0);
        s.add_entity(
            "k",
            Entity::Circle {
                center: "c".into(),
                radius: 2.0,
            },
        )
        .unwrap();
        s.pattern(
            &all(&["k"]),
            &Pattern::Linear {
                dx: 10.0,
                dy: 5.0,
                count: 4,
            },
        )
        .unwrap();
        let circles = s
            .entities()
            .values()
            .filter(|e| matches!(e, Entity::Circle { .. }))
            .count();
        assert_eq!(circles, 4);
        assert!(s.entities().values().any(|e| matches!(e, Entity::Point { x, y } if (x - 40.0).abs() < 1e-9 && (y - 15.0).abs() < 1e-9)));
        // A full turn of 4 about the origin: 90 degrees apart.
        let mut t = Sketch::new();
        pt(&mut t, "c", 10.0, 0.0);
        t.add_entity(
            "k",
            Entity::Circle {
                center: "c".into(),
                radius: 2.0,
            },
        )
        .unwrap();
        t.pattern(
            &all(&["k"]),
            &Pattern::Circular {
                cx: 0.0,
                cy: 0.0,
                degrees: 360.0,
                count: 4,
            },
        )
        .unwrap();
        for want in [(0.0, 10.0), (-10.0, 0.0), (0.0, -10.0)] {
            assert!(
                t.entities()
                    .values()
                    .any(|e| matches!(e, Entity::Point { x, y } if near((*x, *y), want))),
                "{want:?}"
            );
        }
        // Part of a turn runs first to last.
        let mut u = Sketch::new();
        pt(&mut u, "c", 10.0, 0.0);
        u.add_entity(
            "k",
            Entity::Circle {
                center: "c".into(),
                radius: 2.0,
            },
        )
        .unwrap();
        u.pattern(
            &all(&["k"]),
            &Pattern::Circular {
                cx: 0.0,
                cy: 0.0,
                degrees: 90.0,
                count: 3,
            },
        )
        .unwrap();
        assert!(u
            .entities()
            .values()
            .any(|e| matches!(e, Entity::Point { x, y } if near((*x, *y), (0.0, 10.0)))));
        // Refusals.
        assert!(u
            .pattern(
                &all(&["k"]),
                &Pattern::Linear {
                    dx: 1.0,
                    dy: 0.0,
                    count: 1
                }
            )
            .is_err());
        assert!(u
            .pattern(
                &all(&["k"]),
                &Pattern::Linear {
                    dx: 0.0,
                    dy: 0.0,
                    count: 3
                }
            )
            .is_err());
        assert!(u
            .pattern(
                &all(&["k"]),
                &Pattern::Linear {
                    dx: 1.0,
                    dy: 0.0,
                    count: 500
                }
            )
            .is_err());
        assert!(u
            .pattern(
                &all(&["k"]),
                &Pattern::Circular {
                    cx: 0.0,
                    cy: 0.0,
                    degrees: 0.0,
                    count: 3
                }
            )
            .is_err());
    }

    #[test]
    fn offsetting_a_closed_outline_a_chain_a_circle_and_an_arc() {
        let mut s = rect();
        s.offset(&all(&["l1", "l2", "l3", "l4"]), 3.0).unwrap();
        // The positive side of a closed outline is outside: 46 x 26 around it.
        let xs: Vec<f64> = s
            .entities()
            .values()
            .filter_map(|e| match e {
                Entity::Point { x, .. } => Some(*x),
                _ => None,
            })
            .collect();
        assert!(
            xs.iter().any(|x| (x + 3.0).abs() < 1e-9) && xs.iter().any(|x| (x - 43.0).abs() < 1e-9),
            "{xs:?}"
        );
        assert_eq!(
            s.constraints()
                .values()
                .filter(|c| matches!(c, Parallel { .. }))
                .count(),
            4
        );
        consistent(&s);
        solved(&mut s);
        // Negative goes inside.
        let mut t = rect();
        t.offset(&all(&["l1", "l2", "l3", "l4"]), -3.0).unwrap();
        assert!(t
            .entities()
            .values()
            .any(|e| matches!(e, Entity::Point { x, y } if near((*x, *y), (3.0, 3.0)))));
        // An open chain: left of the way it is walked (from the lower-id end).
        let mut u = Sketch::new();
        pt(&mut u, "a", 0.0, 0.0);
        pt(&mut u, "b", 10.0, 0.0);
        pt(&mut u, "c", 10.0, 10.0);
        line(&mut u, "ab", "a", "b");
        line(&mut u, "bc", "b", "c");
        u.offset(&all(&["ab", "bc"]), 2.0).unwrap();
        // Walking a -> b -> c the left side is up and then to the left (inside the corner).
        assert!(u
            .entities()
            .values()
            .any(|e| matches!(e, Entity::Point { x, y } if near((*x, *y), (0.0, 2.0)))));
        assert!(
            u.entities()
                .values()
                .any(|e| matches!(e, Entity::Point { x, y } if near((*x, *y), (8.0, 2.0)))),
            "the corner is a sharp meeting of the offset lines"
        );
        assert!(u
            .entities()
            .values()
            .any(|e| matches!(e, Entity::Point { x, y } if near((*x, *y), (8.0, 10.0)))));
        solved(&mut u);
        // A circle grows concentrically; an arc keeps its angles.
        let mut v = Sketch::new();
        pt(&mut v, "c", 0.0, 0.0);
        v.add_entity(
            "k",
            Entity::Circle {
                center: "c".into(),
                radius: 5.0,
            },
        )
        .unwrap();
        v.offset(&all(&["k"]), 2.0).unwrap();
        assert!(v
            .entities()
            .values()
            .any(|e| matches!(e, Entity::Circle { radius, .. } if (radius - 7.0).abs() < 1e-9)));
        assert!(v.offset(&all(&["k"]), -9.0).is_err(), "cannot shrink away");
        pt(&mut v, "s", 5.0, 0.0);
        pt(&mut v, "e", 0.0, 5.0);
        v.add_entity(
            "arc",
            Entity::Arc {
                center: "c".into(),
                start: "s".into(),
                end: "e".into(),
            },
        )
        .unwrap();
        v.offset(&all(&["arc"]), 3.0).unwrap();
        assert!(v
            .entities()
            .values()
            .any(|e| matches!(e, Entity::Point { x, y } if near((*x, *y), (8.0, 0.0)))));
        consistent(&v);
        solved(&mut v);
        // Refusals: nothing, zero, a mixture, a branching set, two separate lines.
        assert!(v.offset(&[], 1.0).is_err());
        assert!(v.offset(&all(&["arc"]), 0.0).is_err());
        let mut w = rect();
        pt(&mut w, "c", 50.0, 50.0);
        w.add_entity(
            "k",
            Entity::Circle {
                center: "c".into(),
                radius: 1.0,
            },
        )
        .unwrap();
        assert!(w.offset(&all(&["l1", "k"]), 1.0).is_err());
        pt(&mut w, "x", 20.0, 10.0);
        line(&mut w, "spur", "x", "p1");
        line(&mut w, "spur2", "x", "p2");
        assert!(w.offset(&all(&["l1", "spur", "spur2", "l4"]), 1.0).is_err());
        assert!(w
            .offset(&all(&["l1", "l3"]), 1.0)
            .unwrap_err()
            .message
            .contains("connected"));
    }

    #[test]
    fn a_fillet_rounds_a_corner_tangent_to_both_lines_and_a_chamfer_cuts_it() {
        let mut s = rect();
        let rep = s.fillet("l1", "l2", 5.0).unwrap();
        let arc = rep
            .created
            .iter()
            .find(|c| matches!(s.entity(c), Some(Entity::Arc { .. })))
            .unwrap()
            .clone();
        // Tangent points are 5 mm back from the corner (a right angle: t = r).
        let Some(Entity::Line { b, .. }) = s.entity("l1").cloned() else {
            panic!()
        };
        assert!(near(s.point(&b).unwrap(), (35.0, 0.0)), "{:?}", s.point(&b));
        let Some(Entity::Line { a, .. }) = s.entity("l2").cloned() else {
            panic!()
        };
        assert!(near(s.point(&a).unwrap(), (40.0, 5.0)));
        assert!(s.entity("p2").is_none(), "the old corner is gone");
        assert!(
            s.constraints()
                .values()
                .filter(|c| matches!(c, Tangent { curve, .. } if *curve == arc))
                .count()
                == 2
        );
        assert!(s.constraints().values().any(|c| matches!(c, Radius { target, value } if *target == arc && (value - 5.0).abs() < 1e-9)));
        consistent(&s);
        solved(&mut s);
        let Some(Entity::Arc { center, start, end }) = s.entity(&arc).cloned() else {
            panic!()
        };
        assert!(near(s.point(&center).unwrap(), (35.0, 5.0)));
        let (c, st, en) = (
            s.point(&center).unwrap(),
            s.point(&start).unwrap(),
            s.point(&end).unwrap(),
        );
        assert!((dist(c, st) - 5.0).abs() < 1e-6 && (dist(c, en) - 5.0).abs() < 1e-6);
        assert!(
            (st.0 - c.0) * (en.1 - c.1) - (st.1 - c.1) * (en.0 - c.0) > 0.0,
            "counter-clockwise short way"
        );
        // All four corners can be rounded one after another.
        for (a, b) in [("l2", "l3"), ("l3", "l4"), ("l4", "l1")] {
            s.fillet(a, b, 2.0).unwrap();
        }
        consistent(&s);
        solved(&mut s);
        // An acute corner needs more of each line: too large is refused with the reason.
        let mut t = Sketch::new();
        pt(&mut t, "o", 0.0, 0.0);
        pt(&mut t, "a", 10.0, 0.0);
        pt(&mut t, "b", 8.0, 6.0);
        line(&mut t, "oa", "o", "a");
        line(&mut t, "ob", "o", "b");
        let e = t.fillet("oa", "ob", 20.0).unwrap_err();
        assert!(e.message.contains("too short"), "{e:?}");
        assert!(t.fillet("oa", "ob", -1.0).is_err());
        assert!(t.fillet("oa", "oa", 1.0).is_err());
        t.fillet("oa", "ob", 1.0).unwrap();
        consistent(&t);
        solved(&mut t);
        // Lines that do not meet: refused; chamfer.
        let mut u = rect();
        pt(&mut u, "x", 100.0, 100.0);
        pt(&mut u, "y", 110.0, 100.0);
        line(&mut u, "far", "x", "y");
        assert!(u
            .fillet("l1", "far", 1.0)
            .unwrap_err()
            .message
            .contains("meet"));
        let rep = u.chamfer("l1", "l2", 4.0, 6.0).unwrap();
        let cut = rep
            .created
            .iter()
            .find(|c| matches!(u.entity(c), Some(Entity::Line { .. })))
            .unwrap()
            .clone();
        let Some(Entity::Line { a, b }) = u.entity(&cut).cloned() else {
            panic!()
        };
        assert!(near(u.point(&a).unwrap(), (36.0, 0.0)) && near(u.point(&b).unwrap(), (40.0, 6.0)));
        consistent(&u);
        solved(&mut u);
        assert!(u.chamfer("l2", "l3", 100.0, 1.0).is_err());
    }

    #[test]
    fn repair_merges_duplicates_and_drops_zero_length_and_repeated_things() {
        let mut s = rect();
        // A second copy of the bottom edge on points sitting in the same places.
        pt(&mut s, "q1", 0.0, 0.0);
        pt(&mut s, "q2", 40.0, 0.0);
        line(&mut s, "dup", "q1", "q2");
        s.add_constraint("h1b", Horizontal { line: "l1".into() })
            .unwrap();
        let rep = s.repair();
        assert!(!rep.removed.is_empty());
        assert!(s.entity("dup").is_none(), "the repeated line is gone");
        assert_eq!(
            s.entities()
                .values()
                .filter(|e| matches!(e, Entity::Point { .. }))
                .count(),
            4
        );
        assert!(
            !s.constraints().contains_key("h1b") && s.constraints().contains_key("h1"),
            "{:?}",
            s.constraints().keys().collect::<Vec<_>>()
        );
        consistent(&s);
        solved(&mut s);
        // A clean sketch is left as it is.
        let mut t = rect();
        let before = t.clone();
        let rep = t.repair();
        assert!(rep.removed.is_empty() && rep.dropped.is_empty());
        assert_eq!(t, before);
        // Two anchored points in the same place are not merged into one anchor twice.
        let mut u = Sketch::new();
        pt(&mut u, "a", 0.0, 0.0);
        pt(&mut u, "b", 0.0, 0.0);
        u.add_constraint(
            "fa",
            Fixed {
                point: "a".into(),
                x: 0.0,
                y: 0.0,
            },
        )
        .unwrap();
        u.repair();
        assert_eq!(u.entities().len(), 1);
    }

    #[test]
    fn fully_define_adds_what_is_missing_and_nothing_more() {
        // A hand-drawn, slightly off rectangle with a circle: nothing is constrained.
        let mut s = Sketch::new();
        for (id, x, y) in [
            ("p1", 0.2, -0.1),
            ("p2", 40.1, 0.2),
            ("p3", 39.8, 20.1),
            ("p4", 0.1, 19.9),
        ] {
            pt(&mut s, id, x, y);
        }
        for (id, a, b) in [
            ("l1", "p1", "p2"),
            ("l2", "p2", "p3"),
            ("l3", "p3", "p4"),
            ("l4", "p4", "p1"),
        ] {
            line(&mut s, id, a, b);
        }
        pt(&mut s, "c", 20.0, 10.0);
        s.add_entity(
            "k",
            Entity::Circle {
                center: "c".into(),
                radius: 4.0,
            },
        )
        .unwrap();
        let rep = s.fully_define().unwrap();
        assert!(!rep.created.is_empty());
        let r = solved(&mut s);
        assert_eq!(r.dof, 0, "{r:?}");
        assert!(r.under_constrained.is_empty());
        consistent(&s);
        // The near-level / near-upright edges were made level / upright rather than dimensioned.
        assert_eq!(
            s.constraints()
                .values()
                .filter(|c| matches!(c, Horizontal { .. }))
                .count(),
            2
        );
        assert_eq!(
            s.constraints()
                .values()
                .filter(|c| matches!(c, Vertical { .. }))
                .count(),
            2
        );
        assert!(s.constraints().values().any(|c| matches!(c, Radius { .. })));
        assert!(
            s.constraints().values().any(|c| matches!(c, Fixed { .. })),
            "something anchors it"
        );
        // Running it again changes nothing.
        let again = s.fully_define().unwrap();
        assert!(again.created.is_empty(), "{again:?}");
        // A slanted line and an arc get a length and a radius / arc length.
        let mut t = Sketch::new();
        pt(&mut t, "o", 0.0, 0.0);
        pt(&mut t, "a", 10.0, 7.0);
        line(&mut t, "slant", "o", "a");
        pt(&mut t, "c", 30.0, 0.0);
        pt(&mut t, "s", 35.0, 0.0);
        pt(&mut t, "e", 30.0, 5.0);
        t.add_entity(
            "arc",
            Entity::Arc {
                center: "c".into(),
                start: "s".into(),
                end: "e".into(),
            },
        )
        .unwrap();
        t.fully_define().unwrap();
        let r = solved(&mut t);
        assert_eq!(r.dof, 0, "{r:?}");
        assert!(t.constraints().values().any(|c| matches!(c, Length { .. })));
        assert!(t
            .constraints()
            .values()
            .any(|c| matches!(c, ArcLength { .. })));
        // Refusals: empty, or already contradictory.
        assert!(Sketch::new().fully_define().is_err());
    }

    #[test]
    fn turning_in_place_takes_level_and_upright_with_it() {
        let all4 = all(&["l1", "l2", "l3", "l4"]);
        // A quarter turn swaps them: the rectangle is now 20 wide and 40 tall, and still square.
        let mut s = rect();
        s.transform(
            &all4,
            &Transform::Rotate {
                cx: 0.0,
                cy: 0.0,
                degrees: 90.0,
            },
            false,
        )
        .unwrap();
        assert_eq!(
            s.constraints()
                .values()
                .filter(|c| matches!(c, Horizontal { .. }))
                .count(),
            2
        );
        assert!(s
            .constraints()
            .get("h1")
            .is_some_and(|c| matches!(c, Vertical { .. })));
        solved(&mut s);
        assert!(
            near(s.point("p3").unwrap(), (-20.0, 40.0)),
            "{:?}",
            s.point("p3")
        );
        // A half turn keeps them.
        let mut t = rect();
        t.transform(
            &all4,
            &Transform::Rotate {
                cx: 0.0,
                cy: 0.0,
                degrees: 180.0,
            },
            false,
        )
        .unwrap();
        assert!(t
            .constraints()
            .get("h1")
            .is_some_and(|c| matches!(c, Horizontal { .. })));
        solved(&mut t);
        // Another angle: the direction relations go, and say so, and the shape really turns.
        let mut u = rect();
        let rep = u
            .transform(
                &all4,
                &Transform::Rotate {
                    cx: 0.0,
                    cy: 0.0,
                    degrees: 30.0,
                },
                false,
            )
            .unwrap();
        assert_eq!(rep.dropped.len(), 4, "{rep:?}");
        assert!(u.constraints().is_empty());
        solved(&mut u);
        let (a, b) = (u.point("p1").unwrap(), u.point("p2").unwrap());
        assert!((det::atan2(b.1 - a.1, b.0 - a.0).to_degrees() - 30.0).abs() < 1e-6);
        // Only the turned shapes lose them.
        let mut w = rect();
        pt(&mut w, "x", 100.0, 0.0);
        pt(&mut w, "y", 110.0, 0.0);
        line(&mut w, "free", "x", "y");
        w.add_constraint(
            "hf",
            Horizontal {
                line: "free".into(),
            },
        )
        .unwrap();
        w.transform(
            &all4,
            &Transform::Rotate {
                cx: 0.0,
                cy: 0.0,
                degrees: 30.0,
            },
            false,
        )
        .unwrap();
        assert!(w.constraints().contains_key("hf"));
    }
}
