//! Pockets, counterbores and bosses built through commands, and everything that keeps them valid.

use core::f64::consts::PI;
use dc_doc::{
    Actor, Authority, Command, CommandKind as K, DiagCode, Diagnostic, Expected, Outcome, Policy,
    Query, QueryResult, Revision,
};
use dc_kernel::TessParams;
use dc_sketch::{Constraint as C, Entity as E, PlaneSpec};

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
    fn cmd(&mut self, kind: K) -> Command {
        self.n += 1;
        Command {
            id: format!("c{}", self.n),
            actor: Actor::Human("ann".into()),
            base_revision: self.a.revision(),
            kind,
        }
    }
    fn run(&mut self, kind: K) -> Outcome {
        let c = self.cmd(kind);
        self.a
            .submit(c)
            .unwrap_or_else(|d| panic!("rejected: {d:?}"))
    }
    fn try_run(&mut self, kind: K) -> Result<Outcome, Diagnostic> {
        let c = self.cmd(kind);
        self.a.submit(c)
    }
    fn summary(&self, f: &str) -> dc_doc::SolidSummary {
        match self
            .a
            .query(&Query::FeatureSummary { feature: f.into() })
            .unwrap()
        {
            QueryResult::FeatureSummary { info } => info.summary,
            other => panic!("{other:?}"),
        }
    }
    fn faces(&self, f: &str) -> Vec<dc_doc::FaceInfo> {
        match self
            .a
            .query(&Query::FeatureFaces {
                feature: f.into(),
                after: None,
                limit: Some(1000),
            })
            .unwrap()
        {
            QueryResult::FeatureFaces { items, .. } => items,
            other => panic!("{other:?}"),
        }
    }
}

fn close(a: f64, b: f64) -> bool {
    (a - b).abs() <= 1e-7 * b.abs().max(1.0)
}
const XY: PlaneSpec = PlaneSpec::Xy { offset: 0.0 };

fn rect(d: &mut Doc, sk: &str, plane: PlaneSpec, x0: f64, y0: f64, x1: f64, y1: f64) {
    d.run(K::SketchCreate {
        sketch: sk.into(),
        plane,
    });
    for (i, (x, y)) in [(x0, y0), (x1, y0), (x1, y1), (x0, y1)].iter().enumerate() {
        d.run(K::SketchAddEntity {
            sketch: sk.into(),
            entity_id: format!("{sk}{i}"),
            entity: E::Point { x: *x, y: *y },
        });
    }
    for i in 0..4 {
        d.run(K::SketchAddEntity {
            sketch: sk.into(),
            entity_id: format!("{sk}e{i}"),
            entity: E::Line {
                a: format!("{sk}{i}"),
                b: format!("{sk}{}", (i + 1) % 4),
            },
        });
    }
    let k = |id: &str, c: C| K::SketchAddConstraint {
        sketch: sk.into(),
        constraint_id: id.into(),
        constraint: c,
    };
    d.run(k(
        "w",
        C::Length {
            line: format!("{sk}e0"),
            value: x1 - x0,
        },
    ));
    d.run(k(
        "h",
        C::Length {
            line: format!("{sk}e1"),
            value: y1 - y0,
        },
    ));
    d.run(k(
        "ha",
        C::Horizontal {
            line: format!("{sk}e0"),
        },
    ));
    d.run(k(
        "vb",
        C::Vertical {
            line: format!("{sk}e1"),
        },
    ));
    d.run(k(
        "hc",
        C::Horizontal {
            line: format!("{sk}e2"),
        },
    ));
    d.run(k(
        "vd",
        C::Vertical {
            line: format!("{sk}e3"),
        },
    ));
    d.run(k(
        "fix",
        C::Fixed {
            point: format!("{sk}0"),
            x: x0,
            y: y0,
        },
    ));
}
fn disc(d: &mut Doc, sk: &str, plane: PlaneSpec, cx: f64, cy: f64, r: f64) {
    d.run(K::SketchCreate {
        sketch: sk.into(),
        plane,
    });
    d.run(K::SketchAddEntity {
        sketch: sk.into(),
        entity_id: "m".into(),
        entity: E::Point { x: cx, y: cy },
    });
    d.run(K::SketchAddEntity {
        sketch: sk.into(),
        entity_id: "c".into(),
        entity: E::Circle {
            center: "m".into(),
            radius: r,
        },
    });
    d.run(K::SketchAddConstraint {
        sketch: sk.into(),
        constraint_id: "fx".into(),
        constraint: C::Fixed {
            point: "m".into(),
            x: cx,
            y: cy,
        },
    });
    d.run(K::SketchAddConstraint {
        sketch: sk.into(),
        constraint_id: "r".into(),
        constraint: C::Radius {
            target: "c".into(),
            value: r,
        },
    });
}
fn extrude(f: &str, sk: &str, prof: &str, depth: f64) -> K {
    K::FeatureExtrude {
        feature: f.into(),
        sketch: sk.into(),
        profiles: vec![prof.into()],
        depth,
        reverse: false,
        extent: Default::default(),
    }
}
fn cut(f: &str, target: &str, sk: &str, prof: &str, depth: Option<f64>, reverse: bool) -> K {
    K::FeatureCut {
        feature: f.into(),
        target: target.into(),
        sketch: sk.into(),
        profiles: vec![prof.into()],
        depth,
        reverse,
        extent: Default::default(),
        pattern: None,
    }
}
fn boss(f: &str, target: &str, sk: &str, prof: &str, depth: f64, reverse: bool) -> K {
    K::FeatureBoss {
        feature: f.into(),
        target: target.into(),
        sketch: sk.into(),
        profiles: vec![prof.into()],
        depth,
        reverse,
        extent: Default::default(),
        pattern: None,
    }
}
/// A 100 x 60 plate, 12 thick.
fn plate() -> Doc {
    let mut d = Doc::new();
    rect(&mut d, "base", XY, 0.0, 0.0, 100.0, 60.0);
    d.run(extrude("plate", "base", "profile.basee0", 12.0));
    d
}
const TOP: PlaneSpec = PlaneSpec::Xy { offset: 12.0 };

#[test]
fn a_pocket_a_counterbore_and_their_named_faces() {
    let mut d = plate();
    // A counterbored hole: through hole r=4, then a wider shallow one r=8, 3 deep.
    disc(&mut d, "thru", TOP, 50.0, 30.0, 4.0);
    d.run(cut("hole", "plate", "thru", "profile.c", None, true));
    disc(&mut d, "cbore", TOP, 50.0, 30.0, 8.0);
    d.run(cut("cb", "hole", "cbore", "profile.c", Some(3.0), true));
    let exact = 12.0 * 6000.0 - 16.0 * PI * 12.0 - (64.0 - 16.0) * PI * 3.0;
    assert!(
        close(d.summary("cb").volume, exact),
        "{} vs {exact}",
        d.summary("cb").volume
    );
    let faces = d.faces("cb");
    let ring = faces
        .iter()
        .find(|f| f.name == "cap_end:cb")
        .expect("the step is a named face");
    assert!(close(ring.area, (64.0 - 16.0) * PI) && ring.normal == Some([0.0, 0.0, 1.0]));
    let radii: Vec<f64> = faces.iter().filter_map(|f| f.radius).collect();
    assert!(radii.contains(&4.0) && radii.contains(&8.0), "{radii:?}");
    // A separate pocket elsewhere, 5 deep.
    disc(&mut d, "pk", TOP, 20.0, 30.0, 6.0);
    d.run(cut("pocket", "cb", "pk", "profile.c", Some(5.0), true));
    assert!(close(d.summary("pocket").volume, exact - 36.0 * PI * 5.0));
    assert!(d.a.solid("pocket").unwrap().validate().is_empty());
}

#[test]
fn a_boss_adds_material_and_a_floating_one_is_refused() {
    let mut d = plate();
    rect(&mut d, "tower", TOP, 30.0, 20.0, 70.0, 40.0);
    d.run(boss(
        "boss1",
        "plate",
        "tower",
        "profile.towere0",
        10.0,
        false,
    ));
    assert!(close(
        d.summary("boss1").volume,
        12.0 * 6000.0 + 40.0 * 20.0 * 10.0
    ));
    assert_eq!(d.summary("boss1").bounds_max[2], 22.0);
    let faces = d.faces("boss1");
    assert!(faces
        .iter()
        .any(|f| f.name == "cap_end:boss1" && close(f.area, 800.0)));
    let ring = faces.iter().find(|f| f.name == "cap_end").unwrap();
    assert!(
        close(ring.area, 6000.0 - 800.0),
        "the plate's top is now a ring around the boss"
    );

    // Floating in the air above the part, or far off to the side.
    rect(
        &mut d,
        "air",
        PlaneSpec::Xy { offset: 40.0 },
        30.0,
        20.0,
        70.0,
        40.0,
    );
    let e = d
        .try_run(boss("b2", "plate", "air", "profile.aire0", 5.0, false))
        .unwrap_err();
    assert_eq!(e.code, DiagCode::FeatureFailed);
    assert!(e.message.contains("does not touch"), "{}", e.message);
    rect(&mut d, "far", TOP, 300.0, 300.0, 340.0, 340.0);
    assert_eq!(
        d.try_run(boss("b3", "plate", "far", "profile.fare0", 5.0, false))
            .unwrap_err()
            .code,
        DiagCode::FeatureFailed
    );
    // A boss buried inside the plate adds nothing.
    rect(&mut d, "buried", XY, 10.0, 10.0, 20.0, 20.0);
    let e = d
        .try_run(boss(
            "b4",
            "plate",
            "buried",
            "profile.buriede0",
            5.0,
            false,
        ))
        .unwrap_err();
    assert!(e.message.contains("adds nothing"), "{}", e.message);
    assert_eq!(
        d.a.state().feature_count(),
        2,
        "refused features leave nothing behind"
    );
}

#[test]
fn bosses_and_pockets_are_parametric_and_follow_their_parents() {
    let mut d = plate();
    rect(&mut d, "tower", TOP, 30.0, 20.0, 70.0, 40.0);
    d.run(boss(
        "boss1",
        "plate",
        "tower",
        "profile.towere0",
        10.0,
        false,
    ));
    disc(
        &mut d,
        "pk",
        PlaneSpec::Xy { offset: 22.0 },
        50.0,
        30.0,
        5.0,
    );
    d.run(cut("pocket", "boss1", "pk", "profile.c", Some(4.0), true));
    let v0 = d.summary("pocket").volume;
    assert!(close(v0, 12.0 * 6000.0 + 8000.0 - 25.0 * PI * 4.0));
    // (Changing the boss's depth would strand the pocket's fixed sketch plane at z=22: refused,
    // and fixed by attaching the pocket's sketch to the boss's top face instead.)
    let e = d
        .try_run(K::FeatureSetDepth {
            feature: "boss1".into(),
            depth: 6.0,
            expected: Expected::Any,
        })
        .unwrap_err();
    assert_eq!(e.code, DiagCode::FeatureFailed);
    assert!(
        e.entities.contains(&"feature/pocket".to_string()),
        "{:?}",
        e.entities
    );
    // The base plate's width.
    d.run(K::SketchSetValue {
        sketch: "base".into(),
        constraint_id: "w".into(),
        value: 140.0,
        expected: Expected::Any,
    });
    assert!(close(d.summary("plate").volume, 140.0 * 60.0 * 12.0));
    assert_eq!(d.a.state().hash(), d.a.state().recompute_hash());
}

#[test]
fn dependencies_and_undo_cover_bosses() {
    let mut d = plate();
    rect(&mut d, "tower", TOP, 30.0, 20.0, 70.0, 40.0);
    d.run(boss(
        "boss1",
        "plate",
        "tower",
        "profile.towere0",
        10.0,
        false,
    ));
    let h = d.a.state_hash();
    let e = d
        .try_run(K::FeatureDelete {
            feature: "plate".into(),
        })
        .unwrap_err();
    assert_eq!(e.code, DiagCode::InUse);
    assert!(e.entities.contains(&"feature/boss1".to_string()));
    assert_eq!(
        d.try_run(K::SketchDelete {
            sketch: "tower".into()
        })
        .unwrap_err()
        .code,
        DiagCode::InUse
    );
    assert_eq!(d.a.state_hash(), h);
    d.run(K::HistoryUndo { revision: None });
    assert!(d.a.state().feature("boss1").is_none());
    d.run(K::HistoryRedo { revision: None });
    assert_eq!(d.a.state_hash(), h, "redo restores the boss exactly");
}

#[test]
fn replay_dry_run_and_json_cover_bosses_and_pockets() {
    let mut d = plate();
    rect(&mut d, "tower", TOP, 30.0, 20.0, 70.0, 40.0);
    let c = d.cmd(boss(
        "boss1",
        "plate",
        "tower",
        "profile.towere0",
        10.0,
        false,
    ));
    let h = d.a.state_hash();
    let predicted = d.a.dry_run(&c).unwrap();
    assert_eq!(d.a.state_hash(), h);
    assert_eq!(d.a.submit(c).unwrap(), predicted);
    disc(&mut d, "pk", TOP, 20.0, 30.0, 6.0);
    d.run(cut("pocket", "boss1", "pk", "profile.c", Some(5.0), true));
    let jsonl = d.a.log_jsonl();
    let b = Authority::replay(&jsonl, Policy::default()).unwrap();
    assert_eq!(b.state_hash(), d.a.state_hash());
    for f in ["plate", "boss1", "pocket"] {
        assert_eq!(
            b.solid(f).unwrap().canonical_bytes(),
            d.a.solid(f).unwrap().canonical_bytes(),
            "{f}"
        );
    }
    for kind in [
        boss("b", "t", "s", "p", 4.5, true),
        cut("c", "t", "s", "p", Some(2.0), false),
    ] {
        let cmd = Command {
            id: "x".into(),
            actor: Actor::Agent("claude".into()),
            base_revision: Revision(1),
            kind,
        };
        let json = serde_json::to_string(&cmd).unwrap();
        assert_eq!(
            serde_json::from_str::<Command>(&json).unwrap(),
            cmd,
            "{json}"
        );
    }
    let c: Command = serde_json::from_str(r#"{"id":"x","actor":{"kind":"human","name":"a"},"base_revision":0,"type":"feature.boss","feature":"b","target":"t","sketch":"s","profiles": ["p"],"depth":3}"#).unwrap();
    assert!(matches!(c.kind, K::FeatureBoss { reverse: false, .. }));
}

#[test]
fn stacked_parts_mesh_watertight_and_close_to_the_exact_volume() {
    let mut d = plate();
    rect(&mut d, "tower", TOP, 30.0, 20.0, 70.0, 40.0);
    d.run(boss(
        "boss1",
        "plate",
        "tower",
        "profile.towere0",
        10.0,
        false,
    ));
    disc(
        &mut d,
        "pk",
        PlaneSpec::Xy { offset: 22.0 },
        50.0,
        30.0,
        6.0,
    );
    d.run(cut("pocket", "boss1", "pk", "profile.c", Some(4.0), true));
    disc(&mut d, "hole", XY, 15.0, 30.0, 7.0);
    d.run(cut("drilled", "pocket", "hole", "profile.c", None, false));
    let solid = d.a.solid("drilled").unwrap();
    assert!(solid.validate().is_empty());
    let mesh = solid.tessellate(TessParams::default()).unwrap();
    assert!(mesh.check().is_empty(), "{:?}", mesh.check());
    assert!((mesh.volume() - solid.volume()).abs() / solid.volume() < 5e-3);
}
