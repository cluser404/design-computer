//! Sketches driven purely through commands and queries, as the GUI, scripts and agents do.

use dc_doc::{
    Actor, Authority, Command, CommandKind as K, DiagCode, Expected, Policy, Query, QueryResult,
    Revision,
};
use dc_sketch::{Constraint as C, Entity as E, SolveStatus};

fn human() -> Actor {
    Actor::Human("ann".into())
}

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
            actor: human(),
            base_revision: self.a.revision(),
            kind,
        }
    }

    fn run(&mut self, kind: K) -> dc_doc::Outcome {
        let c = self.cmd(kind);
        self.a
            .submit(c)
            .unwrap_or_else(|d| panic!("command rejected: {d:?}"))
    }

    fn try_run(&mut self, kind: K) -> Result<dc_doc::Outcome, dc_doc::Diagnostic> {
        let c = self.cmd(kind);
        self.a.submit(c)
    }

    fn q(&self, q: Query) -> QueryResult {
        self.a.query(&q).unwrap()
    }
}

fn point(sketch: &str, id: &str, x: f64, y: f64) -> K {
    K::SketchAddEntity {
        sketch: sketch.into(),
        entity_id: id.into(),
        entity: E::Point { x, y },
    }
}
fn line(sketch: &str, id: &str, a: &str, b: &str) -> K {
    K::SketchAddEntity {
        sketch: sketch.into(),
        entity_id: id.into(),
        entity: E::Line {
            a: a.into(),
            b: b.into(),
        },
    }
}
fn constrain(sketch: &str, id: &str, c: C) -> K {
    K::SketchAddConstraint {
        sketch: sketch.into(),
        constraint_id: id.into(),
        constraint: c,
    }
}
fn create(sketch: &str) -> K {
    K::SketchCreate {
        sketch: sketch.into(),
        plane: Default::default(),
    }
}

/// A sloppy rectangle, fully dimensioned 80 x 40, built with ordinary commands.
fn build_rectangle(d: &mut Doc) {
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
    d.run(constrain("s1", "h1", C::Horizontal { line: "l1".into() }));
    d.run(constrain("s1", "v2", C::Vertical { line: "l2".into() }));
    d.run(constrain("s1", "h3", C::Horizontal { line: "l3".into() }));
    d.run(constrain("s1", "v4", C::Vertical { line: "l4".into() }));
    d.run(constrain(
        "s1",
        "w",
        C::Length {
            line: "l1".into(),
            value: 80.0,
        },
    ));
    d.run(constrain(
        "s1",
        "h",
        C::Length {
            line: "l2".into(),
            value: 40.0,
        },
    ));
    d.run(constrain(
        "s1",
        "anchor",
        C::Fixed {
            point: "p1".into(),
            x: 0.0,
            y: 0.0,
        },
    ));
}

fn pos(d: &Doc, id: &str) -> (f64, f64) {
    d.a.state().sketch("s1").unwrap().sketch.point(id).unwrap()
}

fn assert_pos(d: &Doc, id: &str, x: f64, y: f64) {
    let (px, py) = pos(d, id);
    assert!(
        (px - x).abs() < 1e-8 && (py - y).abs() < 1e-8,
        "{id} = ({px}, {py}), want ({x}, {y})"
    );
}

#[test]
fn rectangle_through_commands_is_solved_and_reports_dof_as_data() {
    let mut d = Doc::new();
    build_rectangle(&mut d);
    assert_pos(&d, "p3", 80.0, 40.0);
    match d.q(Query::SketchSummary {
        sketch: "s1".into(),
    }) {
        QueryResult::SketchSummary { info, report } => {
            assert_eq!((info.entities, info.constraints), (8, 7));
            assert!(info.fully_constrained);
            assert_eq!(info.status, SolveStatus::Solved);
            assert_eq!(report.dof, 0);
        }
        other => panic!("{other:?}"),
    }
    assert_eq!(d.a.state().hash(), d.a.state().recompute_hash());
}

#[test]
fn changes_show_what_the_solver_moved() {
    let mut d = Doc::new();
    build_rectangle(&mut d);
    let out = d.run(K::SketchSetValue {
        sketch: "s1".into(),
        constraint_id: "w".into(),
        value: 100.0,
        expected: Expected::Value(80.0),
    });
    let paths: Vec<&str> = out.changes.iter().map(|c| c.entity.as_str()).collect();
    assert!(paths.contains(&"sketch/s1/w"), "{paths:?}");
    assert!(
        paths.contains(&"sketch/s1/p2") && paths.contains(&"sketch/s1/p3"),
        "{paths:?}"
    );
    assert!(
        !paths.contains(&"sketch/s1/p1"),
        "anchored point did not move: {paths:?}"
    );
    let p2 = out
        .changes
        .iter()
        .find(|c| c.entity == "sketch/s1/p2")
        .unwrap();
    assert_eq!(p2.before.as_ref().unwrap()["x"], 80.0);
    assert_eq!(p2.after.as_ref().unwrap()["x"], 100.0);
    assert!(out.sketch_reports["s1"].is_fully_constrained());
    assert_pos(&d, "p3", 100.0, 40.0);
}

#[test]
fn stale_dimension_edit_is_detected() {
    let mut d = Doc::new();
    build_rectangle(&mut d);
    let h = d.a.state_hash();
    let e = d
        .try_run(K::SketchSetValue {
            sketch: "s1".into(),
            constraint_id: "w".into(),
            value: 100.0,
            expected: Expected::Value(75.0),
        })
        .unwrap_err();
    assert_eq!(e.code, DiagCode::StaleExpectation);
    assert_eq!(e.entities, ["sketch/s1/w"]);
    assert_eq!(d.a.state_hash(), h);
}

#[test]
fn contradictory_constraint_is_rejected_naming_the_culprit() {
    let mut d = Doc::new();
    build_rectangle(&mut d);
    let (h, rev) = (d.a.state_hash(), d.a.revision());
    let e = d
        .try_run(constrain(
            "s1",
            "clash",
            C::Distance {
                a: "p1".into(),
                b: "p2".into(),
                value: 90.0,
            },
        ))
        .unwrap_err();
    assert_eq!(e.code, DiagCode::Unsolvable);
    assert_eq!(e.entities, ["sketch/s1/clash"]);
    assert!(e.suggestion.is_some());
    assert_eq!(
        (d.a.state_hash(), d.a.revision()),
        (h, rev),
        "nothing changed"
    );
    // A redundant (but consistent) constraint is fine and reported as redundant.
    let ok = d.run(constrain(
        "s1",
        "perp",
        C::Perpendicular {
            a: "l1".into(),
            b: "l2".into(),
        },
    ));
    assert_eq!(ok.sketch_reports["s1"].redundant, ["perp"]);
}

#[test]
fn transactions_are_atomic_across_sketch_operations() {
    let mut d = Doc::new();
    d.run(K::ParamSet {
        name: "keep".into(),
        value: 1.0,
        expected: Expected::Any,
    });
    let h = d.a.state_hash();
    let tx = K::Transaction {
        label: "new part".into(),
        ops: vec![
            create("s9"),
            point("s9", "a", 0.0, 0.0),
            K::ParamSet {
                name: "keep".into(),
                value: 2.0,
                expected: Expected::Any,
            },
            point("s9", "a", 1.0, 1.0), // duplicate ID: fails the whole transaction
        ],
    };
    let e = d.try_run(tx).unwrap_err();
    assert_eq!(e.code, DiagCode::AlreadyExists);
    assert_eq!(d.a.state_hash(), h);
    assert!(
        d.a.state().sketch("s9").is_none(),
        "sketch creation must be rolled back"
    );
    assert_eq!(d.a.state().get("keep"), Some(1.0));

    // The same transaction without the mistake commits as one revision.
    let tx = K::Transaction {
        label: "new part".into(),
        ops: vec![
            create("s9"),
            point("s9", "a", 0.0, 0.0),
            point("s9", "b", 5.0, 0.0),
        ],
    };
    let before = d.a.revision().0;
    d.run(tx);
    assert_eq!(d.a.revision().0, before + 1);
    assert_eq!(d.a.state().sketch("s9").unwrap().sketch.entities().len(), 2);
}

#[test]
fn deleting_a_sketch_inside_a_failed_transaction_is_undone() {
    let mut d = Doc::new();
    build_rectangle(&mut d);
    let h = d.a.state_hash();
    let tx = K::Transaction {
        label: "oops".into(),
        ops: vec![
            K::SketchDelete {
                sketch: "s1".into(),
            },
            K::SketchDelete {
                sketch: "nope".into(),
            },
        ],
    };
    assert_eq!(d.try_run(tx).unwrap_err().code, DiagCode::NotFound);
    assert_eq!(d.a.state_hash(), h);
    assert_pos(&d, "p3", 80.0, 40.0);
}

#[test]
fn removing_an_entity_in_use_explains_what_uses_it() {
    let mut d = Doc::new();
    build_rectangle(&mut d);
    let e = d
        .try_run(K::SketchRemoveEntity {
            sketch: "s1".into(),
            entity_id: "p1".into(),
        })
        .unwrap_err();
    assert_eq!(e.code, DiagCode::InUse);
    assert!(
        e.entities.contains(&"sketch/s1/l1".to_string()),
        "{:?}",
        e.entities
    );
    assert!(
        e.entities.contains(&"sketch/s1/anchor".to_string()),
        "{:?}",
        e.entities
    );
}

#[test]
fn dry_run_predicts_exactly_and_commits_nothing() {
    let mut d = Doc::new();
    build_rectangle(&mut d);
    let (h, rev) = (d.a.state_hash(), d.a.revision());
    let c = d.cmd(K::SketchSetValue {
        sketch: "s1".into(),
        constraint_id: "h".into(),
        value: 55.0,
        expected: Expected::Any,
    });
    let predicted = d.a.dry_run(&c).unwrap();
    assert_eq!((d.a.state_hash(), d.a.revision()), (h, rev));
    assert_pos(&d, "p3", 80.0, 40.0);
    let real = d.a.submit(c).unwrap();
    assert_eq!(predicted, real);
    assert_pos(&d, "p3", 80.0, 55.0);
}

#[test]
fn dragging_commits_one_reviewable_change() {
    let mut d = Doc::new();
    build_rectangle(&mut d);
    d.run(K::SketchRemoveConstraint {
        sketch: "s1".into(),
        constraint_id: "h".into(),
    });
    let before = d.a.revision().0;
    let out = d.run(K::SketchDragPoint {
        sketch: "s1".into(),
        point: "p3".into(),
        x: 80.0,
        y: 62.5,
    });
    assert_eq!(d.a.revision().0, before + 1);
    assert_pos(&d, "p3", 80.0, 62.5);
    assert_pos(&d, "p4", 0.0, 62.5);
    assert!(out.changes.iter().any(|c| c.entity == "sketch/s1/p4"));
}

#[test]
fn replay_reproduces_sketches_bit_for_bit_and_detects_tampering() {
    let mut d = Doc::new();
    build_rectangle(&mut d);
    d.run(K::SketchSetValue {
        sketch: "s1".into(),
        constraint_id: "w".into(),
        value: 95.5,
        expected: Expected::Any,
    });
    let jsonl = d.a.log_jsonl();
    let b = Authority::replay(&jsonl, Policy::default()).unwrap();
    assert_eq!(b.state_hash(), d.a.state_hash());
    assert_eq!(b.log_jsonl(), jsonl);
    assert_eq!(
        serde_json::to_string(&b.state().sketch("s1").unwrap().sketch).unwrap(),
        serde_json::to_string(&d.a.state().sketch("s1").unwrap().sketch).unwrap(),
    );
    let tampered = jsonl.replacen("95.5", "95.25", 1);
    assert_ne!(tampered, jsonl);
    assert!(Authority::replay(&tampered, Policy::default()).is_err());
}

#[test]
fn sketch_queries_are_paginated_and_missing_sketches_are_errors() {
    let mut d = Doc::new();
    d.run(create("big"));
    for i in 0..250 {
        d.run(point("big", &format!("p{i:04}"), i as f64, 0.0));
    }
    d.run(create("other"));
    // Walk the entities in pages of 100.
    let mut all = Vec::new();
    let mut after = None;
    loop {
        match d.q(Query::SketchEntities {
            sketch: "big".into(),
            after: after.clone(),
            limit: Some(100),
        }) {
            QueryResult::SketchEntities { items, next, .. } => {
                assert!(items.len() <= 100);
                all.extend(items);
                match next {
                    Some(n) => after = Some(n),
                    None => break,
                }
            }
            other => panic!("{other:?}"),
        }
    }
    assert_eq!(all.len(), 250);
    assert!(all.windows(2).all(|w| w[0].0 < w[1].0));
    match d.q(Query::Sketches {
        prefix: String::new(),
        after: None,
        limit: None,
    }) {
        QueryResult::Sketches { items, next } => {
            let ids: Vec<_> = items.iter().map(|i| i.id.as_str()).collect();
            assert_eq!(ids, ["big", "other"]);
            assert!(next.is_none());
            assert_eq!(items[0].entities, 250);
            assert_eq!(items[0].dof, 500);
        }
        other => panic!("{other:?}"),
    }
    let e =
        d.a.query(&Query::SketchSummary {
            sketch: "missing".into(),
        })
        .unwrap_err();
    assert_eq!(e.code, DiagCode::NotFound);
    assert_eq!(e.entities, ["sketch/missing"]);
    match d.q(Query::Summary) {
        QueryResult::Summary { sketch_count, .. } => assert_eq!(sketch_count, 2),
        other => panic!("{other:?}"),
    }
}

#[test]
fn json_shape_of_sketch_commands_is_agent_friendly() {
    let c = Command {
        id: "x".into(),
        actor: Actor::Agent("claude".into()),
        base_revision: Revision(4),
        kind: K::SketchAddConstraint {
            sketch: "s1".into(),
            constraint_id: "w".into(),
            constraint: C::Length {
                line: "l1".into(),
                value: 80.0,
            },
        },
    };
    let json = serde_json::to_string(&c).unwrap();
    assert_eq!(
        json,
        r#"{"id":"x","actor":{"kind":"agent","name":"claude"},"base_revision":4,"type":"sketch.add_constraint","sketch":"s1","constraint_id":"w","constraint":{"type":"length","line":"l1","value":80.0}}"#
    );
}

/// Every command variant must survive JSON serialization unchanged. This is what catches
/// field-name collisions with the flattened envelope (`id`, `actor`, `base_revision`), which
/// produce duplicate JSON keys that cannot be parsed back.
#[test]
fn every_command_variant_round_trips_through_json() {
    let kinds = vec![
        K::ParamSet {
            name: "n".into(),
            value: 1.5,
            expected: Expected::Value(1.0),
        },
        K::ParamDelete {
            name: "n".into(),
            expected: Expected::Absent,
        },
        create("s"),
        K::SketchDelete { sketch: "s".into() },
        point("s", "p", 1.0, 2.0),
        K::SketchAddEntity {
            sketch: "s".into(),
            entity_id: "c".into(),
            entity: E::Circle {
                center: "p".into(),
                radius: 2.0,
            },
        },
        K::SketchAddEntity {
            sketch: "s".into(),
            entity_id: "a".into(),
            entity: E::Arc {
                center: "p".into(),
                start: "q".into(),
                end: "r".into(),
            },
        },
        K::SketchRemoveEntity {
            sketch: "s".into(),
            entity_id: "p".into(),
        },
        K::SketchSetConstruction {
            sketch: "s".into(),
            entity_id: "l".into(),
            construction: true,
        },
        K::SketchSetInfinite {
            sketch: "s".into(),
            entity_id: "l".into(),
            infinite: true,
        },
        constrain(
            "s",
            "k",
            C::Coincident {
                a: "p".into(),
                b: "q".into(),
            },
        ),
        constrain(
            "s",
            "k",
            C::Tangent {
                line: "l".into(),
                curve: "c".into(),
            },
        ),
        K::SketchRemoveConstraint {
            sketch: "s".into(),
            constraint_id: "k".into(),
        },
        K::SketchSetValue {
            sketch: "s".into(),
            constraint_id: "k".into(),
            value: 3.0,
            expected: Expected::Any,
        },
        K::SketchDragPoint {
            sketch: "s".into(),
            point: "p".into(),
            x: 1.0,
            y: 2.0,
        },
        K::SketchPlaceDimension {
            sketch: "s".into(),
            constraint_id: "d".into(),
            x: 1.5,
            y: -2.5,
        },
        K::SketchTrimBetween {
            sketch: "s".into(),
            bounds: vec!["a".into(), "b".into()],
            inside: true,
            construction: false,
        },
        K::SketchTransform {
            sketch: "s".into(),
            entities: vec!["l".into()],
            transform: dc_sketch::Transform::Rotate {
                cx: 1.0,
                cy: 2.0,
                degrees: 30.0,
            },
            copy: true,
        },
        K::SketchMirror {
            sketch: "s".into(),
            entities: vec!["l".into()],
            about: dc_sketch::MirrorAbout::Line { entity: "m".into() },
        },
        K::SketchPattern {
            sketch: "s".into(),
            entities: vec!["l".into()],
            pattern: dc_sketch::Pattern::Circular {
                cx: 0.0,
                cy: 0.0,
                degrees: 360.0,
                count: 6,
            },
        },
        K::SketchOffset {
            sketch: "s".into(),
            entities: vec!["l".into()],
            distance: 2.5,
        },
        K::SketchFillet {
            sketch: "s".into(),
            first: "a".into(),
            second: "b".into(),
            radius: 3.0,
        },
        K::SketchChamfer {
            sketch: "s".into(),
            first: "a".into(),
            second: "b".into(),
            distance: 3.0,
            second_distance: Some(2.0),
        },
        K::SketchRepair { sketch: "s".into() },
        K::SketchFullyDefine { sketch: "s".into() },
        K::SketchSetDriven {
            sketch: "s".into(),
            constraint_id: "d".into(),
            driven: true,
        },
        K::SketchSetDimensionStyle {
            sketch: "s".into(),
            constraint_id: "d".into(),
            style: dc_sketch::DimStyle {
                decimals: Some(3),
                tol_plus: 0.1,
                tol_minus: 0.05,
            },
        },
        K::SketchSetPlane {
            sketch: "s".into(),
            plane: dc_sketch::PlaneSpec::Xy { offset: 4.0 },
        },
        K::SketchTrim {
            sketch: "s".into(),
            entity_id: "l".into(),
            x: 1.0,
            y: 2.0,
            construction: true,
            bounds: vec!["a".into(), "b".into()],
        },
        K::SketchCornerTrim {
            sketch: "s".into(),
            first: "a".into(),
            first_x: 1.0,
            first_y: 2.0,
            second: "b".into(),
            second_x: 3.0,
            second_y: 4.0,
        },
        K::SketchMergePoints {
            sketch: "s".into(),
            a: "p".into(),
            b: "q".into(),
        },
        K::SketchExtend {
            sketch: "s".into(),
            entity_id: "l".into(),
            x: 1.0,
            y: 2.0,
            construction: false,
        },
        K::SketchSplit {
            sketch: "s".into(),
            entity_id: "l".into(),
            x: 1.0,
            y: 2.0,
        },
        K::SketchDrag {
            sketch: "s".into(),
            points: vec![dc_sketch::DragTarget {
                point: "p".into(),
                x: 1.0,
                y: 2.0,
            }],
            radius: Some(dc_sketch::DragRadius {
                circle: "k".into(),
                value: 4.0,
            }),
        },
        K::Transaction {
            label: "t".into(),
            ops: vec![create("s"), point("s", "p", 0.5, 0.5)],
        },
    ];
    for kind in kinds {
        let cmd = Command {
            id: "cmd-1".into(),
            actor: Actor::Agent("claude".into()),
            base_revision: Revision(7),
            kind,
        };
        let json = serde_json::to_string(&cmd).unwrap();
        let back: Command = serde_json::from_str(&json).unwrap_or_else(|e| panic!("{e}: {json}"));
        assert_eq!(back, cmd, "{json}");
        assert_eq!(
            back.id, "cmd-1",
            "the command id must not be shadowed by a variant field"
        );
    }
}

#[test]
fn sketches_carry_their_3d_plane_and_report_it() {
    use dc_sketch::PlaneSpec;
    let mut d = Doc::new();
    d.run(K::SketchCreate {
        sketch: "front".into(),
        plane: PlaneSpec::Xz { offset: 5.0 },
    });
    d.run(create("default"));
    match d.q(Query::SketchSummary {
        sketch: "front".into(),
    }) {
        QueryResult::SketchSummary { info, .. } => {
            assert_eq!(info.plane, PlaneSpec::Xz { offset: 5.0 })
        }
        other => panic!("{other:?}"),
    }
    match d.q(Query::SketchSummary {
        sketch: "default".into(),
    }) {
        QueryResult::SketchSummary { info, .. } => {
            assert_eq!(info.plane, PlaneSpec::Xy { offset: 0.0 })
        }
        other => panic!("{other:?}"),
    }
    // Wire format: plane is optional on input, and a bad offset is refused.
    let json = r#"{"id":"c","actor":{"kind":"human","name":"a"},"base_revision":0,"type":"sketch.create","sketch":"s"}"#;
    let c: Command = serde_json::from_str(json).unwrap();
    assert!(matches!(
        c.kind,
        K::SketchCreate {
            plane: PlaneSpec::Xy { .. },
            ..
        }
    ));
    let e = d
        .try_run(K::SketchCreate {
            sketch: "bad".into(),
            plane: PlaneSpec::Yz { offset: f64::NAN },
        })
        .unwrap_err();
    assert_eq!(e.code, DiagCode::InvalidValue);
    // Replay keeps the plane.
    let b = Authority::replay(&d.a.log_jsonl(), Policy::default()).unwrap();
    assert_eq!(b.state_hash(), d.a.state_hash());
    assert_eq!(
        b.state().sketch("front").unwrap().sketch.plane_spec(),
        PlaneSpec::Xz { offset: 5.0 }
    );
}

fn set_construction(sketch: &str, id: &str, on: bool) -> K {
    K::SketchSetConstruction {
        sketch: sketch.into(),
        entity_id: id.into(),
        construction: on,
    }
}

fn profile_count(d: &Doc, sketch: &str) -> usize {
    match d.q(Query::SketchProfiles {
        sketch: sketch.into(),
        after: None,
        limit: None,
    }) {
        QueryResult::SketchProfiles { profiles, .. } => profiles.len(),
        other => panic!("{other:?}"),
    }
}

#[test]
fn construction_geometry_is_a_logged_undoable_reviewable_flag() {
    let mut d = Doc::new();
    build_rectangle(&mut d);
    d.run(line("s1", "diag", "p1", "p3"));
    let before = d.a.state_hash();
    let out = d.run(set_construction("s1", "diag", true));
    assert_ne!(
        d.a.state_hash(),
        before,
        "the flag is part of the document state"
    );
    assert!(
        out.changes
            .iter()
            .any(|c| c.entity == "sketch/s1/diag#construction"
                && c.before == Some(false.into())
                && c.after == Some(true.into())),
        "{:?}",
        out.changes
    );
    match d.q(Query::SketchEntities {
        sketch: "s1".into(),
        after: None,
        limit: None,
    }) {
        QueryResult::SketchEntities { construction, .. } => {
            assert_eq!(construction, vec!["diag".to_string()])
        }
        other => panic!("{other:?}"),
    }
    // Replay reproduces it exactly.
    let jsonl = d.a.log_jsonl();
    let b = Authority::replay(&jsonl, Policy::default()).unwrap();
    assert_eq!(b.state_hash(), d.a.state_hash());
    assert!(b
        .state()
        .sketch("s1")
        .unwrap()
        .sketch
        .is_construction("diag"));
    // Undo gives back the bit-identical previous state.
    d.run(K::HistoryUndo { revision: None });
    assert_eq!(d.a.state_hash(), before);
    assert!(!d
        .a
        .state()
        .sketch("s1")
        .unwrap()
        .sketch
        .is_construction("diag"));
}

#[test]
fn a_diagonal_only_splits_the_profile_until_it_is_construction() {
    let mut d = Doc::new();
    build_rectangle(&mut d);
    d.run(line("s1", "diag", "p1", "p3"));
    assert_eq!(
        profile_count(&d, "s1"),
        0,
        "a normal diagonal makes the rectangle a branched, unusable profile"
    );
    d.run(set_construction("s1", "diag", true));
    assert_eq!(
        profile_count(&d, "s1"),
        1,
        "as construction it no longer matters"
    );
    d.run(set_construction("s1", "diag", false));
    assert_eq!(profile_count(&d, "s1"), 0);
}

#[test]
fn construction_geometry_still_constrains_and_only_lines_circles_and_arcs_can_be_construction() {
    let mut d = Doc::new();
    build_rectangle(&mut d);
    d.run(line("s1", "diag", "p1", "p3"));
    d.run(set_construction("s1", "diag", true));
    // The diagonal still constrains the sketch: a length that contradicts the rectangle's own
    // dimensions is refused, the consistent one is accepted.
    let (p1, p3) = (pos(&d, "p1"), pos(&d, "p3"));
    let real = dc_math::det::hypot(p3.0 - p1.0, p3.1 - p1.1);
    let e = d
        .try_run(constrain(
            "s1",
            "bad",
            C::Length {
                line: "diag".into(),
                value: real + 25.0,
            },
        ))
        .unwrap_err();
    assert_eq!(e.code, DiagCode::Unsolvable);
    d.run(constrain(
        "s1",
        "dlen",
        C::Length {
            line: "diag".into(),
            value: real,
        },
    ));
    let rev = d.a.revision();
    let e = d.try_run(set_construction("s1", "p1", true)).unwrap_err();
    assert_eq!(e.code, DiagCode::InvalidValue);
    let e = d.try_run(set_construction("s1", "nope", true)).unwrap_err();
    assert_eq!(e.code, DiagCode::NotFound);
    assert_eq!(d.a.revision(), rev, "refused commands change nothing");
    // Removing the entity removes the flag with it.
    d.run(K::SketchRemoveConstraint {
        sketch: "s1".into(),
        constraint_id: "dlen".into(),
    });
    d.run(K::SketchRemoveEntity {
        sketch: "s1".into(),
        entity_id: "diag".into(),
    });
    assert!(d
        .a
        .state()
        .sketch("s1")
        .unwrap()
        .sketch
        .construction()
        .is_empty());
}

#[test]
fn documents_without_construction_geometry_serialise_exactly_as_before() {
    let mut d = Doc::new();
    build_rectangle(&mut d);
    let json = serde_json::to_string(&d.a.state().sketch("s1").unwrap().sketch).unwrap();
    assert!(!json.contains("construction"), "{json}");
}

#[test]
fn dragging_a_shape_is_one_logged_undoable_command_that_replays() {
    let mut d = Doc::new();
    build_rectangle(&mut d);
    // Free the width and height so the edge can move, and add a circle with a free radius.
    for c in ["w", "h"] {
        d.run(K::SketchRemoveConstraint {
            sketch: "s1".into(),
            constraint_id: c.into(),
        });
    }
    d.run(point("s1", "cc", 10.0, 10.0));
    d.run(K::SketchAddEntity {
        sketch: "s1".into(),
        entity_id: "k".into(),
        entity: dc_sketch::Entity::Circle {
            center: "cc".into(),
            radius: 4.0,
        },
    });
    d.run(constrain(
        "s1",
        "fcc",
        C::Fixed {
            point: "cc".into(),
            x: 10.0,
            y: 10.0,
        },
    ));
    let (rev, hash) = (d.a.revision().0, d.a.state_hash());
    let (p2, p3) = (pos(&d, "p2"), pos(&d, "p3"));
    let out = d.run(K::SketchDrag {
        sketch: "s1".into(),
        points: vec![
            dc_sketch::DragTarget {
                point: "p2".into(),
                x: p2.0 + 12.0,
                y: p2.1,
            },
            dc_sketch::DragTarget {
                point: "p3".into(),
                x: p3.0 + 12.0,
                y: p3.1,
            },
        ],
        radius: Some(dc_sketch::DragRadius {
            circle: "k".into(),
            value: 6.5,
        }),
    });
    assert_eq!(d.a.revision().0, rev + 1, "one command");
    assert_pos(&d, "p2", p2.0 + 12.0, p2.1);
    assert_pos(&d, "p3", p3.0 + 12.0, p3.1);
    assert!(
        out.changes.iter().any(|c| c.entity == "sketch/s1/k"),
        "the radius is in the diff"
    );
    // Undo restores it exactly; replay reproduces the dragged state.
    d.run(K::HistoryUndo { revision: None });
    assert_eq!(d.a.state_hash(), hash);
    // A bad target is refused with the sketch untouched.
    let before = d.a.state_hash();
    assert!(d
        .try_run(K::SketchDrag {
            sketch: "s1".into(),
            points: vec![dc_sketch::DragTarget {
                point: "nope".into(),
                x: 0.0,
                y: 0.0
            }],
            radius: None,
        })
        .is_err());
    assert_eq!(d.a.state_hash(), before);
}

#[test]
fn trim_extend_and_split_are_logged_undoable_reviewable_commands() {
    let mut d = Doc::new();
    d.run(create("s1"));
    // A horizontal line crossed by two verticals.
    for (id, x, y) in [
        ("a", 0.0, 0.0),
        ("b", 40.0, 0.0),
        ("c", 10.0, -5.0),
        ("d", 10.0, 5.0),
        ("e", 30.0, -5.0),
        ("f", 30.0, 5.0),
    ] {
        d.run(point("s1", id, x, y));
    }
    for (id, p, q) in [("h", "a", "b"), ("v1", "c", "d"), ("v2", "e", "f")] {
        d.run(line("s1", id, p, q));
    }
    d.run(constrain("s1", "hz", C::Horizontal { line: "h".into() }));
    let (rev, hash0) = (d.a.revision().0, d.a.state_hash());
    let out = d.run(K::SketchTrim {
        sketch: "s1".into(),
        entity_id: "h".into(),
        x: 36.0,
        y: 0.0,
        construction: false,
        bounds: vec![],
    });
    assert_eq!(d.a.revision().0, rev + 1, "one command");
    assert_pos(&d, "p1", 30.0, 0.0);
    assert!(
        out.changes.iter().any(|c| c.entity == "sketch/s1/h"),
        "the diff shows the line changed: {:?}",
        out.changes.iter().map(|c| &c.entity).collect::<Vec<_>>()
    );
    // Undo puts the original back bit for bit; redo and replay agree.
    d.run(K::HistoryUndo { revision: None });
    assert_eq!(d.a.state_hash(), hash0);
    d.run(K::HistoryRedo { revision: None });
    let after_trim = d.a.state_hash();
    // Extend it back out to the second vertical's far side, then split in the middle.
    d.run(K::SketchSplit {
        sketch: "s1".into(),
        entity_id: "h".into(),
        x: 15.0,
        y: 1.0,
    });
    let lines = |d: &Doc| match d.q(Query::SketchEntities {
        sketch: "s1".into(),
        after: None,
        limit: None,
    }) {
        QueryResult::SketchEntities { items, .. } => items
            .iter()
            .filter(|(_, e)| matches!(e, dc_sketch::Entity::Line { .. }))
            .count(),
        other => panic!("{other:?}"),
    };
    assert_eq!(lines(&d), 4, "the line is two now");
    assert_ne!(d.a.state_hash(), after_trim);
    // A request that cannot be honoured is refused with a reason and changes nothing.
    let before = (d.a.revision(), d.a.state_hash());
    let e = d
        .try_run(K::SketchExtend {
            sketch: "s1".into(),
            entity_id: "v1".into(),
            x: 10.0,
            y: 5.0,
            construction: false,
        })
        .unwrap_err();
    assert!(!e.message.is_empty());
    assert_eq!((d.a.revision(), d.a.state_hash()), before);
    assert!(d
        .try_run(K::SketchTrim {
            sketch: "s1".into(),
            entity_id: "nope".into(),
            x: 0.0,
            y: 0.0,
            construction: false,
            bounds: vec![],
        })
        .is_err());
    assert!(d
        .try_run(K::SketchTrim {
            sketch: "s1".into(),
            entity_id: "h".into(),
            x: f64::NAN,
            y: 0.0,
            construction: false,
            bounds: vec![],
        })
        .is_err());
}

#[test]
fn corner_trim_merge_and_bounded_trim_are_commands_too() {
    let mut d = Doc::new();
    d.run(create("s1"));
    for (id, x, y) in [
        ("a", 0.0, 0.0),
        ("b", 40.0, 0.0),
        ("c", 20.0, -20.0),
        ("e", 20.0, 20.0),
        ("g", 8.0, -4.0),
        ("h", 8.0, 4.0),
        ("i", 32.0, -4.0),
        ("j", 32.0, 4.0),
    ] {
        d.run(point("s1", id, x, y));
    }
    for (id, p, q) in [
        ("hz", "a", "b"),
        ("vt", "c", "e"),
        ("w1", "g", "h"),
        ("w2", "i", "j"),
    ] {
        d.run(line("s1", id, p, q));
    }
    let hash0 = d.a.state_hash();
    // Trim between the two bars only: the middle of `hz` goes (and the vertical `vt` is ignored).
    d.run(K::SketchTrim {
        sketch: "s1".into(),
        entity_id: "hz".into(),
        x: 20.0,
        y: 0.0,
        construction: false,
        bounds: vec!["w1".into(), "w2".into()],
    });
    assert_pos(&d, "b", 40.0, 0.0);
    let lines_after = |d: &Doc| match d.q(Query::SketchEntities {
        sketch: "s1".into(),
        after: None,
        limit: None,
    }) {
        QueryResult::SketchEntities { items, .. } => items
            .iter()
            .filter(|(_, e)| matches!(e, dc_sketch::Entity::Line { .. }))
            .count(),
        other => panic!("{other:?}"),
    };
    assert_eq!(lines_after(&d), 5, "hz became two pieces");
    d.run(K::HistoryUndo { revision: None });
    assert_eq!(d.a.state_hash(), hash0);

    // Corner trim of `hz` and `vt` to where they cross; keeps the left and lower parts.
    d.run(K::SketchCornerTrim {
        sketch: "s1".into(),
        first: "hz".into(),
        first_x: 4.0,
        first_y: 0.0,
        second: "vt".into(),
        second_x: 20.0,
        second_y: -10.0,
    });
    assert_eq!(lines_after(&d), 4);
    d.run(K::HistoryUndo { revision: None });
    assert_eq!(d.a.state_hash(), hash0);

    // Merge two points; refused (nothing changes) when it would collapse a line.
    let before = d.a.state_hash();
    assert!(d
        .try_run(K::SketchMergePoints {
            sketch: "s1".into(),
            a: "g".into(),
            b: "h".into(),
        })
        .is_err());
    assert_eq!(d.a.state_hash(), before);
    d.run(K::SketchMergePoints {
        sketch: "s1".into(),
        a: "b".into(),
        b: "j".into(),
    });
    assert_eq!(d.a.revision().0, d.a.revision().0);
    d.run(K::HistoryUndo { revision: None });
    assert_eq!(d.a.state_hash(), before);
}

#[test]
fn where_a_dimension_is_drawn_is_a_logged_undoable_replayable_part_of_the_sketch() {
    let mut d = Doc::new();
    build_rectangle(&mut d);
    let hash0 = d.a.state_hash();
    let rev = d.a.revision().0;
    let out = d.run(K::SketchPlaceDimension {
        sketch: "s1".into(),
        constraint_id: "w".into(),
        x: 40.0,
        y: -12.0,
    });
    assert_eq!(d.a.revision().0, rev + 1);
    assert_ne!(d.a.state_hash(), hash0, "it is part of the design");
    assert!(
        out.changes
            .iter()
            .any(|c| c.entity == "sketch/s1/w#placement"),
        "the diff says it moved: {:?}",
        out.changes.iter().map(|c| &c.entity).collect::<Vec<_>>()
    );
    // Agents can read it back.
    match d.q(Query::SketchConstraints {
        sketch: "s1".into(),
        after: None,
        limit: None,
    }) {
        QueryResult::SketchConstraints { placements, .. } => {
            assert_eq!(placements.get("w"), Some(&[40.0, -12.0]));
            assert_eq!(placements.len(), 1);
        }
        other => panic!("{other:?}"),
    }
    // Moving it again replaces it; the diff carries both places.
    let out = d.run(K::SketchPlaceDimension {
        sketch: "s1".into(),
        constraint_id: "w".into(),
        x: 40.0,
        y: -20.0,
    });
    let c = out
        .changes
        .iter()
        .find(|c| c.entity.ends_with("#placement"))
        .unwrap();
    assert_eq!(c.before.as_ref().unwrap()[1], -12.0);
    assert_eq!(c.after.as_ref().unwrap()[1], -20.0);
    // Undo walks back one step at a time.
    d.run(K::HistoryUndo { revision: None });
    d.run(K::HistoryUndo { revision: None });
    assert_eq!(d.a.state_hash(), hash0);
    d.run(K::HistoryRedo { revision: None });
    // A relation has no place; neither does a dimension that is not there; nor a NaN.
    for (id, x) in [("h1", 1.0), ("nope", 1.0), ("w", f64::NAN)] {
        let before = d.a.state_hash();
        assert!(d
            .try_run(K::SketchPlaceDimension {
                sketch: "s1".into(),
                constraint_id: id.into(),
                x,
                y: 0.0,
            })
            .is_err());
        assert_eq!(d.a.state_hash(), before);
    }
    // Deleting the dimension takes its place with it.
    d.run(K::SketchRemoveConstraint {
        sketch: "s1".into(),
        constraint_id: "w".into(),
    });
    match d.q(Query::SketchConstraints {
        sketch: "s1".into(),
        after: None,
        limit: None,
    }) {
        QueryResult::SketchConstraints { placements, .. } => assert!(placements.is_empty()),
        other => panic!("{other:?}"),
    }
}

#[test]
fn dimensions_follow_their_geometry_through_every_kind_of_edit_and_undo_puts_them_back() {
    let mut d = Doc::new();
    build_rectangle(&mut d);
    let placement = |d: &Doc| match d.q(Query::SketchConstraints {
        sketch: "s1".into(),
        after: None,
        limit: None,
    }) {
        QueryResult::SketchConstraints { placements, .. } => placements.get("w").copied(),
        other => panic!("{other:?}"),
    };
    d.run(K::SketchPlaceDimension {
        sketch: "s1".into(),
        constraint_id: "w".into(),
        x: 40.0,
        y: -12.0,
    });
    assert_eq!(placement(&d), Some([40.0, -12.0]));
    let hash = d.a.state_hash();
    // The width goes from 80 to 100: the bottom edge's middle moves 10 right, so does its dimension.
    let out = d.run(K::SketchSetValue {
        sketch: "s1".into(),
        constraint_id: "w".into(),
        value: 100.0,
        expected: Expected::Any,
    });
    assert_eq!(placement(&d), Some([50.0, -12.0]));
    assert!(
        out.changes
            .iter()
            .any(|c| c.entity == "sketch/s1/w#placement"),
        "the diff shows it followed"
    );
    d.run(K::HistoryUndo { revision: None });
    assert_eq!(placement(&d), Some([40.0, -12.0]));
    assert_eq!(d.a.state_hash(), hash, "undo is exact");
    // Dragging a point does it too.
    d.run(K::SketchRemoveConstraint {
        sketch: "s1".into(),
        constraint_id: "h".into(),
    });
    d.run(K::SketchDrag {
        sketch: "s1".into(),
        points: vec![
            dc_sketch::DragTarget {
                point: "p1".into(),
                x: 0.0,
                y: 0.0,
            },
            dc_sketch::DragTarget {
                point: "p2".into(),
                x: 80.0,
                y: 0.0,
            },
        ],
        radius: None,
    });
    assert_eq!(placement(&d), Some([40.0, -12.0]), "no movement, no change");
}

#[test]
fn the_modify_commands_are_logged_undoable_and_agents_can_use_them() {
    use dc_sketch::{MirrorAbout, Pattern, Transform};
    let mut d = Doc::new();
    d.run(create("s1"));
    for (id, x, y) in [
        ("p1", 0.0, 0.0),
        ("p2", 40.0, 0.0),
        ("p3", 40.0, 20.0),
        ("p4", 0.0, 20.0),
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
    let all = || vec!["l1".to_string(), "l2".into(), "l3".into(), "l4".into()];
    let count = |d: &Doc| match d.q(Query::Sketches {
        prefix: String::new(),
        after: None,
        limit: None,
    }) {
        QueryResult::Sketches { items, .. } => items[0].entities,
        other => panic!("{other:?}"),
    };
    let hash0 = d.a.state_hash();
    assert_eq!(count(&d), 8);
    // Copy, mirror, pattern each add geometry in one command and undo cleanly.
    d.run(K::SketchTransform {
        sketch: "s1".into(),
        entities: all(),
        transform: Transform::Move { dx: 100.0, dy: 0.0 },
        copy: true,
    });
    assert_eq!(count(&d), 16);
    d.run(K::SketchMirror {
        sketch: "s1".into(),
        entities: all(),
        about: MirrorAbout::AxisY,
    });
    assert!(count(&d) > 16);
    d.run(K::SketchPattern {
        sketch: "s1".into(),
        entities: all(),
        pattern: Pattern::Linear {
            dx: 0.0,
            dy: -50.0,
            count: 3,
        },
    });
    for _ in 0..3 {
        d.run(K::HistoryUndo { revision: None });
    }
    assert_eq!(
        d.a.state_hash(),
        hash0,
        "three undos return to the start exactly"
    );
    // Fillet, chamfer, offset, repair and fully-define.
    d.run(K::SketchFillet {
        sketch: "s1".into(),
        first: "l1".into(),
        second: "l2".into(),
        radius: 5.0,
    });
    d.run(K::SketchChamfer {
        sketch: "s1".into(),
        first: "l2".into(),
        second: "l3".into(),
        distance: 4.0,
        second_distance: None,
    });
    d.run(K::SketchOffset {
        sketch: "s1".into(),
        entities: vec!["l4".into()],
        distance: 2.0,
    });
    d.run(K::SketchRepair {
        sketch: "s1".into(),
    });
    d.run(K::SketchFullyDefine {
        sketch: "s1".into(),
    });
    match d.q(Query::SketchSummary {
        sketch: "s1".into(),
    }) {
        QueryResult::SketchSummary { report, .. } => assert_eq!(report.dof, 0, "{report:?}"),
        other => panic!("{other:?}"),
    }
    // Refused, with nothing changed.
    let (rev, hash) = (d.a.revision(), d.a.state_hash());
    for kind in [
        K::SketchFillet {
            sketch: "s1".into(),
            first: "l1".into(),
            second: "l3".into(),
            radius: 1.0,
        },
        K::SketchOffset {
            sketch: "s1".into(),
            entities: vec![],
            distance: 1.0,
        },
        K::SketchPattern {
            sketch: "s1".into(),
            entities: all(),
            pattern: Pattern::Linear {
                dx: 1.0,
                dy: 0.0,
                count: 1,
            },
        },
        K::SketchSetDriven {
            sketch: "s1".into(),
            constraint_id: "nope".into(),
            driven: true,
        },
    ] {
        assert!(d.try_run(kind).is_err());
    }
    assert_eq!((d.a.revision(), d.a.state_hash()), (rev, hash));
}

#[test]
fn reference_dimensions_styles_and_the_plane_are_commands_with_diffs() {
    let mut d = Doc::new();
    build_rectangle(&mut d);
    let out = d.run(K::SketchSetDriven {
        sketch: "s1".into(),
        constraint_id: "h".into(),
        driven: true,
    });
    assert!(out.changes.iter().any(|c| c.entity == "sketch/s1/h#driven"));
    match d.q(Query::SketchConstraints {
        sketch: "s1".into(),
        after: None,
        limit: None,
    }) {
        QueryResult::SketchConstraints { driven, .. } => assert_eq!(driven, ["h"]),
        other => panic!("{other:?}"),
    }
    // As a reference the 40 mm height no longer constrains: the sketch has a free degree of freedom.
    match d.q(Query::SketchSummary {
        sketch: "s1".into(),
    }) {
        QueryResult::SketchSummary { report, .. } => assert_eq!(report.dof, 1, "{report:?}"),
        other => panic!("{other:?}"),
    }
    let out = d.run(K::SketchSetDimensionStyle {
        sketch: "s1".into(),
        constraint_id: "w".into(),
        style: dc_sketch::DimStyle {
            decimals: Some(1),
            tol_plus: 0.2,
            tol_minus: 0.1,
        },
    });
    assert!(out.changes.iter().any(|c| c.entity == "sketch/s1/w#style"));
    match d.q(Query::SketchConstraints {
        sketch: "s1".into(),
        after: None,
        limit: None,
    }) {
        QueryResult::SketchConstraints { styles, .. } => {
            assert_eq!(styles["w"].tol_plus, 0.2);
        }
        other => panic!("{other:?}"),
    }
    d.run(K::SketchSetPlane {
        sketch: "s1".into(),
        plane: dc_sketch::PlaneSpec::Xy { offset: 12.0 },
    });
    match d.q(Query::SketchSummary {
        sketch: "s1".into(),
    }) {
        QueryResult::SketchSummary { info, .. } => {
            assert_eq!(info.plane, dc_sketch::PlaneSpec::Xy { offset: 12.0 });
        }
        other => panic!("{other:?}"),
    }
    d.run(K::HistoryUndo { revision: None });
    d.run(K::HistoryUndo { revision: None });
    d.run(K::HistoryUndo { revision: None });
    match d.q(Query::SketchSummary {
        sketch: "s1".into(),
    }) {
        QueryResult::SketchSummary { report, .. } => assert_eq!(report.dof, 0),
        other => panic!("{other:?}"),
    }
}

#[test]
fn an_infinite_construction_line_is_a_logged_undoable_flag_that_the_queries_report() {
    let mut d = Doc::new();
    build_rectangle(&mut d);
    d.run(line("s1", "diag", "p1", "p3"));
    let infinite = |on: bool| K::SketchSetInfinite {
        sketch: "s1".into(),
        entity_id: "diag".into(),
        infinite: on,
    };
    // Only construction lines can be infinite.
    let e = d
        .try_run(infinite(true))
        .expect_err("a real line is not infinite");
    assert!(e.message.contains("construction"), "{}", e.message);
    d.run(set_construction("s1", "diag", true));
    let before = d.a.state_hash();
    let out = d.run(infinite(true));
    assert_ne!(d.a.state_hash(), before);
    assert!(
        out.changes
            .iter()
            .any(|c| c.entity == "sketch/s1/diag#infinite" && c.after == Some(true.into())),
        "{:?}",
        out.changes
    );
    match d.q(Query::SketchEntities {
        sketch: "s1".into(),
        after: None,
        limit: None,
    }) {
        QueryResult::SketchEntities { infinite, .. } => {
            assert_eq!(infinite, vec!["diag".to_string()])
        }
        other => panic!("{other:?}"),
    }
    // Undo restores the exact earlier state; replay reproduces the later one.
    let after = d.a.state_hash();
    d.run(K::HistoryUndo { revision: None });
    assert_eq!(d.a.state_hash(), before);
    d.run(K::HistoryRedo { revision: None });
    assert_eq!(d.a.state_hash(), after);
    let b = Authority::replay(&d.a.log_jsonl(), Policy::default()).unwrap();
    assert_eq!(b.state_hash(), d.a.state_hash());
    // Normal geometry again clears it.
    d.run(set_construction("s1", "diag", false));
    assert!(!d.a.state().sketch("s1").unwrap().sketch.is_infinite("diag"));
}
