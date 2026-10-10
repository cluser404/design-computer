//! Fillets and chamfers of edges parallel to the stack's axis, checked against exact areas, the
//! slab volume, the validator, and the tessellation.

use core::f64::consts::PI;
use dc_features::{build_solid, Round, StepError, Stepped};
use dc_kernel::{Mesh, Solid, TessParams};
use dc_math::{Plane, Tolerance, Vec2, Vec3};
use dc_sketch::{Entity, Profile, Sketch};

fn xy() -> Plane {
    Plane::new(Vec3::ZERO, Vec3::X, Vec3::Y, Tolerance::DEFAULT).unwrap()
}

/// A closed polygon, and optionally a hole polygon inside it.
fn polygon(outer: &[(f64, f64)], hole: Option<&[(f64, f64)]>) -> Profile {
    let mut s = Sketch::new();
    for (tag, pts) in [("o", Some(outer)), ("h", hole)] {
        let Some(pts) = pts else { continue };
        for (i, (x, y)) in pts.iter().enumerate() {
            s.add_entity(&format!("{tag}{i}"), Entity::Point { x: *x, y: *y })
                .unwrap();
        }
        for i in 0..pts.len() {
            s.add_entity(
                &format!("{tag}e{i}"),
                Entity::Line {
                    a: format!("{tag}{i}"),
                    b: format!("{tag}{}", (i + 1) % pts.len()),
                },
            )
            .unwrap();
        }
    }
    let r = s.profiles();
    assert!(r.problems.is_empty(), "{:?}", r.problems);
    assert_eq!(r.profiles.len(), 1);
    r.profiles.into_iter().next().unwrap()
}

fn rect(x0: f64, y0: f64, x1: f64, y1: f64) -> Profile {
    polygon(&[(x0, y0), (x1, y0), (x1, y1), (x0, y1)], None)
}

fn slab_volume(st: &Stepped) -> f64 {
    st.slabs
        .iter()
        .enumerate()
        .map(|(k, s)| s.iter().map(|p| p.area).sum::<f64>() * (st.levels[k + 1].w - st.levels[k].w))
        .sum()
}

fn close(a: f64, b: f64) -> bool {
    (a - b).abs() <= 1e-7 * b.abs().max(1.0)
}

fn build(st: &Stepped) -> Solid {
    build_solid(st).unwrap_or_else(|e| panic!("{e}"))
}

fn watertight(s: &Solid) -> Mesh {
    let m = s.tessellate(TessParams::default()).unwrap();
    let p = m.check();
    assert!(p.is_empty(), "not watertight: {p:?}");
    m
}

/// What rounding a right-angle corner by `r` takes off (or, for a concave one, adds).
fn corner_area(r: f64, kind: Round) -> f64 {
    match kind {
        Round::Fillet => r * r * (1.0 - PI / 4.0),
        Round::Chamfer => r * r / 2.0,
    }
}

const BOTH: [Round; 2] = [Round::Fillet, Round::Chamfer];

#[test]
fn a_convex_corner_loses_exactly_the_corner_area() {
    for kind in BOTH {
        let st = Stepped::extrusion(xy(), rect(0.0, 0.0, 40.0, 20.0), 10.0, false);
        let r = st
            .round_edge(Vec2::new(0.0, 0.0), 0.0, 10.0, 3.0, kind, "round")
            .unwrap();
        let want = (800.0 - corner_area(3.0, kind)) * 10.0;
        assert!(close(slab_volume(&r), want), "{kind:?}");
        let s = build(&r);
        assert!(close(s.volume(), want), "{kind:?}: {}", s.volume());
        assert!(s.faces.iter().any(|f| f.name.contains("round")), "{kind:?}");
        let m = watertight(&s);
        assert!((m.volume() - want).abs() < 0.01 * want, "{kind:?}");
        // The original is untouched (the operation is pure).
        assert!(close(slab_volume(&st), 8000.0));
    }
}

#[test]
fn all_four_corners_can_be_rounded_one_after_another() {
    let mut st = Stepped::extrusion(xy(), rect(0.0, 0.0, 40.0, 20.0), 10.0, false);
    for (i, (x, y)) in [(0.0, 0.0), (40.0, 0.0), (40.0, 20.0), (0.0, 20.0)]
        .into_iter()
        .enumerate()
    {
        st = st
            .round_edge(
                Vec2::new(x, y),
                0.0,
                10.0,
                4.0,
                Round::Fillet,
                &format!("r{i}"),
            )
            .unwrap();
    }
    let want = (800.0 - 4.0 * corner_area(4.0, Round::Fillet)) * 10.0;
    let s = build(&st);
    assert!(close(s.volume(), want), "{}", s.volume());
    // One cylinder face per rounded corner, named for it.
    for i in 0..4 {
        let want = format!("side:r{i}");
        assert!(
            s.faces.iter().any(|f| f.name == want),
            "no face {want}: {:?}",
            s.faces.iter().map(|f| &f.name).collect::<Vec<_>>()
        );
    }
    assert_eq!(
        s.faces
            .iter()
            .filter(|f| matches!(f.surface, dc_kernel::Surface::Cylinder { .. }))
            .count(),
        4
    );
    watertight(&s);
}

#[test]
fn a_concave_corner_is_filled_not_cut() {
    // An L: the inside corner at (20, 20) is reflex; rounding it adds material.
    let l = polygon(
        &[
            (0.0, 0.0),
            (40.0, 0.0),
            (40.0, 20.0),
            (20.0, 20.0),
            (20.0, 40.0),
            (0.0, 40.0),
        ],
        None,
    );
    let area = 40.0 * 20.0 + 20.0 * 20.0;
    for kind in BOTH {
        let st = Stepped::extrusion(xy(), l.clone(), 10.0, false);
        let r = st
            .round_edge(Vec2::new(20.0, 20.0), 0.0, 10.0, 4.0, kind, "in")
            .unwrap();
        let want = (area + corner_area(4.0, kind)) * 10.0;
        assert!(
            close(slab_volume(&r), want),
            "{kind:?}: {}",
            slab_volume(&r)
        );
        let s = build(&r);
        assert!(close(s.volume(), want), "{kind:?}: {}", s.volume());
        watertight(&s);
    }
}

#[test]
fn a_corner_of_a_pocket_is_rounded_too_and_fills_it() {
    let p = polygon(
        &[(0.0, 0.0), (60.0, 0.0), (60.0, 40.0), (0.0, 40.0)],
        Some(&[(20.0, 10.0), (40.0, 10.0), (40.0, 30.0), (20.0, 30.0)]),
    );
    for kind in BOTH {
        let st = Stepped::extrusion(xy(), p.clone(), 10.0, false);
        let r = st
            .round_edge(Vec2::new(20.0, 10.0), 0.0, 10.0, 3.0, kind, "pk")
            .unwrap();
        // The hole loses a corner, so the part gains it.
        let want = (2400.0 - 400.0 + corner_area(3.0, kind)) * 10.0;
        assert!(
            close(slab_volume(&r), want),
            "{kind:?}: {}",
            slab_volume(&r)
        );
        let s = build(&r);
        assert!(close(s.volume(), want), "{kind:?}: {}", s.volume());
        watertight(&s);
    }
}

#[test]
fn an_edge_that_runs_up_through_several_layers_rounds_in_all_of_them() {
    // A plate with a boss that shares its corner; the edge runs the whole height.
    let st = Stepped::extrusion(xy(), rect(0.0, 0.0, 40.0, 20.0), 10.0, false)
        .boss(&rect(0.0, 0.0, 10.0, 10.0), 10.0, 16.0, "b")
        .unwrap();
    assert_eq!(st.slabs.len(), 2);
    let r = st
        .round_edge(Vec2::new(0.0, 0.0), 0.0, 16.0, 2.0, Round::Fillet, "tall")
        .unwrap();
    let a = corner_area(2.0, Round::Fillet);
    let want = (800.0 - a) * 10.0 + (100.0 - a) * 6.0;
    assert!(close(slab_volume(&r), want), "{}", slab_volume(&r));
    let s = build(&r);
    assert!(close(s.volume(), want), "{}", s.volume());
    // One tall face of the fillet, not one per layer.
    let tall: Vec<_> = s.faces.iter().filter(|f| f.name.contains("tall")).collect();
    assert_eq!(
        tall.len(),
        1,
        "{:?}",
        tall.iter().map(|f| &f.name).collect::<Vec<_>>()
    );
    watertight(&s);
    // Rounding only the lower layer is a different, still valid, part: the boss keeps its
    // sharp corner and overhangs the rounded plate.
    let low = st
        .round_edge(Vec2::new(0.0, 0.0), 0.0, 10.0, 2.0, Round::Fillet, "low")
        .unwrap();
    let want_low = (800.0 - a) * 10.0 + 100.0 * 6.0;
    let s = build(&low);
    assert!(close(s.volume(), want_low), "{}", s.volume());
    watertight(&s);
}

#[test]
fn refusals_say_why() {
    let st = Stepped::extrusion(xy(), rect(0.0, 0.0, 40.0, 20.0), 10.0, false);
    let msg = |e: StepError| e.to_string();
    // Too big for the 20 long edge.
    let e = st
        .round_edge(Vec2::new(0.0, 0.0), 0.0, 10.0, 25.0, Round::Chamfer, "x")
        .unwrap_err();
    assert!(msg(e).contains("too big"), "too big");
    // Not a corner of the outline.
    let e = st
        .round_edge(Vec2::new(5.0, 5.0), 0.0, 10.0, 1.0, Round::Fillet, "x")
        .unwrap_err();
    assert!(msg(e).contains("no corner"), "no corner");
    // Zero, negative and not finite sizes.
    for bad in [0.0, -1.0, f64::NAN, f64::INFINITY] {
        assert!(st
            .round_edge(Vec2::new(0.0, 0.0), 0.0, 10.0, bad, Round::Fillet, "x")
            .is_err());
    }
    // A span outside the part.
    let e = st
        .round_edge(Vec2::new(0.0, 0.0), 20.0, 30.0, 1.0, Round::Fillet, "x")
        .unwrap_err();
    assert!(msg(e).contains("no layer"), "no layer");
    // Next to an arc: not supported yet, and said so.
    let d = {
        let mut s = Sketch::new();
        s.add_entity("m", Entity::Point { x: 5.0, y: 5.0 }).unwrap();
        s.add_entity(
            "c",
            Entity::Circle {
                center: "m".into(),
                radius: 2.0,
            },
        )
        .unwrap();
        s.profiles().profiles.into_iter().next().unwrap()
    };
    let st2 = Stepped::extrusion(xy(), d, 10.0, false);
    assert!(st2
        .round_edge(Vec2::new(7.0, 5.0), 0.0, 10.0, 0.5, Round::Fillet, "x")
        .is_err());
}

#[test]
fn rounding_the_same_corner_twice_is_refused_and_a_rounded_part_stays_valid_when_cut() {
    let st = Stepped::extrusion(xy(), rect(0.0, 0.0, 40.0, 20.0), 10.0, false)
        .round_edge(Vec2::new(0.0, 0.0), 0.0, 10.0, 3.0, Round::Fillet, "a")
        .unwrap();
    // The corner is gone: there is nothing left at that point to round.
    assert!(st
        .round_edge(Vec2::new(0.0, 0.0), 0.0, 10.0, 1.0, Round::Fillet, "b")
        .is_err());
    // A hole through the rounded plate still works and validates.
    let hole = {
        let mut s = Sketch::new();
        s.add_entity("m", Entity::Point { x: 20.0, y: 10.0 })
            .unwrap();
        s.add_entity(
            "c",
            Entity::Circle {
                center: "m".into(),
                radius: 4.0,
            },
        )
        .unwrap();
        s.profiles().profiles.into_iter().next().unwrap()
    };
    let cut = st.cut(&hole, 0.0, f64::INFINITY, "h").unwrap();
    let want = (800.0 - corner_area(3.0, Round::Fillet) - PI * 16.0) * 10.0;
    let s = build(&cut);
    assert!(close(s.volume(), want), "{}", s.volume());
    watertight(&s);
}
