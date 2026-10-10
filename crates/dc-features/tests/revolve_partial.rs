//! Revolves by less than a full turn: exact volumes, valid solids, watertight meshes.

use dc_features::revolve_partial;
use dc_kernel::{Solid, TessParams};
use dc_math::{Plane, Tolerance, Vec2, Vec3};
use dc_sketch::{Entity, Profile, Sketch};

/// A sketch plane that contains the world z axis: x along world x, y along world z.
fn xz() -> Plane {
    Plane::new(Vec3::ZERO, Vec3::X, Vec3::Z, Tolerance::DEFAULT).unwrap()
}

fn rect(x0: f64, y0: f64, x1: f64, y1: f64) -> Profile {
    let mut s = Sketch::new();
    for (i, (x, y)) in [(x0, y0), (x1, y0), (x1, y1), (x0, y1)].iter().enumerate() {
        s.add_entity(&format!("p{i}"), Entity::Point { x: *x, y: *y })
            .unwrap();
    }
    for i in 0..4 {
        s.add_entity(
            &format!("e{i}"),
            Entity::Line {
                a: format!("p{i}"),
                b: format!("p{}", (i + 1) % 4),
            },
        )
        .unwrap();
    }
    s.profiles().profiles.into_iter().next().unwrap()
}

fn check(s: &Solid, want: f64) {
    assert!(s.validate().is_empty(), "{:?}", s.validate());
    assert!(
        (s.volume() - want).abs() < 1e-6 * want,
        "volume {} vs {want}",
        s.volume()
    );
    let m = s.tessellate(TessParams::default()).expect("tessellates");
    assert!(m.check().is_empty(), "{:?}", m.check());
    assert!(
        (m.volume() - want).abs() / want < 8e-3,
        "mesh {} vs {want}",
        m.volume()
    );
    let names: std::collections::BTreeSet<&String> = s.faces.iter().map(|f| &f.name).collect();
    assert_eq!(names.len(), s.faces.len(), "unique names");
}

// The axis is the sketch's y axis (world z): origin (0, 0), direction (0, 1).
fn axis() -> (Vec2, Vec2) {
    (Vec2::new(0.0, 0.0), Vec2::new(0.0, 1.0))
}

#[test]
fn a_ring_sector_of_a_quarter_turn() {
    let (o, d) = axis();
    for deg in [90.0, 180.0, 270.0, 30.0] {
        let s = revolve_partial(&xz(), &rect(5.0, 0.0, 10.0, 20.0), o, d, deg).unwrap();
        check(&s, deg.to_radians() / 2.0 * (100.0 - 25.0) * 20.0);
        // Two end caps, the outer and inner walls, and the bottom and top sectors.
        assert_eq!(s.faces.len(), 6, "{deg}");
    }
}

#[test]
fn a_profile_touching_the_axis_gives_a_solid_wedge() {
    let (o, d) = axis();
    let s = revolve_partial(&xz(), &rect(0.0, 0.0, 8.0, 12.0), o, d, 120.0).unwrap();
    check(&s, 120.0f64.to_radians() / 2.0 * 64.0 * 12.0);
    // The side on the axis makes no face: two caps, the wall, and the bottom and top sectors.
    assert_eq!(s.faces.len(), 5);
}

#[test]
fn the_same_profile_turned_the_other_way_round_the_axis_is_the_mirror_image() {
    let (o, d) = axis();
    let a = revolve_partial(&xz(), &rect(5.0, 0.0, 10.0, 20.0), o, d, 90.0).unwrap();
    let b = revolve_partial(
        &xz(),
        &rect(5.0, 0.0, 10.0, 20.0),
        o,
        Vec2::new(0.0, -1.0),
        90.0,
    )
    .unwrap();
    check(&b, a.volume());
    // Opposite axis direction turns the other way: the bounding boxes differ.
    let (ba, bb) = (a.bounds().unwrap(), b.bounds().unwrap());
    assert!((ba.min - bb.min).length() > 1.0 || (ba.max - bb.max).length() > 1.0);
}

#[test]
fn what_cannot_be_revolved_by_a_partial_turn_is_refused() {
    let (o, d) = axis();
    let e = revolve_partial(&xz(), &rect(5.0, 0.0, 10.0, 20.0), o, d, 360.0).unwrap_err();
    assert!(e.to_string().contains("less than 360"), "{e}");
    let e = revolve_partial(&xz(), &rect(5.0, 0.0, 10.0, 20.0), o, d, 0.0).unwrap_err();
    assert!(e.to_string().contains("more than 0"), "{e}");
    // A profile across the axis.
    let e = revolve_partial(&xz(), &rect(-5.0, 0.0, 5.0, 10.0), o, d, 90.0).unwrap_err();
    assert!(e.to_string().contains("crosses the axis"), "{e}");
    // A slanted edge.
    let mut s = Sketch::new();
    for (i, (x, y)) in [(5.0, 0.0), (10.0, 0.0), (8.0, 10.0)].iter().enumerate() {
        s.add_entity(&format!("p{i}"), Entity::Point { x: *x, y: *y })
            .unwrap();
    }
    for i in 0..3 {
        s.add_entity(
            &format!("e{i}"),
            Entity::Line {
                a: format!("p{i}"),
                b: format!("p{}", (i + 1) % 3),
            },
        )
        .unwrap();
    }
    let tri = s.profiles().profiles.into_iter().next().unwrap();
    let e = revolve_partial(&xz(), &tri, o, d, 90.0).unwrap_err();
    assert!(e.to_string().contains("slanted"), "{e}");
}
