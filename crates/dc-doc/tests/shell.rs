//! The shell feature through the command layer: exact volumes, parametric rebuilds, undo/replay,
//! the thickness through `feature.set_depth`, and clear refusals.

use dc_doc::{
    Actor, Authority, Command, CommandKind as K, DiagCode, Diagnostic, Expected, Outcome, Policy,
    Query, QueryResult,
};
use dc_sketch::{Entity as E, PlaneSpec};

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

fn plate(d: &mut Doc) {
    d.run(K::SketchCreate {
        sketch: "s".into(),
        plane: PlaneSpec::Xy { offset: 0.0 },
    });
    for (i, (x, y)) in [(0.0, 0.0), (40.0, 0.0), (40.0, 20.0), (0.0, 20.0)]
        .iter()
        .enumerate()
    {
        d.run(K::SketchAddEntity {
            sketch: "s".into(),
            entity_id: format!("p{i}"),
            entity: E::Point { x: *x, y: *y },
        });
    }
    for i in 0..4 {
        d.run(K::SketchAddEntity {
            sketch: "s".into(),
            entity_id: format!("e{i}"),
            entity: E::Line {
                a: format!("p{i}"),
                b: format!("p{}", (i + 1) % 4),
            },
        });
    }
    d.run(K::FeatureExtrude {
        feature: "plate".into(),
        sketch: "s".into(),
        profiles: vec!["profile.e0".into()],
        depth: 10.0,
        reverse: false,
        extent: Default::default(),
    });
}

fn shell(f: &str, target: &str, faces: &[&str], t: f64) -> K {
    K::FeatureShell {
        feature: f.into(),
        target: target.into(),
        faces: faces.iter().map(|s| (*s).to_owned()).collect(),
        thickness: t,
    }
}

#[test]
fn a_tray_follows_its_sketch_and_its_wall_thickness_is_a_parameter() {
    let mut d = Doc::new();
    plate(&mut d);
    d.run(shell("tray", "plate", &["cap_end"], 2.0));
    assert!(
        near(d.volume("tray"), 8000.0 - 36.0 * 16.0 * 8.0),
        "{}",
        d.volume("tray")
    );
    let hash = d.a.state().hash();
    // The wall through set_depth.
    d.run(K::FeatureSetDepth {
        feature: "tray".into(),
        depth: 1.0,
        expected: Expected::Value(2.0),
    });
    assert!(
        near(d.volume("tray"), 8000.0 - 38.0 * 18.0 * 9.0),
        "{}",
        d.volume("tray")
    );
    // The part's depth.
    d.run(K::FeatureSetDepth {
        feature: "plate".into(),
        depth: 20.0,
        expected: Expected::Value(10.0),
    });
    assert!(
        near(d.volume("tray"), 16000.0 - 38.0 * 18.0 * 19.0),
        "{}",
        d.volume("tray")
    );
    d.run(K::HistoryUndo { revision: None });
    d.run(K::HistoryUndo { revision: None });
    assert_eq!(d.a.state().hash(), hash);
    d.run(K::HistoryRedo { revision: None });
    d.run(K::HistoryRedo { revision: None });
    let replayed = Authority::replay(&d.a.log_jsonl(), Policy::default()).expect("replays");
    assert_eq!(replayed.state().hash(), d.a.state().hash());
}

#[test]
fn a_shelled_part_goes_on_through_the_boolean() {
    let mut d = Doc::new();
    plate(&mut d);
    d.run(shell("tray", "plate", &["cap_end"], 2.0));
    // A cut from a sketch on the tray's outer wall goes through the boolean and opens a slot.
    d.run(K::SketchCreate {
        sketch: "w".into(),
        plane: PlaneSpec::Face {
            feature: "tray".into(),
            face: "side:e0".into(),
            offset: 0.0,
        },
    });
    d.run(K::SketchAddEntity {
        sketch: "w".into(),
        entity_id: "m".into(),
        entity: E::Point { x: 20.0, y: 5.0 },
    });
    d.run(K::SketchAddEntity {
        sketch: "w".into(),
        entity_id: "c".into(),
        entity: E::Circle {
            center: "m".into(),
            radius: 2.0,
        },
    });
    d.run(K::FeatureCut {
        feature: "hole".into(),
        target: "tray".into(),
        sketch: "w".into(),
        profiles: vec!["profile.c".into()],
        depth: Some(5.0),
        reverse: true,
        extent: Default::default(),
        pattern: None,
    });
    // The wall is 2 thick, so the hole removes a cylinder 2 long (and goes on into the cavity).
    assert!(
        near(
            d.volume("hole"),
            d.volume("tray") - std::f64::consts::PI * 4.0 * 2.0
        ),
        "{}",
        d.volume("hole")
    );
}

#[test]
fn what_cannot_be_shelled_is_refused_with_the_reason_and_leaves_nothing() {
    let mut d = Doc::new();
    plate(&mut d);
    let e = d
        .try_run(shell("x", "plate", &["side:e0"], 2.0))
        .unwrap_err();
    assert_eq!(e.code, DiagCode::Unsupported, "{e:?}");
    assert!(
        e.message.contains("cap_start") && e.message.contains("cap_end"),
        "{}",
        e.message
    );
    let e = d
        .try_run(shell("x", "plate", &["cap_end"], 12.0))
        .unwrap_err();
    assert_eq!(e.code, DiagCode::FeatureFailed, "{e:?}");
    assert_eq!(e.suggestion.as_deref(), Some("use a thinner wall"));
    let e = d.try_run(shell("x", "ghost", &[], 1.0)).unwrap_err();
    assert_eq!(e.code, DiagCode::NotFound, "{e:?}");
    assert!(d.a.state().feature("x").is_none());
    // A part that is not a prism (a pocket in it).
    let notch = K::SketchCreate {
        sketch: "n".into(),
        plane: PlaneSpec::Xy { offset: 10.0 },
    };
    d.run(notch);
    for (i, (x, y)) in [(30.0, -1.0), (41.0, -1.0), (41.0, 21.0), (30.0, 21.0)]
        .iter()
        .enumerate()
    {
        d.run(K::SketchAddEntity {
            sketch: "n".into(),
            entity_id: format!("p{i}"),
            entity: E::Point { x: *x, y: *y },
        });
    }
    for i in 0..4 {
        d.run(K::SketchAddEntity {
            sketch: "n".into(),
            entity_id: format!("e{i}"),
            entity: E::Line {
                a: format!("p{i}"),
                b: format!("p{}", (i + 1) % 4),
            },
        });
    }
    d.run(K::FeatureCut {
        feature: "stepped".into(),
        target: "plate".into(),
        sketch: "n".into(),
        profiles: vec!["profile.e0".into()],
        depth: Some(5.0),
        reverse: true,
        extent: Default::default(),
        pattern: None,
    });
    let e = d
        .try_run(shell("y", "stepped", &["cap_end"], 1.0))
        .unwrap_err();
    assert_eq!(e.code, DiagCode::Unsupported, "{e:?}");
    assert!(e.message.contains("prism"), "{}", e.message);
}
