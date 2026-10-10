//! Rounding the circular rim of a shaft or a hole: volumes and areas against the closed forms
//! (frusta, Pappus), the validator, and a watertight tessellation whose volume agrees.

use core::f64::consts::PI;
use dc_features::{build_solid, Stepped};
use dc_kernel::{RimError, RimRound, Solid, TessParams};
use dc_math::{Plane, Tolerance, Vec3};
use dc_sketch::{Entity, Profile, Sketch};

fn xy() -> Plane {
    Plane::new(Vec3::ZERO, Vec3::X, Vec3::Y, Tolerance::DEFAULT).unwrap()
}

fn disc(cx: f64, cy: f64, r: f64) -> Profile {
    let mut s = Sketch::new();
    s.add_entity("m", Entity::Point { x: cx, y: cy }).unwrap();
    s.add_entity(
        "c",
        Entity::Circle {
            center: "m".into(),
            radius: r,
        },
    )
    .unwrap();
    s.profiles().profiles.remove(0)
}

/// A 40 x 40 plate, 10 thick, with a hole of radius `r` in the middle.
fn plate_with_hole(r: f64) -> Solid {
    let mut s = Sketch::new();
    for (i, (x, y)) in [(-20.0, -20.0), (20.0, -20.0), (20.0, 20.0), (-20.0, 20.0)]
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
    s.add_entity("m", Entity::Point { x: 0.0, y: 0.0 }).unwrap();
    s.add_entity(
        "c",
        Entity::Circle {
            center: "m".into(),
            radius: r,
        },
    )
    .unwrap();
    let p = s.profiles().profiles.remove(0);
    build_solid(&Stepped::extrusion(xy(), p, 10.0, false)).unwrap()
}

fn shaft(r: f64, h: f64) -> Solid {
    build_solid(&Stepped::extrusion(xy(), disc(0.0, 0.0, r), h, false)).unwrap()
}

fn near(a: f64, b: f64) -> bool {
    (a - b).abs() <= 1e-9 * b.abs().max(1.0)
}

fn check_mesh(s: &Solid) {
    assert!(s.validate().is_empty(), "{:?}", s.validate());
    let m = s.tessellate(TessParams::default()).expect("tessellates");
    assert!(m.check().is_empty(), "{:?}", m.check());
    // The mesh is inscribed in the true surface, so slightly smaller, but close.
    assert!(
        (m.volume() - s.volume()).abs() / s.volume() < 5e-3,
        "mesh {} exact {}",
        m.volume(),
        s.volume()
    );
}

#[test]
fn chamfering_the_top_and_bottom_rim_of_a_shaft_matches_the_frustum() {
    let (r, h, d) = (10.0, 20.0, 2.0);
    let base = shaft(r, h);
    let frustum = PI * d / 3.0 * (r * r + r * (r - d) + (r - d) * (r - d));
    let want_v = PI * r * r * h - (PI * r * r * d - frustum);
    let want_a = PI * (r - d) * (r - d)
        + PI * (2.0 * r - d) * d * 2.0f64.sqrt()
        + 2.0 * PI * r * (h - d)
        + PI * r * r;
    for cap in ["cap_end", "cap_start"] {
        let s = base
            .round_rim(cap, "side:c", RimRound::Chamfer, d, "round:a")
            .unwrap_or_else(|e| panic!("{cap}: {e:?}"));
        assert!(
            near(s.volume(), want_v),
            "{cap}: {} vs {want_v}",
            s.volume()
        );
        assert!(near(s.area(), want_a), "{cap}: {} vs {want_a}", s.area());
        check_mesh(&s);
        // The faces are what they should be.
        let names: Vec<&str> = s.faces.iter().map(|f| f.name.as_str()).collect();
        assert!(names.contains(&"round:a"), "{names:?}");
    }
    // Both ends, one after the other.
    let both = base
        .round_rim("cap_end", "side:c", RimRound::Chamfer, d, "round:a")
        .unwrap()
        .round_rim("cap_start", "side:c", RimRound::Chamfer, d, "round:b")
        .unwrap();
    assert!(near(
        both.volume(),
        PI * r * r * h - 2.0 * (PI * r * r * d - frustum)
    ));
    check_mesh(&both);
}

#[test]
fn filleting_the_rim_of_a_shaft_matches_pappus_and_the_torus_area() {
    let (r, h, d) = (10.0, 20.0, 2.0);
    let base = shaft(r, h);
    let removed =
        2.0 * PI * (d * d * r - d * d * d / 2.0 - PI * d * d / 4.0 * (r - d) - d * d * d / 3.0);
    let torus = 2.0 * PI * d * ((r - d) * PI / 2.0 + d);
    let want_a = PI * (r - d) * (r - d) + torus + 2.0 * PI * r * (h - d) + PI * r * r;
    for cap in ["cap_end", "cap_start"] {
        let s = base
            .round_rim(cap, "side:c", RimRound::Fillet, d, "round:a")
            .unwrap_or_else(|e| panic!("{cap}: {e:?}"));
        assert!(
            near(s.volume(), PI * r * r * h - removed),
            "{cap}: {}",
            s.volume()
        );
        assert!(near(s.area(), want_a), "{cap}: {} vs {want_a}", s.area());
        check_mesh(&s);
    }
}

#[test]
fn the_mouth_of_a_hole_can_be_chamfered_and_filleted() {
    let (r, d) = (5.0, 1.5);
    let base = plate_with_hole(r);
    let v0 = base.volume();
    for cap in ["cap_end", "cap_start"] {
        let c = base
            .round_rim(cap, "side:c", RimRound::Chamfer, d, "round:a")
            .unwrap_or_else(|e| panic!("{cap}: {e:?}"));
        let frustum = PI * d / 3.0 * (r * r + r * (r + d) + (r + d) * (r + d));
        assert!(
            near(c.volume(), v0 - (frustum - PI * r * r * d)),
            "{cap}: {}",
            c.volume()
        );
        check_mesh(&c);
        let f = base
            .round_rim(cap, "side:c", RimRound::Fillet, d, "round:a")
            .unwrap_or_else(|e| panic!("{cap}: {e:?}"));
        let removed =
            2.0 * PI * (d * d * (r + d / 2.0) - PI * d * d / 4.0 * (r + d) + d * d * d / 3.0);
        assert!(
            near(f.volume(), v0 - removed),
            "{cap}: {} vs {}",
            f.volume(),
            v0 - removed
        );
        check_mesh(&f);
    }
}

#[test]
fn what_cannot_be_rounded_is_refused_with_the_reason() {
    let base = shaft(10.0, 20.0);
    let e = base
        .round_rim("cap_end", "side:c", RimRound::Chamfer, 25.0, "r")
        .unwrap_err();
    assert!(matches!(e, RimError::TooLarge(_)), "{e:?}");
    let e = base
        .round_rim("cap_end", "side:c", RimRound::Fillet, 10.0, "r")
        .unwrap_err();
    assert!(matches!(e, RimError::TooLarge(_)), "{e:?}");
    let e = base
        .round_rim("cap_end", "cap_start", RimRound::Chamfer, 1.0, "r")
        .unwrap_err();
    assert!(matches!(e, RimError::Unsupported(_)), "{e:?}");
    let e = base
        .round_rim("cap_end", "nope", RimRound::Chamfer, 1.0, "r")
        .unwrap_err();
    assert!(matches!(e, RimError::NotFound(_)), "{e:?}");
    // The edge cannot be rounded twice: its faces no longer meet.
    let once = base
        .round_rim("cap_end", "side:c", RimRound::Chamfer, 1.0, "r")
        .unwrap();
    assert!(once
        .round_rim("cap_end", "side:c", RimRound::Chamfer, 1.0, "r2")
        .is_err());
    // A hole whose mouth would reach the plate's edge.
    let wide = plate_with_hole(18.0);
    let e = wide
        .round_rim("cap_end", "side:c", RimRound::Chamfer, 3.0, "r")
        .unwrap_err();
    assert!(matches!(e, RimError::TooLarge(_)), "{e:?}");
}
