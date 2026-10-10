//! Boolean operations with cylinders: holes, bosses, slabs cut from a round bar, angled cuts
//! (ellipse edges). Each result must validate, match the closed-form volume, tessellate
//! watertight, and agree with probe points through the mesh.

use core::f64::consts::PI;
use dc_kernel::boolean::{BoolError, BoolOp};
use dc_kernel::{Solid, TessParams};
use dc_math::{det, Vec3};

fn v(x: f64, y: f64, z: f64) -> Vec3 {
    Vec3::new(x, y, z)
}

fn cube(a: [f64; 3], b: [f64; 3]) -> Solid {
    Solid::cuboid(v(a[0], a[1], a[2]), v(b[0], b[1], b[2])).unwrap()
}

fn bar(x: f64, y: f64, z: f64, r: f64, h: f64) -> Solid {
    Solid::cylinder(v(x, y, z), v(0.0, 0.0, 1.0), r, h).unwrap()
}

fn run(a: &Solid, b: &Solid, op: BoolOp) -> Solid {
    match a.boolean(b, op) {
        Ok(r) => {
            assert!(r.solid.validate().is_empty(), "{:?}", r.solid.validate());
            let names: std::collections::BTreeSet<&String> =
                r.solid.faces.iter().map(|f| &f.name).collect();
            assert_eq!(names.len(), r.solid.faces.len(), "face names are unique");
            let enames: std::collections::BTreeSet<&String> =
                r.solid.edges.iter().map(|e| &e.name).collect();
            assert_eq!(enames.len(), r.solid.edges.len(), "edge names are unique");
            r.solid
        }
        Err(e) => panic!("{op:?} failed: {e}"),
    }
}

fn vol(s: &Solid, want: f64) {
    assert!(
        (s.volume() - want).abs() < 1e-6 * want.abs().max(1.0),
        "volume {} vs {want}",
        s.volume()
    );
}

/// The mesh is watertight and its volume is within `tol` of the exact one.
fn meshes_well(s: &Solid, tol: f64) {
    let m = s.tessellate(TessParams::default()).expect("tessellates");
    assert!(m.check().is_empty(), "{:?}", m.check());
    let exact = s.volume();
    assert!(
        (m.volume() - exact).abs() / exact < tol,
        "mesh {} vs {exact}",
        m.volume()
    );
}

#[test]
fn the_cylinder_constructor_is_valid() {
    let c = bar(1.0, 2.0, 3.0, 2.0, 5.0);
    assert!(c.validate().is_empty(), "{:?}", c.validate());
    vol(&c, PI * 4.0 * 5.0);
    meshes_well(&c, 1e-2);
    let tilted = Solid::cylinder(v(0.0, 0.0, 0.0), v(1.0, 1.0, 1.0), 1.5, 4.0).unwrap();
    assert!(tilted.validate().is_empty());
    vol(&tilted, PI * 1.5 * 1.5 * 4.0);
}

#[test]
fn a_round_hole_through_a_block() {
    let block = cube([0.0, 0.0, 0.0], [20.0, 20.0, 10.0]);
    let tool = bar(10.0, 10.0, -1.0, 3.0, 12.0);
    let r = run(&block, &tool, BoolOp::Subtract);
    vol(&r, 4000.0 - PI * 9.0 * 10.0);
    assert_eq!(r.faces.len(), 7, "six block faces and the hole wall");
    meshes_well(&r, 5e-3);
}

#[test]
fn a_blind_round_hole() {
    let block = cube([0.0, 0.0, 0.0], [20.0, 20.0, 10.0]);
    let tool = bar(10.0, 10.0, 6.0, 3.0, 8.0);
    let r = run(&block, &tool, BoolOp::Subtract);
    vol(&r, 4000.0 - PI * 9.0 * 4.0);
    assert_eq!(r.faces.len(), 8, "block, wall, floor");
    meshes_well(&r, 5e-3);
}

#[test]
fn a_round_boss_on_top_of_a_block() {
    let block = cube([0.0, 0.0, 0.0], [20.0, 20.0, 10.0]);
    let tool = bar(10.0, 10.0, 10.0, 3.0, 4.0);
    let r = run(&block, &tool, BoolOp::Union);
    vol(&r, 4000.0 + PI * 9.0 * 4.0);
    assert_eq!(
        r.faces.len(),
        8,
        "six block faces, the boss wall and its top"
    );
    meshes_well(&r, 5e-3);
}

#[test]
fn a_slab_cut_from_a_round_bar_along_a_plane_parallel_to_its_axis() {
    // Remove everything with x > 1: a circular segment is left.
    let b = bar(0.0, 0.0, 0.0, 3.0, 10.0);
    let tool = cube([1.0, -5.0, -1.0], [5.0, 5.0, 11.0]);
    let r = run(&b, &tool, BoolOp::Subtract);
    let d = 1.0f64;
    let seg = 9.0 * det::acos(d / 3.0) - d * det::sqrt(9.0 - d * d);
    vol(&r, (PI * 9.0 - seg) * 10.0);
    meshes_well(&r, 8e-3);
}

#[test]
fn an_angled_cut_gives_an_elliptical_face() {
    // A plane through the axis at height 5 rising 0.5 per mm in x: everything above is removed.
    let b = bar(0.0, 0.0, 0.0, 3.0, 10.0);
    let poly = [
        v(-5.0, -5.0, 5.0 - 2.5),
        v(5.0, -5.0, 5.0 + 2.5),
        v(5.0, 5.0, 5.0 + 2.5),
        v(-5.0, 5.0, 5.0 - 2.5),
    ];
    let tool = Solid::prism(&poly, &[], v(0.0, 0.0, 20.0)).unwrap();
    let r = run(&b, &tool, BoolOp::Subtract);
    vol(&r, PI * 9.0 * 5.0);
    assert!(
        r.edges
            .iter()
            .any(|e| matches!(e.curve, dc_kernel::Curve::Ellipse { .. })),
        "an ellipse edge"
    );
    meshes_well(&r, 8e-3);
}

#[test]
fn a_bar_cut_across_by_a_perpendicular_plane() {
    let b = bar(0.0, 0.0, 0.0, 3.0, 10.0);
    let tool = cube([-5.0, -5.0, 6.0], [5.0, 5.0, 12.0]);
    let r = run(&b, &tool, BoolOp::Subtract);
    vol(&r, PI * 9.0 * 6.0);
    assert_eq!(r.faces.len(), 3);
    meshes_well(&r, 5e-3);
}

#[test]
fn the_intersection_of_a_block_and_a_bar() {
    let block = cube([-1.0, -5.0, 2.0], [5.0, 5.0, 8.0]);
    let b = bar(0.0, 0.0, 0.0, 3.0, 10.0);
    let r = run(&b, &block, BoolOp::Intersect);
    let d = 1.0f64;
    let seg = 9.0 * det::acos(d / 3.0) - d * det::sqrt(9.0 - d * d);
    // x from -1 to 3: the disc minus the segment beyond x = -1.
    vol(&r, (PI * 9.0 - seg) * 6.0);
    meshes_well(&r, 8e-3);
}

#[test]
fn a_flush_round_hole_and_a_boss_on_the_same_face_chain() {
    let block = cube([0.0, 0.0, 0.0], [30.0, 20.0, 10.0]);
    let a = run(&block, &bar(8.0, 10.0, -1.0, 3.0, 12.0), BoolOp::Subtract);
    let b = run(&a, &bar(22.0, 10.0, -1.0, 2.0, 12.0), BoolOp::Subtract);
    vol(&b, 6000.0 - PI * 9.0 * 10.0 - PI * 4.0 * 10.0);
    let c = run(&b, &bar(15.0, 10.0, 10.0, 2.0, 3.0), BoolOp::Union);
    vol(
        &c,
        6000.0 - PI * 9.0 * 10.0 - PI * 4.0 * 10.0 + PI * 4.0 * 3.0,
    );
    meshes_well(&c, 8e-3);
}

#[test]
fn cylinders_that_cross_are_refused_with_a_reason() {
    let a = bar(0.0, 0.0, 0.0, 3.0, 10.0);
    let across = Solid::cylinder(v(-5.0, 0.0, 5.0), v(1.0, 0.0, 0.0), 1.0, 10.0).unwrap();
    match a.boolean(&across, BoolOp::Subtract) {
        Err(BoolError::Unsupported(why)) => assert!(why.contains("cylinders that cross"), "{why}"),
        other => panic!("{other:?}"),
    }
}

// ---- random cases checked with analytic point-in-solid tests ----------------------------------

struct Rng(u64);
impl Rng {
    fn next(&mut self) -> f64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        (self.0 >> 11) as f64 / (1u64 << 53) as f64
    }
    fn range(&mut self, lo: f64, hi: f64) -> f64 {
        lo + (hi - lo) * self.next()
    }
}

/// Point-in-solid by ray parity through the mesh, over three directions; `None` if unclear.
fn inside_mesh(mesh: &dc_kernel::Mesh, p: Vec3) -> Option<bool> {
    let dirs = [
        v(0.371_390_7, 0.557_086, 0.742_781_4),
        v(-0.267_261_2, 0.534_522_5, 0.801_783_7),
        v(0.801_783_7, -0.534_522_5, 0.267_261_2),
    ];
    let mut votes = 0;
    for d in dirs {
        let mut n = 0;
        for t in &mesh.triangles {
            let (a, b, c) = (
                mesh.positions[t[0] as usize],
                mesh.positions[t[1] as usize],
                mesh.positions[t[2] as usize],
            );
            let (e1, e2) = (b - a, c - a);
            let h = d.cross(e2);
            let det_ = e1.dot(h);
            if det_.abs() < 1e-12 {
                continue;
            }
            let f = 1.0 / det_;
            let sv = p - a;
            let u = f * sv.dot(h);
            let q = sv.cross(e1);
            let w = f * d.dot(q);
            let tt = f * e2.dot(q);
            let eps = 1e-7;
            if tt > 1e-9 && u > -eps && w > -eps && u + w < 1.0 + eps {
                if u < eps || w < eps || u + w > 1.0 - eps {
                    return None;
                }
                n += 1;
            }
        }
        votes += n % 2;
    }
    match votes {
        0 => Some(false),
        3 => Some(true),
        _ => None,
    }
}

#[test]
fn random_blocks_and_bars_agree_with_analytic_probes() {
    let mut rng = Rng(0xDEAD_BEEF_CAFE_F00D);
    let (mut ran, mut refused) = (0, 0);
    for _ in 0..60 {
        let g = |r: &mut Rng, lo: f64, hi: f64| (r.range(lo, hi) * 2.0).round() / 2.0;
        let b0 = [
            g(&mut rng, 0.0, 4.0),
            g(&mut rng, 0.0, 4.0),
            g(&mut rng, 0.0, 4.0),
        ];
        let b1 = [
            b0[0] + g(&mut rng, 4.0, 10.0),
            b0[1] + g(&mut rng, 4.0, 10.0),
            b0[2] + g(&mut rng, 3.0, 8.0),
        ];
        let (cx, cy) = (g(&mut rng, 2.0, 12.0), g(&mut rng, 2.0, 12.0));
        let r = g(&mut rng, 1.0, 3.5);
        let (z0, h) = (g(&mut rng, -1.0, 6.0), g(&mut rng, 2.0, 10.0));
        let block = cube(b0, b1);
        let bar = bar(cx, cy, z0, r, h);
        let in_block = |p: Vec3| {
            p.x > b0[0] && p.x < b1[0] && p.y > b0[1] && p.y < b1[1] && p.z > b0[2] && p.z < b1[2]
        };
        let in_bar = |p: Vec3| {
            (p.x - cx) * (p.x - cx) + (p.y - cy) * (p.y - cy) < r * r && p.z > z0 && p.z < z0 + h
        };
        for (op, name) in [
            (BoolOp::Union, "union"),
            (BoolOp::Subtract, "subtract"),
            (BoolOp::Intersect, "intersect"),
        ] {
            let res = match block.boolean(&bar, op) {
                Ok(r) => r.solid,
                Err(BoolError::Empty) => continue,
                Err(e @ (BoolError::Invalid(_) | BoolError::Degenerate(_))) => {
                    refused += 1;
                    eprintln!(
                        "REFUSED {name} block {b0:?}-{b1:?} bar ({cx},{cy},{z0}) r {r} h {h}: {}",
                        e.to_string().chars().take(300).collect::<String>()
                    );
                    continue;
                }
                Err(BoolError::Unsupported(why)) if why.contains("tangent") => {
                    refused += 1;
                    continue;
                }
                Err(e) => panic!("{name}: {e}"),
            };
            assert!(res.validate().is_empty(), "{name}");
            let mesh = res.tessellate(TessParams::default()).expect("tessellates");
            for _ in 0..10 {
                let p = v(
                    rng.range(-1.0, 16.0),
                    rng.range(-1.0, 16.0),
                    rng.range(-2.0, 14.0),
                );
                let want = match op {
                    BoolOp::Union => in_block(p) || in_bar(p),
                    BoolOp::Subtract => in_block(p) && !in_bar(p),
                    BoolOp::Intersect => in_block(p) && in_bar(p),
                };
                // Skip points near any boundary: the mesh is a polygonal approximation.
                let near = |lo: f64, hi: f64, c: f64| (c - lo).abs() < 0.1 || (c - hi).abs() < 0.1;
                let rad = det::sqrt((p.x - cx) * (p.x - cx) + (p.y - cy) * (p.y - cy));
                if near(b0[0], b1[0], p.x)
                    || near(b0[1], b1[1], p.y)
                    || near(b0[2], b1[2], p.z)
                    || near(z0, z0 + h, p.z)
                    || (rad - r).abs() < 0.15
                {
                    continue;
                }
                if let Some(got) = inside_mesh(&mesh, p) {
                    assert_eq!(
                        got, want,
                        "{name} probe {p:?} block {b0:?}-{b1:?} bar ({cx},{cy},{z0}) r {r} h {h}"
                    );
                }
            }
        }
        ran += 1;
    }
    eprintln!("blocks and bars: {ran} ran, {refused} refused");
    assert!(ran >= 40);
}

// ---- cylinders with parallel axes ------------------------------------------------------------

/// Area of the lens where two discs overlap.
fn lens(r1: f64, r2: f64, d: f64) -> f64 {
    let a = r1 * r1 * det::acos((d * d + r1 * r1 - r2 * r2) / (2.0 * d * r1));
    let b = r2 * r2 * det::acos((d * d + r2 * r2 - r1 * r1) / (2.0 * d * r2));
    let c = 0.5 * det::sqrt((-d + r1 + r2) * (d + r1 - r2) * (d - r1 + r2) * (d + r1 + r2));
    a + b - c
}

#[test]
fn a_bar_with_an_overlapping_parallel_hole() {
    let b = bar(0.0, 0.0, 0.0, 5.0, 10.0);
    let tool = bar(6.0, 0.0, -1.0, 3.0, 12.0);
    let r = run(&b, &tool, BoolOp::Subtract);
    vol(&r, (PI * 25.0 - lens(5.0, 3.0, 6.0)) * 10.0);
    meshes_well(&r, 1e-2);
    let u = run(&b, &tool, BoolOp::Union);
    vol(
        &u,
        (PI * 25.0 + PI * 9.0 - lens(5.0, 3.0, 6.0)) * 10.0 + PI * 9.0 * 2.0,
    );
    let i = run(&b, &tool, BoolOp::Intersect);
    vol(&i, lens(5.0, 3.0, 6.0) * 10.0);
}

#[test]
fn two_overlapping_round_holes_make_a_slot_like_pocket_in_a_block() {
    let block = cube([0.0, 0.0, 0.0], [30.0, 20.0, 10.0]);
    let a = run(&block, &bar(10.0, 10.0, -1.0, 3.0, 12.0), BoolOp::Subtract);
    let b = run(&a, &bar(14.0, 10.0, -1.0, 3.0, 12.0), BoolOp::Subtract);
    vol(&b, (600.0 - (2.0 * PI * 9.0 - lens(3.0, 3.0, 4.0))) * 10.0);
    assert_eq!(
        b.faces.len(),
        6 + 2,
        "two hole walls (one face each) and the six block faces"
    );
    meshes_well(&b, 1e-2);
}

#[test]
fn parallel_cylinders_that_only_touch_are_refused_when_the_contact_is_real() {
    let b = bar(0.0, 0.0, 0.0, 5.0, 10.0);
    let tool = bar(8.0, 0.0, 2.0, 3.0, 5.0);
    match b.boolean(&tool, BoolOp::Subtract) {
        Err(BoolError::Unsupported(why)) => assert!(why.contains("tangent"), "{why}"),
        other => panic!("{other:?}"),
    }
}
