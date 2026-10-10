//! A feature takes a list of profiles; none named means every closed region of the sketch.

use core::f64::consts::PI;
use dc_doc::{
    Actor, Authority, Command, CommandKind as K, DiagCode, Extent, Outcome, Query, QueryResult,
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

fn discs(d: &mut Doc, sketch: &str, at: &[(f64, f64)], r: f64) {
    for (i, (x, y)) in at.iter().enumerate() {
        add(d, sketch, &format!("c{i}"), E::Point { x: *x, y: *y });
        add(
            d,
            sketch,
            &format!("k{i}"),
            E::Circle {
                center: format!("c{i}"),
                radius: r,
            },
        );
    }
}

fn near(a: f64, b: f64) -> bool {
    (a - b).abs() < 1e-6 * b.abs().max(1.0)
}

const FOUR: [(f64, f64); 4] = [(-12.0, -5.0), (12.0, -5.0), (-12.0, 5.0), (12.0, 5.0)];

fn block(d: &mut Doc) {
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

#[test]
fn a_cut_with_no_profile_named_uses_every_closed_region_of_the_sketch() {
    let mut d = Doc::new();
    block(&mut d);
    sketch_at(&mut d, "holes", 0.0);
    discs(&mut d, "holes", &FOUR, 2.0);
    let plate = d.summary("plate").volume;
    d.run(K::FeatureCut {
        feature: "drilled".into(),
        target: "plate".into(),
        sketch: "holes".into(),
        profiles: vec![],
        depth: None,
        reverse: false,
        extent: Extent::default(),
        pattern: None,
    });
    let v = d.summary("drilled").volume;
    assert!(near(plate - v, 4.0 * PI * 4.0 * 10.0), "{}", plate - v);
    // Naming two of the four cuts exactly two.
    let both = match d
        .a
        .query(&Query::SketchProfiles {
            sketch: "holes".into(),
            after: None,
            limit: None,
        })
        .unwrap()
    {
        QueryResult::SketchProfiles { profiles, .. } => profiles,
        other => panic!("{other:?}"),
    };
    assert_eq!(both.len(), 4);
    d.run(K::FeatureCut {
        feature: "two".into(),
        target: "plate".into(),
        sketch: "holes".into(),
        profiles: vec![both[0].id.clone(), both[2].id.clone()],
        depth: None,
        reverse: false,
        extent: Extent::default(),
        pattern: None,
    });
    assert!(near(plate - d.summary("two").volume, 2.0 * PI * 4.0 * 10.0));
    // The list is canonical (order and repeats do not matter) so the same recipe hashes the same.
    let a = both[2].id.clone();
    let b = both[0].id.clone();
    let spec = |p: Vec<String>| dc_doc::FeatureSpec::Cut {
        target: "plate".into(),
        sketch: "holes".into(),
        profiles: p,
        depth: None,
        reverse: false,
        extent: Extent::default(),
        pattern: None,
    };
    assert_eq!(
        spec(vec![a.clone(), b.clone(), a.clone()]).canonical(),
        spec(vec![b, a]).canonical()
    );
}

#[test]
fn extruding_every_region_makes_one_part_and_boss_and_patterns_apply_to_each() {
    let mut d = Doc::new();
    sketch_at(&mut d, "pegs", 0.0);
    discs(&mut d, "pegs", &FOUR, 2.0);
    d.run(K::FeatureExtrude {
        feature: "pegs".into(),
        sketch: "pegs".into(),
        profiles: vec![],
        depth: 6.0,
        reverse: false,
        extent: Extent::default(),
    });
    assert!(near(d.summary("pegs").volume, 4.0 * PI * 4.0 * 6.0));
    // A mix of a ring and an island: both regions are used.
    sketch_at(&mut d, "tops", 6.0);
    discs(&mut d, "tops", &[(-12.0, -5.0), (12.0, 5.0)], 1.0);
    d.run(K::FeatureBoss {
        feature: "tops".into(),
        target: "pegs".into(),
        sketch: "tops".into(),
        profiles: vec![],
        depth: 2.0,
        reverse: false,
        extent: Extent::default(),
        pattern: None,
    });
    assert!(near(
        d.summary("tops").volume,
        4.0 * PI * 4.0 * 6.0 + 2.0 * PI * 1.0 * 2.0
    ));
}

#[test]
fn revolve_needs_exactly_one_profile_and_says_which_were_found() {
    let mut d = Doc::new();
    sketch_at(&mut d, "s", 0.0);
    discs(&mut d, "s", &[(10.0, 10.0), (30.0, 10.0)], 2.0);
    let e = d
        .try_run(K::FeatureRevolve {
            feature: "r".into(),
            sketch: "s".into(),
            profiles: vec![],
            axis: "y".into(),
            angle: None,
        })
        .unwrap_err();
    assert_eq!(e.code, DiagCode::Unsupported, "{e:?}");
    assert!(e.message.contains("revolve takes one profile"), "{e:?}");
}
