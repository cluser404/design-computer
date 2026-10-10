//! Loft and sweep through the command layer: exact volumes, two-sketch dependencies (editing
//! either sketch rebuilds the feature), undo/replay, and clear refusals.

use dc_doc::{
    Actor, Authority, Command, CommandKind as K, DiagCode, Diagnostic, Expected, Outcome, Policy,
    Query, QueryResult,
};
use dc_sketch::{DragTarget, Entity as E, PlaneSpec};

struct Doc {
    a: Authority,
    n: u64,
}

impl Doc {
    fn new() -> Self {
        Doc {
            a: Authority::default(),
            n: 0,
        }
    }
    fn run(&mut self, kind: K) -> Outcome {
        self.try_run(kind)
            .unwrap_or_else(|d| panic!("rejected: {d:?}"))
    }
    fn try_run(&mut self, kind: K) -> Result<Outcome, Diagnostic> {
        self.n += 1;
        let c = Command {
            id: format!("c{}", self.n),
            actor: Actor::Human("ann".into()),
            base_revision: self.a.revision(),
            kind,
        };
        self.a.submit(c)
    }
    fn volume(&self, f: &str) -> f64 {
        match self
            .a
            .query(&Query::FeatureSummary { feature: f.into() })
            .unwrap()
        {
            QueryResult::FeatureSummary { info } => info.summary.volume,
            other => panic!("{other:?}"),
        }
    }
}

fn near(a: f64, b: f64) -> bool {
    (a - b).abs() < 1e-6 * b.abs().max(1.0)
}

fn square(d: &mut Doc, sk: &str, offset: f64, half: f64) {
    d.run(K::SketchCreate {
        sketch: sk.into(),
        plane: PlaneSpec::Xy { offset },
    });
    for (i, (x, y)) in [(-half, -half), (half, -half), (half, half), (-half, half)]
        .iter()
        .enumerate()
    {
        d.run(K::SketchAddEntity {
            sketch: sk.into(),
            entity_id: format!("p{i}"),
            entity: E::Point { x: *x, y: *y },
        });
    }
    for i in 0..4 {
        d.run(K::SketchAddEntity {
            sketch: sk.into(),
            entity_id: format!("e{i}"),
            entity: E::Line {
                a: format!("p{i}"),
                b: format!("p{}", (i + 1) % 4),
            },
        });
    }
}

fn loft(f: &str, a: &str, b: &str) -> K {
    K::FeatureLoft {
        feature: f.into(),
        sketch: a.into(),
        profiles: vec![],
        to_sketch: b.into(),
        to_profiles: vec![],
    }
}

fn frustum(a1: f64, a2: f64, h: f64) -> f64 {
    h / 3.0 * (a1 + a2 + (a1 * a2).sqrt())
}

#[test]
fn a_loft_between_two_sketches_follows_both_and_survives_undo_and_replay() {
    let mut d = Doc::new();
    square(&mut d, "low", 0.0, 5.0);
    square(&mut d, "high", 6.0, 2.0);
    d.run(loft("f", "low", "high"));
    assert!(
        near(d.volume("f"), frustum(100.0, 16.0, 6.0)),
        "{}",
        d.volume("f")
    );
    let hash = d.a.state().hash();
    // Move the top plane up: the loft follows (the second sketch is a dependency too).
    d.run(K::SketchDrag {
        sketch: "high".into(),
        points: vec![
            DragTarget {
                point: "p0".into(),
                x: -3.0,
                y: -3.0,
            },
            DragTarget {
                point: "p1".into(),
                x: 3.0,
                y: -3.0,
            },
            DragTarget {
                point: "p2".into(),
                x: 3.0,
                y: 3.0,
            },
            DragTarget {
                point: "p3".into(),
                x: -3.0,
                y: 3.0,
            },
        ],
        radius: None,
    });
    assert!(
        near(d.volume("f"), frustum(100.0, 36.0, 6.0)),
        "{}",
        d.volume("f")
    );
    // The lower sketch too.
    d.run(K::SketchDrag {
        sketch: "low".into(),
        points: vec![
            DragTarget {
                point: "p0".into(),
                x: -6.0,
                y: -6.0,
            },
            DragTarget {
                point: "p1".into(),
                x: 6.0,
                y: -6.0,
            },
            DragTarget {
                point: "p2".into(),
                x: 6.0,
                y: 6.0,
            },
            DragTarget {
                point: "p3".into(),
                x: -6.0,
                y: 6.0,
            },
        ],
        radius: None,
    });
    assert!(
        near(d.volume("f"), frustum(144.0, 36.0, 6.0)),
        "{}",
        d.volume("f")
    );
    d.run(K::HistoryUndo { revision: None });
    d.run(K::HistoryUndo { revision: None });
    assert_eq!(d.a.state().hash(), hash);
    d.run(K::HistoryRedo { revision: None });
    d.run(K::HistoryRedo { revision: None });
    let replayed = Authority::replay(&d.a.log_jsonl(), Policy::default()).expect("replays");
    assert_eq!(replayed.state().hash(), d.a.state().hash());
    // Neither sketch can be deleted while the loft reads it.
    for sk in ["low", "high"] {
        let e = d
            .try_run(K::SketchDelete { sketch: sk.into() })
            .unwrap_err();
        assert!(
            e.message.contains("'f'") || e.message.contains("f"),
            "{}",
            e.message
        );
    }
}

#[test]
fn a_loft_that_cannot_be_flat_or_is_not_made_of_lines_is_refused_with_the_reason() {
    let mut d = Doc::new();
    square(&mut d, "low", 0.0, 5.0);
    // The top outline is a triangle: different number of corners.
    d.run(K::SketchCreate {
        sketch: "tri".into(),
        plane: PlaneSpec::Xy { offset: 5.0 },
    });
    for (i, (x, y)) in [(0.0, 0.0), (4.0, 0.0), (0.0, 4.0)].iter().enumerate() {
        d.run(K::SketchAddEntity {
            sketch: "tri".into(),
            entity_id: format!("p{i}"),
            entity: E::Point { x: *x, y: *y },
        });
    }
    for i in 0..3 {
        d.run(K::SketchAddEntity {
            sketch: "tri".into(),
            entity_id: format!("e{i}"),
            entity: E::Line {
                a: format!("p{i}"),
                b: format!("p{}", (i + 1) % 3),
            },
        });
    }
    let e = d.try_run(loft("x", "low", "tri")).unwrap_err();
    assert_eq!(e.code, DiagCode::FeatureFailed, "{e:?}");
    assert!(e.message.contains("same number"), "{}", e.message);
    // A circle is not made of lines.
    d.run(K::SketchCreate {
        sketch: "round".into(),
        plane: PlaneSpec::Xy { offset: 5.0 },
    });
    d.run(K::SketchAddEntity {
        sketch: "round".into(),
        entity_id: "m".into(),
        entity: E::Point { x: 0.0, y: 0.0 },
    });
    d.run(K::SketchAddEntity {
        sketch: "round".into(),
        entity_id: "c".into(),
        entity: E::Circle {
            center: "m".into(),
            radius: 2.0,
        },
    });
    let e = d.try_run(loft("y", "low", "round")).unwrap_err();
    assert_eq!(e.code, DiagCode::Unsupported, "{e:?}");
    assert!(e.message.contains("straight lines"), "{}", e.message);
    assert!(d.a.state().feature("x").is_none() && d.a.state().feature("y").is_none());
}

#[test]
fn a_sweep_along_a_slanted_line_is_an_oblique_prism_and_straight_along_the_normal_it_takes_any_profile(
) {
    let mut d = Doc::new();
    square(&mut d, "s", 0.0, 3.0);
    // The path: a line in another sketch, from (0,0,0) to (4,0,10) would need a 3D path; a
    // sketch is flat, so use the XZ plane: its x runs along world x, its y along world z.
    d.run(K::SketchCreate {
        sketch: "path".into(),
        plane: PlaneSpec::Xz { offset: 0.0 },
    });
    d.run(K::SketchAddEntity {
        sketch: "path".into(),
        entity_id: "a".into(),
        entity: E::Point { x: 0.0, y: 0.0 },
    });
    d.run(K::SketchAddEntity {
        sketch: "path".into(),
        entity_id: "b".into(),
        entity: E::Point { x: 4.0, y: 10.0 },
    });
    d.run(K::SketchAddEntity {
        sketch: "path".into(),
        entity_id: "l".into(),
        entity: E::Line {
            a: "a".into(),
            b: "b".into(),
        },
    });
    d.run(K::FeatureSweep {
        feature: "sw".into(),
        sketch: "s".into(),
        profiles: vec![],
        path_sketch: "path".into(),
        path: "l".into(),
    });
    // A slanted sweep keeps the base area times the height (Cavalieri).
    assert!(near(d.volume("sw"), 36.0 * 10.0), "{}", d.volume("sw"));
    // A vertical path: a plain extrusion, so a profile with a circle is fine too.
    d.run(K::SketchAddEntity {
        sketch: "path".into(),
        entity_id: "c".into(),
        entity: E::Point { x: 10.0, y: 0.0 },
    });
    d.run(K::SketchAddEntity {
        sketch: "path".into(),
        entity_id: "d".into(),
        entity: E::Point { x: 10.0, y: 7.0 },
    });
    d.run(K::SketchAddEntity {
        sketch: "path".into(),
        entity_id: "up".into(),
        entity: E::Line {
            a: "c".into(),
            b: "d".into(),
        },
    });
    d.run(K::SketchCreate {
        sketch: "disc".into(),
        plane: PlaneSpec::Xy { offset: 0.0 },
    });
    d.run(K::SketchAddEntity {
        sketch: "disc".into(),
        entity_id: "m".into(),
        entity: E::Point { x: 0.0, y: 0.0 },
    });
    d.run(K::SketchAddEntity {
        sketch: "disc".into(),
        entity_id: "k".into(),
        entity: E::Circle {
            center: "m".into(),
            radius: 2.0,
        },
    });
    d.run(K::FeatureSweep {
        feature: "pole".into(),
        sketch: "disc".into(),
        profiles: vec![],
        path_sketch: "path".into(),
        path: "up".into(),
    });
    assert!(
        near(d.volume("pole"), std::f64::consts::PI * 4.0 * 7.0),
        "{}",
        d.volume("pole")
    );
    // But a circle swept along the slanted line is refused: an oblique cylinder.
    let e = d
        .try_run(K::FeatureSweep {
            feature: "bad".into(),
            sketch: "disc".into(),
            profiles: vec![],
            path_sketch: "path".into(),
            path: "l".into(),
        })
        .unwrap_err();
    assert_eq!(e.code, DiagCode::Unsupported, "{e:?}");
    assert!(e.message.contains("oblique cylinder"), "{}", e.message);
    // The path moves, the sweep follows.
    d.run(K::SketchDrag {
        sketch: "path".into(),
        points: vec![DragTarget {
            point: "b".into(),
            x: 4.0,
            y: 20.0,
        }],
        radius: None,
    });
    assert!(near(d.volume("sw"), 36.0 * 20.0), "{}", d.volume("sw"));
    let _ = Expected::Any;
}
