//! Agent-facing test hooks: widget tags and an app-state snapshot for `eguidev`. With the
//! `devtools` feature off (the default) the tags cost one `Option` check per widget and nothing is
//! served; with it on, `edev mcp` (Claude Code) can list widgets by id, click, type, press keys,
//! inject pointer events in the 3D view, take screenshots and read the snapshot.

use crate::state::{AppState, Selected};
use eguidev::{
    track_response_full, Anchor, AnchorCheck, DevMcp, FixtureResponse, FixtureSpec, WidgetMeta,
    WidgetRole, WidgetValue,
};
use std::sync::atomic::{AtomicI64, Ordering};
use std::sync::{Arc, Mutex};

/// What an agent can read without looking at pixels: the app's state as data.
#[derive(Clone, Default)]
pub struct Snapshot(Arc<Mutex<serde_json::Value>>);

impl Snapshot {
    pub fn set(&self, v: serde_json::Value) {
        if let Ok(mut g) = self.0.lock() {
            *g = v;
        }
    }

    pub fn get(&self) -> serde_json::Value {
        self.0.lock().map(|g| g.clone()).unwrap_or_default()
    }
}

/// The handle and the snapshot the app keeps.
pub struct Dev {
    pub handle: DevMcp,
    pub snapshot: Snapshot,
    /// Fixture names asked for by a script, applied on the next frame.
    queue: Arc<Mutex<Vec<String>>>,
    /// How many fixtures were asked for, and how many have been applied (shown by the widget
    /// `doc.epoch`, which a fixture's anchor waits for, so a script never reads stale state).
    #[allow(dead_code)] // shared with the fixture handler, which counts requests
    asked: Arc<AtomicI64>,
    applied: Arc<AtomicI64>,
}

impl Dev {
    /// An inert handle (until the runtime is attached) with the `app` diagnostic registered.
    pub fn new() -> Dev {
        let snapshot = Snapshot::default();
        let s = snapshot.clone();
        let queue: Arc<Mutex<Vec<String>>> = Arc::default();
        let q = queue.clone();
        let asked: Arc<AtomicI64> = Arc::default();
        let applied: Arc<AtomicI64> = Arc::default();
        let a2 = asked.clone();
        let anchor = |id: &str| Anchor {
            widget_id: id.to_owned(),
            viewport_id: None,
            check: AnchorCheck::Visible,
        };
        let spec = |name: &str, what: &str| {
            let mut f = FixtureSpec::new(name, what);
            f.anchors.push(anchor("tab.features"));
            f
        };
        let handle = DevMcp::new()
            .fixtures([
                spec("empty", "A new empty document"),
                spec("demo", "The small demo part: plate, boss, pocket and holes"),
                spec("showcase", "The showcase part with curved and flat faces"),
                spec(
                    "plate_with_four_holes",
                    "A 40 x 20 x 10 plate (feature plate) and an unused sketch `holes` with four circles of radius 2",
                ),
                spec(
                    "two_sketches",
                    "No features; sketch `low` (a 40 x 20 rectangle at z=0), sketch `high` (a 20 x 10 rectangle at z=30) and sketch `path` (front plane, one line from (0,0) to (0,25))",
                ),
                spec(
                    "shaft",
                    "A cylinder of radius 10 and height 20 (feature shaft, sketch base)",
                ),
            ])
            .on_fixture_ui(move |_ctx, call| {
                if let Ok(mut g) = q.lock() {
                    g.push(call.name.clone());
                }
                let n = a2.fetch_add(1, Ordering::SeqCst) + 1;
                Ok(FixtureResponse {
                    anchors: vec![Anchor {
                        widget_id: "doc.epoch".to_owned(),
                        viewport_id: None,
                        check: AnchorCheck::Value(WidgetValue::Int(n)),
                    }],
                    ..Default::default()
                })
            })
            .and_then(|h| h.diagnostic("app", move || Ok(s.get())))
            .unwrap_or_else(|_| DevMcp::new());
        Dev {
            handle,
            snapshot,
            queue,
            asked,
            applied,
        }
    }

    /// Turns the embedded runtime on (the `devtools` feature); a no-op otherwise.
    pub fn attach(self) -> Dev {
        #[cfg(feature = "devtools")]
        let handle = eguidev_runtime::attach(self.handle);
        #[cfg(not(feature = "devtools"))]
        let handle = self.handle;
        Dev { handle, ..self }
    }

    /// Applies the fixtures scripts asked for since the last frame.
    pub fn apply_fixtures(&self, app: &mut AppState) {
        let names: Vec<String> = self
            .queue
            .lock()
            .map(|mut q| std::mem::take(&mut *q))
            .unwrap_or_default();
        for n in names {
            *app = AppState::new();
            match n.as_str() {
                "demo" => app.load_demo(),
                "showcase" => app.load_showcase(),
                "plate_with_four_holes" => run_all(app, &plate_with_four_holes()),
                "shaft" => run_all(app, &shaft()),
                "two_sketches" => run_all(app, &two_sketches()),
                _ => {}
            }
            self.applied.fetch_add(1, Ordering::SeqCst);
        }
    }

    /// Shows the count of applied fixtures as the widget `doc.epoch` (a one-point rect).
    pub fn tag_epoch(&self, ui: &eframe::egui::Ui) {
        let r = eframe::egui::Rect::from_min_size(ui.max_rect().min, eframe::egui::vec2(1.0, 1.0));
        eguidev::publish_rect_meta(
            ui,
            "doc.epoch",
            r,
            WidgetMeta {
                role: WidgetRole::Label,
                value: Some(WidgetValue::Int(self.applied.load(Ordering::SeqCst))),
                visible: true,
                ..Default::default()
            },
        );
    }

    /// Publishes the state an agent may ask for. Called once per frame.
    pub fn publish(&self, app: &AppState, extra: serde_json::Value) {
        let one = |s: &Selected| -> serde_json::Value {
            match s {
                Selected::None => serde_json::json!({"kind": "none"}),
                Selected::Sketch(s) => serde_json::json!({"kind": "sketch", "sketch": s}),
                Selected::Feature(f) => serde_json::json!({"kind": "feature", "feature": f}),
                Selected::Face { feature, face } => {
                    serde_json::json!({"kind": "face", "feature": feature, "face": face})
                }
                Selected::Edge { feature, edge } => {
                    serde_json::json!({"kind": "edge", "feature": feature, "edge": edge})
                }
                Selected::SketchEntity { sketch, entity } => {
                    serde_json::json!({"kind": "sketch_entity", "sketch": sketch, "entity": entity})
                }
            }
        };
        let selection: Vec<serde_json::Value> = app.selection().iter().map(one).collect();
        let selected = match &app.selected {
            Selected::None => serde_json::json!({"kind": "none"}),
            Selected::Sketch(s) => serde_json::json!({"kind": "sketch", "sketch": s}),
            Selected::Feature(f) => serde_json::json!({"kind": "feature", "feature": f}),
            Selected::Face { feature, face } => {
                serde_json::json!({"kind": "face", "feature": feature, "face": face})
            }
            Selected::Edge { feature, edge } => {
                serde_json::json!({"kind": "edge", "feature": feature, "edge": edge})
            }
            Selected::SketchEntity { sketch, entity } => {
                serde_json::json!({"kind": "sketch_entity", "sketch": sketch, "entity": entity})
            }
        };
        let state = app.authority.state();
        let draft = app.feature_draft.as_ref().map(|d| {
            serde_json::json!({
                "kind": d.kind.name(), "name": d.name, "editing": d.editing,
                "sketch": d.sketch, "profiles": d.profiles, "edges": d.edges,
                "size": d.size, "depth": d.depth, "axis": d.axis, "reverse": d.reverse,
                "problem": app.draft_problem(),
            })
        });
        self.snapshot.set(serde_json::json!({
            "revision": app.authority.revision().0,
            "state_hash": format!("{:?}", app.authority.state_hash()),
            "features": state.ordered_features(),
            "result": state.result_features(),
            "hidden": state.hidden_features(),
            "sketches": app.sketch_rows().iter().map(|r| r.id.clone()).collect::<Vec<_>>(),
            "selected": selected,
            "selection": selection,
            "select_mode": format!("{:?}", app.select_mode),
            "sketching": app.sketching.as_ref().map(|s| s.sketch.clone()),
            "draft": draft,
            "messages": app.messages.iter().rev().take(8).map(|m| m.text.clone()).collect::<Vec<_>>(),
            "camera_distance": app.camera.distance,
            "volumes": app.feature_rows().iter().map(|r| (r.id.clone(), r.volume)).collect::<std::collections::BTreeMap<_, _>>(),
            "ui": extra,
        }));
    }
}

impl Default for Dev {
    fn default() -> Self {
        Dev::new()
    }
}

fn run_all(app: &mut AppState, lines: &[serde_json::Value]) {
    for l in lines {
        app.run_console(&l.to_string());
    }
}

fn plate_with_four_holes() -> Vec<serde_json::Value> {
    use serde_json::json;
    let mut v =
        vec![json!({"type":"sketch.create","sketch":"base","plane":{"base":"xy","offset":0}})];
    for (i, (x, y)) in [(0, 0), (40, 0), (40, 20), (0, 20)].iter().enumerate() {
        v.push(json!({"type":"sketch.add_entity","sketch":"base","entity_id":format!("p{i}"),"entity":{"type":"point","x":x,"y":y}}));
    }
    for i in 0..4 {
        v.push(json!({"type":"sketch.add_entity","sketch":"base","entity_id":format!("l{i}"),"entity":{"type":"line","a":format!("p{i}"),"b":format!("p{}", (i+1)%4)}}));
    }
    v.push(json!({"type":"feature.extrude","feature":"plate","sketch":"base","depth":10}));
    v.push(json!({"type":"sketch.create","sketch":"holes","plane":{"base":"xy","offset":0}}));
    for (i, (x, y)) in [(10, 5), (30, 5), (10, 15), (30, 15)].iter().enumerate() {
        v.push(json!({"type":"sketch.add_entity","sketch":"holes","entity_id":format!("m{i}"),"entity":{"type":"point","x":x,"y":y}}));
        v.push(json!({"type":"sketch.add_entity","sketch":"holes","entity_id":format!("c{i}"),"entity":{"type":"circle","center":format!("m{i}"),"radius":2}}));
    }
    v
}

fn two_sketches() -> Vec<serde_json::Value> {
    use serde_json::json;
    let mut v = Vec::new();
    for (name, z, w, h) in [("low", 0, 40, 20), ("high", 30, 20, 10)] {
        v.push(json!({"type":"sketch.create","sketch":name,"plane":{"base":"xy","offset":z}}));
        for (i, (x, y)) in [(0, 0), (w, 0), (w, h), (0, h)].iter().enumerate() {
            v.push(json!({"type":"sketch.add_entity","sketch":name,"entity_id":format!("p{i}"),"entity":{"type":"point","x":x,"y":y}}));
        }
        for i in 0..4 {
            v.push(json!({"type":"sketch.add_entity","sketch":name,"entity_id":format!("l{i}"),"entity":{"type":"line","a":format!("p{i}"),"b":format!("p{}", (i+1)%4)}}));
        }
    }
    v.push(json!({"type":"sketch.create","sketch":"path","plane":{"base":"xz","offset":0}}));
    v.push(json!({"type":"sketch.add_entity","sketch":"path","entity_id":"a","entity":{"type":"point","x":0,"y":0}}));
    v.push(json!({"type":"sketch.add_entity","sketch":"path","entity_id":"b","entity":{"type":"point","x":0,"y":25}}));
    v.push(json!({"type":"sketch.add_entity","sketch":"path","entity_id":"line","entity":{"type":"line","a":"a","b":"b"}}));
    v
}

fn shaft() -> Vec<serde_json::Value> {
    use serde_json::json;
    vec![
        json!({"type":"sketch.create","sketch":"base","plane":{"base":"xy","offset":0}}),
        json!({"type":"sketch.add_entity","sketch":"base","entity_id":"m","entity":{"type":"point","x":0,"y":0}}),
        json!({"type":"sketch.add_entity","sketch":"base","entity_id":"c","entity":{"type":"circle","center":"m","radius":10}}),
        json!({"type":"feature.extrude","feature":"shaft","sketch":"base","depth":20}),
    ]
}

/// The number a script set on the widget `id` this frame (`widget(id):set_value(x)`), if any.
/// Call it before drawing the widget and store the value where the widget reads from.
pub fn number_override(ui: &eframe::egui::Ui, id: &str) -> Option<f64> {
    match eguidev::take_widget_value_override(ui, id)? {
        WidgetValue::Float(v) => Some(v),
        WidgetValue::Int(v) => Some(v as f64),
        _ => None,
    }
}

/// Registers a number box so a script can read it and set it (`widget(id):set_value(x)`).
pub fn tag_number(id: &str, label: &str, value: f64, response: &eframe::egui::Response) {
    track_response_full(
        id,
        response,
        WidgetMeta {
            role: WidgetRole::DragValue,
            label: Some(label.to_owned()),
            value: Some(WidgetValue::Float(value)),
            visible: true,
            ..Default::default()
        },
    );
}

/// Registers a custom-drawn widget (a ribbon button, a tree row) under a stable id so scripts
/// can find it, read its label and click it.
pub fn tag(
    id: &str,
    role: WidgetRole,
    label: &str,
    on: Option<bool>,
    response: &eframe::egui::Response,
) {
    track_response_full(
        id,
        response,
        WidgetMeta {
            role,
            label: Some(label.to_owned()),
            value: on.map(WidgetValue::Bool),
            visible: true,
            ..Default::default()
        },
    );
}
