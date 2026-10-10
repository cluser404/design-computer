use dc_math::{Plane, Vec3};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};
use std::fmt;

/// Geometry in a sketch. Lines and arcs reference *points* by ID, so moving a shared point moves
/// everything attached to it (this is what makes "coincident" structural and cheap).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Entity {
    Point {
        x: f64,
        y: f64,
    },
    Line {
        a: String,
        b: String,
    },
    Circle {
        center: String,
        radius: f64,
    },
    /// Counter-clockwise from `start` to `end` around `center`.
    Arc {
        center: String,
        start: String,
        end: String,
    },
}

/// One of the two axes through the sketch origin.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum SketchAxis {
    X,
    Y,
}

/// Geometric and dimensional constraints. Values are in millimetres.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Constraint {
    Coincident {
        a: String,
        b: String,
    },
    Horizontal {
        line: String,
    },
    Vertical {
        line: String,
    },
    Parallel {
        a: String,
        b: String,
    },
    Perpendicular {
        a: String,
        b: String,
    },
    EqualLength {
        a: String,
        b: String,
    },
    /// Line length.
    Length {
        line: String,
        value: f64,
    },
    /// Distance between two points.
    Distance {
        a: String,
        b: String,
        value: f64,
    },
    /// Horizontal distance from `a` to `b`: `b.x - a.x = value`. The sign records which way round
    /// the points are; the dimension's magnitude is what `value()` reports and what is edited.
    HorizontalDistance {
        a: String,
        b: String,
        value: f64,
    },
    /// Vertical distance from `a` to `b`: `b.y - a.y = value` (sign as above).
    VerticalDistance {
        a: String,
        b: String,
        value: f64,
    },
    /// Perpendicular distance from a point to the infinite line through a line entity. Positive on
    /// the line's left (looking from its first point to its second), negative on its right.
    PointLineDistance {
        point: String,
        line: String,
        value: f64,
    },
    /// Circle radius, or arc radius (center to start).
    Radius {
        target: String,
        value: f64,
    },
    /// Pin a point at fixed coordinates (an anchor).
    Fixed {
        point: String,
        x: f64,
        y: f64,
    },
    PointOnLine {
        point: String,
        line: String,
    },
    /// A point on one of the sketch's own axes: `x` is the horizontal axis (y = 0), `y` the
    /// vertical one (x = 0). Needs no origin point.
    OnAxis {
        point: String,
        axis: SketchAxis,
    },
    /// A point at the middle of a line.
    Midpoint {
        point: String,
        line: String,
    },
    /// Two points that mirror each other across a line.
    Symmetric {
        a: String,
        b: String,
        line: String,
    },
    /// Two circles / arcs with the same radius.
    EqualRadius {
        a: String,
        b: String,
    },
    /// A point on a circle, or on the full circle an arc is part of.
    PointOnCurve {
        point: String,
        curve: String,
    },
    /// The angle in degrees from line `a` to line `b` (counter-clockwise positive; the sign
    /// records which way round it was drawn). The lines are directed a -> b, so this is the
    /// angle between the lines as drawn, not an undirected one.
    Angle {
        a: String,
        b: String,
        value: f64,
    },
    /// The length of an arc measured along it, in millimetres.
    ArcLength {
        arc: String,
        value: f64,
    },
    /// Two points that mirror each other across one of the sketch's own axes.
    SymmetricAxis {
        a: String,
        b: String,
        axis: SketchAxis,
    },
    /// Two circles / arcs touching each other (outside each other, or one inside the other,
    /// whichever they are now).
    TangentCurves {
        a: String,
        b: String,
    },
    /// Line tangent to a circle or arc.
    Tangent {
        line: String,
        curve: String,
    },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum Kind {
    Point,
    Line,
    Circle,
    Arc,
}

fn canon(v: f64) -> f64 {
    // -0.0 and 0.0 are the same number but different bytes; store one spelling.
    if v == 0.0 {
        0.0
    } else {
        v
    }
}

impl Entity {
    /// Same entity with `-0.0` replaced by `0.0`, so equal geometry serializes identically.
    pub fn canonical(self) -> Entity {
        match self {
            Entity::Point { x, y } => Entity::Point {
                x: canon(x),
                y: canon(y),
            },
            Entity::Circle { center, radius } => Entity::Circle {
                center,
                radius: canon(radius),
            },
            other => other,
        }
    }

    pub fn kind(&self) -> Kind {
        match self {
            Entity::Point { .. } => Kind::Point,
            Entity::Line { .. } => Kind::Line,
            Entity::Circle { .. } => Kind::Circle,
            Entity::Arc { .. } => Kind::Arc,
        }
    }

    /// IDs of the entities this one is built from.
    pub fn refs(&self) -> Vec<&str> {
        match self {
            Entity::Point { .. } => vec![],
            Entity::Line { a, b } => vec![a, b],
            Entity::Circle { center, .. } => vec![center],
            Entity::Arc { center, start, end } => vec![center, start, end],
        }
    }
}

impl Constraint {
    /// Same constraint with `-0.0` replaced by `0.0`.
    pub fn canonical(self) -> Constraint {
        match self {
            Constraint::Length { line, value } => Constraint::Length {
                line,
                value: canon(value),
            },
            Constraint::Distance { a, b, value } => Constraint::Distance {
                a,
                b,
                value: canon(value),
            },
            Constraint::Radius { target, value } => Constraint::Radius {
                target,
                value: canon(value),
            },
            Constraint::HorizontalDistance { a, b, value } => Constraint::HorizontalDistance {
                a,
                b,
                value: canon(value),
            },
            Constraint::VerticalDistance { a, b, value } => Constraint::VerticalDistance {
                a,
                b,
                value: canon(value),
            },
            Constraint::PointLineDistance { point, line, value } => Constraint::PointLineDistance {
                point,
                line,
                value: canon(value),
            },
            Constraint::Angle { a, b, value } => Constraint::Angle {
                a,
                b,
                value: canon(value),
            },
            Constraint::ArcLength { arc, value } => Constraint::ArcLength {
                arc,
                value: canon(value),
            },
            Constraint::Fixed { point, x, y } => Constraint::Fixed {
                point,
                x: canon(x),
                y: canon(y),
            },
            other => other,
        }
    }

    /// The same constraint with every reference to `from` pointing at `to` instead.
    pub(crate) fn renamed(mut self, from: &str, to: &str) -> Constraint {
        let fix = |s: &mut String| {
            if s == from {
                *s = to.to_owned();
            }
        };
        match &mut self {
            Constraint::Coincident { a, b }
            | Constraint::Parallel { a, b }
            | Constraint::Perpendicular { a, b }
            | Constraint::EqualLength { a, b }
            | Constraint::EqualRadius { a, b }
            | Constraint::TangentCurves { a, b }
            | Constraint::SymmetricAxis { a, b, .. }
            | Constraint::Angle { a, b, .. }
            | Constraint::Distance { a, b, .. }
            | Constraint::HorizontalDistance { a, b, .. }
            | Constraint::VerticalDistance { a, b, .. } => {
                fix(a);
                fix(b);
            }
            Constraint::Horizontal { line }
            | Constraint::Vertical { line }
            | Constraint::Length { line, .. } => fix(line),
            Constraint::Radius { target, .. } => fix(target),
            Constraint::ArcLength { arc, .. } => fix(arc),
            Constraint::Fixed { point, .. } | Constraint::OnAxis { point, .. } => fix(point),
            Constraint::PointOnLine { point, line }
            | Constraint::Midpoint { point, line }
            | Constraint::PointLineDistance { point, line, .. } => {
                fix(point);
                fix(line);
            }
            Constraint::Symmetric { a, b, line } => {
                fix(a);
                fix(b);
                fix(line);
            }
            Constraint::PointOnCurve { point, curve } => {
                fix(point);
                fix(curve);
            }
            Constraint::Tangent { line, curve } => {
                fix(line);
                fix(curve);
            }
        }
        self
    }

    /// The dimension value, for constraints that have one (length, distance, radius).
    pub fn value(&self) -> Option<f64> {
        match self {
            Constraint::Length { value, .. }
            | Constraint::Distance { value, .. }
            | Constraint::ArcLength { value, .. }
            | Constraint::Radius { value, .. } => Some(*value),
            // Signed internally (which side); the dimension itself is the magnitude.
            Constraint::HorizontalDistance { value, .. }
            | Constraint::VerticalDistance { value, .. }
            | Constraint::PointLineDistance { value, .. }
            | Constraint::Angle { value, .. } => Some(value.abs()),
            _ => None,
        }
    }

    /// IDs of the entities this constraint acts on.
    pub fn refs(&self) -> Vec<&str> {
        match self {
            Constraint::Coincident { a, b }
            | Constraint::Parallel { a, b }
            | Constraint::Perpendicular { a, b }
            | Constraint::EqualLength { a, b }
            | Constraint::EqualRadius { a, b }
            | Constraint::TangentCurves { a, b }
            | Constraint::SymmetricAxis { a, b, .. }
            | Constraint::Angle { a, b, .. }
            | Constraint::Distance { a, b, .. }
            | Constraint::HorizontalDistance { a, b, .. }
            | Constraint::VerticalDistance { a, b, .. } => vec![a, b],
            Constraint::Horizontal { line }
            | Constraint::Vertical { line }
            | Constraint::Length { line, .. } => vec![line],
            Constraint::Radius { target, .. } => vec![target],
            Constraint::ArcLength { arc, .. } => vec![arc],
            Constraint::Fixed { point, .. } | Constraint::OnAxis { point, .. } => vec![point],
            Constraint::PointOnLine { point, line }
            | Constraint::Midpoint { point, line }
            | Constraint::PointLineDistance { point, line, .. } => vec![point, line],
            Constraint::Symmetric { a, b, line } => vec![a, b, line],
            Constraint::PointOnCurve { point, curve } => vec![point, curve],
            Constraint::Tangent { line, curve } => vec![line, curve],
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum SketchErrorCode {
    InvalidId,
    InvalidValue,
    AlreadyExists,
    NotFound,
    WrongKind,
    InUse,
}

/// A sketch edit that was refused. `entities` lists the IDs involved.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct SketchError {
    pub code: SketchErrorCode,
    pub message: String,
    pub entities: Vec<String>,
}

impl SketchError {
    fn new(code: SketchErrorCode, message: impl Into<String>, entities: Vec<String>) -> Self {
        Self {
            code,
            message: message.into(),
            entities,
        }
    }
}

impl fmt::Display for SketchError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.message)
    }
}

impl std::error::Error for SketchError {}

fn valid_id(id: &str) -> Result<(), SketchError> {
    let ok = !id.is_empty()
        && id.len() <= 128
        && id
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '.' | '-'));
    if ok {
        Ok(())
    } else {
        Err(SketchError::new(
            SketchErrorCode::InvalidId,
            format!("invalid id {id:?}: use 1-128 characters from [A-Za-z0-9_.-]"),
            vec![],
        ))
    }
}

fn finite(name: &str, v: f64) -> Result<(), SketchError> {
    if v.is_finite() {
        Ok(())
    } else {
        Err(SketchError::new(
            SketchErrorCode::InvalidValue,
            format!("{name} must be finite, got {v}"),
            vec![],
        ))
    }
}

fn positive(name: &str, v: f64) -> Result<(), SketchError> {
    finite(name, v)?;
    if v > 0.0 {
        Ok(())
    } else {
        Err(SketchError::new(
            SketchErrorCode::InvalidValue,
            format!("{name} must be greater than zero, got {v}"),
            vec![],
        ))
    }
}

/// Where a sketch lives in 3D. Stored as *intent* (a named base plane and an offset), not as raw
/// vectors, so it stays meaningful and diff-friendly. Planes on solid faces come later and need
/// persistent face naming.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "base", rename_all = "snake_case")]
pub enum PlaneSpec {
    /// On a flat face of a feature's solid, identified by the face's stable name (see
    /// `feature.faces`, e.g. `cap_end`, `cap_end:boss1`, `side:e0`). The sketch follows the face
    /// when the part changes. The plane's normal is the face's outward normal; the origin and x
    /// axis come from the face. `offset` moves the plane along that normal.
    Face {
        feature: String,
        face: String,
        #[serde(default)]
        offset: f64,
    },
    /// x = X, y = Y, normal = +Z
    Xy {
        #[serde(default)]
        offset: f64,
    },
    /// x = X, y = Z, normal = -Y
    Xz {
        #[serde(default)]
        offset: f64,
    },
    /// x = Y, y = Z, normal = +X
    Yz {
        #[serde(default)]
        offset: f64,
    },
}

impl Default for PlaneSpec {
    fn default() -> Self {
        PlaneSpec::Xy { offset: 0.0 }
    }
}

impl PlaneSpec {
    pub fn offset(&self) -> f64 {
        match self {
            PlaneSpec::Xy { offset }
            | PlaneSpec::Xz { offset }
            | PlaneSpec::Yz { offset }
            | PlaneSpec::Face { offset, .. } => *offset,
        }
    }

    /// The concrete plane for the three world planes. Offset is measured along the normal.
    /// `None` for a face-attached plane, which can only be resolved against a document.
    pub fn named_plane(&self) -> Option<Plane> {
        let (x, y) = match self {
            PlaneSpec::Xy { .. } => (Vec3::X, Vec3::Y),
            PlaneSpec::Xz { .. } => (Vec3::X, Vec3::Z),
            PlaneSpec::Yz { .. } => (Vec3::Y, Vec3::Z),
            PlaneSpec::Face { .. } => return None,
        };
        Some(
            Plane {
                origin: Vec3::ZERO,
                x_axis: x,
                y_axis: y,
            }
            .offset(self.offset()),
        )
    }

    /// The feature this plane is attached to, if any.
    pub fn attached_feature(&self) -> Option<&str> {
        match self {
            PlaneSpec::Face { feature, .. } => Some(feature),
            _ => None,
        }
    }

    pub fn canonical(self) -> PlaneSpec {
        match self {
            PlaneSpec::Face {
                feature,
                face,
                offset,
            } => PlaneSpec::Face {
                feature,
                face,
                offset: canon(offset),
            },
            PlaneSpec::Xy { offset } => PlaneSpec::Xy {
                offset: canon(offset),
            },
            PlaneSpec::Xz { offset } => PlaneSpec::Xz {
                offset: canon(offset),
            },
            PlaneSpec::Yz { offset } => PlaneSpec::Yz {
                offset: canon(offset),
            },
        }
    }

    pub fn validate(&self) -> Result<(), SketchError> {
        finite("plane offset", self.offset())
    }
}

/// A 2D sketch. Maps are ordered so iteration (and therefore solving and serialization) is
/// deterministic.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct Sketch {
    #[serde(default)]
    pub(crate) plane: PlaneSpec,
    pub(crate) entities: BTreeMap<String, Entity>,
    pub(crate) constraints: BTreeMap<String, Constraint>,
    /// Lines, circles and arcs that guide the design but are not part of any profile (centre
    /// lines, diagonals). They still take part in constraints. Omitted from the saved form when
    /// empty, so documents without any hash exactly as before.
    #[serde(default, skip_serializing_if = "BTreeSet::is_empty")]
    pub(crate) construction: BTreeSet<String>,
    /// Construction lines that run on for ever in both directions (SolidWorks' "infinite
    /// length"): they have no ends, so only where they are placed has to be defined. Only
    /// construction lines can be infinite. Omitted when empty.
    #[serde(default, skip_serializing_if = "BTreeSet::is_empty")]
    pub(crate) infinite: BTreeSet<String>,
    /// Where each dimension's text / line was put (sketch coordinates). Presentation, but part
    /// of the design: it is saved, undoable, and the same for everyone looking at the sketch.
    /// Omitted when empty, so documents without any hash exactly as before.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub(crate) placements: BTreeMap<String, (f64, f64)>,
    /// Dimensions that only report a measurement (reference dimensions): they do not constrain
    /// anything, and show the value the geometry has. Omitted when empty.
    #[serde(default, skip_serializing_if = "BTreeSet::is_empty")]
    pub(crate) driven: BTreeSet<String>,
    /// How each dimension is written: decimals and tolerance. Presentation only. Omitted when empty.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub(crate) styles: BTreeMap<String, DimStyle>,
}

/// How a dimension is written (presentation only; it never changes the geometry).
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct DimStyle {
    /// Digits after the decimal point; `None` writes as many as are needed (up to two).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub decimals: Option<u8>,
    /// Plus and minus tolerance in the dimension's own unit; written beside the value when set.
    #[serde(default, skip_serializing_if = "is_zero")]
    pub tol_plus: f64,
    #[serde(default, skip_serializing_if = "is_zero")]
    pub tol_minus: f64,
}

fn is_zero(v: &f64) -> bool {
    *v == 0.0
}

impl Sketch {
    pub fn new() -> Self {
        Self::default()
    }

    /// A new empty sketch on the given plane.
    pub fn on_plane(plane: PlaneSpec) -> Result<Self, SketchError> {
        plane.validate()?;
        Ok(Self {
            plane: plane.canonical(),
            ..Self::default()
        })
    }

    pub fn plane_spec(&self) -> PlaneSpec {
        self.plane.clone()
    }

    /// The 3D plane for a sketch on a world plane; `None` if it is attached to a face (resolve it
    /// with the document, which knows the solids).
    pub fn named_plane(&self) -> Option<Plane> {
        self.plane.named_plane()
    }

    pub fn entities(&self) -> &BTreeMap<String, Entity> {
        &self.entities
    }

    /// IDs of the entities marked as construction geometry.
    pub fn construction(&self) -> &BTreeSet<String> {
        &self.construction
    }

    /// Where a dimension was placed, if it was.
    pub fn placement(&self, id: &str) -> Option<(f64, f64)> {
        self.placements.get(id).copied()
    }

    pub fn placements(&self) -> &BTreeMap<String, (f64, f64)> {
        &self.placements
    }

    /// Records where a dimension is drawn. Only dimensions (constraints with a value) have one.
    pub fn set_placement(&mut self, id: &str, x: f64, y: f64) -> Result<(), SketchError> {
        finite("x", x)?;
        finite("y", y)?;
        match self.constraints.get(id) {
            None => Err(SketchError::new(
                SketchErrorCode::NotFound,
                format!("constraint '{id}' does not exist"),
                vec![id.to_owned()],
            )),
            Some(c) if c.value().is_none() => Err(SketchError::new(
                SketchErrorCode::WrongKind,
                format!("'{id}' is a relation, not a dimension, so it has no place to draw"),
                vec![id.to_owned()],
            )),
            Some(_) => {
                // Where something is drawn needs no more than a micrometre; rounding keeps clicks'
                // floating-point noise out of the file.
                let round = |v: f64| canon((v * 1e6).round() / 1e6);
                self.placements.insert(id.to_owned(), (round(x), round(y)));
                Ok(())
            }
        }
    }

    /// Forgets presentation (place, driven flag, style) of dimensions that no longer exist.
    pub fn prune_placements(&mut self) {
        let alive = &self.constraints;
        self.placements.retain(|id, _| alive.contains_key(id));
        self.driven.retain(|id| alive.contains_key(id));
        self.styles.retain(|id, _| alive.contains_key(id));
    }

    /// Is this dimension only a measurement (it does not constrain)?
    pub fn is_driven(&self, id: &str) -> bool {
        self.driven.contains(id)
    }

    pub fn driven(&self) -> &BTreeSet<String> {
        &self.driven
    }

    /// Makes a dimension a reference (it follows the geometry) or a driving one again.
    pub fn set_driven(&mut self, id: &str, on: bool) -> Result<(), SketchError> {
        match self.constraints.get(id) {
            None => Err(SketchError::new(
                SketchErrorCode::NotFound,
                format!("constraint '{id}' does not exist"),
                vec![id.to_owned()],
            )),
            Some(c) if c.value().is_none() => Err(SketchError::new(
                SketchErrorCode::WrongKind,
                format!("'{id}' is a relation, not a dimension, so it cannot be a reference"),
                vec![id.to_owned()],
            )),
            Some(_) => {
                if on {
                    self.driven.insert(id.to_owned());
                } else {
                    self.driven.remove(id);
                }
                Ok(())
            }
        }
    }

    pub fn style(&self, id: &str) -> Option<&DimStyle> {
        self.styles.get(id)
    }

    pub fn styles(&self) -> &BTreeMap<String, DimStyle> {
        &self.styles
    }

    /// Sets how a dimension is written. A default style removes the entry.
    pub fn set_style(&mut self, id: &str, style: DimStyle) -> Result<(), SketchError> {
        finite("tol_plus", style.tol_plus)?;
        finite("tol_minus", style.tol_minus)?;
        if style.decimals.is_some_and(|d| d > 8) {
            return Err(SketchError::new(
                SketchErrorCode::InvalidValue,
                "at most 8 decimals",
                vec![id.to_owned()],
            ));
        }
        match self.constraints.get(id) {
            None => Err(SketchError::new(
                SketchErrorCode::NotFound,
                format!("constraint '{id}' does not exist"),
                vec![id.to_owned()],
            )),
            Some(c) if c.value().is_none() => Err(SketchError::new(
                SketchErrorCode::WrongKind,
                format!("'{id}' is a relation, not a dimension"),
                vec![id.to_owned()],
            )),
            Some(_) => {
                if style == DimStyle::default() {
                    self.styles.remove(id);
                } else {
                    self.styles.insert(id.to_owned(), style);
                }
                Ok(())
            }
        }
    }

    /// Moves the sketch to another plane (a face, or XY / XZ / YZ with an offset).
    pub fn set_plane(&mut self, plane: PlaneSpec) {
        self.plane = plane.canonical();
    }

    pub fn is_construction(&self, id: &str) -> bool {
        self.construction.contains(id)
    }

    pub fn is_infinite(&self, id: &str) -> bool {
        self.infinite.contains(id)
    }

    /// Is this a point that only infinite lines are drawn through? Such points are the line's
    /// definition, not something to show, pick or snap to.
    pub fn only_infinite_line_ends(&self, id: &str) -> bool {
        if self.infinite.is_empty() {
            return false;
        }
        let mut used = false;
        for (k, e) in &self.entities {
            if e.refs().contains(&id) {
                if !self.infinite.contains(k) {
                    return false;
                }
                used = true;
            }
        }
        used
    }

    /// IDs of the construction lines that are infinite.
    pub fn infinite(&self) -> &BTreeSet<String> {
        &self.infinite
    }

    /// Makes a construction line infinite (or finite again).
    pub fn set_infinite(&mut self, id: &str, on: bool) -> Result<(), SketchError> {
        self.expect_kind(id, &[Kind::Line])?;
        if on && !self.construction.contains(id) {
            return Err(SketchError::new(
                SketchErrorCode::InvalidValue,
                format!(
                    "'{id}' is not construction geometry; only construction lines can be infinite"
                ),
                vec![id.to_owned()],
            ));
        }
        if on {
            self.infinite.insert(id.to_owned());
        } else {
            self.infinite.remove(id);
        }
        Ok(())
    }

    /// An error if `id` is an infinite line, which has no ends to work on.
    pub(crate) fn refuse_infinite(&self, id: &str, what: &str) -> Result<(), SketchError> {
        if self.infinite.contains(id) {
            return Err(SketchError::new(
                SketchErrorCode::InvalidValue,
                format!("'{id}' is an infinite line: it has no ends to {what}"),
                vec![id.to_owned()],
            ));
        }
        Ok(())
    }

    /// Marks a line, circle or arc as construction geometry (or back to normal).
    /// Sets a circle's radius directly (the solver then adapts everything else).
    pub(crate) fn set_circle_radius(&mut self, id: &str, radius: f64) -> Result<(), SketchError> {
        if !radius.is_finite() || radius <= 0.0 {
            return Err(SketchError::new(
                SketchErrorCode::InvalidValue,
                "a radius must be a positive number",
                vec![id.to_owned()],
            ));
        }
        self.expect_kind(id, &[Kind::Circle])?;
        if let Some(Entity::Circle { radius: r, .. }) = self.entities.get_mut(id) {
            *r = radius;
        }
        Ok(())
    }

    pub fn set_construction(&mut self, id: &str, on: bool) -> Result<(), SketchError> {
        match self.entities.get(id) {
            None => Err(SketchError::new(
                SketchErrorCode::NotFound,
                format!("entity '{id}' does not exist"),
                vec![id.to_owned()],
            )),
            Some(Entity::Point { .. }) => Err(SketchError::new(
                SketchErrorCode::InvalidValue,
                format!(
                    "'{id}' is a point; only lines, circles and arcs can be construction geometry"
                ),
                vec![id.to_owned()],
            )),
            Some(_) => {
                if on {
                    self.construction.insert(id.to_owned());
                } else {
                    self.construction.remove(id);
                    self.infinite.remove(id);
                }
                Ok(())
            }
        }
    }

    pub fn constraints(&self) -> &BTreeMap<String, Constraint> {
        &self.constraints
    }

    pub fn entity(&self, id: &str) -> Option<&Entity> {
        self.entities.get(id)
    }

    pub fn point(&self, id: &str) -> Option<(f64, f64)> {
        match self.entities.get(id) {
            Some(Entity::Point { x, y }) => Some((*x, *y)),
            _ => None,
        }
    }

    pub(crate) fn expect_kind(&self, id: &str, allowed: &[Kind]) -> Result<(), SketchError> {
        match self.entities.get(id) {
            None => Err(SketchError::new(
                SketchErrorCode::NotFound,
                format!("entity '{id}' does not exist"),
                vec![id.to_owned()],
            )),
            Some(e) if allowed.contains(&e.kind()) => Ok(()),
            Some(e) => Err(SketchError::new(
                SketchErrorCode::WrongKind,
                format!(
                    "entity '{id}' is a {:?}, expected one of {allowed:?}",
                    e.kind()
                ),
                vec![id.to_owned()],
            )),
        }
    }

    pub fn add_entity(&mut self, id: &str, entity: Entity) -> Result<(), SketchError> {
        valid_id(id)?;
        if self.entities.contains_key(id) || self.constraints.contains_key(id) {
            return Err(SketchError::new(
                SketchErrorCode::AlreadyExists,
                format!("'{id}' already exists"),
                vec![id.to_owned()],
            ));
        }
        match &entity {
            Entity::Point { x, y } => {
                finite("x", *x)?;
                finite("y", *y)?;
            }
            Entity::Line { a, b } => {
                self.expect_kind(a, &[Kind::Point])?;
                self.expect_kind(b, &[Kind::Point])?;
                if a == b {
                    return Err(SketchError::new(
                        SketchErrorCode::InvalidValue,
                        "a line needs two different points",
                        vec![a.clone()],
                    ));
                }
            }
            Entity::Circle { center, radius } => {
                self.expect_kind(center, &[Kind::Point])?;
                positive("radius", *radius)?;
            }
            Entity::Arc { center, start, end } => {
                for p in [center, start, end] {
                    self.expect_kind(p, &[Kind::Point])?;
                }
                if center == start || center == end || start == end {
                    return Err(SketchError::new(
                        SketchErrorCode::InvalidValue,
                        "an arc needs three different points",
                        vec![center.clone(), start.clone(), end.clone()],
                    ));
                }
            }
        }
        self.entities.insert(id.to_owned(), entity);
        Ok(())
    }

    /// Moves a point (used for dragging; the solver then pulls constrained geometry along).
    pub fn set_point(&mut self, id: &str, x: f64, y: f64) -> Result<(), SketchError> {
        finite("x", x)?;
        finite("y", y)?;
        self.expect_kind(id, &[Kind::Point])?;
        self.entities.insert(id.to_owned(), Entity::Point { x, y });
        Ok(())
    }

    /// Refuses if anything still refers to the entity, and says what.
    pub fn remove_entity(&mut self, id: &str) -> Result<Entity, SketchError> {
        if !self.entities.contains_key(id) {
            return Err(SketchError::new(
                SketchErrorCode::NotFound,
                format!("entity '{id}' does not exist"),
                vec![id.to_owned()],
            ));
        }
        let mut users: Vec<String> = self
            .entities
            .iter()
            .filter(|(_, e)| e.refs().contains(&id))
            .map(|(k, _)| k.clone())
            .chain(
                self.constraints
                    .iter()
                    .filter(|(_, c)| c.refs().contains(&id))
                    .map(|(k, _)| k.clone()),
            )
            .collect();
        if !users.is_empty() {
            users.sort();
            return Err(SketchError::new(
                SketchErrorCode::InUse,
                format!("'{id}' is used by {}", users.join(", ")),
                users,
            ));
        }
        self.construction.remove(id);
        self.infinite.remove(id);
        Ok(self.entities.remove(id).expect("checked above"))
    }

    pub fn add_constraint(&mut self, id: &str, c: Constraint) -> Result<(), SketchError> {
        valid_id(id)?;
        if self.entities.contains_key(id) || self.constraints.contains_key(id) {
            return Err(SketchError::new(
                SketchErrorCode::AlreadyExists,
                format!("'{id}' already exists"),
                vec![id.to_owned()],
            ));
        }
        use Constraint::*;
        use Kind::*;
        match &c {
            Coincident { a, b }
            | Distance { a, b, .. }
            | HorizontalDistance { a, b, .. }
            | VerticalDistance { a, b, .. } => {
                self.expect_kind(a, &[Point])?;
                self.expect_kind(b, &[Point])?;
                if a == b {
                    return Err(SketchError::new(
                        SketchErrorCode::InvalidValue,
                        "a constraint between a point and itself does nothing",
                        vec![a.clone()],
                    ));
                }
            }
            Horizontal { line } | Vertical { line } | Length { line, .. } => {
                self.expect_kind(line, &[Line])?
            }
            Parallel { a, b } | Perpendicular { a, b } | EqualLength { a, b } => {
                self.expect_kind(a, &[Line])?;
                self.expect_kind(b, &[Line])?;
                if a == b {
                    return Err(SketchError::new(
                        SketchErrorCode::InvalidValue,
                        "a constraint between a line and itself does nothing",
                        vec![a.clone()],
                    ));
                }
            }
            Radius { target, .. } => self.expect_kind(target, &[Circle, Arc])?,
            ArcLength { arc, .. } => self.expect_kind(arc, &[Arc])?,
            Fixed { point, .. } | OnAxis { point, .. } => self.expect_kind(point, &[Point])?,
            PointOnLine { point, line } => {
                self.expect_kind(point, &[Point])?;
                self.expect_kind(line, &[Line])?;
            }
            PointLineDistance { point, line, .. } => {
                self.expect_kind(point, &[Point])?;
                self.expect_kind(line, &[Line])?;
                if let Some(Entity::Line { a, b }) = self.entities.get(line.as_str()) {
                    if a == point || b == point {
                        return Err(SketchError::new(
                            SketchErrorCode::InvalidValue,
                            "the point is an end of that line, so its distance is always zero",
                            vec![point.clone(), line.clone()],
                        ));
                    }
                }
            }
            Midpoint { point, line } => {
                self.expect_kind(point, &[Point])?;
                self.expect_kind(line, &[Line])?;
                if let Some(Entity::Line { a, b }) = self.entities.get(line.as_str()) {
                    if a == point || b == point {
                        return Err(SketchError::new(
                            SketchErrorCode::InvalidValue,
                            "the point is an end of that line, so it cannot also be its middle",
                            vec![point.clone(), line.clone()],
                        ));
                    }
                }
            }
            Symmetric { a, b, line } => {
                self.expect_kind(a, &[Point])?;
                self.expect_kind(b, &[Point])?;
                self.expect_kind(line, &[Line])?;
                let on_line = match self.entities.get(line.as_str()) {
                    Some(Entity::Line { a: p, b: q }) => [a, b].iter().any(|x| *x == p || *x == q),
                    _ => false,
                };
                if a == b || on_line {
                    return Err(SketchError::new(
                        SketchErrorCode::InvalidValue,
                        "symmetry needs two different points that are not ends of the mirror line",
                        vec![a.clone(), b.clone(), line.clone()],
                    ));
                }
            }
            Angle { a, b, .. } => {
                self.expect_kind(a, &[Line])?;
                self.expect_kind(b, &[Line])?;
                if a == b {
                    return Err(SketchError::new(
                        SketchErrorCode::InvalidValue,
                        "an angle needs two different lines",
                        vec![a.clone()],
                    ));
                }
            }
            SymmetricAxis { a, b, .. } => {
                self.expect_kind(a, &[Point])?;
                self.expect_kind(b, &[Point])?;
                if a == b {
                    return Err(SketchError::new(
                        SketchErrorCode::InvalidValue,
                        "symmetry needs two different points",
                        vec![a.clone()],
                    ));
                }
            }
            EqualRadius { a, b } | TangentCurves { a, b } => {
                self.expect_kind(a, &[Circle, Arc])?;
                self.expect_kind(b, &[Circle, Arc])?;
                if a == b {
                    return Err(SketchError::new(
                        SketchErrorCode::InvalidValue,
                        "a constraint between a curve and itself does nothing",
                        vec![a.clone()],
                    ));
                }
            }
            PointOnCurve { point, curve } => {
                self.expect_kind(point, &[Point])?;
                self.expect_kind(curve, &[Circle, Arc])?;
                let part_of = self
                    .entities
                    .get(curve.as_str())
                    .is_some_and(|e| e.refs().contains(&point.as_str()));
                if part_of {
                    return Err(SketchError::new(
                        SketchErrorCode::InvalidValue,
                        "the point is the centre or an end of that curve, so it is not a point on it",
                        vec![point.clone(), curve.clone()],
                    ));
                }
            }
            Tangent { line, curve } => {
                self.expect_kind(line, &[Line])?;
                self.expect_kind(curve, &[Circle, Arc])?;
            }
        }
        match &c {
            Length { value, .. }
            | Distance { value, .. }
            | Radius { value, .. }
            | ArcLength { value, .. } => positive("value", *value)?,
            HorizontalDistance { value, .. }
            | VerticalDistance { value, .. }
            | PointLineDistance { value, .. } => finite("value", *value)?,
            Angle { value, .. } => {
                finite("value", *value)?;
                if value.abs() <= 0.0 || value.abs() >= 180.0 {
                    return Err(SketchError::new(
                        SketchErrorCode::InvalidValue,
                        "an angle must be between 0 and 180 degrees (use parallel for 0 or 180)",
                        c.refs().into_iter().map(str::to_owned).collect(),
                    ));
                }
            }
            Fixed { x, y, .. } => {
                finite("x", *x)?;
                finite("y", *y)?;
            }
            _ => {}
        }
        self.constraints.insert(id.to_owned(), c);
        Ok(())
    }

    /// Changes a dimension (length, distance or radius). The usual parametric edit.
    pub fn set_constraint_value(&mut self, id: &str, new_value: f64) -> Result<(), SketchError> {
        positive("value", new_value)?;
        if self.driven.contains(id) {
            return Err(SketchError::new(
                SketchErrorCode::WrongKind,
                format!("'{id}' is a reference dimension: it follows the geometry; make it driving first"),
                vec![id.to_owned()],
            ));
        }
        if matches!(self.constraints.get(id), Some(Constraint::Angle { .. })) && new_value >= 180.0
        {
            return Err(SketchError::new(
                SketchErrorCode::InvalidValue,
                "an angle must be between 0 and 180 degrees",
                vec![id.to_owned()],
            ));
        }
        match self.constraints.get_mut(id) {
            None => Err(SketchError::new(
                SketchErrorCode::NotFound,
                format!("constraint '{id}' does not exist"),
                vec![id.to_owned()],
            )),
            Some(
                Constraint::Length { value, .. }
                | Constraint::Distance { value, .. }
                | Constraint::ArcLength { value, .. }
                | Constraint::Radius { value, .. },
            ) => {
                *value = canon(new_value);
                Ok(())
            }
            Some(
                Constraint::HorizontalDistance { value, .. }
                | Constraint::VerticalDistance { value, .. }
                | Constraint::PointLineDistance { value, .. }
                | Constraint::Angle { value, .. },
            ) => {
                // Editing a distance or angle keeps the side it was on.
                let sign = if *value < 0.0 { -1.0 } else { 1.0 };
                *value = canon(sign * new_value);
                Ok(())
            }
            Some(_) => Err(SketchError::new(
                SketchErrorCode::WrongKind,
                format!("constraint '{id}' has no dimension value to set"),
                vec![id.to_owned()],
            )),
        }
    }

    pub fn remove_constraint(&mut self, id: &str) -> Result<Constraint, SketchError> {
        self.placements.remove(id);
        self.driven.remove(id);
        self.styles.remove(id);
        self.constraints.remove(id).ok_or_else(|| {
            SketchError::new(
                SketchErrorCode::NotFound,
                format!("constraint '{id}' does not exist"),
                vec![id.to_owned()],
            )
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pt(s: &mut Sketch, id: &str, x: f64, y: f64) {
        s.add_entity(id, Entity::Point { x, y }).unwrap();
    }

    #[test]
    fn builds_and_validates_references() {
        let mut s = Sketch::new();
        pt(&mut s, "p1", 0.0, 0.0);
        pt(&mut s, "p2", 10.0, 0.0);
        s.add_entity(
            "l1",
            Entity::Line {
                a: "p1".into(),
                b: "p2".into(),
            },
        )
        .unwrap();
        let e = s
            .add_entity(
                "l2",
                Entity::Line {
                    a: "p1".into(),
                    b: "nope".into(),
                },
            )
            .unwrap_err();
        assert_eq!(e.code, SketchErrorCode::NotFound);
        assert_eq!(e.entities, ["nope"]);
        let e = s
            .add_entity(
                "l3",
                Entity::Line {
                    a: "p1".into(),
                    b: "l1".into(),
                },
            )
            .unwrap_err();
        assert_eq!(e.code, SketchErrorCode::WrongKind);
        let e = s
            .add_entity(
                "l4",
                Entity::Line {
                    a: "p1".into(),
                    b: "p1".into(),
                },
            )
            .unwrap_err();
        assert_eq!(e.code, SketchErrorCode::InvalidValue);
        let e = s
            .add_entity("p1", Entity::Point { x: 1.0, y: 1.0 })
            .unwrap_err();
        assert_eq!(e.code, SketchErrorCode::AlreadyExists);
    }

    #[test]
    fn rejects_bad_values_and_ids() {
        let mut s = Sketch::new();
        assert_eq!(
            s.add_entity("bad id", Entity::Point { x: 0.0, y: 0.0 })
                .unwrap_err()
                .code,
            SketchErrorCode::InvalidId
        );
        assert_eq!(
            s.add_entity(
                "p",
                Entity::Point {
                    x: f64::NAN,
                    y: 0.0
                }
            )
            .unwrap_err()
            .code,
            SketchErrorCode::InvalidValue
        );
        pt(&mut s, "p1", 0.0, 0.0);
        pt(&mut s, "p2", 1.0, 0.0);
        s.add_entity(
            "l",
            Entity::Line {
                a: "p1".into(),
                b: "p2".into(),
            },
        )
        .unwrap();
        assert_eq!(
            s.add_entity(
                "c",
                Entity::Circle {
                    center: "p1".into(),
                    radius: -1.0
                }
            )
            .unwrap_err()
            .code,
            SketchErrorCode::InvalidValue
        );
        let e = s
            .add_constraint(
                "k",
                Constraint::Length {
                    line: "l".into(),
                    value: 0.0,
                },
            )
            .unwrap_err();
        assert_eq!(e.code, SketchErrorCode::InvalidValue);
        let e = s
            .add_constraint("k", Constraint::Horizontal { line: "p1".into() })
            .unwrap_err();
        assert_eq!(e.code, SketchErrorCode::WrongKind);
        let e = s
            .add_constraint(
                "k",
                Constraint::Coincident {
                    a: "p1".into(),
                    b: "p1".into(),
                },
            )
            .unwrap_err();
        assert_eq!(e.code, SketchErrorCode::InvalidValue);
    }

    #[test]
    fn removal_reports_dependents() {
        let mut s = Sketch::new();
        pt(&mut s, "p1", 0.0, 0.0);
        pt(&mut s, "p2", 1.0, 0.0);
        s.add_entity(
            "l",
            Entity::Line {
                a: "p1".into(),
                b: "p2".into(),
            },
        )
        .unwrap();
        s.add_constraint("h", Constraint::Horizontal { line: "l".into() })
            .unwrap();
        let e = s.remove_entity("p1").unwrap_err();
        assert_eq!(e.code, SketchErrorCode::InUse);
        assert_eq!(e.entities, ["l"]);
        let e = s.remove_entity("l").unwrap_err();
        assert_eq!(e.entities, ["h"]);
        s.remove_constraint("h").unwrap();
        s.remove_entity("l").unwrap();
        s.remove_entity("p1").unwrap();
    }

    #[test]
    fn serialization_is_deterministic_and_roundtrips() {
        let mut s = Sketch::new();
        // Insert in non-sorted order; output must be sorted.
        pt(&mut s, "b", 1.0, 2.0);
        pt(&mut s, "a", 3.0, 4.0);
        s.add_entity(
            "l",
            Entity::Line {
                a: "a".into(),
                b: "b".into(),
            },
        )
        .unwrap();
        let json = serde_json::to_string(&s).unwrap();
        assert!(json.find("\"a\"").unwrap() < json.find("\"b\"").unwrap());
        let back: Sketch = serde_json::from_str(&json).unwrap();
        assert_eq!(back, s);
        assert_eq!(serde_json::to_string(&back).unwrap(), json);
    }
    #[test]
    fn a_dimension_remembers_where_it_was_placed_and_forgets_when_it_goes() {
        let mut s = Sketch::new();
        s.add_entity("a", Entity::Point { x: 0.0, y: 0.0 }).unwrap();
        s.add_entity("b", Entity::Point { x: 10.0, y: 0.0 })
            .unwrap();
        s.add_entity(
            "l",
            Entity::Line {
                a: "a".into(),
                b: "b".into(),
            },
        )
        .unwrap();
        s.add_constraint(
            "len",
            Constraint::Length {
                line: "l".into(),
                value: 10.0,
            },
        )
        .unwrap();
        s.add_constraint("hz", Constraint::Horizontal { line: "l".into() })
            .unwrap();
        // Nothing placed: serialises exactly as before (no new key).
        let before = serde_json::to_string(&s).unwrap();
        assert!(!before.contains("placements"));
        s.set_placement("len", 5.0, -0.0).unwrap();
        assert_eq!(s.placement("len"), Some((5.0, 0.0)), "no negative zero");
        assert!(serde_json::to_string(&s).unwrap().contains("placements"));
        let back: Sketch = serde_json::from_str(&serde_json::to_string(&s).unwrap()).unwrap();
        assert_eq!(back, s);
        // Only dimensions have a place; bad numbers and unknown ids are refused.
        assert_eq!(
            s.set_placement("hz", 1.0, 1.0).unwrap_err().code,
            SketchErrorCode::WrongKind
        );
        assert_eq!(
            s.set_placement("nope", 1.0, 1.0).unwrap_err().code,
            SketchErrorCode::NotFound
        );
        assert!(s.set_placement("len", f64::NAN, 0.0).is_err());
        assert_eq!(
            s.placement("len"),
            Some((5.0, 0.0)),
            "refusals change nothing"
        );
        // Removing the dimension removes its place.
        s.remove_constraint("len").unwrap();
        assert!(s.placements().is_empty());
        assert_eq!(
            serde_json::to_string(&s).unwrap(),
            serde_json::to_string(&{
                let mut t = s.clone();
                t.placements.clear();
                t
            })
            .unwrap()
        );
        // Pruning drops places whose dimension vanished some other way.
        s.add_constraint(
            "len2",
            Constraint::Length {
                line: "l".into(),
                value: 10.0,
            },
        )
        .unwrap();
        s.set_placement("len2", 1.0, 2.0).unwrap();
        s.constraints.remove("len2");
        s.prune_placements();
        assert!(s.placements().is_empty());
    }
}

#[cfg(test)]
mod plane_tests {
    use super::*;

    #[test]
    fn named_planes_have_documented_normals_and_offsets() {
        let n = |s: PlaneSpec| s.named_plane().unwrap().normal();
        assert_eq!(n(PlaneSpec::Xy { offset: 0.0 }), Vec3::Z);
        assert_eq!(n(PlaneSpec::Xz { offset: 0.0 }), -Vec3::Y);
        assert_eq!(n(PlaneSpec::Yz { offset: 0.0 }), Vec3::X);
        // Offset runs along the normal, so an XZ plane at offset 5 sits at y = -5.
        assert_eq!(
            PlaneSpec::Xz { offset: 5.0 }.named_plane().unwrap().origin,
            Vec3::new(0.0, -5.0, 0.0)
        );
    }

    #[test]
    fn plane_defaults_and_serializes_as_intent() {
        let s = Sketch::new();
        assert_eq!(s.plane_spec(), PlaneSpec::Xy { offset: 0.0 });
        let on = Sketch::on_plane(PlaneSpec::Yz { offset: 12.5 }).unwrap();
        let json = serde_json::to_string(&on).unwrap();
        assert!(
            json.contains(r#""plane":{"base":"yz","offset":12.5}"#),
            "{json}"
        );
        assert_eq!(serde_json::from_str::<Sketch>(&json).unwrap(), on);
        // Old data without a plane still loads, on XY.
        let old: Sketch = serde_json::from_str(r#"{"entities":{},"constraints":{}}"#).unwrap();
        assert_eq!(old.plane_spec(), PlaneSpec::Xy { offset: 0.0 });
        assert!(Sketch::on_plane(PlaneSpec::Xy { offset: f64::NAN }).is_err());
    }
}

#[cfg(test)]
mod face_plane_tests {
    use super::*;

    #[test]
    fn a_face_plane_serializes_as_a_reference_and_has_no_world_frame() {
        let spec = PlaneSpec::Face {
            feature: "plate".into(),
            face: "cap_end".into(),
            offset: 0.0,
        };
        assert!(spec.named_plane().is_none(), "needs a document to resolve");
        assert_eq!(spec.attached_feature(), Some("plate"));
        assert_eq!(PlaneSpec::Xy { offset: 0.0 }.attached_feature(), None);
        let json = serde_json::to_string(&spec).unwrap();
        assert_eq!(
            json,
            r#"{"base":"face","feature":"plate","face":"cap_end","offset":0.0}"#
        );
        assert_eq!(serde_json::from_str::<PlaneSpec>(&json).unwrap(), spec);
        // offset is optional on input
        let no_offset: PlaneSpec =
            serde_json::from_str(r#"{"base":"face","feature":"p","face":"f"}"#).unwrap();
        assert_eq!(no_offset.offset(), 0.0);
        let s = Sketch::on_plane(spec.clone()).unwrap();
        assert_eq!(s.plane_spec(), spec);
        assert!(s.named_plane().is_none());
        assert!(Sketch::on_plane(PlaneSpec::Face {
            feature: "p".into(),
            face: "f".into(),
            offset: f64::NAN
        })
        .is_err());
    }
}
