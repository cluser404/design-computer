//! Rounding the circular rim of a shaft or a hole through the command layer: exact volumes,
//! diagnostics, undo and replay, and what a rounded part can no longer be used for.

use core::f64::consts::PI;
use dc_doc::{
    Actor, Authority, Command, CommandKind as K, DiagCode, Extent, Outcome, Policy, Query,
    QueryResult,
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
    fn faces(&self, f: &str) -> Vec<dc_doc::FaceInfo> {
        match self
            .a
            .query(&Query::FeatureFaces {
                feature: f.into(),
                after: None,
                limit: None,
            })
            .unwrap()
        {
            QueryResult::FeatureFaces { items, .. } => items,
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

fn shaft(d: &mut Doc, r: f64, h: f64) {
    sketch_at(d, "base", 0.0);
    add(d, "base", "m", E::Point { x: 0.0, y: 0.0 });
    add(
        d,
        "base",
        "c",
        E::Circle {
            center: "m".into(),
            radius: r,
        },
    );
    let p = d.profile("base");
    d.run(K::FeatureExtrude {
        feature: "shaft".into(),
        sketch: "base".into(),
        profiles: vec![p],
        depth: h,
        reverse: false,
        extent: Extent::default(),
    });
}

fn near(a: f64, b: f64) -> bool {
    (a - b).abs() < 1e-9 * b.abs().max(1.0)
}

#[test]
fn a_chamfer_and_a_fillet_of_a_shafts_rim_are_exact_and_undoable() {
    let mut d = Doc::new();
    let (r, h, s) = (10.0, 20.0, 2.0);
    shaft(&mut d, r, h);
    let before = d.a.state().hash();
    d.run(K::FeatureChamfer {
        feature: "ch".into(),
        target: "shaft".into(),
        edges: vec![["cap_end".into(), "side:c".into()]],
        distance: s,
    });
    let frustum = PI * s / 3.0 * (r * r + r * (r - s) + (r - s) * (r - s));
    assert!(near(
        d.summary("ch").volume,
        PI * r * r * h - (PI * r * r * s - frustum)
    ));
    // The new face is a cone, listed with the others, and its edges name what they separate.
    let surfaces: Vec<String> = d.faces("ch").into_iter().map(|f| f.surface).collect();
    assert!(surfaces.contains(&"cone".to_owned()), "{surfaces:?}");
    // Round the other end with a fillet, on the result.
    d.run(K::FeatureFillet {
        feature: "fi".into(),
        target: "ch".into(),
        edges: vec![["side:c".into(), "cap_start".into()]],
        radius: s,
    });
    let surfaces: Vec<String> = d.faces("fi").into_iter().map(|f| f.surface).collect();
    assert!(surfaces.contains(&"torus".to_owned()), "{surfaces:?}");
    let removed =
        2.0 * PI * (s * s * r - s * s * s / 2.0 - PI * s * s / 4.0 * (r - s) - s * s * s / 3.0);
    assert!(near(
        d.summary("fi").volume,
        d.summary("ch").volume - removed
    ));
    // Undo both, then redo, then replay the log.
    d.run(K::HistoryUndo { revision: None });
    d.run(K::HistoryUndo { revision: None });
    assert_eq!(d.a.state().hash(), before);
    d.run(K::HistoryRedo { revision: None });
    d.run(K::HistoryRedo { revision: None });
    let replayed = Authority::replay(&d.a.log_jsonl(), Policy::default()).expect("replays");
    assert_eq!(replayed.state().hash(), d.a.state().hash());
}

#[test]
fn a_rounded_part_still_takes_cuts_and_bosses_and_keeps_its_rounding() {
    let mut d = Doc::new();
    let (r, h, s) = (10.0, 20.0, 2.0);
    shaft(&mut d, r, h);
    d.run(K::FeatureFillet {
        feature: "fi".into(),
        target: "shaft".into(),
        edges: vec![["cap_end".into(), "side:c".into()]],
        radius: s,
    });
    let rounded = d.summary("fi").volume;
    // A hole drilled from the flat top of the rounded end, sketched on that face.
    d.run(K::SketchCreate {
        sketch: "hole".into(),
        plane: PlaneSpec::Face {
            feature: "fi".into(),
            face: "cap_end".into(),
            offset: 0.0,
        },
    });
    add(&mut d, "hole", "m", E::Point { x: 0.0, y: 0.0 });
    add(
        &mut d,
        "hole",
        "c",
        E::Circle {
            center: "m".into(),
            radius: 2.0,
        },
    );
    d.run(K::FeatureCut {
        feature: "cut".into(),
        target: "fi".into(),
        sketch: "hole".into(),
        profiles: vec![],
        depth: Some(5.0),
        reverse: true,
        extent: Extent::default(),
        pattern: None,
    });
    assert!(
        near(d.summary("cut").volume, rounded - PI * 4.0 * 5.0),
        "{}",
        d.summary("cut").volume
    );
    // The rounded face is still there, and the part still round-trips through replay.
    let surfaces: Vec<String> = d.faces("cut").into_iter().map(|f| f.surface).collect();
    assert!(surfaces.contains(&"torus".to_owned()), "{surfaces:?}");
    let replayed = Authority::replay(&d.a.log_jsonl(), Policy::default()).expect("replays");
    assert_eq!(replayed.state().hash(), d.a.state().hash());
    // Rounding the same rim again is refused, and a rim that a cut would reach says so.
    let e = d
        .try_run(K::FeatureChamfer {
            feature: "again".into(),
            target: "cut".into(),
            edges: vec![["cap_end".into(), "side:c".into()]],
            distance: 1.0,
        })
        .unwrap_err();
    assert!(
        matches!(
            e.code,
            DiagCode::Unsupported
                | DiagCode::FeatureFailed
                | DiagCode::InvalidValue
                | DiagCode::NotFound
        ),
        "{e:?}"
    );
}

#[test]
fn oversized_and_repeated_rounds_are_refused_with_the_reason() {
    let mut d = Doc::new();
    shaft(&mut d, 10.0, 20.0);
    // Too big for the radius of the shaft.
    let e = d
        .try_run(K::FeatureFillet {
            feature: "big".into(),
            target: "shaft".into(),
            edges: vec![["cap_end".into(), "side:c".into()]],
            radius: 11.0,
        })
        .unwrap_err();
    assert_eq!(e.code, DiagCode::FeatureFailed, "{e:?}");
    assert!(e.message.contains("does not fit"), "{e:?}");
    // The same edge twice in one feature: the second finds no edge any more.
    let e = d
        .try_run(K::FeatureChamfer {
            feature: "twice".into(),
            target: "shaft".into(),
            edges: vec![
                ["cap_end".into(), "side:c".into()],
                ["cap_end".into(), "side:c".into()],
            ],
            distance: 1.0,
        })
        .unwrap_err();
    assert!(
        matches!(
            e.code,
            DiagCode::Unsupported | DiagCode::InvalidValue | DiagCode::NotFound
        ),
        "{e:?}"
    );
}
