//! A part made to judge the viewport by: curved and flat faces, steps, a thin rib, deep holes and
//! pockets, a counterbore, all built through ordinary commands (so it is also a sample document).

use crate::state::AppState;
use dc_doc::CommandKind;
use serde_json::{json, Value};

#[derive(Clone, Copy)]
enum Shape {
    Rect(f64, f64, f64, f64),
    Circle(f64, f64, f64),
}

impl Shape {
    fn profile(self) -> &'static str {
        match self {
            Shape::Rect(..) => "profile.e0",
            Shape::Circle(..) => "profile.c",
        }
    }
}

/// The ops that create sketch `name` on `plane` holding `shape`, fully defined.
fn sketch_ops(name: &str, plane: Value, shape: Shape) -> Vec<Value> {
    let con = |id: &str, c: Value| json!({"type":"sketch.add_constraint","sketch":name,"constraint_id":id,"constraint":c});
    let point = |id: &str, x: f64, y: f64| json!({"type":"sketch.add_entity","sketch":name,"entity_id":id,"entity":{"type":"point","x":x,"y":y}});
    let mut ops = vec![json!({"type":"sketch.create","sketch":name,"plane":plane})];
    match shape {
        Shape::Rect(x0, y0, x1, y1) => {
            for (i, (x, y)) in [(x0, y0), (x1, y0), (x1, y1), (x0, y1)].iter().enumerate() {
                ops.push(point(&format!("p{i}"), *x, *y));
            }
            for i in 0..4 {
                ops.push(
                    json!({"type":"sketch.add_entity","sketch":name,"entity_id":format!("e{i}"),
                    "entity":{"type":"line","a":format!("p{i}"),"b":format!("p{}", (i + 1) % 4)}}),
                );
            }
            ops.push(con("ha", json!({"type":"horizontal","line":"e0"})));
            ops.push(con("vb", json!({"type":"vertical","line":"e1"})));
            ops.push(con("hc", json!({"type":"horizontal","line":"e2"})));
            ops.push(con("vd", json!({"type":"vertical","line":"e3"})));
            ops.push(con(
                "w",
                json!({"type":"length","line":"e0","value":x1 - x0}),
            ));
            ops.push(con(
                "h",
                json!({"type":"length","line":"e1","value":y1 - y0}),
            ));
            ops.push(con(
                "anchor",
                json!({"type":"fixed","point":"p0","x":x0,"y":y0}),
            ));
        }
        Shape::Circle(cx, cy, r) => {
            ops.push(point("m", cx, cy));
            ops.push(con("fx", json!({"type":"fixed","point":"m","x":cx,"y":cy})));
            ops.push(
                json!({"type":"sketch.add_entity","sketch":name,"entity_id":"c",
                "entity":{"type":"circle","center":"m","radius":r}}),
            );
            ops.push(con("r", json!({"type":"radius","target":"c","value":r})));
        }
    }
    ops
}

fn face(feature: &str, name: &str) -> Value {
    json!({"base":"face","feature":feature,"face":name})
}

impl AppState {
    /// Loads the showcase part (see the module note). Says why if a step fails.
    pub fn load_showcase(&mut self) {
        // Each step: (feature, kind, target, sketch plane, shape, depth); a cut with no depth goes
        // all the way through. Everything is sketched on the top face of the previous feature.
        struct Step {
            feature: &'static str,
            boss: bool,
            plane: Value,
            shape: Shape,
            depth: Option<f64>,
        }
        let on_plate = || face("plate", "cap_end");
        let on = |f: &str| face(f, &format!("cap_end:{f}"));
        let steps = vec![
            Step {
                feature: "cylinder",
                boss: true,
                plane: on_plate(),
                shape: Shape::Circle(40.0, 48.0, 24.0),
                depth: Some(30.0),
            },
            Step {
                feature: "step1",
                boss: true,
                plane: on_plate(),
                shape: Shape::Rect(80.0, 14.0, 128.0, 46.0),
                depth: Some(10.0),
            },
            Step {
                feature: "step2",
                boss: true,
                plane: on("step1"),
                shape: Shape::Rect(92.0, 20.0, 116.0, 40.0),
                depth: Some(10.0),
            },
            Step {
                feature: "rib",
                boss: true,
                plane: on_plate(),
                shape: Shape::Rect(68.0, 50.0, 72.0, 80.0),
                depth: Some(22.0),
            },
            Step {
                feature: "bore",
                boss: false,
                plane: on("cylinder"),
                shape: Shape::Circle(40.0, 48.0, 13.0),
                depth: None,
            },
            Step {
                feature: "pocket",
                boss: false,
                plane: on_plate(),
                shape: Shape::Rect(80.0, 56.0, 128.0, 80.0),
                depth: Some(8.0),
            },
            Step {
                feature: "hole_a",
                boss: false,
                plane: on_plate(),
                shape: Shape::Circle(16.0, 10.0, 3.0),
                depth: None,
            },
            Step {
                feature: "hole_b",
                boss: false,
                plane: on_plate(),
                shape: Shape::Circle(32.0, 10.0, 3.0),
                depth: None,
            },
            Step {
                feature: "hole_c",
                boss: false,
                plane: on_plate(),
                shape: Shape::Circle(48.0, 10.0, 3.0),
                depth: None,
            },
            Step {
                feature: "cbore",
                boss: false,
                plane: on_plate(),
                shape: Shape::Circle(14.0, 80.0, 7.0),
                depth: Some(4.0),
            },
            Step {
                feature: "cbore_hole",
                boss: false,
                plane: face("cbore", "cap_end:cbore"),
                shape: Shape::Circle(14.0, 80.0, 3.5),
                depth: None,
            },
            Step {
                feature: "step_hole",
                boss: false,
                plane: on("step2"),
                shape: Shape::Circle(104.0, 30.0, 5.0),
                depth: Some(14.0),
            },
        ];
        let mut plate_ops = sketch_ops(
            "s_plate",
            json!({"base":"xy"}),
            Shape::Rect(0.0, 0.0, 140.0, 90.0),
        );
        plate_ops.push(
            json!({"type":"feature.extrude","feature":"plate","sketch":"s_plate",
            "profiles":["profile.e0"],"depth":14}),
        );
        let mut all: Vec<Value> =
            vec![json!({"type":"transaction","label":"plate","ops":plate_ops})];
        let mut previous = "plate".to_owned();
        for s in steps {
            let sk = format!("s_{}", s.feature);
            let mut ops = sketch_ops(&sk, s.plane, s.shape);
            if s.boss {
                ops.push(
                    json!({"type":"feature.boss","feature":s.feature,"target":previous,
                    "sketch":sk,"profiles":[s.shape.profile()],"depth":s.depth.unwrap_or(1.0)}),
                );
            } else {
                let mut cut = json!({"type":"feature.cut","feature":s.feature,"target":previous,
                    "sketch":sk,"profiles":[s.shape.profile()],"reverse":true});
                if let Some(d) = s.depth {
                    cut["depth"] = json!(d);
                }
                ops.push(cut);
            }
            all.push(json!({"type":"transaction","label":s.feature,"ops":ops}));
            previous = s.feature.to_owned();
        }
        for step in all {
            let label = step["label"].as_str().unwrap_or("?").to_owned();
            match serde_json::from_value::<CommandKind>(step) {
                Ok(k) => {
                    if self.submit(k).is_err() {
                        self.error(format!("the showcase part failed at '{label}'"), None);
                        return;
                    }
                }
                Err(e) => {
                    self.error(format!("the showcase part is malformed: {e}"), None);
                    return;
                }
            }
        }
        self.messages.clear();
        self.info("loaded the showcase part: curved and flat faces, steps, a thin rib, deep holes");
        self.fit();
    }
}
