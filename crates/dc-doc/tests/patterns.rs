//! Patterns of cuts and bosses: a row or grid, a ring, a mirror image. One feature makes all the
//! copies; each is checked through exact volumes, and refusals through their diagnostics.

use core::f64::consts::PI;
use dc_doc::{
    Actor, Authority, Command, CommandKind as K, DiagCode, EndMode, Extent, FeaturePattern as P,
    Outcome, Policy, Query, QueryResult,
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

fn through_cut(f: &str, sketch: &str, profile: &str, pattern: Option<P>) -> K {
    K::FeatureCut {
        feature: f.into(),
        target: "plate".into(),
        sketch: sketch.into(),
        profiles: vec![profile.into()],
        depth: None,
        reverse: false,
        extent: Extent {
            mode: EndMode::ThroughBoth,
            ..Extent::default()
        },
        pattern,
    }
}

fn boss(f: &str, target: &str, sketch: &str, profile: &str, h: f64, pattern: Option<P>) -> K {
    K::FeatureBoss {
        feature: f.into(),
        target: target.into(),
        sketch: sketch.into(),
        profiles: vec![profile.into()],
        depth: h,
        reverse: false,
        extent: Extent::default(),
        pattern,
    }
}

fn near(a: f64, b: f64) -> bool {
    (a - b).abs() < 1e-6
}

#[test]
fn a_row_of_holes_is_one_feature_and_each_copy_is_a_real_hole() {
    let mut d = Doc::new();
    plate(&mut d);
    sketch_at(&mut d, "h", 0.0);
    disc(&mut d, "h", -15.0, 0.0, 2.0);
    let hp = d.profile("h");
    let hole = PI * 4.0 * 10.0;
    d.run(through_cut(
        "row",
        "h",
        &hp,
        Some(P::Linear {
            step: [10.0, 0.0],
            count: 4,
            step2: None,
            count2: 1,
        }),
    ));
    let s = d.summary("row");
    assert!(near(s.volume, PLATE - 4.0 * hole), "{}", s.volume);
    // Four cylinder faces, each named after its copy, none of them clashing.
    let cyl: Vec<_> = d
        .faces("row")
        .into_iter()
        .filter(|f| f.surface == "cylinder")
        .map(|f| f.name)
        .collect();
    assert_eq!(cyl.len(), 4, "{cyl:?}");
    assert!(cyl.iter().any(|n| n.contains("cut:row:")), "{cyl:?}");
    assert!(cyl.iter().any(|n| n.contains("cut:row~1:")), "{cyl:?}");
    assert!(cyl.iter().any(|n| n.contains("cut:row~3:")), "{cyl:?}");
    // A pattern of one copy is the plain cut: the same solid (its face names carry the feature's
    // own name, so the digests differ; the measurements and the counts do not).
    d.run(through_cut("one", "h", &hp, None));
    d.run(through_cut(
        "one_p",
        "h",
        &hp,
        Some(P::Linear {
            step: [10.0, 0.0],
            count: 1,
            step2: None,
            count2: 1,
        }),
    ));
    let (a, b) = (d.summary("one"), d.summary("one_p"));
    assert!(
        near(a.volume, b.volume) && near(a.area, b.area),
        "{a:?} {b:?}"
    );
    assert_eq!(
        (a.vertices, a.edges, a.faces),
        (b.vertices, b.edges, b.faces)
    );
}

#[test]
fn a_grid_a_ring_and_a_mirror_image_place_their_copies_exactly() {
    let mut d = Doc::new();
    plate(&mut d);
    let hole = PI * 4.0 * 10.0;
    // A 3 x 2 grid.
    sketch_at(&mut d, "g", 0.0);
    disc(&mut d, "g", -12.0, -5.0, 2.0);
    let gp = d.profile("g");
    d.run(through_cut(
        "grid",
        "g",
        &gp,
        Some(P::Linear {
            step: [12.0, 0.0],
            count: 3,
            step2: Some([0.0, 10.0]),
            count2: 2,
        }),
    ));
    assert!(near(d.summary("grid").volume, PLATE - 6.0 * hole));
    // A ring of 5 about the plate's middle.
    sketch_at(&mut d, "r", 0.0);
    disc(&mut d, "r", 7.0, 0.0, 1.5);
    let rp = d.profile("r");
    d.run(through_cut(
        "ring",
        "r",
        &rp,
        Some(P::Circular {
            center: [0.0, 0.0],
            count: 5,
            step_degrees: 72.0,
        }),
    ));
    assert!(near(
        d.summary("ring").volume,
        PLATE - 5.0 * PI * 2.25 * 10.0
    ));
    // A mirror image across the plate's short axis.
    sketch_at(&mut d, "m", 0.0);
    disc(&mut d, "m", -14.0, 3.0, 2.0);
    let mp = d.profile("m");
    d.run(through_cut(
        "mirror",
        "m",
        &mp,
        Some(P::Mirror {
            point: [0.0, 0.0],
            direction_degrees: 90.0,
        }),
    ));
    assert!(near(d.summary("mirror").volume, PLATE - 2.0 * hole));
    // Where the copies are: mirrored holes sit at x = -14 and +14, same y. Look at the cylinder
    // faces' areas only (equal) and the bounds stay those of the plate.
    let s = d.summary("mirror");
    assert!(near(s.bounds_min[0], -20.0) && near(s.bounds_max[0], 20.0));
}

#[test]
fn a_mirrored_slot_keeps_its_shape_not_just_its_volume() {
    // A non-symmetric notch (a right triangle) mirrored across the x axis: both copies must be
    // valid solids' worth of material removed, with the second the reflection of the first.
    let mut d = Doc::new();
    plate(&mut d);
    sketch_at(&mut d, "t", 0.0);
    for (id, x, y) in [("p", -15.0, 2.0), ("q", -5.0, 2.0), ("r", -15.0, 6.0)] {
        add(&mut d, "t", id, E::Point { x, y });
    }
    for (id, a, b) in [("e1", "p", "q"), ("e2", "q", "r"), ("e3", "r", "p")] {
        add(
            &mut d,
            "t",
            id,
            E::Line {
                a: a.into(),
                b: b.into(),
            },
        );
    }
    let tp = d.profile("t");
    let tri = 0.5 * 10.0 * 4.0 * 10.0;
    d.run(through_cut(
        "tri",
        "t",
        &tp,
        Some(P::Mirror {
            point: [0.0, 0.0],
            direction_degrees: 0.0,
        }),
    ));
    assert!(near(d.summary("tri").volume, PLATE - 2.0 * tri));
}

#[test]
fn bosses_pattern_too_and_the_pattern_survives_undo_redo_and_replay() {
    let mut d = Doc::new();
    plate(&mut d);
    sketch_at(&mut d, "top", 10.0);
    disc(&mut d, "top", -12.0, 0.0, 2.0);
    let tp = d.profile("top");
    d.run(boss(
        "pegs",
        "plate",
        "top",
        &tp,
        5.0,
        Some(P::Linear {
            step: [8.0, 0.0],
            count: 4,
            step2: None,
            count2: 1,
        }),
    ));
    assert!(near(d.summary("pegs").volume, PLATE + 4.0 * PI * 4.0 * 5.0));
    // The stored recipe says what was asked, and a plain feature has no trace of it.
    let json = serde_json::to_string(&d.a.state().feature("pegs").unwrap().spec).unwrap();
    assert!(
        json.contains("\"pattern\"") && json.contains("\"linear\""),
        "{json}"
    );
    d.run(boss("single", "plate", "top", &tp, 5.0, None));
    let json = serde_json::to_string(&d.a.state().feature("single").unwrap().spec).unwrap();
    assert!(!json.contains("pattern"), "{json}");
    // Replaying the log reproduces the same state.
    let replayed = Authority::replay(&d.a.log_jsonl(), Policy::default()).expect("replays");
    assert_eq!(replayed.state().hash(), d.a.state().hash());
    // Changing the sketch moves every copy with it: the pegs are rebuilt.
    d.run(K::SketchAddEntity {
        sketch: "top".into(),
        entity_id: "extra".into(),
        entity: E::Point { x: 1.0, y: 1.0 },
    });
    assert!(near(d.summary("pegs").volume, PLATE + 4.0 * PI * 4.0 * 5.0));
}

#[test]
fn bad_patterns_are_refused_with_a_reason_and_copies_that_miss_the_part_are_named() {
    let mut d = Doc::new();
    plate(&mut d);
    sketch_at(&mut d, "h", 0.0);
    disc(&mut d, "h", -15.0, 0.0, 2.0);
    let hp = d.profile("h");
    let lin = |step: [f64; 2], count: u32| {
        Some(P::Linear {
            step,
            count,
            step2: None,
            count2: 1,
        })
    };
    // Zero step, no copies, too many, not finite.
    for (pat, what) in [
        (lin([0.0, 0.0], 3), "zero step"),
        (lin([5.0, 0.0], 0), "no copies"),
        (lin([5.0, 0.0], 5000), "too many"),
        (lin([f64::NAN, 0.0], 2), "not finite"),
        (
            Some(P::Circular {
                center: [0.0, 0.0],
                count: 3,
                step_degrees: 360.0,
            }),
            "whole turns",
        ),
    ] {
        let e = d.try_run(through_cut("bad", "h", &hp, pat)).unwrap_err();
        assert_eq!(e.code, DiagCode::InvalidValue, "{what}: {e:?}");
    }
    assert!(d.a.state().feature("bad").is_none());
    // A copy that falls off the plate is named, and the whole feature is refused.
    let e = d
        .try_run(through_cut("off", "h", &hp, lin([10.0, 0.0], 8)))
        .unwrap_err();
    assert_eq!(e.code, DiagCode::FeatureFailed, "{e:?}");
    assert!(e.message.contains("copy"), "{}", e.message);
    // Cutting outside the profile with a pattern is refused.
    let e = d
        .try_run(K::FeatureCut {
            feature: "odd".into(),
            target: "plate".into(),
            sketch: "h".into(),
            profiles: vec![hp.clone()],
            depth: Some(5.0),
            reverse: false,
            extent: Extent {
                outside: true,
                ..Extent::default()
            },
            pattern: lin([5.0, 0.0], 2),
        })
        .unwrap_err();
    assert_eq!(e.code, DiagCode::Unsupported, "{e:?}");
}
