//! Extrusions are checked against exact analytic answers, in both directions, on several planes.

use core::f64::consts::PI;
use dc_features::{extrude, ExtrudeError};
use dc_kernel::{IssueCode, Solid};
use dc_math::Vec3;
use dc_sketch::{Entity, PlaneSpec, Profile, Sketch};

fn pt(s: &mut Sketch, id: &str, x: f64, y: f64) {
    s.add_entity(id, Entity::Point { x, y }).unwrap();
}
fn line(s: &mut Sketch, id: &str, a: &str, b: &str) {
    s.add_entity(
        id,
        Entity::Line {
            a: a.into(),
            b: b.into(),
        },
    )
    .unwrap();
}
fn circle(s: &mut Sketch, id: &str, center: &str, r: f64) {
    s.add_entity(
        id,
        Entity::Circle {
            center: center.into(),
            radius: r,
        },
    )
    .unwrap();
}
fn rect(s: &mut Sketch, p: &str, x0: f64, y0: f64, x1: f64, y1: f64) {
    for (k, (x, y)) in [(x0, y0), (x1, y0), (x1, y1), (x0, y1)].iter().enumerate() {
        pt(s, &format!("{p}{}", k + 1), *x, *y);
    }
    for k in 0..4 {
        line(
            s,
            &format!("{p}{}", (b'a' + k as u8) as char),
            &format!("{p}{}", k + 1),
            &format!("{p}{}", (k + 1) % 4 + 1),
        );
    }
}
fn only_profile(s: &Sketch) -> Profile {
    let r = s.profiles();
    assert!(r.problems.is_empty(), "{:?}", r.problems);
    assert_eq!(
        r.profiles.len(),
        1,
        "{:?}",
        r.profiles.iter().map(|p| &p.id).collect::<Vec<_>>()
    );
    r.profiles.into_iter().next().unwrap()
}
fn build(s: &Sketch, depth: f64, reverse: bool) -> Solid {
    extrude(&s.named_plane().unwrap(), &only_profile(s), depth, reverse)
        .unwrap_or_else(|e| panic!("{e}"))
}
fn close(a: f64, b: f64) -> bool {
    (a - b).abs() <= 1e-9 * b.abs().max(1.0)
}
fn counts(s: &Solid) -> (usize, usize, usize) {
    (s.vertices.len(), s.edges.len(), s.faces.len())
}
fn assert_bounds(s: &Solid, min: [f64; 3], max: [f64; 3]) {
    let b = s.bounds().unwrap();
    let got = [b.min.x, b.min.y, b.min.z, b.max.x, b.max.y, b.max.z];
    let want = [min[0], min[1], min[2], max[0], max[1], max[2]];
    for (g, w) in got.iter().zip(want) {
        assert!((g - w).abs() < 1e-9, "bounds {got:?} != {want:?}");
    }
}

#[test]
fn box_has_exact_topology_volume_area_and_bounds() {
    let mut s = Sketch::new();
    rect(&mut s, "r", 0.0, 0.0, 80.0, 40.0);
    let b = build(&s, 20.0, false);
    assert!(b.validate().is_empty());
    assert_eq!(counts(&b), (8, 12, 6));
    assert!(close(b.volume(), 64_000.0), "{}", b.volume());
    assert!(close(b.area(), 11_200.0), "{}", b.area());
    assert_bounds(&b, [0.0, 0.0, 0.0], [80.0, 40.0, 20.0]);
    let mut names: Vec<_> = b.faces.iter().map(|f| f.name.as_str()).collect();
    names.sort_unstable();
    assert_eq!(
        names,
        [
            "cap_end",
            "cap_start",
            "side:ra",
            "side:rb",
            "side:rc",
            "side:rd"
        ]
    );
}

#[test]
fn reverse_extrudes_the_other_way_with_identical_measures() {
    let mut s = Sketch::new();
    rect(&mut s, "r", 0.0, 0.0, 80.0, 40.0);
    let (f, r) = (build(&s, 20.0, false), build(&s, 20.0, true));
    assert!(r.validate().is_empty());
    assert!(close(r.volume(), f.volume()) && close(r.area(), f.area()));
    assert_bounds(&r, [0.0, 0.0, -20.0], [80.0, 40.0, 0.0]);
}

#[test]
fn a_circle_makes_a_cylinder() {
    let mut s = Sketch::new();
    pt(&mut s, "c", 5.0, -3.0);
    circle(&mut s, "k", "c", 10.0);
    for reverse in [false, true] {
        let c = build(&s, 30.0, reverse);
        assert!(c.validate().is_empty());
        assert_eq!(counts(&c), (2, 3, 3), "V−E+F = 2");
        assert!(close(c.volume(), PI * 100.0 * 30.0), "{}", c.volume());
        assert!(
            close(c.area(), 2.0 * PI * 100.0 + 2.0 * PI * 10.0 * 30.0),
            "{}",
            c.area()
        );
        let (z0, z1) = if reverse { (-30.0, 0.0) } else { (0.0, 30.0) };
        assert_bounds(&c, [-5.0, -13.0, z0], [15.0, 7.0, z1]);
    }
}

#[test]
fn a_hole_makes_a_genus_one_solid() {
    let mut s = Sketch::new();
    rect(&mut s, "r", 0.0, 0.0, 100.0, 100.0);
    pt(&mut s, "c", 50.0, 50.0);
    circle(&mut s, "h", "c", 20.0);
    for reverse in [false, true] {
        let b = build(&s, 10.0, reverse);
        assert!(b.validate().is_empty());
        // V−E+F−R = 10−15+7−2 = 0 = 2(1−G): one handle.
        assert_eq!(counts(&b), (10, 15, 7));
        let base = 10_000.0 - PI * 400.0;
        assert!(close(b.volume(), base * 10.0), "{}", b.volume());
        assert!(
            close(b.area(), 2.0 * base + 400.0 * 10.0 + 2.0 * PI * 20.0 * 10.0),
            "{}",
            b.area()
        );
        // The hole's wall faces inward: its face is the only one with opposite sense.
        let hole = b.faces.iter().find(|f| f.name == "side:h").unwrap();
        assert!(!hole.same_sense);
    }
}

#[test]
fn an_annulus_and_multiple_holes() {
    let mut s = Sketch::new();
    pt(&mut s, "c", 0.0, 0.0);
    circle(&mut s, "outer", "c", 10.0);
    circle(&mut s, "inner", "c", 4.0);
    let a = build(&s, 5.0, false);
    assert!(a.validate().is_empty());
    assert!(close(a.volume(), PI * (100.0 - 16.0) * 5.0));

    let mut m = Sketch::new();
    rect(&mut m, "r", 0.0, 0.0, 100.0, 50.0);
    for (i, x) in [20.0, 50.0, 80.0].iter().enumerate() {
        pt(&mut m, &format!("c{i}"), *x, 25.0);
        circle(&mut m, &format!("h{i}"), &format!("c{i}"), 8.0);
    }
    let b = build(&m, 12.0, true);
    assert!(b.validate().is_empty());
    assert!(close(b.volume(), (5000.0 - 3.0 * PI * 64.0) * 12.0));
}

#[test]
fn convex_arcs_use_exact_extremes_for_bounds() {
    // Upper half disc: (-10,0) .. (10,0) with the arc over the top.
    let mut s = Sketch::new();
    pt(&mut s, "c", 0.0, 0.0);
    pt(&mut s, "l", -10.0, 0.0);
    pt(&mut s, "r", 10.0, 0.0);
    s.add_entity(
        "arc",
        Entity::Arc {
            center: "c".into(),
            start: "r".into(),
            end: "l".into(),
        },
    )
    .unwrap();
    line(&mut s, "base", "l", "r");
    let d = build(&s, 4.0, false);
    assert!(d.validate().is_empty());
    assert!(close(d.volume(), PI * 50.0 * 4.0), "{}", d.volume());
    // y reaches the radius at the top of the arc, where there is no vertex.
    assert_bounds(&d, [-10.0, 0.0, 0.0], [10.0, 10.0, 4.0]);
}

#[test]
fn concave_arcs_are_traversed_against_their_natural_direction() {
    // 40 x 20 block with a semicircular notch (radius 5) dipping into the top edge.
    let mut s = Sketch::new();
    for (id, x, y) in [
        ("a", 0.0, 0.0),
        ("b", 40.0, 0.0),
        ("c", 40.0, 20.0),
        ("d", 25.0, 20.0),
        ("e", 15.0, 20.0),
        ("f", 0.0, 20.0),
        ("o", 20.0, 20.0),
    ] {
        pt(&mut s, id, x, y);
    }
    line(&mut s, "ab", "a", "b");
    line(&mut s, "bc", "b", "c");
    line(&mut s, "cd", "c", "d");
    // Counter-clockwise from e (angle π) to d (angle 0 = 2π) passes through the bottom (20, 15).
    s.add_entity(
        "notch",
        Entity::Arc {
            center: "o".into(),
            start: "e".into(),
            end: "d".into(),
        },
    )
    .unwrap();
    line(&mut s, "ef", "e", "f");
    line(&mut s, "fa", "f", "a");
    for reverse in [false, true] {
        let b = build(&s, 10.0, reverse);
        assert!(b.validate().is_empty(), "{:?}", b.validate());
        assert!(
            close(b.volume(), (800.0 - PI * 25.0 / 2.0) * 10.0),
            "{}",
            b.volume()
        );
        let notch = b.faces.iter().find(|f| f.name == "side:notch").unwrap();
        assert!(!notch.same_sense, "material is outside the notch circle");
    }
}

#[test]
fn every_plane_and_direction_gives_the_right_place_and_measures() {
    let want_volume = 30.0 * 20.0 * 7.0;
    // (plane, expected bounds for forward extrusion)
    let cases = [
        (
            PlaneSpec::Xy { offset: 0.0 },
            [0.0, 0.0, 0.0],
            [30.0, 20.0, 7.0],
        ),
        (
            PlaneSpec::Xy { offset: 5.0 },
            [0.0, 0.0, 5.0],
            [30.0, 20.0, 12.0],
        ),
        // XZ: x = X, y = Z, normal = -Y, so extruding forward goes toward -Y.
        (
            PlaneSpec::Xz { offset: 0.0 },
            [0.0, -7.0, 0.0],
            [30.0, 0.0, 20.0],
        ),
        // YZ: x = Y, y = Z, normal = +X.
        (
            PlaneSpec::Yz { offset: 2.0 },
            [2.0, 0.0, 0.0],
            [9.0, 30.0, 20.0],
        ),
    ];
    for (plane, min, max) in cases {
        let mut s = Sketch::on_plane(plane.clone()).unwrap();
        rect(&mut s, "r", 0.0, 0.0, 30.0, 20.0);
        let f = build(&s, 7.0, false);
        assert!(f.validate().is_empty(), "{plane:?}");
        assert!(close(f.volume(), want_volume), "{plane:?}: {}", f.volume());
        assert_bounds(&f, min, max);
        // Reversing flips the slab to the other side of the plane, volume unchanged.
        let r = build(&s, 7.0, true);
        assert!(r.validate().is_empty());
        assert!(close(r.volume(), want_volume));
        let n = s.named_plane().unwrap().normal();
        let (fb, rb) = (f.bounds().unwrap(), r.bounds().unwrap());
        let mid = |b: dc_kernel::Bounds| (b.min + b.max) * 0.5;
        assert!(close((mid(fb) - mid(rb)).dot(n), 7.0), "{plane:?}");
    }
}

#[test]
fn many_sizes_all_validate_and_match_the_formula() {
    for (w, h, d) in [
        (1.0, 1.0, 1.0),
        (0.5, 123.0, 0.01),
        (1000.0, 2.0, 750.0),
        (3.3, 3.3, 3.3),
    ] {
        for reverse in [false, true] {
            let mut s = Sketch::new();
            rect(&mut s, "r", -w / 2.0, -h / 2.0, w / 2.0, h / 2.0);
            let b = build(&s, d, reverse);
            assert!(b.validate().is_empty());
            assert!(close(b.volume(), w * h * d), "{w}x{h}x{d}");
            assert!(
                close(b.area(), 2.0 * (w * h + w * d + h * d)),
                "{w}x{h}x{d}"
            );
        }
    }
}

#[test]
fn bad_depths_are_rejected() {
    let mut s = Sketch::new();
    rect(&mut s, "r", 0.0, 0.0, 1.0, 1.0);
    let p = only_profile(&s);
    for d in [0.0, -1.0, f64::NAN, f64::INFINITY] {
        assert!(
            matches!(
                extrude(&s.named_plane().unwrap(), &p, d, false),
                Err(ExtrudeError::InvalidDepth(_))
            ),
            "{d}"
        );
    }
}

#[test]
fn construction_is_deterministic_byte_for_byte() {
    let mut s = Sketch::new();
    rect(&mut s, "r", 0.0, 0.0, 100.0, 100.0);
    pt(&mut s, "c", 50.0, 50.0);
    circle(&mut s, "h", "c", 20.0);
    assert_eq!(
        build(&s, 10.0, false).canonical_bytes(),
        build(&s.clone(), 10.0, false).canonical_bytes()
    );
    assert_ne!(
        build(&s, 10.0, false).canonical_bytes(),
        build(&s, 10.0, true).canonical_bytes()
    );
}

#[test]
fn the_validator_catches_each_kind_of_damage() {
    let mut s = Sketch::new();
    rect(&mut s, "r", 0.0, 0.0, 10.0, 10.0);
    let good = build(&s, 5.0, false);
    let codes =
        |solid: &Solid| -> Vec<IssueCode> { solid.validate().iter().map(|i| i.code).collect() };
    assert!(codes(&good).is_empty());

    // A coedge flipped: that edge is now used twice in the same direction.
    let mut flipped = good.clone();
    flipped.faces[2].loops[0][0].reversed ^= true;
    assert!(
        codes(&flipped).contains(&IssueCode::EdgeUse)
            || codes(&flipped).contains(&IssueCode::OpenLoop)
    );

    // A vertex moved off its faces.
    let mut moved = good.clone();
    moved.vertices[3] += Vec3::new(0.0, 0.0, 0.5);
    assert!(
        codes(&moved).contains(&IssueCode::OffSurface),
        "{:?}",
        codes(&moved)
    );

    // A face missing: the surface is no longer closed.
    let mut open = good.clone();
    open.faces.pop();
    assert!(codes(&open).contains(&IssueCode::EdgeUse));

    // Every face's sense inverted: inside out.
    let mut inside_out = good.clone();
    for f in &mut inside_out.faces {
        f.same_sense = !f.same_sense;
    }
    assert!(
        codes(&inside_out).contains(&IssueCode::Orientation),
        "{:?}",
        codes(&inside_out)
    );

    // An index out of range is reported, not a panic.
    let mut bad = good.clone();
    bad.faces[0].loops[0][0].edge = 999;
    assert_eq!(codes(&bad), [IssueCode::Malformed]);
    let mut nan = good.clone();
    nan.vertices[0] = Vec3::new(f64::NAN, 0.0, 0.0);
    assert_eq!(codes(&nan), [IssueCode::Malformed]);
}
