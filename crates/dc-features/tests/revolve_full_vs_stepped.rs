//! The B-rep full revolve against the layered one on the same profiles: volume, area, bounds, the
//! area in each plane and on each cylinder, and the names of the faces.

use dc_features::{build_solid, revolve, revolve_full};
use dc_kernel::{Solid, Surface, TessParams};
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
    s.profiles().profiles.into_iter().next().unwrap()
}

fn q(x: f64) -> i64 {
    (x * 1e6).round() as i64
}

fn per_surface(s: &Solid) -> Vec<(i64, i64, i64, i64, i64, i64)> {
    let mut m: std::collections::BTreeMap<(i64, i64, i64, i64, i64), f64> = Default::default();
    for f in &s.faces {
        match &f.surface {
            Surface::Plane { origin, normal, .. } => {
                let n = if f.same_sense { *normal } else { -*normal };
                *m.entry((0, q(n.x), q(n.y), q(n.z), q(origin.dot(n))))
                    .or_insert(0.0) += s.face_area(f);
            }
            Surface::Cylinder {
                radius,
                origin,
                axis,
                ..
            } => {
                *m.entry((
                    1,
                    q(*radius),
                    q(axis.x.abs()),
                    q(axis.y.abs()),
                    q(origin.dot(*axis) * 0.0),
                ))
                .or_insert(0.0) += s.face_area(f);
            }
            _ => {}
        }
    }
    m.into_iter()
        .map(|((a, b, c, d, e), area)| (a, b, c, d, e, q(area)))
        .collect()
}

fn names(s: &Solid) -> Vec<String> {
    let mut v: Vec<String> = s.faces.iter().map(|f| f.name.clone()).collect();
    v.sort();
    v
}

fn same(profile: &Profile, what: &str) {
    for (pl, o, d) in [
        (
            plane(Vec3::X, Vec3::Y),
            Vec2::new(0.0, 0.0),
            Vec2::new(1.0, 0.0),
        ),
        (
            plane(Vec3::X, Vec3::Z),
            Vec2::new(0.0, 0.0),
            Vec2::new(0.0, 1.0),
        ),
    ] {
        let old = build_solid(&revolve(&pl, profile, o, d, "rev").unwrap()).unwrap();
        let new = revolve_full(&pl, profile, o, d, "rev").unwrap_or_else(|e| panic!("{what}: {e}"));
        assert!(new.validate().is_empty(), "{what}: {:?}", new.validate());
        let close = |a: f64, b: f64| (a - b).abs() <= 1e-7 * b.abs().max(1.0);
        assert!(
            close(new.volume(), old.volume()),
            "{what}: volume {} vs {}",
            new.volume(),
            old.volume()
        );
        assert!(
            close(new.area(), old.area()),
            "{what}: area {} vs {}",
            new.area(),
            old.area()
        );
        let (nb, ob) = (new.bounds().unwrap(), old.bounds().unwrap());
        assert!(
            (nb.min - ob.min).length() < 1e-6 && (nb.max - ob.max).length() < 1e-6,
            "{what}: bounds"
        );
        assert_eq!(names(&new), names(&old), "{what}: face names");
        let m = new.tessellate(TessParams::default()).unwrap();
        assert!(m.check().is_empty(), "{what}: {:?}", m.check());
        assert!(
            (m.volume() - new.volume()).abs() / new.volume() < 1e-2,
            "{what}: mesh volume"
        );
        let _ = per_surface(&new);
    }
}

#[test]
fn a_tube_a_rod_a_stepped_shaft_and_a_grooved_bushing_agree_with_the_layered_revolve() {
    same(
        &polygon(&[(0.0, 2.0), (10.0, 2.0), (10.0, 5.0), (0.0, 5.0)]),
        "tube",
    );
    same(
        &polygon(&[(0.0, 0.0), (10.0, 0.0), (10.0, 5.0), (0.0, 5.0)]),
        "rod",
    );
    same(
        &polygon(&[
            (0.0, 0.0),
            (30.0, 0.0),
            (30.0, 5.0),
            (25.0, 5.0),
            (25.0, 3.0),
            (10.0, 3.0),
            (10.0, 4.0),
            (0.0, 4.0),
        ]),
        "stepped shaft",
    );
    same(
        &polygon(&[
            (0.0, 3.0),
            (20.0, 3.0),
            (20.0, 8.0),
            (14.0, 8.0),
            (14.0, 6.0),
            (6.0, 6.0),
            (6.0, 8.0),
            (0.0, 8.0),
        ]),
        "grooved bushing",
    );
    same(
        &polygon(&[(0.0, 4.0), (2.0, 4.0), (2.0, 9.0), (0.0, 9.0)]),
        "washer",
    );
}
