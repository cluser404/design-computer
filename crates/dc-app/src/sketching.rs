//! Interactive sketching on top of [`AppState`]: tools, snapping, previews, relations and the
//! commands they turn into. Nothing here touches egui, so all of it can be tested without a window.
//!
//! Rules this follows (see AGENTS.md §4): every committed action is an ordinary command (one
//! sentence, one undo step), and the in-progress shape (first corner, rubber band, cursor, a point
//! being dragged) is ephemeral UI state that is never logged.

use crate::dimensioning::{DimDraw, DimState};
use crate::state::{AppState, Selected};
use dc_doc::{CommandKind, PlaneFrame};
use dc_math::{det, Vec3};
use dc_render::{raster::grid_step, Camera, ProjectionKind};
use dc_sketch::{Constraint, Entity, PlaneSpec};
use std::collections::{BTreeMap, BTreeSet};

/// Screen distances (points) for snapping and picking.
const SNAP_POINT_PX: f64 = 10.0;
const SNAP_EDGE_PX: f64 = 7.0;
const GUIDE_PX: f64 = 6.0;
const PICK_PX: f64 = 8.0;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Tool {
    Select,
    Line,
    Rectangle,
    CenterRectangle,
    Circle,
    Arc,
    Arc3,
    Dimension,
    Trim,
    Extend,
    Split,
    Centerline,
    ThreePointRect,
    ThreePointCentreRect,
    Parallelogram,
    Polygon,
    Slot,
    TangentArc,
    Point,
}

impl Tool {
    pub const ALL: [Tool; 19] = [
        Tool::Select,
        Tool::Line,
        Tool::Centerline,
        Tool::Rectangle,
        Tool::CenterRectangle,
        Tool::ThreePointRect,
        Tool::ThreePointCentreRect,
        Tool::Parallelogram,
        Tool::Polygon,
        Tool::Slot,
        Tool::Circle,
        Tool::Arc,
        Tool::Arc3,
        Tool::TangentArc,
        Tool::Point,
        Tool::Dimension,
        Tool::Trim,
        Tool::Extend,
        Tool::Split,
    ];

    /// The tools that make new geometry, in the order the Draw menu lists them.
    pub const DRAW: [Tool; 14] = [
        Tool::Line,
        Tool::Centerline,
        Tool::Rectangle,
        Tool::CenterRectangle,
        Tool::ThreePointRect,
        Tool::ThreePointCentreRect,
        Tool::Parallelogram,
        Tool::Polygon,
        Tool::Slot,
        Tool::Circle,
        Tool::Arc,
        Tool::Arc3,
        Tool::TangentArc,
        Tool::Point,
    ];

    pub fn name(self) -> &'static str {
        match self {
            Tool::Select => "Select",
            Tool::Line => "Line",
            Tool::Rectangle => "Corner rectangle",
            Tool::CenterRectangle => "Centre rectangle",
            Tool::Circle => "Circle",
            Tool::Arc => "Centre arc",
            Tool::Arc3 => "3-point arc",
            Tool::Dimension => "Dimension",
            Tool::Trim => "Trim",
            Tool::Extend => "Extend",
            Tool::Split => "Split",
            Tool::Centerline => "Centerline",
            Tool::ThreePointRect => "3-point corner rectangle",
            Tool::ThreePointCentreRect => "3-point centre rectangle",
            Tool::Parallelogram => "Parallelogram",
            Tool::Polygon => "Polygon",
            Tool::Slot => "Straight slot",
            Tool::TangentArc => "Tangent arc",
            Tool::Point => "Point",
        }
    }

    /// One line of how to use the tool, for tooltips.
    pub fn hint(self) -> &'static str {
        match self {
            Tool::Select => "Click to select (Shift-click adds), drag a point to move it, Delete removes",
            Tool::Line => "Click the start, then each next point; click the last point again to stop",
            Tool::Rectangle => "Click one corner, then the opposite corner",
            Tool::CenterRectangle => "Click the centre, then a corner; builds construction diagonals",
            Tool::Circle => "Click the centre, then a point on the circle",
            Tool::Arc => "Click the centre, the start, then the end; move the cursor the way the arc should sweep",
            Tool::Arc3 => "Click the start, the end, then a point the arc passes through",
            Tool::Dimension => "Click a line, circle or arc, or two points, then type the value",
            Tool::Trim => "Click the part of a line, arc or circle to remove (shown in red); drag across several to trim them all at once",
            Tool::Extend => "Click near the end of a line or arc to lengthen it to the next shape it meets",
            Tool::Split => "Click a line or arc to cut it in two at that point",
            Tool::Centerline => "Like Line, but the lines are construction geometry (dashed, not part of the profile)",
            Tool::ThreePointRect => "Click two corners (one side, at any angle), then a third point for the width",
            Tool::ThreePointCentreRect => "Click the centre, the middle of one side, then a corner distance for the other half-side",
            Tool::Parallelogram => "Click three corners; the fourth completes it",
            Tool::Polygon => "Click the centre, then a vertex (or the middle of a side if circumscribed); the sides are set in the toolbar",
            Tool::Slot => "Click the two end centres, then a point for the width",
            Tool::TangentArc => "Click the end point of a line or arc to continue from, then where the tangent arc should end",
            Tool::Point => "Click to place a point",
        }
    }

    pub fn key(self) -> &'static str {
        match self {
            Tool::Select => "S",
            Tool::Line => "L",
            Tool::Rectangle => "R",
            Tool::CenterRectangle => "E",
            Tool::Circle => "C",
            Tool::Arc => "A",
            Tool::Arc3 => "T",
            Tool::Dimension => "D",
            Tool::Trim => "W",
            Tool::Extend => "N",
            Tool::Split => "P",
            _ => "",
        }
    }

    /// Tools that take clicks to build geometry (as opposed to select / dimension).
    pub fn draws(self) -> bool {
        !matches!(
            self,
            Tool::Select | Tool::Dimension | Tool::Trim | Tool::Extend | Tool::Split
        )
    }
}

/// A point in the middle of being placed.
#[derive(Clone, Debug, PartialEq)]
pub struct Anchor {
    pub pos: (f64, f64),
    /// An existing sketch point it snapped onto (reused rather than duplicated).
    pub point: Option<String>,
    /// A line it snapped onto (a new point there gets a point-on-line relation).
    pub on_line: Option<String>,
    /// A line whose middle it snapped to (a new point there gets a midpoint relation).
    pub midpoint_of: Option<String>,
    /// A circle or arc it snapped onto at a quadrant (point-on-curve relation).
    pub on_curve: Option<String>,
    /// It snapped onto the origin and there is no origin point yet: the new point is fixed there.
    pub origin: bool,
    /// It snapped onto one of the sketch's axes (the new point gets an on-axis relation).
    pub axis: Option<dc_sketch::SketchAxis>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Guide {
    Horizontal,
    Vertical,
}

/// Where the cursor lands after snapping.
#[derive(Clone, Debug, PartialEq)]
pub struct Snap {
    pub pos: (f64, f64),
    pub point: Option<String>,
    pub on_line: Option<String>,
    pub guide: Option<Guide>,
    pub origin: bool,
    pub axis: Option<dc_sketch::SketchAxis>,
    pub midpoint: Option<String>,
    pub on_curve: Option<String>,
    pub infer: Option<crate::inference::Infer>,
}

impl Snap {
    fn anchor(&self) -> Anchor {
        Anchor {
            pos: self.pos,
            point: self.point.clone(),
            on_line: self.on_line.clone(),
            midpoint_of: self.midpoint.clone(),
            on_curve: self.on_curve.clone(),
            origin: self.origin,
            axis: self.axis,
        }
    }
}

/// What is being dragged and how it moves.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DragKind {
    /// A point follows the cursor (offset by where it was grabbed).
    Point,
    /// A line or arc moves with the cursor: all its points shift by the same amount.
    Shape,
    /// A circle's rim follows the cursor: the radius becomes the distance to the centre.
    Radius,
}

/// How often the (heavier) check that a drag keeps the features valid may run.
const FEATURE_CHECK_EVERY: std::time::Duration = std::time::Duration::from_millis(100);

/// A sketch entity being dragged: what the cursor wants, and what the solver says everything
/// would look like there. Nothing is logged until release.
#[derive(Clone, Debug)]
pub struct Drag {
    /// The entity that was grabbed.
    pub point: String,
    pub kind: DragKind,
    /// Where it was grabbed (the point's own position for `Point`, the cursor otherwise).
    pub start: (f64, f64),
    /// Where the cursor wants it now: the point's target, or the cursor's snapped position.
    pub target: (f64, f64),
    /// Points that move together, with where they started (`Shape`), and the grab offset (`Point`).
    pub anchors: Vec<(String, P2)>,
    pub offset: P2,
    /// The circle and its centre (`Radius`) and its radius when grabbed.
    pub circle: Option<(String, P2, f64)>,
    /// What the engine is asked to hold, now and at the start.
    pub holds: DragHolds,
    pub initial: DragHolds,
    pub preview: BTreeMap<String, (f64, f64)>,
    /// The whole sketch as it would be after the drag: points, circle radii, everything the
    /// solver moved. The overlay draws this instead of the committed sketch while dragging.
    pub sketch: Option<dc_sketch::Sketch>,
    /// Why the committed sketch would break a feature built from it, if it would.
    pub broken: Option<String>,
    /// Several selected things move together (the grabbed one is among them).
    pub multi: bool,
    /// Why it cannot follow the cursor (fully defined, or free only along one direction).
    pub stuck: Option<String>,
    check_pending: bool,
    last_check: Option<std::time::Instant>,
}

/// The points and radius a drag holds at given places.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct DragHolds {
    pub points: Vec<dc_sketch::DragTarget>,
    pub radius: Option<dc_sketch::DragRadius>,
}

impl DragHolds {
    fn command(&self, sketch: &str) -> CommandKind {
        match (self.points.as_slice(), &self.radius) {
            ([t], None) => CommandKind::SketchDragPoint {
                sketch: sketch.to_owned(),
                point: t.point.clone(),
                x: t.x,
                y: t.y,
            },
            _ => CommandKind::SketchDrag {
                sketch: sketch.to_owned(),
                points: self.points.clone(),
                radius: self.radius.clone(),
            },
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Relation {
    Horizontal,
    Vertical,
    Parallel,
    Perpendicular,
    Equal,
    Coincident,
    PointOnLine,
    Tangent,
    Midpoint,
    Symmetric,
    Concentric,
    Collinear,
    OnCurve,
    Merge,
    Fix,
}

impl Problem {
    fn max_problem(self, other: Problem) -> Problem {
        let rank = |p: Problem| match p {
            Problem::None => 0,
            Problem::Over => 1,
            Problem::Conflict => 2,
        };
        if rank(other) > rank(self) {
            other
        } else {
            self
        }
    }
}

impl Relation {
    pub const ALL: [Relation; 15] = [
        Relation::Horizontal,
        Relation::Vertical,
        Relation::Parallel,
        Relation::Perpendicular,
        Relation::Equal,
        Relation::Coincident,
        Relation::PointOnLine,
        Relation::Tangent,
        Relation::Midpoint,
        Relation::Symmetric,
        Relation::Concentric,
        Relation::Collinear,
        Relation::OnCurve,
        Relation::Merge,
        Relation::Fix,
    ];

    pub fn name(self) -> &'static str {
        match self {
            Relation::Horizontal => "Horizontal",
            Relation::Vertical => "Vertical",
            Relation::Parallel => "Parallel",
            Relation::Perpendicular => "Perpendicular",
            Relation::Equal => "Equal",
            Relation::Coincident => "Coincident",
            Relation::PointOnLine => "On line",
            Relation::Tangent => "Tangent",
            Relation::Midpoint => "Midpoint",
            Relation::Symmetric => "Symmetric",
            Relation::Concentric => "Concentric",
            Relation::Collinear => "Collinear",
            Relation::OnCurve => "On curve",
            Relation::Merge => "Merge",
            Relation::Fix => "Fix",
        }
    }
}

#[derive(Clone, Debug)]
pub struct SketchSession {
    pub sketch: String,
    pub frame: PlaneFrame,
    pub tool: Tool,
    /// Click 1 of the shape in progress, and click 2 for arcs.
    pub first: Option<Anchor>,
    pub second: Option<Anchor>,
    /// Which way a centre arc sweeps, decided by the first movement after its start point.
    pub arc_ccw: Option<bool>,
    pub cursor: Option<Snap>,
    /// The dimension being built: what is picked, the live preview and the value box.
    pub dim: DimState,
    /// Where a dimension is being dragged to (sketch coordinates) while the button is down. Where
    /// dimensions stay is in the sketch itself (`Sketch::placement`).
    pub dim_places: BTreeMap<String, P2>,
    /// A dimension being dragged by hand.
    pub dim_drag: Option<crate::dimensioning::DimDrag>,
    /// Guide lines shown while a dimension snaps to something (sketch coordinates).
    pub dim_guides: Vec<[P2; 2]>,
    /// Centerlines are drawn infinite (they have no ends, only a place).
    pub centerline_infinite: bool,
    /// Line / Centerline: the first click is the middle, the second one end, the other end mirrors.
    pub midpoint_line: bool,
    pub selection: Vec<String>,
    /// New lines, circles and arcs are construction geometry.
    pub construction: bool,
    pub snap_grid: bool,
    /// The other snaps (points, edges, guides, midpoints, quadrants, inference).
    pub snaps: crate::inference::SnapOptions,
    /// Sides of the polygon tool, and whether the click is a corner or the middle of a side.
    pub polygon_sides: u32,
    pub polygon_circumscribed: bool,
    pub drag: Option<Drag>,
    /// What the Select tool's pointer is over: an entity id or one of the `SEL_*` datums.
    pub hover: Option<String>,
    /// Screen position of that pointer, for the tooltip.
    pub hover_at: Option<P2>,
    /// Draw the directions every free point can still move in, not only for the selection.
    pub show_freedom: bool,
    /// Entities named by the last change that was refused for contradicting the constraints.
    pub conflict: Vec<String>,
    /// Existing relations the last refused change clashed with, and a sentence saying so.
    pub conflict_relations: Vec<String>,
    pub conflict_note: Option<String>,
    /// A relation picked by its glyph (instead of an entity), and the one under the pointer.
    pub relations: Vec<String>,
    pub hover_relation: Option<String>,
    /// Draw a small symbol for every relation.
    pub show_relations: bool,
    /// How big those symbols are drawn (1 = normal).
    pub glyph_scale: f64,
    /// Where the line chain in progress has been (for taking a segment back with Ctrl+Z).
    pub chain: Vec<Anchor>,
    /// What the last "Check sketch" found (shown on the sketch until something changes).
    pub check: Option<Vec<crate::tools::CheckProblem>>,
    /// A modify operation whose result is being previewed (the Modify panel is open on it).
    pub modify_preview: Option<crate::modify::ModifyOp>,
    /// A shape is being drawn by pressing, dragging and releasing (SolidWorks' other way to draw
    /// a line, rectangle or circle) rather than by two clicks.
    pub drag_drawing: bool,
    /// What the Trim / Extend / Split tool would do under the pointer.
    pub edit: Option<crate::editing::EditHover>,
    /// How the Trim tool works, what it has picked so far, and whether construction lines cut.
    pub trim_mode: crate::editing::TrimMode,
    pub corner_pick: Option<(String, P2)>,
    pub trim_bounds: Vec<String>,
    pub cut_construction: bool,
    /// A power-trim stroke in progress.
    pub stroke: Option<crate::editing::TrimStroke>,
    /// A selection rectangle being dragged out.
    pub boxsel: Option<crate::selecting::BoxSel>,
    restore: Camera,
    restore_auto_fit: bool,
}

fn v3(a: [f64; 3]) -> Vec3 {
    Vec3::new(a[0], a[1], a[2])
}

impl SketchSession {
    fn world(&self, (x, y): (f64, f64)) -> Vec3 {
        v3(self.frame.origin) + v3(self.frame.x_axis) * x + v3(self.frame.y_axis) * y
    }

    fn local(&self, p: Vec3) -> (f64, f64) {
        let d = p - v3(self.frame.origin);
        (d.dot(v3(self.frame.x_axis)), d.dot(v3(self.frame.y_axis)))
    }

    pub(crate) fn clear_progress_public(&mut self) {
        self.clear_progress();
        self.chain.clear();
    }

    fn clear_progress(&mut self) {
        self.chain.clear();
        self.first = None;
        self.second = None;
        self.arc_ccw = None;
        self.dim = DimState::default();
    }
}

/// How fine the snap grid is at a given zoom: a tenth of the visible grid spacing.
pub fn snap_step(camera_distance: f64) -> f64 {
    grid_step(camera_distance) / 10.0
}

pub(crate) fn fmt_num(v: f64) -> String {
    let s = format!("{v:.4}");
    s.trim_end_matches('0').trim_end_matches('.').to_owned()
}

pub(crate) fn fresh(used: &mut BTreeSet<String>, prefix: &str) -> String {
    let id = (1u32..)
        .map(|n| format!("{prefix}{n}"))
        .find(|id| !used.contains(id))
        .expect("an unused id exists");
    used.insert(id.clone());
    id
}

pub type P2 = (f64, f64);

pub(crate) fn dist(a: P2, b: P2) -> f64 {
    det::hypot(a.0 - b.0, a.1 - b.1)
}

/// The circle through three points, if they are not (nearly) in a line.
pub fn circumcircle(a: P2, b: P2, c: P2) -> Option<(P2, f64)> {
    let d = 2.0 * (a.0 * (b.1 - c.1) + b.0 * (c.1 - a.1) + c.0 * (a.1 - b.1));
    if d.abs() < 1e-9 {
        return None;
    }
    let (a2, b2, c2) = (
        a.0 * a.0 + a.1 * a.1,
        b.0 * b.0 + b.1 * b.1,
        c.0 * c.0 + c.1 * c.1,
    );
    let ux = (a2 * (b.1 - c.1) + b2 * (c.1 - a.1) + c2 * (a.1 - b.1)) / d;
    let uy = (a2 * (c.0 - b.0) + b2 * (a.0 - c.0) + c2 * (b.0 - a.0)) / d;
    let centre = (ux, uy);
    Some((centre, dist(centre, a)))
}

fn angle(c: P2, p: P2) -> f64 {
    det::atan2(p.1 - c.1, p.0 - c.0)
}

/// Angle of `p` seen from `c`.
pub(crate) fn angle_of(c: P2, p: P2) -> f64 {
    angle(c, p)
}

/// Counter-clockwise sweep of an arc about `c` from `start` to `end`.
pub(crate) fn arc_sweep(c: P2, start: P2, end: P2) -> f64 {
    ccw_from(angle(c, start), angle(c, end))
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

pub(crate) struct Batch {
    pub(crate) sketch: String,
    pub(crate) used: BTreeSet<String>,
    pub(crate) ops: Vec<CommandKind>,
    /// New lines, circles and arcs become construction geometry.
    pub(crate) construction: bool,
    /// New lines are infinite construction lines (the Centerline tool's "Infinite" option).
    pub(crate) infinite: bool,
}

impl Batch {
    pub(crate) fn push_entity(&mut self, prefix: &str, entity: Entity) -> String {
        let id = fresh(&mut self.used, prefix);
        self.ops.push(CommandKind::SketchAddEntity {
            sketch: self.sketch.clone(),
            entity_id: id.clone(),
            entity,
        });
        id
    }

    pub(crate) fn mark_construction(&mut self, id: &str) {
        self.ops.push(CommandKind::SketchSetConstruction {
            sketch: self.sketch.clone(),
            entity_id: id.to_owned(),
            construction: true,
        });
    }

    /// A line, circle or arc that follows the "construction" setting.
    pub(crate) fn shape(&mut self, prefix: &str, entity: Entity) -> String {
        let is_line = matches!(entity, Entity::Line { .. });
        let id = self.push_entity(prefix, entity);
        if self.construction {
            self.mark_construction(&id);
            if self.infinite && is_line {
                self.ops.push(CommandKind::SketchSetInfinite {
                    sketch: self.sketch.clone(),
                    entity_id: id.clone(),
                    infinite: true,
                });
            }
        }
        id
    }

    /// A point at the anchor, or the existing point it snapped to. A new point that landed on a
    /// line is related to it.
    pub(crate) fn point(&mut self, a: &Anchor) -> String {
        if let Some(p) = &a.point {
            return p.clone();
        }
        let id = self.push_entity(
            "p",
            Entity::Point {
                x: a.pos.0,
                y: a.pos.1,
            },
        );
        if let Some(line) = &a.on_line {
            self.constraint(
                "ol",
                Constraint::PointOnLine {
                    point: id.clone(),
                    line: line.clone(),
                },
            );
        }
        if let Some(line) = &a.midpoint_of {
            self.constraint(
                "mp",
                Constraint::Midpoint {
                    point: id.clone(),
                    line: line.clone(),
                },
            );
        }
        if a.origin {
            self.constraint(
                "fx",
                Constraint::Fixed {
                    point: id.clone(),
                    x: 0.0,
                    y: 0.0,
                },
            );
        }
        if let Some(axis) = a.axis {
            self.constraint(
                "ax",
                Constraint::OnAxis {
                    point: id.clone(),
                    axis,
                },
            );
        }
        if let Some(curve) = &a.on_curve {
            self.constraint(
                "oc",
                Constraint::PointOnCurve {
                    point: id.clone(),
                    curve: curve.clone(),
                },
            );
        }
        id
    }

    pub(crate) fn corner(&mut self, pos: P2) -> String {
        self.point(&Anchor {
            pos,
            point: None,
            on_line: None,
            midpoint_of: None,
            on_curve: None,
            origin: false,
            axis: None,
        })
    }

    pub(crate) fn line(&mut self, a: &str, b: &str) -> String {
        self.shape(
            "l",
            Entity::Line {
                a: a.into(),
                b: b.into(),
            },
        )
    }

    pub(crate) fn constraint(&mut self, prefix: &str, c: Constraint) {
        let id = fresh(&mut self.used, prefix);
        self.ops.push(CommandKind::SketchAddConstraint {
            sketch: self.sketch.clone(),
            constraint_id: id,
            constraint: c,
        });
    }
}

/// Everything the UI needs to draw the active sketch, in screen points.
#[derive(Clone, Debug, Default)]
pub struct Overlay {
    pub strokes: Vec<Stroke>,
    pub points: Vec<Dot>,
    pub preview: Vec<Vec<P2>>,
    pub marker: Option<(P2, Marker)>,
    pub labels: Vec<(P2, String)>,
    /// A short note next to the cursor about what a click would do ("coincident", "on edge").
    pub hint: Option<String>,
    /// Dimensions, committed and being made, drawn ISO style.
    pub dims: Vec<DimDraw>,
    /// What the Dimension tool says to do next.
    pub dim_prompt: Option<String>,
    /// Where the value box goes (screen), when one is open.
    pub edit_at: Option<P2>,
    /// Axes and origin to highlight while dimensioning: (axis x?, axis y?, origin?) as (hover/picked).
    pub axes: Vec<AxisDraw>,
    /// Directions the free points can still move in.
    pub freedom: Vec<FreedomMark>,
    /// What the pointer is over and how free it is (Select tool).
    pub tooltip: Option<(P2, String)>,
    /// Number of constraints that add nothing (the sketch is over defined).
    pub redundant: usize,
    /// The last change was refused because it contradicted the constraints.
    pub conflict: bool,
    /// The selection rectangle being dragged out: (corner, corner, crossing).
    pub select_box: Option<(P2, P2, bool)>,
    /// Why the thing being dragged cannot follow the cursor (it is held by relations), if so.
    pub drag_note: Option<String>,
    /// Dashed guides while a dimension being dragged snaps to something.
    pub dim_guides: Vec<[P2; 2]>,
    /// What the Modify panel's operation would make (screen polylines).
    pub modify_preview: Vec<Vec<P2>>,
    /// Midpoint handles shown on hovered or selected lines (screen position, pointed at).
    pub mid_handles: Vec<(P2, bool)>,
    /// Set while dragging something that would make a feature built from the sketch invalid.
    pub drag_warning: Option<String>,
    /// What Trim would remove (red), what Extend would add (green) and where Split would cut.
    pub edit_removed: Vec<Vec<P2>>,
    pub edit_added: Vec<Vec<P2>>,
    pub edit_markers: Vec<P2>,
    /// The power-trim stroke's trail.
    pub trim_trail: Vec<P2>,
    /// A symbol for each relation, next to what it constrains.
    pub glyphs: Vec<crate::relations::Glyph>,
    /// The picked relation, in words (Select tool).
    pub relation: Option<String>,
    pub dof: usize,
    /// Only construction geometry is still free: the real shapes are defined.
    pub shapes_defined: bool,
    pub name: String,
}

/// An axis or the origin to highlight, as a screen segment (a point when it is the origin).
#[derive(Clone, Debug)]
pub struct AxisDraw {
    pub mark: AxisMark,
    /// Already part of the dimension (as opposed to just under the cursor).
    pub picked: bool,
    pub a: P2,
    pub b: P2,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AxisMark {
    X,
    Y,
    Origin,
}

#[derive(Clone, Debug)]
pub struct Stroke {
    pub pts: Vec<P2>,
    pub selected: bool,
    /// The cursor is over it in the Dimension tool.
    pub hover: bool,
    /// Fully determined by the constraints (drawn dark); otherwise it can still move (blue).
    pub defined: bool,
    pub problem: Problem,
    pub construction: bool,
}

#[derive(Clone, Debug)]
pub struct Dot {
    pub pos: P2,
    pub selected: bool,
    pub hover: bool,
    pub defined: bool,
    pub problem: Problem,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Marker {
    Free,
    Point,
    OnEdge,
    Origin,
    Horizontal,
    Vertical,
    Midpoint,
    Quadrant,
    Inferred,
}

/// The points an entity is defined by.
pub(crate) fn defining_points(e: &Entity) -> Vec<&String> {
    match e {
        Entity::Point { .. } => vec![],
        Entity::Line { a, b } => vec![a, b],
        Entity::Circle { center, .. } => vec![center],
        Entity::Arc { center, start, end } => vec![center, start, end],
    }
}

/// The two ends of an infinite line on the screen: far past any window, along the line.
fn far_ends(p: P2, q: P2) -> (P2, P2) {
    let (dx, dy) = (q.0 - p.0, q.1 - p.1);
    let len = det::hypot(dx, dy);
    if len < 1e-9 {
        return (p, q);
    }
    let k = 20_000.0 / len;
    ((p.0 - dx * k, p.1 - dy * k), (q.0 + dx * k, q.1 + dy * k))
}

/// Selection ids for the things every sketch has but no one drew: the origin and the two axes.
pub const SEL_ORIGIN: &str = "@origin";
/// A line's middle before it is a real point: `@mid:<line>`.
pub const MID_PREFIX: &str = "@mid:";
const MID_PICK_PX: f64 = 8.0;
/// A selected line shows its middle when the pointer is this close to it.
const MID_REVEAL_PX: f64 = 28.0;
pub const SEL_AXIS_X: &str = "@x";
pub const SEL_AXIS_Y: &str = "@y";

/// Is this selection id the origin or an axis rather than a sketch entity?
pub fn is_datum(id: &str) -> bool {
    id.starts_with('@')
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Sort {
    Point,
    Line,
    Circle,
    Arc,
    Origin,
    AxisX,
    AxisY,
}

impl Sort {
    fn is_datum(self) -> bool {
        matches!(self, Sort::Origin | Sort::AxisX | Sort::AxisY)
    }
}

/// What is wrong with an entity's constraints, if anything.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Problem {
    #[default]
    None,
    /// A constraint on it adds nothing new (the sketch is over defined).
    Over,
    /// A constraint on it contradicts the others: the last change was refused.
    Conflict,
}

/// Where and how a point (or a circle's radius) can still move, in screen space.
#[derive(Clone, Debug)]
pub struct FreedomMark {
    pub pos: P2,
    /// 1 = along `dir`, 2 = any way.
    pub dof: u8,
    /// Unit screen vector of the motion when `dof` is 1.
    pub dir: Option<P2>,
    /// This is a radius (a circle's or an arc's), drawn as arrows along it, not a point.
    pub radius: bool,
    /// This is an arc's sweep: arrows along the arc, as it can still open or close.
    pub sweep: bool,
    /// The thing is hovered or selected, so its marks are drawn stronger.
    pub emphasis: bool,
}

impl AppState {
    // ---- entering and leaving -------------------------------------------------------------

    /// Creates a new sketch on `plane` and starts editing it. Returns its name.
    pub fn begin_sketch(&mut self, plane: PlaneSpec) -> Option<String> {
        self.end_sketch();
        let name = (1u32..)
            .map(|n| format!("sketch{n}"))
            .find(|n| self.authority.state().sketch(n).is_none())
            .expect("a free name exists");
        self.submit(CommandKind::SketchCreate {
            sketch: name.clone(),
            plane,
        })
        .ok()?;
        self.edit_sketch(&name).then_some(name)
    }

    /// Starts editing an existing sketch: the view turns to look straight at its plane.
    pub fn edit_sketch(&mut self, name: &str) -> bool {
        self.end_sketch();
        let Some(frame) = self
            .sketch_rows()
            .into_iter()
            .find(|r| r.id == name)
            .and_then(|r| r.info.frame)
        else {
            self.error(
                format!("sketch '{name}' has no resolvable plane"),
                Some("its face may no longer exist".into()),
            );
            return false;
        };
        let restore = self.camera;
        let restore_auto_fit = self.auto_fit;
        self.camera = self.camera_facing(&frame);
        self.auto_fit = false;
        self.sketching = Some(SketchSession {
            sketch: name.to_owned(),
            frame,
            tool: Tool::Select,
            first: None,
            second: None,
            arc_ccw: None,
            cursor: None,
            dim: DimState::default(),
            dim_places: BTreeMap::new(),
            dim_drag: None,
            dim_guides: Vec::new(),
            midpoint_line: false,
            centerline_infinite: false,
            selection: Vec::new(),
            construction: false,
            snap_grid: false,
            snaps: Default::default(),
            polygon_sides: 6,
            polygon_circumscribed: false,
            drag: None,
            hover: None,
            hover_at: None,
            show_freedom: true,
            conflict: Vec::new(),
            conflict_relations: Vec::new(),
            conflict_note: None,
            relations: Vec::new(),
            hover_relation: None,
            show_relations: true,
            glyph_scale: 1.0,
            modify_preview: None,
            chain: Vec::new(),
            check: None,
            drag_drawing: false,
            edit: None,
            trim_mode: Default::default(),
            corner_pick: None,
            trim_bounds: Vec::new(),
            cut_construction: false,
            stroke: None,
            boxsel: None,
            restore,
            restore_auto_fit,
        });
        self.selected = Selected::Sketch(name.to_owned());
        self.hovered = None;
        self.view_version += 1;
        true
    }

    /// Leaves sketch mode and puts the camera back.
    pub fn end_sketch(&mut self) {
        if let Some(s) = self.sketching.take() {
            self.camera = s.restore;
            self.auto_fit = s.restore_auto_fit;
            self.selected = Selected::Sketch(s.sketch);
            self.view_version += 1;
        }
    }

    /// Called after every document change: undoing the sketch's creation ends the session, and
    /// anything selected that no longer exists is forgotten.
    pub(crate) fn drop_session_if_gone(&mut self) {
        let gone = self
            .sketching
            .as_ref()
            .is_some_and(|s| self.authority.state().sketch(&s.sketch).is_none());
        if gone {
            self.end_sketch();
            self.selected = Selected::None;
            return;
        }
        let alive: Option<BTreeSet<String>> = self.sketch_data().map(|sk| {
            sk.entities()
                .keys()
                .chain(sk.constraints().keys())
                .cloned()
                .collect()
        });
        if let (Some(alive), Some(s)) = (alive, self.sketching.as_mut()) {
            s.selection.retain(|id| is_datum(id) || alive.contains(id));
            s.conflict.retain(|id| alive.contains(id));
            s.conflict_relations.retain(|id| alive.contains(id));
            s.relations.retain(|r| alive.contains(r));
            if s.hover_relation
                .as_ref()
                .is_some_and(|r| !alive.contains(r))
            {
                s.hover_relation = None;
            }
            s.dim.forget(&alive);
            s.dim_places.retain(|id, _| alive.contains(id));
        }
    }

    /// A camera looking straight down onto `frame`, orthographic, centred on the part.
    pub(crate) fn camera_facing(&self, frame: &PlaneFrame) -> Camera {
        let n = v3(frame.normal);
        let x = v3(frame.x_axis);
        let (yaw, pitch) = if n.z.abs() > 0.999 {
            (
                det::atan2(-x.x, x.y),
                if n.z > 0.0 { 1.0 } else { -1.0 } * core::f64::consts::FRAC_PI_2,
            )
        } else {
            (det::atan2(n.y, n.x), det::asin(n.z.clamp(-1.0, 1.0)))
        };
        let (centre, extent) = match self.scene.body_bounds() {
            Some((lo, hi)) => ((lo + hi) * 0.5, (hi - lo).length()),
            None => (v3(frame.origin), 0.0),
        };
        let o = v3(frame.origin);
        let target = centre - n * (centre - o).dot(n);
        Camera {
            target,
            // With a part in the scene, frame the part; the wide default is for an empty sketch.
            distance: if extent > 0.0 {
                (extent * 2.4).max(20.0)
            } else {
                300.0
            },
            yaw,
            pitch,
            projection: ProjectionKind::Orthographic,
            ..self.camera
        }
    }

    // ---- tools ----------------------------------------------------------------------------

    pub fn set_tool(&mut self, tool: Tool) {
        if tool == Tool::Dimension {
            self.materialize_selected_midpoints();
        }
        if let Some(s) = &mut self.sketching {
            // Things already selected become the first picks of a Smart Dimension, as in
            // SolidWorks: select first, then ask for the dimension (or the other way round).
            let picks: Vec<crate::dimensioning::Pick> = if tool == Tool::Dimension {
                s.selection
                    .iter()
                    .take(2)
                    .map(|id| match id.as_str() {
                        SEL_ORIGIN => crate::dimensioning::Pick::Origin,
                        SEL_AXIS_X => crate::dimensioning::Pick::AxisX,
                        SEL_AXIS_Y => crate::dimensioning::Pick::AxisY,
                        _ => crate::dimensioning::Pick::Entity(id.clone()),
                    })
                    .collect()
            } else {
                Vec::new()
            };
            s.tool = tool;
            s.clear_progress();
            s.dim.picks = picks;
            s.drag = None;
            s.edit = None;
            s.stroke = None;
            s.corner_pick = None;
            s.trim_bounds.clear();
            if tool != Tool::Select {
                s.selection.clear();
            }
            self.view_version += 1;
        }
    }

    /// New shapes are drawn as construction geometry (dashed, not part of any profile).
    pub fn set_construction_mode(&mut self, on: bool) {
        if let Some(s) = &mut self.sketching {
            s.construction = on;
        }
    }

    /// Escape / right-click: closes the dimension box, else drops the shape in progress, else
    /// returns to the Select tool, else clears the selection. Returns whether anything happened.
    pub fn sketch_cancel(&mut self) -> bool {
        let Some(s) = &mut self.sketching else {
            return false;
        };
        // Dropping a dimension drag puts it back where it was.
        s.dim_guides.clear();
        s.drag_drawing = false;
        let dropped = s.dim_drag.take().map(|d| s.dim_places.remove(&d.id));
        let acted = if dropped.is_some()
            || s.drag.take().is_some()
            || s.boxsel.take().is_some()
            || s.stroke.take().is_some()
        {
            true
        } else if s.dim.active() || s.first.is_some() || s.second.is_some() {
            s.clear_progress();
            true
        } else if s.tool != Tool::Select {
            s.tool = Tool::Select;
            true
        } else {
            let had = !s.selection.is_empty() || !s.relations.is_empty();
            s.selection.clear();
            s.relations.clear();
            had
        };
        if acted {
            self.view_version += 1;
        }
        acted
    }

    // ---- coordinates ----------------------------------------------------------------------

    /// Sketch coordinates -> screen points in a `w` x `h` viewport.
    pub fn sketch_to_screen(&self, pos: P2, w: f64, h: f64) -> Option<P2> {
        let s = self.sketching.as_ref()?;
        self.camera.project(s.world(pos), w, h).map(|p| (p.x, p.y))
    }

    /// Screen point -> sketch coordinates (the ray through it meets the sketch plane).
    pub fn screen_to_sketch(&self, px: f64, py: f64, w: f64, h: f64) -> Option<P2> {
        let s = self.sketching.as_ref()?;
        let (o, d) = self.camera.ray(px, py, w, h);
        let n = v3(s.frame.normal);
        let denom = d.dot(n);
        if denom.abs() < 1e-9 {
            return None;
        }
        let t = (v3(s.frame.origin) - o).dot(n) / denom;
        Some(s.local(o + d * t))
    }

    pub(crate) fn sketch_data(&self) -> Option<&dc_sketch::Sketch> {
        let s = self.sketching.as_ref()?;
        Some(&self.authority.state().sketch(&s.sketch)?.sketch)
    }

    pub(crate) fn used_ids(&self) -> BTreeSet<String> {
        self.sketch_data()
            .map(|sk| {
                sk.entities()
                    .keys()
                    .chain(sk.constraints().keys())
                    .cloned()
                    .collect()
            })
            .unwrap_or_default()
    }

    /// The sketch as it should be drawn right now: the drag preview if a point is being dragged.
    pub fn view_sketch(&self) -> Option<&dc_sketch::Sketch> {
        let s = self.sketching.as_ref()?;
        s.drag
            .as_ref()
            .and_then(|d| d.sketch.as_ref())
            .or_else(|| s.stroke.as_ref().map(|t| &t.trial))
            .or_else(|| self.sketch_data())
    }

    fn sort_of(&self, id: &str) -> Option<Sort> {
        match id {
            SEL_ORIGIN => return Some(Sort::Origin),
            SEL_AXIS_X => return Some(Sort::AxisX),
            SEL_AXIS_Y => return Some(Sort::AxisY),
            _ => {}
        }
        if let Some(line) = id.strip_prefix(MID_PREFIX) {
            return matches!(self.sketch_data()?.entity(line)?, Entity::Line { .. })
                .then_some(Sort::Point);
        }
        Some(match self.sketch_data()?.entity(id)? {
            Entity::Point { .. } => Sort::Point,
            Entity::Line { .. } => Sort::Line,
            Entity::Circle { .. } => Sort::Circle,
            Entity::Arc { .. } => Sort::Arc,
        })
    }

    // ---- snapping -------------------------------------------------------------------------

    /// What a click at this screen point would place: an existing point, the origin, an axis
    /// guide from the first point, a line, or the snap grid.
    pub fn compute_snap(&self, px: f64, py: f64, w: f64, h: f64) -> Option<Snap> {
        let s = self.sketching.as_ref()?;
        let opts = s.snaps;
        let raw = self.screen_to_sketch(px, py, w, h)?;
        let sk = self.sketch_data()?;
        let screen = |pos: P2| self.sketch_to_screen(pos, w, h);
        let near = |pos: P2| screen(pos).map(|(sx, sy)| det::hypot(sx - px, sy - py));
        let none = |pos: P2| Snap {
            pos,
            point: None,
            on_line: None,
            guide: None,
            origin: false,
            axis: None,
            midpoint: None,
            on_curve: None,
            infer: None,
        };
        if opts.points {
            let best = sk
                .entities()
                .iter()
                .filter_map(|(id, e)| match e {
                    Entity::Point { .. } if sk.only_infinite_line_ends(id) => None,
                    Entity::Point { x, y } => near((*x, *y))
                        .filter(|d| *d <= SNAP_POINT_PX)
                        .map(|d| (d, id, (*x, *y))),
                    _ => None,
                })
                .min_by(|a, b| a.0.total_cmp(&b.0));
            if let Some((_, id, pos)) = best {
                return Some(Snap {
                    point: Some(id.clone()),
                    ..none(pos)
                });
            }
            if near((0.0, 0.0)).is_some_and(|d| d <= SNAP_POINT_PX) {
                return Some(Snap {
                    origin: true,
                    ..none((0.0, 0.0))
                });
            }
        }
        if s.tool.draws() {
            if opts.midpoints {
                if let Some((line, p)) = self.midpoint_near(&near, SNAP_POINT_PX) {
                    return Some(Snap {
                        midpoint: Some(line),
                        ..none(p)
                    });
                }
            }
            if opts.quadrants {
                if let Some((curve, p)) = self.quadrant_near(&near, SNAP_POINT_PX) {
                    return Some(Snap {
                        on_curve: Some(curve),
                        ..none(p)
                    });
                }
            }
        }
        // Perpendicular, collinear or tangent to what the new line starts from.
        if opts.inference && matches!(s.tool, Tool::Line | Tool::Centerline) {
            if let Some((p, infer)) = s.first.as_ref().and_then(|a| self.inferred(a, raw)) {
                return Some(Snap {
                    infer: Some(infer),
                    ..none(p)
                });
            }
        }
        let mut pos = raw;
        let mut guide = None;
        let mut fixed = (false, false);
        if opts.guides && matches!(s.tool, Tool::Line | Tool::Centerline) {
            if let Some(a) = s.first.as_ref().map(|a| a.pos) {
                if let Some((ax, ay)) = screen(a) {
                    if (py - ay).abs() <= GUIDE_PX {
                        pos.1 = a.1;
                        guide = Some(Guide::Horizontal);
                        fixed.1 = true;
                    } else if (px - ax).abs() <= GUIDE_PX {
                        pos.0 = a.0;
                        guide = Some(Guide::Vertical);
                        fixed.0 = true;
                    }
                }
            }
        }
        if guide.is_none() && opts.edges && s.tool.draws() {
            // Onto the middle of an existing line.
            let hit = sk
                .entities()
                .iter()
                .filter_map(|(id, e)| match e {
                    Entity::Line { a, b } => {
                        let (pa, pb) = (sk.point(a)?, sk.point(b)?);
                        let (sa, sb) = (screen(pa)?, screen(pb)?);
                        // An infinite line can be snapped to anywhere along its length.
                        if sk.is_infinite(id) {
                            let (fa, fb) = far_ends(sa, sb);
                            let d = segment_distance((px, py), fa, fb);
                            return (d <= SNAP_EDGE_PX)
                                .then(|| (d, id.clone(), closest_on_line(raw, pa, pb)));
                        }
                        let d = segment_distance((px, py), sa, sb);
                        (d <= SNAP_EDGE_PX)
                            .then(|| (d, id.clone(), closest_on_segment(raw, pa, pb)))
                    }
                    _ => None,
                })
                .min_by(|a, b| a.0.total_cmp(&b.0));
            if let Some((_, id, p)) = hit {
                return Some(Snap {
                    on_line: Some(id),
                    ..none(p)
                });
            }
        }
        // Onto one of the sketch's own axes: the new point gets an on-axis relation.
        let mut axis = None;
        if guide.is_none() && opts.edges && s.tool.draws() {
            if let Some((ox, oy)) = screen((0.0, 0.0)) {
                let on_x = (py - oy).abs() <= SNAP_EDGE_PX;
                let on_y = (px - ox).abs() <= SNAP_EDGE_PX;
                if on_x && on_y {
                    // (the origin itself was handled above, with the points)
                } else if on_x {
                    pos.1 = 0.0;
                    fixed.1 = true;
                    axis = Some(dc_sketch::SketchAxis::X);
                } else if on_y {
                    pos.0 = 0.0;
                    fixed.0 = true;
                    axis = Some(dc_sketch::SketchAxis::Y);
                }
            }
        }
        if s.snap_grid {
            let step = snap_step(self.camera.distance);
            if !fixed.0 {
                pos.0 = (pos.0 / step).round() * step + 0.0;
            }
            if !fixed.1 {
                pos.1 = (pos.1 / step).round() * step + 0.0;
            }
        }
        Some(Snap {
            guide,
            axis,
            ..none(pos)
        })
    }

    /// Mouse moved (or left the viewport, with `None`).
    pub fn sketch_hover(&mut self, at: Option<(f64, f64, f64, f64)>) {
        // A relation's symbol can be pointed at with any tool (to read it); only the Select
        // tool picks it, so drawing never loses a click to a symbol.
        if self
            .sketching
            .as_ref()
            .is_some_and(|s| s.tool != Tool::Select)
        {
            let glyph = at.and_then(|(x, y, w, h)| self.glyph_at(x, y, w, h));
            if let Some(s) = &mut self.sketching {
                s.hover_at = at.filter(|_| glyph.is_some()).map(|(x, y, _, _)| (x, y));
                s.hover_relation = glyph;
                s.hover = None;
            }
        }
        if self
            .sketching
            .as_ref()
            .is_some_and(|s| s.tool == Tool::Dimension)
        {
            if let Some(s) = &mut self.sketching {
                s.cursor = None;
            }
            self.dimension_hover(at);
            return;
        }
        if self
            .sketching
            .as_ref()
            .is_some_and(|s| matches!(s.tool, Tool::Trim | Tool::Extend | Tool::Split))
        {
            self.edit_hover(at);
            return;
        }
        if self
            .sketching
            .as_ref()
            .is_some_and(|s| s.tool == Tool::Select && s.drag.is_none())
        {
            let glyph = at.and_then(|(x, y, w, h)| self.relation_at(x, y, w, h));
            let over = if glyph.is_some() {
                None
            } else {
                at.and_then(|(x, y, w, h)| self.select_pick_at(x, y, w, h))
            };
            if let Some(s) = &mut self.sketching {
                s.hover_at = at
                    .filter(|_| over.is_some() || glyph.is_some())
                    .map(|(x, y, _, _)| (x, y));
                s.hover = over;
                s.hover_relation = glyph;
            }
        } else if let Some(s) = &mut self.sketching {
            // (A symbol's hover for the other tools was set at the top.)
            s.hover = None;
            if s.tool == Tool::Select {
                s.hover_at = None;
                s.hover_relation = None;
            }
        }
        let snap = at.and_then(|(x, y, w, h)| self.compute_snap(x, y, w, h));
        let Some(s) = &mut self.sketching else { return };
        // A centre arc's direction is whichever way the cursor first moves after its start.
        if s.tool == Tool::Arc && s.arc_ccw.is_none() {
            if let (Some(c), Some(st), Some(cur)) = (&s.first, &s.second, &snap) {
                let cross = (st.pos.0 - c.pos.0) * (cur.pos.1 - c.pos.1)
                    - (st.pos.1 - c.pos.1) * (cur.pos.0 - c.pos.0);
                let r = dist(c.pos, st.pos).max(1e-9);
                if (cross / (r * r)).abs() > 0.05 {
                    s.arc_ccw = Some(cross > 0.0);
                }
            }
        }
        s.cursor = snap;
    }

    // ---- clicking -------------------------------------------------------------------------

    /// A left click in the viewport while sketching.
    pub fn sketch_click(&mut self, px: f64, py: f64, w: f64, h: f64) {
        self.sketch_click_with(px, py, w, h, false);
    }

    /// `additive` (Shift/Ctrl held) adds to or removes from the selection instead of replacing it.
    pub fn sketch_click_with(&mut self, px: f64, py: f64, w: f64, h: f64, additive: bool) {
        let Some(tool) = self.sketching.as_ref().map(|s| s.tool) else {
            return;
        };
        match tool {
            Tool::Select => self.sketch_pick(px, py, w, h, additive),
            Tool::Dimension => self.dimension_click(px, py, w, h),
            Tool::Trim | Tool::Extend | Tool::Split => {
                self.edit_click(px, py, w, h);
            }
            _ => {
                if let Some(snap) = self.compute_snap(px, py, w, h) {
                    self.place(tool, &snap);
                }
            }
        }
        self.view_version += 1;
    }

    fn place(&mut self, tool: Tool, snap: &Snap) {
        let mut here = snap.anchor();
        if here.origin {
            // Drawing from the origin glues the shape to it: an existing origin point is reused,
            // otherwise the new point is fixed at (0, 0).
            if let Some(o) = self.origin_entity() {
                here.point = Some(o);
                here.origin = false;
            }
        }
        let (first, second) = self
            .sketching
            .as_ref()
            .map(|s| (s.first.clone(), s.second.clone()))
            .unwrap_or_default();
        let ccw = self.sketching.as_ref().and_then(|s| s.arc_ccw);
        let midpoint_line = self.sketching.as_ref().is_some_and(|s| s.midpoint_line);
        let mut keep: (Option<Anchor>, Option<Anchor>) = (None, None);
        match (tool, first, second) {
            (Tool::Point, _, _) => {
                let _ = self.add_point_at(&here);
            }
            (_, None, _) => keep.0 = Some(here),
            (Tool::Polygon, Some(c), _) => {
                let _ = self.add_polygon(&c, &here);
            }
            (Tool::TangentArc, Some(a), _) => {
                if self.add_tangent_arc(&a, &here).is_err() {
                    keep.0 = Some(a);
                }
            }
            (
                Tool::ThreePointRect
                | Tool::ThreePointCentreRect
                | Tool::Parallelogram
                | Tool::Slot,
                Some(a),
                None,
            ) => {
                if a.pos == here.pos {
                    keep.0 = Some(a);
                } else {
                    keep = (Some(a), Some(here));
                }
            }
            (Tool::ThreePointRect, Some(a), Some(b)) => {
                if self.add_rect3(&a, &b, here.pos).is_err() {
                    keep = (Some(a), Some(b));
                }
            }
            (Tool::ThreePointCentreRect, Some(a), Some(b)) => {
                if self.add_rect3_centre(&a, &b, here.pos).is_err() {
                    keep = (Some(a), Some(b));
                }
            }
            (Tool::Parallelogram, Some(a), Some(b)) => {
                if self.add_parallelogram(&a, &b, &here).is_err() {
                    keep = (Some(a), Some(b));
                }
            }
            (Tool::Slot, Some(a), Some(b)) => {
                if self.add_slot(&a, &b, here.pos).is_err() {
                    keep = (Some(a), Some(b));
                }
            }
            (Tool::Rectangle, Some(a), _) => {
                let _ = self.add_rectangle(&a, &here);
            }
            (Tool::CenterRectangle, Some(c), _) => {
                let _ = self.add_center_rectangle(&c, &here);
            }
            (Tool::Circle, Some(a), _) => {
                let _ = self.add_circle(&a, &here);
            }
            (Tool::Line | Tool::Centerline, Some(a), _) if midpoint_line => {
                if a.pos != here.pos {
                    let centre = tool == Tool::Centerline;
                    if self
                        .add_midpoint_line(&a, &here, snap.guide, centre)
                        .is_err()
                    {
                        keep.0 = Some(a);
                    }
                }
            }
            (Tool::Line | Tool::Centerline, Some(a), _) => {
                let same = a.pos == here.pos || (a.point.is_some() && a.point == here.point);
                if same {
                    // clicking the same spot again ends the chain
                } else {
                    let centre = tool == Tool::Centerline;
                    let started = a.clone();
                    keep.0 = match self.add_line(&a, &here, snap.guide, snap.infer.as_ref(), centre)
                    {
                        Ok(end) => {
                            self.remember_chain_point(started);
                            Some(Anchor {
                                pos: here.pos,
                                point: Some(end),
                                on_line: None,
                                midpoint_of: None,
                                on_curve: None,
                                origin: false,
                                axis: None,
                            })
                        }
                        Err(()) => Some(a),
                    };
                }
            }
            (Tool::Arc | Tool::Arc3, Some(a), None) => {
                if a.pos == here.pos {
                    keep.0 = Some(a);
                } else {
                    keep = (Some(a), Some(here));
                }
            }
            (Tool::Arc, Some(c), Some(st)) => {
                if self.add_arc_centre(&c, &st, here.pos, ccw).is_err() {
                    keep = (Some(c), Some(st));
                }
            }
            (Tool::Arc3, Some(a), Some(b)) => {
                if self.add_arc_3pt(&a, &b, &here).is_err() {
                    keep = (Some(a), Some(b));
                }
            }
            (_, Some(a), s) => keep = (Some(a), s),
        }
        if let Some(s) = &mut self.sketching {
            s.first = keep.0;
            s.second = keep.1;
            if s.first.is_none() {
                s.chain.clear();
            }
            if s.second.is_none() {
                s.arc_ccw = None;
            }
        }
    }

    pub(crate) fn batch(&self) -> Option<Batch> {
        let s = self.sketching.as_ref()?;
        Some(Batch {
            sketch: s.sketch.clone(),
            used: self.used_ids(),
            ops: Vec::new(),
            construction: s.construction,
            infinite: false,
        })
    }

    pub(crate) fn commit(&mut self, label: &str, b: Batch) -> Result<(), ()> {
        self.submit(CommandKind::Transaction {
            label: label.into(),
            ops: b.ops,
        })
        .map(|_| ())
        .map_err(|_| ())
    }

    fn add_rectangle(&mut self, a: &Anchor, b: &Anchor) -> Result<(), ()> {
        let ((x0, y0), (x1, y1)) = (a.pos, b.pos);
        if (x1 - x0).abs() < 1e-6 || (y1 - y0).abs() < 1e-6 {
            self.error(
                "a rectangle needs a width and a height",
                Some("click a second corner that is not in line with the first".into()),
            );
            return Err(());
        }
        let mut bt = self.batch().ok_or(())?;
        let p1 = bt.point(a);
        let p2 = bt.corner((x1, y0));
        let p3 = bt.point(b);
        let p4 = bt.corner((x0, y1));
        Self::rectangle_edges(&mut bt, [&p1, &p2, &p3, &p4]);
        self.commit("Add rectangle", bt)
    }

    /// Four edges between four corners (bottom-left, bottom-right, top-right, top-left) with the
    /// horizontal and vertical relations of an axis-aligned rectangle.
    fn rectangle_edges(bt: &mut Batch, [p1, p2, p3, p4]: [&str; 4]) {
        let bottom = bt.line(p1, p2);
        let right = bt.line(p2, p3);
        let top = bt.line(p3, p4);
        let left = bt.line(p4, p1);
        bt.constraint("h", Constraint::Horizontal { line: bottom });
        bt.constraint("h", Constraint::Horizontal { line: top });
        bt.constraint("v", Constraint::Vertical { line: right });
        bt.constraint("v", Constraint::Vertical { line: left });
    }

    /// A rectangle from its centre, built like SolidWorks does: two construction diagonals and the
    /// centre point constrained to both, so editing the size keeps it centred.
    fn add_center_rectangle(&mut self, c: &Anchor, corner: &Anchor) -> Result<(), ()> {
        let (dx, dy) = (
            (corner.pos.0 - c.pos.0).abs(),
            (corner.pos.1 - c.pos.1).abs(),
        );
        if dx < 1e-6 || dy < 1e-6 {
            self.error(
                "a rectangle needs a width and a height",
                Some("click a corner that is not in line with the centre".into()),
            );
            return Err(());
        }
        let (cx, cy) = c.pos;
        let mut bt = self.batch().ok_or(())?;
        let m = bt.point(c);
        let p1 = bt.corner((cx - dx, cy - dy));
        let p2 = bt.corner((cx + dx, cy - dy));
        let p3 = bt.corner((cx + dx, cy + dy));
        let p4 = bt.corner((cx - dx, cy + dy));
        Self::rectangle_edges(&mut bt, [&p1, &p2, &p3, &p4]);
        for (a, b) in [(&p1, &p3), (&p2, &p4)] {
            let d = bt.push_entity(
                "g",
                Entity::Line {
                    a: a.clone(),
                    b: b.clone(),
                },
            );
            bt.mark_construction(&d);
            bt.constraint(
                "ol",
                Constraint::PointOnLine {
                    point: m.clone(),
                    line: d,
                },
            );
        }
        self.commit("Add centre rectangle", bt)
    }

    fn add_circle(&mut self, centre: &Anchor, edge: &Anchor) -> Result<(), ()> {
        let mut r = dist(centre.pos, edge.pos);
        if self.sketching.as_ref().is_some_and(|s| s.snap_grid) && edge.point.is_none() {
            let step = snap_step(self.camera.distance);
            r = ((r / step).round() * step).max(step);
        }
        if r < 1e-6 {
            self.error(
                "a circle needs a radius",
                Some("click away from the centre".into()),
            );
            return Err(());
        }
        let mut bt = self.batch().ok_or(())?;
        let c = bt.point(centre);
        bt.shape(
            "c",
            Entity::Circle {
                center: c,
                radius: r,
            },
        );
        self.commit("Add circle", bt)
    }

    /// Adds one line; returns the id of its end point so a chain can continue from it.
    /// A line drawn from its middle: the first click is the middle (a point with the midpoint
    /// relation), the second one end, and the other end is the mirror image.
    fn add_midpoint_line(
        &mut self,
        mid: &Anchor,
        end: &Anchor,
        guide: Option<Guide>,
        centerline: bool,
    ) -> Result<(), ()> {
        let mut bt = self.batch().ok_or(())?;
        bt.construction |= centerline;
        bt.infinite = centerline
            && self
                .sketching
                .as_ref()
                .is_some_and(|s| s.centerline_infinite);
        let far = Anchor {
            pos: (2.0 * mid.pos.0 - end.pos.0, 2.0 * mid.pos.1 - end.pos.1),
            point: None,
            on_line: None,
            midpoint_of: None,
            on_curve: None,
            origin: false,
            axis: None,
        };
        let pm = bt.point(mid);
        let pe = bt.point(end);
        let pf = bt.point(&far);
        let line = bt.line(&pf, &pe);
        bt.constraint(
            "mp",
            Constraint::Midpoint {
                point: pm,
                line: line.clone(),
            },
        );
        match guide {
            Some(Guide::Horizontal) => bt.constraint("h", Constraint::Horizontal { line }),
            Some(Guide::Vertical) => bt.constraint("v", Constraint::Vertical { line }),
            None => {}
        }
        self.commit("Add line from its middle", bt)
    }

    fn add_line(
        &mut self,
        a: &Anchor,
        b: &Anchor,
        guide: Option<Guide>,
        infer: Option<&crate::inference::Infer>,
        centerline: bool,
    ) -> Result<String, ()> {
        let mut bt = self.batch().ok_or(())?;
        bt.construction |= centerline;
        bt.infinite = centerline
            && self
                .sketching
                .as_ref()
                .is_some_and(|s| s.centerline_infinite);
        let pa = bt.point(a);
        let pb = bt.point(b);
        let line = bt.line(&pa, &pb);
        match guide {
            Some(Guide::Horizontal) => {
                bt.constraint("h", Constraint::Horizontal { line: line.clone() })
            }
            Some(Guide::Vertical) => {
                bt.constraint("v", Constraint::Vertical { line: line.clone() })
            }
            None => {}
        }
        // A direction the line was pulled onto is a relation too.
        if let Some(i) = infer {
            use crate::inference::InferKind;
            match i.kind {
                InferKind::Perpendicular => bt.constraint(
                    "pe",
                    Constraint::Perpendicular {
                        a: line.clone(),
                        b: i.to.clone(),
                    },
                ),
                InferKind::Collinear => bt.constraint(
                    "ol",
                    Constraint::PointOnLine {
                        point: pb.clone(),
                        line: i.to.clone(),
                    },
                ),
                InferKind::Tangent => bt.constraint(
                    "t",
                    Constraint::Tangent {
                        line: line.clone(),
                        curve: i.to.clone(),
                    },
                ),
            }
        }
        self.commit("Add line", bt).map(|()| pb)
    }

    /// An arc about `c` from `st`, ending where the cursor points; sweeps the way the cursor moved.
    fn add_arc_centre(
        &mut self,
        c: &Anchor,
        st: &Anchor,
        towards: P2,
        ccw: Option<bool>,
    ) -> Result<(), ()> {
        let Some((start, end, _)) = arc_from_centre(c.pos, st.pos, towards, ccw) else {
            self.error(
                "an arc needs an end point",
                Some("click away from the centre".into()),
            );
            return Err(());
        };
        let mut bt = self.batch().ok_or(())?;
        let pc = bt.point(c);
        // The start the user clicked may be an existing point; the end is always a new one.
        let (ps, pe) = if start == st.pos {
            (bt.point(st), bt.corner(end))
        } else {
            (bt.corner(start), bt.point(st))
        };
        bt.shape(
            "a",
            Entity::Arc {
                center: pc,
                start: ps,
                end: pe,
            },
        );
        self.commit("Add arc", bt)
    }

    /// An arc from `a` to `b` passing through `via`.
    fn add_arc_3pt(&mut self, a: &Anchor, b: &Anchor, via: &Anchor) -> Result<(), ()> {
        let Some((centre, _, a_is_start)) = arc_through(a.pos, b.pos, via.pos) else {
            self.error(
                "those three points are in a straight line",
                Some("move the third point off the line between the first two".into()),
            );
            return Err(());
        };
        let mut bt = self.batch().ok_or(())?;
        let (pa, pb) = (bt.point(a), bt.point(b));
        let pc = bt.corner(centre);
        let (start, end) = if a_is_start { (pa, pb) } else { (pb, pa) };
        bt.shape(
            "a",
            Entity::Arc {
                center: pc,
                start,
                end,
            },
        );
        self.commit("Add 3-point arc", bt)
    }

    // ---- picking, selection and dimensions -------------------------------------------------

    /// The nearest sketch entity to a screen point, within the pick radius.
    pub(crate) fn nearest_entity(
        &self,
        px: f64,
        py: f64,
        w: f64,
        h: f64,
        skip_points: bool,
    ) -> Option<String> {
        let sk = self.sketch_data()?;
        let scr = |id: &str| sk.point(id).and_then(|p| self.sketch_to_screen(p, w, h));
        let mut best: Option<(f64, String)> = None;
        let mut offer = |d: f64, id: &String, bias: f64| {
            let d = d - bias;
            if d <= PICK_PX && best.as_ref().is_none_or(|b| d < b.0) {
                best = Some((d, id.clone()));
            }
        };
        let ring = |c: P2, r: f64| -> Option<(f64, f64, f64)> {
            let (cx, cy) = self.sketch_to_screen(c, w, h)?;
            let (ex, _) = self.sketch_to_screen((c.0 + r, c.1), w, h)?;
            Some((cx, cy, (ex - cx).abs()))
        };
        for (id, e) in sk.entities() {
            match e {
                Entity::Point { x, y } if !skip_points && !sk.only_infinite_line_ends(id) => {
                    if let Some((sx, sy)) = self.sketch_to_screen((*x, *y), w, h) {
                        offer(det::hypot(sx - px, sy - py), id, 2.0);
                    }
                }
                Entity::Line { a, b } => {
                    if let (Some(p), Some(q)) = (scr(a), scr(b)) {
                        let (p, q) = if sk.is_infinite(id) {
                            far_ends(p, q)
                        } else {
                            (p, q)
                        };
                        offer(segment_distance((px, py), p, q), id, 0.0);
                    }
                }
                Entity::Circle { center, radius } => {
                    if let Some((cx, cy, r)) = sk.point(center).and_then(|c| ring(c, *radius)) {
                        offer((det::hypot(px - cx, py - cy) - r).abs(), id, 0.0);
                    }
                }
                Entity::Arc { center, start, end } => {
                    if let (Some(c), Some(s), Some(en)) =
                        (sk.point(center), sk.point(start), sk.point(end))
                    {
                        let r = dist(c, s);
                        if let Some((cx, cy, r_px)) = ring(c, r) {
                            // Only the swept part of the circle counts.
                            let cursor = self.screen_to_sketch(px, py, w, h);
                            let on_arc = cursor.is_some_and(|q| {
                                let sweep = ccw_from(angle(c, s), angle(c, en));
                                ccw_from(angle(c, s), angle(c, q)) <= sweep
                            });
                            if on_arc {
                                offer((det::hypot(px - cx, py - cy) - r_px).abs(), id, 0.0);
                            }
                        }
                    }
                }
                _ => {}
            }
        }
        best.map(|b| b.1)
    }

    /// What the Select tool would pick at a screen point: a drawn entity first, then the origin,
    /// then an axis. The origin is the sketch's own fixed point when it has one.
    pub fn select_pick_at(&self, px: f64, py: f64, w: f64, h: f64) -> Option<String> {
        if let Some(id) = self.nearest_entity(px, py, w, h, false) {
            // A line's middle is a handle of its own, there for the taking while the pointer is
            // on the line (and not on a real point that already sits there).
            if let Some(m) = self.midpoint_handle_at(px, py, w, h, &id) {
                return Some(m);
            }
            return Some(id);
        }
        if self.origin_at(px, py, w, h) {
            return Some(
                self.origin_entity()
                    .unwrap_or_else(|| SEL_ORIGIN.to_owned()),
            );
        }
        match self.axis_at(px, py, w, h)? {
            crate::dimensioning::Pick::AxisX => Some(SEL_AXIS_X.to_owned()),
            crate::dimensioning::Pick::AxisY => Some(SEL_AXIS_Y.to_owned()),
            _ => None,
        }
    }

    /// The middle of `line` as a pickable handle (`@mid:<line>`), when the pointer is near it.
    fn midpoint_handle_at(&self, px: f64, py: f64, w: f64, h: f64, line: &str) -> Option<String> {
        let sk = self.sketch_data()?;
        // As in SolidWorks, a line shows its middle only once it is selected.
        if !self.sketching.as_ref()?.selection.iter().any(|x| x == line) {
            return None;
        }
        let Some(Entity::Line { a, b }) = sk.entity(line) else {
            return None;
        };
        // An infinite line has no ends, so no middle.
        if sk.is_infinite(line) {
            return None;
        }
        let (p, q) = (sk.point(a)?, sk.point(b)?);
        let (mx, my) = self.sketch_to_screen(((p.0 + q.0) / 2.0, (p.1 + q.1) / 2.0), w, h)?;
        (det::hypot(mx - px, my - py) <= MID_PICK_PX).then(|| format!("{MID_PREFIX}{line}"))
    }

    /// Replaces selected midpoint handles by real points (made when a relation or a dimension
    /// needs them); selecting one changes nothing in the document until then.
    pub fn materialize_selected_midpoints(&mut self) {
        let ids: Vec<String> = self
            .sketching
            .as_ref()
            .map(|s| s.selection.clone())
            .unwrap_or_default();
        for (i, id) in ids.iter().enumerate() {
            let Some(line) = id.strip_prefix(MID_PREFIX) else {
                continue;
            };
            let real = self.materialize_midpoint(line);
            if let Some(s) = &mut self.sketching {
                match real {
                    Some(p) if !s.selection.contains(&p) => s.selection[i] = p,
                    _ => s.selection[i].clear(),
                }
            }
        }
        if let Some(s) = &mut self.sketching {
            s.selection.retain(|i| !i.is_empty());
        }
    }

    /// A real point standing for the middle of `line`: the one that already has the relation, or
    /// a new one made with it (one undo step).
    pub fn materialize_midpoint(&mut self, line: &str) -> Option<String> {
        let sk = self.sketch_data()?;
        let existing = sk.constraints().values().find_map(|c| match c {
            Constraint::Midpoint { point, line: l } if l == line => Some(point.clone()),
            _ => None,
        });
        if existing.is_some() {
            return existing;
        }
        let Some(Entity::Line { a, b }) = sk.entity(line) else {
            return None;
        };
        let (p, q) = (sk.point(a)?, sk.point(b)?);
        let mut bt = self.batch()?;
        let id = bt.point(&Anchor {
            pos: ((p.0 + q.0) / 2.0, (p.1 + q.1) / 2.0),
            point: None,
            on_line: None,
            midpoint_of: Some(line.to_owned()),
            on_curve: None,
            origin: false,
            axis: None,
        });
        self.commit("Midpoint of line", bt).ok()?;
        Some(id)
    }

    fn sketch_pick(&mut self, px: f64, py: f64, w: f64, h: f64, additive: bool) {
        if let Some(glyph) = self.relation_at(px, py, w, h) {
            self.select_relation(Some(glyph), additive);
            return;
        }
        if let Some(s) = &mut self.sketching {
            if !additive {
                s.relations.clear();
            }
        }
        let hit = self.select_pick_at(px, py, w, h);
        if let Some(s) = &mut self.sketching {
            s.conflict.clear();
            match (hit, additive) {
                (Some(id), true) => {
                    if let Some(i) = s.selection.iter().position(|x| *x == id) {
                        s.selection.remove(i);
                    } else {
                        s.selection.push(id);
                    }
                }
                (Some(id), false) => s.selection = vec![id],
                (None, false) => s.selection.clear(),
                (None, true) => {}
            }
        }
    }

    // ---- relations ------------------------------------------------------------------------

    fn selection_sorts(&self) -> Vec<(String, Sort)> {
        let Some(s) = &self.sketching else {
            return vec![];
        };
        let all: Vec<(String, Sort)> = s
            .selection
            .iter()
            .filter_map(|id| Some((id.clone(), self.sort_of(id)?)))
            .collect();
        // The origin and axes only take part when they are one half of a pair (an axis and a
        // line, the origin and a point). In a bigger selection, like a box around a drawing, they
        // are just along for the ride and must not block "horizontal" and the like.
        let datums = all.iter().filter(|(_, k)| k.is_datum()).count();
        let points = all.iter().filter(|(_, k)| *k == Sort::Point).count();
        let axis = all
            .iter()
            .any(|(_, k)| matches!(k, Sort::AxisX | Sort::AxisY));
        // Two points and an axis are a mirror pair about that axis.
        if (all.len() == 2 && datums == 1) || (all.len() == 3 && datums == 1 && points == 2 && axis)
        {
            all
        } else {
            all.into_iter().filter(|(_, k)| !k.is_datum()).collect()
        }
    }

    /// The relations that make sense for what is selected, in toolbar order.
    pub fn available_relations(&self) -> Vec<Relation> {
        let sel = self.selection_sorts();
        let count = |k: Sort| sel.iter().filter(|(_, s)| *s == k).count();
        let (np, nl, nc, na) = (
            count(Sort::Point),
            count(Sort::Line),
            count(Sort::Circle),
            count(Sort::Arc),
        );
        let mut out = Vec::new();
        if sel.is_empty() {
            return out;
        }
        if sel.len() == 3
            && np == 2
            && sel
                .iter()
                .any(|(_, k)| matches!(k, Sort::AxisX | Sort::AxisY))
        {
            out.push(Relation::Symmetric);
        }
        if sel.len() == 2 {
            if let Some(d) = sel.iter().map(|(_, s)| *s).find(|s| s.is_datum()) {
                let other = sel.iter().map(|(_, s)| *s).find(|s| !s.is_datum());
                match (d, other) {
                    (Sort::Origin, Some(Sort::Point | Sort::Circle | Sort::Arc)) => {
                        out.push(Relation::Coincident)
                    }
                    (Sort::Origin, Some(Sort::Line)) => out.push(Relation::PointOnLine),
                    (Sort::AxisX | Sort::AxisY, Some(Sort::Point | Sort::Circle | Sort::Arc)) => {
                        out.push(Relation::PointOnLine)
                    }
                    (Sort::AxisX | Sort::AxisY, Some(Sort::Line)) => out.extend([
                        Relation::Parallel,
                        Relation::Perpendicular,
                        Relation::PointOnLine,
                    ]),
                    _ => {}
                }
            }
        }
        if nl >= 1 && nl == sel.len() {
            out.extend([Relation::Horizontal, Relation::Vertical]);
        }
        if nl == 2 && sel.len() == 2 {
            out.extend([
                Relation::Parallel,
                Relation::Perpendicular,
                Relation::Equal,
                Relation::Collinear,
            ]);
        }
        if nc + na == 2 && sel.len() == 2 {
            out.extend([Relation::Equal, Relation::Concentric]);
        }
        if np == 2 && sel.len() == 2 {
            out.extend([Relation::Coincident, Relation::Merge]);
        }
        if np == 1 && nl == 1 && sel.len() == 2 {
            out.extend([Relation::PointOnLine, Relation::Midpoint]);
        }
        if np == 1 && nc + na == 1 && sel.len() == 2 {
            out.push(Relation::OnCurve);
        }
        if np == 2 && nl == 1 && sel.len() == 3 {
            out.push(Relation::Symmetric);
        }
        if (nl == 1 && nc + na == 1 || nc + na == 2) && sel.len() == 2 {
            out.push(Relation::Tangent);
        }
        if !self.points_to_fix().is_empty() {
            out.push(Relation::Fix);
        }
        out
    }

    /// Points of the selection that are not already pinned, with their current positions.
    fn points_to_fix(&self) -> Vec<(String, P2)> {
        let (Some(sk), Some(s)) = (self.sketch_data(), self.sketching.as_ref()) else {
            return vec![];
        };
        let fixed: BTreeSet<&String> = sk
            .constraints()
            .values()
            .filter_map(|c| match c {
                Constraint::Fixed { point, .. } => Some(point),
                _ => None,
            })
            .collect();
        let mut pts: Vec<String> = Vec::new();
        for id in &s.selection {
            match sk.entity(id) {
                Some(Entity::Point { .. }) => pts.push(id.clone()),
                Some(e) => pts.extend(defining_points(e).into_iter().cloned()),
                None => {}
            }
        }
        pts.sort();
        pts.dedup();
        pts.into_iter()
            .filter(|p| !fixed.contains(p))
            .filter_map(|p| sk.point(&p).map(|pos| (p, pos)))
            .collect()
    }

    /// Adds the relation to the selection as one command.
    pub fn apply_relation(&mut self, r: Relation) -> bool {
        if !self.available_relations().contains(&r) {
            return false;
        }
        self.materialize_selected_midpoints();
        let sel = self.selection_sorts();
        let ids: Vec<String> = sel.iter().map(|(i, _)| i.clone()).collect();
        if r == Relation::Merge {
            let sketch = self.sketching.as_ref().map(|s| s.sketch.clone());
            let before: BTreeMap<String, String> = self
                .sketch_data()
                .map(|sk| {
                    sk.constraints()
                        .iter()
                        .map(|(id, c)| (id.clone(), crate::relations::relation_label(c)))
                        .collect()
                })
                .unwrap_or_default();
            let ok = sketch.is_some_and(|sketch| {
                self.submit(CommandKind::SketchMergePoints {
                    sketch,
                    a: ids[0].clone(),
                    b: ids[1].clone(),
                })
                .is_ok()
            });
            if ok {
                if let Some(s) = &mut self.sketching {
                    s.selection.clear();
                }
                // Relations that no longer make sense once the points are one are dropped:
                // say which, so nothing disappears unnoticed.
                let kept: BTreeSet<String> = self
                    .sketch_data()
                    .map(|sk| sk.constraints().keys().cloned().collect())
                    .unwrap_or_default();
                let dropped: Vec<&String> = before
                    .iter()
                    .filter(|(id, _)| !kept.contains(*id))
                    .map(|(_, label)| label)
                    .collect();
                if !dropped.is_empty() {
                    self.info(format!(
                        "merged; dropped {} relation(s) that no longer apply: {}",
                        dropped.len(),
                        dropped
                            .iter()
                            .map(|l| l.as_str())
                            .collect::<Vec<_>>()
                            .join(", ")
                    ));
                }
            }
            return ok;
        }
        let Some(mut bt) = self.batch() else {
            return false;
        };
        if sel.iter().any(|(_, s)| s.is_datum()) {
            if !self.datum_relation(&mut bt, r, &sel) {
                return false;
            }
            return self
                .commit(&format!("Add relation: {}", r.name()), bt)
                .is_ok();
        }
        match r {
            Relation::Horizontal => ids
                .iter()
                .for_each(|l| bt.constraint("h", Constraint::Horizontal { line: l.clone() })),
            Relation::Vertical => ids
                .iter()
                .for_each(|l| bt.constraint("v", Constraint::Vertical { line: l.clone() })),
            Relation::Parallel => bt.constraint(
                "pa",
                Constraint::Parallel {
                    a: ids[0].clone(),
                    b: ids[1].clone(),
                },
            ),
            Relation::Perpendicular => bt.constraint(
                "pe",
                Constraint::Perpendicular {
                    a: ids[0].clone(),
                    b: ids[1].clone(),
                },
            ),
            Relation::Equal => bt.constraint(
                "eq",
                if sel[0].1 == Sort::Line {
                    Constraint::EqualLength {
                        a: ids[0].clone(),
                        b: ids[1].clone(),
                    }
                } else {
                    Constraint::EqualRadius {
                        a: ids[0].clone(),
                        b: ids[1].clone(),
                    }
                },
            ),
            Relation::Midpoint | Relation::OnCurve | Relation::Symmetric => {
                let of = |k: Sort| sel.iter().position(|(_, s)| *s == k);
                let curve = of(Sort::Circle).or(of(Sort::Arc));
                let Some(p) = of(Sort::Point) else {
                    return false;
                };
                match r {
                    Relation::Midpoint => bt.constraint(
                        "mp",
                        Constraint::Midpoint {
                            point: ids[p].clone(),
                            line: ids[of(Sort::Line).unwrap_or(p)].clone(),
                        },
                    ),
                    Relation::OnCurve => bt.constraint(
                        "oc",
                        Constraint::PointOnCurve {
                            point: ids[p].clone(),
                            curve: ids[curve.unwrap_or(p)].clone(),
                        },
                    ),
                    _ => {
                        let points: Vec<&String> = sel
                            .iter()
                            .filter(|(_, s)| *s == Sort::Point)
                            .map(|(i, _)| i)
                            .collect();
                        bt.constraint(
                            "sy",
                            Constraint::Symmetric {
                                a: points[0].clone(),
                                b: points[1].clone(),
                                line: ids[of(Sort::Line).unwrap_or(p)].clone(),
                            },
                        );
                    }
                }
            }
            Relation::Concentric => {
                let Some(sk) = self.sketch_data() else {
                    return false;
                };
                let centre = |id: &str| match sk.entity(id) {
                    Some(Entity::Circle { center, .. } | Entity::Arc { center, .. }) => {
                        Some(center.clone())
                    }
                    _ => None,
                };
                let (Some(a), Some(b)) = (centre(&ids[0]), centre(&ids[1])) else {
                    return false;
                };
                if a == b {
                    return false;
                }
                bt.constraint("cc", Constraint::Coincident { a, b });
            }
            Relation::Collinear => {
                let Some(sk) = self.sketch_data() else {
                    return false;
                };
                let (Some(Entity::Line { a: a1, b: a2 }), Some(Entity::Line { a: b1, b: b2 })) =
                    (sk.entity(&ids[0]), sk.entity(&ids[1]))
                else {
                    return false;
                };
                let ends: Vec<&String> = [b1, b2]
                    .into_iter()
                    .filter(|p| *p != a1 && *p != a2)
                    .collect();
                if ends.is_empty() {
                    return false;
                }
                for p in ends {
                    bt.constraint(
                        "cl",
                        Constraint::PointOnLine {
                            point: p.clone(),
                            line: ids[0].clone(),
                        },
                    );
                }
            }
            Relation::Coincident => bt.constraint(
                "co",
                Constraint::Coincident {
                    a: ids[0].clone(),
                    b: ids[1].clone(),
                },
            ),
            Relation::PointOnLine => {
                let (p, l) = if sel[0].1 == Sort::Point {
                    (0, 1)
                } else {
                    (1, 0)
                };
                bt.constraint(
                    "ol",
                    Constraint::PointOnLine {
                        point: ids[p].clone(),
                        line: ids[l].clone(),
                    },
                );
            }
            Relation::Tangent => {
                if sel
                    .iter()
                    .all(|(_, k)| matches!(k, Sort::Circle | Sort::Arc))
                {
                    bt.constraint(
                        "t",
                        Constraint::TangentCurves {
                            a: ids[0].clone(),
                            b: ids[1].clone(),
                        },
                    );
                } else {
                    let (l, c) = if sel[0].1 == Sort::Line {
                        (0, 1)
                    } else {
                        (1, 0)
                    };
                    bt.constraint(
                        "t",
                        Constraint::Tangent {
                            line: ids[l].clone(),
                            curve: ids[c].clone(),
                        },
                    );
                }
            }
            Relation::Merge => return false, // handled above: it is not a constraint
            Relation::Fix => {
                for (p, (x, y)) in self.points_to_fix() {
                    bt.constraint("fx", Constraint::Fixed { point: p, x, y });
                }
            }
        }
        self.commit(&format!("Add relation: {}", r.name()), bt)
            .is_ok()
    }

    /// The relation between the origin or an axis and one other thing (see `available_relations`).
    fn datum_relation(&self, bt: &mut Batch, r: Relation, sel: &[(String, Sort)]) -> bool {
        let Some((d, _)) = sel.iter().find(|(_, s)| s.is_datum()) else {
            return false;
        };
        if r == Relation::Symmetric {
            let pts: Vec<&String> = sel
                .iter()
                .filter(|(_, k)| *k == Sort::Point)
                .map(|(i, _)| i)
                .collect();
            let axis = match d.as_str() {
                SEL_AXIS_X => dc_sketch::SketchAxis::X,
                SEL_AXIS_Y => dc_sketch::SketchAxis::Y,
                _ => return false,
            };
            if let [p, q] = pts.as_slice() {
                bt.constraint(
                    "sa",
                    Constraint::SymmetricAxis {
                        a: (*p).clone(),
                        b: (*q).clone(),
                        axis,
                    },
                );
                return true;
            }
            return false;
        }
        let Some((id, sort)) = sel.iter().find(|(_, s)| !s.is_datum()) else {
            return false;
        };
        let axis = match d.as_str() {
            SEL_AXIS_X => Some(dc_sketch::SketchAxis::X),
            SEL_AXIS_Y => Some(dc_sketch::SketchAxis::Y),
            _ => None,
        };
        let Some(sk) = self.sketch_data() else {
            return false;
        };
        // The point that stands for the thing: itself, or a circle's / arc's centre.
        let anchor = match sk.entity(id) {
            Some(Entity::Point { .. }) => Some(id.clone()),
            Some(Entity::Circle { center, .. } | Entity::Arc { center, .. }) => {
                Some(center.clone())
            }
            _ => None,
        };
        match (r, axis, sort) {
            (Relation::Coincident, None, _) => {
                let Some(p) = anchor else { return false };
                let (x, y) = (0.0, 0.0);
                bt.constraint("fx", Constraint::Fixed { point: p, x, y });
            }
            (Relation::PointOnLine, None, Sort::Line) => {
                let Some(Entity::Line { a, b }) = sk.entity(id) else {
                    return false;
                };
                let o = match self.origin_entity() {
                    Some(o) => o,
                    None => {
                        let o = bt.push_entity("o", Entity::Point { x: 0.0, y: 0.0 });
                        bt.constraint(
                            "fx",
                            Constraint::Fixed {
                                point: o.clone(),
                                x: 0.0,
                                y: 0.0,
                            },
                        );
                        o
                    }
                };
                if *a == o || *b == o {
                    return false;
                }
                bt.constraint(
                    "ol",
                    Constraint::PointOnLine {
                        point: o,
                        line: id.clone(),
                    },
                );
            }
            (Relation::PointOnLine, Some(a), Sort::Line) => {
                if let Some(Entity::Line { a: p, b: q }) = sk.entity(id) {
                    for end in [p.clone(), q.clone()] {
                        bt.constraint(
                            "ax",
                            Constraint::OnAxis {
                                point: end,
                                axis: a,
                            },
                        );
                    }
                }
            }
            (Relation::PointOnLine, Some(a), _) => {
                let Some(p) = anchor else { return false };
                bt.constraint("ax", Constraint::OnAxis { point: p, axis: a });
            }
            (Relation::Parallel | Relation::Perpendicular, Some(a), Sort::Line) => {
                let along = (r == Relation::Parallel) == (a == dc_sketch::SketchAxis::X);
                let line = id.clone();
                bt.constraint(
                    if along { "h" } else { "v" },
                    if along {
                        Constraint::Horizontal { line }
                    } else {
                        Constraint::Vertical { line }
                    },
                );
            }
            _ => return false,
        }
        true
    }

    /// Removes a relation or dimension by its id.
    pub fn remove_constraint(&mut self, sketch: &str, id: &str) -> bool {
        self.submit(CommandKind::SketchRemoveConstraint {
            sketch: sketch.to_owned(),
            constraint_id: id.to_owned(),
        })
        .is_ok()
    }

    /// Flips construction on the selected lines, circles and arcs (or, with nothing selected,
    /// the mode for new shapes).
    pub fn toggle_construction(&mut self) -> bool {
        let Some(s) = &self.sketching else {
            return false;
        };
        if s.selection.is_empty() {
            let on = !s.construction;
            self.set_construction_mode(on);
            return true;
        }
        let (Some(sk), sketch) = (self.sketch_data(), s.sketch.clone()) else {
            return false;
        };
        let ops: Vec<CommandKind> = s
            .selection
            .iter()
            .filter(|id| !matches!(sk.entity(id), Some(Entity::Point { .. }) | None))
            .map(|id| CommandKind::SketchSetConstruction {
                sketch: sketch.clone(),
                entity_id: id.clone(),
                construction: !sk.is_construction(id),
            })
            .collect();
        if ops.is_empty() {
            return false;
        }
        self.submit(CommandKind::Transaction {
            label: "Toggle construction".into(),
            ops,
        })
        .is_ok()
    }

    /// Makes the selected construction lines infinite, or finite again if they all are already.
    pub fn toggle_infinite(&mut self) -> bool {
        let Some(s) = &self.sketching else {
            return false;
        };
        let (Some(sk), sketch) = (self.sketch_data(), s.sketch.clone()) else {
            return false;
        };
        let lines: Vec<&String> = s
            .selection
            .iter()
            .filter(|id| {
                matches!(sk.entity(id), Some(Entity::Line { .. })) && sk.is_construction(id)
            })
            .collect();
        if lines.is_empty() {
            return false;
        }
        let on = !lines.iter().all(|id| sk.is_infinite(id));
        let ops: Vec<CommandKind> = lines
            .iter()
            .map(|id| CommandKind::SketchSetInfinite {
                sketch: sketch.clone(),
                entity_id: (*id).clone(),
                infinite: on,
            })
            .collect();
        self.submit(CommandKind::Transaction {
            label: "Toggle infinite length".into(),
            ops,
        })
        .is_ok()
    }

    /// Deletes the selected entities together with what only they needed.
    pub fn sketch_delete_selected(&mut self) -> bool {
        if self
            .sketching
            .as_ref()
            .is_some_and(|s| !s.relations.is_empty() && s.selection.is_empty())
        {
            return self.delete_selected_relation();
        }
        let Some(s) = &self.sketching else {
            return false;
        };
        if s.selection.is_empty() {
            return false;
        }
        let (selection, sketch, relations) =
            (s.selection.clone(), s.sketch.clone(), s.relations.clone());
        let Some(sk) = self.sketch_data() else {
            return false;
        };
        let mut gone: Vec<String> = Vec::new();
        for id in selection.iter().filter(|i| !is_datum(i)) {
            if sk.entity(id).is_some() && !gone.contains(id) {
                gone.push(id.clone());
            }
        }
        if gone.is_empty() {
            return false;
        }
        // End points and centres that nothing else still uses go too.
        let shapes: Vec<String> = gone
            .iter()
            .filter(|g| !matches!(sk.entity(g), Some(Entity::Point { .. })))
            .cloned()
            .collect();
        for id in &shapes {
            if let Some(e) = sk.entity(id) {
                for p in defining_points(e) {
                    let used_elsewhere = sk
                        .entities()
                        .iter()
                        .any(|(oid, o)| !gone.contains(oid) && defining_points(o).contains(&p));
                    if !used_elsewhere && !gone.contains(p) {
                        gone.push(p.clone());
                    }
                }
            }
        }
        let mut ops: Vec<CommandKind> = sk
            .constraints()
            .iter()
            .filter(|(_, c)| c.refs().iter().any(|r| gone.iter().any(|g| g == r)))
            .map(|(cid, _)| CommandKind::SketchRemoveConstraint {
                sketch: sketch.clone(),
                constraint_id: cid.clone(),
            })
            .collect();
        // Relations and dimensions picked as well go in the same step.
        let already: BTreeSet<String> = ops
            .iter()
            .filter_map(|o| match o {
                CommandKind::SketchRemoveConstraint { constraint_id, .. } => {
                    Some(constraint_id.clone())
                }
                _ => None,
            })
            .collect();
        for cid in relations
            .iter()
            .filter(|c| !already.contains(*c) && sk.constraints().contains_key(*c))
        {
            ops.push(CommandKind::SketchRemoveConstraint {
                sketch: sketch.clone(),
                constraint_id: cid.clone(),
            });
        }
        // Shapes before the points they stand on.
        gone.sort_by_key(|g| matches!(sk.entity(g), Some(Entity::Point { .. })));
        ops.extend(gone.into_iter().map(|g| CommandKind::SketchRemoveEntity {
            sketch: sketch.clone(),
            entity_id: g,
        }));
        let ok = self
            .submit(CommandKind::Transaction {
                label: "Delete sketch entities".into(),
                ops,
            })
            .is_ok();
        if ok {
            if let Some(s) = &mut self.sketching {
                s.selection.clear();
                s.relations.clear();
            }
        }
        ok
    }

    // ---- dragging a point -------------------------------------------------------------------

    /// Pressing with a drawing tool and dragging draws the shape in one gesture: the press is the
    /// first click and the release the second (a line then ends instead of chaining on).
    pub fn drag_draw_begin(&mut self, px: f64, py: f64, w: f64, h: f64) -> bool {
        let ok = self.sketching.as_ref().is_some_and(|s| {
            s.first.is_none()
                && matches!(
                    s.tool,
                    Tool::Line | Tool::Rectangle | Tool::CenterRectangle | Tool::Circle
                )
        });
        if !ok {
            return false;
        }
        self.sketch_click(px, py, w, h);
        if let Some(s) = &mut self.sketching {
            s.drag_drawing = s.first.is_some();
        }
        true
    }

    /// Releasing finishes the shape drawn by [`drag_draw_begin`](Self::drag_draw_begin).
    pub fn drag_draw_end(&mut self, px: f64, py: f64, w: f64, h: f64) -> bool {
        let Some(s) = &mut self.sketching else {
            return false;
        };
        if !s.drag_drawing {
            return false;
        }
        s.drag_drawing = false;
        let line = s.tool == Tool::Line;
        let start = s.first.as_ref().map(|a| a.pos);
        let moved = start
            .zip(self.screen_to_sketch(px, py, w, h))
            .is_some_and(|(a, b)| {
                self.sketch_to_screen(a, w, h)
                    .zip(self.sketch_to_screen(b, w, h))
                    .is_some_and(|(p, q)| dist(p, q) > 3.0)
            });
        // Letting go where it started was a click: the shape just waits for its next click.
        if !moved {
            return true;
        }
        self.sketch_click(px, py, w, h);
        if line && moved {
            if let Some(s) = &mut self.sketching {
                s.first = None; // one line, not a chain
            }
        }
        true
    }

    /// Starts dragging what is under the cursor (Select tool): a point follows the cursor, a line
    /// or arc moves, a circle's rim resizes it. Returns whether there was something to grab.
    pub fn sketch_drag_begin(&mut self, px: f64, py: f64, w: f64, h: f64) -> bool {
        if self
            .sketching
            .as_ref()
            .is_none_or(|s| s.tool != Tool::Select)
            || self.relation_at(px, py, w, h).is_some()
        {
            return false;
        }
        let Some(id) = self.nearest_entity(px, py, w, h, false) else {
            return false;
        };
        let (Some(cursor), Some(sk)) = (self.screen_to_sketch(px, py, w, h), self.sketch_data())
        else {
            return false;
        };
        let at = |p: &String| sk.point(p).map(|pos| (p.clone(), pos));
        // Grabbing one of several selected things moves them all.
        let picked: Vec<&String> = self
            .sketching
            .as_ref()
            .map(|s| s.selection.iter().filter(|i| !is_datum(i)).collect())
            .unwrap_or_default();
        let multi = picked.len() > 1 && picked.contains(&&id);
        let multi_anchors: Option<Vec<(String, P2)>> = multi.then(|| {
            let mut ids: BTreeSet<String> = BTreeSet::new();
            for p in &picked {
                match sk.entity(p) {
                    Some(Entity::Point { .. }) => {
                        ids.insert((*p).clone());
                    }
                    Some(e) => ids.extend(defining_points(e).into_iter().cloned()),
                    None => {}
                }
            }
            ids.iter().filter_map(at).collect()
        });
        let (kind, start, anchors, offset, circle) = match (multi_anchors, sk.entity(&id)) {
            (Some(anchors), _) => (DragKind::Shape, cursor, anchors, (0.0, 0.0), None),
            (None, e) => match e {
                Some(Entity::Point { x, y }) => (
                    DragKind::Point,
                    (*x, *y),
                    vec![(id.clone(), (*x, *y))],
                    (x - cursor.0, y - cursor.1),
                    None,
                ),
                Some(Entity::Line { a, b }) => {
                    let pts: Option<Vec<_>> = [a, b].into_iter().map(at).collect();
                    match pts {
                        Some(p) => (DragKind::Shape, cursor, p, (0.0, 0.0), None),
                        None => return false,
                    }
                }
                Some(Entity::Arc { center, start, end }) => {
                    let pts: Option<Vec<_>> = [center, start, end].into_iter().map(at).collect();
                    match pts {
                        Some(p) => (DragKind::Shape, cursor, p, (0.0, 0.0), None),
                        None => return false,
                    }
                }
                Some(Entity::Circle { center, radius }) => match sk.point(center) {
                    Some(c) => (
                        DragKind::Radius,
                        cursor,
                        // The centre is held where it is: pulling the rim only changes the size.
                        vec![(center.clone(), c)],
                        (0.0, 0.0),
                        Some((id.clone(), c, *radius)),
                    ),
                    None => return false,
                },
                None => return false,
            },
        };
        let initial = DragHolds {
            points: anchors
                .iter()
                .map(|(p, (x, y))| dc_sketch::DragTarget {
                    point: p.clone(),
                    x: *x,
                    y: *y,
                })
                .collect(),
            radius: circle.as_ref().map(|(c, _, r)| dc_sketch::DragRadius {
                circle: c.clone(),
                value: *r,
            }),
        };
        if let Some(s) = &mut self.sketching {
            if !multi {
                s.selection = vec![id.clone()];
            }
            s.relations.clear();
            s.drag = Some(Drag {
                point: id,
                kind,
                start,
                target: start,
                anchors,
                offset,
                circle,
                holds: initial.clone(),
                initial,
                preview: BTreeMap::new(),
                sketch: None,
                broken: None,
                multi,
                stuck: None,
                check_pending: false,
                last_check: None,
            });
        }
        true
    }

    /// The cursor moved while dragging, grid snapping as set.
    pub fn sketch_drag_move(&mut self, px: f64, py: f64, w: f64, h: f64) {
        self.sketch_drag_move_with(px, py, w, h, false);
    }

    /// The cursor moved while dragging: solve a copy of the sketch with what is dragged held at
    /// the cursor, so everything related to it (lines, circles and their radii, dimensions,
    /// relation symbols) is drawn following it. Nothing is logged and the document is untouched;
    /// this is the same solver call the release commits, so what you see is what you get.
    /// `free` (Ctrl held) switches grid snapping off for this move.
    ///
    /// Cost: one clone of the sketch and one solve of the connected cluster only, and only when
    /// what is held changed. Whether the features built from the sketch survive is checked at most
    /// every 100 ms, and only if there are any.
    pub fn sketch_drag_move_with(&mut self, px: f64, py: f64, w: f64, h: f64, free: bool) {
        let Some(cursor) = self.screen_to_sketch(px, py, w, h) else {
            return;
        };
        let Some(s) = &self.sketching else { return };
        let Some(d) = &s.drag else { return };
        let step = snap_step(self.camera.distance);
        let snap = |v: f64| {
            if s.snap_grid && !free {
                (v / step).round() * step + 0.0
            } else {
                v
            }
        };
        let (target, holds) = match d.kind {
            DragKind::Point => {
                let t = (snap(cursor.0 + d.offset.0), snap(cursor.1 + d.offset.1));
                (
                    t,
                    DragHolds {
                        points: vec![dc_sketch::DragTarget {
                            point: d.point.clone(),
                            x: t.0,
                            y: t.1,
                        }],
                        radius: None,
                    },
                )
            }
            DragKind::Shape => {
                // An edge that is horizontal or vertical only moves across itself: sliding it
                // along its own length would drag the rest of the shape sideways for nothing.
                let sk = self.sketch_data();
                let axis_locked = |horizontal: bool| {
                    sk.is_some_and(|sk| {
                        sk.constraints().values().any(|c| match c {
                            Constraint::Horizontal { line } => horizontal && *line == d.point,
                            Constraint::Vertical { line } => !horizontal && *line == d.point,
                            _ => false,
                        })
                    })
                };
                let mut delta = (snap(cursor.0 - d.start.0), snap(cursor.1 - d.start.1));
                if !d.multi && axis_locked(true) {
                    delta.0 = 0.0;
                }
                if !d.multi && axis_locked(false) {
                    delta.1 = 0.0;
                }
                (
                    (d.start.0 + delta.0, d.start.1 + delta.1),
                    DragHolds {
                        points: d
                            .anchors
                            .iter()
                            .map(|(p, (x, y))| dc_sketch::DragTarget {
                                point: p.clone(),
                                x: x + delta.0,
                                y: y + delta.1,
                            })
                            .collect(),
                        radius: None,
                    },
                )
            }
            DragKind::Radius => {
                let Some((circle, centre, _)) = &d.circle else {
                    return;
                };
                let r = snap(dist(*centre, cursor)).max(1e-3);
                (
                    cursor,
                    DragHolds {
                        points: d
                            .anchors
                            .iter()
                            .map(|(p, (x, y))| dc_sketch::DragTarget {
                                point: p.clone(),
                                x: *x,
                                y: *y,
                            })
                            .collect(),
                        radius: Some(dc_sketch::DragRadius {
                            circle: circle.clone(),
                            value: r,
                        }),
                    },
                )
            }
        };
        if d.sketch.is_some() && d.holds == holds {
            // Nothing new to solve; maybe a pending feature check is due.
            self.drag_check(false);
            return;
        }
        let Some(original) = self.sketch_data() else {
            return;
        };
        let mut trial = original.clone();
        // Only the cluster connected to what is dragged is solved. A refused drag leaves `trial`
        // as it was, which is also what release would do.
        if trial
            .drag_local(&holds.points, holds.radius.as_ref())
            .is_err()
        {
            return;
        }
        // Dimensions follow what they measure, in the preview as they will once committed.
        trial.follow_placements(original);
        let reached = holds.points.iter().all(|t| {
            trial
                .point(&t.point)
                .is_some_and(|p| dist(p, (t.x, t.y)) < 1e-6)
        }) && holds.radius.as_ref().is_none_or(|r| {
            matches!(trial.entity(&r.circle), Some(Entity::Circle { radius, .. })
                if (radius - r.value).abs() < 1e-6)
        });
        let stuck = if reached {
            None
        } else {
            self.why_stuck(&holds)
        };
        let preview: BTreeMap<String, (f64, f64)> = trial
            .entities()
            .keys()
            .filter_map(|id| {
                let now = trial.point(id)?;
                (original.point(id) != Some(now)).then(|| (id.clone(), now))
            })
            .collect();
        if let Some(d) = self.sketching.as_mut().and_then(|s| s.drag.as_mut()) {
            d.target = target;
            d.holds = holds;
            d.preview = preview;
            d.stuck = stuck;
            d.sketch = Some(trial);
            d.check_pending = true;
        }
        self.drag_check(false);
    }

    /// Why what is being dragged cannot follow the cursor, when that is worth saying: it is fully
    /// defined (and by what), or it can only slide in one direction.
    fn why_stuck(&self, holds: &DragHolds) -> Option<String> {
        let s = self.sketching.as_ref()?;
        let d = s.drag.as_ref()?;
        let slot = self.authority.state().sketch(&s.sketch)?;
        let report = &slot.report;
        let sk = &slot.sketch;
        let pinned_points = holds
            .points
            .iter()
            .all(|t| !report.motion.contains_key(&t.point));
        let radius_pinned = holds
            .radius
            .as_ref()
            .is_none_or(|r| !report.radius_free.contains(&r.circle));
        if pinned_points && radius_pinned {
            // What holds it: relations that mention it or the points it stands on.
            let mut touched: BTreeSet<String> =
                holds.points.iter().map(|t| t.point.clone()).collect();
            touched.extend(holds.radius.iter().map(|r| r.circle.clone()));
            if let Some(Entity::Circle { center, .. }) = sk.entity(&d.point) {
                touched.insert(center.clone());
            }
            let held: Vec<String> = sk
                .constraints()
                .values()
                .filter(|c| c.refs().iter().any(|r| touched.contains(*r)))
                .take(3)
                .map(crate::relations::relation_label)
                .collect();
            let by = if held.is_empty() {
                String::new()
            } else {
                format!(" Held by: {}.", held.join("; "))
            };
            return Some(format!(
                "Fully defined, so it cannot move.{by} Remove or change one to free it."
            ));
        }
        if let ([t], None) = (holds.points.as_slice(), &holds.radius) {
            if let Some(dir) = report.motion.get(&t.point).and_then(|m| m.direction) {
                let how = if dir[0].abs() < 1e-6 {
                    "vertically".to_owned()
                } else if dir[1].abs() < 1e-6 {
                    "horizontally".to_owned()
                } else {
                    format!(
                        "along {}°",
                        fmt_num(det::atan2(dir[1], dir[0]).to_degrees())
                    )
                };
                return Some(format!("Held by its relations: it can only move {how}."));
            }
        }
        // The drag could not be followed (only asked when it was not): say so in general terms.
        Some(
            "Held by relations or dimensions: it cannot follow the cursor exactly, so it settles at the nearest valid position."
                .to_owned(),
        )
    }

    /// Asks the document whether the drag, committed now, would keep every feature built from the
    /// sketch valid (a dry run: nothing is logged). Skipped when nothing is built from the sketch,
    /// and rate limited, because a feature rebuild is far more work than moving a point.
    /// `force` ignores the rate limit.
    pub fn drag_check(&mut self, force: bool) {
        let Some(s) = &self.sketching else { return };
        let Some(d) = &s.drag else { return };
        if !d.check_pending {
            return;
        }
        if !force
            && d.last_check
                .is_some_and(|t| t.elapsed() < FEATURE_CHECK_EVERY)
        {
            return;
        }
        let sketch = s.sketch.clone();
        let broken = if self
            .authority
            .state()
            .features_of_sketch(&sketch)
            .is_empty()
        {
            None
        } else {
            let cmd = dc_doc::Command {
                id: "drag-check".into(),
                actor: self.actor.clone(),
                base_revision: self.authority.revision(),
                kind: d.holds.command(&sketch),
            };
            self.authority.dry_run(&cmd).err().map(|e| e.message)
        };
        if let Some(d) = self.sketching.as_mut().and_then(|s| s.drag.as_mut()) {
            d.broken = broken;
            d.check_pending = false;
            d.last_check = Some(std::time::Instant::now());
        }
    }

    /// Does the drag in progress still have a feature check to run (so the UI keeps redrawing)?
    pub fn drag_check_due(&self) -> bool {
        self.sketching
            .as_ref()
            .and_then(|s| s.drag.as_ref())
            .is_some_and(|d| d.check_pending)
    }

    /// Releasing the mouse: one command for the whole drag.
    pub fn sketch_drag_end(&mut self) -> bool {
        let Some(s) = &mut self.sketching else {
            return false;
        };
        let (Some(d), sketch) = (s.drag.take(), s.sketch.clone()) else {
            return false;
        };
        self.view_version += 1;
        // Nothing to commit if the cursor never moved it, or the sketch refused to follow.
        if d.sketch.is_none() || d.holds == d.initial {
            return false;
        }
        if self.sketch_data() == d.sketch.as_ref() {
            self.info("it did not move: it is fully defined");
            return false;
        }
        self.submit(d.holds.command(&sketch)).is_ok()
    }

    // ---- what to draw ---------------------------------------------------------------------

    /// The active sketch, its preview and the cursor marker, in screen points.
    pub fn sketch_overlay(&self, w: f64, h: f64) -> Option<Overlay> {
        let s = self.sketching.as_ref()?;
        let slot = self.authority.state().sketch(&s.sketch)?;
        let sk = &slot.sketch;
        let free: BTreeSet<&String> = slot.report.under_constrained.iter().collect();
        // While a point is being dragged, draw the solved copy (circle radii and all).
        let sk = self.view_sketch().unwrap_or(sk);
        let point = |id: &str| sk.point(id);
        let scr = |p: P2| self.sketch_to_screen(p, w, h);
        let mut o = Overlay {
            dof: slot.report.dof,
            shapes_defined: slot.report.shape_dof == Some(0),
            name: s.sketch.clone(),
            ..Overlay::default()
        };
        let sel = |id: &str| s.selection.iter().any(|x| x == id) || s.dim.picked_entity(id);
        // What a hovered or picked relation acts on lights up like a hovered entity.
        let related: BTreeSet<String> = s
            .hover_relation
            .iter()
            .chain(s.relations.iter())
            .flat_map(|c| self.relation_entities(c))
            .collect();
        let hov = |id: &str| {
            s.dim.hovered_entity(id) || s.hover.as_deref() == Some(id) || related.contains(id)
        };
        // Entities named by constraints that add nothing, and by the change that was refused.
        let mut over: BTreeSet<&str> = BTreeSet::new();
        for c in slot
            .report
            .redundant
            .iter()
            .filter_map(|c| sk.constraints().get(c))
        {
            over.extend(c.refs());
        }
        o.redundant = slot.report.redundant.len();
        o.conflict = !s.conflict.is_empty();
        let broken = s.drag.as_ref().filter(|d| d.broken.is_some());
        o.drag_warning = broken.and_then(|d| d.broken.clone());
        o.drag_note = s.drag.as_ref().and_then(|d| d.stuck.clone());
        o.modify_preview = s
            .modify_preview
            .as_ref()
            .map(|op| {
                self.modify_preview(op)
                    .into_iter()
                    .map(|l| l.into_iter().filter_map(scr).collect())
                    .collect()
            })
            .unwrap_or_default();
        o.dim_guides = s
            .dim_guides
            .iter()
            .filter_map(|[a, b]| Some([scr(*a)?, scr(*b)?]))
            .collect();
        o.select_box = s.boxsel.map(|b| (b.from, b.to, b.crossing()));
        let problem_of = |id: &str| {
            if broken.is_some_and(|d| d.point == id || d.preview.contains_key(id))
                || s.conflict.iter().any(|c| c == id)
                || s.check
                    .as_ref()
                    .is_some_and(|c| c.iter().any(|p| p.entities.iter().any(|e| e == id)))
            {
                Problem::Conflict
            } else if over.contains(id) {
                Problem::Over
            } else {
                Problem::None
            }
        };
        for (id, e) in sk.entities() {
            let (selected, defined, construction) =
                (sel(id), !free.contains(id), sk.is_construction(id));
            let hover = hov(id);
            let mut problem = problem_of(id);
            // A shape is as bad as the points it stands on.
            for p in defining_points(e) {
                problem = problem.max_problem(problem_of(p));
            }
            let stroke = |pts: Vec<P2>| Stroke {
                pts,
                selected,
                hover,
                defined,
                problem,
                construction,
            };
            match e {
                Entity::Point { .. } => {
                    // The two points an infinite line is drawn through are not shown: it has
                    // no ends. (The line itself is what you grab.)
                    if sk.only_infinite_line_ends(id) {
                        continue;
                    }
                    if let Some(p) = point(id).and_then(scr) {
                        o.points.push(Dot {
                            pos: p,
                            selected,
                            hover,
                            defined,
                            problem,
                        });
                    }
                }
                Entity::Line { a, b } => {
                    if let (Some(p), Some(q)) = (point(a).and_then(scr), point(b).and_then(scr)) {
                        // An infinite line runs off both edges of the view.
                        let (p, q) = if sk.is_infinite(id) {
                            far_ends(p, q)
                        } else {
                            (p, q)
                        };
                        o.strokes.push(stroke(vec![p, q]));
                    }
                }
                Entity::Circle { center, radius } => {
                    if let Some(c) = point(center) {
                        let pts = arc_points(c, *radius, 0.0, core::f64::consts::TAU, 72)
                            .into_iter()
                            .filter_map(scr)
                            .collect();
                        o.strokes.push(stroke(pts));
                    }
                }
                Entity::Arc { center, start, end } => {
                    if let (Some(c), Some(a), Some(b)) = (point(center), point(start), point(end)) {
                        let sweep = ccw_from(angle(c, a), angle(c, b));
                        let pts = arc_points(c, dist(c, a), angle(c, a), sweep, 48)
                            .into_iter()
                            .filter_map(scr)
                            .collect();
                        o.strokes.push(stroke(pts));
                    }
                }
            }
        }
        self.dimension_overlay(&mut o, w, h, &point);
        for d in &mut o.dims {
            if let Some(id) = &d.id {
                d.selected = s.relations.contains(id);
                d.hover = s.hover_relation.as_deref() == Some(id.as_str());
            }
        }
        self.freedom_overlay(&mut o, s, slot, w, h, &point);
        o.glyphs = self.relation_glyphs(w, h);
        if let Some(e) = &s.edit {
            let line = |p: &Vec<P2>| p.iter().filter_map(|q| scr(*q)).collect::<Vec<_>>();
            o.edit_removed = e.preview.iter().map(line).collect();
            o.edit_added = e.added.iter().map(line).collect();
            o.edit_markers = e.markers.iter().filter_map(|p| scr(*p)).collect();
            if let (Some(why), Some(at)) = (&e.problem, scr(e.at)) {
                o.tooltip = Some(((at.0, at.1), why.clone()));
            }
        }
        if let Some(t) = &s.stroke {
            o.trim_trail = t.trail.clone();
        }
        o.relation = match s.relations.as_slice() {
            [] => None,
            [one] => sk
                .constraints()
                .get(one)
                .map(crate::relations::relation_label),
            many => Some(format!("{} relations", many.len())),
        };
        if let (Some(text), Some(at)) = (crate::relations::tooltip_for(self, s), s.hover_at) {
            o.tooltip = Some((at, text));
        }
        if let Some(cur) = &s.cursor {
            let (kind, hint) = if cur.origin {
                (Marker::Origin, Some("origin"))
            } else if cur.axis.is_some() {
                (Marker::OnEdge, Some("on axis"))
            } else if cur.midpoint.is_some() {
                (Marker::Midpoint, Some("midpoint"))
            } else if cur.on_curve.is_some() {
                (Marker::Quadrant, Some("on circle"))
            } else if let Some(i) = &cur.infer {
                (Marker::Inferred, Some(i.kind.name()))
            } else if cur.point.is_some() {
                (Marker::Point, Some("coincident"))
            } else if cur.on_line.is_some() {
                (Marker::OnEdge, Some("on edge"))
            } else {
                match cur.guide {
                    Some(Guide::Horizontal) => (Marker::Horizontal, Some("horizontal")),
                    Some(Guide::Vertical) => (Marker::Vertical, Some("vertical")),
                    None => (Marker::Free, None),
                }
            };
            if s.tool.draws() || s.tool == Tool::Dimension {
                if let Some(p) = scr(cur.pos) {
                    o.marker = Some((p, kind));
                }
                o.hint = hint.map(str::to_owned);
            }
            if let Some(a) = &s.first {
                self.draw_preview(&mut o, s, a, cur.pos, &scr);
            }
        }
        Some(o)
    }

    /// Arrows for how the free points can still move, an axis highlight for a hovered or selected
    /// axis, and the tooltip that says it in words.
    fn freedom_overlay(
        &self,
        o: &mut Overlay,
        s: &SketchSession,
        slot: &dc_doc::SketchSlot,
        w: f64,
        h: f64,
        point: &dyn Fn(&str) -> Option<P2>,
    ) {
        let sk = &slot.sketch;
        let report = &slot.report;
        // Which points get marks: everything free, or only what is hovered / selected.
        let mut focus: BTreeSet<String> = BTreeSet::new();
        for id in s.selection.iter().chain(s.hover.iter()) {
            match sk.entity(id) {
                Some(Entity::Point { .. }) => {
                    focus.insert(id.clone());
                }
                Some(e) => focus.extend(defining_points(e).into_iter().cloned()),
                None => {}
            }
        }
        for (id, m) in &report.motion {
            if !s.show_freedom && !focus.contains(id) {
                continue;
            }
            if s.tool == Tool::Dimension {
                continue;
            }
            let Some(p) = point(id) else { continue };
            let Some(at) = self.sketch_to_screen(p, w, h) else {
                continue;
            };
            let dir_screen = m.direction.and_then(|d| {
                let tip = self.sketch_to_screen((p.0 + d[0], p.1 + d[1]), w, h)?;
                let v = (tip.0 - at.0, tip.1 - at.1);
                let len = det::hypot(v.0, v.1);
                (len > 1e-12).then_some((v.0 / len, v.1 / len))
            });
            o.freedom.push(FreedomMark {
                pos: at,
                dof: m.dof,
                dir: dir_screen,
                radius: false,
                sweep: false,
                emphasis: focus.contains(id),
            });
        }
        for id in &report.radius_free {
            if !s.show_freedom && !s.selection.contains(id) && s.hover.as_deref() != Some(id) {
                continue;
            }
            if let Some(Entity::Circle { center, radius }) = sk.entity(id) {
                let Some(c) = point(center) else { continue };
                let (Some(at), Some(out)) = (
                    self.sketch_to_screen((c.0 + radius, c.1), w, h),
                    self.sketch_to_screen((c.0 + radius + 1.0, c.1), w, h),
                ) else {
                    continue;
                };
                let d = (out.0 - at.0, out.1 - at.1);
                let len = det::hypot(d.0, d.1).max(1e-12);
                o.freedom.push(FreedomMark {
                    pos: at,
                    dof: 1,
                    dir: Some((d.0 / len, d.1 / len)),
                    radius: true,
                    sweep: false,
                    emphasis: s.selection.contains(id) || s.hover.as_deref() == Some(id),
                });
            }
        }
        // Whole lines and arcs: shown for the hovered or selected one only, so a sketch does
        // not fill up with arrows. A line gets none (its end points show their own, and its
        // middle belongs to the midpoint handle; the tooltip still says how free it is); an arc
        // shows whether its radius and its sweep (how far round it goes) are still free.
        let chosen = |id: &String| s.selection.contains(id) || s.hover.as_deref() == Some(id);
        for id in sk
            .entities()
            .iter()
            .filter(|(id, e)| matches!(e, Entity::Arc { .. }) && chosen(id))
            .map(|(id, _)| id)
        {
            let Some(Entity::Arc { center, start, end }) = sk.entity(id) else {
                continue;
            };
            let (Some(c), Some(st), Some(en)) = (point(center), point(start), point(end)) else {
                continue;
            };
            let (r, a0, sw) = (
                dist(c, st),
                angle(c, st),
                ccw_from(angle(c, st), angle(c, en)),
            );
            let mid_angle = a0 + sw / 2.0;
            let rim = (c.0 + r * det::cos(mid_angle), c.1 + r * det::sin(mid_angle));
            let (Some(at), Some(out), Some(next)) = (
                self.sketch_to_screen(rim, w, h),
                self.sketch_to_screen(
                    (
                        c.0 + (r + 1.0) * det::cos(mid_angle),
                        c.1 + (r + 1.0) * det::sin(mid_angle),
                    ),
                    w,
                    h,
                ),
                self.sketch_to_screen(
                    (rim.0 - det::sin(mid_angle), rim.1 + det::cos(mid_angle)),
                    w,
                    h,
                ),
            ) else {
                continue;
            };
            let unit = |a: P2, b: P2| {
                let v = (b.0 - a.0, b.1 - a.1);
                let l = det::hypot(v.0, v.1).max(1e-12);
                (v.0 / l, v.1 / l)
            };
            if report.radius_free.iter().any(|x| x == id) {
                o.freedom.push(FreedomMark {
                    pos: at,
                    dof: 1,
                    dir: Some(unit(at, out)),
                    radius: true,
                    sweep: false,
                    emphasis: true,
                });
            }
            if report.sweep_free.iter().any(|x| x == id) {
                o.freedom.push(FreedomMark {
                    pos: at,
                    dof: 1,
                    dir: Some(unit(at, next)),
                    radius: false,
                    sweep: true,
                    emphasis: true,
                });
            }
        }
        // Midpoint handles: a line shows its middle once it is selected.
        if s.tool == Tool::Select {
            // A selected line shows its middle only while the pointer is close to it (as it is
            // being used); a middle that is itself selected stays.
            let mut shown: Vec<&str> = s
                .selection
                .iter()
                .map(String::as_str)
                .filter(|id| matches!(sk.entity(id), Some(Entity::Line { .. })))
                .filter(|id| !sk.is_infinite(id))
                .filter(|id| {
                    let (Some(at), Some(Entity::Line { a, b })) = (s.hover_at, sk.entity(id))
                    else {
                        return false;
                    };
                    let (Some(p), Some(q)) = (sk.point(a), sk.point(b)) else {
                        return false;
                    };
                    self.sketch_to_screen(((p.0 + q.0) / 2.0, (p.1 + q.1) / 2.0), w, h)
                        .is_some_and(|m| dist(m, at) <= MID_REVEAL_PX)
                })
                .collect();
            for sel_mid in s
                .selection
                .iter()
                .filter_map(|i| i.strip_prefix(MID_PREFIX))
            {
                if !shown.contains(&sel_mid) {
                    shown.push(sel_mid);
                }
            }
            for id in shown {
                if let Some(Entity::Line { a, b }) = sk.entity(id) {
                    if let (Some(p), Some(q)) = (sk.point(a), sk.point(b)) {
                        if let Some(m) =
                            self.sketch_to_screen(((p.0 + q.0) / 2.0, (p.1 + q.1) / 2.0), w, h)
                        {
                            let hot = s.hover.as_deref().and_then(|h| h.strip_prefix(MID_PREFIX))
                                == Some(id)
                                || s.selection
                                    .iter()
                                    .any(|x| x.strip_prefix(MID_PREFIX) == Some(id));
                            o.mid_handles.push((m, hot));
                        }
                    }
                }
            }
        }
        // Selected and hovered axes / origin.
        if s.tool == Tool::Select {
            let mark = |id: &str| match id {
                SEL_ORIGIN => Some(AxisMark::Origin),
                SEL_AXIS_X => Some(AxisMark::X),
                SEL_AXIS_Y => Some(AxisMark::Y),
                _ => None,
            };
            for id in &s.selection {
                if let Some(a) = mark(id).and_then(|m| self.axis_draw(m, true, w, h)) {
                    o.axes.push(a);
                }
            }
            if let Some(id) = s
                .hover
                .as_deref()
                .filter(|h| !s.selection.iter().any(|x| x == h))
            {
                if let Some(a) = mark(id).and_then(|m| self.axis_draw(m, false, w, h)) {
                    o.axes.push(a);
                }
            }
            if let (Some(id), Some(at)) = (s.hover.as_deref(), s.hover_at) {
                let pinned = |p: &str| !report.motion.contains_key(p);
                let hint = match sk.entity(id) {
                    Some(Entity::Circle { center, .. }) => {
                        if report.radius_free.iter().any(|c| c == id) {
                            "  ·  drag to resize"
                        } else if pinned(center) {
                            "  ·  fully defined"
                        } else {
                            "  ·  radius is fixed (drag its centre to move it)"
                        }
                    }
                    Some(e) => {
                        let all_pinned = match e {
                            Entity::Point { .. } => pinned(id),
                            _ => defining_points(e).iter().all(|p| pinned(p)),
                        };
                        if all_pinned {
                            "  ·  fully defined, cannot be dragged"
                        } else {
                            "  ·  drag to move"
                        }
                    }
                    None => "",
                };
                o.tooltip = Some((at, format!("{}{hint}", self.describe(id, report))));
            }
        }
    }

    /// One line on what an entity is and how free it still is.
    fn describe(&self, id: &str, report: &dc_sketch::SolveReport) -> String {
        match id {
            SEL_ORIGIN => return "Origin".into(),
            SEL_AXIS_X => return "Horizontal axis".into(),
            SEL_AXIS_Y => return "Vertical axis".into(),
            _ => {}
        }
        if let Some(line) = id.strip_prefix(MID_PREFIX) {
            return format!("Midpoint of {line}  ·  click to select it, relate it to other shapes");
        }
        let Some(sk) = self.sketch_data() else {
            return id.into();
        };
        let move_text = |p: &str| match report.motion.get(p) {
            None => "pinned".to_owned(),
            Some(m) if m.dof >= 2 => "can move any way".to_owned(),
            Some(m) => match m.direction {
                Some([x, _]) if x.abs() < 1e-6 => "can move vertically only".to_owned(),
                Some([_, y]) if y.abs() < 1e-6 => "can move horizontally only".to_owned(),
                Some([x, y]) => format!(
                    "can move along {}° only",
                    fmt_num(det::atan2(y, x).to_degrees())
                ),
                None => "can move".to_owned(),
            },
        };
        match sk.entity(id) {
            Some(Entity::Point { .. }) => format!("Point {id}: {}", move_text(id)),
            Some(e) => {
                let kind = match e {
                    Entity::Line { .. } => "Line",
                    Entity::Circle { .. } => "Circle",
                    _ => "Arc",
                };
                let free = defining_points(e)
                    .into_iter()
                    .filter(|p| report.motion.contains_key(p.as_str()))
                    .count();
                let radius = report.radius_free.iter().any(|c| c == id);
                if free == 0 && !radius && !report.sweep_free.iter().any(|c| c == id) {
                    format!("{kind} {id}: fully defined")
                } else {
                    let mut parts = Vec::new();
                    if free > 0 {
                        parts.push(format!(
                            "{free} point{} free",
                            if free == 1 { "" } else { "s" }
                        ));
                    }
                    if radius {
                        parts.push("radius free".into());
                    }
                    if report.sweep_free.iter().any(|c| c == id) {
                        parts.push("sweep free".into());
                    }
                    format!("{kind} {id}: {}", parts.join(", "))
                }
            }
            None => id.into(),
        }
    }

    fn draw_preview(
        &self,
        o: &mut Overlay,
        s: &SketchSession,
        a: &Anchor,
        cur: P2,
        scr: &dyn Fn(P2) -> Option<P2>,
    ) {
        let line = |pts: Vec<P2>| pts.into_iter().filter_map(scr).collect::<Vec<_>>();
        let (x0, y0) = a.pos;
        let (x1, y1) = cur;
        let tag = |o: &mut Overlay, at: P2, text: String| {
            if let Some(m) = scr(at) {
                o.labels.push((m, text));
            }
        };
        match s.tool {
            Tool::Rectangle => {
                o.preview
                    .push(line(vec![(x0, y0), (x1, y0), (x1, y1), (x0, y1), (x0, y0)]));
                tag(
                    o,
                    ((x0 + x1) / 2.0, y1),
                    format!(
                        "{} × {}",
                        fmt_num((x1 - x0).abs()),
                        fmt_num((y1 - y0).abs())
                    ),
                );
            }
            Tool::CenterRectangle => {
                let (dx, dy) = ((x1 - x0).abs(), (y1 - y0).abs());
                o.preview.push(line(vec![
                    (x0 - dx, y0 - dy),
                    (x0 + dx, y0 - dy),
                    (x0 + dx, y0 + dy),
                    (x0 - dx, y0 + dy),
                    (x0 - dx, y0 - dy),
                ]));
                o.preview
                    .push(line(vec![(x0 - dx, y0 - dy), (x0 + dx, y0 + dy)]));
                o.preview
                    .push(line(vec![(x0 + dx, y0 - dy), (x0 - dx, y0 + dy)]));
                tag(
                    o,
                    (x0, y0 + dy),
                    format!("{} × {}", fmt_num(2.0 * dx), fmt_num(2.0 * dy)),
                );
            }
            Tool::Circle => {
                let r = dist(a.pos, cur);
                o.preview
                    .push(line(arc_points(a.pos, r, 0.0, core::f64::consts::TAU, 72)));
                o.preview.push(line(vec![(x0, y0), (x1, y1)]));
                tag(o, cur, format!("Ø{}", fmt_num(2.0 * r)));
            }
            Tool::Line | Tool::Centerline if s.midpoint_line => {
                let far = (2.0 * x0 - x1, 2.0 * y0 - y1);
                let (pf, pe) = if s.tool == Tool::Centerline && s.centerline_infinite {
                    far_ends(far, (x1, y1))
                } else {
                    (far, (x1, y1))
                };
                o.preview.push(line(vec![pf, pe]));
                o.mid_handles.push(((x0, y0), true));
                tag(o, cur, fmt_num(2.0 * dist(a.pos, cur)));
            }
            Tool::Line | Tool::Centerline => {
                let (pa, pb) = if s.tool == Tool::Centerline && s.centerline_infinite {
                    far_ends((x0, y0), (x1, y1))
                } else {
                    ((x0, y0), (x1, y1))
                };
                o.preview.push(line(vec![pa, pb]));
                tag(
                    o,
                    ((x0 + x1) / 2.0, (y0 + y1) / 2.0),
                    fmt_num(dist(a.pos, cur)),
                );
            }
            Tool::Arc => match &s.second {
                None => {
                    o.preview.push(line(vec![(x0, y0), (x1, y1)]));
                    tag(o, cur, format!("R{}", fmt_num(dist(a.pos, cur))));
                }
                Some(st) => {
                    if let Some((start, _, sweep)) = arc_from_centre(a.pos, st.pos, cur, s.arc_ccw)
                    {
                        let r = dist(a.pos, st.pos);
                        o.preview
                            .push(line(arc_points(a.pos, r, angle(a.pos, start), sweep, 48)));
                        o.preview.push(line(vec![a.pos, st.pos]));
                        tag(o, cur, format!("{}°", fmt_num(sweep.to_degrees().round())));
                    }
                }
            },
            Tool::Arc3 => match &s.second {
                None => o.preview.push(line(vec![(x0, y0), (x1, y1)])),
                Some(b) => {
                    if let Some((c, r, a_is_start)) = arc_through(a.pos, b.pos, cur) {
                        let (st, en) = if a_is_start {
                            (a.pos, b.pos)
                        } else {
                            (b.pos, a.pos)
                        };
                        o.preview.push(line(arc_points(
                            c,
                            r,
                            angle(c, st),
                            ccw_from(angle(c, st), angle(c, en)),
                            48,
                        )));
                        tag(o, cur, format!("R{}", fmt_num(r)));
                    } else {
                        o.preview.push(line(vec![a.pos, b.pos]));
                    }
                }
            },
            t if t.draws() => {
                let (lines, label) = self.new_shape_preview(t, a, s.second.as_ref(), cur);
                for l in lines {
                    o.preview.push(l.into_iter().filter_map(scr).collect());
                }
                if let Some(text) = label {
                    tag(o, cur, text);
                }
            }
            _ => {}
        }
    }
}

/// Start point, end point and sweep (radians, counter-clockwise from the returned start) of a
/// centre arc: it starts at `start` and ends where the cursor points. `ccw` is the direction the
/// cursor moved first; `None` (not moved yet) is counter-clockwise.
fn arc_from_centre(c: P2, start: P2, towards: P2, ccw: Option<bool>) -> Option<(P2, P2, f64)> {
    let r = dist(c, start);
    if r < 1e-9 || dist(c, towards) < 1e-9 {
        return None;
    }
    let a = angle(c, towards);
    let end = (c.0 + r * det::cos(a), c.1 + r * det::sin(a));
    let sa = angle(c, start);
    if ccw.unwrap_or(true) {
        let sweep = ccw_from(sa, a);
        (sweep > 1e-9).then_some((start, end, sweep))
    } else {
        // Clockwise from `start` is counter-clockwise from the end back to `start`.
        let sweep = ccw_from(a, sa);
        (sweep > 1e-9).then_some((end, start, sweep))
    }
}

/// Centre, radius and whether `a` is the arc's counter-clockwise start, for the arc from `a` to
/// `b` that passes through `via`.
fn arc_through(a: P2, b: P2, via: P2) -> Option<(P2, f64, bool)> {
    let (c, r) = circumcircle(a, b, via)?;
    let (aa, am) = (angle(c, a), angle(c, via));
    let sweep_to_b = ccw_from(aa, angle(c, b));
    let via_off = ccw_from(aa, am);
    Some((c, r, via_off < sweep_to_b))
}

fn arc_points(c: P2, r: f64, start: f64, sweep: f64, n: usize) -> Vec<P2> {
    (0..=n)
        .map(|i| {
            let t = start + sweep * (i as f64) / (n as f64);
            (c.0 + r * det::cos(t), c.1 + r * det::sin(t))
        })
        .collect()
}

/// The nearest point to `p` on the whole line through `a` and `b`.
fn closest_on_line(p: P2, a: P2, b: P2) -> P2 {
    let (dx, dy) = (b.0 - a.0, b.1 - a.1);
    let len2 = dx * dx + dy * dy;
    if len2 < 1e-18 {
        return a;
    }
    let t = ((p.0 - a.0) * dx + (p.1 - a.1) * dy) / len2;
    (a.0 + t * dx, a.1 + t * dy)
}

fn closest_on_segment(p: P2, a: P2, b: P2) -> P2 {
    let (dx, dy) = (b.0 - a.0, b.1 - a.1);
    let len2 = dx * dx + dy * dy;
    let t = if len2 < 1e-18 {
        0.0
    } else {
        (((p.0 - a.0) * dx + (p.1 - a.1) * dy) / len2).clamp(0.0, 1.0)
    };
    (a.0 + t * dx, a.1 + t * dy)
}

pub(crate) fn segment_distance(p: P2, a: P2, b: P2) -> f64 {
    dist(p, closest_on_segment(p, a, b))
}
