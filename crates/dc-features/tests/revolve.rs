//! Full revolves of profiles made of lines parallel and square to the axis: exact volumes and
//! areas, the placement of the body in space, the faces it gets, and what later features can do
//! to it.

use core::f64::consts::PI;
use dc_features::{build_solid, revolve, StepError, Stepped};
use dc_kernel::{Mesh, Solid, TessParams};
use dc_math::{Plane, Tolerance, Vec2, Vec3};
use dc_sketch::{Entity, Profile, Sketch};

fn plane(x: Vec3, y: Vec3) -> Plane {
    Plane::new(Vec3::ZERO, x, y, Tolerance::DEFAULT).unwrap()
}

fn polygon(pts: &[(f64, f64)]) -> Profile {
    let mut s = Sketch::new();
    for (i, (x, y)) in pts.iter().enumerate() {
        s.add_entity(&format!("p{i}"), Entity::Point { x: *x, y: *y })
            .unwrap();
    }
    for i in 0..pts.len() {
        s.add_entity(
            &format!("e{i}"),
            Entity::Line {
                a: format!("p{i}"),
                b: format!("p{}", (i + 1) % pts.len()),
            },
        )
        .unwrap();
    }
    let r = s.profiles();
    assert!(r.problems.is_empty(), "{:?}", r.problems);
    assert_eq!(r.profiles.len(), 1);
    r.profiles.into_iter().next().unwrap()
}

fn close(a: f64, b: f64) -> bool {
    (a - b).abs() <= 1e-7 * b.abs().max(1.0)
}

fn watertight(s: &Solid) -> Mesh {
    let m = s.tessellate(TessParams::default()).unwrap();
    let p = m.check();
    assert!(p.is_empty(), "not watertight: {p:?}");
    m
}

fn about_x(profile: &Profile) -> Result<Stepped, StepError> {
    revolve(
        &plane(Vec3::X, Vec3::Y),
        profile,
        Vec2::new(0.0, 0.0),
        Vec2::new(1.0, 0.0),
        "rev",
    )
}

fn names(s: &Solid) -> Vec<String> {
    let mut v: Vec<String> = s.faces.iter().map(|f| f.name.clone()).collect();
    v.sort();
    v
}

#[test]
fn a_rectangle_beside_the_axis_makes_a_tube_and_one_touching_it_a_rod() {
    // x from 0 to 10, y (radius) from 2 to 5.
    let tube = polygon(&[(0.0, 2.0), (10.0, 2.0), (10.0, 5.0), (0.0, 5.0)]);
    let st = about_x(&tube).unwrap();
    let s = build_solid(&st).unwrap();
    assert!(
        close(s.volume(), PI * (25.0 - 4.0) * 10.0),
        "{}",
        s.volume()
    );
    // Lateral areas plus two rings.
    assert!(close(
        s.area(),
        2.0 * PI * 5.0 * 10.0 + 2.0 * PI * 2.0 * 10.0 + 2.0 * PI * (25.0 - 4.0)
    ));
    let n = names(&s);
    assert!(
        n.contains(&"cap_start".to_owned()) && n.contains(&"cap_end".to_owned()),
        "{n:?}"
    );
    assert_eq!(
        n.iter().filter(|f| f.starts_with("side:")).count(),
        2,
        "{n:?}"
    );
    watertight(&s);
    // Touching the axis: a solid rod.
    let rod = polygon(&[(0.0, 0.0), (10.0, 0.0), (10.0, 4.0), (0.0, 4.0)]);
    let s = build_solid(&about_x(&rod).unwrap()).unwrap();
    assert!(close(s.volume(), PI * 16.0 * 10.0), "{}", s.volume());
    assert_eq!(
        s.faces.len(),
        3,
        "a cylinder and two discs: {:?}",
        names(&s)
    );
    watertight(&s);
}

#[test]
fn a_stepped_shaft_and_a_bushing_with_a_bore_have_the_exact_volume_and_faces() {
    let shaft = polygon(&[
        (0.0, 0.0),
        (30.0, 0.0),
        (30.0, 5.0),
        (25.0, 5.0),
        (25.0, 3.0),
        (10.0, 3.0),
        (10.0, 4.0),
        (0.0, 4.0),
    ]);
    let s = build_solid(&about_x(&shaft).unwrap()).unwrap();
    assert!(
        close(s.volume(), PI * (16.0 * 10.0 + 9.0 * 15.0 + 25.0 * 5.0)),
        "{}",
        s.volume()
    );
    // Three cylinders, two end discs and two shoulders.
    assert_eq!(
        s.faces
            .iter()
            .filter(|f| f.name.starts_with("side:"))
            .count(),
        3
    );
    assert_eq!(s.faces.len(), 3 + 2 + 2, "{:?}", names(&s));
    watertight(&s);
    // A bushing: outer radius 5 and length 20, bore 2 for 8 then 3 for the rest.
    let bushing = polygon(&[
        (0.0, 2.0),
        (8.0, 2.0),
        (8.0, 3.0),
        (20.0, 3.0),
        (20.0, 5.0),
        (0.0, 5.0),
    ]);
    let s = build_solid(&about_x(&bushing).unwrap()).unwrap();
    assert!(
        close(s.volume(), PI * (25.0 * 20.0 - 4.0 * 8.0 - 9.0 * 12.0)),
        "{}",
        s.volume()
    );
    watertight(&s);
}

#[test]
fn the_body_stands_where_the_axis_is_in_space() {
    // A sketch on the xz plane (x right, z up) revolved about its own y axis (world z).
    let rod = polygon(&[(0.0, 0.0), (3.0, 0.0), (3.0, 10.0), (0.0, 10.0)]);
    let st = revolve(
        &plane(Vec3::X, Vec3::Z),
        &rod,
        Vec2::new(0.0, 0.0),
        Vec2::new(0.0, 1.0),
        "rev",
    )
    .unwrap();
    let s = build_solid(&st).unwrap();
    let b = s.bounds().unwrap();
    assert!(close(b.min.z, 0.0) && close(b.max.z, 10.0), "{b:?}");
    assert!(close(b.max.x, 3.0) && close(b.min.x, -3.0), "{b:?}");
    assert!(close(b.max.y, 3.0) && close(b.min.y, -3.0), "{b:?}");
    assert!(close(s.volume(), PI * 9.0 * 10.0));
    // The axis need not pass through the origin: shift the rod and the axis together.
    let shifted = polygon(&[(5.0, 7.0), (8.0, 7.0), (8.0, 17.0), (5.0, 17.0)]);
    let st = revolve(
        &plane(Vec3::X, Vec3::Z),
        &shifted,
        Vec2::new(5.0, 7.0),
        Vec2::new(0.0, 1.0),
        "rev",
    )
    .unwrap();
    let b = build_solid(&st).unwrap().bounds().unwrap();
    assert!(close(b.min.z, 7.0) && close(b.max.z, 17.0), "{b:?}");
    assert!(close(b.min.x, 2.0) && close(b.max.x, 8.0), "{b:?}");
    // And the other way round (the axis pointing down): same body.
    let flipped = revolve(
        &plane(Vec3::X, Vec3::Z),
        &shifted,
        Vec2::new(5.0, 17.0),
        Vec2::new(0.0, -1.0),
        "rev",
    )
    .unwrap();
    assert!(close(
        build_solid(&flipped).unwrap().volume(),
        PI * 9.0 * 10.0
    ));
}

#[test]
fn what_cannot_be_revolved_is_refused_with_the_reason() {
    let msg = |e: StepError| e.to_string();
    // A slanted edge (a cone).
    let cone = polygon(&[(0.0, 0.0), (10.0, 0.0), (10.0, 3.0), (0.0, 5.0)]);
    let e = about_x(&cone).unwrap_err();
    assert!(matches!(e, StepError::Unsupported(_)) && msg(e).contains("cone"));
    // An arc (a sphere or torus).
    let mut s = Sketch::new();
    for (id, x, y) in [("c", 5.0, 3.0), ("a", 7.0, 3.0), ("b", 3.0, 3.0)] {
        s.add_entity(id, Entity::Point { x, y }).unwrap();
    }
    s.add_entity(
        "arc",
        Entity::Arc {
            center: "c".into(),
            start: "a".into(),
            end: "b".into(),
        },
    )
    .unwrap();
    s.add_entity(
        "chord",
        Entity::Line {
            a: "b".into(),
            b: "a".into(),
        },
    )
    .unwrap();
    let half = s.profiles().profiles.into_iter().next().unwrap();
    let e = about_x(&half).unwrap_err();
    assert!(matches!(e, StepError::Unsupported(_)) && msg(e).contains("sphere"));
    // A profile across the axis.
    let across = polygon(&[(0.0, -2.0), (10.0, -2.0), (10.0, 3.0), (0.0, 3.0)]);
    let e = about_x(&across).unwrap_err();
    assert!(msg(e).contains("crosses the axis"));
    // No direction.
    let ok = polygon(&[(0.0, 1.0), (10.0, 1.0), (10.0, 3.0), (0.0, 3.0)]);
    let e = revolve(
        &plane(Vec3::X, Vec3::Y),
        &ok,
        Vec2::new(0.0, 0.0),
        Vec2::new(0.0, 0.0),
        "rev",
    )
    .unwrap_err();
    assert!(msg(e).contains("no direction"));
}

#[test]
fn a_revolved_part_takes_axial_cuts_and_bosses_like_any_other() {
    let rod = polygon(&[(0.0, 0.0), (20.0, 0.0), (20.0, 6.0), (0.0, 6.0)]);
    let st = about_x(&rod).unwrap();
    // A hole of radius 2 drilled from the start end, 8 deep, in the model's own frame
    // (where the axis is the stack's axis and the section is the circle round the origin).
    let hole = {
        let mut s = Sketch::new();
        s.add_entity("m", Entity::Point { x: 0.0, y: 0.0 }).unwrap();
        s.add_entity(
            "k",
            Entity::Circle {
                center: "m".into(),
                radius: 2.0,
            },
        )
        .unwrap();
        s.profiles().profiles.into_iter().next().unwrap()
    };
    let drilled = st.cut(&hole, 0.0, 8.0, "drill").unwrap();
    let want = PI * 36.0 * 20.0 - PI * 4.0 * 8.0;
    let s = build_solid(&drilled).unwrap();
    assert!(close(s.volume(), want), "{}", s.volume());
    watertight(&s);
    // A boss: a stub of radius 3 and length 4 out of the far end.
    let stub = {
        let mut s = Sketch::new();
        s.add_entity("m", Entity::Point { x: 0.0, y: 0.0 }).unwrap();
        s.add_entity(
            "k",
            Entity::Circle {
                center: "m".into(),
                radius: 3.0,
            },
        )
        .unwrap();
        s.profiles().profiles.into_iter().next().unwrap()
    };
    let with_stub = drilled.boss(&stub, 20.0, 24.0, "stub").unwrap();
    let s = build_solid(&with_stub).unwrap();
    assert!(close(s.volume(), want + PI * 9.0 * 4.0), "{}", s.volume());
    watertight(&s);
}
