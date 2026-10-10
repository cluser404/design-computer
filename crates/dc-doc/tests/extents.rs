//! End conditions beyond a plain depth: mid plane, a second direction, through all in both
//! directions, and cutting what is outside the profile.

use core::f64::consts::PI;
use dc_doc::{
    Actor, Authority, Command, CommandKind as K, DiagCode, EndMode, Extent, Outcome, Query,
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
        let c = Command {
            id: format!("c{}", self.n),
            actor: Actor::Human("ann".into()),
            base_revision: self.a.revision(),
            kind,
        };
        self.a.submit(c)
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

/// A 40 x 20 rectangle centred on the origin.
fn rectangle(d: &mut Doc, sketch: &str) {
    for (id, x, y) in [
        ("a", -20.0, -10.0),
        ("b", 20.0, -10.0),
        ("c", 20.0, 10.0),
        ("e", -20.0, 10.0),
    ] {
        add(d, sketch, id, E::Point { x, y });
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

fn circle(d: &mut Doc, sketch: &str, r: f64) {
    add(d, sketch, "cc", E::Point { x: 0.0, y: 0.0 });
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

fn extrude(f: &str, sketch: &str, profile: &str, depth: f64, extent: Extent) -> K {
    K::FeatureExtrude {
        feature: f.into(),
        sketch: sketch.into(),
        profiles: vec![profile.into()],
        depth,
        reverse: false,
        extent,
    }
}

fn cut(
    f: &str,
    target: &str,
    sketch: &str,
    profile: &str,
    depth: Option<f64>,
    extent: Extent,
) -> K {
    K::FeatureCut {
        feature: f.into(),
        target: target.into(),
        sketch: sketch.into(),
        profiles: vec![profile.into()],
        depth,
        reverse: false,
        extent,
        pattern: None,
    }
}

fn mode(m: EndMode) -> Extent {
    Extent {
        mode: m,
        ..Extent::default()
    }
}

#[test]
fn mid_plane_extrusion_is_symmetric_and_a_second_direction_goes_the_other_way() {
    let mut d = Doc::new();
    sketch_at(&mut d, "s", 0.0);
    rectangle(&mut d, "s");
    let p = d.profile("s");
    d.run(extrude("mid", "s", &p, 20.0, mode(EndMode::MidPlane)));
    let s = d.summary("mid");
    assert!(
        (s.bounds_min[2] + 10.0).abs() < 1e-9 && (s.bounds_max[2] - 10.0).abs() < 1e-9,
        "{s:?}"
    );
    assert!((s.volume - 40.0 * 20.0 * 20.0).abs() < 1e-6);
    // 10 up and 5 down.
    d.run(extrude(
        "two",
        "s",
        &p,
        10.0,
        Extent {
            depth2: Some(5.0),
            ..Extent::default()
        },
    ));
    let s = d.summary("two");
    assert!(
        (s.bounds_min[2] + 5.0).abs() < 1e-9 && (s.bounds_max[2] - 10.0).abs() < 1e-9,
        "{s:?}"
    );
    assert!((s.volume - 40.0 * 20.0 * 15.0).abs() < 1e-6);
    // The plain extrusion is untouched: no extent in its stored form, same bounds as before.
    d.run(extrude("plain", "s", &p, 10.0, Extent::default()));
    let json = serde_json::to_string(&d.a.state().feature("plain").unwrap().spec).unwrap();
    assert!(!json.contains("extent"), "{json}");
}

#[test]
fn cuts_can_go_through_in_both_directions_from_the_middle_and_outside() {
    let mut d = Doc::new();
    sketch_at(&mut d, "base", 0.0);
    rectangle(&mut d, "base");
    let p = d.profile("base");
    d.run(extrude("plate", "base", &p, 10.0, Extent::default()));
    let plate = d.summary("plate").volume;
    // A hole sketched half way up the plate.
    sketch_at(&mut d, "hole", 5.0);
    circle(&mut d, "hole", 3.0);
    let hp = d.profile("hole");
    let disc = PI * 9.0;
    // Through all, one way: only the upper half goes.
    d.run(cut("one", "plate", "hole", &hp, None, Extent::default()));
    assert!((plate - d.summary("one").volume - disc * 5.0).abs() < 1e-6);
    // Through all both ways: the whole thickness.
    d.run(cut(
        "both",
        "plate",
        "hole",
        &hp,
        None,
        mode(EndMode::ThroughBoth),
    ));
    assert!((plate - d.summary("both").volume - disc * 10.0).abs() < 1e-6);
    // Mid plane, 4 deep: 2 each side of the sketch plane.
    d.run(cut(
        "mid",
        "plate",
        "hole",
        &hp,
        Some(4.0),
        mode(EndMode::MidPlane),
    ));
    assert!((plate - d.summary("mid").volume - disc * 4.0).abs() < 1e-6);
    // 1 up and 2 down (blind with a second direction).
    d.run(cut(
        "twoway",
        "plate",
        "hole",
        &hp,
        Some(1.0),
        Extent {
            depth2: Some(2.0),
            ..Extent::default()
        },
    ));
    assert!((plate - d.summary("twoway").volume - disc * 3.0).abs() < 1e-6);
    // Cut outside: only the disc is left (through the whole thickness).
    d.run(cut(
        "outside",
        "plate",
        "hole",
        &hp,
        None,
        Extent {
            mode: EndMode::ThroughBoth,
            outside: true,
            ..Extent::default()
        },
    ));
    assert!(
        (d.summary("outside").volume - disc * 10.0).abs() < 1e-6,
        "{:?}",
        d.summary("outside")
    );
}

#[test]
fn option_combinations_that_make_no_sense_are_refused_with_a_reason() {
    let mut d = Doc::new();
    sketch_at(&mut d, "s", 0.0);
    rectangle(&mut d, "s");
    let p = d.profile("s");
    for (extent, why) in [
        (mode(EndMode::ThroughBoth), "cuts"),
        (
            Extent {
                outside: true,
                ..Extent::default()
            },
            "cuts",
        ),
        (
            Extent {
                mode: EndMode::MidPlane,
                depth2: Some(3.0),
                ..Extent::default()
            },
            "second direction",
        ),
        (
            Extent {
                depth2: Some(-1.0),
                ..Extent::default()
            },
            "depth",
        ),
    ] {
        let e = d
            .try_run(extrude("bad", "s", &p, 10.0, extent))
            .expect_err("refused");
        assert!(
            e.message.contains(why),
            "expected '{why}' in: {}",
            e.message
        );
        assert!(d.a.state().feature("bad").is_none());
    }
    let _ = DiagCode::Unsupported;
}
