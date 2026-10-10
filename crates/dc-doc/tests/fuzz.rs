//! Random command sequences against the authority. Whatever is thrown at it, these must hold:
//! nothing panics; a refused command changes nothing; every committed sketch is solved and
//! finite; undo returns to the previous state hash and redo to the next one; replaying the log
//! gives the same hash.

use dc_doc::{Actor, Authority, Command, CommandKind as K, Expected, Policy, Revision};
use dc_sketch::{
    Constraint as C, DragRadius, DragTarget, Entity as E, MirrorAbout, Pattern, PlaneSpec,
    SketchAxis, SolveStatus, Transform,
};

struct Rng(u64);
impl Rng {
    fn next(&mut self) -> u64 {
        // splitmix64: deterministic and good enough
        self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }
    fn below(&mut self, n: usize) -> usize {
        (self.next() % n.max(1) as u64) as usize
    }
    fn coord(&mut self) -> f64 {
        // multiples of 0.5 in [-60, 60], with the occasional exact zero or repeat-friendly value
        (self.below(241) as f64 - 120.0) * 0.5
    }
    fn positive(&mut self) -> f64 {
        (self.below(80) as f64 + 1.0) * 0.5
    }
}

fn pick(r: &mut Rng, v: &[String]) -> String {
    if v.is_empty() {
        "missing".to_owned()
    } else {
        v[r.below(v.len())].clone()
    }
}

struct World {
    a: Authority,
    n: u64,
    next_id: u32,
}

type Ids = (
    Vec<String>,
    Vec<String>,
    Vec<String>,
    Vec<String>,
    Vec<String>,
);

fn lists(w: &World) -> Ids {
    let (mut pts, mut lines, mut curves, mut cons, mut all) =
        (vec![], vec![], vec![], vec![], vec![]);
    if let Some(slot) = w.a.state().sketch("s") {
        for (id, e) in slot.sketch.entities() {
            all.push(id.clone());
            match e {
                E::Point { .. } => pts.push(id.clone()),
                E::Line { .. } => lines.push(id.clone()),
                _ => curves.push(id.clone()),
            }
        }
        cons.extend(slot.sketch.constraints().keys().cloned());
    }
    (pts, lines, curves, cons, all)
}

fn make(w: &mut World, r: &mut Rng) -> K {
    let (pts, lines, curves, cons, all) = lists(w);
    let s = || "s".to_owned();
    w.next_id += 1;
    let nid = format!("n{}", w.next_id);
    match r.below(40) {
        0..=6 => K::SketchAddEntity {
            sketch: s(),
            entity_id: nid,
            entity: E::Point {
                x: r.coord(),
                y: r.coord(),
            },
        },
        7..=11 => K::SketchAddEntity {
            sketch: s(),
            entity_id: nid,
            entity: E::Line {
                a: pick(r, &pts),
                b: pick(r, &pts),
            },
        },
        12..=13 => K::SketchAddEntity {
            sketch: s(),
            entity_id: nid,
            entity: E::Circle {
                center: pick(r, &pts),
                radius: r.positive(),
            },
        },
        14 => K::SketchAddEntity {
            sketch: s(),
            entity_id: nid,
            entity: E::Arc {
                center: pick(r, &pts),
                start: pick(r, &pts),
                end: pick(r, &pts),
            },
        },
        15..=22 => {
            let (a, b, l, c) = (
                pick(r, &pts),
                pick(r, &pts),
                pick(r, &lines),
                pick(r, &curves),
            );
            let l2 = pick(r, &lines);
            let any = pick(r, &all);
            let v = r.positive();
            let constraint = match r.below(22) {
                0 => C::Coincident { a, b },
                1 => C::Horizontal { line: l },
                2 => C::Vertical { line: l },
                3 => C::Parallel { a: l, b: l2 },
                4 => C::Perpendicular { a: l, b: l2 },
                5 => C::EqualLength { a: l, b: l2 },
                6 => C::Length { line: l, value: v },
                7 => C::Distance { a, b, value: v },
                8 => C::HorizontalDistance { a, b, value: v },
                9 => C::VerticalDistance { a, b, value: v },
                10 => C::Radius {
                    target: c,
                    value: v,
                },
                11 => C::Fixed {
                    point: a,
                    x: r.coord(),
                    y: r.coord(),
                },
                12 => C::PointOnLine { point: a, line: l },
                13 => C::OnAxis {
                    point: a,
                    axis: SketchAxis::X,
                },
                14 => C::Midpoint { point: a, line: l },
                15 => C::Symmetric { a, b, line: l },
                16 => C::EqualRadius { a: c, b: any },
                17 => C::PointOnCurve { point: a, curve: c },
                18 => C::Angle {
                    a: l,
                    b: l2,
                    value: (r.below(170) + 5) as f64,
                },
                19 => C::Tangent { line: l, curve: c },
                20 => C::PointLineDistance {
                    point: a,
                    line: l,
                    value: v,
                },
                _ => C::ArcLength { arc: c, value: v },
            };
            w.next_id += 1;
            K::SketchAddConstraint {
                sketch: s(),
                constraint_id: format!("k{}", w.next_id),
                constraint,
            }
        }
        23 => K::SketchRemoveConstraint {
            sketch: s(),
            constraint_id: pick(r, &cons),
        },
        24 => K::SketchRemoveEntity {
            sketch: s(),
            entity_id: pick(r, &all),
        },
        25..=26 => K::SketchSetValue {
            sketch: s(),
            constraint_id: pick(r, &cons),
            value: r.positive(),
            expected: Expected::default(),
        },
        27 => K::SketchDrag {
            sketch: s(),
            points: vec![DragTarget {
                point: pick(r, &pts),
                x: r.coord(),
                y: r.coord(),
            }],
            radius: if r.below(3) == 0 {
                Some(DragRadius {
                    circle: pick(r, &curves),
                    value: r.positive(),
                })
            } else {
                None
            },
        },
        28 => K::SketchTrim {
            sketch: s(),
            entity_id: pick(r, &all),
            x: r.coord(),
            y: r.coord(),
            construction: false,
            bounds: vec![],
        },
        29 => K::SketchExtend {
            sketch: s(),
            entity_id: pick(r, &all),
            x: r.coord(),
            y: r.coord(),
            construction: false,
        },
        30 => K::SketchSplit {
            sketch: s(),
            entity_id: pick(r, &all),
            x: r.coord(),
            y: r.coord(),
        },
        31 => K::SketchTransform {
            sketch: s(),
            entities: vec![pick(r, &all), pick(r, &all)],
            transform: match r.below(3) {
                0 => Transform::Move {
                    dx: r.coord(),
                    dy: r.coord(),
                },
                1 => Transform::Rotate {
                    cx: r.coord(),
                    cy: r.coord(),
                    degrees: (r.below(360)) as f64,
                },
                _ => Transform::Scale {
                    cx: r.coord(),
                    cy: r.coord(),
                    factor: r.positive() / 10.0,
                },
            },
            copy: r.below(2) == 0,
        },
        32 => K::SketchMirror {
            sketch: s(),
            entities: vec![pick(r, &all), pick(r, &all)],
            about: match r.below(3) {
                0 => MirrorAbout::AxisX,
                1 => MirrorAbout::AxisY,
                _ => MirrorAbout::Line {
                    entity: pick(r, &lines),
                },
            },
        },
        33 => K::SketchPattern {
            sketch: s(),
            entities: vec![pick(r, &all)],
            pattern: if r.below(2) == 0 {
                Pattern::Linear {
                    dx: r.coord(),
                    dy: r.coord(),
                    count: r.below(5) as u32 + 1,
                }
            } else {
                Pattern::Circular {
                    cx: r.coord(),
                    cy: r.coord(),
                    degrees: 360.0,
                    count: r.below(5) as u32 + 1,
                }
            },
        },
        34 => K::SketchOffset {
            sketch: s(),
            entities: vec![pick(r, &lines), pick(r, &lines), pick(r, &curves)],
            distance: r.positive() / 4.0,
        },
        35 => K::SketchFillet {
            sketch: s(),
            first: pick(r, &lines),
            second: pick(r, &lines),
            radius: r.positive() / 4.0,
        },
        36 => K::SketchChamfer {
            sketch: s(),
            first: pick(r, &lines),
            second: pick(r, &lines),
            distance: r.positive() / 4.0,
            second_distance: None,
        },
        37 => K::SketchMergePoints {
            sketch: s(),
            a: pick(r, &pts),
            b: pick(r, &pts),
        },
        38 => {
            if r.below(2) == 0 {
                K::SketchRepair { sketch: s() }
            } else {
                K::SketchFullyDefine { sketch: s() }
            }
        }
        39 if r.below(2) == 0 => K::SketchSetInfinite {
            sketch: s(),
            entity_id: pick(r, &lines),
            infinite: r.below(3) != 0,
        },
        _ => K::SketchSetConstruction {
            sketch: s(),
            entity_id: pick(r, &all),
            construction: r.below(2) == 0,
        },
    }
}

fn submit(w: &mut World, kind: K) -> Result<(), dc_doc::Diagnostic> {
    w.n += 1;
    let c = Command {
        id: format!("c{}", w.n),
        actor: Actor::Human("fuzz".into()),
        base_revision: w.a.revision(),
        kind,
    };
    w.a.submit(c).map(|_| ())
}

fn check_sketch(w: &World, seed: u64, step: usize, what: &str) {
    let Some(slot) = w.a.state().sketch("s") else {
        return;
    };
    for (id, e) in slot.sketch.entities() {
        let finite = match e {
            E::Point { x, y } => x.is_finite() && y.is_finite(),
            E::Circle { radius, .. } => radius.is_finite() && *radius > 0.0,
            _ => true,
        };
        assert!(
            finite,
            "seed {seed} step {step} after {what}: {id} is not finite: {e:?}"
        );
    }
    let mut again = slot.sketch.clone();
    let rep = again.solve();
    assert!(
        rep.status == SolveStatus::Solved && rep.max_residual < 1e-6,
        "seed {seed} step {step} after {what}: committed sketch is not solved: re-solve {:?}, residual {:e} (stored {:?}, {:e})",
        rep.status,
        rep.max_residual,
        slot.report.status,
        slot.report.max_residual
    );
}

fn run(seed: u64, steps: usize) -> (usize, usize) {
    let mut r = Rng(seed);
    let mut w = World {
        a: Authority::new(Policy::default()),
        n: 0,
        next_id: 0,
    };
    submit(
        &mut w,
        K::SketchCreate {
            sketch: "s".into(),
            plane: PlaneSpec::Xy { offset: 0.0 },
        },
    )
    .unwrap();
    let (mut accepted, mut refused) = (0, 0);
    for step in 0..steps {
        let kind = make(&mut w, &mut r);
        let what = format!("{kind:?}");
        let what = &what[..what.len().min(160)];
        let (rev, hash) = (w.a.revision(), w.a.state_hash());
        match submit(&mut w, kind) {
            Err(_) => {
                refused += 1;
                assert_eq!(
                    (w.a.revision(), w.a.state_hash()),
                    (rev, hash),
                    "seed {seed} step {step}: a refused command changed the document: {what}"
                );
            }
            Ok(()) => {
                accepted += 1;
                check_sketch(&w, seed, step, what);
                // Now and then: undo must restore the previous hash and redo the new one.
                if r.below(6) == 0 && w.a.revision() != rev {
                    let after = w.a.state_hash();
                    if submit(&mut w, K::HistoryUndo { revision: None }).is_ok() {
                        assert_eq!(
                            w.a.state_hash(),
                            hash,
                            "seed {seed} step {step}: undo did not restore the state after {what}"
                        );
                        if submit(&mut w, K::HistoryRedo { revision: None }).is_ok() {
                            assert_eq!(
                                w.a.state_hash(),
                                after,
                                "seed {seed} step {step}: redo did not restore the state after {what}"
                            );
                        }
                    }
                }
            }
        }
    }
    let replayed = Authority::replay(&w.a.log_jsonl(), Policy::default())
        .unwrap_or_else(|e| panic!("seed {seed}: replay failed: {e:?}"));
    assert_eq!(
        replayed.state_hash(),
        w.a.state_hash(),
        "seed {seed}: replaying the log gave a different state"
    );
    let _ = Revision(0);
    (accepted, refused)
}

/// A bigger sweep for hunting: `cargo test --release -p dc-doc --test fuzz -- --ignored --nocapture`.
#[test]
#[ignore]
fn a_large_random_sweep() {
    let (mut acc, mut refd) = (0, 0);
    let first: u64 = std::env::var("FUZZ_SEED")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(1000);
    let last: u64 = std::env::var("FUZZ_LAST")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(1400);
    for seed in first..last {
        let (a, r) = run(seed, 250);
        acc += a;
        refd += r;
    }
    println!("accepted {acc}, refused {refd}");
}

#[test]
fn random_sketch_commands_keep_every_contract() {
    let (mut acc, mut refd) = (0, 0);
    for seed in 1..=5 {
        let (a, r) = run(seed, 100);
        acc += a;
        refd += r;
    }
    println!("accepted {acc}, refused {refd}");
    assert!(
        acc > 300,
        "the generator should get plenty of commands through ({acc})"
    );
}
