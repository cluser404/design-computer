//! Cuts and bosses from planes the layered model cannot hold: a sketch on a side face, a tilted
//! sketch plane, a part that already has a hole across its axis. Volumes are exact closed forms.

use core::f64::consts::PI;
use dc_features::extrude;
use dc_features::solid_ops::{boss_with_tool, cut_with_tool, reach, tool_solid, ToolError};
use dc_kernel::boolean::BoolError;
use dc_kernel::Solid;
use dc_math::{det, Plane, Tolerance, Vec3};
use dc_sketch::{Entity, Profile, Sketch};

fn xy() -> Plane {
    Plane::new(Vec3::ZERO, Vec3::X, Vec3::Y, Tolerance::DEFAULT).unwrap()
}

fn profile(build: impl FnOnce(&mut Sketch)) -> Profile {
    let mut s = Sketch::new();
    build(&mut s);
    s.profiles().profiles.into_iter().next().unwrap()
}

fn rect(x0: f64, y0: f64, x1: f64, y1: f64) -> Profile {
    profile(|s| {
        let c = [(x0, y0), (x1, y0), (x1, y1), (x0, y1)];
        for (i, (x, y)) in c.iter().enumerate() {
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

fn block() -> Solid {
    extrude(&xy(), &rect(0.0, 0.0, 40.0, 20.0), 12.0, false).unwrap()
}

/// The sketch plane on the face x = 40, looking out of the part: its x axis is world y, its y
/// axis world z, its normal world x. The origin is at the face's centre.
fn east_face() -> Plane {
    Plane::new(
        Vec3::new(40.0, 10.0, 6.0),
        Vec3::Y,
        Vec3::Z,
        Tolerance::DEFAULT,
    )
    .unwrap()
}

fn vol(s: &Solid, want: f64) {
    assert!(
        (s.volume() - want).abs() < 1e-6 * want,
        "volume {} vs {want}",
        s.volume()
    );
}

#[test]
fn a_blind_hole_in_a_side_face() {
    let b = block();
    let plane = east_face();
    // Into the part: against the plane's normal.
    let tool = tool_solid(&plane, &disc(0.0, 0.0, 4.0), -15.0, 0.0).unwrap();
    let r = cut_with_tool(&b, &tool, "hole").unwrap();
    assert!(r.validate().is_empty(), "{:?}", r.validate());
    vol(&r, 9600.0 - PI * 16.0 * 15.0);
    let names: Vec<&str> = r.faces.iter().map(|f| f.name.as_str()).collect();
    assert!(
        names.iter().any(|n| n.starts_with("side:cut:hole:")),
        "{names:?}"
    );
    assert!(
        names.contains(&"cap_start:hole") || names.contains(&"cap_end:hole"),
        "{names:?}"
    );
}

#[test]
fn a_hole_all_the_way_through_the_part_sideways() {
    let b = block();
    let plane = east_face();
    let l = reach(&b, &plane);
    let tool = tool_solid(&plane, &disc(0.0, 0.0, 4.0), -l, 0.0).unwrap();
    let r = cut_with_tool(&b, &tool, "hole").unwrap();
    assert!(r.validate().is_empty());
    vol(&r, 9600.0 - PI * 16.0 * 40.0);
    // Two faces lose a circular hole each, one wall.
    assert_eq!(r.faces.len(), 7);
}

#[test]
fn a_slot_cut_from_a_side_face() {
    let b = block();
    let plane = east_face();
    let tool = tool_solid(&plane, &rect(-5.0, -3.0, 5.0, 3.0), -10.0, 0.0).unwrap();
    let r = cut_with_tool(&b, &tool, "slot").unwrap();
    vol(&r, 9600.0 - 10.0 * 6.0 * 10.0);
}

#[test]
fn a_boss_on_a_side_face() {
    let b = block();
    let plane = east_face();
    let tool = tool_solid(&plane, &disc(0.0, 0.0, 4.0), 0.0, 8.0).unwrap();
    let r = boss_with_tool(&b, &tool, "peg").unwrap();
    vol(&r, 9600.0 + PI * 16.0 * 8.0);
    assert!(r.validate().is_empty());
}

#[test]
fn a_floating_boss_is_refused() {
    let b = block();
    let plane = east_face();
    let tool = tool_solid(&plane, &disc(0.0, 0.0, 4.0), 3.0, 8.0).unwrap();
    match boss_with_tool(&b, &tool, "peg") {
        Err(ToolError::Floating(_)) => {}
        other => panic!("{other:?}"),
    }
}

#[test]
fn a_cut_that_misses_the_part_is_refused() {
    let b = block();
    let plane = east_face();
    let tool = tool_solid(&plane, &disc(0.0, 0.0, 4.0), 2.0, 8.0).unwrap();
    assert!(matches!(
        cut_with_tool(&b, &tool, "x"),
        Err(ToolError::NoEffect(_))
    ));
}

#[test]
fn a_cut_from_a_tilted_plane() {
    // A sketch plane tilted 30 degrees about the world y axis through the middle of the top face.
    let a = PI / 6.0;
    let (c, s) = (det::cos(a), det::sin(a));
    let plane = Plane::new(
        Vec3::new(20.0, 10.0, 12.0),
        Vec3::new(c, 0.0, -s),
        Vec3::Y,
        Tolerance::DEFAULT,
    )
    .unwrap();
    let b = block();
    // Normal is x × y = (c,0,-s) × (0,1,0) = (s, 0, c): pointing up and a little to +x.
    let tool = tool_solid(&plane, &rect(-4.0, -4.0, 4.0, 4.0), -6.0, 0.0).unwrap();
    let r = cut_with_tool(&b, &tool, "tilt").unwrap();
    assert!(r.validate().is_empty(), "{:?}", r.validate());
    assert!(r.volume() < 9600.0);
    let m = r.tessellate(dc_kernel::TessParams::default()).unwrap();
    assert!(m.check().is_empty());
}

#[test]
fn a_second_cut_that_crosses_the_first_round_hole_is_refused_with_the_reason() {
    let b = block();
    let east = east_face();
    let l = reach(&b, &east);
    let sideways = tool_solid(&east, &disc(0.0, 0.0, 4.0), -l, 0.0).unwrap();
    let drilled = cut_with_tool(&b, &sideways, "h1").unwrap();
    // A vertical hole through the middle crosses the horizontal one.
    let top = Plane::new(
        Vec3::new(20.0, 10.0, 12.0),
        Vec3::X,
        Vec3::Y,
        Tolerance::DEFAULT,
    )
    .unwrap();
    let down = tool_solid(&top, &disc(0.0, 0.0, 3.0), -l, 0.0).unwrap();
    match cut_with_tool(&drilled, &down, "h2") {
        Err(ToolError::Bool(BoolError::Unsupported(why))) => {
            assert!(why.contains("cross"), "{why}")
        }
        other => panic!("{other:?}"),
    }
}
