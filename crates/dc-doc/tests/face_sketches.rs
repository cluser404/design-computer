//! Sketches placed on faces of solids: they follow the part, and everything that depends on a face
//! is protected.

use core::f64::consts::PI;
use dc_doc::{
    Actor, Authority, Command, CommandKind as K, DiagCode, Diagnostic, Expected, Outcome, Policy,
    Query, QueryResult,
};
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
    fn bounds(&self, f: &str) -> ([f64; 3], [f64; 3]) {
        match self
            .a
            .query(&Query::FeatureSummary { feature: f.into() })
            .unwrap()
        {
            QueryResult::FeatureSummary { info } => {
                (info.summary.bounds_min, info.summary.bounds_max)
            }
            other => panic!("{other:?}"),
        }
    }
    fn frame(&self, sk: &str) -> dc_doc::PlaneFrame {
        match self
            .a
            .query(&Query::SketchSummary { sketch: sk.into() })
            .unwrap()
        {
            QueryResult::SketchSummary { info, .. } => {
                info.frame.expect("the sketch's face exists")
            }
            other => panic!("{other:?}"),
        }
    }
}
fn close(a: f64, b: f64) -> bool {
    (a - b).abs() <= 1e-7 * b.abs().max(1.0)
}
const XY: PlaneSpec = PlaneSpec::Xy { offset: 0.0 };
fn face(feature: &str, face: &str) -> PlaneSpec {
    PlaneSpec::Face {
        feature: feature.into(),
        face: face.into(),
        offset: 0.0,
    }
}

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
fn boss(f: &str, target: &str, sk: &str, prof: &str, depth: f64) -> K {
    K::FeatureBoss {
        feature: f.into(),
        target: target.into(),
        sketch: sk.into(),
        profiles: vec![prof.into()],
        depth,
        reverse: false,
        extent: Default::default(),
        pattern: None,
    }
}
fn plate() -> Doc {
    let mut d = Doc::new();
    rect(&mut d, "base", XY, 0.0, 0.0, 100.0, 60.0);
    d.run(extrude("plate", "base", "profile.basee0", 12.0));
    d
}

#[test]
fn a_cut_sketched_on_the_top_face_follows_the_part_when_it_grows() {
    let mut d = plate();
    disc(&mut d, "tool", face("plate", "cap_end"), 50.0, 30.0, 8.0);
    let f = d.frame("tool");
    assert_eq!(
        (f.origin[2], f.normal),
        (12.0, [0.0, 0.0, 1.0]),
        "on the top face, normal pointing out"
    );
    d.run(cut("hole", "plate", "tool", "profile.c", None, true));
    assert!(close(d.volume("hole"), 12.0 * (6000.0 - 64.0 * PI)));
    // The thing that used to break: the plate gets thicker and the tool goes with its top face.
    d.run(K::FeatureSetDepth {
        feature: "plate".into(),
        depth: 20.0,
        expected: Expected::Value(12.0),
    });
    assert!(
        close(d.volume("hole"), 20.0 * (6000.0 - 64.0 * PI)),
        "{}",
        d.volume("hole")
    );
    assert_eq!(
        d.frame("tool").origin[2],
        20.0,
        "the sketch moved with the face"
    );
    assert_eq!(d.a.state().hash(), d.a.state().recompute_hash());
}

#[test]
fn a_whole_tower_follows_edits_at_the_bottom_in_dependency_order() {
    let mut d = plate();
    // A boss on the plate's top face, and a pocket in the boss's top face.
    rect(
        &mut d,
        "tower",
        face("plate", "cap_end"),
        30.0,
        20.0,
        70.0,
        40.0,
    );
    d.run(boss("boss1", "plate", "tower", "profile.towere0", 10.0));
    disc(
        &mut d,
        "pk",
        face("boss1", "cap_end:boss1"),
        50.0,
        30.0,
        5.0,
    );
    d.run(cut("pocket", "boss1", "pk", "profile.c", Some(4.0), true));
    let expect = |plate_h: f64, boss_h: f64| 6000.0 * plate_h + 800.0 * boss_h - 25.0 * PI * 4.0;
    assert!(close(d.volume("pocket"), expect(12.0, 10.0)));
    assert_eq!(d.frame("pk").origin[2], 22.0);

    // The previously-refused edit: change the boss's depth. The pocket's sketch is on the boss's top.
    d.run(K::FeatureSetDepth {
        feature: "boss1".into(),
        depth: 6.0,
        expected: Expected::Value(10.0),
    });
    assert!(
        close(d.volume("pocket"), expect(12.0, 6.0)),
        "{}",
        d.volume("pocket")
    );
    assert_eq!(d.frame("pk").origin[2], 18.0);
    // The plate's depth: boss and pocket both ride up.
    d.run(K::FeatureSetDepth {
        feature: "plate".into(),
        depth: 20.0,
        expected: Expected::Any,
    });
    assert!(close(d.volume("pocket"), expect(20.0, 6.0)));
    assert_eq!(d.frame("pk").origin[2], 26.0);
    // And the plate's sketch: wider base. Everything above is rebuilt, in order.
    d.run(K::SketchSetValue {
        sketch: "base".into(),
        constraint_id: "w".into(),
        value: 140.0,
        expected: Expected::Any,
    });
    assert!(close(
        d.volume("pocket"),
        140.0 * 60.0 * 20.0 + 800.0 * 6.0 - 25.0 * PI * 4.0
    ));
    assert_eq!(d.a.state().hash(), d.a.state().recompute_hash());
    assert!(d.a.solid("pocket").unwrap().validate().is_empty());
}

#[test]
fn sketches_on_the_bottom_face_look_from_outside_and_build_downward() {
    let mut d = plate();
    // Seen from outside the bottom face the sketch's y axis runs the other way:
    // y = -30..-10 in the sketch is world y = 10..30.
    rect(
        &mut d,
        "foot",
        face("plate", "cap_start"),
        10.0,
        -30.0,
        30.0,
        -10.0,
    );
    let f = d.frame("foot");
    assert_eq!(f.normal, [0.0, 0.0, -1.0], "outward from the bottom face");
    assert_eq!(f.origin[2], 0.0);
    d.run(boss("feet", "plate", "foot", "profile.foote0", 5.0));
    assert!(close(d.volume("feet"), 72_000.0 + 400.0 * 5.0));
    let (lo, hi) = d.bounds("feet");
    assert_eq!(
        (lo[2], hi[2]),
        (-5.0, 12.0),
        "the boss grows out of the bottom, downward"
    );
}

#[test]
fn a_sketch_can_be_offset_from_its_face() {
    let mut d = plate();
    d.run(K::SketchCreate {
        sketch: "above".into(),
        plane: PlaneSpec::Face {
            feature: "plate".into(),
            face: "cap_end".into(),
            offset: 5.0,
        },
    });
    assert_eq!(d.frame("above").origin[2], 17.0);
}

#[test]
fn side_faces_work_as_sketch_planes_and_cuts_from_them_go_through_the_boolean() {
    let mut d = plate();
    // The plate's first side face. Creating a sketch on it is fine.
    d.run(K::SketchCreate {
        sketch: "wall".into(),
        plane: face("plate", "side:basee0"),
    });
    let f = d.frame("wall");
    assert!(
        f.normal[2].abs() < 1e-12,
        "a vertical wall has a horizontal normal: {:?}",
        f.normal
    );
    disc(&mut d, "w2", face("plate", "side:basee0"), 50.0, 6.0, 3.0);
    // A cut from there does not run along the part's axis: the general boolean builds it, five
    // millimetres deep into the plate (reverse: against the face's outward normal).
    d.run(cut("sidehole", "plate", "w2", "profile.c", Some(5.0), true));
    let v = match d
        .a
        .query(&Query::FeatureSummary {
            feature: "sidehole".into(),
        })
        .unwrap()
    {
        QueryResult::FeatureSummary { info } => info.summary.volume,
        other => panic!("{other:?}"),
    };
    let plate_volume = 100.0 * 60.0 * 12.0;
    assert!(
        (v - (plate_volume - 9.0 * core::f64::consts::PI * 5.0)).abs() < 1e-6,
        "{v}"
    );
    // The face it made is named after the feature, and a sketch can sit on it in turn.
    let faces = match d
        .a
        .query(&Query::FeatureFaces {
            feature: "sidehole".into(),
            after: None,
            limit: Some(100),
        })
        .unwrap()
    {
        QueryResult::FeatureFaces { items, .. } => items,
        other => panic!("{other:?}"),
    };
    assert!(
        faces
            .iter()
            .any(|f| f.name.starts_with("side:cut:sidehole:")),
        "{:?}",
        faces.iter().map(|f| f.name.clone()).collect::<Vec<_>>()
    );
}

#[test]
fn bad_face_references_explain_themselves() {
    let mut d = plate();
    let e = d
        .try_run(K::SketchCreate {
            sketch: "s".into(),
            plane: face("plate", "cap_middle"),
        })
        .unwrap_err();
    assert_eq!(e.code, DiagCode::NotFound);
    for name in ["cap_end", "cap_start", "side:basee0"] {
        assert!(
            e.message.contains(name),
            "should list {name}: {}",
            e.message
        );
    }
    assert!(e.suggestion.as_deref().unwrap().contains("feature.faces"));
    let e = d
        .try_run(K::SketchCreate {
            sketch: "s".into(),
            plane: face("ghost", "cap_end"),
        })
        .unwrap_err();
    assert_eq!(e.code, DiagCode::NotFound);
    assert!(e.message.contains("ghost"));
    // A curved face cannot hold a sketch.
    disc(&mut d, "t", face("plate", "cap_end"), 50.0, 30.0, 8.0);
    d.run(cut("hole", "plate", "t", "profile.c", None, true));
    let e = d
        .try_run(K::SketchCreate {
            sketch: "s".into(),
            plane: face("hole", "side:cut:hole:c"),
        })
        .unwrap_err();
    assert_eq!(e.code, DiagCode::WrongKind);
    assert!(e.message.contains("curved"), "{}", e.message);
    assert!(d.a.state().sketch("s").is_none(), "nothing was created");
}

#[test]
fn what_sketches_depend_on_cannot_be_deleted_or_removed_by_an_edit() {
    let mut d = plate();
    disc(&mut d, "t", face("plate", "cap_end"), 50.0, 30.0, 8.0);
    // A sketch on a face protects the feature, even with nothing built from it yet.
    let e = d
        .try_run(K::FeatureDelete {
            feature: "plate".into(),
        })
        .unwrap_err();
    assert_eq!(e.code, DiagCode::InUse);
    assert!(
        e.entities.contains(&"sketch/t".to_string()),
        "{:?}",
        e.entities
    );
    // A pocket's floor exists only while the pocket is blind: sketch on it, then make it through.
    disc(&mut d, "pk", face("plate", "cap_end"), 20.0, 30.0, 6.0);
    d.run(cut("pocket", "plate", "pk", "profile.c", Some(5.0), true));
    d.run(K::SketchCreate {
        sketch: "onfloor".into(),
        plane: face("pocket", "cap_end:pocket"),
    });
    let h = d.a.state_hash();
    let e = d
        .try_run(K::FeatureSetDepth {
            feature: "pocket".into(),
            depth: 12.0,
            expected: Expected::Value(5.0),
        })
        .unwrap_err();
    assert_eq!(e.code, DiagCode::FeatureFailed);
    assert!(
        e.entities.contains(&"sketch/onfloor".to_string()),
        "{:?}",
        e.entities
    );
    assert!(e.message.contains("detach"), "{}", e.message);
    assert_eq!(d.a.state_hash(), h, "refused, nothing changed");
    // Taking things down in order works.
    d.run(K::Transaction {
        label: "tear down".into(),
        ops: vec![
            K::SketchDelete {
                sketch: "onfloor".into(),
            },
            K::FeatureDelete {
                feature: "pocket".into(),
            },
            K::SketchDelete {
                sketch: "pk".into(),
            },
            K::SketchDelete { sketch: "t".into() },
            K::FeatureDelete {
                feature: "plate".into(),
            },
            K::SketchDelete {
                sketch: "base".into(),
            },
        ],
    });
    assert_eq!(d.a.state().feature_count() + d.a.state().sketch_count(), 0);
}

#[test]
fn undo_redo_replay_and_files_carry_face_attached_chains() {
    let mut d = plate();
    rect(
        &mut d,
        "tower",
        face("plate", "cap_end"),
        30.0,
        20.0,
        70.0,
        40.0,
    );
    d.run(boss("boss1", "plate", "tower", "profile.towere0", 10.0));
    disc(
        &mut d,
        "pk",
        face("boss1", "cap_end:boss1"),
        50.0,
        30.0,
        5.0,
    );
    d.run(cut("pocket", "boss1", "pk", "profile.c", Some(4.0), true));
    let h = d.a.state_hash();
    d.run(K::FeatureSetDepth {
        feature: "plate".into(),
        depth: 30.0,
        expected: Expected::Any,
    });
    let h2 = d.a.state_hash();
    d.run(K::HistoryUndo { revision: None });
    assert_eq!(d.a.state_hash(), h, "the whole chain is back");
    d.run(K::HistoryRedo { revision: None });
    assert_eq!(d.a.state_hash(), h2);
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
    // The command JSON for a face plane is what an agent would write.
    let c: Command = serde_json::from_str(
        r#"{"id":"x","actor":{"kind":"agent","name":"claude"},"base_revision":0,"type":"sketch.create","sketch":"s","plane":{"base":"face","feature":"plate","face":"cap_end"}}"#,
    )
    .unwrap();
    assert!(
        matches!(c.kind, K::SketchCreate { plane: PlaneSpec::Face { offset, .. }, .. } if offset == 0.0)
    );
    assert_eq!(
        serde_json::from_str::<Command>(&serde_json::to_string(&c).unwrap()).unwrap(),
        c
    );
}

#[test]
fn a_sketch_on_a_boolean_made_face_follows_the_feature_and_a_split_face_is_explained() {
    let mut d = plate();
    // A blind rectangular pocket from the front wall (the general boolean builds it).
    rect(
        &mut d,
        "wallpocket",
        face("plate", "side:basee0"),
        40.0,
        3.0,
        60.0,
        9.0,
    );
    d.run(cut(
        "pocket",
        "plate",
        "wallpocket",
        "profile.wallpockete0",
        Some(5.0),
        true,
    ));
    let faces = |d: &Doc, f: &str| -> Vec<String> {
        match d
            .a
            .query(&Query::FeatureFaces {
                feature: f.into(),
                after: None,
                limit: Some(100),
            })
            .unwrap()
        {
            QueryResult::FeatureFaces { items, .. } => items.into_iter().map(|f| f.name).collect(),
            other => panic!("{other:?}"),
        }
    };
    let before = faces(&d, "pocket");
    assert!(
        before.contains(&"cap_start:pocket".to_owned()),
        "{before:?}"
    );
    // A sketch on the pocket's floor follows it when the pocket gets deeper.
    disc(
        &mut d,
        "onfloor",
        face("pocket", "cap_start:pocket"),
        50.0,
        6.0,
        2.0,
    );
    let y0 = d.frame("onfloor").origin[1];
    d.run(K::FeatureSetDepth {
        feature: "pocket".into(),
        depth: 8.0,
        expected: Expected::Value(5.0),
    });
    assert_eq!(faces(&d, "pocket"), before, "the same names after the edit");
    let y1 = d.frame("onfloor").origin[1];
    assert!(
        (y1 - y0 - 3.0).abs() < 1e-9,
        "the floor moved 3 mm into the part: {y0} -> {y1}"
    );
    // A face that a cut splits into pieces: asking for the old name is refused and names them.
    rect(
        &mut d,
        "slot",
        face("plate", "cap_end"),
        -5.0,
        25.0,
        105.0,
        35.0,
    );
    d.run(cut(
        "through",
        "plate",
        "slot",
        "profile.slote0",
        None,
        true,
    ));
    let names = faces(&d, "through");
    let pieces: Vec<&String> = names.iter().filter(|n| n.starts_with("cap_end#")).collect();
    assert_eq!(pieces.len(), 2, "{names:?}");
    let e = d
        .try_run(K::SketchCreate {
            sketch: "oncut".into(),
            plane: face("through", "cap_end"),
        })
        .unwrap_err();
    assert!(
        e.message.contains("it was split into") && e.message.contains(pieces[0].as_str()),
        "{}",
        e.message
    );
    // A piece, by its structural name, is a fine place for a sketch.
    d.run(K::SketchCreate {
        sketch: "onpiece".into(),
        plane: face("through", pieces[0]),
    });
    assert!(d.frame("onpiece").origin[2] > 0.0);
}
