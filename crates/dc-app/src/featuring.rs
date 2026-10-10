//! Feature dialogs: Extrude, Cut and Boss from the UI. A draft holds the choices (sketch,
//! profile, depth, direction, target); the document is not touched until it is committed as one
//! ordinary command, but the drafted result is previewed live by asking the authority what the
//! command would build. Logic only, no egui.

use crate::state::{AppState, Selected};
use dc_doc::query::ProfileInfo;
use dc_doc::{
    Command, CommandKind, EndMode, Extent, FeaturePattern, FeatureSpec, Query, QueryResult,
};
use dc_kernel::Solid;
use dc_render::Hit;
use std::sync::Arc;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FeatureKind {
    Extrude,
    Cut,
    Boss,
    /// A full turn of a sketch profile about an axis (shafts, bushings, washers).
    Revolve,
    /// Rounds edges parallel to the part's axis.
    Fillet,
    /// Bevels edges parallel to the part's axis.
    Chamfer,
    /// Hollows a prism-shaped part, leaving end faces open.
    Shell,
    /// Joins the outlines of two sketches.
    Loft,
    /// Pushes a profile along a straight line of another sketch.
    Sweep,
}

impl FeatureKind {
    pub const ALL: [FeatureKind; 9] = [
        FeatureKind::Extrude,
        FeatureKind::Cut,
        FeatureKind::Boss,
        FeatureKind::Revolve,
        FeatureKind::Fillet,
        FeatureKind::Chamfer,
        FeatureKind::Shell,
        FeatureKind::Loft,
        FeatureKind::Sweep,
    ];

    pub fn name(self) -> &'static str {
        match self {
            FeatureKind::Extrude => "Extrude",
            FeatureKind::Cut => "Cut",
            FeatureKind::Boss => "Boss",
            FeatureKind::Revolve => "Revolve",
            FeatureKind::Fillet => "Fillet",
            FeatureKind::Chamfer => "Chamfer",
            FeatureKind::Shell => "Shell",
            FeatureKind::Loft => "Loft",
            FeatureKind::Sweep => "Sweep",
        }
    }

    /// Does it work from a sketch? (Fillet and chamfer work on the part itself.)
    pub fn needs_sketch(self) -> bool {
        !matches!(
            self,
            FeatureKind::Fillet | FeatureKind::Chamfer | FeatureKind::Shell
        )
    }

    /// Does it round or bevel edges (picked in the view)?
    pub fn rounds_edges(self) -> bool {
        matches!(self, FeatureKind::Fillet | FeatureKind::Chamfer)
    }

    pub fn hint(self) -> &'static str {
        match self {
            FeatureKind::Extrude => "Push a closed sketch profile out into a solid",
            FeatureKind::Cut => {
                "Remove material with a sketch profile, to a depth or all the way through"
            }
            FeatureKind::Boss => {
                "Add material to a part with a sketch profile (it must touch the part)"
            }
            FeatureKind::Revolve => {
                "Turn a closed profile once round an axis (lines parallel or square to it)"
            }
            FeatureKind::Fillet => {
                "Round edges: click them in the view (edges along the part's axis, and the circular rim of a shaft or hole)"
            }
            FeatureKind::Chamfer => {
                "Bevel edges: click them in the view (edges along the part's axis, and the circular rim of a shaft or hole)"
            }
            FeatureKind::Shell => {
                "Hollow out a prism-shaped part with walls of one thickness, leaving end faces open"
            }
            FeatureKind::Loft => {
                "Join the outlines of two sketches with straight edges (the sketches must be in different places and have outlines with the same number of corners)"
            }
            FeatureKind::Sweep => {
                "Push a profile along a straight line of another sketch (from its first point to its second)"
            }
        }
    }

    fn prefix(self) -> &'static str {
        match self {
            FeatureKind::Extrude => "extrude",
            FeatureKind::Cut => "cut",
            FeatureKind::Boss => "boss",
            FeatureKind::Revolve => "revolve",
            FeatureKind::Fillet => "fillet",
            FeatureKind::Chamfer => "chamfer",
            FeatureKind::Shell => "shell",
            FeatureKind::Loft => "loft",
            FeatureKind::Sweep => "sweep",
        }
    }
}

/// How far a feature goes, as SolidWorks offers it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EndCondition {
    /// From the sketch plane out to a depth.
    Blind,
    /// All the way through the part (cuts), in the chosen direction.
    ThroughAll,
    /// All the way through in both directions (cuts).
    ThroughBoth,
    /// Symmetric about the sketch plane: the depth is the total.
    MidPlane,
}

impl EndCondition {
    pub fn name(self) -> &'static str {
        match self {
            EndCondition::Blind => "Blind",
            EndCondition::ThroughAll => "Through All",
            EndCondition::ThroughBoth => "Through All - Both",
            EndCondition::MidPlane => "Mid Plane",
        }
    }

    /// What a kind of feature can end with.
    pub fn for_kind(kind: FeatureKind) -> &'static [EndCondition] {
        match kind {
            FeatureKind::Cut => &[
                EndCondition::Blind,
                EndCondition::ThroughAll,
                EndCondition::ThroughBoth,
                EndCondition::MidPlane,
            ],
            _ => &[EndCondition::Blind, EndCondition::MidPlane],
        }
    }

    /// Does this end condition use a depth?
    pub fn has_depth(self) -> bool {
        matches!(self, EndCondition::Blind | EndCondition::MidPlane)
    }
}

/// The choices of a feature that is being set up (or whose recipe is being edited).
#[derive(Clone, Debug, PartialEq)]
pub struct FeatureDraft {
    pub kind: FeatureKind,
    pub name: String,
    /// An existing feature is being edited: its name is fixed and accepting runs `feature.update`.
    pub editing: bool,
    pub sketch: String,
    /// The closed regions to use; empty means every region of the sketch (a revolve uses
    /// exactly one: it holds the one chosen).
    pub profiles: Vec<String>,
    /// The feature a cut, boss, fillet or chamfer works on.
    pub target: Option<String>,
    pub depth: f64,
    pub end: EndCondition,
    /// A second direction (against the first) with its own depth.
    pub second: bool,
    pub depth2: f64,
    /// A cut that removes what is outside the profile instead of inside it.
    pub outside: bool,
    pub reverse: bool,
    /// Copies of a cut or boss.
    pub pattern: Option<FeaturePattern>,
    /// What a revolve turns about: `x`, `y` or the ID of a line of the sketch.
    pub axis: String,
    /// Degrees a revolve turns by; 360 is a full turn.
    pub angle: f64,
    /// Fillet and chamfer: the edges, each named by the two faces that meet there.
    pub edges: Vec<[String; 2]>,
    /// The radius of a fillet or the distance of a chamfer (the wall of a shell).
    pub size: f64,
    /// Shell: the end faces left open.
    pub faces: Vec<String>,
    /// Loft: the sketch it ends in; sweep: the sketch holding the path.
    pub sketch2: String,
    /// Loft: the regions of the second sketch (empty = every region).
    pub profiles2: Vec<String>,
    /// Sweep: the line of `sketch2` the profile follows.
    pub path: String,
}

/// Something that would make a refused draft work, found by trying it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DraftFix {
    /// Going the other way from the sketch plane builds.
    ReverseDirection,
}

impl DraftFix {
    pub fn label(self) -> &'static str {
        match self {
            DraftFix::ReverseDirection => "Reverse the direction",
        }
    }
}

/// What the current draft would build, or why it cannot.
#[derive(Clone)]
pub struct FeaturePreview {
    pub for_draft: FeatureDraft,
    pub result: Result<Vec<(String, Arc<Solid>)>, String>,
    /// If it cannot be built, a one-click change that would make it work.
    pub fix: Option<DraftFix>,
    /// The entities (`sketch/<id>[/<item>]`, `feature/<id>`) the refusal is about; the view
    /// draws them in red.
    pub entities: Vec<String>,
}

impl AppState {
    /// The closed regions of a sketch that can be pushed out.
    pub fn profiles_of(&self, sketch: &str) -> Vec<ProfileInfo> {
        match self.authority.query(&Query::SketchProfiles {
            sketch: sketch.to_owned(),
            after: None,
            limit: Some(200),
        }) {
            Ok(QueryResult::SketchProfiles { profiles, .. }) => profiles,
            _ => Vec::new(),
        }
    }

    /// Sketches that have at least one profile.
    pub fn extrudable_sketches(&self) -> Vec<String> {
        self.sketch_rows()
            .into_iter()
            .map(|r| r.id)
            .filter(|s| !self.profiles_of(s).is_empty())
            .collect()
    }

    /// The solids a cut or boss can work on: the feature that is the current result.
    pub fn feature_targets(&self) -> Vec<String> {
        self.authority.state().result_features()
    }

    /// The lines of a sketch a revolve can turn about, as `(id, what to show)`; the two axes
    /// of the sketch come first.
    pub fn axes_of(&self, sketch: &str) -> Vec<(String, String)> {
        let mut v = vec![
            ("x".to_owned(), "the sketch's x axis".to_owned()),
            ("y".to_owned(), "the sketch's y axis".to_owned()),
        ];
        if let Some(sk) = self.authority.state().sketch(sketch) {
            for (id, e) in sk.sketch.entities() {
                if matches!(e, dc_sketch::Entity::Line { .. }) {
                    let kind = if sk.sketch.is_construction(id) {
                        "construction line"
                    } else {
                        "line"
                    };
                    v.push((id.clone(), format!("{kind} {id}")));
                }
            }
        }
        v
    }

    fn free_feature_name(&self, kind: FeatureKind) -> String {
        (1u32..)
            .map(|n| format!("{}{n}", kind.prefix()))
            .find(|n| self.authority.state().feature(n).is_none())
            .expect("a free name exists")
    }

    fn blank_draft(&self, kind: FeatureKind) -> FeatureDraft {
        FeatureDraft {
            kind,
            name: self.free_feature_name(kind),
            editing: false,
            sketch: String::new(),
            profiles: Vec::new(),
            target: None,
            depth: 10.0,
            end: EndCondition::Blind,
            second: false,
            depth2: 5.0,
            outside: false,
            reverse: false,
            pattern: None,
            axis: "y".to_owned(),
            angle: 360.0,
            edges: Vec::new(),
            size: 2.0,
            faces: Vec::new(),
            sketch2: String::new(),
            profiles2: Vec::new(),
            path: String::new(),
        }
    }

    /// Opens the dialog for `kind` with sensible defaults: the selected (or just finished) sketch,
    /// its first profile, and the latest part as the target. Says why when it cannot.
    pub fn begin_feature(&mut self, kind: FeatureKind) -> bool {
        let editing = self.sketching.as_ref().map(|s| s.sketch.clone());
        if editing.is_some() {
            self.end_sketch();
        }
        if let Some(bar) = self.authority.state().rollback() {
            self.error(
                format!("the rollback bar is after '{bar}': features after it are off"),
                Some("roll forward to the end first".into()),
            );
            return false;
        }
        let target = self.feature_targets().last().cloned();
        if !matches!(
            kind,
            FeatureKind::Extrude | FeatureKind::Revolve | FeatureKind::Loft | FeatureKind::Sweep
        ) && target.is_none()
        {
            self.error(
                format!("{} works on an existing part", kind.name()),
                Some("make an extrusion first".into()),
            );
            return false;
        }
        let mut draft = self.blank_draft(kind);
        if kind.needs_sketch() {
            let usable = self.extrudable_sketches();
            let chosen = match &self.selected {
                Selected::Sketch(s) if usable.contains(s) => Some(s.clone()),
                _ => None,
            }
            .or_else(|| editing.filter(|s| usable.contains(s)))
            .or_else(|| usable.last().cloned());
            let Some(sketch) = chosen else {
                self.error(
                    format!("{} needs a sketch with a closed shape", kind.name()),
                    Some(
                        "draw a closed profile (a rectangle, a circle, a closed outline) first"
                            .into(),
                    ),
                );
                return false;
            };
            draft.sketch = sketch;
            draft.profiles = self.default_profiles(kind, &draft.sketch);
            if matches!(kind, FeatureKind::Loft | FeatureKind::Sweep) {
                if let Some(second) = self.second_sketch_for(kind, &draft.sketch) {
                    draft.sketch2 = second.clone();
                    if kind == FeatureKind::Sweep {
                        draft.path = self
                            .path_lines(&second)
                            .first()
                            .cloned()
                            .unwrap_or_default();
                    }
                } else {
                    self.error(
                        if kind == FeatureKind::Loft {
                            "a loft needs two sketches with closed shapes".to_owned()
                        } else {
                            "a sweep needs a sketch with a closed shape and another with a line to follow".to_owned()
                        },
                        Some("draw the second sketch on another plane first".into()),
                    );
                    return false;
                }
            }
            if kind == FeatureKind::Revolve {
                // A centre line is the natural axis if the sketch has one.
                draft.axis = self
                    .axes_of(&draft.sketch)
                    .into_iter()
                    .find(|(_, name)| name.starts_with("construction line"))
                    .map_or_else(|| "y".to_owned(), |(id, _)| id);
            }
        }
        draft.target = matches!(
            kind,
            FeatureKind::Cut
                | FeatureKind::Boss
                | FeatureKind::Fillet
                | FeatureKind::Chamfer
                | FeatureKind::Shell
        )
        .then_some(target)
        .flatten();
        // Edges that are already selected are the first edges to round (all of one part).
        if kind.rounds_edges() {
            for s in self.selection() {
                if let Selected::Edge { feature, edge } = s {
                    if draft.edges.is_empty() {
                        draft.target = Some(feature.clone());
                    }
                    if draft.target.as_deref() == Some(feature.as_str()) {
                        if let Some((a, b)) = edge.split_once('|') {
                            let pair = [a.to_owned(), b.to_owned()];
                            if !draft.edges.contains(&pair) {
                                draft.edges.push(pair);
                            }
                        }
                    }
                }
            }
        }
        // Selected end faces are the faces a shell leaves open.
        if kind == FeatureKind::Shell {
            if let Some(t) = draft.target.clone() {
                if let Some((a, b)) = self.shell_end_faces(&t) {
                    for s in self.selection() {
                        if let Selected::Face { feature, face } = s {
                            if feature == t
                                && (face == a || face == b)
                                && !draft.faces.contains(&face)
                            {
                                draft.faces.push(face);
                            }
                        }
                    }
                }
            }
        }
        self.feature_draft = Some(draft);
        self.update_feature_preview();
        // A cut starts into the material: if the first try never reaches the part (a sketch on
        // the top face cuts upwards by default), go the other way.
        if kind == FeatureKind::Cut
            && self.feature_preview.as_ref().and_then(|p| p.fix) == Some(DraftFix::ReverseDirection)
        {
            self.apply_draft_fix(DraftFix::ReverseDirection);
        }
        true
    }

    /// The lines of a sketch a sweep can follow (ids).
    pub fn path_lines(&self, sketch: &str) -> Vec<String> {
        self.axes_of(sketch)
            .into_iter()
            .map(|(id, _)| id)
            .filter(|id| id != "x" && id != "y")
            .collect()
    }

    /// The sketches a loft can end in (another sketch with closed shapes) or a sweep can follow
    /// (another sketch with a line).
    pub fn second_sketch_choices(&self, kind: FeatureKind, first: &str) -> Vec<String> {
        let all: Vec<String> = self.sketch_rows().into_iter().map(|r| r.id).collect();
        all.into_iter()
            .filter(|s| s != first)
            .filter(|s| {
                if kind == FeatureKind::Loft {
                    !self.profiles_of(s).is_empty()
                } else {
                    !self.path_lines(s).is_empty()
                }
            })
            .collect()
    }

    fn second_sketch_for(&self, kind: FeatureKind, first: &str) -> Option<String> {
        self.second_sketch_choices(kind, first).pop()
    }

    /// The two end faces a shell of `feature` can leave open, if it is a prism.
    pub fn shell_end_faces(&self, feature: &str) -> Option<(String, String)> {
        let slot = self.authority.state().feature(feature)?;
        dc_doc::shell_end_faces(&slot.solid)
    }

    /// Opens the page of an existing feature with its recipe filled in; accepting replaces the
    /// recipe (`feature.update`), everything built on it is rebuilt.
    pub fn begin_edit_feature(&mut self, id: &str) -> bool {
        let Some(slot) = self.authority.state().feature(id) else {
            return false;
        };
        let spec = slot.spec.clone();
        let hidden = self.authority.state().hidden_features();
        if hidden.contains(id) {
            self.error(
                format!("'{id}' is off (suppressed or after the rollback bar)"),
                Some("switch it on first".into()),
            );
            return false;
        }
        let Some(draft) = self.draft_from_spec(id, &spec) else {
            return false;
        };
        self.feature_draft = Some(draft);
        self.update_feature_preview();
        true
    }

    fn draft_from_spec(&self, id: &str, spec: &FeatureSpec) -> Option<FeatureDraft> {
        let end_of = |e: &Extent, depth: Option<f64>| match (e.mode, depth) {
            (EndMode::MidPlane, _) => EndCondition::MidPlane,
            (EndMode::ThroughBoth, _) => EndCondition::ThroughBoth,
            (EndMode::Blind, None) => EndCondition::ThroughAll,
            (EndMode::Blind, Some(_)) => EndCondition::Blind,
        };
        let (kind, mut d) = match spec {
            FeatureSpec::Extrude { .. } => (FeatureKind::Extrude, None),
            FeatureSpec::Cut { .. } => (FeatureKind::Cut, None),
            FeatureSpec::Boss { .. } => (FeatureKind::Boss, None),
            FeatureSpec::Revolve { .. } => (FeatureKind::Revolve, None),
            FeatureSpec::Fillet { .. } => (FeatureKind::Fillet, None),
            FeatureSpec::Chamfer { .. } => (FeatureKind::Chamfer, None),
            FeatureSpec::Shell { .. } => (FeatureKind::Shell, None),
            FeatureSpec::Loft { .. } => (FeatureKind::Loft, None),
            FeatureSpec::Sweep { .. } => (FeatureKind::Sweep, None),
        };
        let mut draft: FeatureDraft = self.blank_draft(kind);
        draft.name = id.to_owned();
        draft.editing = true;
        match spec {
            FeatureSpec::Extrude {
                sketch,
                profiles,
                depth,
                reverse,
                extent,
            } => {
                draft.sketch = sketch.clone();
                draft.profiles = profiles.clone();
                draft.depth = *depth;
                draft.reverse = *reverse;
                draft.end = end_of(extent, Some(*depth));
                d = Some(extent.clone());
            }
            FeatureSpec::Cut {
                target,
                sketch,
                profiles,
                depth,
                reverse,
                extent,
                pattern,
            } => {
                draft.target = Some(target.clone());
                draft.sketch = sketch.clone();
                draft.profiles = profiles.clone();
                draft.depth = depth.unwrap_or(draft.depth);
                draft.reverse = *reverse;
                draft.end = end_of(extent, *depth);
                draft.outside = extent.outside;
                draft.pattern = pattern.clone();
                d = Some(extent.clone());
            }
            FeatureSpec::Boss {
                target,
                sketch,
                profiles,
                depth,
                reverse,
                extent,
                pattern,
            } => {
                draft.target = Some(target.clone());
                draft.sketch = sketch.clone();
                draft.profiles = profiles.clone();
                draft.depth = *depth;
                draft.reverse = *reverse;
                draft.end = end_of(extent, Some(*depth));
                draft.pattern = pattern.clone();
                d = Some(extent.clone());
            }
            FeatureSpec::Revolve {
                sketch,
                profiles,
                axis,
                angle,
            } => {
                draft.sketch = sketch.clone();
                draft.profiles = profiles.clone();
                draft.axis = axis.clone();
                draft.angle = angle.unwrap_or(360.0);
            }
            FeatureSpec::Fillet {
                target,
                edges,
                radius,
            } => {
                draft.target = Some(target.clone());
                draft.edges = edges.clone();
                draft.size = *radius;
            }
            FeatureSpec::Chamfer {
                target,
                edges,
                distance,
            } => {
                draft.target = Some(target.clone());
                draft.edges = edges.clone();
                draft.size = *distance;
            }
            FeatureSpec::Shell {
                target,
                faces,
                thickness,
            } => {
                draft.target = Some(target.clone());
                draft.faces = faces.clone();
                draft.size = *thickness;
            }
            FeatureSpec::Loft {
                sketch,
                profiles,
                to_sketch,
                to_profiles,
            } => {
                draft.sketch = sketch.clone();
                draft.profiles = profiles.clone();
                draft.sketch2 = to_sketch.clone();
                draft.profiles2 = to_profiles.clone();
            }
            FeatureSpec::Sweep {
                sketch,
                profiles,
                path_sketch,
                path,
            } => {
                draft.sketch = sketch.clone();
                draft.profiles = profiles.clone();
                draft.sketch2 = path_sketch.clone();
                draft.path = path.clone();
            }
        }
        if let Some(e) = d {
            if let Some(d2) = e.depth2 {
                draft.second = true;
                draft.depth2 = d2;
            }
        }
        Some(draft)
    }

    /// Starts Fillet or Chamfer with one edge already chosen (from the selection).
    pub fn begin_rounding_with_edge(&mut self, kind: FeatureKind, feature: &str, edge: &str) {
        if !self.begin_feature(kind) {
            return;
        }
        if let (Some(d), Some((a, b))) = (&mut self.feature_draft, edge.split_once('|')) {
            d.target = Some(feature.to_owned());
            d.edges = vec![[a.to_owned(), b.to_owned()]];
        }
        self.update_feature_preview();
    }

    /// "line", "circle" ... of a sketch entity, for the selection panel.
    pub fn entity_kind(&self, sketch: &str, entity: &str) -> &'static str {
        use dc_sketch::Entity;
        match self
            .authority
            .state()
            .sketch(sketch)
            .and_then(|s| s.sketch.entity(entity))
        {
            Some(Entity::Line { .. }) => "line",
            Some(Entity::Circle { .. }) => "circle",
            Some(Entity::Arc { .. }) => "arc",
            Some(Entity::Point { .. }) => "point",
            None => "entity",
        }
    }

    /// What a new draft uses: every region, except for a revolve, which needs one (the first).
    fn default_profiles(&self, kind: FeatureKind, sketch: &str) -> Vec<String> {
        if kind == FeatureKind::Revolve {
            self.profiles_of(sketch)
                .first()
                .map(|p| vec![p.id.clone()])
                .unwrap_or_default()
        } else {
            Vec::new()
        }
    }

    /// Picks another sketch for the draft.
    pub fn draft_set_sketch(&mut self, sketch: &str) {
        let kind = self.feature_draft.as_ref().map(|d| d.kind);
        let profiles = kind.map_or_else(Vec::new, |k| self.default_profiles(k, sketch));
        if let Some(d) = &mut self.feature_draft {
            d.sketch = sketch.to_owned();
            d.profiles = profiles;
        }
    }

    /// Clicking a region: a revolve takes exactly that one; the others toggle it in the list
    /// (the first click on "every region" narrows it to that one).
    pub fn draft_toggle_profile(&mut self, id: &str) {
        let Some(d) = &mut self.feature_draft else {
            return;
        };
        if d.kind == FeatureKind::Revolve || d.profiles.is_empty() {
            d.profiles = vec![id.to_owned()];
        } else if let Some(at) = d.profiles.iter().position(|p| p == id) {
            // The last region stays: an empty list would mean "every region".
            if d.profiles.len() > 1 {
                d.profiles.remove(at);
            }
        } else {
            d.profiles.push(id.to_owned());
        }
    }

    /// The closed region of `sketch` under a click in the 3D view, if any: the click is cast onto
    /// the sketch's plane and the smallest profile that contains the point wins (so a circle
    /// inside a rectangle is a hole in the rectangle's region, not a region of its own).
    pub fn pick_profile(&self, sketch: &str, px: f64, py: f64, w: f64, h: f64) -> Option<String> {
        let plane = self.authority.state().sketch_plane(sketch).ok()?;
        let (origin, dir) = self.camera.ray(px, py, w, h);
        let n = plane.normal();
        let denom = dir.dot(n);
        if denom.abs() < 1e-9 {
            return None;
        }
        let t = (plane.origin - origin).dot(n) / denom;
        if t < 0.0 {
            return None;
        }
        let at = plane.to_2d(origin + dir * t);
        let sk = &self.authority.state().sketch(sketch)?.sketch;
        sk.profiles()
            .profiles
            .into_iter()
            .filter(|p| p.contains(at))
            .min_by(|a, b| a.area.total_cmp(&b.area))
            .map(|p| p.id)
    }

    /// A click in the view while a feature is being set up. For features from a sketch it picks
    /// the region under the click; for a fillet or chamfer it collects faces two at a time (the
    /// two that meet along an edge). `false` if the click was not for the draft.
    pub fn draft_click(&mut self, px: f64, py: f64, w: f64, h: f64) -> bool {
        let Some(d) = self.feature_draft.clone() else {
            return false;
        };
        if d.kind.rounds_edges() {
            // A click on an edge of the target part adds it, or takes it out if it is there.
            if let Some(Hit::Edge { body, name, .. }) = self.pick_hit_at(px, py, w, h) {
                if d.target.as_deref() == Some(body.as_str()) {
                    if let Some((a, b)) = name.split_once('|') {
                        let pair = [a.to_owned(), b.to_owned()];
                        if let Some(dr) = &mut self.feature_draft {
                            match dr.edges.iter().position(|e| *e == pair) {
                                Some(at) => {
                                    dr.edges.remove(at);
                                }
                                None => dr.edges.push(pair),
                            }
                        }
                    }
                } else {
                    self.info(format!(
                        "that edge belongs to '{body}', not to the part this works on"
                    ));
                }
            }
            return true;
        }
        if d.kind == FeatureKind::Revolve {
            // A line of the sketch in the view is the axis.
            if let Some(Hit::SketchLine { sketch, entity, .. }) = self.pick_hit_at(px, py, w, h) {
                if sketch == d.sketch && self.entity_kind(&sketch, &entity) == "line" {
                    if let Some(dr) = &mut self.feature_draft {
                        dr.axis = entity;
                    }
                    return true;
                }
            }
        }
        match self.pick_profile(&d.sketch, px, py, w, h) {
            Some(id) => {
                self.draft_toggle_profile(&id);
                true
            }
            None => false,
        }
    }

    /// The command this draft stands for, or what is missing.
    pub fn draft_command(&self, d: &FeatureDraft) -> Result<CommandKind, String> {
        let positive = |v: f64| v.is_finite() && v > 0.0;
        let need_target = || {
            d.target
                .clone()
                .ok_or_else(|| "choose the part to work on".to_owned())
        };
        let made = match d.kind {
            FeatureKind::Fillet | FeatureKind::Chamfer => {
                if d.edges.is_empty() {
                    return Err("click the edges to round in the view".into());
                }
                if !positive(d.size) {
                    return Err("the size must be more than zero".into());
                }
                if d.kind == FeatureKind::Fillet {
                    CommandKind::FeatureFillet {
                        feature: d.name.clone(),
                        target: need_target()?,
                        edges: d.edges.clone(),
                        radius: d.size,
                    }
                } else {
                    CommandKind::FeatureChamfer {
                        feature: d.name.clone(),
                        target: need_target()?,
                        edges: d.edges.clone(),
                        distance: d.size,
                    }
                }
            }
            FeatureKind::Shell => {
                if !positive(d.size) {
                    return Err("the wall thickness must be more than zero".into());
                }
                CommandKind::FeatureShell {
                    feature: d.name.clone(),
                    target: need_target()?,
                    faces: d.faces.clone(),
                    thickness: d.size,
                }
            }
            FeatureKind::Loft => {
                if self.profiles_of(&d.sketch).is_empty() {
                    return Err("the first sketch has no closed shape to use".into());
                }
                if d.sketch2.is_empty() || self.profiles_of(&d.sketch2).is_empty() {
                    return Err("choose a second sketch with a closed shape".into());
                }
                CommandKind::FeatureLoft {
                    feature: d.name.clone(),
                    sketch: d.sketch.clone(),
                    profiles: d.profiles.clone(),
                    to_sketch: d.sketch2.clone(),
                    to_profiles: d.profiles2.clone(),
                }
            }
            FeatureKind::Sweep => {
                if self.profiles_of(&d.sketch).is_empty() {
                    return Err("this sketch has no closed shape to use".into());
                }
                if d.sketch2.is_empty() || d.path.is_empty() {
                    return Err("choose the sketch and the line the profile follows".into());
                }
                CommandKind::FeatureSweep {
                    feature: d.name.clone(),
                    sketch: d.sketch.clone(),
                    profiles: d.profiles.clone(),
                    path_sketch: d.sketch2.clone(),
                    path: d.path.clone(),
                }
            }
            FeatureKind::Revolve => {
                if self.profiles_of(&d.sketch).is_empty() {
                    return Err("this sketch has no closed shape to use".into());
                }
                CommandKind::FeatureRevolve {
                    feature: d.name.clone(),
                    sketch: d.sketch.clone(),
                    profiles: d.profiles.clone(),
                    axis: d.axis.clone(),
                    angle: ((d.angle - 360.0).abs() > 1e-9).then_some(d.angle),
                }
            }
            FeatureKind::Extrude | FeatureKind::Cut | FeatureKind::Boss => {
                if self.profiles_of(&d.sketch).is_empty() {
                    return Err("this sketch has no closed shape to use".into());
                }
                if d.end.has_depth() && !positive(d.depth) {
                    return Err("the depth must be more than zero".into());
                }
                let second = d.second
                    && d.end != EndCondition::ThroughBoth
                    && d.end != EndCondition::MidPlane;
                if second && !positive(d.depth2) {
                    return Err("the second direction's depth must be more than zero".into());
                }
                let extent = Extent {
                    mode: match d.end {
                        EndCondition::MidPlane => EndMode::MidPlane,
                        EndCondition::ThroughBoth => EndMode::ThroughBoth,
                        _ => EndMode::Blind,
                    },
                    depth2: second.then_some(d.depth2),
                    outside: d.kind == FeatureKind::Cut && d.outside,
                };
                match d.kind {
                    FeatureKind::Extrude => CommandKind::FeatureExtrude {
                        feature: d.name.clone(),
                        sketch: d.sketch.clone(),
                        profiles: d.profiles.clone(),
                        depth: d.depth,
                        reverse: d.reverse,
                        extent,
                    },
                    FeatureKind::Cut => CommandKind::FeatureCut {
                        feature: d.name.clone(),
                        target: need_target()?,
                        sketch: d.sketch.clone(),
                        profiles: d.profiles.clone(),
                        depth: d.end.has_depth().then_some(d.depth),
                        reverse: d.reverse,
                        extent,
                        pattern: d.pattern.clone(),
                    },
                    _ => CommandKind::FeatureBoss {
                        feature: d.name.clone(),
                        target: need_target()?,
                        sketch: d.sketch.clone(),
                        profiles: d.profiles.clone(),
                        depth: d.depth,
                        reverse: d.reverse,
                        extent,
                        pattern: d.pattern.clone(),
                    },
                }
            }
        };
        if d.editing {
            let spec = spec_of(&made).ok_or("this kind of feature cannot be edited")?;
            Ok(CommandKind::FeatureUpdate {
                feature: d.name.clone(),
                spec,
            })
        } else {
            Ok(made)
        }
    }

    /// What the draft would build, asked of the authority.
    fn preview_of(&mut self, d: &FeatureDraft) -> Result<Vec<(String, Arc<Solid>)>, String> {
        self.preview_with_entities(d).0
    }

    #[allow(clippy::type_complexity)]
    fn preview_with_entities(
        &mut self,
        d: &FeatureDraft,
    ) -> (Result<Vec<(String, Arc<Solid>)>, String>, Vec<String>) {
        let mut entities = Vec::new();
        let actor = self.actor.clone();
        let base = self.authority.revision();
        let result = self.draft_command(d).and_then(|kind| {
            let cmd = Command {
                id: "preview".into(),
                actor,
                base_revision: base,
                kind,
            };
            self.authority.preview_solids(&cmd).map_err(|diag| {
                entities.clone_from(&diag.entities);
                match &diag.suggestion {
                    Some(s) => format!("{} ({s})", diag.message),
                    None => diag.message.clone(),
                }
            })
        });
        (result, entities)
    }

    /// Recomputes the preview if the draft changed since it was last computed.
    pub fn update_feature_preview(&mut self) {
        let Some(d) = self.feature_draft.clone() else {
            if self.feature_preview.take().is_some() {
                self.preview_version += 1;
                self.refresh_scene();
            }
            return;
        };
        if self
            .feature_preview
            .as_ref()
            .is_some_and(|p| p.for_draft == d)
        {
            return;
        }
        let (result, entities) = self.preview_with_entities(&d);
        // A refused feature that would build the other way round says so, and can do it for you.
        let fix = (result.is_err()
            && d.kind.needs_sketch()
            && !matches!(
                d.kind,
                FeatureKind::Revolve | FeatureKind::Loft | FeatureKind::Sweep
            ))
        .then(|| {
            let mut flipped = d.clone();
            flipped.reverse = !flipped.reverse;
            self.preview_of(&flipped)
                .is_ok()
                .then_some(DraftFix::ReverseDirection)
        })
        .flatten();
        self.feature_preview = Some(FeaturePreview {
            for_draft: d,
            result,
            fix,
            entities,
        });
        self.preview_version += 1;
        self.refresh_scene();
        self.view_version += 1;
    }

    /// Why the draft cannot be made right now, if it cannot.
    pub fn draft_problem(&self) -> Option<String> {
        self.feature_preview
            .as_ref()
            .and_then(|p| p.result.as_ref().err().cloned())
    }

    /// A one-click change that would make the refused draft build.
    pub fn draft_fix(&self) -> Option<DraftFix> {
        self.feature_preview.as_ref().and_then(|p| p.fix)
    }

    /// Makes the change `fix` stands for.
    pub fn apply_draft_fix(&mut self, fix: DraftFix) {
        if let Some(d) = &mut self.feature_draft {
            match fix {
                DraftFix::ReverseDirection => d.reverse = !d.reverse,
            }
        }
        self.update_feature_preview();
    }

    /// Makes the feature as one command; the feature becomes the selection.
    pub fn commit_feature_draft(&mut self) -> bool {
        let Some(d) = self.feature_draft.clone() else {
            return false;
        };
        let kind = match self.draft_command(&d) {
            Ok(k) => k,
            Err(why) => {
                self.error(why, None);
                return false;
            }
        };
        if self.submit(kind).is_err() {
            return false;
        }
        self.cancel_feature_draft();
        self.select(Selected::Feature(d.name));
        true
    }

    pub fn cancel_feature_draft(&mut self) {
        self.feature_draft = None;
        self.update_feature_preview();
    }
}

/// The recipe a feature command stands for.
pub fn spec_of(kind: &CommandKind) -> Option<FeatureSpec> {
    Some(match kind.clone() {
        CommandKind::FeatureExtrude {
            sketch,
            profiles,
            depth,
            reverse,
            extent,
            ..
        } => FeatureSpec::Extrude {
            sketch,
            profiles,
            depth,
            reverse,
            extent,
        },
        CommandKind::FeatureCut {
            target,
            sketch,
            profiles,
            depth,
            reverse,
            extent,
            pattern,
            ..
        } => FeatureSpec::Cut {
            target,
            sketch,
            profiles,
            depth,
            reverse,
            extent,
            pattern,
        },
        CommandKind::FeatureBoss {
            target,
            sketch,
            profiles,
            depth,
            reverse,
            extent,
            pattern,
            ..
        } => FeatureSpec::Boss {
            target,
            sketch,
            profiles,
            depth,
            reverse,
            extent,
            pattern,
        },
        CommandKind::FeatureRevolve {
            sketch,
            profiles,
            axis,
            angle,
            ..
        } => FeatureSpec::Revolve {
            sketch,
            profiles,
            axis,
            angle,
        },
        CommandKind::FeatureFillet {
            target,
            edges,
            radius,
            ..
        } => FeatureSpec::Fillet {
            target,
            edges,
            radius,
        },
        CommandKind::FeatureChamfer {
            target,
            edges,
            distance,
            ..
        } => FeatureSpec::Chamfer {
            target,
            edges,
            distance,
        },
        CommandKind::FeatureShell {
            target,
            faces,
            thickness,
            ..
        } => FeatureSpec::Shell {
            target,
            faces,
            thickness,
        },
        CommandKind::FeatureLoft {
            sketch,
            profiles,
            to_sketch,
            to_profiles,
            ..
        } => FeatureSpec::Loft {
            sketch,
            profiles,
            to_sketch,
            to_profiles,
        },
        CommandKind::FeatureSweep {
            sketch,
            profiles,
            path_sketch,
            path,
            ..
        } => FeatureSpec::Sweep {
            sketch,
            profiles,
            path_sketch,
            path,
        },
        _ => return None,
    })
}
