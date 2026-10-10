//! Box selection in sketch mode: drag a rectangle on empty space. Left to right selects what is
//! entirely inside (window); right to left selects everything the rectangle touches (crossing),
//! as in SolidWorks. Logic only, no egui.

use crate::sketching::{
    angle_of, arc_sweep, defining_points, dist, Tool, P2, SEL_AXIS_X, SEL_AXIS_Y, SEL_ORIGIN,
};
use crate::state::AppState;
use dc_math::det;
use dc_sketch::Entity;
use std::collections::BTreeSet;

/// A rectangle being dragged out, in screen points.
#[derive(Clone, Copy, Debug)]
pub struct BoxSel {
    pub from: P2,
    pub to: P2,
}

impl BoxSel {
    /// Right to left: select what the rectangle touches, not only what it contains.
    pub fn crossing(&self) -> bool {
        self.to.0 < self.from.0
    }

    pub fn min(&self) -> P2 {
        (self.from.0.min(self.to.0), self.from.1.min(self.to.1))
    }

    pub fn max(&self) -> P2 {
        (self.from.0.max(self.to.0), self.from.1.max(self.to.1))
    }
}

fn inside(p: P2, lo: P2, hi: P2) -> bool {
    p.0 >= lo.0 && p.0 <= hi.0 && p.1 >= lo.1 && p.1 <= hi.1
}

/// Does the segment touch the rectangle? (Liang–Barsky clipping.)
fn segment_hits(a: P2, b: P2, lo: P2, hi: P2) -> bool {
    let (mut t0, mut t1) = (0.0_f64, 1.0_f64);
    let d = (b.0 - a.0, b.1 - a.1);
    for (p, q) in [
        (-d.0, a.0 - lo.0),
        (d.0, hi.0 - a.0),
        (-d.1, a.1 - lo.1),
        (d.1, hi.1 - a.1),
    ] {
        if p == 0.0 {
            if q < 0.0 {
                return false;
            }
        } else {
            let t = q / p;
            if p < 0.0 {
                t0 = t0.max(t);
            } else {
                t1 = t1.min(t);
            }
            if t0 > t1 {
                return false;
            }
        }
    }
    true
}

impl AppState {
    /// Starts a selection rectangle at a screen point (Select tool, nothing being dragged).
    pub fn sketch_box_begin(&mut self, px: f64, py: f64) -> bool {
        let Some(s) = &mut self.sketching else {
            return false;
        };
        if s.tool != Tool::Select || s.drag.is_some() {
            return false;
        }
        s.boxsel = Some(BoxSel {
            from: (px, py),
            to: (px, py),
        });
        true
    }

    pub fn sketch_box_move(&mut self, px: f64, py: f64) {
        if let Some(b) = self.sketching.as_mut().and_then(|s| s.boxsel.as_mut()) {
            b.to = (px, py);
            self.view_version += 1;
        }
    }

    /// Finishes the rectangle and selects what it covers (adding to the selection when
    /// `additive`). Returns how many things it selected.
    pub fn sketch_box_end(&mut self, additive: bool, w: f64, h: f64) -> usize {
        let Some(b) = self.sketching.as_mut().and_then(|s| s.boxsel.take()) else {
            return 0;
        };
        self.view_version += 1;
        // Geometry (and the origin / axes) wins; relation symbols are taken only when the
        // rectangle covers no geometry, so a box around a shape never drags its symbols along.
        let mut hits = self.box_hits(&b, w, h);
        hits.extend(self.box_datums(&b, w, h));
        // Dimensions are picked together with geometry, as in SolidWorks; relation symbols only
        // when the box covers nothing else.
        let mut relations = self.box_dimensions(&b, w, h);
        if hits.is_empty() && relations.is_empty() {
            relations = self.box_relations(&b, w, h);
        }
        let Some(s) = &mut self.sketching else {
            return 0;
        };
        if !additive {
            s.selection.clear();
            s.relations.clear();
        }
        let n = hits.len() + relations.len();
        for id in hits {
            if !s.selection.contains(&id) {
                s.selection.push(id);
            }
        }
        for id in relations {
            if !s.relations.contains(&id) {
                s.relations.push(id);
            }
        }
        n
    }

    /// The origin (when it is inside the rectangle and has no point of its own) and, for a
    /// crossing rectangle, the axes it touches. A window never takes an axis: it is endless.
    pub fn box_datums(&self, b: &BoxSel, w: f64, h: f64) -> Vec<String> {
        let (lo, hi) = (b.min(), b.max());
        let Some(o) = self.sketch_to_screen((0.0, 0.0), w, h) else {
            return Vec::new();
        };
        let mut out = Vec::new();
        if self.origin_entity().is_none() && inside(o, lo, hi) {
            out.push(SEL_ORIGIN.to_owned());
        }
        if b.crossing() {
            let corners = [(lo.0, lo.1), (hi.0, lo.1), (lo.0, hi.1), (hi.0, hi.1)];
            for (id, unit) in [(SEL_AXIS_X, (1.0, 0.0)), (SEL_AXIS_Y, (0.0, 1.0))] {
                let Some(tip) = self.sketch_to_screen(unit, w, h) else {
                    continue;
                };
                let d = (tip.0 - o.0, tip.1 - o.1);
                // Which side of the axis line each corner is on; a change of sign means it crosses.
                let side: Vec<f64> = corners
                    .iter()
                    .map(|c| d.0 * (c.1 - o.1) - d.1 * (c.0 - o.0))
                    .collect();
                let (min, max) = side
                    .iter()
                    .fold((f64::MAX, f64::MIN), |(a, b), v| (a.min(*v), b.max(*v)));
                if min <= 0.0 && max >= 0.0 {
                    out.push(id.to_owned());
                }
            }
        }
        out
    }

    /// Dimensions whose text is inside the rectangle (either direction).
    pub fn box_dimensions(&self, b: &BoxSel, w: f64, h: f64) -> Vec<String> {
        let (lo, hi) = (b.min(), b.max());
        let Some(o) = self.sketch_overlay(w, h) else {
            return Vec::new();
        };
        let mut out: Vec<String> = Vec::new();
        for d in o.dims.iter().filter(|d| !d.preview) {
            if let Some(id) = &d.id {
                if inside(d.text_at, lo, hi) && !out.contains(id) {
                    out.push(id.clone());
                }
            }
        }
        out
    }

    /// Relations whose symbol is inside the rectangle (either direction).
    pub fn box_relations(&self, b: &BoxSel, w: f64, h: f64) -> Vec<String> {
        let (lo, hi) = (b.min(), b.max());
        let mut out: Vec<String> = Vec::new();
        for g in self.relation_glyphs(w, h) {
            if inside(g.pos, lo, hi) && !out.contains(&g.constraint) {
                out.push(g.constraint);
            }
        }
        out
    }

    /// Everything that can be selected: shapes, and points no shape uses.
    pub fn sketch_select_all(&mut self) -> usize {
        let Some(sk) = self.sketch_data() else {
            return 0;
        };
        let used: BTreeSet<&String> = sk
            .entities()
            .values()
            .flat_map(|e| defining_points(e))
            .collect();
        let all: Vec<String> = sk
            .entities()
            .iter()
            .filter(|(id, e)| !matches!(e, Entity::Point { .. }) || !used.contains(id))
            .map(|(id, _)| id.clone())
            .collect();
        let n = all.len();
        if let Some(s) = &mut self.sketching {
            s.selection = all;
            s.relations.clear();
        }
        self.view_version += 1;
        n
    }

    /// Selects the chain a line or arc belongs to: everything reachable through shared end
    /// points (or points glued by a coincident relation) over lines and arcs. A circle is a
    /// chain of its own.
    pub fn sketch_select_chain(&mut self, id: &str, additive: bool) -> usize {
        self.select_joined(id, additive, false)
    }

    /// Like [`sketch_select_chain`](Self::sketch_select_chain), but only across joints where the
    /// two shapes leave in the same direction (a smooth outline: lines and arcs that are tangent).
    pub fn sketch_select_tangency(&mut self, id: &str, additive: bool) -> usize {
        self.select_joined(id, additive, true)
    }

    fn select_joined(&mut self, id: &str, additive: bool, tangent_only: bool) -> usize {
        let Some(sk) = self.sketch_data() else {
            return 0;
        };
        // Points that a coincident relation glues together count as one joint, exactly like a
        // shared point does.
        let mut joint: std::collections::BTreeMap<String, String> =
            std::collections::BTreeMap::new();
        for c in sk.constraints().values() {
            if let dc_sketch::Constraint::Coincident { a, b } = c {
                let (ra, rb) = (
                    joint.get(a).cloned().unwrap_or_else(|| a.clone()),
                    joint.get(b).cloned().unwrap_or_else(|| b.clone()),
                );
                let keep = ra.clone().min(rb.clone());
                for v in joint.values_mut() {
                    if *v == ra || *v == rb {
                        *v = keep.clone();
                    }
                }
                joint.insert(a.clone(), keep.clone());
                joint.insert(b.clone(), keep);
            }
        }
        let joint_of = |p: &String| joint.get(p).cloned().unwrap_or_else(|| p.clone());
        // (joint, the point itself) for each end of a line or arc.
        let ends = |e: &Entity| -> Vec<(String, String)> {
            let pair =
                |a: &String, b: &String| vec![(joint_of(a), a.clone()), (joint_of(b), b.clone())];
            match e {
                Entity::Line { a, b } => pair(a, b),
                Entity::Arc { start, end, .. } => pair(start, end),
                _ => Vec::new(),
            }
        };
        // Which way a shape leaves a point (up to sign), for the tangency test.
        let direction = |e: &Entity, p: &String| -> Option<P2> {
            let unit = |v: P2| {
                let l = dist((0.0, 0.0), v).max(1e-12);
                (v.0 / l, v.1 / l)
            };
            match e {
                Entity::Line { a, b } => {
                    let (pa, pb) = (sk.point(a)?, sk.point(b)?);
                    Some(unit((pb.0 - pa.0, pb.1 - pa.1)))
                }
                Entity::Arc { center, .. } => {
                    let (c, q) = (sk.point(center)?, sk.point(p)?);
                    let r = unit((q.0 - c.0, q.1 - c.1));
                    Some((-r.1, r.0))
                }
                _ => None,
            }
        };
        let Some(first) = sk.entity(id) else { return 0 };
        let mut chain: BTreeSet<String> = BTreeSet::new();
        if ends(first).is_empty() {
            chain.insert(id.to_owned()); // a circle or a point
        } else {
            let mut todo = vec![id.to_owned()];
            while let Some(n) = todo.pop() {
                if !chain.insert(n.clone()) {
                    continue;
                }
                let Some(e) = sk.entity(&n) else { continue };
                for (jn, pt) in ends(e) {
                    for (other, oe) in sk.entities() {
                        if chain.contains(other) {
                            continue;
                        }
                        let Some((_, opt)) = ends(oe).into_iter().find(|(j, _)| *j == jn) else {
                            continue;
                        };
                        if tangent_only {
                            let (d1, d2) = (direction(e, &pt), direction(oe, &opt));
                            let smooth = d1
                                .zip(d2)
                                .is_some_and(|(u, v)| (u.0 * v.1 - u.1 * v.0).abs() < 1e-6);
                            if !smooth {
                                continue;
                            }
                        }
                        todo.push(other.clone());
                    }
                }
            }
        }
        let n = chain.len();
        if let Some(s) = &mut self.sketching {
            if !additive {
                s.selection.clear();
            }
            s.relations.clear();
            for c in chain {
                if !s.selection.contains(&c) {
                    s.selection.push(c);
                }
            }
        }
        self.view_version += 1;
        n
    }

    /// What a rectangle covers: shapes, and points that no shape uses (a rectangle's corners are
    /// not selected separately, so relations like "horizontal" stay on offer).
    pub fn box_hits(&self, b: &BoxSel, w: f64, h: f64) -> Vec<String> {
        let Some(sk) = self.sketch_data() else {
            return Vec::new();
        };
        let (lo, hi) = (b.min(), b.max());
        let crossing = b.crossing();
        let scr = |p: P2| self.sketch_to_screen(p, w, h);
        let used: BTreeSet<&String> = sk
            .entities()
            .values()
            .flat_map(|e| defining_points(e))
            .collect();
        let mut out = Vec::new();
        for (id, e) in sk.entities() {
            let hit = match e {
                Entity::Point { x, y } => {
                    !used.contains(id) && scr((*x, *y)).is_some_and(|p| inside(p, lo, hi))
                }
                Entity::Line { a, b } => {
                    match (sk.point(a).and_then(scr), sk.point(b).and_then(scr)) {
                        (Some(p), Some(q)) => {
                            if crossing {
                                segment_hits(p, q, lo, hi)
                            } else {
                                inside(p, lo, hi) && inside(q, lo, hi)
                            }
                        }
                        _ => false,
                    }
                }
                Entity::Circle { center, radius } => {
                    let Some(c) = sk.point(center) else { continue };
                    let (Some(cs), Some(edge)) = (scr(c), scr((c.0 + radius, c.1))) else {
                        continue;
                    };
                    let r = dist(cs, edge);
                    if crossing {
                        let nearest = (cs.0.clamp(lo.0, hi.0), cs.1.clamp(lo.1, hi.1));
                        let far = [(lo.0, lo.1), (hi.0, lo.1), (lo.0, hi.1), (hi.0, hi.1)]
                            .into_iter()
                            .map(|q| dist(cs, q))
                            .fold(0.0, f64::max);
                        dist(cs, nearest) <= r && r <= far
                    } else {
                        inside((cs.0 - r, cs.1 - r), lo, hi) && inside((cs.0 + r, cs.1 + r), lo, hi)
                    }
                }
                Entity::Arc { center, start, end } => {
                    let (Some(c), Some(s), Some(en)) =
                        (sk.point(center), sk.point(start), sk.point(end))
                    else {
                        continue;
                    };
                    let r = dist(c, s);
                    let sweep = arc_sweep(c, s, en);
                    let a0 = angle_of(c, s);
                    let pts: Vec<P2> = (0..=32)
                        .filter_map(|i| {
                            let t = a0 + sweep * f64::from(i) / 32.0;
                            scr((c.0 + r * det::cos(t), c.1 + r * det::sin(t)))
                        })
                        .collect();
                    if crossing {
                        pts.windows(2).any(|p| segment_hits(p[0], p[1], lo, hi))
                    } else {
                        !pts.is_empty() && pts.iter().all(|p| inside(*p, lo, hi))
                    }
                }
            };
            if hit {
                out.push(id.clone());
            }
        }
        out
    }
}
