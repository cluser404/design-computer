//! Shelling prism-shaped parts: exact volumes, valid solids, watertight meshes, and refusals.

use core::f64::consts::PI;
use dc_features::{extrude, shell_prism, ShellError};
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

fn rect(w: f64, h: f64) -> Profile {
    profile(|s| {
        for (i, (x, y)) in [(0.0, 0.0), (w, 0.0), (w, h), (0.0, h)].iter().enumerate() {
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
    })
}

fn disc(r: f64) -> Profile {
    profile(|s| {
        s.add_entity("m", Entity::Point { x: 0.0, y: 0.0 }).unwrap();
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
                radius: 5.0,
            },
        )
        .unwrap();
    })
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
    assert_eq!(names.len(), s.faces.len(), "unique face names");
}

fn block() -> Solid {
    extrude(&xy(), &rect(40.0, 20.0), 10.0, false).unwrap()
}

#[test]
fn an_open_tray() {
    let s = shell_prism(&block(), &["cap_end".into()], 2.0, "sh").unwrap();
    check(&s, 8000.0 - 36.0 * 16.0 * 8.0);
    assert!(s.faces.iter().any(|f| f.name.starts_with("side:shell:sh:")));
    // Outer: 6 faces; cavity: floor and four walls.
    assert_eq!(s.faces.len(), 6 + 5);
}

#[test]
fn a_closed_hollow_part_has_an_inner_shell() {
    let s = shell_prism(&block(), &[], 2.0, "sh").unwrap();
    check(&s, 8000.0 - 36.0 * 16.0 * 6.0);
    assert_eq!(s.shell_count(), 2, "the outside and the void");
}

#[test]
fn a_tube_open_at_both_ends() {
    let s = shell_prism(&block(), &["cap_start".into(), "cap_end".into()], 2.0, "sh").unwrap();
    check(&s, 8000.0 - 36.0 * 16.0 * 10.0);
}

#[test]
fn a_round_part_keeps_a_round_cavity() {
    let cyl = extrude(&xy(), &disc(10.0), 20.0, false).unwrap();
    let s = shell_prism(&cyl, &["cap_end".into()], 2.0, "sh").unwrap();
    check(&s, PI * 100.0 * 20.0 - PI * 64.0 * 18.0);
}

#[test]
fn a_profile_with_a_hole_gets_a_wall_round_the_hole() {
    let part = extrude(&xy(), &plate_with_hole(), 12.0, false).unwrap();
    let s = shell_prism(&part, &["cap_end".into()], 2.0, "sh").unwrap();
    // Cavity: (36 x 26 minus a circle of radius 7), 10 deep.
    let want = (40.0 * 30.0 - PI * 25.0) * 12.0 - (36.0 * 26.0 - PI * 49.0) * 10.0;
    check(&s, want);
}

#[test]
fn what_cannot_be_shelled_is_refused_with_the_reason() {
    let e = shell_prism(&block(), &["side:e0".into()], 2.0, "x").unwrap_err();
    assert!(matches!(e, ShellError::Unsupported(_)), "{e:?}");
    let e = shell_prism(&block(), &["cap_end".into()], 12.0, "x").unwrap_err();
    assert!(matches!(e, ShellError::TooThick(_)), "{e:?}");
    let e = shell_prism(&block(), &[], 6.0, "x").unwrap_err();
    assert!(matches!(e, ShellError::TooThick(_)), "{e:?}");
    let e = shell_prism(&block(), &[], -1.0, "x").unwrap_err();
    assert!(matches!(e, ShellError::Unsupported(_)), "{e:?}");
    // A part with a step is not a prism.
    let stepped = {
        use dc_kernel::boolean::BoolOp;
        let notch = Solid::cuboid(Vec3::new(30.0, -1.0, 5.0), Vec3::new(41.0, 21.0, 11.0)).unwrap();
        block().boolean(&notch, BoolOp::Subtract).unwrap().solid
    };
    let e = shell_prism(&stepped, &["cap_end".into()], 1.0, "x").unwrap_err();
    assert!(matches!(e, ShellError::Unsupported(_)), "{e:?}");
}
