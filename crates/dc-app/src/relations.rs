//! How relations show up in a sketch: their labels, the small glyphs drawn next to the geometry
//! they constrain, picking them with the mouse, and working out which relations a refused change
//! clashed with. Logic only, no egui.

use crate::sketching::{defining_points, dist, fmt_num, Problem, SketchSession, P2};
use crate::state::AppState;
use dc_doc::CommandKind;
use dc_math::det;
use dc_sketch::{Constraint, Entity, Sketch};
use std::collections::BTreeMap;

/// Pixels between a glyph's centre and the pointer at which it counts as under it.
const GLYPH_HIT_PX: f64 = 10.0;
/// Pixels between neighbouring glyphs that share an anchor.
const GLYPH_STEP_PX: f64 = 19.0;

/// A constraint in words, with its value when it has one.
pub fn relation_label(c: &Constraint) -> String {
    use Constraint::*;
    let v = |x: f64| fmt_num(x.abs());
    match c {
        Coincident { a, b } => format!("coincident {a}, {b}"),
        Horizontal { line } => format!("horizontal {line}"),
        Vertical { line } => format!("vertical {line}"),
        Parallel { a, b } => format!("parallel {a}, {b}"),
        Perpendicular { a, b } => format!("perpendicular {a}, {b}"),
        EqualLength { a, b } => format!("equal length {a}, {b}"),
        EqualRadius { a, b } => format!("equal radius {a}, {b}"),
        Fixed { point, .. } => format!("fixed {point}"),
        PointOnLine { point, line } => format!("{point} on {line}"),
        PointOnCurve { point, curve } => format!("{point} on {curve}"),
        Midpoint { point, line } => format!("{point} at the middle of {line}"),
        Symmetric { a, b, line } => format!("{a}, {b} symmetric about {line}"),
        OnAxis { point, axis } => format!(
            "{point} on the {} axis",
            match axis {
                dc_sketch::SketchAxis::X => "horizontal",
                dc_sketch::SketchAxis::Y => "vertical",
            }
        ),
        Tangent { line, curve } => format!("tangent {line}, {curve}"),
        TangentCurves { a, b } => format!("tangent {a}, {b}"),
        SymmetricAxis { a, b, axis } => format!(
            "{a}, {b} symmetric about the {} axis",
            match axis {
                dc_sketch::SketchAxis::X => "horizontal",
                dc_sketch::SketchAxis::Y => "vertical",
            }
        ),
        Angle { a, b, value } => format!("angle {a}, {b} = {}°", v(*value)),
        Length { line, value } => format!("length {line} = {}", v(*value)),
        Distance { a, b, value } => format!("distance {a}, {b} = {}", v(*value)),
        HorizontalDistance { a, b, value } => {
            format!("horizontal distance {a}, {b} = {}", v(*value))
        }
        VerticalDistance { a, b, value } => format!("vertical distance {a}, {b} = {}", v(*value)),
        PointLineDistance { point, line, value } => {
            format!("distance {point} to {line} = {}", v(*value))
        }
        Radius { target, value } => format!("radius {target} = {}", v(*value)),
        ArcLength { arc, value } => format!("arc length {arc} = {}", v(*value)),
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum GlyphKind {
    Horizontal,
    Vertical,
    Parallel,
    Perpendicular,
    Equal,
    Coincident,
    OnLine,
    OnAxis,
    Fixed,
    Tangent,
    Midpoint,
    Symmetric,
}

/// One relation's symbol, in screen points. A relation between two lines has one glyph on each.
#[derive(Clone, Debug)]
pub struct Glyph {
    pub constraint: String,
    pub kind: GlyphKind,
    pub pos: P2,
    pub selected: bool,
    pub hover: bool,
    pub problem: Problem,
    /// How big it is drawn relative to the default (the sketch's "relation size" setting).
    pub scale: f64,
}

fn kind_of(c: &Constraint) -> Option<GlyphKind> {
    use Constraint::*;
    Some(match c {
        Horizontal { .. } => GlyphKind::Horizontal,
        Vertical { .. } => GlyphKind::Vertical,
        Parallel { .. } => GlyphKind::Parallel,
        Perpendicular { .. } => GlyphKind::Perpendicular,
        EqualLength { .. } | EqualRadius { .. } => GlyphKind::Equal,
        Coincident { .. } => GlyphKind::Coincident,
        PointOnLine { .. } | PointOnCurve { .. } => GlyphKind::OnLine,
        OnAxis { .. } => GlyphKind::OnAxis,
        Fixed { .. } => GlyphKind::Fixed,
        Tangent { .. } | TangentCurves { .. } => GlyphKind::Tangent,
        SymmetricAxis { .. } => GlyphKind::Symmetric,
        Midpoint { .. } => GlyphKind::Midpoint,
        Symmetric { .. } => GlyphKind::Symmetric,
        // Dimensions are drawn by the dimension code.
        _ => return None,
    })
}

impl AppState {
    /// Screen position beside an entity where a relation's glyph sits.
    fn glyph_anchor(&self, sk: &Sketch, id: &str, w: f64, h: f64, scale: f64) -> Option<P2> {
        let scr = |p: P2| self.sketch_to_screen(p, w, h);
        match sk.entity(id)? {
            Entity::Point { x, y } => {
                let (sx, sy) = scr((*x, *y))?;
                Some((sx + 11.0 * scale, sy - 11.0 * scale))
            }
            Entity::Line { a, b } => {
                let (p, q) = (scr(sk.point(a)?)?, scr(sk.point(b)?)?);
                let (dx, dy) = (q.0 - p.0, q.1 - p.1);
                let len = det::hypot(dx, dy).max(1e-9);
                let (nx, ny) = (-dy / len, dx / len);
                // Always on the upper side of the line, so glyphs do not flip as it turns.
                let (nx, ny) = if ny > 0.0 { (-nx, -ny) } else { (nx, ny) };
                Some((
                    (p.0 + q.0) / 2.0 + nx * 13.0 * scale,
                    (p.1 + q.1) / 2.0 + ny * 13.0 * scale,
                ))
            }
            Entity::Circle { center, radius } => {
                let c = sk.point(center)?;
                let (sx, sy) = scr(c)?;
                let (ex, _) = scr((c.0 + radius, c.1))?;
                let r = (ex - sx).abs() + 12.0 * scale;
                let k = core::f64::consts::FRAC_1_SQRT_2;
                Some((sx + r * k, sy - r * k))
            }
            Entity::Arc { center, start, end } => {
                // Outside the middle of the arc, not at one end.
                let (cp, sp, ep) = (sk.point(center)?, sk.point(start)?, sk.point(end)?);
                let r = dist(cp, sp);
                let a0 = det::atan2(sp.1 - cp.1, sp.0 - cp.0);
                let sweep = {
                    let t = core::f64::consts::TAU;
                    let d = (det::atan2(ep.1 - cp.1, ep.0 - cp.0) - a0) % t;
                    if d < 0.0 {
                        d + t
                    } else {
                        d
                    }
                };
                let mid = (
                    cp.0 + r * det::cos(a0 + sweep / 2.0),
                    cp.1 + r * det::sin(a0 + sweep / 2.0),
                );
                let (c, m) = (scr(cp)?, scr(mid)?);
                let (dx, dy) = (m.0 - c.0, m.1 - c.1);
                let len = det::hypot(dx, dy).max(1e-9);
                Some((m.0 + dx / len * 12.0 * scale, m.1 + dy / len * 12.0 * scale))
            }
        }
    }

    /// Where every relation's glyph goes right now. Empty unless the sketch shows its relations.
    pub fn relation_glyphs(&self, w: f64, h: f64) -> Vec<Glyph> {
        let (Some(s), Some(sk)) = (self.sketching.as_ref(), self.view_sketch()) else {
            return Vec::new();
        };
        if !s.show_relations {
            return Vec::new();
        }
        let redundant: Vec<&String> = self
            .authority
            .state()
            .sketch(&s.sketch)
            .map(|slot| slot.report.redundant.iter().collect())
            .unwrap_or_default();
        let mut taken: BTreeMap<(i64, i64), u32> = BTreeMap::new();
        let mut out = Vec::new();
        for (cid, c) in sk.constraints() {
            let Some(kind) = kind_of(c) else { continue };
            let on: Vec<&str> = match c {
                Constraint::Parallel { a, b }
                | Constraint::Perpendicular { a, b }
                | Constraint::EqualLength { a, b }
                | Constraint::EqualRadius { a, b } => vec![a, b],
                Constraint::Symmetric { a, b, .. }
                | Constraint::SymmetricAxis { a, b, .. }
                | Constraint::TangentCurves { a, b } => vec![a, b],
                Constraint::Horizontal { line } | Constraint::Vertical { line } => vec![line],
                Constraint::Tangent { line, .. } => vec![line],
                Constraint::Coincident { a, .. } => vec![a],
                Constraint::PointOnLine { point, .. }
                | Constraint::PointOnCurve { point, .. }
                | Constraint::OnAxis { point, .. }
                | Constraint::Fixed { point, .. }
                | Constraint::Midpoint { point, .. } => vec![point],
                _ => continue,
            };
            let problem = if s.conflict_relations.contains(cid) {
                Problem::Conflict
            } else if redundant.contains(&cid) {
                Problem::Over
            } else {
                Problem::None
            };
            for entity in on {
                let Some(mut pos) = self.glyph_anchor(sk, entity, w, h, s.glyph_scale) else {
                    continue;
                };
                // Neighbours at the same spot fan out sideways instead of overlapping.
                let step = GLYPH_STEP_PX * s.glyph_scale;
                let cell = ((pos.0 / step).round() as i64, (pos.1 / step).round() as i64);
                let n = taken.entry(cell).or_insert(0);
                pos.0 += f64::from(*n) * step;
                *n += 1;
                out.push(Glyph {
                    constraint: cid.clone(),
                    kind,
                    pos,
                    selected: s.relations.contains(cid),
                    hover: s.hover_relation.as_deref() == Some(cid.as_str()),
                    problem,
                    scale: s.glyph_scale,
                });
            }
        }
        out
    }

    /// The relation or dimension under the pointer: its symbol, or a dimension's text.
    pub fn relation_at(&self, px: f64, py: f64, w: f64, h: f64) -> Option<String> {
        self.glyph_at(px, py, w, h)
            .or_else(|| self.dimension_label_at(px, py, w, h))
    }

    /// The relation whose glyph is under the pointer, if any.
    pub fn glyph_at(&self, px: f64, py: f64, w: f64, h: f64) -> Option<String> {
        let scale = self.sketching.as_ref().map_or(1.0, |s| s.glyph_scale);
        self.relation_glyphs(w, h)
            .into_iter()
            .map(|g| (det::hypot(g.pos.0 - px, g.pos.1 - py), g.constraint))
            .filter(|(d, _)| *d <= GLYPH_HIT_PX * scale)
            .min_by(|a, b| a.0.total_cmp(&b.0))
            .map(|(_, id)| id)
    }

    /// Entities a relation acts on, including the points of its lines and curves.
    pub(crate) fn relation_entities(&self, cid: &str) -> Vec<String> {
        let Some(sk) = self.sketch_data() else {
            return Vec::new();
        };
        let Some(c) = sk.constraints().get(cid) else {
            return Vec::new();
        };
        let mut out: Vec<String> = Vec::new();
        for r in c.refs() {
            out.push(r.to_owned());
            if let Some(e) = sk.entity(r) {
                out.extend(defining_points(e).into_iter().cloned());
            }
        }
        out.sort();
        out.dedup();
        out
    }

    /// Selects a relation (and no entity); `additive` toggles it in the set instead.
    pub(crate) fn select_relation(&mut self, id: Option<String>, additive: bool) {
        if let Some(s) = &mut self.sketching {
            match (id, additive) {
                (Some(id), true) => {
                    if let Some(i) = s.relations.iter().position(|r| *r == id) {
                        s.relations.remove(i);
                    } else {
                        s.relations.push(id);
                    }
                }
                (Some(id), false) => {
                    s.selection.clear();
                    s.relations = vec![id];
                }
                (None, _) => s.relations.clear(),
            }
        }
        self.view_version += 1;
    }

    /// Deletes the selected relations as one command, if any are selected.
    pub fn delete_selected_relation(&mut self) -> bool {
        let Some(s) = &self.sketching else {
            return false;
        };
        if s.relations.is_empty() {
            return false;
        }
        let (ids, sketch) = (s.relations.clone(), s.sketch.clone());
        let ops: Vec<CommandKind> = ids
            .into_iter()
            .map(|constraint_id| CommandKind::SketchRemoveConstraint {
                sketch: sketch.clone(),
                constraint_id,
            })
            .collect();
        let ok = self
            .submit(CommandKind::Transaction {
                label: "Remove relations".into(),
                ops,
            })
            .is_ok();
        if ok {
            if let Some(s) = &mut self.sketching {
                s.relations.clear();
                s.hover_relation = None;
            }
        }
        ok
    }

    /// A refused change: remember where it clashed and say with what. Returns the message to show.
    pub(crate) fn note_conflict(
        &mut self,
        kind: &CommandKind,
        d: &dc_doc::Diagnostic,
    ) -> Option<String> {
        let sk = self.sketch_data()?;
        let mut ids: Vec<String> = Vec::new();
        collect_refs(kind, sk, &mut ids);
        // The existing constraints the solver itself blamed are named `sketch/<sketch>/<id>`.
        for e in &d.entities {
            if let Some(c) = e.rsplit('/').next().and_then(|c| sk.constraints().get(c)) {
                ids.extend(c.refs().into_iter().map(str::to_owned));
            }
        }
        let culprits = clashing_relations(sk, kind);
        for cid in &culprits {
            if let Some(c) = sk.constraints().get(cid) {
                ids.extend(c.refs().into_iter().map(str::to_owned));
            }
        }
        ids.sort();
        ids.dedup();
        let note = (!culprits.is_empty()).then(|| {
            let names: Vec<String> = culprits
                .iter()
                .filter_map(|c| sk.constraints().get(c).map(relation_label))
                .collect();
            format!(
                "That clashes with: {}. Remove or change one of them first.",
                names.join("; ")
            )
        });
        if let Some(s) = &mut self.sketching {
            s.conflict = ids;
            s.conflict_relations = culprits;
            s.conflict_note = note.clone();
        }
        note
    }
}

fn collect_refs(kind: &CommandKind, sk: &Sketch, out: &mut Vec<String>) {
    match kind {
        CommandKind::Transaction { ops, .. } => ops.iter().for_each(|o| collect_refs(o, sk, out)),
        CommandKind::SketchAddConstraint { constraint, .. } => {
            out.extend(constraint.refs().into_iter().map(str::to_owned))
        }
        CommandKind::SketchSetValue { constraint_id, .. } => {
            if let Some(c) = sk.constraints().get(constraint_id) {
                out.extend(c.refs().into_iter().map(str::to_owned));
            }
        }
        CommandKind::SketchDragPoint { point, .. } => out.push(point.clone()),
        CommandKind::SketchDrag { points, radius, .. } => {
            out.extend(points.iter().map(|t| t.point.clone()));
            out.extend(radius.iter().map(|r| r.circle.clone()));
        }
        _ => {}
    }
}

/// Applies the sketch edits in `kind` to a copy; `None` if it holds anything else or an edit fails.
fn apply_attempt(sk: &Sketch, kind: &CommandKind) -> Option<Sketch> {
    fn go(t: &mut Sketch, kind: &CommandKind) -> Option<()> {
        match kind {
            CommandKind::Transaction { ops, .. } => ops.iter().try_for_each(|o| go(t, o)),
            CommandKind::SketchAddEntity {
                entity_id, entity, ..
            } => t.add_entity(entity_id, entity.clone()).ok(),
            CommandKind::SketchAddConstraint {
                constraint_id,
                constraint,
                ..
            } => t.add_constraint(constraint_id, constraint.clone()).ok(),
            CommandKind::SketchSetValue {
                constraint_id,
                value,
                ..
            } => t.set_constraint_value(constraint_id, *value).ok(),
            CommandKind::SketchSetConstruction {
                entity_id,
                construction,
                ..
            } => t.set_construction(entity_id, *construction).ok(),
            _ => None,
        }
    }
    let mut t = sk.clone();
    go(&mut t, kind)?;
    Some(t)
}

/// The existing relations whose removal lets the refused change succeed: what it clashed with.
/// First each relation on its own (up to a cap); if no single one is enough, pairs of the
/// relations that touch the same geometry as the change.
pub(crate) fn clashing_relations(sk: &Sketch, kind: &CommandKind) -> Vec<String> {
    let Some(attempt) = apply_attempt(sk, kind) else {
        return Vec::new();
    };
    let ids: Vec<&String> = sk
        .constraints()
        .keys()
        .filter(|id| attempt.constraints().contains_key(*id))
        .collect();
    let solves_without = |drop: &[&String]| {
        let mut t = attempt.clone();
        drop.iter().all(|id| t.remove_constraint(id).is_ok()) && t.solve().applied
    };
    let singles: Vec<String> = ids
        .iter()
        .take(200)
        .filter(|id| solves_without(&[id]))
        .map(|id| (*id).clone())
        .collect();
    if !singles.is_empty() {
        return singles;
    }
    // Pairs, among the relations on the same geometry as the change (including their points).
    let mut touched: Vec<String> = Vec::new();
    collect_refs(kind, sk, &mut touched);
    let near = |c: &Constraint| {
        c.refs().iter().any(|r| {
            touched.iter().any(|t| t == r)
                || sk
                    .entity(r)
                    .is_some_and(|e| defining_points(e).iter().any(|p| touched.contains(*p)))
        })
    };
    let pool: Vec<&String> = ids
        .iter()
        .copied()
        .filter(|id| sk.constraints().get(*id).is_some_and(near))
        .take(24)
        .collect();
    let mut found: Vec<String> = Vec::new();
    for (i, a) in pool.iter().enumerate() {
        for b in &pool[i + 1..] {
            if solves_without(&[a, b]) {
                for x in [a, b] {
                    if !found.contains(x) {
                        found.push((*x).clone());
                    }
                }
            }
        }
    }
    found.sort();
    found
}

/// What the hovered relation's tooltip says.
pub(crate) fn tooltip_for(app: &AppState, s: &SketchSession) -> Option<String> {
    let id = s.hover_relation.as_deref()?;
    let c = app.sketch_data()?.constraints().get(id)?;
    let mut text = relation_label(c);
    if c.value().is_some() {
        text.push_str("  ·  drag to move, double-click to change");
    }
    if s.conflict_relations.iter().any(|x| x == id) {
        text.push_str("  (clashes with the last change)");
    } else if app
        .authority
        .state()
        .sketch(&s.sketch)
        .is_some_and(|slot| slot.report.redundant.iter().any(|x| x == id))
    {
        text.push_str("  (redundant)");
    }
    Some(text)
}
