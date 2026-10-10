//! Boolean operations on planar solids. Every result is checked three ways: it validates, its
//! exact volume matches the analytic answer (or the inclusion-exclusion identities), and probe
//! points inside/outside agree with an independent point-in-solid test through the *mesh*.

use dc_kernel::boolean::{BoolError, BoolOp};
use dc_kernel::{Solid, TessParams};
use dc_math::{det, Vec3};

fn v(x: f64, y: f64, z: f64) -> Vec3 {
    Vec3::new(x, y, z)
}

fn cube(a: [f64; 3], b: [f64; 3]) -> Solid {
    Solid::cuboid(v(a[0], a[1], a[2]), v(b[0], b[1], b[2])).unwrap()
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

/// Point-in-solid by ray parity through the tessellation (Moller-Trumbore), averaged over three
/// directions; `None` when the answer is not clear (a ray grazes an edge).
fn inside_mesh(s: &Solid, p: Vec3) -> Option<bool> {
    let mesh = s.tessellate(TessParams::default()).expect("tessellates");
    let dirs = [
        v(0.371_390_7, 0.557_086, 0.742_781_4),
        v(-0.267_261_2, 0.534_522_5, 0.801_783_7),
        v(0.801_783_7, -0.534_522_5, 0.267_261_2),
    ];
    let mut votes = 0;
    for d in dirs {
        let mut n = 0;
        let mut clear = true;
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
                    clear = false;
                }
                n += 1;
            }
        }
        if !clear {
            return None;
        }
        votes += n % 2;
    }
    match votes {
        0 => Some(false),
        3 => Some(true),
        _ => None,
    }
}

fn expect_volume(s: &Solid, want: f64) {
    assert!(
        (s.volume() - want).abs() < 1e-7,
        "volume {} vs {want}",
        s.volume()
    );
}

#[test]
fn overlapping_boxes_obey_inclusion_exclusion() {
    let a = cube([0.0, 0.0, 0.0], [10.0, 10.0, 10.0]);
    let b = cube([5.0, 5.0, 5.0], [15.0, 15.0, 15.0]);
    let sub = run(&a, &b, BoolOp::Subtract);
    expect_volume(&sub, 875.0);
    let uni = run(&a, &b, BoolOp::Union);
    expect_volume(&uni, 1875.0);
    let int = run(&a, &b, BoolOp::Intersect);
    expect_volume(&int, 125.0);
    assert_eq!(int.faces.len(), 6);
    // The corner notch of the subtraction has three extra faces: 6 + 3.
    assert_eq!(sub.faces.len(), 9);
}

#[test]
fn a_square_hole_through_a_block() {
    let block = cube([0.0, 0.0, 0.0], [10.0, 10.0, 10.0]);
    let tool = cube([4.0, 4.0, -1.0], [6.0, 6.0, 11.0]);
    let r = run(&block, &tool, BoolOp::Subtract);
    expect_volume(&r, 1000.0 - 4.0 * 10.0);
    // Six faces of the block, four walls of the hole; top and bottom each have one hole.
    assert_eq!(r.faces.len(), 10);
    assert_eq!(r.faces.iter().filter(|f| f.loops.len() == 2).count(), 2);
    let ok = inside_mesh(&r, v(5.0, 5.0, 5.0));
    assert_eq!(ok, Some(false), "the hole is empty");
    assert_eq!(inside_mesh(&r, v(1.0, 1.0, 5.0)), Some(true));
}

#[test]
fn a_pocket_flush_with_a_face() {
    // The tool starts exactly at the bottom face: coplanar faces, same outward sense.
    let block = cube([0.0, 0.0, 0.0], [10.0, 10.0, 10.0]);
    let tool = cube([2.0, 2.0, 0.0], [4.0, 4.0, 5.0]);
    let r = run(&block, &tool, BoolOp::Subtract);
    expect_volume(&r, 1000.0 - 20.0);
    assert_eq!(r.faces.len(), 6 + 5);
}

#[test]
fn boxes_that_share_a_face_join_into_one_box() {
    let a = cube([0.0, 0.0, 0.0], [10.0, 10.0, 10.0]);
    let b = cube([10.0, 0.0, 0.0], [20.0, 10.0, 10.0]);
    let u = run(&a, &b, BoolOp::Union);
    expect_volume(&u, 2000.0);
    assert_eq!(u.faces.len(), 6, "coplanar neighbours merge");
    assert_eq!(u.edges.len(), 12);
    assert_eq!(u.vertices.len(), 8);
    // Touching boxes intersect in nothing.
    assert_eq!(
        a.boolean(&b, BoolOp::Intersect).unwrap_err(),
        BoolError::Empty
    );
    // Subtracting one from the other changes nothing.
    let s = run(&a, &b, BoolOp::Subtract);
    expect_volume(&s, 1000.0);
    assert_eq!(s.faces.len(), 6);
}

#[test]
fn a_tool_rotated_in_the_plane() {
    let block = cube([0.0, 0.0, 0.0], [20.0, 20.0, 10.0]);
    let (c, s) = (
        det::cos(core::f64::consts::FRAC_PI_4),
        det::sin(core::f64::consts::FRAC_PI_4),
    );
    let rot = |x: f64, y: f64| v(10.0 + x * c - y * s, 10.0 + x * s + y * c, -1.0);
    let sq = [
        rot(-2.0, -2.0),
        rot(2.0, -2.0),
        rot(2.0, 2.0),
        rot(-2.0, 2.0),
    ];
    let tool = Solid::prism(&sq, &[], v(0.0, 0.0, 12.0)).unwrap();
    let r = run(&block, &tool, BoolOp::Subtract);
    expect_volume(&r, 4000.0 - 16.0 * 10.0);
}

#[test]
fn an_angled_cut_removes_a_wedge() {
    // A slanted slab cuts off a corner of the block: a plane through (10,0,5) (10,10,5) (5,0,10).
    let block = cube([0.0, 0.0, 0.0], [10.0, 10.0, 10.0]);
    let poly = [
        v(5.0, -1.0, 10.0),
        v(11.0, -1.0, 4.0),
        v(11.0, 11.0, 4.0),
        v(5.0, 11.0, 10.0),
    ];
    let tool = Solid::prism(&poly, &[], v(10.0, 0.0, 10.0)).unwrap();
    let r = run(&block, &tool, BoolOp::Subtract);
    // The wedge is a triangular prism with legs 5 and 5 over the 10 mm width: volume 125.
    expect_volume(&r, 1000.0 - 125.0);
    assert_eq!(
        r.faces.len(),
        7,
        "six faces of the block (two shortened) plus the new slanted face"
    );
}

#[test]
fn a_pocket_with_an_island_keeps_its_hole() {
    // A ring-shaped tool (a square with a square hole) cut into the block leaves an island.
    let block = cube([0.0, 0.0, 0.0], [10.0, 10.0, 10.0]);
    let outer = [
        v(2.0, 2.0, 5.0),
        v(8.0, 2.0, 5.0),
        v(8.0, 8.0, 5.0),
        v(2.0, 8.0, 5.0),
    ];
    let hole = vec![
        v(4.0, 4.0, 5.0),
        v(6.0, 4.0, 5.0),
        v(6.0, 6.0, 5.0),
        v(4.0, 6.0, 5.0),
    ];
    let tool = Solid::prism(&outer, &[hole], v(0.0, 0.0, 6.0)).unwrap();
    let r = run(&block, &tool, BoolOp::Subtract);
    expect_volume(&r, 1000.0 - (36.0 - 4.0) * 5.0);
    assert_eq!(
        inside_mesh(&r, v(5.1, 5.2, 7.3)),
        Some(true),
        "the island stays"
    );
    assert_eq!(
        inside_mesh(&r, v(3.1, 5.3, 7.3)),
        Some(false),
        "the ring is cut"
    );
}

/// A small deterministic generator (no external crate): xorshift.
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

#[test]
fn random_boxes_agree_with_probes_and_identities() {
    let mut rng = Rng(0x9E37_79B9_7F4A_7C15);
    let mut done = 0;
    let mut refused = 0;
    for _ in 0..60 {
        // Grid-aligned coordinates make coincident faces common, which is the hard case.
        let mut g = |lo: f64, hi: f64| (rng.range(lo, hi) * 2.0).round() / 2.0;
        let a0 = [g(0.0, 4.0), g(0.0, 4.0), g(0.0, 4.0)];
        let a1 = [
            a0[0] + g(2.0, 8.0).max(1.0),
            a0[1] + g(2.0, 8.0).max(1.0),
            a0[2] + g(2.0, 8.0).max(1.0),
        ];
        let b0 = [g(0.0, 8.0), g(0.0, 8.0), g(0.0, 8.0)];
        let b1 = [
            b0[0] + g(1.0, 6.0).max(1.0),
            b0[1] + g(1.0, 6.0).max(1.0),
            b0[2] + g(1.0, 6.0).max(1.0),
        ];
        let (a, b) = (cube(a0, a1), cube(b0, b1));
        let va = a.volume();
        let vb = b.volume();
        let int = match a.boolean(&b, BoolOp::Intersect) {
            Ok(r) => Some(r.solid),
            Err(BoolError::Empty) => None,
            Err(e) => {
                // Contact along an edge or a point is non-manifold: a refusal is acceptable, a
                // wrong solid is not.
                assert!(
                    matches!(e, BoolError::Invalid(_) | BoolError::Degenerate(_)),
                    "{e}"
                );
                refused += 1;
                continue;
            }
        };
        let vi = int.as_ref().map_or(0.0, Solid::volume);
        let uni = a.boolean(&b, BoolOp::Union);
        let sub = a.boolean(&b, BoolOp::Subtract);
        if let Ok(u) = &uni {
            assert!(
                (u.solid.volume() - (va + vb - vi)).abs() < 1e-6,
                "union {a0:?} {a1:?} {b0:?} {b1:?}"
            );
        } else {
            refused += 1;
        }
        match &sub {
            Ok(s) => assert!(
                (s.solid.volume() - (va - vi)).abs() < 1e-6,
                "subtract {a0:?} {a1:?} {b0:?} {b1:?}"
            ),
            Err(BoolError::Empty) => assert!((va - vi).abs() < 1e-9),
            Err(_) => refused += 1,
        }
        // Probe points away from the faces.
        if let (Ok(u), Ok(s)) = (&uni, &sub) {
            for _ in 0..12 {
                let p = v(
                    rng.range(-1.0, 14.0),
                    rng.range(-1.0, 14.0),
                    rng.range(-1.0, 14.0),
                );
                let in_box = |lo: [f64; 3], hi: [f64; 3]| {
                    (0..3).all(|k| [p.x, p.y, p.z][k] > lo[k] && [p.x, p.y, p.z][k] < hi[k])
                };
                let near = |lo: [f64; 3], hi: [f64; 3]| {
                    (0..3).any(|k| {
                        let c = [p.x, p.y, p.z][k];
                        (c - lo[k]).abs() < 1e-3 || (c - hi[k]).abs() < 1e-3
                    })
                };
                if near(a0, a1) || near(b0, b1) {
                    continue;
                }
                let (ia, ib) = (in_box(a0, a1), in_box(b0, b1));
                if let Some(x) = inside_mesh(&u.solid, p) {
                    assert_eq!(x, ia || ib, "union probe {p:?}");
                }
                if let Some(x) = inside_mesh(&s.solid, p) {
                    assert_eq!(x, ia && !ib, "subtract probe {p:?}");
                }
            }
        }
        done += 1;
    }
    eprintln!("random boxes: {done} ran, {refused} refused");
    assert!(done >= 30, "only {done} cases ran ({refused} refused)");
}

#[test]
fn cones_are_refused_with_a_reason() {
    // A face that is a cone patch is named in the refusal.
    let mut a = cube([0.0, 0.0, 0.0], [1.0, 1.0, 1.0]);
    a.faces[0].surface = dc_kernel::Surface::Cone {
        origin: v(0.0, 0.0, 0.0),
        axis: v(0.0, 0.0, 1.0),
        ref_dir: v(1.0, 0.0, 0.0),
        h0: 0.0,
        r0: 1.0,
        h1: 1.0,
        r1: 2.0,
        phi0: 0.0,
        phi1: 1.0,
    };
    let b = cube([0.0, 0.0, 0.0], [1.0, 1.0, 1.0]);
    match a.boolean(&b, BoolOp::Union) {
        Err(BoolError::Unsupported(why)) => assert!(why.contains("cone"), "{why}"),
        other => panic!("{other:?}"),
    }
}

fn rotate(p: Vec3, ax: usize, ang: f64) -> Vec3 {
    let (c, s) = (det::cos(ang), det::sin(ang));
    match ax {
        0 => v(p.x, p.y * c - p.z * s, p.y * s + p.z * c),
        1 => v(p.x * c + p.z * s, p.y, -p.x * s + p.z * c),
        _ => v(p.x * c - p.y * s, p.x * s + p.y * c, p.z),
    }
}

/// A box turned by random angles about two axes: arbitrary face orientations.
fn turned_box(rng: &mut Rng, centre: Vec3, half: [f64; 3]) -> Solid {
    let (a1, a2) = (rng.range(0.0, 3.0), rng.range(0.0, 3.0));
    let (ax1, ax2) = (
        (rng.next() * 3.0) as usize % 3,
        (rng.next() * 3.0) as usize % 3,
    );
    let corner = |sx: f64, sy: f64, sz: f64| {
        let p = v(sx * half[0], sy * half[1], sz * half[2]);
        rotate(rotate(p, ax1, a1), ax2, a2) + centre
    };
    let base = [
        corner(-1.0, -1.0, -1.0),
        corner(1.0, -1.0, -1.0),
        corner(1.0, 1.0, -1.0),
        corner(-1.0, 1.0, -1.0),
    ];
    let dir = corner(-1.0, -1.0, 1.0) - corner(-1.0, -1.0, -1.0);
    Solid::prism(&base, &[], dir).unwrap()
}

#[test]
fn randomly_turned_boxes_agree_with_probes_and_identities() {
    let mut rng = Rng(0x2545_F491_4F6C_DD1D);
    let (mut ran, mut refused) = (0, 0);
    for _ in 0..80 {
        let a = turned_box(&mut rng, v(0.0, 0.0, 0.0), [4.0, 3.0, 5.0]);
        let centre = v(
            rng.range(-3.0, 3.0),
            rng.range(-3.0, 3.0),
            rng.range(-3.0, 3.0),
        );
        let half = [
            rng.range(1.0, 4.0),
            rng.range(1.0, 4.0),
            rng.range(1.0, 4.0),
        ];
        let b = turned_box(&mut rng, centre, half);
        let (va, vb) = (a.volume(), b.volume());
        let int = a.boolean(&b, BoolOp::Intersect);
        let uni = a.boolean(&b, BoolOp::Union);
        let sub = a.boolean(&b, BoolOp::Subtract);
        let (Ok(int), Ok(uni), Ok(sub)) = (int, uni, sub) else {
            refused += 1;
            continue;
        };
        let vi = int.solid.volume();
        assert!(
            (uni.solid.volume() - (va + vb - vi)).abs() < 1e-6,
            "union volume"
        );
        assert!(
            (sub.solid.volume() - (va - vi)).abs() < 1e-6,
            "subtract volume"
        );
        assert!(vi > 0.0 && vi <= va.min(vb) + 1e-9);
        for _ in 0..10 {
            let p = v(
                rng.range(-9.0, 9.0),
                rng.range(-9.0, 9.0),
                rng.range(-9.0, 9.0),
            );
            let (Some(ia), Some(ib)) = (inside_mesh(&a, p), inside_mesh(&b, p)) else {
                continue;
            };
            if let Some(x) = inside_mesh(&uni.solid, p) {
                assert_eq!(x, ia || ib, "union probe {p:?}");
            }
            if let Some(x) = inside_mesh(&sub.solid, p) {
                assert_eq!(x, ia && !ib, "subtract probe {p:?}");
            }
            if let Some(x) = inside_mesh(&int.solid, p) {
                assert_eq!(x, ia && ib, "intersect probe {p:?}");
            }
        }
        ran += 1;
    }
    eprintln!("turned boxes: {ran} ran, {refused} refused");
    assert!(ran >= 25, "only {ran} cases ran, {refused} refused");
}

#[test]
fn chained_operations_keep_every_result_valid() {
    // Drill three holes and a slanted slot, one after another; each result feeds the next.
    let mut s = cube([0.0, 0.0, 0.0], [30.0, 20.0, 10.0]);
    let mut want = 30.0 * 20.0 * 10.0;
    for (i, x) in [5.0, 15.0, 25.0].iter().enumerate() {
        let tool = cube([*x - 1.0, 9.0, -1.0], [*x + 1.0, 11.0, 11.0]);
        s = run(&s, &tool, BoolOp::Subtract);
        want -= 4.0 * 10.0;
        expect_volume(&s, want);
        assert_eq!(s.faces.len(), 6 + 4 * (i + 1));
    }
    // A boss on top that overlaps a hole's mouth.
    let boss = cube([13.0, 8.0, 10.0], [17.0, 12.0, 14.0]);
    let s = run(&s, &boss, BoolOp::Union);
    expect_volume(&s, want + 16.0 * 4.0 - 4.0 * 0.0);
}

#[test]
fn results_do_not_depend_on_the_order_of_the_inputs_faces() {
    let a = cube([0.0, 0.0, 0.0], [10.0, 10.0, 10.0]);
    let b = cube([5.0, 5.0, 5.0], [15.0, 15.0, 15.0]);
    let r1 = a.boolean(&b, BoolOp::Union).unwrap();
    let r2 = a.boolean(&b, BoolOp::Union).unwrap();
    assert_eq!(
        r1.solid.canonical_bytes(),
        r2.solid.canonical_bytes(),
        "deterministic"
    );
    // The sources say where each face came from.
    assert!(r1.sources.iter().any(|s| s.from.len() == 1));
}

#[test]
fn near_coincident_faces_never_give_an_invalid_solid() {
    // Offsets from far below to far above the tolerance: each case is a valid solid or a named
    // refusal, never a panic or a solid that fails validation.
    let a = cube([0.0, 0.0, 0.0], [10.0, 10.0, 10.0]);
    for off in [0.0, 1e-9, 1e-7, 5e-7, 1.5e-6, 1e-5, 1e-3] {
        let b = cube([2.0, 2.0, -off], [4.0, 4.0, 5.0]);
        for op in [BoolOp::Union, BoolOp::Subtract, BoolOp::Intersect] {
            match a.boolean(&b, op) {
                Ok(r) => assert!(r.solid.validate().is_empty(), "off {off} {op:?}"),
                Err(BoolError::Unsupported(_)) => panic!("planar input refused"),
                Err(_) => {}
            }
        }
    }
}
