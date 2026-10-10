//! How a document is organised: names shown for things, suppressing features, the rollback bar,
//! folders, and replacing a feature's recipe. Each is undoable, replays, and is part of the hash.

use core::f64::consts::PI;
use dc_doc::{
    Actor, Authority, Command, CommandKind as K, DiagCode, EndMode, Extent, FeatureSpec, Outcome,
    Policy, Query, QueryResult,
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
    fn try_run(&mut self, kind: K) -> Result<Outcome, dc_doc::Diagnostic> {
        self.n += 1;
        self.a.submit(Command {
            id: format!("c{}", self.n),
            actor: Actor::Human("ann".into()),
            base_revision: self.a.revision(),
            kind,
        })
    }
    fn run(&mut self, kind: K) {
        self.try_run(kind)
            .unwrap_or_else(|d| panic!("rejected: {d:?}"));
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
    fn profile(&self, sketch: &str) -> String {
        match self
            .a
            .query(&Query::SketchProfiles {
                sketch: sketch.into(),
                after: None,
                limit: None,
            })
            .unwrap()
        {
            QueryResult::SketchProfiles { profiles, .. } => profiles[0].id.clone(),
            other => panic!("{other:?}"),
        }
    }
}

fn sketch_at(d: &mut Doc, name: &str, offset: f64) {
    d.run(K::SketchCreate {
        sketch: name.into(),
        plane: PlaneSpec::Xy { offset },
    });
}

fn add(d: &mut Doc, sketch: &str, id: &str, e: E) {
    d.run(K::SketchAddEntity {
        sketch: sketch.into(),
        entity_id: id.into(),
        entity: e,
    });
}

/// A `w` x `h` rectangle centred on the origin.
fn rectangle(d: &mut Doc, sketch: &str, w: f64, h: f64) {
    let (x, y) = (w / 2.0, h / 2.0);
    for (id, px, py) in [("a", -x, -y), ("b", x, -y), ("c", x, y), ("e", -x, y)] {
        add(d, sketch, id, E::Point { x: px, y: py });
    }
    for (id, p, q) in [
        ("l1", "a", "b"),
        ("l2", "b", "c"),
        ("l3", "c", "e"),
        ("l4", "e", "a"),
    ] {
        add(
            d,
            sketch,
            id,
            E::Line {
                a: p.into(),
                b: q.into(),
            },
        );
    }
}

fn disc(d: &mut Doc, sketch: &str, x: f64, y: f64, r: f64) {
    add(d, sketch, "cc", E::Point { x, y });
    add(
        d,
        sketch,
        "k",
        E::Circle {
            center: "cc".into(),
            radius: r,
        },
    );
}

/// A 40 x 20 x 10 plate, and the name of its profile.
fn plate(d: &mut Doc) {
    sketch_at(d, "base", 0.0);
    rectangle(d, "base", 40.0, 20.0);
    let p = d.profile("base");
    d.run(K::FeatureExtrude {
        feature: "plate".into(),
        sketch: "base".into(),
        profiles: vec![p],
        depth: 10.0,
        reverse: false,
        extent: Extent::default(),
    });
}

const PLATE: f64 = 40.0 * 20.0 * 10.0;

fn near(a: f64, b: f64) -> bool {
    (a - b).abs() < 1e-6 * b.abs().max(1.0)
}

/// plate (40 x 20 x 10), peg (a boss on it), hole (a cut through the peg): a chain of three.
fn chain(d: &mut Doc) {
    plate(d);
    sketch_at(d, "top", 10.0);
    disc(d, "top", -12.0, 0.0, 3.0);
    let p = d.profile("top");
    d.run(K::FeatureBoss {
        feature: "peg".into(),
        target: "plate".into(),
        sketch: "top".into(),
        profiles: vec![p],
        depth: 5.0,
        reverse: false,
        extent: Extent::default(),
        pattern: None,
    });
    sketch_at(d, "bore", 0.0);
    disc(d, "bore", -12.0, 0.0, 1.0);
    let b = d.profile("bore");
    d.run(K::FeatureCut {
        feature: "hole".into(),
        target: "peg".into(),
        sketch: "bore".into(),
        profiles: vec![b],
        depth: None,
        reverse: false,
        extent: Extent {
            mode: EndMode::ThroughBoth,
            ..Extent::default()
        },
        pattern: None,
    });
}

fn results(d: &Doc) -> Vec<String> {
    d.a.state().result_features()
}

fn org(d: &Doc) -> dc_doc::QueryResult {
    d.a.query(&Query::Organization).unwrap()
}

#[test]
fn labels_rename_for_people_while_ids_stay_and_everything_is_undoable_and_replays() {
    let mut d = Doc::new();
    chain(&mut d);
    let before = d.a.state().hash();
    d.run(K::FeatureSetLabel {
        feature: "peg".into(),
        label: "  Locating peg  ".into(),
    });
    d.run(K::SketchSetLabel {
        sketch: "top".into(),
        label: "Peg outline".into(),
    });
    assert_eq!(d.a.state().label("feature", "peg"), "Locating peg");
    assert_eq!(d.a.state().label("sketch", "top"), "Peg outline");
    assert_eq!(
        d.a.state().label("feature", "plate"),
        "plate",
        "no label shows the id"
    );
    // References are by id and keep working: the hole still cuts the peg, volumes unchanged.
    assert!(d.a.state().feature("peg").is_some() && d.a.state().feature("hole").is_some());
    assert_eq!(
        d.a.state().hash(),
        d.a.state().recompute_hash(),
        "the incremental hash agrees"
    );
    assert_ne!(d.a.state().hash(), before);
    // Clearing a label (empty text) goes back to the id and to the old hash.
    d.run(K::FeatureSetLabel {
        feature: "peg".into(),
        label: "".into(),
    });
    d.run(K::SketchSetLabel {
        sketch: "top".into(),
        label: "".into(),
    });
    assert_eq!(
        d.a.state().hash(),
        before,
        "no labels left is the same document"
    );
    // Bad labels and unknown things are refused.
    let e = d
        .try_run(K::FeatureSetLabel {
            feature: "peg".into(),
            label: "x".repeat(81),
        })
        .unwrap_err();
    assert_eq!(e.code, DiagCode::InvalidValue, "{e:?}");
    let e = d
        .try_run(K::FeatureSetLabel {
            feature: "ghost".into(),
            label: "a".into(),
        })
        .unwrap_err();
    assert_eq!(e.code, DiagCode::NotFound, "{e:?}");
    // Undo and redo, and a replay of the whole log.
    d.run(K::FeatureSetLabel {
        feature: "hole".into(),
        label: "Bore".into(),
    });
    let labelled = d.a.state().hash();
    d.run(K::HistoryUndo { revision: None });
    assert_eq!(d.a.state().label("feature", "hole"), "hole");
    d.run(K::HistoryRedo { revision: None });
    assert_eq!(d.a.state().hash(), labelled);
    let replayed = Authority::replay(&d.a.log_jsonl(), Policy::default()).expect("replays");
    assert_eq!(replayed.state().hash(), labelled);
    assert_eq!(replayed.state().label("feature", "hole"), "Bore");
    // Deleting the feature takes its label with it, and undo brings both back.
    d.run(K::FeatureDelete {
        feature: "hole".into(),
    });
    assert!(d.a.state().meta("label/feature/hole").is_none());
    d.run(K::HistoryUndo { revision: None });
    assert_eq!(d.a.state().label("feature", "hole"), "Bore");
}

#[test]
fn suppressing_a_feature_switches_off_everything_built_on_it() {
    let mut d = Doc::new();
    chain(&mut d);
    assert_eq!(results(&d), ["hole"]);
    d.run(K::FeatureSuppress {
        feature: "hole".into(),
        suppressed: true,
    });
    assert_eq!(
        results(&d),
        ["peg"],
        "the peg is the end of the chain again"
    );
    d.run(K::FeatureSuppress {
        feature: "peg".into(),
        suppressed: true,
    });
    assert_eq!(
        results(&d),
        ["plate"],
        "the hole goes with the peg it is cut in"
    );
    let dc_doc::QueryResult::Organization {
        suppressed, hidden, ..
    } = org(&d)
    else {
        panic!()
    };
    assert_eq!(suppressed, ["hole", "peg"]);
    assert_eq!(hidden, ["hole", "peg"]);
    // Suppressing the peg alone also hides the hole: unsuppress the hole and it stays hidden.
    d.run(K::FeatureSuppress {
        feature: "hole".into(),
        suppressed: false,
    });
    let dc_doc::QueryResult::Organization { hidden, .. } = org(&d) else {
        panic!()
    };
    assert_eq!(hidden, ["hole", "peg"]);
    d.run(K::FeatureSuppress {
        feature: "peg".into(),
        suppressed: false,
    });
    assert_eq!(results(&d), ["hole"]);
    assert_eq!(d.a.state().hash(), d.a.state().recompute_hash());
    // Undo / redo a suppression.
    d.run(K::FeatureSuppress {
        feature: "peg".into(),
        suppressed: true,
    });
    d.run(K::HistoryUndo { revision: None });
    assert_eq!(results(&d), ["hole"]);
    // A feature's volume is still known while it is off (it stays built, ready to switch on).
    assert!(d.summary("peg").volume > PLATE);
    let e = d
        .try_run(K::FeatureSuppress {
            feature: "nope".into(),
            suppressed: true,
        })
        .unwrap_err();
    assert_eq!(e.code, DiagCode::NotFound, "{e:?}");
}

#[test]
fn the_rollback_bar_switches_off_what_comes_after_and_nothing_can_be_added_meanwhile() {
    let mut d = Doc::new();
    chain(&mut d);
    d.run(K::DocumentRollback {
        to: Some("peg".into()),
    });
    assert_eq!(results(&d), ["peg"], "the hole is after the bar");
    let dc_doc::QueryResult::Organization {
        rollback,
        hidden,
        order,
        ..
    } = org(&d)
    else {
        panic!()
    };
    assert_eq!(rollback.as_deref(), Some("peg"));
    assert_eq!(hidden, ["hole"]);
    assert_eq!(order, ["plate", "peg", "hole"]);
    // Adding while rolled back is refused, with the way out.
    sketch_at(&mut d, "extra", 0.0);
    disc(&mut d, "extra", 5.0, 5.0, 1.0);
    let p = d.profile("extra");
    let e = d
        .try_run(K::FeatureCut {
            feature: "late".into(),
            target: "peg".into(),
            sketch: "extra".into(),
            profiles: vec![p],
            depth: None,
            reverse: false,
            extent: Extent::default(),
            pattern: None,
        })
        .unwrap_err();
    assert_eq!(e.code, DiagCode::RolledBack, "{e:?}");
    assert!(
        e.suggestion.iter().any(|s| s.contains("roll forward")),
        "{e:?}"
    );
    // Moving the bar further back, and then forward again.
    d.run(K::DocumentRollback {
        to: Some("plate".into()),
    });
    assert_eq!(results(&d), ["plate"]);
    d.run(K::DocumentRollback { to: None });
    assert_eq!(results(&d), ["hole"]);
    assert_eq!(d.a.state().hash(), d.a.state().recompute_hash());
    // Deleting a feature the bar does not sit after leaves the bar alone.
    d.run(K::DocumentRollback {
        to: Some("peg".into()),
    });
    d.run(K::FeatureDelete {
        feature: "hole".into(),
    });
    assert_eq!(d.a.state().rollback(), Some("peg"));
    // Deleting the feature the bar sits after puts the bar at the end.
    d.run(K::FeatureDelete {
        feature: "peg".into(),
    });
    assert!(
        d.a.state().rollback().is_none(),
        "no bar left pointing at nothing"
    );
    // Undo restores the bar as it was.
    d.run(K::HistoryUndo { revision: None });
    assert_eq!(d.a.state().rollback(), Some("peg"));
    let replayed = Authority::replay(&d.a.log_jsonl(), Policy::default()).expect("replays");
    assert_eq!(replayed.state().hash(), d.a.state().hash());
}

#[test]
fn folders_group_features_a_feature_is_in_one_folder_and_deleting_it_empties_its_place() {
    let mut d = Doc::new();
    chain(&mut d);
    d.run(K::FolderSet {
        folder: "Holes".into(),
        features: vec!["hole".into()],
    });
    d.run(K::FolderSet {
        folder: "Base".into(),
        features: vec!["plate".into(), "peg".into()],
    });
    let folders = d.a.state().folders();
    assert_eq!(
        folders,
        vec![
            (
                "Base".to_owned(),
                vec!["plate".to_owned(), "peg".to_owned()]
            ),
            ("Holes".to_owned(), vec!["hole".to_owned()])
        ]
    );
    // Moving the peg to Holes takes it out of Base.
    d.run(K::FolderSet {
        folder: "Holes".into(),
        features: vec!["hole".into(), "peg".into()],
    });
    let f = d.a.state().folders();
    assert_eq!(f[0].1, ["plate"]);
    assert_eq!(f[1].1, ["hole", "peg"]);
    // An empty list deletes the folder; deleting a feature leaves the others in place.
    d.run(K::FolderSet {
        folder: "Base".into(),
        features: vec![],
    });
    assert!(d.a.state().meta("folder/Base").is_none());
    d.run(K::FeatureDelete {
        feature: "hole".into(),
    });
    assert_eq!(d.a.state().folders()[0].1, ["peg"]);
    d.run(K::HistoryUndo { revision: None });
    assert_eq!(d.a.state().folders()[0].1, ["hole", "peg"]);
    assert_eq!(d.a.state().hash(), d.a.state().recompute_hash());
    // Refusals.
    for (name, list, code) in [
        ("", vec!["peg"], DiagCode::InvalidName),
        ("a/b", vec!["peg"], DiagCode::InvalidName),
        ("X", vec!["ghost"], DiagCode::NotFound),
        ("X", vec!["peg", "peg"], DiagCode::InvalidValue),
    ] {
        let e = d
            .try_run(K::FolderSet {
                folder: name.into(),
                features: list.iter().map(|s| (*s).to_owned()).collect(),
            })
            .unwrap_err();
        assert_eq!(e.code, code, "{name:?} {list:?}: {e:?}");
    }
    let replayed = Authority::replay(&d.a.log_jsonl(), Policy::default()).expect("replays");
    assert_eq!(replayed.state().hash(), d.a.state().hash());
}

#[test]
fn updating_a_feature_replaces_its_recipe_rebuilds_what_depends_on_it_and_refuses_what_breaks() {
    let mut d = Doc::new();
    chain(&mut d);
    let FeatureSpec::Boss {
        target,
        sketch,
        profiles,
        reverse,
        extent,
        pattern,
        ..
    } = d.a.state().feature("peg").unwrap().spec.clone()
    else {
        panic!()
    };
    let taller = |depth: f64| FeatureSpec::Boss {
        target: target.clone(),
        sketch: sketch.clone(),
        profiles: profiles.clone(),
        depth,
        reverse,
        extent: extent.clone(),
        pattern: pattern.clone(),
    };
    let v0 = d.summary("hole").volume;
    d.run(K::FeatureUpdate {
        feature: "peg".into(),
        spec: taller(8.0),
    });
    // The peg is 3 taller and the hole through it follows: volume grows by the peg's extra, less the bore.
    let grew = d.summary("hole").volume - v0;
    assert!(near(grew, PI * (9.0 - 1.0) * 3.0), "{grew}");
    // Undo restores the recipe and the dependent's solid.
    d.run(K::HistoryUndo { revision: None });
    assert!(near(d.summary("hole").volume, v0));
    // The kind cannot change.
    let e = d
        .try_run(K::FeatureUpdate {
            feature: "peg".into(),
            spec: FeatureSpec::Extrude {
                sketch: "top".into(),
                profiles: vec!["x".into()],
                depth: 1.0,
                reverse: false,
                extent: Extent::default(),
            },
        })
        .unwrap_err();
    assert_eq!(e.code, DiagCode::WrongKind, "{e:?}");
    // A recipe that cannot be built (a profile that does not exist) is refused and changes nothing.
    let h = d.a.state().hash();
    let bad = FeatureSpec::Boss {
        target: target.clone(),
        sketch: sketch.clone(),
        profiles: vec!["profile.nothing".into()],
        depth: 5.0,
        reverse,
        extent: extent.clone(),
        pattern: None,
    };
    let e = d
        .try_run(K::FeatureUpdate {
            feature: "peg".into(),
            spec: bad,
        })
        .unwrap_err();
    assert!(
        matches!(e.code, DiagCode::NotFound | DiagCode::FeatureFailed),
        "{e:?}"
    );
    assert_eq!(d.a.state().hash(), h);
    // And it replays.
    d.run(K::FeatureUpdate {
        feature: "peg".into(),
        spec: taller(7.0),
    });
    let replayed = Authority::replay(&d.a.log_jsonl(), Policy::default()).expect("replays");
    assert_eq!(replayed.state().hash(), d.a.state().hash());
}

#[test]
fn the_new_commands_round_trip_through_json_and_old_documents_hash_as_before() {
    let kinds = vec![
        K::FeatureSetLabel {
            feature: "a".into(),
            label: "A".into(),
        },
        K::SketchSetLabel {
            sketch: "s".into(),
            label: "S".into(),
        },
        K::FeatureSuppress {
            feature: "a".into(),
            suppressed: true,
        },
        K::DocumentRollback {
            to: Some("a".into()),
        },
        K::DocumentRollback { to: None },
        K::FolderSet {
            folder: "F".into(),
            features: vec!["a".into()],
        },
    ];
    for kind in kinds {
        let cmd = Command {
            id: "c".into(),
            actor: Actor::Agent("x".into()),
            base_revision: dc_doc::Revision(0),
            kind,
        };
        let json = serde_json::to_string(&cmd).unwrap();
        let back: Command = serde_json::from_str(&json).unwrap_or_else(|e| panic!("{e}: {json}"));
        assert_eq!(back, cmd);
    }
    // A document that uses none of it hashes exactly as the incremental and the from-scratch
    // computations say (the organisation adds nothing when empty).
    let mut d = Doc::new();
    chain(&mut d);
    assert_eq!(d.a.state().hash(), d.a.state().recompute_hash());
    let dc_doc::QueryResult::Organization {
        labels,
        folders,
        suppressed,
        rollback,
        hidden,
        order,
    } = org(&d)
    else {
        panic!()
    };
    assert!(labels.is_empty() && folders.is_empty() && suppressed.is_empty() && hidden.is_empty());
    assert_eq!(rollback, None);
    assert_eq!(order, ["plate", "peg", "hole"]);
}
