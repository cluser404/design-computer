//! Cut features: through-cuts of prismatic parts, built purely through commands.

use core::f64::consts::PI;
use dc_doc::{
    Actor, Authority, Command, CommandKind as K, DiagCode, Diagnostic, Expected, Outcome, Policy,
    Query, QueryResult, Revision,
};
use dc_kernel::TessParams;
use dc_sketch::{Constraint as C, Entity as E, PlaneSpec};

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
                limit: Some(1000),
            })
            .unwrap()
        {
            QueryResult::FeatureFaces { items, .. } => items,
            other => panic!("{other:?}"),
        }
    }
}

fn close(a: f64, b: f64) -> bool {
    (a - b).abs() <= 1e-7 * b.abs().max(1.0)
}

fn sketch_on(sk: &str, plane: PlaneSpec) -> K {
    K::SketchCreate {
        sketch: sk.into(),
        plane,
    }
}
fn point(sk: &str, id: &str, x: f64, y: f64) -> K {
    K::SketchAddEntity {
        sketch: sk.into(),
        entity_id: id.into(),
        entity: E::Point { x, y },
    }
}
fn line(sk: &str, id: &str, a: &str, b: &str) -> K {
    K::SketchAddEntity {
        sketch: sk.into(),
        entity_id: id.into(),
        entity: E::Line {
            a: a.into(),
            b: b.into(),
        },
    }
}
fn circle(sk: &str, id: &str, c: &str, r: f64) -> K {
    K::SketchAddEntity {
        sketch: sk.into(),
        entity_id: id.into(),
        entity: E::Circle {
            center: c.into(),
            radius: r,
        },
    }
}
fn k(sk: &str, id: &str, c: C) -> K {
    K::SketchAddConstraint {
        sketch: sk.into(),
        constraint_id: id.into(),
        constraint: c,
    }
}

/// A rectangle sketch `sk` on `plane` with ids `<sk>1..4`, lines `<sk>a..d`, and constraints
/// pinning it to exactly x0..x1 by y0..y1 (so later edits are parametric).
fn rect(d: &mut Doc, sk: &str, plane: PlaneSpec, x0: f64, y0: f64, x1: f64, y1: f64) {
    d.run(sketch_on(sk, plane));
    let c = [(x0, y0), (x1, y0), (x1, y1), (x0, y1)];
    for (i, (x, y)) in c.iter().enumerate() {
        d.run(point(sk, &format!("{sk}{}", i + 1), *x, *y));
    }
    for i in 0..4 {
        d.run(line(
            sk,
            &format!("{sk}{}", (b'a' + i as u8) as char),
            &format!("{sk}{}", i + 1),
            &format!("{sk}{}", (i + 1) % 4 + 1),
        ));
    }
    d.run(k(
        sk,
        "w",
        C::Length {
            line: format!("{sk}a"),
            value: x1 - x0,
        },
    ));
    d.run(k(
        sk,
        "h",
        C::Length {
            line: format!("{sk}b"),
            value: y1 - y0,
        },
    ));
    d.run(k(
        sk,
        "ha",
        C::Horizontal {
            line: format!("{sk}a"),
        },
    ));
    d.run(k(
        sk,
        "vb",
        C::Vertical {
            line: format!("{sk}b"),
        },
    ));
    d.run(k(
        sk,
        "hc",
        C::Horizontal {
            line: format!("{sk}c"),
        },
    ));
    d.run(k(
        sk,
        "vd",
        C::Vertical {
            line: format!("{sk}d"),
        },
    ));
    d.run(k(
        sk,
        "fix",
        C::Fixed {
            point: format!("{sk}1"),
            x: x0,
            y: y0,
        },
    ));
}
fn disc(d: &mut Doc, sk: &str, plane: PlaneSpec, cx: f64, cy: f64, r: f64) {
    d.run(sketch_on(sk, plane));
    d.run(point(sk, "m", cx, cy));
    d.run(circle(sk, "c", "m", r));
    d.run(k(
        sk,
        "fx",
        C::Fixed {
            point: "m".into(),
            x: cx,
            y: cy,
        },
    ));
    d.run(k(
        sk,
        "r",
        C::Radius {
            target: "c".into(),
            value: r,
        },
    ));
}
fn extrude(f: &str, sk: &str, prof: &str, depth: f64) -> K {
    K::FeatureExtrude {
        feature: f.into(),
        sketch: sk.into(),
        profiles: vec![prof.into()],
        depth,
        reverse: false,
        extent: Default::default(),
    }
}
fn cut(f: &str, target: &str, sk: &str, prof: &str, depth: Option<f64>, reverse: bool) -> K {
    K::FeatureCut {
        feature: f.into(),
        target: target.into(),
        sketch: sk.into(),
        profiles: vec![prof.into()],
        depth,
        reverse,
        extent: Default::default(),
        pattern: None,
    }
}

const XY: PlaneSpec = PlaneSpec::Xy { offset: 0.0 };

/// A 100 x 60 plate, 12 thick, as feature `plate`.
fn plate() -> Doc {
    let mut d = Doc::new();
    rect(&mut d, "base", XY, 0.0, 0.0, 100.0, 60.0);
    d.run(extrude("plate", "base", "profile.basea", 12.0));
    d
}

#[test]
fn a_through_hole_by_cut_matches_extruding_a_profile_with_a_hole() {
    // The independent construction: one sketch with the hole already in it.
    let mut ref_doc = Doc::new();
    rect(&mut ref_doc, "s", XY, 0.0, 0.0, 100.0, 60.0);
    ref_doc.run(point("s", "m", 50.0, 30.0));
    ref_doc.run(circle("s", "hole", "m", 8.0));
    ref_doc.run(extrude("direct", "s", "profile.sa", 12.0));
    let direct = ref_doc.summary("direct");

    // Cutting from the top face downward, and from the bottom face upward: same result.
    for (plane, reverse) in [(PlaneSpec::Xy { offset: 12.0 }, true), (XY, false)] {
        let mut d = plate();
        disc(&mut d, "tool", plane, 50.0, 30.0, 8.0);
        let out = d.run(cut("hole", "plate", "tool", "profile.c", None, reverse));
        assert!(out.changes.iter().any(|c| c.entity == "feature/hole"));
        let s = d.summary("hole");
        assert!(
            close(s.volume, direct.volume),
            "{} vs {}",
            s.volume,
            direct.volume
        );
        assert!(close(s.area, direct.area), "{} vs {}", s.area, direct.area);
        assert_eq!(
            (s.vertices, s.edges, s.faces),
            (direct.vertices, direct.edges, direct.faces)
        );
        assert!(close(s.volume, 12.0 * (6000.0 - 64.0 * PI)));
        // The hole's wall is named after the cut feature and faces inward.
        let wall = d
            .faces("hole")
            .into_iter()
            .find(|f| f.name == "side:cut:hole:c")
            .expect("named wall");
        assert_eq!(
            (wall.surface.as_str(), wall.radius, wall.convex),
            ("cylinder", Some(8.0), Some(false))
        );
    }
}

#[test]
fn cuts_are_parametric_in_every_input() {
    let mut d = plate();
    disc(&mut d, "tool", XY, 50.0, 30.0, 8.0);
    d.run(cut("hole", "plate", "tool", "profile.c", None, false));
    let vol = |d: &Doc| d.summary("hole").volume;
    assert!(close(vol(&d), 12.0 * (6000.0 - 64.0 * PI)));

    // 1. The tool: a bigger hole.
    d.run(K::SketchSetValue {
        sketch: "tool".into(),
        constraint_id: "r".into(),
        value: 12.0,
        expected: Expected::Value(8.0),
    });
    assert!(close(vol(&d), 12.0 * (6000.0 - 144.0 * PI)));
    // 2. The part's sketch: a wider plate.
    d.run(K::SketchSetValue {
        sketch: "base".into(),
        constraint_id: "w".into(),
        value: 140.0,
        expected: Expected::Any,
    });
    assert!(close(vol(&d), 12.0 * (140.0 * 60.0 - 144.0 * PI)));
    // 3. The part's depth: the cut follows.
    d.run(K::FeatureSetDepth {
        feature: "plate".into(),
        depth: 20.0,
        expected: Expected::Value(12.0),
    });
    assert!(close(vol(&d), 20.0 * (140.0 * 60.0 - 144.0 * PI)));
    assert_eq!(d.summary("hole").bounds_max[2], 20.0);
    assert_eq!(d.a.state().hash(), d.a.state().recompute_hash());
}

#[test]
fn cuts_stack_on_cuts_and_the_result_is_always_a_valid_closed_solid() {
    let mut d = plate();
    disc(
        &mut d,
        "t1",
        PlaneSpec::Xy { offset: 12.0 },
        25.0,
        30.0,
        8.0,
    );
    d.run(cut("c1", "plate", "t1", "profile.c", None, true));
    disc(
        &mut d,
        "t2",
        PlaneSpec::Xy { offset: 12.0 },
        75.0,
        30.0,
        10.0,
    );
    d.run(cut("c2", "c1", "t2", "profile.c", None, true));
    let s = d.summary("c2");
    assert!(
        close(s.volume, 12.0 * (6000.0 - 64.0 * PI - 100.0 * PI)),
        "{}",
        s.volume
    );
    assert!(d.a.solid("c2").unwrap().validate().is_empty());
    // Both walls are present, each named after the cut that made it.
    let names: Vec<String> = d.faces("c2").into_iter().map(|f| f.name).collect();
    assert!(
        names.contains(&"side:cut:c1:c".to_string())
            && names.contains(&"side:cut:c2:c".to_string()),
        "{names:?}"
    );
    // Editing the first tool ripples through both cuts.
    d.run(K::SketchSetValue {
        sketch: "t1".into(),
        constraint_id: "r".into(),
        value: 4.0,
        expected: Expected::Any,
    });
    assert!(close(
        d.summary("c2").volume,
        12.0 * (6000.0 - 16.0 * PI - 100.0 * PI)
    ));
}

#[test]
fn slots_notches_and_splits() {
    // A slot open to the top edge.
    let mut d = plate();
    rect(
        &mut d,
        "slot",
        PlaneSpec::Xy { offset: 12.0 },
        40.0,
        40.0,
        60.0,
        70.0,
    );
    d.run(cut("slotted", "plate", "slot", "profile.slota", None, true));
    assert!(
        close(d.summary("slotted").volume, 12.0 * (6000.0 - 20.0 * 20.0)),
        "{}",
        d.summary("slotted").volume
    );
    assert!(d.a.solid("slotted").unwrap().validate().is_empty());

    // A cut across the whole part splits it into two bodies.
    let mut d = plate();
    rect(
        &mut d,
        "gap",
        PlaneSpec::Xy { offset: 12.0 },
        45.0,
        -5.0,
        55.0,
        65.0,
    );
    d.run(cut("split", "plate", "gap", "profile.gapa", None, true));
    let solid = d.a.solid("split").unwrap();
    assert!(solid.validate().is_empty(), "{:?}", solid.validate());
    assert!(close(
        d.summary("split").volume,
        12.0 * (6000.0 - 10.0 * 60.0)
    ));
    // Two disjoint bodies mesh watertight, too.
    let mesh = solid.tessellate(TessParams::default()).unwrap();
    assert!(mesh.check().is_empty());
    assert!(close(mesh.volume(), solid.volume()));
}

#[test]
fn a_cut_that_goes_deeper_than_the_part_is_through_and_a_finite_depth_must_reach() {
    let mut d = plate();
    disc(
        &mut d,
        "tool",
        PlaneSpec::Xy { offset: 12.0 },
        50.0,
        30.0,
        8.0,
    );
    // Exactly the thickness, and more than it: both go through.
    d.run(cut("a", "plate", "tool", "profile.c", Some(12.0), true));
    d.run(cut("b", "plate", "tool", "profile.c", Some(500.0), true));
    let (a, b) = (d.summary("a"), d.summary("b"));
    // (Digests differ: face names embed the feature id, by design.)
    assert!(close(a.volume, b.volume) && close(a.area, b.area));
    assert_eq!(
        (a.vertices, a.edges, a.faces),
        (b.vertices, b.edges, b.faces)
    );
    // Shallower than the part: a blind pocket with a floor, no longer refused.
    d.run(cut("c", "plate", "tool", "profile.c", Some(5.0), true));
    assert!(
        close(d.summary("c").volume, 12.0 * 6000.0 - 64.0 * PI * 5.0),
        "{}",
        d.summary("c").volume
    );
    assert!(
        d.faces("c").iter().any(|f| f.name == "cap_end:c"),
        "the pocket floor is a named face"
    );
}

#[test]
fn unsupported_and_pointless_cuts_are_refused_with_reasons() {
    let mut d = plate();
    disc(
        &mut d,
        "mid",
        PlaneSpec::Xy { offset: 6.0 },
        50.0,
        30.0,
        8.0,
    );
    // A sketch plane half-way up the part cuts from there down: supported now.
    d.run(cut("midcut", "plate", "mid", "profile.c", None, true));
    assert!(close(
        d.summary("midcut").volume,
        12.0 * 6000.0 - 64.0 * PI * 6.0
    ));
    // Wrong direction from the top face: cutting away from the part.
    disc(
        &mut d,
        "top",
        PlaneSpec::Xy { offset: 12.0 },
        50.0,
        30.0,
        8.0,
    );
    let e = d
        .try_run(cut("x", "plate", "top", "profile.c", None, false))
        .unwrap_err();
    assert_eq!(e.code, DiagCode::FeatureFailed, "{e:?}");
    assert!(
        e.message.contains("never reaches the part")
            && e.suggestion
                .as_deref()
                .is_some_and(|s| s.contains("reverse")),
        "should say why and which way to go: {e:?}"
    );
    // A sketch on another plane (a side face): no longer refused, the general boolean cuts
    // a round hole sideways through the whole plate (through all, one way).
    disc(
        &mut d,
        "side",
        PlaneSpec::Xz { offset: 0.0 },
        50.0,
        6.0,
        3.0,
    );
    // The XZ plane faces -y, away from the plate: cut the other way round.
    d.run(cut("side_hole", "plate", "side", "profile.c", None, true));
    assert!(
        close(d.summary("side_hole").volume, 72000.0 - 9.0 * PI * 60.0),
        "{}",
        d.summary("side_hole").volume
    );
    // A tool that misses the part, and one that swallows it.
    disc(
        &mut d,
        "far",
        PlaneSpec::Xy { offset: 12.0 },
        500.0,
        500.0,
        5.0,
    );
    let e = d
        .try_run(cut("x", "plate", "far", "profile.c", None, true))
        .unwrap_err();
    assert_eq!(e.code, DiagCode::FeatureFailed);
    assert!(e.message.contains("remove nothing"), "{}", e.message);
    rect(
        &mut d,
        "huge",
        PlaneSpec::Xy { offset: 12.0 },
        -10.0,
        -10.0,
        200.0,
        200.0,
    );
    let e = d
        .try_run(cut("x", "plate", "huge", "profile.hugea", None, true))
        .unwrap_err();
    assert!(e.message.contains("nothing would be left"), "{}", e.message);
    // Tangent to the edge: ambiguous, so refused rather than guessed.
    disc(
        &mut d,
        "tan",
        PlaneSpec::Xy { offset: 12.0 },
        30.0,
        30.0,
        30.0,
    );
    let e = d
        .try_run(cut("x", "plate", "tan", "profile.c", None, true))
        .unwrap_err();
    assert_eq!(e.code, DiagCode::Unsupported);
    assert!(e.message.contains("touch"), "{}", e.message);
    // Unknown target / bad values / duplicate.
    assert_eq!(
        d.try_run(cut("x", "ghost", "top", "profile.c", None, true))
            .unwrap_err()
            .code,
        DiagCode::NotFound
    );
    assert_eq!(
        d.try_run(cut("x", "plate", "top", "profile.c", Some(0.0), true))
            .unwrap_err()
            .code,
        DiagCode::InvalidValue
    );
    assert_eq!(
        d.try_run(cut("x", "plate", "top", "profile.zzz", None, true))
            .unwrap_err()
            .code,
        DiagCode::NotFound
    );
    d.run(cut("ok", "plate", "top", "profile.c", None, true));
    assert_eq!(
        d.try_run(cut("ok", "plate", "top", "profile.c", None, true))
            .unwrap_err()
            .code,
        DiagCode::AlreadyExists
    );
    // None of that left anything behind.
    assert_eq!(
        d.a.state().feature_count(),
        4,
        "plate, midcut, side_hole and ok; none of the refused ones"
    );
}

#[test]
fn dependencies_are_protected() {
    let mut d = plate();
    disc(
        &mut d,
        "tool",
        PlaneSpec::Xy { offset: 12.0 },
        50.0,
        30.0,
        8.0,
    );
    d.run(cut("hole", "plate", "tool", "profile.c", None, true));
    let h = d.a.state_hash();
    // The cut's target and its tool sketch cannot be deleted from under it.
    let e = d
        .try_run(K::FeatureDelete {
            feature: "plate".into(),
        })
        .unwrap_err();
    assert_eq!(e.code, DiagCode::InUse);
    assert!(e.entities.contains(&"feature/hole".to_string()));
    let e = d
        .try_run(K::SketchDelete {
            sketch: "tool".into(),
        })
        .unwrap_err();
    assert_eq!(e.code, DiagCode::InUse);
    assert_eq!(d.a.state_hash(), h);
    // An edit to the part that would leave the cut with nothing to cut is rejected, naming the cut:
    // shrink the plate so the hole at x=50 falls outside it.
    let e = d
        .try_run(K::SketchSetValue {
            sketch: "base".into(),
            constraint_id: "w".into(),
            value: 20.0,
            expected: Expected::Any,
        })
        .unwrap_err();
    assert_eq!(e.code, DiagCode::FeatureFailed);
    assert!(
        e.entities.contains(&"feature/hole".to_string()),
        "{:?}",
        e.entities
    );
    assert_eq!(d.a.state_hash(), h);
    // In dependency order, one transaction removes everything.
    d.run(K::Transaction {
        label: "tear down".into(),
        ops: vec![
            K::FeatureDelete {
                feature: "hole".into(),
            },
            K::FeatureDelete {
                feature: "plate".into(),
            },
            K::SketchDelete {
                sketch: "tool".into(),
            },
            K::SketchDelete {
                sketch: "base".into(),
            },
        ],
    });
    assert_eq!(d.a.state().feature_count() + d.a.state().sketch_count(), 0);
}

#[test]
fn undo_restores_a_cut_and_everything_it_rebuilt() {
    let mut d = plate();
    disc(&mut d, "tool", XY, 50.0, 30.0, 8.0);
    d.run(cut("hole", "plate", "tool", "profile.c", None, false));
    let h = d.a.state_hash();
    d.run(K::FeatureSetDepth {
        feature: "plate".into(),
        depth: 30.0,
        expected: Expected::Any,
    });
    assert!(close(d.summary("hole").volume, 30.0 * (6000.0 - 64.0 * PI)));
    d.run(K::HistoryUndo { revision: None });
    assert_eq!(
        d.a.state_hash(),
        h,
        "the plate and the cut built on it are both back"
    );
    assert!(close(d.summary("hole").volume, 12.0 * (6000.0 - 64.0 * PI)));
    // Undoing the cut's creation leaves the plate; redo brings the cut back identically.
    let creation =
        d.a.log()
            .iter()
            .find(|e| matches!(e.command.kind, K::FeatureCut { .. }))
            .unwrap()
            .revision;
    // (later commands touched `plate`'s dependents only through the undone depth edit)
    d.run(K::HistoryRedo { revision: None });
    d.run(K::HistoryUndo { revision: None });
    d.run(K::HistoryUndo {
        revision: Some(creation),
    });
    assert!(d.a.state().feature("hole").is_none());
    assert!(d.a.state().feature("plate").is_some());
}

#[test]
fn dry_run_replay_and_json_cover_cuts() {
    let mut d = plate();
    disc(
        &mut d,
        "tool",
        PlaneSpec::Xy { offset: 12.0 },
        50.0,
        30.0,
        8.0,
    );
    let c = d.cmd(cut("hole", "plate", "tool", "profile.c", None, true));
    let h = d.a.state_hash();
    let predicted = d.a.dry_run(&c).unwrap();
    assert_eq!(d.a.state_hash(), h);
    assert!(d.a.state().feature("hole").is_none());
    assert_eq!(d.a.submit(c).unwrap(), predicted);

    d.run(K::SketchSetValue {
        sketch: "tool".into(),
        constraint_id: "r".into(),
        value: 9.5,
        expected: Expected::Any,
    });
    let jsonl = d.a.log_jsonl();
    let b = Authority::replay(&jsonl, Policy::default()).unwrap();
    assert_eq!(b.state_hash(), d.a.state_hash());
    assert_eq!(
        b.solid("hole").unwrap().canonical_bytes(),
        d.a.solid("hole").unwrap().canonical_bytes()
    );
    assert!(Authority::replay(&jsonl.replacen("9.5", "9.25", 1), Policy::default()).is_err());

    for kind in [
        cut("h", "t", "s", "p", None, false),
        cut("h", "t", "s", "p", Some(4.5), true),
    ] {
        let cmd = Command {
            id: "x".into(),
            actor: Actor::Agent("claude".into()),
            base_revision: Revision(1),
            kind,
        };
        let json = serde_json::to_string(&cmd).unwrap();
        assert_eq!(
            serde_json::from_str::<Command>(&json).unwrap(),
            cmd,
            "{json}"
        );
    }
    let c: Command = serde_json::from_str(r#"{"id":"x","actor":{"kind":"human","name":"a"},"base_revision":0,"type":"feature.cut","feature":"h","target":"t","sketch":"s","profiles": ["p"]}"#).unwrap();
    assert!(
        matches!(
            c.kind,
            K::FeatureCut {
                depth: None,
                reverse: false,
                ..
            }
        ),
        "depth and reverse are optional"
    );
}

#[test]
fn a_cut_part_exports_as_a_watertight_mesh_close_to_the_exact_volume() {
    let mut d = plate();
    disc(
        &mut d,
        "t1",
        PlaneSpec::Xy { offset: 12.0 },
        30.0,
        30.0,
        9.0,
    );
    d.run(cut("c1", "plate", "t1", "profile.c", None, true));
    rect(
        &mut d,
        "slot",
        PlaneSpec::Xy { offset: 12.0 },
        60.0,
        20.0,
        110.0,
        40.0,
    );
    d.run(cut("c2", "c1", "slot", "profile.slota", None, true));
    let solid = d.a.solid("c2").unwrap();
    assert!(solid.validate().is_empty());
    let mesh = solid.tessellate(TessParams::default()).unwrap();
    assert!(mesh.check().is_empty(), "{:?}", mesh.check());
    assert!((mesh.volume() - solid.volume()).abs() / solid.volume() < 5e-3);
    assert!(mesh.volume() > 0.0);
}
