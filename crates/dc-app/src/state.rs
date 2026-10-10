//! Everything the app does, independent of any window toolkit, so it can be tested directly.
//!
//! The rule (AGENTS.md §4.1) is that the UI never changes the document itself: every edit is a
//! command submitted to the authority, exactly as an agent's would be. This type is the only place
//! the UI does that.

use crate::featuring::FeatureKind;
use dc_doc::{
    Actor, Authority, Command, CommandKind, DiagCode, Diagnostic, Expected, FaceInfo, FeatureSpec,
    Hash128, Outcome, Policy, Query, QueryResult, SketchInfo,
};
use dc_math::det;
use dc_render::{
    pick, pick_hit, scene_of_authority, Camera, FaceRef, Filter, Hit, Image, ProjectionKind,
    RenderMode, RenderOptions, Scene, ViewPreset,
};
use dc_sketch::Constraint;
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Level {
    Info,
    Error,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Message {
    pub level: Level,
    pub text: String,
    /// What to try next, when the engine said.
    pub hint: Option<String>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Material {
    Steel,
    Aluminum,
    Brass,
    Copper,
    Plastic,
    /// Glossy coloured paint under a clear coat (a car or an appliance).
    Paint,
}

impl Material {
    pub const ALL: [Material; 6] = [
        Material::Steel,
        Material::Aluminum,
        Material::Brass,
        Material::Copper,
        Material::Plastic,
        Material::Paint,
    ];

    pub fn name(self) -> &'static str {
        match self {
            Material::Steel => "Steel",
            Material::Aluminum => "Aluminium",
            Material::Brass => "Brass",
            Material::Copper => "Copper",
            Material::Plastic => "ABS plastic",
            Material::Paint => "Glossy paint",
        }
    }

    /// How much of a clear, glossy layer lies over the base (0 none, 1 a full lacquer): a sharp
    /// reflection of the studio on top of whatever the base looks like.
    pub fn clearcoat(self) -> f32 {
        match self {
            Material::Paint => 1.0,
            _ => 0.0,
        }
    }

    /// `(base colour in linear light, metallic 0..1, roughness 0..1)`.
    pub fn params(self) -> ([f32; 3], f32, f32) {
        match self {
            Material::Steel => ([0.74, 0.72, 0.70], 1.0, 0.12),
            Material::Aluminum => ([0.91, 0.92, 0.92], 1.0, 0.18),
            Material::Brass => ([0.91, 0.78, 0.42], 1.0, 0.20),
            Material::Copper => ([0.95, 0.64, 0.54], 1.0, 0.20),
            Material::Plastic => ([0.10, 0.28, 0.62], 0.0, 0.22),
            Material::Paint => ([0.62, 0.05, 0.04], 0.0, 0.55),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Default)]
pub enum Selected {
    #[default]
    None,
    Sketch(String),
    Feature(String),
    Face {
        feature: String,
        face: String,
    },
    /// An edge of a feature's solid, named by the two faces that meet there (`a|b`).
    Edge {
        feature: String,
        edge: String,
    },
    /// A line, arc or circle of a sketch picked in the 3D view.
    SketchEntity {
        sketch: String,
        entity: String,
    },
}

/// How a click in the 3D view picks things.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum SelectMode {
    /// A click picks the face, edge or line under the pointer (what SolidWorks does).
    #[default]
    Direct,
    /// A click picks the feature first; another click on the same feature picks the face or
    /// edge under the pointer (drilling down).
    FeatureFirst,
}

/// What the pointer is over besides a face: drawn highlighted, and what a click would select.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Hover {
    Edge { feature: String, edge: String },
    SketchEntity { sketch: String, entity: String },
}

#[derive(Clone, Debug, PartialEq)]
pub struct SketchRow {
    pub id: String,
    pub info: SketchInfo,
}

#[derive(Clone, Debug, PartialEq)]
pub struct FeatureRow {
    pub id: String,
    pub kind: String,
    pub target: Option<String>,
    pub sketch: String,
    pub volume: f64,
    /// The editable depth, if the feature has one.
    pub depth: Option<f64>,
}

pub struct AppState {
    pub authority: Authority,
    pub actor: Actor,
    pub camera: Camera,
    pub scene: Scene,
    /// The thing selected last (the one panels and commands read).
    pub selected: Selected,
    /// Further things selected with it (Ctrl-click, a rubber band), earlier ones first. Valid
    /// only while `selected` is still `extra_for`: code that sets `selected` directly drops them.
    extra: Vec<Selected>,
    extra_for: Selected,
    pub select_mode: SelectMode,
    pub hovered: Option<FaceRef>,
    /// An edge or sketch line under the pointer (a face is `hovered`).
    pub hover_other: Option<Hover>,
    /// The scene before a drafted feature is shown in place of it.
    pub base_scene: Scene,
    /// Which kinds of things a click in the view can pick.
    pub filter: Filter,
    pub messages: Vec<Message>,
    pub path: Option<PathBuf>,
    /// The model's origin (a small red / green / blue triad), as in SolidWorks. On by default.
    pub show_origins: bool,
    /// The grid of the plane being sketched on. Off by default, as in SolidWorks (its sketch
    /// origin is marked either way).
    pub show_sketch_grid: bool,
    pub show_edges: bool,
    /// Ambient occlusion in the GPU viewport. On by default; a view setting, not document data.
    pub ambient_occlusion: bool,
    /// An extra smoothing pass over the image in the GPU viewport. Off by default.
    pub fxaa: bool,
    /// A faint reflection of the part in the ground in RealView. On by default.
    pub floor_reflection: bool,
    /// What RealView reflects (a view setting, not document data).
    pub environment: crate::environment::Environment,
    pub show_sketches: bool,
    /// Realistic shading (reflections, shadows) in the GPU viewport. View setting, not document.
    pub realview: bool,
    pub material: Material,
    /// Bumps whenever what the viewport shows may have changed (document, selection, camera...).
    pub view_version: u64,
    /// Width / height of the viewport, once the UI knows it.
    viewport_aspect: Option<f64>,
    /// Keep the view fitted to the part (and to the window) until the user moves the camera.
    pub(crate) auto_fit: bool,
    /// The sketch being edited, if the app is in sketch mode.
    pub sketching: Option<crate::sketching::SketchSession>,
    saved_hash: Hash128,
    extras: BTreeMap<String, Vec<u8>>,
    scene_for: Option<(u64, Hash128)>,
    /// What the scene was last built for, besides the document: bodies hidden while sketching.
    scene_hidden: bool,
    /// Hide the solid bodies while sketching (the sketch outlines stay).
    pub hide_bodies: bool,
    /// Bumps whenever `scene` is rebuilt, so GPU copies know when to re-upload.
    pub scene_version: u64,
    /// The feature being set up in a dialog, if any (nothing is in the document yet).
    pub feature_draft: Option<crate::featuring::FeatureDraft>,
    /// What that draft would build, shown in the viewport.
    pub feature_preview: Option<crate::featuring::FeaturePreview>,
    /// Bumps when the preview changes; the scene follows it.
    pub(crate) preview_version: u64,
    scene_preview: u64,
    /// Which sketch the scene was built showing (see `shown_sketch`).
    scene_shown: Option<String>,
}

const ORBIT_PER_PX: f64 = 0.008;

impl Default for AppState {
    fn default() -> Self {
        Self::new()
    }
}

impl AppState {
    pub fn new() -> Self {
        let authority = Authority::new(Policy::default());
        let saved_hash = authority.state_hash();
        let mut s = AppState {
            authority,
            actor: Actor::Human("local".into()),
            // As in SolidWorks: a new part opens in the isometric view, drawn orthographically;
            // perspective is a switch (View > Perspective).
            camera: Camera {
                projection: ProjectionKind::Orthographic,
                ..Camera::preset(ViewPreset::Iso)
            },
            scene: Scene::default(),
            selected: Selected::None,
            extra: Vec::new(),
            extra_for: Selected::None,
            select_mode: SelectMode::Direct,
            hovered: None,
            hover_other: None,
            base_scene: Scene::default(),
            filter: Filter::ALL,
            messages: Vec::new(),
            path: None,
            ambient_occlusion: true,
            fxaa: false,
            floor_reflection: true,
            environment: crate::environment::Environment::default(),
            show_origins: true,
            show_sketch_grid: false,
            show_edges: true,
            show_sketches: true,
            realview: true,
            material: Material::Steel,
            view_version: 0,
            viewport_aspect: None,
            auto_fit: true,
            sketching: None,
            saved_hash,
            extras: BTreeMap::new(),
            scene_for: None,
            scene_hidden: false,
            hide_bodies: false,
            feature_draft: None,
            feature_preview: None,
            preview_version: 0,
            scene_preview: 0,
            scene_shown: None,
            scene_version: 0,
        };
        s.refresh_scene();
        s
    }

    // ---- messages -------------------------------------------------------------------------

    pub fn info(&mut self, text: impl Into<String>) {
        self.messages.push(Message {
            level: Level::Info,
            text: text.into(),
            hint: None,
        });
        self.trim_messages();
    }

    pub fn error(&mut self, text: impl Into<String>, hint: Option<String>) {
        self.messages.push(Message {
            level: Level::Error,
            text: text.into(),
            hint,
        });
        self.trim_messages();
    }

    fn trim_messages(&mut self) {
        if self.messages.len() > 200 {
            self.messages.drain(0..self.messages.len() - 200);
        }
    }

    fn report(&mut self, d: &Diagnostic) {
        let mut text = d.message.clone();
        if !d.entities.is_empty() {
            text.push_str(&format!("  [{}]", d.entities.join(", ")));
        }
        self.error(text, d.suggestion.clone());
    }

    // ---- the document ---------------------------------------------------------------------

    /// Is the document different from what was last saved or opened?
    pub fn dirty(&self) -> bool {
        self.authority.state_hash() != self.saved_hash
    }

    /// Submits a command as the human actor. Failures become readable messages.
    pub fn submit(&mut self, kind: CommandKind) -> Result<Outcome, Diagnostic> {
        let base = format!("ui-{}", self.authority.revision().0 + 1);
        let mut attempt = 0;
        loop {
            let id = if attempt == 0 {
                base.clone()
            } else {
                format!("{base}-{attempt}")
            };
            let cmd = Command {
                id,
                actor: self.actor.clone(),
                base_revision: self.authority.revision(),
                kind: kind.clone(),
            };
            match self.authority.submit(cmd) {
                Ok(o) => {
                    self.after_change();
                    let n = o.changes.len();
                    self.info(format!(
                        "revision {}: {n} change{}",
                        o.revision.0,
                        if n == 1 { "" } else { "s" }
                    ));
                    return Ok(o);
                }
                Err(d) if d.code == DiagCode::DuplicateCommand && attempt < 5 => attempt += 1,
                Err(d) => {
                    let note = if d.code == DiagCode::Unsolvable {
                        self.note_conflict(&kind, &d)
                    } else {
                        None
                    };
                    match note {
                        Some(text) => self.error(text, d.suggestion.clone()),
                        None => self.report(&d),
                    }
                    return Err(d);
                }
            }
        }
    }

    fn after_change(&mut self) {
        if let Some(s) = &mut self.sketching {
            s.conflict.clear();
            s.check = None;
            s.conflict_relations.clear();
            s.conflict_note = None;
        }
        self.refresh_scene();
        self.drop_session_if_gone();
        // A selection can outlive what it pointed at.
        let gone = match &self.selected {
            Selected::None => false,
            Selected::Sketch(s) | Selected::SketchEntity { sketch: s, .. } => {
                self.authority.state().sketch(s).is_none()
            }
            Selected::Feature(f)
            | Selected::Face { feature: f, .. }
            | Selected::Edge { feature: f, .. } => self.authority.state().feature(f).is_none(),
        };
        if gone {
            self.selected = Selected::None;
        }
        self.view_version += 1;
    }

    pub fn undo(&mut self) -> Result<Outcome, Diagnostic> {
        self.submit(CommandKind::HistoryUndo { revision: None })
    }

    pub fn redo(&mut self) -> Result<Outcome, Diagnostic> {
        self.submit(CommandKind::HistoryRedo { revision: None })
    }

    /// The sketch that is selected or being edited: its outline is drawn even when a feature uses it.
    pub fn shown_sketch(&self) -> Option<String> {
        // A feature being set up shows its sketch, so its lines can be picked (a revolve axis).
        if let Some(d) = self
            .feature_draft
            .as_ref()
            .filter(|d| d.kind.needs_sketch())
        {
            if self.sketching.is_none() {
                return Some(d.sketch.clone());
            }
        }
        match (&self.sketching, &self.selected) {
            (Some(s), _) => Some(s.sketch.clone()),
            (None, Selected::Sketch(s) | Selected::SketchEntity { sketch: s, .. }) => {
                Some(s.clone())
            }
            _ => None,
        }
    }

    /// Rebuilds the drawn scene if the document changed.
    pub fn refresh_scene(&mut self) {
        let key = (self.authority.revision().0, self.authority.state_hash());
        let hidden = self.hide_bodies && self.sketching.is_some();
        if self.scene_for != Some(key)
            || self.scene_hidden != hidden
            || self.scene_preview != self.preview_version
            || self.scene_shown != self.shown_sketch()
        {
            self.scene = scene_of_authority(&self.authority);
            // The document as it is, before a drafted feature replaces it: edges are picked on
            // this, so a rounded edge can still be clicked to take it back out.
            self.base_scene = self.scene.clone();
            // A feature being set up replaces the bodies by what the document would show, with
            // the new one in an accent colour.
            if let (Some(p), Some(d)) = (&self.feature_preview, &self.feature_draft) {
                if let Ok(solids) = &p.result {
                    self.scene.bodies = solids
                        .iter()
                        .filter_map(|(id, solid)| {
                            let color = if *id == d.name {
                                [255, 170, 60]
                            } else {
                                [150, 165, 185]
                            };
                            dc_render::Body::from_solid(id, solid, color).ok()
                        })
                        .collect();
                }
            }
            if hidden {
                self.scene.bodies.clear();
            }
            // As in SolidWorks, a sketch that a feature was made from is put away: it shows
            // while it is selected or being edited, and sketches nothing uses stay visible.
            let shown = self.shown_sketch();
            let doc = self.authority.state();
            // What a refused feature page is about is drawn in red, whatever else is put away.
            let blamed: Vec<String> = self
                .feature_preview
                .as_ref()
                .filter(|p| p.result.is_err())
                .map(|p| p.entities.clone())
                .unwrap_or_default();
            let blames = |line: &str| {
                blamed
                    .iter()
                    .any(|e| line == e || line.starts_with(&format!("{e}/")))
            };
            self.scene.lines.retain(|l| {
                let sketch = l.id.split('/').nth(1).unwrap_or("");
                blames(&l.id)
                    || shown.as_deref() == Some(sketch)
                    || doc.features_of_sketch(sketch).is_empty()
            });
            for l in &mut self.scene.lines {
                if blames(&l.id) {
                    l.color = [220, 40, 40];
                }
            }
            for b in &mut self.scene.bodies {
                if blamed.iter().any(|e| *e == format!("feature/{}", b.id)) {
                    b.color = [220, 80, 70];
                }
            }
            self.scene_shown = shown;
            self.scene_preview = self.preview_version;
            self.scene_for = Some(key);
            self.scene_hidden = hidden;
            self.scene_version += 1;
        }
    }

    // ---- reading the document for the panels --------------------------------------------

    pub fn sketch_rows(&self) -> Vec<SketchRow> {
        match self.authority.query(&Query::Sketches {
            prefix: String::new(),
            after: None,
            limit: Some(1000),
        }) {
            Ok(QueryResult::Sketches { items, .. }) => items
                .into_iter()
                .map(|info| SketchRow {
                    id: info.id.clone(),
                    info,
                })
                .collect(),
            _ => Vec::new(),
        }
    }

    pub fn feature_rows(&self) -> Vec<FeatureRow> {
        match self.authority.query(&Query::Features {
            prefix: String::new(),
            after: None,
            limit: Some(1000),
        }) {
            Ok(QueryResult::Features { items, .. }) => items
                .into_iter()
                .map(|f| {
                    let (target, depth) = match &f.spec {
                        FeatureSpec::Extrude { depth, .. } | FeatureSpec::Boss { depth, .. } => {
                            (None, Some(*depth))
                        }
                        FeatureSpec::Cut { depth, .. } => (None, *depth),
                        FeatureSpec::Revolve { .. } => (None, None),
                        FeatureSpec::Fillet { radius, .. } => (None, Some(*radius)),
                        FeatureSpec::Chamfer { distance, .. } => (None, Some(*distance)),
                        FeatureSpec::Shell { thickness, .. } => (None, Some(*thickness)),
                        FeatureSpec::Loft { .. } | FeatureSpec::Sweep { .. } => (None, None),
                    };
                    let target = f.spec.target().map(str::to_owned).or(target);
                    FeatureRow {
                        id: f.id,
                        kind: f.kind,
                        target,
                        sketch: f.spec.sketch().unwrap_or_default().to_owned(),
                        volume: f.summary.volume,
                        depth,
                    }
                })
                .collect(),
            _ => Vec::new(),
        }
    }

    pub fn faces_of(&self, feature: &str) -> Vec<FaceInfo> {
        match self.authority.query(&Query::FeatureFaces {
            feature: feature.into(),
            after: None,
            limit: Some(1000),
        }) {
            Ok(QueryResult::FeatureFaces { items, .. }) => items,
            _ => Vec::new(),
        }
    }

    pub fn constraints_of(&self, sketch: &str) -> Vec<(String, Constraint)> {
        match self.authority.query(&Query::SketchConstraints {
            sketch: sketch.into(),
            after: None,
            limit: Some(1000),
        }) {
            Ok(QueryResult::SketchConstraints { items, .. }) => items,
            _ => Vec::new(),
        }
    }

    // ---- editing (always through commands) ----------------------------------------------

    pub fn set_depth(&mut self, feature: &str, depth: f64) -> Result<Outcome, Diagnostic> {
        let current = self
            .authority
            .state()
            .feature(feature)
            .map(|f| match &f.spec {
                FeatureSpec::Extrude { depth, .. } | FeatureSpec::Boss { depth, .. } => {
                    Some(*depth)
                }
                FeatureSpec::Cut { depth, .. } => *depth,
                FeatureSpec::Revolve { .. } => None,
                FeatureSpec::Fillet { radius, .. } => Some(*radius),
                FeatureSpec::Chamfer { distance, .. } => Some(*distance),
                FeatureSpec::Shell { thickness, .. } => Some(*thickness),
                FeatureSpec::Loft { .. } | FeatureSpec::Sweep { .. } => None,
            });
        let expected = match current {
            Some(Some(d)) => Expected::Value(d),
            _ => Expected::Any,
        };
        self.submit(CommandKind::FeatureSetDepth {
            feature: feature.into(),
            depth,
            expected,
        })
    }

    pub fn set_dimension(
        &mut self,
        sketch: &str,
        constraint: &str,
        value: f64,
    ) -> Result<Outcome, Diagnostic> {
        let expected = self
            .constraints_of(sketch)
            .iter()
            .find(|(id, _)| id == constraint)
            .and_then(|(_, c)| c.value())
            .map_or(Expected::Any, Expected::Value);
        self.submit(CommandKind::SketchSetValue {
            sketch: sketch.into(),
            constraint_id: constraint.into(),
            value,
            expected,
        })
    }

    pub fn delete_selected(&mut self) -> Result<Outcome, Diagnostic> {
        // Several features or sketches selected together go in one step, the later ones first.
        let all = self.selection();
        if all.len() > 1
            && all
                .iter()
                .all(|s| matches!(s, Selected::Feature(_) | Selected::Sketch(_)))
        {
            let order = self.authority.state().ordered_features();
            let rank = |f: &str| order.iter().position(|x| x == f).unwrap_or(0);
            let mut features: Vec<String> = all
                .iter()
                .filter_map(|s| match s {
                    Selected::Feature(f) => Some(f.clone()),
                    _ => None,
                })
                .collect();
            features.sort_by_key(|f| std::cmp::Reverse(rank(f)));
            let mut ops: Vec<CommandKind> = features
                .into_iter()
                .map(|feature| CommandKind::FeatureDelete { feature })
                .collect();
            ops.extend(all.iter().filter_map(|s| match s {
                Selected::Sketch(sketch) => Some(CommandKind::SketchDelete {
                    sketch: sketch.clone(),
                }),
                _ => None,
            }));
            return self.submit(CommandKind::Transaction {
                label: "Delete".into(),
                ops,
            });
        }
        match self.selected.clone() {
            Selected::Feature(f) => self.submit(CommandKind::FeatureDelete { feature: f }),
            Selected::Sketch(s) => self.submit(CommandKind::SketchDelete { sketch: s }),
            Selected::Face { feature, .. } | Selected::Edge { feature, .. } => {
                Err(Diagnostic::new(
                    DiagCode::InvalidValue,
                    format!("a face or an edge cannot be deleted; select the feature '{feature}' to delete that"),
                )
                .suggest("click the feature in the tree, then press Delete"))
            }
            Selected::SketchEntity { sketch, .. } => Err(Diagnostic::new(
                DiagCode::InvalidValue,
                format!("edit sketch '{sketch}' to delete one of its lines"),
            )),
            Selected::None => Err(Diagnostic::new(DiagCode::NotFound, "nothing is selected")),
        }
    }

    /// Runs a line typed into the console: `undo`, `redo`, `fit`, or one JSON command (the same
    /// JSON an agent would send, without the envelope).
    pub fn run_console(&mut self, line: &str) {
        let line = line.trim();
        match line {
            "" => {}
            "undo" => {
                let _ = self.undo();
            }
            "redo" => {
                let _ = self.redo();
            }
            "fit" => self.fit(),
            _ => match serde_json::from_str::<CommandKind>(line) {
                Ok(kind) => {
                    let _ = self.submit(kind);
                }
                Err(e) => self.error(
                    format!("not a command: {e}"),
                    Some("type undo, redo, fit, or a JSON command like {\"type\":\"param.set\",\"name\":\"x\",\"value\":1}".into()),
                ),
            },
        }
    }

    // ---- files ----------------------------------------------------------------------------

    pub fn open(&mut self, path: &Path) -> Result<(), String> {
        let bytes =
            std::fs::read(path).map_err(|e| format!("cannot read {}: {e}", path.display()))?;
        let d = dc_io::read(&bytes, Policy::default())
            .map_err(|e| format!("{}: {e}", path.display()))?;
        self.end_sketch();
        self.authority = d.authority;
        self.extras = d.extras;
        self.path = Some(path.to_owned());
        self.saved_hash = self.authority.state_hash();
        self.selected = Selected::None;
        self.hovered = None;
        self.scene_for = None;
        self.refresh_scene();
        self.fit();
        self.info(format!(
            "opened {} (revision {})",
            path.display(),
            self.authority.revision().0
        ));
        Ok(())
    }

    /// Saves atomically (write a temp file, then rename).
    pub fn save(&mut self, path: &Path) -> Result<(), String> {
        let bytes = dc_io::write(&self.authority, &self.extras);
        let tmp = path.with_extension("dcad.tmp");
        std::fs::write(&tmp, &bytes).map_err(|e| format!("cannot write {}: {e}", tmp.display()))?;
        std::fs::rename(&tmp, path)
            .map_err(|e| format!("cannot finish {}: {e}", path.display()))?;
        self.path = Some(path.to_owned());
        self.saved_hash = self.authority.state_hash();
        self.view_version += 1;
        self.info(format!("saved {} ({} bytes)", path.display(), bytes.len()));
        Ok(())
    }

    // ---- the view ---------------------------------------------------------------------------

    /// Tells the app the viewport's shape. The first time, the view is fitted to it (the camera
    /// was fitted before the window existed); afterwards the user's camera is left alone.
    pub fn set_viewport_aspect(&mut self, aspect: f64) {
        let old = self.viewport_aspect.replace(aspect);
        let changed = old.is_none_or(|o| (o - aspect).abs() > 0.02 * o.max(1e-9));
        if changed && self.auto_fit {
            self.fit();
        }
    }

    pub fn fit(&mut self) {
        self.auto_fit = true;
        let aspect = self.viewport_aspect.unwrap_or(4.0 / 3.0);
        if let Some((lo, hi)) = self.scene.body_bounds().or_else(|| self.scene.bounds()) {
            self.camera.fit_for(lo, hi, aspect);
        } else {
            self.camera.target = dc_math::Vec3::ZERO;
            self.camera.distance = 200.0;
        }
        self.view_version += 1;
    }

    pub fn set_view(&mut self, preset: ViewPreset) {
        let projection = self.camera.projection;
        self.camera = Camera::preset(preset);
        self.camera.projection = projection;
        self.fit();
    }

    pub fn toggle_projection(&mut self) {
        self.camera.projection = match self.camera.projection {
            ProjectionKind::Perspective => ProjectionKind::Orthographic,
            ProjectionKind::Orthographic => ProjectionKind::Perspective,
        };
        self.view_version += 1;
    }

    /// Drag to orbit: the scene follows the cursor.
    pub fn orbit_px(&mut self, dx: f64, dy: f64) {
        self.camera.orbit(-dx * ORBIT_PER_PX, dy * ORBIT_PER_PX);
        self.auto_fit = false;
        self.view_version += 1;
    }

    pub fn pan_px(&mut self, dx: f64, dy: f64, viewport_h: f64) {
        self.camera.pan(dx, dy, viewport_h);
        self.auto_fit = false;
        self.view_version += 1;
    }

    /// Scroll to zoom; positive scroll moves toward the part.
    pub fn zoom_scroll(&mut self, scroll: f64) {
        self.camera.zoom(det::exp(-scroll * 0.0015));
        self.auto_fit = false;
        self.view_version += 1;
    }

    // ---- picking ----------------------------------------------------------------------------

    pub fn pick_at(&self, px: f64, py: f64, w: f64, h: f64) -> Option<FaceRef> {
        pick(&self.scene, &self.camera, px, py, w, h).map(|p| FaceRef {
            body: p.body,
            face: p.face,
        })
    }

    /// Updates which face is under the cursor (`None` = cursor left the viewport or is over nothing).
    pub fn hover_at(&mut self, at: Option<(f64, f64, f64, f64)>) {
        let hit = at.and_then(|(x, y, w, h)| self.pick_hit_at(x, y, w, h));
        let (face, other) = match hit {
            Some(Hit::Face(f)) => (
                Some(FaceRef {
                    body: f.body,
                    face: f.face,
                }),
                None,
            ),
            Some(Hit::Edge { body, name, .. }) => (
                None,
                Some(Hover::Edge {
                    feature: body,
                    edge: name,
                }),
            ),
            Some(Hit::SketchLine { sketch, entity, .. }) => {
                (None, Some(Hover::SketchEntity { sketch, entity }))
            }
            None => (None, None),
        };
        if face != self.hovered || other != self.hover_other {
            self.hovered = face;
            self.hover_other = other;
            self.view_version += 1;
        }
    }

    pub fn click_at(&mut self, px: f64, py: f64, w: f64, h: f64) {
        self.click_view(px, py, w, h, false);
    }

    /// What a hit selects.
    fn selected_of(hit: Hit) -> Selected {
        match hit {
            Hit::Face(f) => Selected::Face {
                feature: f.body,
                face: f.face,
            },
            Hit::Edge { body, name, .. } => Selected::Edge {
                feature: body,
                edge: name,
            },
            Hit::SketchLine { sketch, entity, .. } => Selected::SketchEntity { sketch, entity },
        }
    }

    /// Everything selected, the one chosen last at the end.
    pub fn selection(&self) -> Vec<Selected> {
        if self.selected == Selected::None {
            return Vec::new();
        }
        let mut v = if self.extra_for == self.selected {
            self.extra.clone()
        } else {
            Vec::new()
        };
        v.push(self.selected.clone());
        v
    }

    /// Replaces the selection by `list` (the last one becomes the primary).
    pub fn set_selection(&mut self, mut list: Vec<Selected>) {
        let mut seen: Vec<Selected> = Vec::new();
        list.retain(|s| {
            let fresh = *s != Selected::None && !seen.contains(s);
            if fresh {
                seen.push(s.clone());
            }
            fresh
        });
        let primary = list.pop().unwrap_or(Selected::None);
        if self.selected != primary || self.extra != list {
            self.selected = primary.clone();
            self.extra = list;
            self.extra_for = primary;
            self.view_version += 1;
        }
    }

    /// Ctrl-click: adds `s` to the selection, or takes it out if it is already there.
    pub fn toggle_select(&mut self, s: Selected) {
        let mut cur = self.selection();
        match cur.iter().position(|x| *x == s) {
            Some(at) => {
                cur.remove(at);
            }
            None => cur.push(s),
        }
        self.set_selection(cur);
    }

    /// Does the selection already touch `feature` (the feature, or one of its faces or edges)?
    fn selection_in(&self, feature: &str) -> bool {
        self.selection().iter().any(|s| match s {
            Selected::Feature(f)
            | Selected::Face { feature: f, .. }
            | Selected::Edge { feature: f, .. } => f == feature,
            _ => false,
        })
    }

    /// A click in the 3D view at pixel `(px, py)`. With `additive` (Ctrl) the thing under the
    /// pointer is added to the selection, or taken out if it is in it; without, it replaces it.
    /// In feature-first mode the first click on a part picks the feature and the next one
    /// drills down to the face or edge.
    pub fn click_view(&mut self, px: f64, py: f64, w: f64, h: f64, additive: bool) {
        // While a feature is being set up, a click inside a region of its sketch picks it.
        if self.draft_click(px, py, w, h) {
            self.update_feature_preview();
            self.view_version += 1;
            return;
        }
        let hit = self.pick_hit_at(px, py, w, h).map(AppState::selected_of);
        let hit = match (hit, self.select_mode) {
            (
                Some(Selected::Face { feature, .. } | Selected::Edge { feature, .. }),
                SelectMode::FeatureFirst,
            ) if !self.selection_in(&feature) => Some(Selected::Feature(feature)),
            (h, _) => h,
        };
        match (hit, additive) {
            (Some(s), true) => self.toggle_select(s),
            (Some(s), false) => self.set_selection(vec![s]),
            (None, true) => {}
            (None, false) => self.set_selection(Vec::new()),
        }
        self.view_version += 1;
    }

    /// A rubber band over the view, `rect` in pixels `(x0, y0, x1, y1)` with the corners in any
    /// order. Dragged to the right (`crossing` false) it takes what lies wholly inside, to the
    /// left what it touches. With `add` it joins the selection, else it replaces it. Only what
    /// the selection filter allows, and only what can be seen.
    pub fn box_select(
        &mut self,
        rect: (f64, f64, f64, f64),
        crossing: bool,
        add: bool,
        w: f64,
        h: f64,
    ) {
        let r = (
            rect.0.min(rect.2),
            rect.1.min(rect.3),
            rect.0.max(rect.2),
            rect.1.max(rect.3),
        );
        let hits = dc_render::box_hits(
            self.pick_scene(),
            &self.camera,
            r,
            crossing,
            w,
            h,
            self.pick_filter(),
        );
        let mut list = if add { self.selection() } else { Vec::new() };
        list.extend(hits.into_iter().map(AppState::selected_of));
        self.set_selection(list);
    }

    /// What is under the pointer, as far as the selection filter allows. While a command is
    /// being set up it only offers what that command can use.
    pub fn pick_hit_at(&self, px: f64, py: f64, w: f64, h: f64) -> Option<Hit> {
        pick_hit(
            self.pick_scene(),
            &self.camera,
            px,
            py,
            w,
            h,
            self.pick_filter(),
            7.0,
        )
    }

    /// What picking looks at: the document itself while edges are being chosen for rounding
    /// (the preview has already rounded them), else what is drawn.
    pub fn pick_scene(&self) -> &Scene {
        if self
            .feature_draft
            .as_ref()
            .is_some_and(|d| d.kind.rounds_edges())
        {
            &self.base_scene
        } else {
            &self.scene
        }
    }

    /// The selection filter in force: the user's, narrowed by the command being set up.
    pub fn pick_filter(&self) -> Filter {
        match self.feature_draft.as_ref().map(|d| d.kind) {
            Some(FeatureKind::Fillet | FeatureKind::Chamfer) => Filter::EDGES,
            Some(FeatureKind::Revolve) => Filter {
                faces: false,
                edges: false,
                sketch_lines: true,
            },
            Some(_) => Filter {
                faces: false,
                edges: false,
                sketch_lines: false,
            },
            None => self.filter,
        }
    }

    pub fn select(&mut self, s: Selected) {
        self.set_selection(vec![s]);
    }

    /// The face to draw highlighted.
    pub fn selected_face_ref(&self) -> Option<FaceRef> {
        match &self.selected {
            Selected::Face { feature, face } => Some(FaceRef {
                body: feature.clone(),
                face: face.clone(),
            }),
            _ => None,
        }
    }

    /// The sketch plane while sketching: its origin is marked, and its grid shown if switched on.
    pub fn local_grid(&self) -> Option<dc_render::raster::GridPlane> {
        let s = self.sketching.as_ref()?;
        let f = &s.frame;
        let v = |a: [f64; 3]| dc_math::Vec3::new(a[0], a[1], a[2]);
        Some(dc_render::raster::GridPlane {
            origin: v(f.origin),
            x: v(f.x_axis),
            y: v(f.y_axis),
            grid: self.show_sketch_grid,
        })
    }

    pub fn render(&self, width: u32, height: u32, supersample: u32) -> Image {
        let mut o = RenderOptions::new(width, height, self.camera);
        o.mode = RenderMode::Shaded;
        o.grid = false;
        o.ambient_occlusion = self.ambient_occlusion;
        o.axes = self.show_origins;
        o.local_grid = self.local_grid();
        o.edges = self.show_edges;
        o.sketches = self.show_sketches;
        o.supersample = supersample;
        o.selected = self.selected_face_ref();
        o.hovered = self.hovered.clone();
        dc_render::render(&self.scene, &o).image
    }

    // ---- a sample part ------------------------------------------------------------------

    /// Builds a small parametric part through ordinary commands (so it is in the history and can
    /// be undone like anything else): a plate, a boss on its top face, a pocket in the boss and a
    /// counterbored hole.
    pub fn load_demo(&mut self) {
        let sketch = |s: &str, plane: serde_json::Value| serde_json::json!({"type":"sketch.create","sketch":s,"plane":plane});
        let pt = |s: &str, id: &str, x: f64, y: f64| serde_json::json!({"type":"sketch.add_entity","sketch":s,"entity_id":id,"entity":{"type":"point","x":x,"y":y}});
        let line = |s: &str, id: &str, a: &str, b: &str| serde_json::json!({"type":"sketch.add_entity","sketch":s,"entity_id":id,"entity":{"type":"line","a":a,"b":b}});
        let con = |s: &str, id: &str, c: serde_json::Value| serde_json::json!({"type":"sketch.add_constraint","sketch":s,"constraint_id":id,"constraint":c});
        let circle = |s: &str, c: &str, r: f64| {
            vec![
                serde_json::json!({"type":"sketch.add_entity","sketch":s,"entity_id":"c","entity":{"type":"circle","center":c,"radius":r}}),
                con(
                    s,
                    "r",
                    serde_json::json!({"type":"radius","target":"c","value":r}),
                ),
            ]
        };
        let rect = |s: &str, plane: serde_json::Value, x0: f64, y0: f64, x1: f64, y1: f64| {
            let mut ops = vec![sketch(s, plane)];
            for (i, (x, y)) in [(x0, y0), (x1, y0), (x1, y1), (x0, y1)].iter().enumerate() {
                ops.push(pt(s, &format!("p{i}"), *x, *y));
            }
            for i in 0..4 {
                ops.push(line(
                    s,
                    &format!("e{i}"),
                    &format!("p{i}"),
                    &format!("p{}", (i + 1) % 4),
                ));
            }
            ops.push(con(
                s,
                "ha",
                serde_json::json!({"type":"horizontal","line":"e0"}),
            ));
            ops.push(con(
                s,
                "vb",
                serde_json::json!({"type":"vertical","line":"e1"}),
            ));
            ops.push(con(
                s,
                "hc",
                serde_json::json!({"type":"horizontal","line":"e2"}),
            ));
            ops.push(con(
                s,
                "vd",
                serde_json::json!({"type":"vertical","line":"e3"}),
            ));
            ops.push(con(
                s,
                "width",
                serde_json::json!({"type":"length","line":"e0","value":x1 - x0}),
            ));
            ops.push(con(
                s,
                "height",
                serde_json::json!({"type":"length","line":"e1","value":y1 - y0}),
            ));
            ops.push(con(
                s,
                "anchor",
                serde_json::json!({"type":"fixed","point":"p0","x":x0,"y":y0}),
            ));
            ops
        };
        let top = |f: &str, face: &str| serde_json::json!({"base":"face","feature":f,"face":face});
        let mut steps: Vec<serde_json::Value> = Vec::new();
        let mut base = rect(
            "base",
            serde_json::json!({"base":"xy"}),
            0.0,
            0.0,
            100.0,
            60.0,
        );
        base.push(serde_json::json!({"type":"feature.extrude","feature":"plate","sketch":"base","profiles":["profile.e0"],"depth":12}));
        steps.push(serde_json::json!({"type":"transaction","label":"plate","ops":base}));
        let mut tower = rect("tower", top("plate", "cap_end"), 55.0, 12.0, 90.0, 48.0);
        tower.push(serde_json::json!({"type":"feature.boss","feature":"boss","target":"plate","sketch":"tower","profiles":["profile.e0"],"depth":14}));
        steps.push(serde_json::json!({"type":"transaction","label":"boss","ops":tower}));
        let mut pocket = vec![
            sketch("pocket", top("boss", "cap_end:boss")),
            pt("pocket", "m", 72.5, 30.0),
            con(
                "pocket",
                "fx",
                serde_json::json!({"type":"fixed","point":"m","x":72.5,"y":30.0}),
            ),
        ];
        pocket.extend(circle("pocket", "m", 9.0));
        pocket.push(serde_json::json!({"type":"feature.cut","feature":"pocket_cut","target":"boss","sketch":"pocket","profiles":["profile.c"],"depth":6,"reverse":true}));
        steps.push(serde_json::json!({"type":"transaction","label":"pocket","ops":pocket}));
        let mut hole = vec![
            sketch("bore", top("plate", "cap_end")),
            pt("bore", "m", 25.0, 30.0),
            con(
                "bore",
                "fx",
                serde_json::json!({"type":"fixed","point":"m","x":25.0,"y":30.0}),
            ),
        ];
        hole.extend(circle("bore", "m", 7.0));
        hole.push(serde_json::json!({"type":"feature.cut","feature":"hole","target":"pocket_cut","sketch":"bore","profiles":["profile.c"],"reverse":true}));
        steps.push(serde_json::json!({"type":"transaction","label":"hole","ops":hole}));
        for s in steps {
            match serde_json::from_value::<CommandKind>(s) {
                Ok(k) => {
                    if self.submit(k).is_err() {
                        return;
                    }
                }
                Err(e) => {
                    self.error(format!("demo is malformed: {e}"), None);
                    return;
                }
            }
        }
        self.messages.clear();
        self.info("loaded the demo part: try editing a depth or a dimension, then undo");
        self.fit();
    }
}
