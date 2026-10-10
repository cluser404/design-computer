//! The general (trimmed) cylinder triangulation must agree with the strip path on every cylinder
//! face the extrude and the stepped model already build: watertight, outward, same volume.

use dc_features::{build_solid, Stepped};
use dc_kernel::{Solid, TessParams};
use dc_math::{Plane, Tolerance, Vec3};
use dc_sketch::{Entity, Profile, Sketch};

fn xy() -> Plane {
    Plane::new(Vec3::ZERO, Vec3::X, Vec3::Y, Tolerance::DEFAULT).unwrap()
}

fn profile(build: impl FnOnce(&mut Sketch)) -> Profile {
    let mut s = Sketch::new();
    build(&mut s);
    s.profiles().profiles.into_iter().next().unwrap()
}

fn disc(cx: f64, cy: f64, r: f64) -> Profile {
    profile(|s| {
        s.add_entity("m", Entity::Point { x: cx, y: cy }).unwrap();
        s.add_entity(
            "c",
            Entity::Circle {
                center: "m".into(),
                radius: r,
            },
        )
        .unwrap();
    })
}

fn plate_with_hole() -> Profile {
    profile(|s| {
        for (i, (x, y)) in [(0.0, 0.0), (40.0, 0.0), (40.0, 30.0), (0.0, 30.0)]
            .iter()
            .enumerate()
        {
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
        s.add_entity("m", Entity::Point { x: 20.0, y: 15.0 })
            .unwrap();
        s.add_entity(
            "h",
            Entity::Circle {
                center: "m".into(),
                radius: 6.0,
            },
        )
        .unwrap();
    })
}

fn d_shape() -> Profile {
    profile(|s| {
        for (id, x, y) in [("c", 0.0, 0.0), ("l", -10.0, 0.0), ("r", 10.0, 0.0)] {
            s.add_entity(id, Entity::Point { x, y }).unwrap();
        }
        s.add_entity(
            "arc",
            Entity::Arc {
                center: "c".into(),
                start: "r".into(),
                end: "l".into(),
            },
        )
        .unwrap();
        s.add_entity(
            "base",
            Entity::Line {
                a: "l".into(),
                b: "r".into(),
            },
        )
        .unwrap();
    })
}

fn check(s: &Solid, what: &str) {
    let strip = s.tessellate(TessParams::default()).unwrap();
    let general = s.tessellate_with(TessParams::default(), true).unwrap();
    assert!(general.check().is_empty(), "{what}: {:?}", general.check());
    assert!(general.volume() > 0.0, "{what}: outward");
    assert!(
        (general.volume() - strip.volume()).abs() <= 1e-6 * strip.volume(),
        "{what}: {} vs {}",
        general.volume(),
        strip.volume()
    );
    assert!(
        (general.area() - strip.area()).abs() <= 1e-6 * strip.area(),
        "{what}: area"
    );
}

#[test]
fn every_cylinder_the_engine_builds_triangulates_the_same_way() {
    check(
        &build_solid(&Stepped::extrusion(
            xy(),
            disc(5.0, -3.0, 10.0),
            20.0,
            false,
        ))
        .unwrap(),
        "disc",
    );
    check(
        &build_solid(&Stepped::extrusion(xy(), plate_with_hole(), 8.0, false)).unwrap(),
        "plate with a hole",
    );
    check(
        &build_solid(&Stepped::extrusion(xy(), d_shape(), 8.0, true)).unwrap(),
        "D shape, downwards",
    );
    let counterbore = Stepped::extrusion(xy(), plate_with_hole(), 10.0, false)
        .cut(&disc(20.0, 15.0, 9.0), 5.0, 10.0, "cb")
        .unwrap();
    check(&build_solid(&counterbore).unwrap(), "counterbore");
    let boss = Stepped::extrusion(xy(), plate_with_hole(), 10.0, false)
        .boss(&disc(8.0, 8.0, 4.0), 10.0, 16.0, "b")
        .unwrap();
    check(&build_solid(&boss).unwrap(), "boss");
}
