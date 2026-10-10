//! Fillets and chamfers of edges parallel to the part's axis, through the command layer: exact
//! volumes, rebuilding when the sketch changes, undo and replay, and refusals with reasons.

use core::f64::consts::PI;
use dc_doc::{
    Actor, Authority, Command, CommandKind as K, DiagCode, Extent, Outcome, Policy, Query,
    QueryResult,
};
use dc_math::det;
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
    (a - b).abs() < 1e-6
}

fn fillet(f: &str, target: &str, edges: &[[&str; 2]], radius: f64) -> K {
    K::FeatureFillet {
        feature: f.into(),
        target: target.into(),
        edges: edges
            .iter()
            .map(|e| [e[0].to_owned(), e[1].to_owned()])
            .collect(),
        radius,
    }
}

fn chamfer(f: &str, target: &str, edges: &[[&str; 2]], distance: f64) -> K {
    K::FeatureChamfer {
        feature: f.into(),
        target: target.into(),
        edges: edges
            .iter()
            .map(|e| [e[0].to_owned(), e[1].to_owned()])
            .collect(),
        distance,
    }
}

/// What a right-angle corner of size `r` takes off.
fn fillet_area(r: f64) -> f64 {
    r * r * (1.0 - PI / 4.0)
}

#[test]
fn rounding_edges_through_commands_is_exact_and_names_its_faces() {
    let mut d = Doc::new();
    plate(&mut d);
    // The plate's corners: l1 (bottom), l2 (right), l3 (top), l4 (left).
    d.run(fillet(
        "round",
        "plate",
        &[["side:l1", "side:l2"], ["side:l3", "side:l4"]],
        3.0,
    ));
    let s = d.summary("round");
    assert!(
        near(s.volume, PLATE - 2.0 * fillet_area(3.0) * 10.0),
        "{}",
        s.volume
    );
    let names: Vec<_> = d.faces("round").into_iter().map(|f| f.name).collect();
    assert!(
        names.iter().any(|n| n.contains("fillet:round:0")),
        "{names:?}"
    );
    assert!(
        names.iter().any(|n| n.contains("fillet:round:1")),
        "{names:?}"
    );
    let cyl = d
        .faces("round")
        .into_iter()
        .filter(|f| f.surface == "cylinder")
        .count();
    assert_eq!(cyl, 2);
    // A chamfer of the other two corners, on top of that: features chain.
    d.run(chamfer(
        "bevel",
        "round",
        &[["side:l2", "side:l3"], ["side:l4", "side:l1"]],
        2.0,
    ));
    let want = PLATE - 2.0 * fillet_area(3.0) * 10.0 - 2.0 * (2.0 * 2.0 / 2.0) * 10.0;
    assert!(
        near(d.summary("bevel").volume, want),
        "{}",
        d.summary("bevel").volume
    );
    // Then a hole is cut through the rounded part (it targets the chamfer feature).
    sketch_at(&mut d, "h", 0.0);
    disc(&mut d, "h", 0.0, 0.0, 3.0);
    let hp = d.profile("h");
    d.run(K::FeatureCut {
        feature: "hole".into(),
        target: "bevel".into(),
        sketch: "h".into(),
        profiles: vec![hp],
        depth: None,
        reverse: false,
        extent: Extent::default(),
        pattern: None,
    });
    assert!(near(d.summary("hole").volume, want - PI * 9.0 * 10.0));
    // A feature that others are built on cannot be deleted.
    let e = d
        .try_run(K::FeatureDelete {
            feature: "round".into(),
        })
        .unwrap_err();
    assert_eq!(e.code, DiagCode::InUse, "{e:?}");
}

#[test]
fn the_radius_is_changed_with_set_depth_and_the_edit_ripples_through() {
    let mut d = Doc::new();
    plate(&mut d);
    d.run(fillet("round", "plate", &[["side:l1", "side:l2"]], 2.0));
    sketch_at(&mut d, "h", 0.0);
    disc(&mut d, "h", 0.0, 0.0, 3.0);
    let hp = d.profile("h");
    d.run(K::FeatureCut {
        feature: "hole".into(),
        target: "round".into(),
        sketch: "h".into(),
        profiles: vec![hp],
        depth: None,
        reverse: false,
        extent: Extent::default(),
        pattern: None,
    });
    d.run(K::FeatureSetDepth {
        feature: "round".into(),
        depth: 5.0,
        expected: dc_doc::Expected::Value(2.0),
    });
    assert!(near(
        d.summary("round").volume,
        PLATE - fillet_area(5.0) * 10.0
    ));
    // The hole built on it was rebuilt too.
    assert!(near(
        d.summary("hole").volume,
        PLATE - fillet_area(5.0) * 10.0 - PI * 9.0 * 10.0
    ));
    // Stale expectations are caught as for any other edit.
    let e = d
        .try_run(K::FeatureSetDepth {
            feature: "round".into(),
            depth: 6.0,
            expected: dc_doc::Expected::Value(2.0),
        })
        .unwrap_err();
    assert_eq!(e.code, DiagCode::StaleExpectation, "{e:?}");
    // A radius the edge cannot take is refused and nothing changes.
    let before = d.a.state().hash();
    let e = d
        .try_run(K::FeatureSetDepth {
            feature: "round".into(),
            depth: 25.0,
            expected: dc_doc::Expected::Any,
        })
        .unwrap_err();
    assert_eq!(e.code, DiagCode::FeatureFailed, "{e:?}");
    assert!(e.message.contains("too big"), "{}", e.message);
    assert_eq!(d.a.state().hash(), before);
}

#[test]
fn rounding_follows_the_sketch_and_survives_undo_redo_and_replay() {
    let mut d = Doc::new();
    plate(&mut d);
    d.run(fillet("round", "plate", &[["side:l1", "side:l2"]], 3.0));
    let hash_rounded = d.a.state().hash();
    // Move the plate's corner point b: the fillet is rebuilt around the new corner.
    d.run(K::SketchDrag {
        sketch: "base".into(),
        points: vec![dc_sketch::DragTarget {
            point: "b".into(),
            x: 18.0,
            y: -10.0,
        }],
        radius: None,
    });
    // The plate is now a trapezoid (bottom 38 wide, top 40, 20 high); its corner at b is no
    // longer a right angle, and a fillet of radius r there takes r^2 (cot(t/2) - (pi - t)/2)
    // off, with t the angle between the two edges.
    let s = d.summary("round");
    let u1 = (-1.0f64, 0.0f64);
    let (dx, dy) = (2.0f64, 20.0f64);
    let len = det::hypot(dx, dy);
    let cos_t = (u1.0 * dx + u1.1 * dy) / len;
    let t = det::acos(cos_t);
    let removed = 9.0 * (1.0 / det::tan(t / 2.0) - (PI - t) / 2.0);
    let want = (780.0 - removed) * 10.0;
    assert!(near(s.volume, want), "{} against {want}", s.volume);
    // Undo the drag and the fillet, redo both: the same hashes come back.
    d.run(K::HistoryUndo { revision: None });
    assert_eq!(d.a.state().hash(), hash_rounded);
    d.run(K::HistoryUndo { revision: None });
    assert!(d.a.state().feature("round").is_none());
    d.run(K::HistoryRedo { revision: None });
    assert_eq!(d.a.state().hash(), hash_rounded);
    let replayed = Authority::replay(&d.a.log_jsonl(), Policy::default()).expect("replays");
    assert_eq!(replayed.state().hash(), d.a.state().hash());
}

#[test]
fn edges_that_cannot_be_rounded_are_refused_and_say_why() {
    let mut d = Doc::new();
    plate(&mut d);
    let try_edge = |d: &mut Doc, a: &str, b: &str, r: f64| {
        d.try_run(fillet("x", "plate", &[[a, b]], r)).unwrap_err()
    };
    // Faces that do not exist; faces that do not meet (opposite sides).
    let e = try_edge(&mut d, "side:l1", "side:nope", 1.0);
    assert_eq!(e.code, DiagCode::NotFound, "{e:?}");
    assert!(
        e.message.contains("side:l1"),
        "lists what exists: {}",
        e.message
    );
    let e = try_edge(&mut d, "side:l1", "side:l3", 1.0);
    assert_eq!(e.code, DiagCode::InvalidValue, "{e:?}");
    // A radius that does not fit the edge's neighbours.
    let e = try_edge(&mut d, "side:l1", "side:l2", 100.0);
    assert_eq!(e.code, DiagCode::FeatureFailed, "{e:?}");
    // Bad sizes and an empty list.
    for bad in [0.0, -1.0, f64::NAN] {
        let e = try_edge(&mut d, "side:l1", "side:l2", bad);
        assert_eq!(e.code, DiagCode::InvalidValue, "{bad}: {e:?}");
    }
    let e = d.try_run(fillet("x", "plate", &[], 1.0)).unwrap_err();
    assert_eq!(e.code, DiagCode::InvalidValue, "{e:?}");
    // A target that is not there.
    let e = d
        .try_run(fillet("x", "ghost", &[["side:l1", "side:l2"]], 1.0))
        .unwrap_err();
    assert_eq!(e.code, DiagCode::NotFound, "{e:?}");
    assert!(d.a.state().feature("x").is_none());
}

#[test]
fn the_top_edge_of_a_plate_is_rounded_on_the_b_rep_and_the_part_goes_on() {
    let mut d = Doc::new();
    plate(&mut d);
    // An edge between the cap and a side: across the part's axis, so the B-rep rounds it.
    let before = d.summary("plate").volume;
    d.run(fillet("top", "plate", &[["side:l1", "cap_end"]], 1.0));
    let s = d.summary("top");
    // The plate is 40 x 20 x 10; side:l1 is 40 long.
    assert!(
        near(s.volume, before - (1.0 - PI / 4.0) * 40.0),
        "{}",
        s.volume
    );
    // The rounded part has no layered model, so a later fillet of another top edge also works.
    d.run(fillet("top2", "top", &[["side:l3", "cap_end"]], 1.0));
    assert!(d.summary("top2").volume < s.volume);
    // And the new face has the feature's name.
    let faces = match d
        .a
        .query(&Query::FeatureFaces {
            feature: "top2".into(),
            after: None,
            limit: Some(100),
        })
        .unwrap()
    {
        QueryResult::FeatureFaces { items, .. } => items,
        other => panic!("{other:?}"),
    };
    let names: Vec<String> = faces.iter().map(|f| f.name.clone()).collect();
    assert!(
        names.contains(&"fillet:top".to_owned()) && names.contains(&"fillet:top2".to_owned()),
        "{names:?}"
    );
}
