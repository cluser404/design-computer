//! Sketch -> extrude as a parametric chain, driven purely by commands and queries.

use core::f64::consts::PI;
use dc_doc::{
    Actor, Authority, Command, CommandKind as K, DiagCode, Diagnostic, Expected, Outcome, Policy,
    Query, QueryResult, Revision,
};
use dc_sketch::{Constraint as C, Entity as E};

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
    fn q(&self, q: Query) -> QueryResult {
        self.a.query(&q).unwrap()
    }
}

fn point(s: &str, id: &str, x: f64, y: f64) -> K {
    K::SketchAddEntity {
        sketch: s.into(),
        entity_id: id.into(),
        entity: E::Point { x, y },
    }
}
fn line(s: &str, id: &str, a: &str, b: &str) -> K {
    K::SketchAddEntity {
        sketch: s.into(),
        entity_id: id.into(),
        entity: E::Line {
            a: a.into(),
            b: b.into(),
        },
    }
}
fn k(s: &str, id: &str, c: C) -> K {
    K::SketchAddConstraint {
        sketch: s.into(),
        constraint_id: id.into(),
        constraint: c,
    }
}
fn create(s: &str) -> K {
    K::SketchCreate {
        sketch: s.into(),
        plane: Default::default(),
    }
}
fn extrude(feature: &str, sketch: &str, profile: &str, depth: f64) -> K {
    K::FeatureExtrude {
        feature: feature.into(),
        sketch: sketch.into(),
        profiles: vec![profile.into()],
        depth,
        reverse: false,
        extent: Default::default(),
    }
}

/// A sloppy, fully dimensioned 80 x 40 rectangle in sketch `s1`.
fn rectangle(d: &mut Doc) {
    d.run(create("s1"));
    for (id, x, y) in [
        ("p1", 0.0, 0.0),
        ("p2", 70.0, 5.0),
        ("p3", 75.0, 45.0),
        ("p4", 3.0, 38.0),
    ] {
        d.run(point("s1", id, x, y));
    }
    for (id, a, b) in [
        ("l1", "p1", "p2"),
        ("l2", "p2", "p3"),
        ("l3", "p3", "p4"),
        ("l4", "p4", "p1"),
    ] {
        d.run(line("s1", id, a, b));
    }
    d.run(k("s1", "h1", C::Horizontal { line: "l1".into() }));
    d.run(k("s1", "v2", C::Vertical { line: "l2".into() }));
    d.run(k("s1", "h3", C::Horizontal { line: "l3".into() }));
    d.run(k("s1", "v4", C::Vertical { line: "l4".into() }));
    d.run(k(
        "s1",
        "w",
        C::Length {
            line: "l1".into(),
            value: 80.0,
        },
    ));
    d.run(k(
        "s1",
        "h",
        C::Length {
            line: "l2".into(),
            value: 40.0,
        },
    ));
    d.run(k(
        "s1",
        "anchor",
        C::Fixed {
            point: "p1".into(),
            x: 0.0,
            y: 0.0,
        },
    ));
}

fn volume(d: &Doc, feature: &str) -> f64 {
    match d.q(Query::FeatureSummary {
        feature: feature.into(),
    }) {
        QueryResult::FeatureSummary { info } => info.summary.volume,
        other => panic!("{other:?}"),
    }
}
fn close(a: f64, b: f64) -> bool {
    (a - b).abs() <= 1e-9 * b.abs().max(1.0)
}

#[test]
fn a_sketch_becomes_a_solid_and_the_solid_is_fully_queryable() {
    let mut d = Doc::new();
    rectangle(&mut d);
    match d.q(Query::SketchProfiles {
        sketch: "s1".into(),
        after: None,
        limit: None,
    }) {
        QueryResult::SketchProfiles {
            profiles,
            problems,
            problem_count,
            ..
        } => {
            assert_eq!(profiles.len(), 1);
            assert_eq!(profiles[0].id, "profile.l1");
            assert!(close(profiles[0].area, 3200.0));
            assert_eq!(profiles[0].outer.len(), 4);
            assert!(problems.is_empty() && problem_count == 0);
        }
        other => panic!("{other:?}"),
    }
    let out = d.run(extrude("e1", "s1", "profile.l1", 20.0));
    assert_eq!(out.changes.len(), 1);
    assert_eq!(out.changes[0].entity, "feature/e1");
    assert!(out.changes[0].before.is_none());

    match d.q(Query::FeatureSummary {
        feature: "e1".into(),
    }) {
        QueryResult::FeatureSummary { info } => {
            assert_eq!(info.kind, "extrude");
            let s = info.summary;
            assert!(
                close(s.volume, 64_000.0) && close(s.area, 11_200.0),
                "{s:?}"
            );
            assert_eq!((s.vertices, s.edges, s.faces), (8, 12, 6));
            assert_eq!(s.bounds_min, [0.0, 0.0, 0.0]);
            assert_eq!(s.bounds_max, [80.0, 40.0, 20.0]);
        }
        other => panic!("{other:?}"),
    }
    match d.q(Query::FeatureFaces {
        feature: "e1".into(),
        after: None,
        limit: None,
    }) {
        QueryResult::FeatureFaces { items, next } => {
            let names: Vec<_> = items.iter().map(|f| f.name.as_str()).collect();
            assert_eq!(
                names,
                [
                    "cap_end",
                    "cap_start",
                    "side:l1",
                    "side:l2",
                    "side:l3",
                    "side:l4"
                ]
            );
            assert!(next.is_none());
            let top = items.iter().find(|f| f.name == "cap_end").unwrap();
            assert_eq!(top.normal, Some([0.0, 0.0, 1.0]));
            assert!(close(top.area, 3200.0));
            let bottom = items.iter().find(|f| f.name == "cap_start").unwrap();
            assert_eq!(bottom.normal, Some([0.0, 0.0, -1.0]));
        }
        other => panic!("{other:?}"),
    }
    match d.q(Query::Features {
        prefix: String::new(),
        after: None,
        limit: None,
    }) {
        QueryResult::Features { items, .. } => assert_eq!(items.len(), 1),
        other => panic!("{other:?}"),
    }
    assert_eq!(d.a.state().hash(), d.a.state().recompute_hash());
}

#[test]
fn editing_a_sketch_dimension_rebuilds_the_solid_and_reports_it() {
    let mut d = Doc::new();
    rectangle(&mut d);
    d.run(extrude("e1", "s1", "profile.l1", 20.0));
    let out = d.run(K::SketchSetValue {
        sketch: "s1".into(),
        constraint_id: "w".into(),
        value: 100.0,
        expected: Expected::Value(80.0),
    });
    assert!(close(volume(&d, "e1"), 100.0 * 40.0 * 20.0));
    let f = out
        .changes
        .iter()
        .find(|c| c.entity == "feature/e1")
        .expect("feature change recorded");
    assert!(close(
        f.before.as_ref().unwrap()["summary"]["volume"]
            .as_f64()
            .unwrap(),
        64_000.0
    ));
    assert!(close(
        f.after.as_ref().unwrap()["summary"]["volume"]
            .as_f64()
            .unwrap(),
        80_000.0
    ));
    // Dragging a free corner after dropping a dimension also flows through.
    d.run(K::SketchRemoveConstraint {
        sketch: "s1".into(),
        constraint_id: "h".into(),
    });
    d.run(K::SketchDragPoint {
        sketch: "s1".into(),
        point: "p3".into(),
        x: 100.0,
        y: 55.0,
    });
    assert!(close(volume(&d, "e1"), 100.0 * 55.0 * 20.0));
    assert_eq!(d.a.state().hash(), d.a.state().recompute_hash());
}

#[test]
fn depth_is_a_parametric_edit_with_a_stale_check() {
    let mut d = Doc::new();
    rectangle(&mut d);
    d.run(extrude("e1", "s1", "profile.l1", 20.0));
    d.run(K::FeatureSetDepth {
        feature: "e1".into(),
        depth: 35.0,
        expected: Expected::Value(20.0),
    });
    assert!(close(volume(&d, "e1"), 80.0 * 40.0 * 35.0));
    let h = d.a.state_hash();
    let e = d
        .try_run(K::FeatureSetDepth {
            feature: "e1".into(),
            depth: 5.0,
            expected: Expected::Value(20.0),
        })
        .unwrap_err();
    assert_eq!(e.code, DiagCode::StaleExpectation);
    assert_eq!(d.a.state_hash(), h);
    for bad in [0.0, -3.0, f64::NAN] {
        let e = d
            .try_run(K::FeatureSetDepth {
                feature: "e1".into(),
                depth: bad,
                expected: Expected::Any,
            })
            .unwrap_err();
        assert_eq!(e.code, DiagCode::InvalidValue, "{bad}");
    }
    assert_eq!(d.a.state_hash(), h);
}

#[test]
fn an_edit_that_would_break_a_feature_is_rejected_and_names_it() {
    let mut d = Doc::new();
    rectangle(&mut d);
    d.run(extrude("e1", "s1", "profile.l1", 20.0));
    let (h, rev) = (d.a.state_hash(), d.a.revision());
    // Remove the bottom edge (and what refers to it): the loop no longer closes.
    let e = d
        .try_run(K::Transaction {
            label: "open the profile".into(),
            ops: vec![
                K::SketchRemoveConstraint {
                    sketch: "s1".into(),
                    constraint_id: "h1".into(),
                },
                K::SketchRemoveConstraint {
                    sketch: "s1".into(),
                    constraint_id: "w".into(),
                },
                K::SketchRemoveEntity {
                    sketch: "s1".into(),
                    entity_id: "l1".into(),
                },
            ],
        })
        .unwrap_err();
    assert_eq!(e.code, DiagCode::FeatureFailed);
    assert!(
        e.entities.contains(&"feature/e1".to_string()),
        "{:?}",
        e.entities
    );
    assert!(e.message.contains("e1") && e.suggestion.is_some(), "{e:?}");
    assert_eq!(
        (d.a.state_hash(), d.a.revision()),
        (h, rev),
        "nothing changed"
    );
    assert!(close(volume(&d, "e1"), 64_000.0));

    // The same intent works when the feature goes first, in one transaction.
    d.run(K::Transaction {
        label: "open the profile".into(),
        ops: vec![
            K::FeatureDelete {
                feature: "e1".into(),
            },
            K::SketchRemoveConstraint {
                sketch: "s1".into(),
                constraint_id: "h1".into(),
            },
            K::SketchRemoveConstraint {
                sketch: "s1".into(),
                constraint_id: "w".into(),
            },
            K::SketchRemoveEntity {
                sketch: "s1".into(),
                entity_id: "l1".into(),
            },
        ],
    });
    assert!(d.a.state().feature("e1").is_none());
}

#[test]
fn a_sketch_in_use_cannot_be_deleted() {
    let mut d = Doc::new();
    rectangle(&mut d);
    d.run(extrude("e1", "s1", "profile.l1", 20.0));
    let e = d
        .try_run(K::SketchDelete {
            sketch: "s1".into(),
        })
        .unwrap_err();
    assert_eq!(e.code, DiagCode::InUse);
    assert!(e.entities.contains(&"feature/e1".to_string()));
    d.run(K::Transaction {
        label: "remove both".into(),
        ops: vec![
            K::FeatureDelete {
                feature: "e1".into(),
            },
            K::SketchDelete {
                sketch: "s1".into(),
            },
        ],
    });
    assert_eq!(d.a.state().feature_count() + d.a.state().sketch_count(), 0);
}

#[test]
fn bad_feature_requests_explain_themselves() {
    let mut d = Doc::new();
    rectangle(&mut d);
    // Unknown profile: the diagnostic lists what exists.
    let e = d
        .try_run(extrude("e1", "s1", "profile.nope", 5.0))
        .unwrap_err();
    assert_eq!(e.code, DiagCode::NotFound);
    assert!(
        e.message.contains("profile.l1"),
        "should list available profiles: {}",
        e.message
    );
    // Unknown sketch.
    let e = d
        .try_run(extrude("e1", "ghost", "profile.l1", 5.0))
        .unwrap_err();
    assert_eq!(
        (e.code, e.entities.clone()),
        (DiagCode::NotFound, vec!["sketch/ghost".to_string()])
    );
    // Bad depth.
    let e = d
        .try_run(extrude("e1", "s1", "profile.l1", 0.0))
        .unwrap_err();
    assert_eq!(e.code, DiagCode::InvalidValue);
    // Duplicate feature.
    d.run(extrude("e1", "s1", "profile.l1", 5.0));
    assert_eq!(
        d.try_run(extrude("e1", "s1", "profile.l1", 5.0))
            .unwrap_err()
            .code,
        DiagCode::AlreadyExists
    );
    // Missing feature.
    assert_eq!(
        d.try_run(K::FeatureDelete {
            feature: "zz".into()
        })
        .unwrap_err()
        .code,
        DiagCode::NotFound
    );
    assert_eq!(
        d.a.query(&Query::FeatureSummary {
            feature: "zz".into()
        })
        .unwrap_err()
        .code,
        DiagCode::NotFound
    );
}

#[test]
fn an_unclosed_sketch_explains_why_it_has_no_profile() {
    let mut d = Doc::new();
    d.run(create("s"));
    for (id, x, y) in [("a", 0.0, 0.0), ("b", 10.0, 0.0), ("c", 10.0, 10.0)] {
        d.run(point("s", id, x, y));
    }
    d.run(line("s", "l1", "a", "b"));
    d.run(line("s", "l2", "b", "c"));
    match d.q(Query::SketchProfiles {
        sketch: "s".into(),
        after: None,
        limit: None,
    }) {
        QueryResult::SketchProfiles {
            profiles,
            problems,
            problem_count,
            ..
        } => {
            assert!(profiles.is_empty());
            assert_eq!(problem_count, 2);
            assert!(
                problems.iter().all(|p| p.message.contains("open end")),
                "{problems:?}"
            );
        }
        other => panic!("{other:?}"),
    }
    let e = d.try_run(extrude("e", "s", "profile.l1", 5.0)).unwrap_err();
    assert_eq!(e.code, DiagCode::NotFound);
    assert!(
        e.message.contains("open end"),
        "should say why: {}",
        e.message
    );
    assert!(
        e.entities.iter().any(|x| x == "sketch/s/a"),
        "{:?}",
        e.entities
    );
}

#[test]
fn holes_extrude_and_report_convexity() {
    let mut d = Doc::new();
    d.run(create("s"));
    for (id, x, y) in [
        ("a", 0.0, 0.0),
        ("b", 100.0, 0.0),
        ("c", 100.0, 100.0),
        ("d", 0.0, 100.0),
        ("m", 50.0, 50.0),
    ] {
        d.run(point("s", id, x, y));
    }
    for (id, p, q) in [
        ("ab", "a", "b"),
        ("bc", "b", "c"),
        ("cd", "c", "d"),
        ("da", "d", "a"),
    ] {
        d.run(line("s", id, p, q));
    }
    d.run(K::SketchAddEntity {
        sketch: "s".into(),
        entity_id: "hole".into(),
        entity: E::Circle {
            center: "m".into(),
            radius: 20.0,
        },
    });
    d.run(extrude("plate", "s", "profile.ab", 10.0));
    assert!(close(volume(&d, "plate"), (10_000.0 - 400.0 * PI) * 10.0));
    match d.q(Query::FeatureFaces {
        feature: "plate".into(),
        after: None,
        limit: None,
    }) {
        QueryResult::FeatureFaces { items, .. } => {
            let hole = items.iter().find(|f| f.name == "side:hole").unwrap();
            assert_eq!(
                (hole.surface.as_str(), hole.radius, hole.convex),
                ("cylinder", Some(20.0), Some(false))
            );
        }
        other => panic!("{other:?}"),
    }
    // Resize the hole through the sketch; the plate follows.
    d.run(K::SketchAddConstraint {
        sketch: "s".into(),
        constraint_id: "rad".into(),
        constraint: C::Radius {
            target: "hole".into(),
            value: 20.0,
        },
    });
    d.run(K::SketchSetValue {
        sketch: "s".into(),
        constraint_id: "rad".into(),
        value: 30.0,
        expected: Expected::Value(20.0),
    });
    assert!(close(volume(&d, "plate"), (10_000.0 - 900.0 * PI) * 10.0));
}

#[test]
fn dry_run_predicts_a_feature_edit_exactly() {
    let mut d = Doc::new();
    rectangle(&mut d);
    d.run(extrude("e1", "s1", "profile.l1", 20.0));
    let (h, rev) = (d.a.state_hash(), d.a.revision());
    let c = d.cmd(K::SketchSetValue {
        sketch: "s1".into(),
        constraint_id: "h".into(),
        value: 55.0,
        expected: Expected::Any,
    });
    let predicted = d.a.dry_run(&c).unwrap();
    assert_eq!((d.a.state_hash(), d.a.revision()), (h, rev));
    assert!(close(volume(&d, "e1"), 64_000.0), "dry run must not leak");
    assert!(predicted.changes.iter().any(|c| c.entity == "feature/e1"));
    assert_eq!(d.a.submit(c).unwrap(), predicted);
    assert!(close(volume(&d, "e1"), 80.0 * 55.0 * 20.0));
}

#[test]
fn failed_transactions_roll_back_features_too() {
    let mut d = Doc::new();
    rectangle(&mut d);
    let h = d.a.state_hash();
    let e = d
        .try_run(K::Transaction {
            label: "extrude then fail".into(),
            ops: vec![
                extrude("e1", "s1", "profile.l1", 20.0),
                extrude("e2", "s1", "profile.zzz", 5.0),
            ],
        })
        .unwrap_err();
    assert_eq!(e.code, DiagCode::NotFound);
    assert_eq!(d.a.state_hash(), h);
    assert!(d.a.state().feature("e1").is_none());
}

#[test]
fn replay_rebuilds_the_same_solids_and_detects_tampering() {
    let mut d = Doc::new();
    rectangle(&mut d);
    d.run(extrude("e1", "s1", "profile.l1", 20.0));
    d.run(K::SketchSetValue {
        sketch: "s1".into(),
        constraint_id: "w".into(),
        value: 91.5,
        expected: Expected::Any,
    });
    d.run(K::FeatureSetDepth {
        feature: "e1".into(),
        depth: 12.25,
        expected: Expected::Any,
    });
    let jsonl = d.a.log_jsonl();
    let b = Authority::replay(&jsonl, Policy::default()).unwrap();
    assert_eq!(b.state_hash(), d.a.state_hash());
    assert_eq!(b.log_jsonl(), jsonl);
    assert_eq!(
        b.solid("e1").unwrap().canonical_bytes(),
        d.a.solid("e1").unwrap().canonical_bytes(),
        "the rebuilt solid is bit-identical"
    );
    let tampered = jsonl.replacen("12.25", "12.5", 1);
    assert!(Authority::replay(&tampered, Policy::default()).is_err());
}

#[test]
fn new_commands_round_trip_through_json() {
    let kinds = vec![
        extrude("e", "s", "profile.a", 5.5),
        K::FeatureExtrude {
            feature: "e".into(),
            sketch: "s".into(),
            profiles: vec!["p".into()],
            depth: 1.0,
            reverse: true,
            extent: Default::default(),
        },
        K::FeatureSetDepth {
            feature: "e".into(),
            depth: 2.0,
            expected: Expected::Value(1.0),
        },
        K::FeatureDelete {
            feature: "e".into(),
        },
        K::FeatureRevolve {
            feature: "r".into(),
            sketch: "s".into(),
            profiles: vec!["p".into()],
            axis: "x".into(),
            angle: Some(120.0),
        },
        K::FeatureFillet {
            feature: "f".into(),
            target: "e".into(),
            edges: vec![["side:l1".into(), "side:l2".into()]],
            radius: 2.5,
        },
        K::FeatureChamfer {
            feature: "c".into(),
            target: "e".into(),
            edges: vec![
                ["side:l1".into(), "side:l2".into()],
                ["a".into(), "b".into()],
            ],
            distance: 1.5,
        },
        K::FeatureCut {
            feature: "pc".into(),
            target: "e".into(),
            sketch: "s".into(),
            profiles: vec!["p".into()],
            depth: Some(3.0),
            reverse: false,
            extent: Default::default(),
            pattern: Some(dc_doc::FeaturePattern::Linear {
                step: [5.0, 0.0],
                count: 3,
                step2: Some([0.0, 4.0]),
                count2: 2,
            }),
        },
        K::FeatureBoss {
            feature: "pb".into(),
            target: "e".into(),
            sketch: "s".into(),
            profiles: vec!["p".into()],
            depth: 3.0,
            reverse: false,
            extent: Default::default(),
            pattern: Some(dc_doc::FeaturePattern::Circular {
                center: [1.0, 2.0],
                count: 6,
                step_degrees: 60.0,
            }),
        },
        K::FeatureCut {
            feature: "pm".into(),
            target: "e".into(),
            sketch: "s".into(),
            profiles: vec!["p".into()],
            depth: None,
            reverse: true,
            extent: Default::default(),
            pattern: Some(dc_doc::FeaturePattern::Mirror {
                point: [0.0, 0.0],
                direction_degrees: 45.0,
            }),
        },
    ];
    for kind in kinds {
        let cmd = Command {
            id: "cmd-1".into(),
            actor: Actor::Agent("claude".into()),
            base_revision: Revision(3),
            kind,
        };
        let json = serde_json::to_string(&cmd).unwrap();
        let back: Command = serde_json::from_str(&json).unwrap_or_else(|e| panic!("{e}: {json}"));
        assert_eq!(back, cmd);
    }
    // `reverse` is optional on input.
    let c: Command = serde_json::from_str(
        r#"{"id":"x","actor":{"kind":"human","name":"a"},"base_revision":0,"type":"feature.extrude","feature":"e","sketch":"s","profiles": ["p"],"depth":3}"#,
    )
    .unwrap();
    assert!(matches!(c.kind, K::FeatureExtrude { reverse: false, .. }));
    // A cut written before patterns existed has no `pattern`, and means a single cut.
    let c: Command = serde_json::from_str(
        r#"{"id":"x","actor":{"kind":"human","name":"a"},"base_revision":0,"type":"feature.cut","feature":"c","target":"e","sketch":"s","profiles": ["p"]}"#,
    )
    .unwrap();
    assert!(matches!(c.kind, K::FeatureCut { pattern: None, .. }));
}

#[test]
fn feature_and_profile_listings_paginate() {
    let mut d = Doc::new();
    d.run(create("s"));
    // Twelve separate squares, each its own profile.
    let mut ops = Vec::new();
    for i in 0..12 {
        let x = 20.0 * i as f64;
        for (j, (px, py)) in [(x, 0.0), (x + 10.0, 0.0), (x + 10.0, 10.0), (x, 10.0)]
            .iter()
            .enumerate()
        {
            ops.push(point("s", &format!("p{i:02}_{j}"), *px, *py));
        }
        for j in 0..4 {
            ops.push(line(
                "s",
                &format!("l{i:02}_{j}"),
                &format!("p{i:02}_{j}"),
                &format!("p{i:02}_{}", (j + 1) % 4),
            ));
        }
    }
    d.run(K::Transaction {
        label: "squares".into(),
        ops,
    });
    let mut ids = Vec::new();
    let mut after = None;
    loop {
        match d.q(Query::SketchProfiles {
            sketch: "s".into(),
            after: after.clone(),
            limit: Some(5),
        }) {
            QueryResult::SketchProfiles { profiles, next, .. } => {
                assert!(profiles.len() <= 5);
                ids.extend(profiles.into_iter().map(|p| p.id));
                match next {
                    Some(n) => after = Some(n),
                    None => break,
                }
            }
            other => panic!("{other:?}"),
        }
    }
    assert_eq!(ids.len(), 12);
    assert!(ids.windows(2).all(|w| w[0] < w[1]));
    for (i, id) in ids.iter().enumerate().take(3) {
        d.run(extrude(&format!("e{i}"), "s", id, 3.0));
    }
    match d.q(Query::Features {
        prefix: String::new(),
        after: None,
        limit: Some(2),
    }) {
        QueryResult::Features { items, next } => {
            assert_eq!(items.len(), 2);
            assert_eq!(next.as_deref(), Some("e1"));
        }
        other => panic!("{other:?}"),
    }
    match d.q(Query::FeatureFaces {
        feature: "e0".into(),
        after: None,
        limit: Some(4),
    }) {
        QueryResult::FeatureFaces { items, next } => {
            assert_eq!(items.len(), 4);
            assert_eq!(next.as_deref(), Some(items[3].name.as_str()));
        }
        other => panic!("{other:?}"),
    }
}

#[test]
fn previewing_a_feature_returns_its_solid_and_changes_nothing() {
    let mut d = Doc::new();
    rectangle(&mut d);
    let (rev, hash) = (d.a.revision(), d.a.state_hash());
    let c = d.cmd(extrude("e1", "s1", "profile.l1", 12.0));
    let solids = d.a.preview_solids(&c).unwrap();
    assert_eq!(solids.len(), 1);
    assert_eq!(solids[0].0, "e1");
    let bb = solids[0].1.bounds().unwrap();
    assert!((bb.max.z - bb.min.z - 12.0).abs() < 1e-9, "{bb:?}");
    assert_eq!((d.a.revision(), d.a.state_hash()), (rev, hash));
    assert!(d.a.state().feature("e1").is_none());
    // The real thing builds the same solid.
    d.run(extrude("e1", "s1", "profile.l1", 12.0));
    assert_eq!(d.a.solid("e1").unwrap().bounds(), solids[0].1.bounds());
    // A command that cannot be built is refused with its reason, also without changes.
    let bad = d.cmd(extrude("e2", "s1", "profile.missing", 5.0));
    assert!(d.a.preview_solids(&bad).is_err());
    assert!(d.a.state().feature("e2").is_none());
}
