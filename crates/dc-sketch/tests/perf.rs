//! Timing probes: `cargo test --release -p dc-sketch --test perf -- --ignored --nocapture`.
use dc_sketch::{Constraint as C, Entity as E, Sketch};
use std::time::Instant;

fn rects(n: usize) -> Sketch {
    let mut s = Sketch::new();
    for i in 0..n {
        let (x, y) = ((i % 20) as f64 * 30.0, (i / 20) as f64 * 30.0);
        let p = |k: usize| format!("r{i}p{k}");
        let l = |k: usize| format!("r{i}l{k}");
        for (k, (px, py)) in [(x, y), (x + 20.0, y), (x + 20.0, y + 10.0), (x, y + 10.0)]
            .iter()
            .enumerate()
        {
            s.add_entity(&p(k), E::Point { x: *px, y: *py }).unwrap();
        }
        for k in 0..4 {
            s.add_entity(
                &l(k),
                E::Line {
                    a: p(k),
                    b: p((k + 1) % 4),
                },
            )
            .unwrap();
            s.add_constraint(
                &format!("r{i}c{k}"),
                if k % 2 == 0 {
                    C::Horizontal { line: l(k) }
                } else {
                    C::Vertical { line: l(k) }
                },
            )
            .unwrap();
        }
        s.add_constraint(
            &format!("r{i}w"),
            C::Length {
                line: l(0),
                value: 20.0,
            },
        )
        .unwrap();
    }
    s
}

fn time<T>(label: &str, f: impl FnOnce() -> T) -> T {
    let t = Instant::now();
    let r = f();
    println!("{label:32} {:8.2} ms", t.elapsed().as_secs_f64() * 1e3);
    r
}

#[test]
#[ignore]
fn where_the_time_goes_at_200_rectangles() {
    let mut s = time("build 200 rectangles", || rects(200));
    time("solve (whole sketch)", || s.solve());
    time("solve again", || s.solve());
    let c = time("clone", || s.clone());
    time("json", || serde_json::to_string(&c).unwrap());
    time("profiles", || s.profiles());
    time("follow_placements", || {
        let mut t = s.clone();
        t.follow_placements(&c);
    });
    time("equality", || s == c);
}

/// One connected chain of `n` lines (a staircase): a single cluster of 2(n+1) variables, the
/// shape that cluster decomposition cannot split. Fully dimensioned from a fixed first point.
fn staircase(n: usize) -> Sketch {
    let mut s = Sketch::new();
    let (mut x, mut y) = (0.0, 0.0);
    s.add_entity("p0", E::Point { x, y }).unwrap();
    s.add_constraint(
        "fix",
        C::Fixed {
            point: "p0".into(),
            x: 0.0,
            y: 0.0,
        },
    )
    .unwrap();
    for i in 0..n {
        // Start a little off the solution so the solver has work to do.
        if i % 2 == 0 {
            x += 10.3;
        } else {
            y += 7.9;
        }
        s.add_entity(&format!("p{}", i + 1), E::Point { x, y })
            .unwrap();
        s.add_entity(
            &format!("l{i}"),
            E::Line {
                a: format!("p{i}"),
                b: format!("p{}", i + 1),
            },
        )
        .unwrap();
        s.add_constraint(
            &format!("d{i}"),
            if i % 2 == 0 {
                C::Horizontal {
                    line: format!("l{i}"),
                }
            } else {
                C::Vertical {
                    line: format!("l{i}"),
                }
            },
        )
        .unwrap();
        s.add_constraint(
            &format!("n{i}"),
            C::Length {
                line: format!("l{i}"),
                value: if i % 2 == 0 { 10.0 } else { 8.0 },
            },
        )
        .unwrap();
    }
    s
}

#[test]
#[ignore]
fn one_big_cluster_scaling() {
    for n in [25, 50, 100, 200, 400] {
        let mut s = staircase(n);
        let t = Instant::now();
        let report = s.solve();
        let solve = t.elapsed().as_secs_f64() * 1e3;
        // The cost of changing one dimension (what a commit pays), and of adding a redundant
        // constraint (which has to find out which rows are dependent).
        let t = Instant::now();
        s.set_constraint_value("n3", 11.0).unwrap();
        let _ = s.solve();
        let edit = t.elapsed().as_secs_f64() * 1e3;
        let t = Instant::now();
        s.add_constraint(
            "extra",
            C::Length {
                line: "l0".into(),
                value: 10.0,
            },
        )
        .ok();
        let _ = s.solve();
        let redundant = t.elapsed().as_secs_f64() * 1e3;
        println!(
            "{n:4} lines, {:4} variables: first solve {solve:8.2} ms, change a dimension {edit:8.2} ms, add a redundant constraint {redundant:8.2} ms  ({:?})",
            2 * (n + 1),
            report.status
        );
    }
}
