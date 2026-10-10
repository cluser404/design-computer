//! Smart dimensioning, the way SolidWorks does it.
//!
//! Pick something (a line, a circle, a point, an axis, the origin) and a live ISO-style dimension
//! follows the cursor, already showing what it would measure. Pick a second thing and the meaning
//! changes: a line and an axis is that line's distance from the axis, two points a distance, a
//! point and a line the perpendicular distance. Where the cursor sits decides whether a distance is
//! aligned, horizontal or vertical. Nothing is committed until you click empty space to place it and
//! type the value; Esc at any point drops it.
//!
//! The resolved dimension is an ordinary constraint, so an agent can do exactly the same through the
//! command layer; the preview, the picks and where a dimension is drawn are UI state only.

use crate::sketching::{
    dist, fmt_num, fresh, segment_distance, AxisDraw, AxisMark, Overlay, Tool, P2,
};
use crate::state::AppState;
use core::f64::consts::FRAC_1_SQRT_2;
use dc_doc::CommandKind;
use dc_math::det;
use dc_sketch::{Constraint, Entity, Sketch};
use std::collections::BTreeSet;

const ORIGIN_PICK_PX: f64 = 10.0;
const AXIS_PICK_PX: f64 = 6.0;
const ARROW_LEN: f64 = 11.0;
const GAP_PX: f64 = 3.0;
/// How near (screen points) a dragged dimension must come to something to snap to it.
const SNAP_PX: f64 = 7.0;
const OVERSHOOT_PX: f64 = 6.0;
/// Within this angle of a line's normal a dimension is aligned with it; further round it is
/// horizontal or vertical, whichever the cursor is nearer.
const ALIGNED_ZONE_DEG: f64 = 25.0;

/// Something the Dimension tool can be pointed at.
#[derive(Clone, Debug, PartialEq)]
pub enum Pick {
    Entity(String),
    Origin,
    /// The horizontal axis (y = 0).
    AxisX,
    /// The vertical axis (x = 0).
    AxisY,
}

/// A point a dimension measures from or to. `Origin` becomes a fixed point when committed.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Pt {
    Id(String),
    Origin,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Orient {
    Aligned,
    Horizontal,
    Vertical,
}

/// What a dimension measures, fully worked out from the picks and the cursor.
#[derive(Clone, Debug, PartialEq)]
pub enum Spec {
    Length {
        line: String,
    },
    /// Between two points; horizontal and vertical run from `a` to `b` (ordered so they are positive).
    Between {
        a: Pt,
        b: Pt,
        orient: Orient,
    },
    /// Perpendicular distance from a point to a line.
    PointLine {
        point: Pt,
        line: String,
    },
    Round {
        target: String,
        diameter: bool,
    },
    /// The angle from line `a` to line `b` (their drawn directions), in degrees.
    Angle {
        a: String,
        b: String,
    },
    /// The length of an arc, measured along it.
    ArcLength {
        arc: String,
    },
}

impl Spec {
    /// The unit the number is typed in.
    pub fn unit(&self) -> &'static str {
        match self {
            Spec::Angle { .. } => "°",
            _ => "mm",
        }
    }

    pub fn title(&self) -> &'static str {
        match self {
            Spec::Length { .. } => "Length",
            Spec::Between {
                orient: Orient::Horizontal,
                ..
            } => "Horizontal distance",
            Spec::Between {
                orient: Orient::Vertical,
                ..
            } => "Vertical distance",
            Spec::Between { .. } => "Distance",
            Spec::PointLine { .. } => "Distance to line",
            Spec::Round { diameter: true, .. } => "Diameter",
            Spec::Round { .. } => "Radius",
            Spec::Angle { .. } => "Angle",
            Spec::ArcLength { .. } => "Arc length",
        }
    }

    fn mentions(&self, ids: &BTreeSet<String>) -> bool {
        let pt = |p: &Pt| matches!(p, Pt::Id(i) if !ids.contains(i));
        match self {
            Spec::Length { line } => !ids.contains(line),
            Spec::Between { a, b, .. } => pt(a) || pt(b),
            Spec::PointLine { point, line } => pt(point) || !ids.contains(line),
            Spec::Round { target, .. } => !ids.contains(target),
            Spec::Angle { a, b } => !ids.contains(a) || !ids.contains(b),
            Spec::ArcLength { arc } => !ids.contains(arc),
        }
    }
}

/// The value box: a placed dimension waiting for its number.
#[derive(Clone, Debug, PartialEq)]
pub struct Edit {
    pub spec: Spec,
    /// Where the dimension was placed (sketch coordinates).
    pub at: P2,
    pub text: String,
    /// The constraint being edited, when this geometry is already dimensioned.
    pub existing: Option<String>,
    pub error: Option<String>,
    /// A reference dimension: it shows the measurement and constrains nothing.
    pub driven: bool,
    /// Decimals and tolerance for how it is written.
    pub style: dc_sketch::DimStyle,
}

#[derive(Clone, Debug, Default)]
pub struct DimState {
    pub picks: Vec<Pick>,
    pub hover: Option<Pick>,
    /// The cursor in sketch coordinates (not snapped: a dimension can go anywhere).
    pub cursor: Option<P2>,
    pub edit: Option<Edit>,
}

impl DimState {
    pub fn active(&self) -> bool {
        !self.picks.is_empty() || self.edit.is_some()
    }

    pub fn picked_entity(&self, id: &str) -> bool {
        self.picks
            .iter()
            .any(|p| matches!(p, Pick::Entity(e) if e == id))
    }

    pub fn hovered_entity(&self, id: &str) -> bool {
        matches!(&self.hover, Some(Pick::Entity(e)) if e == id)
    }

    /// Drops anything that refers to entities that no longer exist.
    pub fn forget(&mut self, alive: &BTreeSet<String>) {
        let gone = |p: &Pick| matches!(p, Pick::Entity(e) if !alive.contains(e));
        self.picks.retain(|p| !gone(p));
        if self.hover.as_ref().is_some_and(gone) {
            self.hover = None;
        }
        // `mentions` is true when something is missing from `alive`.
        if self.edit.as_ref().is_some_and(|e| e.spec.mentions(alive)) {
            self.edit = None;
        }
    }
}

/// A dimension ready to paint, in screen points. Arrows are `(tip, direction the arrow points)`.
#[derive(Clone, Debug, Default)]
pub struct DimDraw {
    pub id: Option<String>,
    pub preview: bool,
    /// Extension (witness) lines.
    pub witness: Vec<[P2; 2]>,
    /// The dimension line (or leader).
    pub line: Vec<[P2; 2]>,
    pub arrows: Vec<(P2, P2)>,
    pub text: String,
    /// Centre of the text.
    pub text_at: P2,
    /// Screen rotation of the text, kept upright (within a quarter turn of horizontal).
    pub angle: f64,
    /// The text sits just above the line (linear dimensions) rather than centred on `text_at`.
    pub above: bool,
    /// Selected (orange) or under the pointer (highlighted), in the Select tool.
    pub selected: bool,
    pub hover: bool,
    /// A reference dimension (drawn in brackets, greyed): it measures, it does not constrain.
    pub driven: bool,
}

pub enum Resolved {
    /// Needs another pick; says what to do.
    Wait(String),
    Refuse(String),
    Spec(Spec),
}

enum Measured {
    Linear {
        a: P2,
        b: P2,
        orient: Orient,
    },
    Round {
        c: P2,
        r: f64,
        diameter: bool,
    },
    /// An arc, measured along its length: its centre, radius, where it starts and how far round.
    ArcLen {
        c: P2,
        r: f64,
        start: f64,
        sweep: f64,
    },
    /// Two lines' angle: where they meet, their directions, the angle from the first to the
    /// second in radians (signed), and the two segments (for extension lines).
    Angle {
        apex: P2,
        u: P2,
        v: P2,
        phi: f64,
        seg_a: (P2, P2),
        seg_b: (P2, P2),
    },
}

/// Directions a radius or diameter likes to point: level, upright and the diagonals.
fn tidy_rays() -> Vec<P2> {
    let k = core::f64::consts::FRAC_1_SQRT_2;
    let mut v = Vec::new();
    for (x, y) in [(1.0, 0.0), (k, k), (0.0, 1.0), (-k, k)] {
        v.push((x, y));
        v.push((-x, -y));
    }
    v
}

// ---- small vector helpers -----------------------------------------------------------------

fn sub(a: P2, b: P2) -> P2 {
    (a.0 - b.0, a.1 - b.1)
}
fn add(a: P2, b: P2) -> P2 {
    (a.0 + b.0, a.1 + b.1)
}
fn mul(a: P2, k: f64) -> P2 {
    (a.0 * k, a.1 * k)
}
fn dot(a: P2, b: P2) -> f64 {
    a.0 * b.0 + a.1 * b.1
}
fn cross(a: P2, b: P2) -> f64 {
    a.0 * b.1 - a.1 * b.0
}
fn len(a: P2) -> f64 {
    det::hypot(a.0, a.1)
}
fn unit(a: P2) -> P2 {
    let l = len(a);
    if l < 1e-12 {
        (1.0, 0.0)
    } else {
        (a.0 / l, a.1 / l)
    }
}
fn perp(a: P2) -> P2 {
    (-a.1, a.0)
}

/// Whether a distance between two points reads best aligned, horizontal or vertical, given where
/// the cursor is: beside the segment (near its normal) means aligned; above or below it means
/// horizontal; left or right of it means vertical.
pub fn orient_for(a: P2, b: P2, cursor: P2) -> Orient {
    let d = sub(b, a);
    let l = len(d);
    if l < 1e-9 || d.0.abs() < 1e-9 * l || d.1.abs() < 1e-9 * l {
        return Orient::Aligned; // already along an axis: aligned says it all
    }
    let mid = mul(add(a, b), 0.5);
    let v = sub(cursor, mid);
    let vl = len(v);
    if vl < 1e-9 {
        return Orient::Aligned;
    }
    let n = perp(unit(d));
    if (dot(v, n) / vl).abs() >= det::cos(ALIGNED_ZONE_DEG.to_radians()) {
        Orient::Aligned
    } else if v.1.abs() >= v.0.abs() {
        Orient::Horizontal
    } else {
        Orient::Vertical
    }
}

// ---- reading the sketch ----------------------------------------------------------------------

enum Kind {
    Point,
    Line(String, String),
    Circle(String),
    Arc(String),
}

/// An arc's start and end point ids.
fn arc_end_points(sk: &Sketch, id: &str) -> Option<(String, String)> {
    match sk.entity(id)? {
        Entity::Arc { start, end, .. } => Some((start.clone(), end.clone())),
        _ => None,
    }
}

fn kind_of(sk: &Sketch, id: &str) -> Option<Kind> {
    Some(match sk.entity(id)? {
        Entity::Point { .. } => Kind::Point,
        Entity::Line { a, b } => Kind::Line(a.clone(), b.clone()),
        Entity::Circle { center, .. } => Kind::Circle(center.clone()),
        Entity::Arc { center, .. } => Kind::Arc(center.clone()),
    })
}

/// The point an entity stands for when it is measured to or from: itself, or a circle's centre.
fn as_point(sk: &Sketch, id: &str) -> Option<Pt> {
    match kind_of(sk, id)? {
        Kind::Point => Some(Pt::Id(id.to_owned())),
        Kind::Circle(c) | Kind::Arc(c) => Some(Pt::Id(c)),
        Kind::Line(..) => None,
    }
}

type PosFn<'a> = &'a dyn Fn(&str) -> Option<P2>;

fn pt_pos(pos: PosFn, p: &Pt) -> Option<P2> {
    match p {
        Pt::Origin => Some((0.0, 0.0)),
        Pt::Id(id) => pos(id),
    }
}

fn line_ends(sk: &Sketch, pos: PosFn, line: &str) -> Option<(P2, P2)> {
    match sk.entity(line)? {
        Entity::Line { a, b } => Some((pos(a)?, pos(b)?)),
        _ => None,
    }
}

/// Signed perpendicular distance from `p` to the line through `a` and `b` (left is positive).
fn side(a: P2, b: P2, p: P2) -> f64 {
    let u = sub(b, a);
    let l = len(u);
    if l < 1e-12 {
        return 0.0;
    }
    (u.0 * (p.1 - a.1) - u.1 * (p.0 - a.0)) / l
}

fn foot(a: P2, b: P2, p: P2) -> P2 {
    let u = sub(b, a);
    let l2 = dot(u, u);
    if l2 < 1e-18 {
        return a;
    }
    add(a, mul(u, dot(sub(p, a), u) / l2))
}

fn measure(sk: &Sketch, pos: PosFn, spec: &Spec) -> Option<Measured> {
    Some(match spec {
        Spec::Length { line } => {
            let (a, b) = line_ends(sk, pos, line)?;
            Measured::Linear {
                a,
                b,
                orient: Orient::Aligned,
            }
        }
        Spec::Between { a, b, orient } => Measured::Linear {
            a: pt_pos(pos, a)?,
            b: pt_pos(pos, b)?,
            orient: *orient,
        },
        Spec::PointLine { point, line } => {
            let (la, lb) = line_ends(sk, pos, line)?;
            let p = pt_pos(pos, point)?;
            Measured::Linear {
                a: p,
                b: foot(la, lb, p),
                orient: Orient::Aligned,
            }
        }
        Spec::Angle { a, b } => {
            let (a1, a2) = line_ends(sk, pos, a)?;
            let (b1, b2) = line_ends(sk, pos, b)?;
            let (u, v) = (unit(sub(a2, a1)), unit(sub(b2, b1)));
            let cross = u.0 * v.1 - u.1 * v.0;
            if cross.abs() < 1e-9 {
                return None; // parallel lines have no apex
            }
            // Where the two lines meet.
            let d = sub(b1, a1);
            let t = (d.0 * v.1 - d.1 * v.0) / cross;
            let apex = add(a1, mul(u, t));
            Measured::Angle {
                apex,
                u,
                v,
                phi: det::atan2(cross, dot(u, v)),
                seg_a: (a1, a2),
                seg_b: (b1, b2),
            }
        }
        Spec::ArcLength { arc } => match sk.entity(arc)? {
            Entity::Arc { center, start, end } => {
                let (c, s, e) = (pos(center)?, pos(start)?, pos(end)?);
                let tau = core::f64::consts::TAU;
                let a0 = det::atan2(s.1 - c.1, s.0 - c.0);
                let mut sweep = (det::atan2(e.1 - c.1, e.0 - c.0) - a0) % tau;
                if sweep < 0.0 {
                    sweep += tau;
                }
                Measured::ArcLen {
                    c,
                    r: dist(c, s),
                    start: a0,
                    sweep,
                }
            }
            _ => return None,
        },
        Spec::Round { target, diameter } => match sk.entity(target)? {
            Entity::Circle { center, radius } => Measured::Round {
                c: pos(center)?,
                r: *radius,
                diameter: *diameter,
            },
            Entity::Arc { center, start, .. } => {
                let c = pos(center)?;
                Measured::Round {
                    c,
                    r: dist(c, pos(start)?),
                    diameter: *diameter,
                }
            }
            _ => return None,
        },
    })
}

fn magnitude(m: &Measured) -> f64 {
    match m {
        Measured::Linear { a, b, orient } => match orient {
            Orient::Aligned => dist(*a, *b),
            Orient::Horizontal => (b.0 - a.0).abs(),
            Orient::Vertical => (b.1 - a.1).abs(),
        },
        Measured::Round { r, diameter, .. } => {
            if *diameter {
                2.0 * r
            } else {
                *r
            }
        }
        Measured::Angle { phi, .. } => phi.abs().to_degrees(),
        Measured::ArcLen { r, sweep, .. } => r * sweep,
    }
}

/// The dimension a constraint expresses, if it is one.
pub fn spec_of(c: &Constraint, sk: &Sketch) -> Option<Spec> {
    let id = |s: &String| Pt::Id(s.clone());
    Some(match c {
        Constraint::Length { line, .. } => Spec::Length { line: line.clone() },
        Constraint::Distance { a, b, .. } => Spec::Between {
            a: id(a),
            b: id(b),
            orient: Orient::Aligned,
        },
        Constraint::HorizontalDistance { a, b, .. } => Spec::Between {
            a: id(a),
            b: id(b),
            orient: Orient::Horizontal,
        },
        Constraint::VerticalDistance { a, b, .. } => Spec::Between {
            a: id(a),
            b: id(b),
            orient: Orient::Vertical,
        },
        Constraint::PointLineDistance { point, line, .. } => Spec::PointLine {
            point: id(point),
            line: line.clone(),
        },
        Constraint::Radius { target, .. } => Spec::Round {
            target: target.clone(),
            diameter: matches!(sk.entity(target)?, Entity::Circle { .. }),
        },
        Constraint::Angle { a, b, .. } => Spec::Angle {
            a: a.clone(),
            b: b.clone(),
        },
        Constraint::ArcLength { arc, .. } => Spec::ArcLength { arc: arc.clone() },
        _ => return None,
    })
}

/// The number the user sees for a constraint: diameters are shown as diameters.
fn shown_value(c: &Constraint, spec: &Spec) -> Option<f64> {
    let v = c.value()?;
    Some(match spec {
        Spec::Round { diameter: true, .. } => 2.0 * v,
        _ => v,
    })
}

fn same_geometry(a: &Spec, b: &Spec, origin: Option<&str>) -> bool {
    let norm = |p: &Pt| match p {
        Pt::Origin => origin.map_or_else(|| "<origin>".to_owned(), str::to_owned),
        Pt::Id(i) => i.clone(),
    };
    match (a, b) {
        (Spec::Length { line: x }, Spec::Length { line: y }) => x == y,
        (
            Spec::Between {
                a: a1,
                b: b1,
                orient: o1,
            },
            Spec::Between {
                a: a2,
                b: b2,
                orient: o2,
            },
        ) => {
            let (p, q, r, s) = (norm(a1), norm(b1), norm(a2), norm(b2));
            o1 == o2 && ((p == r && q == s) || (p == s && q == r))
        }
        (
            Spec::PointLine {
                point: p1,
                line: l1,
            },
            Spec::PointLine {
                point: p2,
                line: l2,
            },
        ) => norm(p1) == norm(p2) && l1 == l2,
        (Spec::Round { target: x, .. }, Spec::Round { target: y, .. }) => x == y,
        (Spec::ArcLength { arc: x }, Spec::ArcLength { arc: y }) => x == y,
        (Spec::Angle { a: a1, b: b1 }, Spec::Angle { a: a2, b: b2 }) => {
            (a1 == a2 && b1 == b2) || (a1 == b2 && b1 == a2)
        }
        _ => false,
    }
}

impl AppState {
    // ---- picking ----------------------------------------------------------------------------

    /// What the Dimension tool would pick at this screen point: the origin, then an entity, then
    /// an axis.
    /// The origin, if the point is on it.
    pub(crate) fn origin_at(&self, px: f64, py: f64, w: f64, h: f64) -> bool {
        self.sketch_to_screen((0.0, 0.0), w, h)
            .is_some_and(|o| det::hypot(px - o.0, py - o.1) <= ORIGIN_PICK_PX)
    }

    /// Which axis line (if any) is under the point.
    pub(crate) fn axis_at(&self, px: f64, py: f64, w: f64, h: f64) -> Option<Pick> {
        let o = self.sketch_to_screen((0.0, 0.0), w, h)?;
        let hx = unit(sub(self.sketch_to_screen((1.0, 0.0), w, h)?, o));
        let vy = unit(sub(self.sketch_to_screen((0.0, 1.0), w, h)?, o));
        let p = sub((px, py), o);
        // Distance to the horizontal axis is measured across it, along the vertical direction.
        if dot(p, perp(hx)).abs() <= AXIS_PICK_PX {
            return Some(Pick::AxisX);
        }
        if dot(p, perp(vy)).abs() <= AXIS_PICK_PX {
            return Some(Pick::AxisY);
        }
        None
    }

    pub fn dimension_pick_at(&self, px: f64, py: f64, w: f64, h: f64) -> Option<Pick> {
        if self.origin_at(px, py, w, h) {
            return Some(Pick::Origin);
        }
        if let Some(id) = self.nearest_entity(px, py, w, h, false) {
            return Some(Pick::Entity(id));
        }
        self.axis_at(px, py, w, h)
    }

    pub(crate) fn dimension_hover(&mut self, at: Option<(f64, f64, f64, f64)>) {
        let (hover, cursor) = match at {
            Some((x, y, w, h)) => (
                self.dimension_pick_at(x, y, w, h),
                self.screen_to_sketch(x, y, w, h),
            ),
            None => (None, None),
        };
        if let Some(s) = &mut self.sketching {
            s.dim.hover = hover;
            // When the pointer leaves the viewport the preview stays where it was, as it does in
            // SolidWorks, rather than jumping somewhere meaningless.
            if s.dim.edit.is_none() && cursor.is_some() {
                s.dim.cursor = cursor;
            }
        }
    }

    /// A click with the Dimension tool: edit an existing dimension, add to what is picked, or place.
    pub(crate) fn dimension_click(&mut self, px: f64, py: f64, w: f64, h: f64) {
        if self.sketching.as_ref().is_none_or(|s| s.dim.edit.is_some()) {
            return; // the value box has the focus
        }
        let Some(cur) = self.screen_to_sketch(px, py, w, h) else {
            return;
        };
        if let Some(s) = &mut self.sketching {
            s.dim.cursor = Some(cur);
        }
        if let Some(id) = self.dimension_label_at(px, py, w, h) {
            self.edit_dimension(&id);
            return;
        }
        let picks = self
            .sketching
            .as_ref()
            .map(|s| s.dim.picks.clone())
            .unwrap_or_default();
        // An arc and one end point may be followed by the other end point: that is its length.
        let arc_and_end = match picks.as_slice() {
            [Pick::Entity(a), Pick::Entity(p)] => self
                .sketch_data()
                .is_some_and(|sk| arc_end_points(sk, a).is_some_and(|(s, e)| *p == s || *p == e)),
            _ => false,
        };
        if picks.len() < 2 || arc_and_end {
            if let Some(p) = self.dimension_pick_at(px, py, w, h) {
                if !picks.contains(&p) {
                    if let Some(s) = &mut self.sketching {
                        s.dim.picks.push(p);
                    }
                    return;
                }
            }
        }
        if picks.is_empty() {
            self.error(
                "nothing to dimension there",
                Some("click a line, circle, arc or point, or an axis or the origin".into()),
            );
            return;
        }
        match self.resolve_dimension(&picks, cur) {
            Resolved::Spec(spec) => self.open_edit(spec, cur),
            Resolved::Wait(msg) => self.info(msg),
            Resolved::Refuse(msg) => self.error(msg, None),
        }
    }

    /// Double-click with the Select tool on a dimension's text edits it.
    pub fn sketch_double_click(&mut self, px: f64, py: f64, w: f64, h: f64) -> bool {
        if let Some(id) = self.dimension_label_at(px, py, w, h) {
            return self.edit_dimension(&id);
        }
        // Double-clicking a line or arc selects the whole chain it is part of.
        let select_tool = self
            .sketching
            .as_ref()
            .is_some_and(|s| s.tool == crate::sketching::Tool::Select);
        if select_tool {
            if let Some(id) = self.nearest_entity(px, py, w, h, true) {
                return self.sketch_select_chain(&id, false) > 0;
            }
        }
        false
    }

    fn open_edit(&mut self, spec: Spec, at: P2) {
        let Some(sk) = self.sketch_data() else { return };
        let pos = |id: &str| sk.point(id);
        let measured = measure(sk, &pos, &spec)
            .map(|m| magnitude(&m))
            .unwrap_or(0.0);
        let origin = self.origin_entity();
        let existing = sk.constraints().iter().find_map(|(cid, c)| {
            let other = spec_of(c, sk)?;
            same_geometry(&spec, &other, origin.as_deref())
                .then(|| (cid.clone(), shown_value(c, &other)))
        });
        let text = fmt_num(existing.as_ref().and_then(|e| e.1).unwrap_or(measured));
        let msg = existing
            .is_some()
            .then(|| "already dimensioned: editing that dimension".to_owned());
        let (driven, style) = existing
            .as_ref()
            .map(|(cid, _)| {
                (
                    sk.is_driven(cid),
                    sk.style(cid).cloned().unwrap_or_default(),
                )
            })
            .unwrap_or_default();
        if let Some(s) = &mut self.sketching {
            s.dim.edit = Some(Edit {
                spec,
                at,
                text,
                existing: existing.map(|e| e.0),
                error: None,
                driven,
                style,
            });
        }
        if let Some(m) = msg {
            self.info(m);
        }
    }

    /// Opens the value box on a dimension that already exists.
    fn edit_dimension(&mut self, cid: &str) -> bool {
        let Some(sk) = self.sketch_data() else {
            return false;
        };
        let Some(c) = sk.constraints().get(cid) else {
            return false;
        };
        let Some(spec) = spec_of(c, sk) else {
            return false;
        };
        let driven = sk.is_driven(cid);
        let style = sk.style(cid).cloned().unwrap_or_default();
        // A reference dimension shows what the geometry measures now, not a stored number.
        let value = if driven {
            let pos = |id: &str| sk.point(id);
            measure(sk, &pos, &spec)
                .map(|m| magnitude(&m))
                .unwrap_or(0.0)
        } else {
            shown_value(c, &spec).unwrap_or(0.0)
        };
        let at = self.dim_place_of(cid).unwrap_or((0.0, 0.0));
        if let Some(s) = &mut self.sketching {
            s.dim.picks.clear();
            s.dim.edit = Some(Edit {
                spec,
                at,
                text: fmt_num(value),
                existing: Some(cid.to_owned()),
                error: None,
                driven,
                style,
            });
        }
        true
    }

    // ---- working out what a set of picks means --------------------------------------------

    /// What the dimension would measure for these picks with the cursor here.
    pub fn resolve_dimension(&self, picks: &[Pick], cursor: P2) -> Resolved {
        let Some(sk) = self.sketch_data() else {
            return Resolved::Wait(String::new());
        };
        let pos = |id: &str| sk.point(id);
        let at = |p: &Pt| pt_pos(&pos, p).unwrap_or_default();
        let between = |a: Pt, b: Pt| -> Resolved {
            let (pa, pb) = (at(&a), at(&b));
            if dist(pa, pb) < 1e-9 {
                return Resolved::Refuse("those two points are in the same place".into());
            }
            let orient = orient_for(pa, pb, cursor);
            let swap = match orient {
                Orient::Horizontal => pb.0 < pa.0,
                Orient::Vertical => pb.1 < pa.1,
                Orient::Aligned => false,
            };
            let (a, b) = if swap { (b, a) } else { (a, b) };
            Resolved::Spec(Spec::Between { a, b, orient })
        };
        let is_ref = |p: &Pick| !matches!(p, Pick::Entity(_));
        match picks {
            [] => Resolved::Wait(
                "Click a line, circle, arc or point, or an axis or the origin.".into(),
            ),
            [Pick::Entity(id)] => match kind_of(sk, id) {
                Some(Kind::Line(a, b)) => {
                    let (pa, pb) = (at(&Pt::Id(a.clone())), at(&Pt::Id(b.clone())));
                    match orient_for(pa, pb, cursor) {
                        Orient::Aligned => Resolved::Spec(Spec::Length { line: id.clone() }),
                        _ => between(Pt::Id(a), Pt::Id(b)),
                    }
                }
                Some(Kind::Circle(_)) => Resolved::Spec(Spec::Round {
                    target: id.clone(),
                    diameter: true,
                }),
                Some(Kind::Arc(_)) => Resolved::Spec(Spec::Round {
                    target: id.clone(),
                    diameter: false,
                }),
                _ => Resolved::Wait(
                    "Click a second point, a line, an axis or the origin to measure to.".into(),
                ),
            },
            [_] => Resolved::Wait("Click a point or line to measure from here.".into()),
            [p, q] => {
                let (p, q) = if is_ref(p) && !is_ref(q) {
                    (q, p)
                } else {
                    (p, q)
                };
                match (p, q) {
                    (Pick::Entity(e), Pick::AxisX | Pick::AxisY) => {
                        let along_x = matches!(q, Pick::AxisY);
                        let coord = |pt: &Pt| {
                            let v = at(pt);
                            if along_x {
                                v.0
                            } else {
                                v.1
                            }
                        };
                        // A line is measured from the end nearer where the cursor is, so where you
                        // hover (or put the dimension) chooses the end.
                        let pt = match kind_of(sk, e) {
                            Some(Kind::Line(a, b)) => {
                                let (a, b) = (Pt::Id(a), Pt::Id(b));
                                if dist(at(&a), cursor) <= dist(at(&b), cursor) {
                                    Some(a)
                                } else {
                                    Some(b)
                                }
                            }
                            _ => as_point(sk, e),
                        };
                        let Some(pt) = pt else {
                            return Resolved::Refuse("cannot measure that to an axis".into());
                        };
                        let (a, b) = if coord(&pt) >= 0.0 {
                            (Pt::Origin, pt)
                        } else {
                            (pt, Pt::Origin)
                        };
                        Resolved::Spec(Spec::Between {
                            a,
                            b,
                            orient: if along_x {
                                Orient::Horizontal
                            } else {
                                Orient::Vertical
                            },
                        })
                    }
                    (Pick::Entity(e), Pick::Origin) => match kind_of(sk, e) {
                        Some(Kind::Line(..)) => Resolved::Spec(Spec::PointLine {
                            point: Pt::Origin,
                            line: e.clone(),
                        }),
                        _ => match as_point(sk, e) {
                            Some(pt) => between(Pt::Origin, pt),
                            None => Resolved::Refuse("cannot measure that to the origin".into()),
                        },
                    },
                    (Pick::Entity(x), Pick::Entity(y))
                        if arc_end_points(sk, x).is_some_and(|(s, e)| *y == s || *y == e) =>
                    {
                        Resolved::Wait(
                            "Click the arc's other end point to dimension its length.".into(),
                        )
                    }
                    (Pick::Entity(x), Pick::Entity(y)) => match (kind_of(sk, x), kind_of(sk, y)) {
                        (Some(Kind::Line(a1, b1)), Some(Kind::Line(a2, b2))) => {
                            let u = sub(at(&Pt::Id(b1)), at(&Pt::Id(a1)));
                            let v = sub(at(&Pt::Id(b2)), at(&Pt::Id(a2.clone())));
                            let cross =
                                (u.0 * v.1 - u.1 * v.0).abs() / (len(u) * len(v)).max(1e-12);
                            if cross < 0.02 {
                                Resolved::Spec(Spec::PointLine {
                                    point: Pt::Id(a2),
                                    line: x.clone(),
                                })
                            } else {
                                Resolved::Spec(Spec::Angle {
                                    a: x.clone(),
                                    b: y.clone(),
                                })
                            }
                        }
                        (Some(Kind::Line(a, b)), _) | (_, Some(Kind::Line(a, b))) => {
                            let other = if matches!(kind_of(sk, x), Some(Kind::Line(..))) {
                                y
                            } else {
                                x
                            };
                            let line = if matches!(kind_of(sk, x), Some(Kind::Line(..))) {
                                x
                            } else {
                                y
                            };
                            match as_point(sk, other) {
                                Some(Pt::Id(p)) if p == a || p == b => Resolved::Refuse(
                                    "that point is an end of the line, so its distance is zero"
                                        .into(),
                                ),
                                Some(point) => Resolved::Spec(Spec::PointLine {
                                    point,
                                    line: line.clone(),
                                }),
                                None => Resolved::Refuse("cannot measure those".into()),
                            }
                        }
                        _ => match (as_point(sk, x), as_point(sk, y)) {
                            (Some(a), Some(b)) if a == b => {
                                Resolved::Refuse("those are the same point".into())
                            }
                            (Some(a), Some(b)) => between(a, b),
                            _ => Resolved::Refuse("cannot measure those".into()),
                        },
                    },
                    _ => Resolved::Refuse(
                        "measure from an axis or the origin to a point, line or circle".into(),
                    ),
                }
            }
            [Pick::Entity(arc), Pick::Entity(p), Pick::Entity(q)] => {
                match arc_end_points(sk, arc) {
                    Some((s, e)) if (*p == s && *q == e) || (*p == e && *q == s) => {
                        Resolved::Spec(Spec::ArcLength { arc: arc.clone() })
                    }
                    _ => Resolved::Refuse(
                        "for an arc's length pick the arc and then its two end points".into(),
                    ),
                }
            }
            _ => Resolved::Refuse("a dimension takes at most two picks".into()),
        }
    }

    /// A fixed point at the origin, if the sketch already has one.
    pub fn origin_entity(&self) -> Option<String> {
        let sk = self.sketch_data()?;
        let at_origin = |id: &str| {
            sk.constraints().values().any(|c| {
                matches!(c, Constraint::Fixed { point, x, y } if point == id && *x == 0.0 && *y == 0.0)
            })
        };
        sk.entities()
            .iter()
            .filter(|(_, e)| matches!(e, Entity::Point { .. }))
            .map(|(id, _)| id)
            .find(|id| at_origin(id))
            .cloned()
    }

    // ---- committing ---------------------------------------------------------------------------

    /// Commits the dimension in the value box (Enter). Returns whether it was accepted.
    pub fn apply_dimension(&mut self) -> bool {
        let Some(s) = &self.sketching else {
            return false;
        };
        let Some(edit) = s.dim.edit.clone() else {
            return false;
        };
        let sketch = s.sketch.clone();
        // A reference dimension just reports: whatever is in the box, it measures.
        let measured_now = self.sketch_data().and_then(|sk| {
            let pos = |id: &str| sk.point(id);
            measure(sk, &pos, &edit.spec).map(|m| magnitude(&m))
        });
        let typed = if edit.driven {
            measured_now.filter(|v| v.is_finite() && *v > 0.0)
        } else {
            edit.text
                .trim()
                .parse::<f64>()
                .ok()
                .filter(|v| v.is_finite() && *v > 0.0)
        };
        let angle = matches!(edit.spec, Spec::Angle { .. });
        let typed = typed.filter(|v| !angle || *v < 180.0);
        let Some(typed) = typed else {
            self.set_edit_error(if angle {
                "enter an angle between 0 and 180 degrees"
            } else {
                "enter a positive number in mm"
            });
            return false;
        };
        let Some(sk) = self.sketch_data() else {
            return false;
        };
        let pos = |id: &str| sk.point(id);
        let result_id;
        let outcome = match &edit.existing {
            Some(cid) => {
                // A diameter is shown as one but stored as a radius.
                let value = match &edit.spec {
                    Spec::Round { diameter: true, .. } => typed / 2.0,
                    _ => typed,
                };
                result_id = cid.clone();
                let was_driven = sk.is_driven(cid);
                let old_style = sk.style(cid).cloned().unwrap_or_default();
                let old_value = sk.constraints().get(cid).and_then(|c| c.value());
                let mut ops: Vec<CommandKind> = Vec::new();
                if was_driven && !edit.driven {
                    ops.push(CommandKind::SketchSetDriven {
                        sketch: sketch.clone(),
                        constraint_id: cid.clone(),
                        driven: false,
                    });
                }
                if !edit.driven && old_value.is_none_or(|v| (v - value).abs() > 1e-12) {
                    ops.push(CommandKind::SketchSetValue {
                        sketch: sketch.clone(),
                        constraint_id: cid.clone(),
                        value,
                        expected: dc_doc::Expected::Any,
                    });
                }
                if !was_driven && edit.driven {
                    ops.push(CommandKind::SketchSetDriven {
                        sketch: sketch.clone(),
                        constraint_id: cid.clone(),
                        driven: true,
                    });
                }
                if edit.style != old_style {
                    ops.push(CommandKind::SketchSetDimensionStyle {
                        sketch: sketch.clone(),
                        constraint_id: cid.clone(),
                        style: edit.style.clone(),
                    });
                }
                if ops.is_empty() {
                    // Nothing changed: just close the box.
                    if let Some(s) = &mut self.sketching {
                        s.dim = Default::default();
                    }
                    self.view_version += 1;
                    return true;
                }
                self.submit(CommandKind::Transaction {
                    label: "Edit dimension".into(),
                    ops,
                })
            }
            None => {
                let mut used = self.used_ids();
                let mut ops: Vec<CommandKind> = Vec::new();
                let mut origin: Option<String> = None;
                let mut resolve =
                    |p: &Pt, ops: &mut Vec<CommandKind>, used: &mut BTreeSet<String>| match p {
                        Pt::Id(i) => i.clone(),
                        Pt::Origin => origin
                            .get_or_insert_with(|| {
                                if let Some(o) = self.origin_entity() {
                                    return o;
                                }
                                let id = if used.contains("origin") {
                                    fresh(used, "origin")
                                } else {
                                    used.insert("origin".into());
                                    "origin".into()
                                };
                                ops.push(CommandKind::SketchAddEntity {
                                    sketch: sketch.clone(),
                                    entity_id: id.clone(),
                                    entity: Entity::Point { x: 0.0, y: 0.0 },
                                });
                                ops.push(CommandKind::SketchAddConstraint {
                                    sketch: sketch.clone(),
                                    constraint_id: fresh(used, "fx"),
                                    constraint: Constraint::Fixed {
                                        point: id.clone(),
                                        x: 0.0,
                                        y: 0.0,
                                    },
                                });
                                id
                            })
                            .clone(),
                    };
                let constraint = match &edit.spec {
                    Spec::Length { line } => Constraint::Length {
                        line: line.clone(),
                        value: typed,
                    },
                    Spec::Between { a, b, orient } => {
                        let (a, b) = (
                            resolve(a, &mut ops, &mut used),
                            resolve(b, &mut ops, &mut used),
                        );
                        match orient {
                            Orient::Aligned => Constraint::Distance { a, b, value: typed },
                            Orient::Horizontal => {
                                Constraint::HorizontalDistance { a, b, value: typed }
                            }
                            Orient::Vertical => Constraint::VerticalDistance { a, b, value: typed },
                        }
                    }
                    Spec::PointLine { point, line } => {
                        let sd = match (line_ends(sk, &pos, line), pt_pos(&pos, point)) {
                            (Some((a, b)), Some(p)) => side(a, b, p),
                            _ => 0.0,
                        };
                        let point = resolve(point, &mut ops, &mut used);
                        Constraint::PointLineDistance {
                            point,
                            line: line.clone(),
                            value: if sd < 0.0 { -typed } else { typed },
                        }
                    }
                    Spec::Round { target, diameter } => Constraint::Radius {
                        target: target.clone(),
                        value: if *diameter { typed / 2.0 } else { typed },
                    },
                    Spec::ArcLength { arc } => Constraint::ArcLength {
                        arc: arc.clone(),
                        value: typed,
                    },
                    Spec::Angle { a, b } => {
                        // The sign records which way round the lines are now.
                        let phi = match measure(sk, &pos, &edit.spec) {
                            Some(Measured::Angle { phi, .. }) => phi,
                            _ => 0.0,
                        };
                        Constraint::Angle {
                            a: a.clone(),
                            b: b.clone(),
                            value: if phi < 0.0 { -typed } else { typed },
                        }
                    }
                };
                let cid = fresh(&mut used, "d");
                result_id = cid.clone();
                ops.push(CommandKind::SketchAddConstraint {
                    sketch: sketch.clone(),
                    constraint_id: cid.clone(),
                    constraint,
                });
                // Where it was put is part of the design: saved, and undone with it.
                ops.push(CommandKind::SketchPlaceDimension {
                    sketch: sketch.clone(),
                    constraint_id: cid.clone(),
                    x: edit.at.0,
                    y: edit.at.1,
                });
                if edit.driven {
                    ops.push(CommandKind::SketchSetDriven {
                        sketch: sketch.clone(),
                        constraint_id: cid.clone(),
                        driven: true,
                    });
                }
                if edit.style != dc_sketch::DimStyle::default() {
                    ops.push(CommandKind::SketchSetDimensionStyle {
                        sketch: sketch.clone(),
                        constraint_id: cid,
                        style: edit.style.clone(),
                    });
                }
                self.submit(CommandKind::Transaction {
                    label: format!("Add {} dimension", edit.spec.title().to_lowercase()),
                    ops,
                })
            }
        };
        match outcome {
            Ok(_) => {
                if let Some(s) = &mut self.sketching {
                    s.dim = Default::default();
                }
                let _ = result_id;
                self.view_version += 1;
                true
            }
            Err(e) => {
                let msg = self
                    .sketching
                    .as_ref()
                    .and_then(|s| s.conflict_note.clone())
                    .unwrap_or(e.message);
                self.set_edit_error(&msg);
                false
            }
        }
    }

    fn set_edit_error(&mut self, msg: &str) {
        if let Some(e) = self.sketching.as_mut().and_then(|s| s.dim.edit.as_mut()) {
            e.error = Some(msg.to_owned());
        }
    }

    // ---- drawing --------------------------------------------------------------------------

    /// Adds every dimension (and the one being made) to the overlay, ISO style.
    pub(crate) fn dimension_overlay(&self, o: &mut Overlay, w: f64, h: f64, pos: PosFn) {
        let (Some(s), Some(sk)) = (self.sketching.as_ref(), self.sketch_data()) else {
            return;
        };
        let editing = s.dim.edit.as_ref().and_then(|e| e.existing.clone());
        for (cid, c) in sk.constraints() {
            if editing.as_deref() == Some(cid.as_str()) {
                continue;
            }
            let Some(spec) = spec_of(c, sk) else { continue };
            let Some(m) = measure(sk, pos, &spec) else {
                continue;
            };
            let place = self
                .dim_place_of(cid)
                .unwrap_or_else(|| self.default_place(&m, w, h));
            let look = Look {
                style: sk.style(cid).cloned().unwrap_or_default(),
                driven: sk.is_driven(cid),
            };
            if let Some(d) = self.draw_dimension(&m, place, w, h, Some(cid.clone()), false, &look) {
                o.dims.push(d);
            }
        }
        let in_tool = s.tool == Tool::Dimension;
        if let Some(edit) = &s.dim.edit {
            if let Some(m) = measure(sk, pos, &edit.spec) {
                let look = Look {
                    style: edit.style.clone(),
                    driven: edit.driven,
                };
                if let Some(d) = self.draw_dimension(&m, edit.at, w, h, None, true, &look) {
                    o.edit_at = Some(d.text_at);
                    o.dims.push(d);
                }
            }
        } else if in_tool {
            let cursor = s.dim.cursor;
            match self.resolve_dimension(&s.dim.picks, cursor.unwrap_or_default()) {
                Resolved::Spec(spec) => {
                    if let Some(m) = measure(sk, pos, &spec) {
                        let place = cursor.unwrap_or_else(|| self.default_place(&m, w, h));
                        if let Some(d) =
                            self.draw_dimension(&m, place, w, h, None, true, &Look::default())
                        {
                            o.dims.push(d);
                        }
                        o.dim_prompt = Some(format!(
                            "{}: click an axis, the origin or another entity to change what is measured, or click to place it.",
                            spec.title()
                        ));
                    }
                }
                Resolved::Wait(msg) | Resolved::Refuse(msg) => o.dim_prompt = Some(msg),
            }
        }
        if in_tool || s.dim.edit.is_some() {
            let mark = |p: &Pick| match p {
                Pick::AxisX => Some(AxisMark::X),
                Pick::AxisY => Some(AxisMark::Y),
                Pick::Origin => Some(AxisMark::Origin),
                Pick::Entity(_) => None,
            };
            let screen_axis = |m: AxisMark, picked: bool| self.axis_draw(m, picked, w, h);
            for p in &s.dim.picks {
                if let Some(a) = mark(p).and_then(|m| screen_axis(m, true)) {
                    o.axes.push(a);
                }
            }
            if let Some(a) = s
                .dim
                .hover
                .as_ref()
                .and_then(mark)
                .and_then(|m| screen_axis(m, false))
            {
                o.axes.push(a);
            }
        }
    }

    /// An axis (long segment across the view) or the origin, in screen points, for highlighting.
    pub(crate) fn axis_draw(&self, m: AxisMark, picked: bool, w: f64, h: f64) -> Option<AxisDraw> {
        let origin = self.sketch_to_screen((0.0, 0.0), w, h)?;
        let reach = 4.0 * (w + h);
        let along =
            |d: P2| -> (P2, P2) { (sub(origin, mul(d, reach)), add(origin, mul(d, reach))) };
        let (a, b) = match m {
            AxisMark::X => along(unit(sub(self.sketch_to_screen((1.0, 0.0), w, h)?, origin))),
            AxisMark::Y => along(unit(sub(self.sketch_to_screen((0.0, 1.0), w, h)?, origin))),
            AxisMark::Origin => (origin, origin),
        };
        Some(AxisDraw {
            mark: m,
            picked,
            a,
            b,
        })
    }

    /// Screen points per millimetre.
    fn px_per_mm(&self, w: f64, h: f64) -> f64 {
        match (
            self.sketch_to_screen((0.0, 0.0), w, h),
            self.sketch_to_screen((1.0, 0.0), w, h),
        ) {
            (Some(a), Some(b)) if dist(a, b) > 1e-9 => dist(a, b),
            _ => 1.0,
        }
    }

    /// The middle of everything in the sketch: dimensions go outward from it, off the shape.
    fn sketch_centre(&self) -> P2 {
        let Some(sk) = self.sketch_data() else {
            return (0.0, 0.0);
        };
        let pts: Vec<P2> = sk.entities().keys().filter_map(|id| sk.point(id)).collect();
        if pts.is_empty() {
            return (0.0, 0.0);
        }
        let (lo, hi) = pts.iter().fold(
            ((f64::MAX, f64::MAX), (f64::MIN, f64::MIN)),
            |(lo, hi), p| {
                (
                    (lo.0.min(p.0), lo.1.min(p.1)),
                    (hi.0.max(p.0), hi.1.max(p.1)),
                )
            },
        );
        mul(add(lo, hi), 0.5)
    }

    /// Where a dimension that was never placed goes: a little off the measured feature, on the
    /// side away from the rest of the sketch so it does not sit on top of the shape.
    fn default_place(&self, m: &Measured, w: f64, h: f64) -> P2 {
        let off = 34.0 / self.px_per_mm(w, h);
        let centre = self.sketch_centre();
        // +1 or -1: which way along `axis` points away from the sketch's centre, seen from `from`.
        let away = |from: P2, axis: P2| -> f64 {
            if dot(sub(from, centre), axis) < -1e-9 {
                -1.0
            } else {
                1.0
            }
        };
        match m {
            Measured::Linear { a, b, orient } => {
                let mid = mul(add(*a, *b), 0.5);
                match orient {
                    Orient::Horizontal => {
                        let s = away(mid, (0.0, 1.0));
                        let edge = if s > 0.0 { a.1.max(b.1) } else { a.1.min(b.1) };
                        (mid.0, edge + s * off)
                    }
                    Orient::Vertical => {
                        let s = away(mid, (1.0, 0.0));
                        let edge = if s > 0.0 { a.0.max(b.0) } else { a.0.min(b.0) };
                        (edge + s * off, mid.1)
                    }
                    Orient::Aligned => {
                        let n = perp(unit(sub(*b, *a)));
                        add(mid, mul(n, away(mid, n) * off))
                    }
                }
            }
            Measured::ArcLen { c, r, start, sweep } => {
                let mid = start + sweep / 2.0;
                add(
                    *c,
                    mul(
                        (det::cos(mid), det::sin(mid)),
                        r + 34.0 / self.px_per_mm(w, h),
                    ),
                )
            }
            Measured::Angle {
                apex,
                u,
                v,
                seg_a,
                seg_b,
                ..
            } => {
                // Along the bisector, out to roughly half the shorter line.
                let bis = {
                    let s = add(*u, *v);
                    if len(s) < 1e-6 {
                        perp(*u)
                    } else {
                        unit(s)
                    }
                };
                let reach = 0.5 * dist(seg_a.0, seg_a.1).min(dist(seg_b.0, seg_b.1));
                let r = reach.max(34.0 / self.px_per_mm(w, h));
                add(*apex, mul(bis, r))
            }
            Measured::Round { c, r, .. } => {
                let d = sub(*c, centre);
                let dir = if len(d) < 1e-9 {
                    (FRAC_1_SQRT_2, FRAC_1_SQRT_2)
                } else {
                    unit(d)
                };
                add(*c, mul(dir, *r + off * 0.6))
            }
        }
    }

    /// An ISO-style dimension in screen points: extension lines with a small gap at the feature and
    /// an overshoot past the dimension line, filled arrowheads, text above the line.
    #[allow(clippy::too_many_arguments)]
    fn draw_dimension(
        &self,
        m: &Measured,
        place: P2,
        w: f64,
        h: f64,
        id: Option<String>,
        preview: bool,
        look: &Look,
    ) -> Option<DimDraw> {
        let scr = |p: P2| self.sketch_to_screen(p, w, h);
        let origin = scr((0.0, 0.0))?;
        let c = scr(place)?;
        let value = magnitude(m);
        match m {
            Measured::Linear { a, b, orient } => {
                let (a_s, b_s) = (scr(*a)?, scr(*b)?);
                let dir = match orient {
                    Orient::Aligned => unit(sub(b_s, a_s)),
                    Orient::Horizontal => unit(sub(scr((1.0, 0.0))?, origin)),
                    Orient::Vertical => unit(sub(scr((0.0, 1.0))?, origin)),
                };
                // Measure along `dir`, offset along its normal to where the cursor put the line.
                let n = perp(dir);
                let nc = dot(c, n);
                let (ta, tb) = (dot(a_s, dir), dot(b_s, dir));
                let at = |t: f64| add(mul(dir, t), mul(n, nc));
                let (p1, p2) = (at(ta), at(tb));
                let mut d = DimDraw {
                    id,
                    preview,
                    text: look.text("", value, ""),
                    driven: look.driven,
                    ..DimDraw::default()
                };
                for (feature, on_line) in [(a_s, p1), (b_s, p2)] {
                    let e = sub(on_line, feature);
                    if len(e) > 1.0 {
                        let eu = unit(e);
                        d.witness.push([
                            add(feature, mul(eu, GAP_PX)),
                            add(on_line, mul(eu, OVERSHOOT_PX)),
                        ]);
                    }
                }
                let span = dist(p1, p2);
                let u = unit(sub(p2, p1));
                if span >= 2.0 * ARROW_LEN + 6.0 {
                    d.line.push([p1, p2]);
                    d.arrows.push((p1, mul(u, -1.0)));
                    d.arrows.push((p2, u));
                } else {
                    // Too short for arrows inside: put them outside, pointing in.
                    let tail = ARROW_LEN + 8.0;
                    d.line.push([add(p1, mul(u, -tail)), p1]);
                    d.line.push([p2, add(p2, mul(u, tail))]);
                    d.arrows.push((p1, u));
                    d.arrows.push((p2, mul(u, -1.0)));
                }
                d.text_at = mul(add(p1, p2), 0.5);
                d.angle = upright(det::atan2(u.1, u.0));
                d.above = true;
                Some(d)
            }
            Measured::ArcLen {
                c: centre,
                r,
                start,
                sweep,
            } => {
                // An arc drawn alongside the arc, `place` away from the centre, with the arrows
                // at both ends and radial extension lines when it is outside.
                let reach = dist(*centre, place).max(*r);
                let at_angle = |a: f64, rad: f64| {
                    scr((centre.0 + rad * det::cos(a), centre.1 + rad * det::sin(a)))
                };
                let mut d = DimDraw {
                    id,
                    preview,
                    text: look.text("arc ", value, ""),
                    driven: look.driven,
                    ..DimDraw::default()
                };
                let n = 32;
                for i in 0..n {
                    let (t0, t1) = (f64::from(i) / f64::from(n), f64::from(i + 1) / f64::from(n));
                    d.line.push([
                        at_angle(start + sweep * t0, reach)?,
                        at_angle(start + sweep * t1, reach)?,
                    ]);
                }
                let tip = |t: f64, towards: f64| -> Option<(P2, P2)> {
                    let (p, q) = (
                        at_angle(start + sweep * t, reach)?,
                        at_angle(start + sweep * (t + towards), reach)?,
                    );
                    Some((p, unit(sub(p, q))))
                };
                // Pointing outwards along the arc: away from the neighbouring sample.
                if let Some(a) = tip(0.0, 0.02) {
                    d.arrows.push(a);
                }
                if let Some(a) = tip(1.0, -0.02) {
                    d.arrows.push(a);
                }
                if reach > *r + 1e-9 {
                    for a in [*start, start + sweep] {
                        let (from, to) = (at_angle(a, *r)?, at_angle(a, reach)?);
                        let e = unit(sub(to, from));
                        d.witness
                            .push([add(from, mul(e, GAP_PX)), add(to, mul(e, OVERSHOOT_PX))]);
                    }
                }
                let mid = start + sweep / 2.0;
                let (m, outer) = (at_angle(mid, reach)?, at_angle(mid, reach + 1.0)?);
                d.text_at = add(m, mul(unit(sub(outer, m)), 16.0));
                d.angle = 0.0;
                Some(d)
            }
            Measured::Angle {
                apex,
                u,
                v,
                phi,
                seg_a,
                seg_b,
            } => {
                let apex_s = scr(*apex)?;
                let dir_s = |d: P2| -> Option<P2> { Some(unit(sub(scr(add(*apex, d))?, apex_s))) };
                let (us, vs) = (dir_s(*u)?, dir_s(*v)?);
                let radius = len(sub(c, apex_s)).max(26.0);
                let (a0, a1) = (det::atan2(us.1, us.0), det::atan2(vs.1, vs.0));
                // The way round that is the angle itself (screen y points down, which flips it).
                let tau = core::f64::consts::TAU;
                let mut sweep = a1 - a0;
                while sweep > core::f64::consts::PI {
                    sweep -= tau;
                }
                while sweep <= -core::f64::consts::PI {
                    sweep += tau;
                }
                let _ = phi;
                let on_arc = |t: f64| {
                    let ang = a0 + sweep * t;
                    add(apex_s, mul((det::cos(ang), det::sin(ang)), radius))
                };
                let mut d = DimDraw {
                    id,
                    preview,
                    text: look.text("", value, "°"),
                    driven: look.driven,
                    ..DimDraw::default()
                };
                let n = 32;
                for i in 0..n {
                    d.line.push([
                        on_arc(f64::from(i) / f64::from(n)),
                        on_arc(f64::from(i + 1) / f64::from(n)),
                    ]);
                }
                // Arrowheads at both ends, pointing along the arc outwards.
                let sgn = if sweep >= 0.0 { 1.0 } else { -1.0 };
                let tangent = |ang: f64| (-det::sin(ang) * sgn, det::cos(ang) * sgn);
                d.arrows.push((on_arc(0.0), mul(tangent(a0), -1.0)));
                d.arrows.push((on_arc(1.0), tangent(a0 + sweep)));
                // Extension lines where a line stops short of the arc.
                for (dir, (p, q)) in [(us, *seg_a), (vs, *seg_b)] {
                    let tip = add(apex_s, mul(dir, radius));
                    let (ps, qs) = (scr(p)?, scr(q)?);
                    let seg = sub(qs, ps);
                    let t = dot(sub(tip, ps), seg) / dot(seg, seg).max(1e-9);
                    if !(-0.001..=1.001).contains(&t) {
                        let near = if dist(ps, tip) < dist(qs, tip) {
                            ps
                        } else {
                            qs
                        };
                        let e = sub(tip, near);
                        if len(e) > 1.0 {
                            let eu = unit(e);
                            d.witness.push([
                                add(near, mul(eu, GAP_PX)),
                                add(tip, mul(eu, OVERSHOOT_PX)),
                            ]);
                        }
                    }
                }
                let mid = a0 + sweep / 2.0;
                d.text_at = add(apex_s, mul((det::cos(mid), det::sin(mid)), radius + 16.0));
                d.angle = 0.0;
                Some(d)
            }
            Measured::Round {
                c: centre,
                r,
                diameter,
            } => {
                let cs = scr(*centre)?;
                let r_s = dist(cs, scr((centre.0 + r, centre.1))?);
                let away = sub(c, cs);
                let dir = if len(away) < 1.0 {
                    (FRAC_1_SQRT_2, -FRAC_1_SQRT_2)
                } else {
                    unit(away)
                };
                let far = add(cs, mul(dir, r_s));
                let outside = len(away) > r_s;
                let end = if outside { c } else { far };
                let mut d = DimDraw {
                    id,
                    preview,
                    text: look.text(if *diameter { "Ø" } else { "R" }, value, ""),
                    driven: look.driven,
                    ..DimDraw::default()
                };
                if *diameter {
                    let near = sub(cs, mul(dir, r_s));
                    d.line.push([near, end]);
                    d.arrows.push((near, mul(dir, -1.0)));
                } else {
                    d.line.push([cs, end]);
                }
                d.arrows.push((far, dir));
                d.text_at = add(end, mul(dir, 16.0));
                d.angle = 0.0;
                Some(d)
            }
        }
    }

    /// The id of the dimension whose text is under this screen point.
    pub fn dimension_label_at(&self, px: f64, py: f64, w: f64, h: f64) -> Option<String> {
        let o = self.sketch_overlay(w, h)?;
        // Extension lines start right at the geometry they measure, so they only count where
        // there is no geometry to grab instead.
        let geometry_here = self.nearest_entity(px, py, w, h, false).is_some();
        o.dims
            .iter()
            .filter(|d| !d.preview)
            .filter_map(|d| {
                let half_w = 4.5 * d.text.chars().count() as f64 + 8.0;
                let on_text =
                    (px - d.text_at.0).abs() <= half_w && (py - d.text_at.1).abs() <= 12.0;
                // The dimension line and its arrows: grab it anywhere along it.
                let on_line = d
                    .line
                    .iter()
                    .any(|[a, b]| segment_distance((px, py), *a, *b) <= 4.0);
                let on_extension = !geometry_here
                    && d.witness
                        .iter()
                        .any(|[a, b]| segment_distance((px, py), *a, *b) <= 4.0);
                (on_text || on_line || on_extension)
                    .then(|| d.id.clone())
                    .flatten()
            })
            .next()
    }

    /// Where a dimension is drawn: where it is being dragged to, else where it was placed.
    pub fn dim_place_of(&self, cid: &str) -> Option<P2> {
        self.sketching
            .as_ref()
            .and_then(|s| s.dim_places.get(cid).copied())
            .or_else(|| self.view_sketch().and_then(|sk| sk.placement(cid)))
    }

    /// Starts dragging the dimension under the pointer (its text or its line). Select tool, or
    /// the Dimension tool while nothing is picked.
    pub fn dimension_drag_begin(&mut self, px: f64, py: f64, w: f64, h: f64) -> bool {
        let ok = self.sketching.as_ref().is_some_and(|s| {
            s.dim.edit.is_none()
                && (s.tool == Tool::Select || (s.tool == Tool::Dimension && s.dim.picks.is_empty()))
        });
        if !ok {
            return false;
        }
        let Some(cid) = self.dimension_label_at(px, py, w, h) else {
            return false;
        };
        let from = self.dim_place_of(&cid);
        if let Some(s) = &mut self.sketching {
            s.dim_guides.clear();
            s.dim_drag = Some(DimDrag {
                id: cid.clone(),
                from,
            });
            s.relations = vec![cid];
            s.selection.clear();
        }
        true
    }

    /// The pointer moved while dragging a dimension: it follows (nothing is logged yet).
    pub fn dimension_drag_move(&mut self, px: f64, py: f64, w: f64, h: f64) {
        self.dimension_drag_move_with(px, py, w, h, false);
    }

    /// As [`dimension_drag_move`](Self::dimension_drag_move); `free` (Ctrl held) switches
    /// snapping off for this move.
    pub fn dimension_drag_move_with(&mut self, px: f64, py: f64, w: f64, h: f64, free: bool) {
        let Some(raw) = self.screen_to_sketch(px, py, w, h) else {
            return;
        };
        let Some(id) = self
            .sketching
            .as_ref()
            .and_then(|s| s.dim_drag.as_ref())
            .map(|d| d.id.clone())
        else {
            return;
        };
        let (at, guides) = self.snap_dimension_place(&id, raw, w, h, free);
        if let Some(s) = &mut self.sketching {
            s.dim_places.insert(id, at);
            s.dim_guides = guides;
            self.view_version += 1;
        }
    }

    /// Where a dragged dimension lands: on the grid if that is on, centred on what it measures,
    /// or level with another dimension of the same kind so they stack neatly. Returns the place
    /// and the guide lines to draw (sketch coordinates).
    fn snap_dimension_place(
        &self,
        cid: &str,
        raw: P2,
        w: f64,
        h: f64,
        free: bool,
    ) -> (P2, Vec<[P2; 2]>) {
        let mut p = raw;
        let mut guides: Vec<[P2; 2]> = Vec::new();
        if free {
            return (p, guides);
        }
        let grid = self.sketching.as_ref().is_some_and(|s| s.snap_grid);
        if grid {
            let step = crate::sketching::snap_step(self.camera.distance);
            p = (
                (p.0 / step).round() * step + 0.0,
                (p.1 / step).round() * step + 0.0,
            );
        }
        let Some(sk) = self.view_sketch() else {
            return (p, guides);
        };
        let pos = |id: &str| sk.point(id);
        let tol = SNAP_PX / self.px_per_mm(w, h);
        // The unit direction along a linear dimension and across it.
        let axes = |a: P2, b: P2, orient: Orient| match orient {
            Orient::Horizontal => ((1.0, 0.0), (0.0, 1.0)),
            Orient::Vertical => ((0.0, 1.0), (1.0, 0.0)),
            Orient::Aligned => {
                let along = unit(sub(b, a));
                (along, perp(along))
            }
        };
        struct Lin {
            mid: P2,
            along: P2,
            normal: P2,
            orient: Orient,
        }
        let measured = |id: &str| -> Option<Lin> {
            let spec = spec_of(sk.constraints().get(id)?, sk)?;
            match measure(sk, &pos, &spec)? {
                Measured::Linear { a, b, orient } => {
                    let (along, normal) = axes(a, b, orient);
                    Some(Lin {
                        mid: mul(add(a, b), 0.5),
                        along,
                        normal,
                        orient,
                    })
                }
                _ => None,
            }
        };
        let Some(Lin {
            mid,
            along,
            normal,
            orient,
        }) = measured(cid)
        else {
            // Round and angular dimensions: the text lands on a ray from the centre (or the
            // apex) at a tidy direction, and on the middle of an angle or an arc.
            let spec = sk.constraints().get(cid).and_then(|c| spec_of(c, sk));
            let Some(m) = spec.and_then(|spec| measure(sk, &pos, &spec)) else {
                return (p, guides);
            };
            let (origin, mut rays): (P2, Vec<P2>) = match m {
                Measured::Round { c, .. } => (c, tidy_rays()),
                Measured::ArcLen {
                    c, start, sweep, ..
                } => {
                    let mid = start + sweep / 2.0;
                    (c, vec![(det::cos(mid), det::sin(mid))])
                }
                Measured::Angle { apex, u, v, .. } => {
                    let bis = unit(add(u, v));
                    let mut r = vec![bis, mul(bis, -1.0)];
                    if bis == (0.0, 0.0) {
                        r.clear();
                    }
                    (apex, r)
                }
                Measured::Linear { .. } => return (p, guides),
            };
            let rel = sub(p, origin);
            let reach = det::hypot(rel.0, rel.1);
            if reach > tol {
                rays.sort_by(|x, y| {
                    cross(*x, rel)
                        .abs()
                        .partial_cmp(&cross(*y, rel).abs())
                        .unwrap_or(core::cmp::Ordering::Equal)
                });
                if let Some(r) = rays
                    .first()
                    .filter(|r| cross(**r, rel).abs() < tol && dot(**r, rel) > 0.0)
                {
                    p = add(origin, mul(*r, reach));
                    guides.push([origin, p]);
                }
            }
            return (p, guides);
        };
        // Centred on what it measures.
        let s = dot(sub(p, mid), along);
        if s.abs() < tol {
            p = sub(p, mul(along, s));
            guides.push([mid, p]);
        }
        // Level with another dimension of the same kind.
        let nc = dot(p, normal);
        let mut best: Option<(f64, P2, f64)> = None; // (distance, its place, its coordinate)
        for oid in sk.constraints().keys() {
            if oid == cid {
                continue;
            }
            let Some(Lin {
                normal: onormal,
                orient: oorient,
                ..
            }) = measured(oid)
            else {
                continue;
            };
            let Some(op) = self.dim_place_of(oid) else {
                continue;
            };
            // Same kind, and lying the same way (aligned ones only if parallel).
            let flip = if dot(onormal, normal) < 0.0 {
                -1.0
            } else {
                1.0
            };
            if oorient != orient || dot(onormal, normal).abs() < 0.9995 {
                continue;
            }
            let onc = flip * dot(op, onormal);
            let gap = (nc - onc).abs();
            if gap < tol && best.is_none_or(|b| gap < b.0) {
                best = Some((gap, op, onc));
            }
        }
        if let Some((_, op, onc)) = best {
            p = add(p, mul(normal, onc - nc));
            guides.push([p, op]);
        }
        (p, guides)
    }

    /// Letting go puts the dimension there: one command, so one undo step.
    pub fn dimension_drag_end(&mut self) -> bool {
        let Some(s) = &mut self.sketching else {
            return false;
        };
        let (Some(d), sketch) = (s.dim_drag.take(), s.sketch.clone()) else {
            return false;
        };
        let to = s.dim_places.remove(&d.id);
        s.dim_guides.clear();
        self.view_version += 1;
        match to {
            Some(p) if Some(p) != d.from => self
                .submit(CommandKind::SketchPlaceDimension {
                    sketch,
                    constraint_id: d.id,
                    x: p.0,
                    y: p.1,
                })
                .is_ok(),
            _ => false,
        }
    }

    #[doc(hidden)]
    pub fn _segment_distance_for_tests(p: P2, a: P2, b: P2) -> f64 {
        segment_distance(p, a, b)
    }
}

/// How a dimension's text is written: the digits, a tolerance, brackets for a reference.
#[derive(Clone, Debug, Default)]
pub struct Look {
    pub style: dc_sketch::DimStyle,
    pub driven: bool,
}

impl Look {
    fn number(&self, v: f64) -> String {
        match self.style.decimals {
            Some(d) => format!("{:.*}", usize::from(d), v),
            None => fmt_dim(v),
        }
    }

    /// The text for a value with a prefix and suffix (`Ø`, `°`), the tolerance after it, in
    /// brackets when the dimension is a reference.
    fn text(&self, prefix: &str, v: f64, suffix: &str) -> String {
        let (p, m) = (self.style.tol_plus, self.style.tol_minus);
        let tol = if p == 0.0 && m == 0.0 {
            String::new()
        } else if (p - m).abs() < 1e-12 {
            format!(" ±{}", self.number(p))
        } else {
            format!(" +{} -{}", self.number(p), self.number(m))
        };
        let t = format!("{prefix}{}{suffix}{tol}", self.number(v));
        if self.driven {
            format!("({t})")
        } else {
            t
        }
    }
}

/// A dimension being dragged to a new place.
#[derive(Clone, Debug)]
pub struct DimDrag {
    pub id: String,
    /// Where it was drawn when the drag began.
    pub from: Option<P2>,
}

/// A dimension as drawn: two decimals at most, like a drawing; the value box keeps full precision.
fn fmt_dim(v: f64) -> String {
    let s = format!("{v:.2}");
    s.trim_end_matches('0').trim_end_matches('.').to_owned()
}

/// Keeps text readable: rotated into (-90°, 90°].
fn upright(mut a: f64) -> f64 {
    let half = core::f64::consts::FRAC_PI_2;
    let pi = core::f64::consts::PI;
    if a > half {
        a -= pi;
    } else if a <= -half {
        a += pi;
    }
    a
}
