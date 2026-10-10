//! Differential test (design note `docs/kernel/boolean.md`, phase B gate): shapes the stepped
//! prism model builds are rebuilt with the general boolean and must agree on exact volume, area,
//! bounding box and the number of faces and edges. The stepped model stays the oracle until the
//! boolean matches it on the whole corpus.

use dc_features::{build_solid, Stepped};
use dc_kernel::boolean::BoolOp;
use dc_kernel::Solid;
use dc_math::{Plane, Tolerance, Vec3};
use dc_sketch::{Entity, Profile, Sketch};

fn xy() -> Plane {
    Plane::new(Vec3::ZERO, Vec3::X, Vec3::Y, Tolerance::DEFAULT).unwrap()
}

fn rect(p: &str, x0: f64, y0: f64, x1: f64, y1: f64) -> Profile {
    let mut s = Sketch::new();
    let c = [(x0, y0), (x1, y0), (x1, y1), (x0, y1)];
    for (i, (x, y)) in c.iter().enumerate() {
        s.add_entity(&format!("{p}{i}"), Entity::Point { x: *x, y: *y })
            .unwrap();
    }
    for i in 0..4 {
        s.add_entity(
            &format!("{p}e{i}"),
            Entity::Line {
                a: format!("{p}{i}"),
                b: format!("{p}{}", (i + 1) % 4),
            },
        )
        .unwrap();
    }
    s.profiles().profiles.into_iter().next().unwrap()
}

fn disc(p: &str, cx: f64, cy: f64, r: f64) -> Profile {
    let mut s = Sketch::new();
    s.add_entity(&format!("{p}m"), Entity::Point { x: cx, y: cy })
        .unwrap();
    s.add_entity(
        &format!("{p}c"),
        Entity::Circle {
            center: format!("{p}m"),
            radius: r,
        },
    )
    .unwrap();
    s.profiles().profiles.into_iter().next().unwrap()
}

fn rod(x: f64, y: f64, z0: f64, r: f64, h: f64) -> Solid {
    Solid::cylinder(Vec3::new(x, y, z0), Vec3::new(0.0, 0.0, 1.0), r, h).unwrap()
}

fn cuboid(x0: f64, y0: f64, z0: f64, x1: f64, y1: f64, z1: f64) -> Solid {
    Solid::cuboid(Vec3::new(x0, y0, z0), Vec3::new(x1, y1, z1)).unwrap()
}

fn same(stepped: &Solid, general: &Solid, what: &str) {
    assert!(
        general.validate().is_empty(),
        "{what}: {:?}",
        general.validate()
    );
    let close = |a: f64, b: f64| (a - b).abs() <= 1e-7 * b.abs().max(1.0);
    assert!(
        close(general.volume(), stepped.volume()),
        "{what}: volume {} vs {}",
        general.volume(),
        stepped.volume()
    );
    assert!(
        close(general.area(), stepped.area()),
        "{what}: area {} vs {}",
        general.area(),
        stepped.area()
    );
    let (gb, sb) = (general.bounds().unwrap(), stepped.bounds().unwrap());
    for (a, b) in [(gb.min, sb.min), (gb.max, sb.max)] {
        assert!((a - b).length() < 1e-9, "{what}: bounds");
    }
    // The stepped model may split one flat face in two where the boolean keeps it whole (a wall
    // that continues across slabs), so compare what lies in each plane, not the number of
    // pieces: for every plane and orientation, the total area of the faces in it.
    assert_eq!(planes(general), planes(stepped), "{what}: area per plane");
    assert_eq!(
        cylinders(general),
        cylinders(stepped),
        "{what}: area per cylinder"
    );
    assert!(
        general.faces.len() <= stepped.faces.len(),
        "{what}: the boolean never has more faces"
    );
    assert!(
        general.edges.len() <= stepped.edges.len(),
        "{what}: nor more edges"
    );
}

/// `(radius, axis line, total area)` of every cylinder the solid has faces on, rounded and sorted.
fn cylinders(s: &Solid) -> Vec<(i64, i64, i64, i64)> {
    use dc_kernel::Surface;
    let q = |x: f64| (x * 1e6).round() as i64;
    let mut m: std::collections::BTreeMap<(i64, i64, i64), f64> = Default::default();
    for f in &s.faces {
        if let Surface::Cylinder { origin, radius, .. } = &f.surface {
            *m.entry((q(*radius), q(origin.x), q(origin.y)))
                .or_insert(0.0) += s.face_area(f);
        }
    }
    m.into_iter()
        .map(|((r, x, y), area)| (r, x, y, q(area)))
        .collect()
}

/// `(normal, offset, total area)` of every plane the solid has faces in, rounded and sorted.
fn planes(s: &Solid) -> Vec<(i64, i64, i64, i64, i64)> {
    use dc_kernel::Surface;
    let q = |x: f64| (x * 1e6).round() as i64;
    let mut m: std::collections::BTreeMap<(i64, i64, i64, i64), f64> = Default::default();
    for f in &s.faces {
        if let Surface::Plane { origin, normal, .. } = &f.surface {
            let n = if f.same_sense { *normal } else { -*normal };
            *m.entry((q(n.x), q(n.y), q(n.z), q(origin.dot(n))))
                .or_insert(0.0) += s.face_area(f);
        }
    }
    m.into_iter()
        .map(|((a, b, c, d), area)| (a, b, c, d, q(area)))
        .collect()
}

fn sub(a: &Solid, b: &Solid) -> Solid {
    a.boolean(b, BoolOp::Subtract).unwrap().solid
}
fn uni(a: &Solid, b: &Solid) -> Solid {
    a.boolean(b, BoolOp::Union).unwrap().solid
}

fn base() -> (Stepped, Solid) {
    let st = Stepped::extrusion(xy(), rect("a", 0.0, 0.0, 80.0, 40.0), 12.0, false);
    (st, cuboid(0.0, 0.0, 0.0, 80.0, 40.0, 12.0))
}

#[test]
fn the_plain_block() {
    let (st, g) = base();
    same(&build_solid(&st).unwrap(), &g, "block");
}

#[test]
fn a_blind_pocket() {
    let (st, g) = base();
    let st = st
        .cut(&rect("t", 10.0, 10.0, 30.0, 30.0), 6.0, 12.0, "c1")
        .unwrap();
    let g = sub(&g, &cuboid(10.0, 10.0, 6.0, 30.0, 30.0, 13.0));
    same(&build_solid(&st).unwrap(), &g, "pocket");
}

#[test]
fn a_through_hole() {
    let (st, g) = base();
    let st = st
        .cut(&rect("t", 10.0, 10.0, 30.0, 30.0), 0.0, 12.0, "c1")
        .unwrap();
    let g = sub(&g, &cuboid(10.0, 10.0, -1.0, 30.0, 30.0, 13.0));
    same(&build_solid(&st).unwrap(), &g, "through hole");
}

#[test]
fn a_counterbore() {
    let (st, g) = base();
    let st = st
        .cut(&rect("t", 20.0, 15.0, 30.0, 25.0), 0.0, 12.0, "c1")
        .unwrap()
        .cut(&rect("u", 15.0, 10.0, 35.0, 30.0), 8.0, 12.0, "c2")
        .unwrap();
    let g = sub(&g, &cuboid(20.0, 15.0, -1.0, 30.0, 25.0, 13.0));
    let g = sub(&g, &cuboid(15.0, 10.0, 8.0, 35.0, 30.0, 13.0));
    same(&build_solid(&st).unwrap(), &g, "counterbore");
}

#[test]
fn a_boss_with_a_pocket_in_it() {
    let (st, g) = base();
    let st = st
        .boss(&rect("b", 10.0, 10.0, 50.0, 30.0), 12.0, 20.0, "b1")
        .unwrap()
        .cut(&rect("t", 20.0, 15.0, 30.0, 25.0), 14.0, 20.0, "c1")
        .unwrap();
    let g = uni(&g, &cuboid(10.0, 10.0, 12.0, 50.0, 30.0, 20.0));
    let g = sub(&g, &cuboid(20.0, 15.0, 14.0, 30.0, 25.0, 21.0));
    same(&build_solid(&st).unwrap(), &g, "boss and pocket");
}

#[test]
fn stacked_bosses() {
    let (st, g) = base();
    let st = st
        .boss(&rect("a", 10.0, 10.0, 70.0, 30.0), 12.0, 20.0, "b1")
        .unwrap()
        .boss(&rect("b", 20.0, 15.0, 60.0, 25.0), 20.0, 30.0, "b2")
        .unwrap();
    let g = uni(&g, &cuboid(10.0, 10.0, 12.0, 70.0, 30.0, 20.0));
    let g = uni(&g, &cuboid(20.0, 15.0, 20.0, 60.0, 25.0, 30.0));
    same(&build_solid(&st).unwrap(), &g, "stacked bosses");
}

#[test]
fn a_cut_that_splits_the_part_in_two_bodies() {
    let (st, g) = base();
    let st = st
        .cut(&rect("t", 30.0, -5.0, 50.0, 45.0), 0.0, 12.0, "c1")
        .unwrap();
    let g = sub(&g, &cuboid(30.0, -5.0, -1.0, 50.0, 45.0, 13.0));
    same(&build_solid(&st).unwrap(), &g, "split");
}

#[test]
fn a_step_along_one_edge() {
    let (st, g) = base();
    let st = st
        .cut(&rect("t", -5.0, -5.0, 85.0, 15.0), 6.0, 12.0, "c1")
        .unwrap();
    let g = sub(&g, &cuboid(-5.0, -5.0, 6.0, 85.0, 15.0, 13.0));
    same(&build_solid(&st).unwrap(), &g, "step");
}

#[test]
fn a_round_hole_through_the_block() {
    let (st, g) = base();
    let st = st
        .cut(&disc("t", 40.0, 20.0, 6.0), 0.0, 12.0, "c1")
        .unwrap();
    let g = sub(&g, &rod(40.0, 20.0, -1.0, 6.0, 14.0));
    same(&build_solid(&st).unwrap(), &g, "round hole");
}

#[test]
fn a_round_counterbore() {
    let (st, g) = base();
    let st = st
        .cut(&disc("t", 40.0, 20.0, 4.0), 0.0, 12.0, "c1")
        .unwrap()
        .cut(&disc("u", 40.0, 20.0, 8.0), 8.0, 12.0, "c2")
        .unwrap();
    let g = sub(&g, &rod(40.0, 20.0, -1.0, 4.0, 14.0));
    let g = sub(&g, &rod(40.0, 20.0, 8.0, 8.0, 5.0));
    same(&build_solid(&st).unwrap(), &g, "round counterbore");
}

#[test]
fn a_round_boss_with_a_round_pocket() {
    let (st, g) = base();
    let st = st
        .boss(&disc("b", 40.0, 20.0, 12.0), 12.0, 20.0, "b1")
        .unwrap()
        .cut(&disc("t", 40.0, 20.0, 5.0), 14.0, 20.0, "c1")
        .unwrap();
    let g = uni(&g, &rod(40.0, 20.0, 12.0, 12.0, 8.0));
    let g = sub(&g, &rod(40.0, 20.0, 14.0, 5.0, 7.0));
    same(&build_solid(&st).unwrap(), &g, "round boss and pocket");
}

#[test]
fn two_round_holes_and_a_round_boss() {
    let (st, g) = base();
    let st = st
        .cut(&disc("a", 20.0, 20.0, 5.0), 0.0, 12.0, "c1")
        .unwrap()
        .cut(&disc("b", 60.0, 20.0, 5.0), 0.0, 12.0, "c2")
        .unwrap()
        .boss(&disc("c", 40.0, 20.0, 6.0), 12.0, 18.0, "b1")
        .unwrap();
    let g = sub(&g, &rod(20.0, 20.0, -1.0, 5.0, 14.0));
    let g = sub(&g, &rod(60.0, 20.0, -1.0, 5.0, 14.0));
    let g = uni(&g, &rod(40.0, 20.0, 12.0, 6.0, 6.0));
    same(&build_solid(&st).unwrap(), &g, "two holes and a boss");
}

// ---- tools built from real sketch profiles (arcs, holes), through `solid_ops` ------------------

use dc_features::solid_ops::{boss_with_tool, cut_with_tool, tool_solid};

fn ring(p: &str, cx: f64, cy: f64, r_out: f64, r_in: f64) -> Profile {
    let mut s = Sketch::new();
    s.add_entity(&format!("{p}m"), Entity::Point { x: cx, y: cy })
        .unwrap();
    s.add_entity(
        &format!("{p}o"),
        Entity::Circle {
            center: format!("{p}m"),
            radius: r_out,
        },
    )
    .unwrap();
    s.add_entity(
        &format!("{p}i"),
        Entity::Circle {
            center: format!("{p}m"),
            radius: r_in,
        },
    )
    .unwrap();
    let r = s.profiles();
    assert_eq!(r.profiles.len(), 1, "{:?}", r.problems);
    assert_eq!(r.profiles[0].holes.len(), 1);
    r.profiles.into_iter().next().unwrap()
}

fn d_shape(p: &str, cx: f64, cy: f64, r: f64) -> Profile {
    let mut s = Sketch::new();
    for (id, x, y) in [("c", cx, cy), ("l", cx - r, cy), ("r", cx + r, cy)] {
        s.add_entity(&format!("{p}{id}"), Entity::Point { x, y })
            .unwrap();
    }
    s.add_entity(
        &format!("{p}arc"),
        Entity::Arc {
            center: format!("{p}c"),
            start: format!("{p}r"),
            end: format!("{p}l"),
        },
    )
    .unwrap();
    s.add_entity(
        &format!("{p}base"),
        Entity::Line {
            a: format!("{p}l"),
            b: format!("{p}r"),
        },
    )
    .unwrap();
    s.profiles().profiles.into_iter().next().unwrap()
}

#[test]
fn an_annular_pocket_leaves_an_island() {
    let (st, base_solid) = base();
    let prof = ring("a", 40.0, 20.0, 12.0, 6.0);
    let st = st.cut(&prof, 6.0, 12.0, "ann").unwrap();
    let tool = tool_solid(&xy(), &prof, 6.0, 13.0).unwrap();
    let g = cut_with_tool(&base_solid, &tool, "ann").unwrap();
    same(&build_solid(&st).unwrap(), &g, "annular pocket");
}

#[test]
fn a_half_round_pocket() {
    let (st, base_solid) = base();
    let prof = d_shape("d", 40.0, 20.0, 10.0);
    let st = st.cut(&prof, 4.0, 12.0, "half").unwrap();
    let tool = tool_solid(&xy(), &prof, 4.0, 13.0).unwrap();
    let g = cut_with_tool(&base_solid, &tool, "half").unwrap();
    same(&build_solid(&st).unwrap(), &g, "half round pocket");
}

#[test]
fn an_annular_boss_and_a_half_round_boss() {
    let (st, base_solid) = base();
    let ring_p = ring("a", 20.0, 20.0, 10.0, 5.0);
    let half = d_shape("d", 60.0, 20.0, 8.0);
    let st = st
        .boss(&ring_p, 12.0, 20.0, "r")
        .unwrap()
        .boss(&half, 12.0, 17.0, "h")
        .unwrap();
    let g = boss_with_tool(
        &base_solid,
        &tool_solid(&xy(), &ring_p, 12.0, 20.0).unwrap(),
        "r",
    )
    .unwrap();
    let g = boss_with_tool(&g, &tool_solid(&xy(), &half, 12.0, 17.0).unwrap(), "h").unwrap();
    same(
        &build_solid(&st).unwrap(),
        &g,
        "annular and half-round bosses",
    );
}
