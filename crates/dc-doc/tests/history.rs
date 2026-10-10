//! Undo / redo: exact, per actor, conflict-checked, and deterministic on replay.

use dc_doc::{
    Actor, Authority, Command, CommandKind as K, DiagCode, Diagnostic, Expected, Outcome, Policy,
    Query, QueryResult, Revision, Role,
};
use dc_sketch::{Constraint as C, Entity as E};

struct Doc {
    a: Authority,
    n: u64,
}

fn ann() -> Actor {
    Actor::Human("ann".into())
}
fn bob() -> Actor {
    Actor::Human("bob".into())
}

impl Doc {
    fn new() -> Self {
        Doc {
            a: Authority::default(),
            n: 0,
        }
    }
    fn cmd_as(&mut self, who: &Actor, kind: K) -> Command {
        self.n += 1;
        Command {
            id: format!("c{}", self.n),
            actor: who.clone(),
            base_revision: self.a.revision(),
            kind,
        }
    }
    fn run_as(&mut self, who: &Actor, kind: K) -> Outcome {
        let c = self.cmd_as(who, kind);
        self.a
            .submit(c)
            .unwrap_or_else(|d| panic!("rejected: {d:?}"))
    }
    fn try_as(&mut self, who: &Actor, kind: K) -> Result<Outcome, Diagnostic> {
        let c = self.cmd_as(who, kind);
        self.a.submit(c)
    }
    fn run(&mut self, kind: K) -> Outcome {
        self.run_as(&ann(), kind)
    }
    fn try_run(&mut self, kind: K) -> Result<Outcome, Diagnostic> {
        self.try_as(&ann(), kind)
    }
}

fn set(name: &str, v: f64) -> K {
    K::ParamSet {
        name: name.into(),
        value: v,
        expected: Expected::Any,
    }
}
fn undo() -> K {
    K::HistoryUndo { revision: None }
}
fn redo() -> K {
    K::HistoryRedo { revision: None }
}
fn undo_rev(r: u64) -> K {
    K::HistoryUndo {
        revision: Some(Revision(r)),
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
fn volume(d: &Doc, f: &str) -> f64 {
    match d
        .a
        .query(&Query::FeatureSummary { feature: f.into() })
        .unwrap()
    {
        QueryResult::FeatureSummary { info } => info.summary.volume,
        other => panic!("{other:?}"),
    }
}

/// A dimensioned rectangle in sketch `s` extruded as feature `e`.
fn box_doc() -> Doc {
    let mut d = Doc::new();
    d.run(K::SketchCreate {
        sketch: "s".into(),
        plane: Default::default(),
    });
    for (id, x, y) in [
        ("a", 0.0, 0.0),
        ("b", 70.0, 5.0),
        ("c", 75.0, 45.0),
        ("d", 3.0, 38.0),
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
    d.run(k("s", "h1", C::Horizontal { line: "ab".into() }));
    d.run(k("s", "v1", C::Vertical { line: "bc".into() }));
    d.run(k("s", "h2", C::Horizontal { line: "cd".into() }));
    d.run(k("s", "v2", C::Vertical { line: "da".into() }));
    d.run(k(
        "s",
        "w",
        C::Length {
            line: "ab".into(),
            value: 80.0,
        },
    ));
    d.run(k(
        "s",
        "h",
        C::Length {
            line: "bc".into(),
            value: 40.0,
        },
    ));
    d.run(k(
        "s",
        "o",
        C::Fixed {
            point: "a".into(),
            x: 0.0,
            y: 0.0,
        },
    ));
    d.run(K::FeatureExtrude {
        feature: "e".into(),
        sketch: "s".into(),
        profiles: vec!["profile.ab".into()],
        depth: 10.0,
        reverse: false,
        extent: Default::default(),
    });
    d
}

#[test]
fn undo_restores_the_exact_state_and_redo_restores_it_back() {
    let mut d = Doc::new();
    d.run(set("x", 1.0));
    let h0 = d.a.state_hash();
    d.run(set("x", 2.0));
    d.run(set("y", 9.0));
    let h2 = d.a.state_hash();

    d.run(undo()); // y
    assert_eq!(d.a.state().get("y"), None);
    d.run(undo()); // x: 2 -> 1
    assert_eq!(d.a.state().get("x"), Some(1.0));
    assert_eq!(
        d.a.state_hash(),
        h0,
        "hash is identical to before those commands"
    );

    d.run(redo()); // most recently undone: x
    assert_eq!(d.a.state().get("x"), Some(2.0));
    d.run(redo());
    assert_eq!(d.a.state().get("y"), Some(9.0));
    assert_eq!(d.a.state_hash(), h2);
    assert_eq!(d.a.state().hash(), d.a.state().recompute_hash());
}

#[test]
fn undo_and_redo_are_new_revisions_in_the_history() {
    let mut d = Doc::new();
    d.run(set("x", 1.0));
    let before = d.a.revision();
    let u = d.run(undo());
    assert_eq!(u.revision.0, before.0 + 1);
    assert_eq!(u.changes.len(), 1);
    assert_eq!(
        (u.changes[0].entity.as_str(), u.changes[0].after.clone()),
        ("x", None)
    );
    let log = d.a.log();
    assert!(
        matches!(log.last().unwrap().command.kind, K::HistoryUndo { revision: Some(Revision(r)) } if r == before.0),
        "the logged command names the concrete revision it undid"
    );
}

#[test]
fn nothing_to_undo_and_double_undo_are_clear_errors() {
    let mut d = Doc::new();
    assert_eq!(d.try_run(undo()).unwrap_err().code, DiagCode::NothingToUndo);
    assert_eq!(d.try_run(redo()).unwrap_err().code, DiagCode::NothingToUndo);
    d.run(set("x", 1.0));
    d.run(undo());
    assert_eq!(
        d.try_run(undo()).unwrap_err().code,
        DiagCode::NothingToUndo,
        "already undone"
    );
    assert_eq!(
        d.try_run(undo_rev(1)).unwrap_err().code,
        DiagCode::NothingToUndo
    );
    d.run(redo());
    assert_eq!(d.try_run(redo()).unwrap_err().code, DiagCode::NothingToUndo);
    // Undo/redo revisions themselves are not undoable commands.
    assert_eq!(
        d.try_run(undo_rev(2)).unwrap_err().code,
        DiagCode::NothingToUndo
    );
    assert_eq!(
        d.try_run(undo_rev(99)).unwrap_err().code,
        DiagCode::NothingToUndo
    );
}

#[test]
fn you_can_only_undo_your_own_work() {
    let mut d = Doc::new();
    d.run_as(&ann(), set("x", 1.0));
    d.run_as(&bob(), set("y", 2.0));
    // Bob's default target is his own command, not Ann's.
    d.run_as(&bob(), undo());
    assert_eq!(d.a.state().get("y"), None);
    assert_eq!(d.a.state().get("x"), Some(1.0), "Ann's work is untouched");
    // Explicitly targeting someone else's revision is refused.
    assert_eq!(
        d.try_as(&bob(), undo_rev(1)).unwrap_err().code,
        DiagCode::PermissionDenied
    );
    // Agents are subject to the same rules (and to their role).
    let agent = Actor::Agent("claude".into());
    d.a.set_role(&agent, Role::Full);
    d.run_as(&agent, set("z", 3.0));
    assert_eq!(
        d.try_as(&agent, undo_rev(1)).unwrap_err().code,
        DiagCode::PermissionDenied
    );
    d.run_as(&agent, undo());
    assert_eq!(d.a.state().get("z"), None);
    assert_eq!(
        d.a.log().last().unwrap().command.actor,
        agent,
        "attributed to the agent"
    );
}

#[test]
fn undoing_around_independent_work_is_fine_but_conflicts_are_refused() {
    let mut d = Doc::new();
    d.run(set("x", 1.0)); // rev 1
    d.run(set("y", 2.0)); // rev 2: independent of x
    d.run(set("x", 5.0)); // rev 3: touches x again

    // Undoing rev 1 would overwrite rev 3's change to x: refused, and it says why.
    let e = d.try_run(undo_rev(1)).unwrap_err();
    assert_eq!(e.code, DiagCode::UndoConflict);
    assert!(e.message.contains('3') && e.entities == ["x"], "{e:?}");
    // But rev 2 (y) is independent of everything after it: allowed even though not the latest.
    d.run(undo_rev(2));
    assert_eq!(d.a.state().get("y"), None);
    assert_eq!(d.a.state().get("x"), Some(5.0));
    // Once rev 3 is undone, rev 1 can be undone too.
    d.run(undo_rev(3));
    d.run(undo_rev(1));
    assert_eq!(d.a.state().get("x"), None);
}

#[test]
fn redo_is_refused_if_something_changed_the_same_item_meanwhile() {
    let mut d = Doc::new();
    d.run(set("x", 1.0));
    d.run(undo());
    d.run(set("x", 7.0)); // a new command on the same item
    let e = d.try_run(redo()).unwrap_err();
    assert_eq!(e.code, DiagCode::UndoConflict);
    assert_eq!(
        d.a.state().get("x"),
        Some(7.0),
        "refusing leaves the document alone"
    );
}

#[test]
fn undo_through_a_sketch_edit_restores_the_sketch_and_the_rebuilt_solid() {
    let mut d = box_doc();
    let (h, v) = (d.a.state_hash(), volume(&d, "e"));
    assert!((v - 32_000.0).abs() < 1e-6);

    d.run(K::SketchSetValue {
        sketch: "s".into(),
        constraint_id: "w".into(),
        value: 100.0,
        expected: Expected::Any,
    });
    assert!((volume(&d, "e") - 40_000.0).abs() < 1e-6);
    let h_edited = d.a.state_hash();

    let u = d.run(undo());
    assert!(
        (volume(&d, "e") - 32_000.0).abs() < 1e-6,
        "the solid is back"
    );
    assert_eq!(d.a.state_hash(), h, "bit-identical to before the edit");
    assert!(
        u.changes.iter().any(|c| c.entity == "feature/e"),
        "{:?}",
        u.changes
    );
    assert!(
        u.changes.iter().any(|c| c.entity == "sketch/s/w"),
        "{:?}",
        u.changes
    );
    assert!(u.sketch_reports["s"].is_fully_constrained());

    d.run(redo());
    assert_eq!(d.a.state_hash(), h_edited);
    assert!((volume(&d, "e") - 40_000.0).abs() < 1e-6);
}

#[test]
fn undoing_a_transaction_undoes_all_of_it_at_once() {
    let mut d = box_doc();
    let h = d.a.state_hash();
    d.run(K::Transaction {
        label: "resize everything".into(),
        ops: vec![
            K::SketchSetValue {
                sketch: "s".into(),
                constraint_id: "w".into(),
                value: 60.0,
                expected: Expected::Any,
            },
            K::FeatureSetDepth {
                feature: "e".into(),
                depth: 25.0,
                expected: Expected::Any,
            },
            set("note", 1.0),
        ],
    });
    assert!((volume(&d, "e") - 60.0 * 40.0 * 25.0).abs() < 1e-6);
    d.run(undo());
    assert_eq!(d.a.state_hash(), h);
    assert_eq!(d.a.state().get("note"), None);
    d.run(redo());
    assert!((volume(&d, "e") - 60.0 * 40.0 * 25.0).abs() < 1e-6);
}

#[test]
fn undoing_creation_and_deletion() {
    let mut d = box_doc();
    let h = d.a.state_hash();
    // Delete the feature, then bring it back with undo.
    d.run(K::FeatureDelete {
        feature: "e".into(),
    });
    assert!(d.a.state().feature("e").is_none());
    d.run(undo());
    assert_eq!(d.a.state_hash(), h);
    assert!(d.a.solid("e").is_some());
    // Undo the feature's creation: the sketch stays, the feature goes.
    d.run(K::FeatureDelete {
        feature: "e".into(),
    });
    d.run(undo()); // restore
    let creation =
        d.a.log()
            .iter()
            .find(|e| matches!(e.command.kind, K::FeatureExtrude { .. }))
            .unwrap()
            .revision;
    // Creation can't be undone while later work (the delete/undo pair) is considered; the delete
    // was undone so it is not active, hence this is allowed:
    d.run(undo_rev(creation.0));
    assert!(d.a.state().feature("e").is_none());
    assert_eq!(d.a.state().sketch_count(), 1);
}

#[test]
fn dry_run_of_undo_and_redo_predicts_exactly_and_leaves_no_trace() {
    let mut d = box_doc();
    d.run(K::FeatureSetDepth {
        feature: "e".into(),
        depth: 30.0,
        expected: Expected::Any,
    });
    let h = d.a.state_hash();
    let log_len = d.a.log().len();

    let c = d.cmd_as(&ann(), undo());
    let predicted = d.a.dry_run(&c).unwrap();
    assert_eq!(d.a.state_hash(), h, "state unchanged by the dry run");
    assert_eq!(d.a.log().len(), log_len);
    assert!((volume(&d, "e") - 80.0 * 40.0 * 30.0).abs() < 1e-6);
    // And the record is still active: a real undo works and matches the prediction.
    let real = d.a.submit(c).unwrap();
    assert_eq!(real, predicted);

    let c = d.cmd_as(&ann(), redo());
    let predicted = d.a.dry_run(&c).unwrap();
    assert!(
        (volume(&d, "e") - 32_000.0).abs() < 1e-6,
        "dry-run redo did not apply"
    );
    assert_eq!(d.a.submit(c).unwrap(), predicted);
    assert!((volume(&d, "e") - 80.0 * 40.0 * 30.0).abs() < 1e-6);
}

#[test]
fn history_commands_cannot_hide_inside_transactions() {
    let mut d = Doc::new();
    d.run(set("x", 1.0));
    let h = d.a.state_hash();
    let e = d
        .try_run(K::Transaction {
            label: "sneaky".into(),
            ops: vec![set("y", 2.0), undo()],
        })
        .unwrap_err();
    assert_eq!(e.code, DiagCode::InvalidValue);
    assert_eq!(d.a.state_hash(), h);
}

#[test]
fn replay_reproduces_undo_redo_history_exactly() {
    let mut d = box_doc();
    d.run(K::SketchSetValue {
        sketch: "s".into(),
        constraint_id: "w".into(),
        value: 90.0,
        expected: Expected::Any,
    });
    d.run(set("p", 4.0));
    d.run(undo()); // p
    d.run(undo()); // w edit
    d.run(redo()); // w edit again
    let jsonl = d.a.log_jsonl();
    let mut b = Authority::replay(&jsonl, Policy::default()).unwrap();
    assert_eq!(b.state_hash(), d.a.state_hash());
    assert_eq!(b.log_jsonl(), jsonl);
    // The replayed document has working undo bookkeeping, not just the same state.
    let c = Command {
        id: "after".into(),
        actor: ann(),
        base_revision: b.revision(),
        kind: undo(),
    };
    b.submit(c).unwrap();
    assert!(
        (match b
            .query(&Query::FeatureSummary {
                feature: "e".into()
            })
            .unwrap()
        {
            QueryResult::FeatureSummary { info } => info.summary.volume,
            _ => unreachable!(),
        } - 32_000.0)
            .abs()
            < 1e-6
    );
    // Tampering with which revision was undone is caught.
    let tampered = jsonl.replacen(
        "\"type\":\"history.undo\",\"revision\":",
        "\"type\":\"history.undo\",\"revision\":1",
        1,
    );
    assert!(Authority::replay(&tampered, Policy::default()).is_err());
}

#[test]
fn history_commands_round_trip_through_json() {
    for kind in [
        undo(),
        redo(),
        undo_rev(7),
        K::HistoryRedo {
            revision: Some(Revision(3)),
        },
    ] {
        let c = Command {
            id: "c".into(),
            actor: Actor::Agent("claude".into()),
            base_revision: Revision(1),
            kind,
        };
        let json = serde_json::to_string(&c).unwrap();
        assert_eq!(serde_json::from_str::<Command>(&json).unwrap(), c, "{json}");
    }
    // Omitted revision is accepted on input.
    let c: Command = serde_json::from_str(
        r#"{"id":"c","actor":{"kind":"human","name":"a"},"base_revision":0,"type":"history.undo"}"#,
    )
    .unwrap();
    assert!(matches!(c.kind, K::HistoryUndo { revision: None }));
}
