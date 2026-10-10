//! Timing probes, not correctness tests: `cargo test --release -p dc-doc --test perf -- --ignored --nocapture`.

use dc_doc::{Actor, Authority, Command, CommandKind as K, Query};
use dc_sketch::{Constraint as C, Entity as E, PlaneSpec};
use std::time::Instant;

struct Doc {
    a: Authority,
    n: u64,
}

impl Doc {
    fn run(&mut self, kind: K) {
        self.n += 1;
        let c = Command {
            id: format!("c{}", self.n),
            actor: Actor::Human("ann".into()),
            base_revision: self.a.revision(),
            kind,
        };
        self.a.submit(c).unwrap_or_else(|d| panic!("{d:?}"));
    }
}

fn add_rect(d: &mut Doc, i: usize) {
    let (x, y) = ((i % 20) as f64 * 30.0, (i / 20) as f64 * 30.0);
    let p = |k: usize| format!("r{i}p{k}");
    let l = |k: usize| format!("r{i}l{k}");
    let pts = [(x, y), (x + 20.0, y), (x + 20.0, y + 10.0), (x, y + 10.0)];
    let mut ops = Vec::new();
    for (k, (px, py)) in pts.iter().enumerate() {
        ops.push(K::SketchAddEntity {
            sketch: "s".into(),
            entity_id: p(k),
            entity: E::Point { x: *px, y: *py },
        });
    }
    for k in 0..4 {
        ops.push(K::SketchAddEntity {
            sketch: "s".into(),
            entity_id: l(k),
            entity: E::Line {
                a: p(k),
                b: p((k + 1) % 4),
            },
        });
    }
    for k in 0..4 {
        ops.push(K::SketchAddConstraint {
            sketch: "s".into(),
            constraint_id: format!("r{i}c{k}"),
            constraint: if k % 2 == 0 {
                C::Horizontal { line: l(k) }
            } else {
                C::Vertical { line: l(k) }
            },
        });
    }
    ops.push(K::SketchAddConstraint {
        sketch: "s".into(),
        constraint_id: format!("r{i}w"),
        constraint: C::Length {
            line: l(0),
            value: 20.0,
        },
    });
    ops.push(K::SketchAddConstraint {
        sketch: "s".into(),
        constraint_id: format!("r{i}h"),
        constraint: C::Length {
            line: l(1),
            value: 10.0,
        },
    });
    for k in ops {
        d.run(k);
    }
}

#[test]
#[ignore]
fn sketch_commands_scale_with_sketch_size() {
    let mut d = Doc {
        a: Authority::default(),
        n: 0,
    };
    d.run(K::SketchCreate {
        sketch: "s".into(),
        plane: PlaneSpec::Xy { offset: 0.0 },
    });
    let mut done = 0;
    for target in [10, 25, 50, 100, 200] {
        let t = Instant::now();
        let mut cmds = 0;
        while done < target {
            add_rect(&mut d, done);
            done += 1;
            cmds += 14;
        }
        let dt = t.elapsed();
        println!(
            "{done:4} rectangles: +{cmds} commands took {:8.1} ms ({:6.2} ms per command)",
            dt.as_secs_f64() * 1e3,
            dt.as_secs_f64() * 1e3 / f64::from(cmds)
        );
    }
    let t = Instant::now();
    d.run(K::SketchSetValue {
        sketch: "s".into(),
        constraint_id: "r0w".into(),
        value: 21.0,
        expected: dc_doc::Expected::default(),
    });
    println!(
        "one value edit at {done} rectangles: {:.1} ms",
        t.elapsed().as_secs_f64() * 1e3
    );
    let t = Instant::now();
    let _ = d.a.query(&Query::SketchProfiles {
        sketch: "s".into(),
        after: None,
        limit: None,
    });
    println!("profiles query: {:.1} ms", t.elapsed().as_secs_f64() * 1e3);
    let t = Instant::now();
    let _ = d.a.state_hash();
    println!("state hash: {:.3} ms", t.elapsed().as_secs_f64() * 1e3);
}
