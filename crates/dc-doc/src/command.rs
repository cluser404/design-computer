use crate::diag::DiagCode;
use crate::feature::{Extent, FeaturePattern};
use crate::{Actor, Diagnostic};
use dc_sketch::{
    Constraint, DimStyle, DragRadius, DragTarget, Entity, MirrorAbout, Pattern, PlaneSpec,
    Transform,
};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// Monotonic document revision. Revision 0 is the empty document.
#[derive(
    Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize, JsonSchema,
)]
#[serde(transparent)]
pub struct Revision(pub u64);

/// Precondition on the current value of a target, used to detect stale edits instead of blindly
/// overwriting (AGENTS.md §4.2).
#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum Expected {
    /// No precondition.
    #[default]
    Any,
    /// The target must not exist.
    Absent,
    /// The target must currently equal this value.
    Value(f64),
}

impl Expected {
    pub(crate) fn check(self, name: &str, current: Option<f64>) -> Result<(), Diagnostic> {
        let ok = match (self, current) {
            (Expected::Any, _) => true,
            (Expected::Absent, None) => true,
            (Expected::Value(e), Some(c)) => e == c,
            _ => false,
        };
        if ok {
            return Ok(());
        }
        let found = match current {
            Some(c) => format!("{c}"),
            None => "absent".to_owned(),
        };
        let wanted = match self {
            Expected::Any => unreachable!(),
            Expected::Absent => "absent".to_owned(),
            Expected::Value(e) => format!("{e}"),
        };
        Err(Diagnostic::new(
            DiagCode::StaleExpectation,
            format!("'{name}' is {found}, but the command expected {wanted}"),
        )
        .entity(name)
        .suggest("re-read the current value and resubmit against the latest revision"))
    }
}

/// The semantic operations. Each variant is one reviewable, undoable unit (AGENTS.md §4.3).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "type")]
pub enum CommandKind {
    #[serde(rename = "param.set")]
    ParamSet {
        name: String,
        value: f64,
        #[serde(default)]
        expected: Expected,
    },
    #[serde(rename = "param.delete")]
    ParamDelete {
        name: String,
        #[serde(default)]
        expected: Expected,
    },
    // NOTE: a command's own `id` (its idempotency key) is flattened into the same JSON object as
    // these variants. Variant fields must therefore never be called `id` (or `actor`,
    // `base_revision`), or the serialized command has duplicate keys and cannot be parsed back.
    /// Creates an empty sketch.
    #[serde(rename = "sketch.create")]
    SketchCreate {
        sketch: String,
        /// Where the sketch lives in 3D. Defaults to the XY plane at offset 0.
        #[serde(default)]
        plane: PlaneSpec,
    },
    #[serde(rename = "sketch.delete")]
    SketchDelete { sketch: String },
    #[serde(rename = "sketch.add_entity")]
    SketchAddEntity {
        sketch: String,
        entity_id: String,
        entity: Entity,
    },
    /// Refused while anything still refers to the entity (the diagnostic lists what).
    #[serde(rename = "sketch.remove_entity")]
    SketchRemoveEntity { sketch: String, entity_id: String },
    /// Marks a line, circle or arc as construction geometry (a guide that is not part of any
    /// profile) or turns it back into normal geometry.
    #[serde(rename = "sketch.set_construction")]
    SketchSetConstruction {
        sketch: String,
        entity_id: String,
        construction: bool,
    },
    /// Makes a construction line infinite (it runs on for ever both ways and has no ends, so
    /// only where it is placed has to be defined) or finite again. Only construction lines can
    /// be infinite.
    #[serde(rename = "sketch.set_infinite")]
    SketchSetInfinite {
        sketch: String,
        entity_id: String,
        infinite: bool,
    },
    #[serde(rename = "sketch.add_constraint")]
    SketchAddConstraint {
        sketch: String,
        constraint_id: String,
        constraint: Constraint,
    },
    #[serde(rename = "sketch.remove_constraint")]
    SketchRemoveConstraint {
        sketch: String,
        constraint_id: String,
    },
    /// Edits a dimension (length, distance, radius): the basic parametric change.
    #[serde(rename = "sketch.set_value")]
    SketchSetValue {
        sketch: String,
        constraint_id: String,
        value: f64,
        #[serde(default)]
        expected: Expected,
    },
    /// Commits a drag: the point lands on (x, y) if feasible, else the nearest valid shape.
    /// The drag gesture itself is ephemeral; only this final result is a command.
    #[serde(rename = "sketch.drag_point")]
    SketchDragPoint {
        sketch: String,
        point: String,
        x: f64,
        y: f64,
    },
    /// Commits a drag of a whole shape: several points land at their targets (a line's two ends,
    /// an arc's three points, which moves it) and / or a circle gets a new radius. Like
    /// `sketch.drag_point`, held exactly if feasible, else the nearest valid shape.
    #[serde(rename = "sketch.drag")]
    SketchDrag {
        sketch: String,
        #[serde(default)]
        points: Vec<DragTarget>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        radius: Option<DragRadius>,
    },
    /// Trim away inside / outside: every line or arc that crosses both bounding shapes loses the
    /// part between them (`inside`) or the parts beyond them.
    #[serde(rename = "sketch.trim_between")]
    SketchTrimBetween {
        sketch: String,
        bounds: Vec<String>,
        inside: bool,
        #[serde(default, skip_serializing_if = "std::ops::Not::not")]
        construction: bool,
    },
    /// Moves, turns or scales shapes in place, or does it to copies (relations among them carry).
    #[serde(rename = "sketch.transform")]
    SketchTransform {
        sketch: String,
        entities: Vec<String>,
        transform: Transform,
        #[serde(default, skip_serializing_if = "std::ops::Not::not")]
        copy: bool,
    },
    /// Mirrored copies of shapes about a line of the sketch or one of its axes, tied to the
    /// originals by symmetry.
    #[serde(rename = "sketch.mirror")]
    SketchMirror {
        sketch: String,
        entities: Vec<String>,
        about: MirrorAbout,
    },
    /// Linear or circular copies of shapes.
    #[serde(rename = "sketch.pattern")]
    SketchPattern {
        sketch: String,
        entities: Vec<String>,
        pattern: Pattern,
    },
    /// Parallel copies of a chain of lines, or concentric copies of circles and arcs.
    #[serde(rename = "sketch.offset")]
    SketchOffset {
        sketch: String,
        entities: Vec<String>,
        distance: f64,
    },
    /// Rounds the corner where two lines meet.
    #[serde(rename = "sketch.fillet")]
    SketchFillet {
        sketch: String,
        first: String,
        second: String,
        radius: f64,
    },
    /// Cuts the corner where two lines meet; `second_distance` defaults to `distance`.
    #[serde(rename = "sketch.chamfer")]
    SketchChamfer {
        sketch: String,
        first: String,
        second: String,
        distance: f64,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        second_distance: Option<f64>,
    },
    /// Merges points that sit in the same place and drops empty, repeated and duplicate things.
    #[serde(rename = "sketch.repair")]
    SketchRepair { sketch: String },
    /// Adds the relations and dimensions still missing to fully define the sketch.
    #[serde(rename = "sketch.fully_define")]
    SketchFullyDefine { sketch: String },
    /// Makes a dimension a reference (it measures, and constrains nothing) or a driving one.
    #[serde(rename = "sketch.set_driven")]
    SketchSetDriven {
        sketch: String,
        constraint_id: String,
        driven: bool,
    },
    /// How a dimension is written: decimals and tolerance (presentation only).
    #[serde(rename = "sketch.set_dimension_style")]
    SketchSetDimensionStyle {
        sketch: String,
        constraint_id: String,
        style: DimStyle,
    },
    /// Moves the sketch to another plane (a world plane with an offset, or a face).
    #[serde(rename = "sketch.set_plane")]
    SketchSetPlane { sketch: String, plane: PlaneSpec },
    /// Moves where a dimension is drawn (its text and dimension line), in sketch coordinates.
    /// Presentation only: it changes no geometry, but it is saved and undoable.
    #[serde(rename = "sketch.place_dimension")]
    SketchPlaceDimension {
        sketch: String,
        constraint_id: String,
        x: f64,
        y: f64,
    },
    /// Trims a line, arc or circle: removes the part around `(x, y)` between the shapes that cut
    /// it (deletes it entirely if nothing does). `entity_id` is the shape being trimmed.
    ///
    /// By default only real geometry cuts it (set `construction` to let construction lines cut
    /// too). A non-empty `bounds` makes only those shapes cut ("trim between bounds").
    #[serde(rename = "sketch.trim")]
    SketchTrim {
        sketch: String,
        entity_id: String,
        x: f64,
        y: f64,
        #[serde(default, skip_serializing_if = "std::ops::Not::not")]
        construction: bool,
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        bounds: Vec<String>,
    },
    /// Corner trim: trims or extends two lines / arcs to the corner where they meet, keeping
    /// the sides near the two clicked places.
    #[serde(rename = "sketch.corner_trim")]
    SketchCornerTrim {
        sketch: String,
        first: String,
        first_x: f64,
        first_y: f64,
        second: String,
        second_x: f64,
        second_y: f64,
    },
    /// Merges two points into one, rewiring everything that used either.
    #[serde(rename = "sketch.merge_points")]
    SketchMergePoints {
        sketch: String,
        a: String,
        b: String,
    },
    /// Extends the end of a line or arc nearest `(x, y)` to the next shape it meets.
    #[serde(rename = "sketch.extend")]
    SketchExtend {
        sketch: String,
        entity_id: String,
        x: f64,
        y: f64,
        #[serde(default, skip_serializing_if = "std::ops::Not::not")]
        construction: bool,
    },
    /// Splits a line or arc in two at the point nearest `(x, y)`.
    #[serde(rename = "sketch.split")]
    SketchSplit {
        sketch: String,
        entity_id: String,
        x: f64,
        y: f64,
    },
    /// Creates an extrusion of a closed sketch profile. Rejected if the profile does not exist
    /// or the result would be invalid. Rebuilt automatically when its sketch changes.
    #[serde(rename = "feature.extrude")]
    FeatureExtrude {
        feature: String,
        sketch: String,
        #[serde(default)]
        profiles: Vec<String>,
        depth: f64,
        #[serde(default)]
        reverse: bool,
        /// Mid plane, a second direction. Omitted means a plain blind extrusion.
        #[serde(default, skip_serializing_if = "Extent::is_default")]
        extent: Extent,
    },
    /// Cuts material away from an existing feature with a sketch profile, from the sketch plane
    /// to `depth` (omit for through-all). Works on parts built along one axis: through-cuts,
    /// blind pockets, counterbores and steps are all supported. A tool that would need another
    /// direction or only touches the part at a point is refused with code `unsupported`.
    #[serde(rename = "feature.cut")]
    FeatureCut {
        feature: String,
        /// The feature to cut (an extrusion, or another cut).
        target: String,
        sketch: String,
        #[serde(default)]
        profiles: Vec<String>,
        #[serde(default)]
        depth: Option<f64>,
        /// Cut against the cut sketch's plane normal instead of along it.
        #[serde(default)]
        reverse: bool,
        /// Mid plane, through all in both directions, a second direction, cutting outside.
        #[serde(default, skip_serializing_if = "Extent::is_default")]
        extent: Extent,
        /// Copies of the cut: a row or grid, a ring, or a mirror image. Omitted for one cut.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        pattern: Option<FeaturePattern>,
    },
    /// Adds material to an existing feature: a sketch profile pushed out from its plane by
    /// `depth`. It must touch the part (typically sketched on one of its flat faces).
    #[serde(rename = "feature.boss")]
    FeatureBoss {
        feature: String,
        target: String,
        sketch: String,
        #[serde(default)]
        profiles: Vec<String>,
        depth: f64,
        #[serde(default)]
        reverse: bool,
        #[serde(default, skip_serializing_if = "Extent::is_default")]
        extent: Extent,
        /// Copies of the boss. Omitted for one boss.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        pattern: Option<FeaturePattern>,
    },
    /// Revolves a closed sketch profile once round an axis: `"x"` or `"y"` (the sketch's axes) or
    /// the ID of a line in the sketch (a centre line). The profile must lie on one side of the
    /// axis and be made of lines parallel or square to it: a shaft, bushing or washer. Slanted
    /// lines (cones) and arcs (spheres, tori) are refused with `unsupported`.
    #[serde(rename = "feature.revolve")]
    FeatureRevolve {
        feature: String,
        sketch: String,
        #[serde(default)]
        profiles: Vec<String>,
        axis: String,
        /// Degrees to turn, more than 0 and less than 360; omitted for a full turn. A partial
        /// turn starts in the sketch plane and follows the right-hand rule about the axis.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        angle: Option<f64>,
    },
    /// Rounds edges that run parallel to the part's axis with an arc of `radius` tangent to both
    /// faces. An edge is named by the two faces that meet along it (names from `feature.faces`),
    /// e.g. `["side:l1", "side:l2"]`. Concave corners are filled, convex ones cut.
    #[serde(rename = "feature.fillet")]
    FeatureFillet {
        feature: String,
        /// The feature to round (an extrusion, cut, boss, or another fillet or chamfer).
        target: String,
        edges: Vec<[String; 2]>,
        radius: f64,
    },
    /// Bevels edges that run parallel to the part's axis: a flat face `distance` from the edge
    /// along each of its two faces. Edges are named as for `feature.fillet`.
    #[serde(rename = "feature.chamfer")]
    FeatureChamfer {
        feature: String,
        target: String,
        edges: Vec<[String; 2]>,
        distance: f64,
    },
    /// Joins the outline of one sketch to the outline of another with flat faces (see
    /// `FeatureSpec::Loft`): both are closed polygons of straight lines with the same number of
    /// corners, corresponding edges parallel or meeting.
    #[serde(rename = "feature.loft")]
    FeatureLoft {
        feature: String,
        sketch: String,
        #[serde(default)]
        profiles: Vec<String>,
        to_sketch: String,
        #[serde(default)]
        to_profiles: Vec<String>,
    },
    /// Pushes a profile along a straight line of another sketch (first point to second).
    #[serde(rename = "feature.sweep")]
    FeatureSweep {
        feature: String,
        sketch: String,
        #[serde(default)]
        profiles: Vec<String>,
        path_sketch: String,
        path: String,
    },
    /// Hollows a prism-shaped part (two flat end faces, flat faces square to them and cylinders
    /// along the axis between) with walls `thickness` thick. `faces` names the end faces to leave
    /// open (none, one or both); anything else is refused with the reason.
    #[serde(rename = "feature.shell")]
    FeatureShell {
        feature: String,
        target: String,
        #[serde(default)]
        faces: Vec<String>,
        thickness: f64,
    },
    /// Replaces a feature's recipe with another of the same kind (a different depth, profile,
    /// end condition, pattern, edges, ...). Everything built on it is rebuilt; if that would break
    /// anything the whole command is refused and nothing changes.
    #[serde(rename = "feature.update")]
    FeatureUpdate {
        feature: String,
        spec: crate::feature::FeatureSpec,
    },
    /// Sets the name people see for a feature (its ID, which references use, never changes).
    /// An empty label goes back to showing the ID.
    #[serde(rename = "feature.set_label")]
    FeatureSetLabel { feature: String, label: String },
    /// Sets the name people see for a sketch.
    #[serde(rename = "sketch.set_label")]
    SketchSetLabel { sketch: String, label: String },
    /// Switches a feature off (or on again). Everything built on it is off too.
    #[serde(rename = "feature.suppress")]
    FeatureSuppress { feature: String, suppressed: bool },
    /// Puts the rollback bar after a feature: the features after it in the tree are off until
    /// the bar is moved on. `None` rolls forward to the end. No feature can be added meanwhile.
    #[serde(rename = "document.rollback")]
    DocumentRollback {
        #[serde(default)]
        to: Option<String>,
    },
    /// Sets the features of a folder in the design tree (creating the folder); a feature is in
    /// at most one folder. An empty list deletes the folder.
    #[serde(rename = "folder.set")]
    FolderSet {
        folder: String,
        features: Vec<String>,
    },
    /// Changes an extrusion's depth (or a cut's depth, or a fillet's radius, or a chamfer's
    /// distance).
    #[serde(rename = "feature.set_depth")]
    FeatureSetDepth {
        feature: String,
        depth: f64,
        #[serde(default)]
        expected: Expected,
    },
    #[serde(rename = "feature.delete")]
    FeatureDelete { feature: String },
    /// Undoes one of your own earlier commands (default: your latest not-yet-undone one),
    /// restoring exactly what it changed. Refused if later commands touched the same items.
    /// Undo is itself a command in the history, so it replays deterministically.
    #[serde(rename = "history.undo")]
    HistoryUndo {
        #[serde(default)]
        revision: Option<Revision>,
    },
    /// Re-applies a command you undid (default: your most recently undone one).
    #[serde(rename = "history.redo")]
    HistoryRedo {
        #[serde(default)]
        revision: Option<Revision>,
    },
    /// Atomic group: all operations apply or none do. Cannot be nested.
    #[serde(rename = "transaction")]
    Transaction {
        label: String,
        ops: Vec<CommandKind>,
    },
}

/// A command submitted to the authority.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct Command {
    /// Globally unique, chosen by the submitter. Resubmitting the same ID is rejected, which makes
    /// retries over an unreliable transport safe.
    pub id: String,
    pub actor: Actor,
    /// Revision the submitter's view was based on.
    pub base_revision: Revision,
    #[serde(flatten)]
    pub kind: CommandKind,
}

/// One entity change produced by a command: what a diff view would show.
///
/// `entity` is a path: `name` for a parameter, `sketch/<id>` for a whole sketch and
/// `sketch/<id>/<item>` for an entity or constraint inside it. `before`/`after` are the item's
/// JSON (absent = did not exist). Points the solver moved appear here too.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct Change {
    pub entity: String,
    pub before: Option<serde_json::Value>,
    pub after: Option<serde_json::Value>,
}
