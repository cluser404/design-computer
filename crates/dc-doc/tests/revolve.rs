//! Revolved parts through the command layer: exact volumes, axes given as sketch axes or as a
//! line, later cuts, refusals with reasons, undo and replay.

use core::f64::consts::PI;
use dc_doc::{
    Actor, Authority, Command, CommandKind as K, DiagCode, EndMode, Extent, Outcome, Policy, Query,
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

fn near(a: f64, b: f64) -> bool {
    (a - b).abs() < 1e-6 * b.abs().max(1.0)
}

/// A closed polygon in sketch `name` (points `{name}p{i}`, lines `{name}e{i}`).
fn polygon(d: &mut Doc, sketch: &str, pts: &[(f64, f64)]) {
    for (i, (x, y)) in pts.iter().enumerate() {
        add(d, sketch, &format!("p{i}"), E::Point { x: *x, y: *y });
    }
    for i in 0..pts.len() {
        add(
            d,
            sketch,
            &format!("e{i}"),
            E::Line {
                a: format!("p{i}"),
                b: format!("p{}", (i + 1) % pts.len()),
            },
        );
    }
}

fn revolve(f: &str, sketch: &str, profile: &str, axis: &str) -> K {
    K::FeatureRevolve {
        feature: f.into(),
        sketch: sketch.into(),
        profiles: vec![profile.into()],
        axis: axis.into(),
        angle: None,
    }
}

fn revolve_by(f: &str, sketch: &str, profile: &str, axis: &str, angle: f64) -> K {
    K::FeatureRevolve {
        feature: f.into(),
        sketch: sketch.into(),
        profiles: vec![profile.into()],
        axis: axis.into(),
        angle: Some(angle),
    }
}

#[test]
fn a_shaft_revolved_about_a_sketch_axis_is_exact_and_can_be_drilled() {
    let mut d = Doc::new();
    sketch_at(&mut d, "s", 0.0);
    // x along the axis: radius 4 for 10, then 3 for 15, then 5 for 5.
    polygon(
        &mut d,
        "s",
        &[
            (0.0, 0.0),
            (30.0, 0.0),
            (30.0, 5.0),
            (25.0, 5.0),
            (25.0, 3.0),
            (10.0, 3.0),
            (10.0, 4.0),
            (0.0, 4.0),
        ],
    );
    let p = d.profile("s");
    d.run(revolve("shaft", "s", &p, "x"));
    let want = PI * (16.0 * 10.0 + 9.0 * 15.0 + 25.0 * 5.0);
    assert!(
        near(d.summary("shaft").volume, want),
        "{}",
        d.summary("shaft").volume
    );
    let s = d.summary("shaft");
    assert!(
        near(s.bounds_min[0], 0.0) && near(s.bounds_max[0], 30.0),
        "{s:?}"
    );
    assert!(
        near(s.bounds_max[1], 5.0) && near(s.bounds_min[1], -5.0),
        "{s:?}"
    );
    let names: Vec<_> = d.faces("shaft").into_iter().map(|f| f.name).collect();
    assert!(
        names.iter().any(|n| n == "cap_start") && names.iter().any(|n| n == "cap_end"),
        "{names:?}"
    );
    assert_eq!(
        d.faces("shaft")
            .iter()
            .filter(|f| f.surface == "cylinder")
            .count(),
        3
    );
    // A through bore along the axis from a sketch square to it (the yz plane, circle round 0,0).
    d.run(K::SketchCreate {
        sketch: "bore".into(),
        plane: PlaneSpec::Yz { offset: 15.0 },
    });
    add(&mut d, "bore", "c0", E::Point { x: 0.0, y: 0.0 });
    add(
        &mut d,
        "bore",
        "k",
        E::Circle {
            center: "c0".into(),
            radius: 1.0,
        },
    );
    let bp = d.profile("bore");
    d.run(K::FeatureCut {
        feature: "drill".into(),
        target: "shaft".into(),
        sketch: "bore".into(),
        profiles: vec![bp],
        depth: None,
        reverse: false,
        extent: Extent {
            mode: EndMode::ThroughBoth,
            ..Extent::default()
        },
        pattern: None,
    });
    assert!(
        near(d.summary("drill").volume, want - PI * 30.0),
        "{}",
        d.summary("drill").volume
    );
    // The stored recipe is the intent: sketch, profile, axis.
    let json = serde_json::to_string(&d.a.state().feature("shaft").unwrap().spec).unwrap();
    assert!(
        json.contains("\"axis\":\"x\"") && json.contains("revolve"),
        "{json}"
    );
}

#[test]
fn an_axis_can_be_a_centre_line_anywhere_and_the_part_follows_it() {
    let mut d = Doc::new();
    sketch_at(&mut d, "s", 0.0);
    // Axis: a line along y at x = 10, drawn as a construction line; the profile is a tube round it.
    add(&mut d, "s", "a0", E::Point { x: 10.0, y: 0.0 });
    add(&mut d, "s", "a1", E::Point { x: 10.0, y: 1.0 });
    add(
        &mut d,
        "s",
        "axis",
        E::Line {
            a: "a0".into(),
            b: "a1".into(),
        },
    );
    d.run(K::SketchSetConstruction {
        sketch: "s".into(),
        entity_id: "axis".into(),
        construction: true,
    });
    polygon(
        &mut d,
        "s",
        &[(12.0, 0.0), (15.0, 0.0), (15.0, 20.0), (12.0, 20.0)],
    );
    let p = d.profile("s");
    d.run(revolve("tube", "s", &p, "axis"));
    let s = d.summary("tube");
    assert!(near(s.volume, PI * (25.0 - 4.0) * 20.0), "{}", s.volume);
    // Round x = 10: from 5 to 15 across, 0 to 20 along y.
    assert!(
        near(s.bounds_min[0], 5.0) && near(s.bounds_max[0], 15.0),
        "{s:?}"
    );
    assert!(
        near(s.bounds_min[1], 0.0) && near(s.bounds_max[1], 20.0),
        "{s:?}"
    );
    // Undo, redo and replay.
    let h = d.a.state().hash();
    d.run(K::HistoryUndo { revision: None });
    assert!(d.a.state().feature("tube").is_none());
    d.run(K::HistoryRedo { revision: None });
    assert_eq!(d.a.state().hash(), h);
    let replayed = Authority::replay(&d.a.log_jsonl(), Policy::default()).expect("replays");
    assert_eq!(replayed.state().hash(), h);
}

#[test]
fn what_cannot_be_revolved_is_refused_and_a_revolve_has_no_depth() {
    let mut d = Doc::new();
    sketch_at(&mut d, "cone", 0.0);
    polygon(
        &mut d,
        "cone",
        &[(0.0, 0.0), (10.0, 0.0), (10.0, 3.0), (0.0, 5.0)],
    );
    let p = d.profile("cone");
    let e = d.try_run(revolve("c", "cone", &p, "x")).unwrap_err();
    assert_eq!(e.code, DiagCode::Unsupported, "{e:?}");
    assert!(e.message.contains("cone"), "{}", e.message);
    // An axis that is not there.
    sketch_at(&mut d, "ok", 0.0);
    polygon(
        &mut d,
        "ok",
        &[(0.0, 1.0), (10.0, 1.0), (10.0, 3.0), (0.0, 3.0)],
    );
    let p = d.profile("ok");
    let e = d.try_run(revolve("c", "ok", &p, "nowhere")).unwrap_err();
    assert_eq!(e.code, DiagCode::NotFound, "{e:?}");
    // A profile across the axis.
    sketch_at(&mut d, "x", 0.0);
    polygon(
        &mut d,
        "x",
        &[(0.0, -2.0), (10.0, -2.0), (10.0, 3.0), (0.0, 3.0)],
    );
    let p = d.profile("x");
    let e = d.try_run(revolve("c", "x", &p, "x")).unwrap_err();
    assert_eq!(e.code, DiagCode::FeatureFailed, "{e:?}");
    assert!(e.message.contains("crosses"), "{}", e.message);
    assert!(d.a.state().feature("c").is_none());
    // set_depth does not apply.
    let p = d.profile("ok");
    d.run(revolve("good", "ok", &p, "x"));
    let e = d
        .try_run(K::FeatureSetDepth {
            feature: "good".into(),
            depth: 5.0,
            expected: dc_doc::Expected::Any,
        })
        .unwrap_err();
    assert_eq!(e.code, DiagCode::WrongKind, "{e:?}");
    // Editing the sketch rebuilds the revolve (a longer tube); breaking it is refused.
    d.run(K::SketchDrag {
        sketch: "ok".into(),
        points: vec![
            dc_sketch::DragTarget {
                point: "p1".into(),
                x: 20.0,
                y: 1.0,
            },
            dc_sketch::DragTarget {
                point: "p2".into(),
                x: 20.0,
                y: 3.0,
            },
        ],
        radius: None,
    });
    assert!(
        near(d.summary("good").volume, PI * (9.0 - 1.0) * 20.0),
        "{}",
        d.summary("good").volume
    );
}

#[test]
fn a_partial_revolve_is_exact_follows_its_sketch_and_survives_undo_and_replay() {
    let mut d = Doc::new();
    sketch_at(&mut d, "s", 0.0);
    // A 5..10 by 0..20 rectangle turned a quarter about the sketch's y axis.
    polygon(
        &mut d,
        "s",
        &[(5.0, 0.0), (10.0, 0.0), (10.0, 20.0), (5.0, 20.0)],
    );
    d.run(revolve_by("q", "s", "profile.e0", "y", 90.0));
    let vol = |d: &Doc| d.summary("q").volume;
    assert!(
        near(vol(&d), std::f64::consts::PI / 4.0 * 75.0 * 20.0),
        "{}",
        vol(&d)
    );
    let hash = d.a.state().hash();
    // Dragging the outer edge outwards rebuilds the part.
    d.run(K::SketchDrag {
        sketch: "s".into(),
        points: vec![
            dc_sketch::DragTarget {
                point: "p1".into(),
                x: 14.0,
                y: 0.0,
            },
            dc_sketch::DragTarget {
                point: "p2".into(),
                x: 14.0,
                y: 20.0,
            },
        ],
        radius: None,
    });
    assert!(
        near(vol(&d), std::f64::consts::PI / 4.0 * (196.0 - 25.0) * 20.0),
        "{}",
        vol(&d)
    );
    d.run(K::HistoryUndo { revision: None });
    assert_eq!(d.a.state().hash(), hash);
    d.run(K::HistoryRedo { revision: None });
    let replayed = Authority::replay(&d.a.log_jsonl(), Policy::default()).expect("replays");
    assert_eq!(replayed.state().hash(), d.a.state().hash());
    // A bad angle and a slanted edge are refused with a reason.
    let e = d
        .try_run(revolve_by("bad", "s", "profile.e0", "y", 400.0))
        .unwrap_err();
    assert!(e.message.contains("less than 360"), "{}", e.message);
    sketch_at(&mut d, "cone", 0.0);
    polygon(&mut d, "cone", &[(5.0, 0.0), (10.0, 0.0), (8.0, 10.0)]);
    let p = d.profile("cone");
    let e = d
        .try_run(revolve_by("c", "cone", &p, "y", 90.0))
        .unwrap_err();
    assert_eq!(e.code, DiagCode::Unsupported, "{e:?}");
}
