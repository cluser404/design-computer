//! The stepped prism model: pockets, counterbores, bosses, steps and splits, checked against exact
//! volumes, the already-tested extrude, and an independent inside/outside test through the mesh.

use core::f64::consts::PI;
use dc_features::{build_solid, extrude, StepError, Stepped};
use dc_kernel::{Mesh, Solid, TessParams};
use dc_math::{Plane, Tolerance, Vec2, Vec3};
use dc_sketch::{Entity, Profile, Sketch};

fn xy() -> Plane {
    Plane::new(Vec3::ZERO, Vec3::X, Vec3::Y, Tolerance::DEFAULT).unwrap()
}

fn profile(build: impl FnOnce(&mut Sketch)) -> Profile {
    let mut s = Sketch::new();
    build(&mut s);
    let r = s.profiles();
    assert!(r.problems.is_empty(), "{:?}", r.problems);
    assert_eq!(r.profiles.len(), 1);
    r.profiles.into_iter().next().unwrap()
}
fn rect(p: &'static str, x0: f64, y0: f64, x1: f64, y1: f64) -> Profile {
    profile(|s| {
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
    })
}
fn disc(p: &'static str, cx: f64, cy: f64, r: f64) -> Profile {
    profile(|s| {
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
    })
}
fn close(a: f64, b: f64) -> bool {
    (a - b).abs() <= 1e-7 * b.abs().max(1.0)
}
fn build(st: &Stepped) -> Solid {
    build_solid(st).unwrap_or_else(|e| panic!("{e}"))
}
/// Volume straight from the slabs: no B-rep involved.
fn slab_volume(st: &Stepped) -> f64 {
    st.slabs
        .iter()
        .enumerate()
        .map(|(k, s)| s.iter().map(|p| p.area).sum::<f64>() * (st.levels[k + 1].w - st.levels[k].w))
        .sum()
}
fn names(s: &Solid) -> Vec<String> {
    let mut v: Vec<String> = s.faces.iter().map(|f| f.name.clone()).collect();
    v.sort();
    v
}
fn watertight(s: &Solid) -> Mesh {
    let m = s
        .tessellate(TessParams::default())
        .unwrap_or_else(|e| panic!("{e}"));
    let p = m.check();
    assert!(p.is_empty(), "not watertight: {p:?}");
    assert!(m.volume() > 0.0);
    m
}

// ---- 1. one slab must equal the existing, separately tested extrude -------------------------

#[test]
fn a_single_slab_reproduces_extrude_exactly() {
    let box_ = rect("a", 0.0, 0.0, 80.0, 40.0);
    let cylinder = disc("k", 5.0, -3.0, 10.0);
    let holed = profile(|s| {
        for (i, (x, y)) in [(0.0, 0.0), (100.0, 0.0), (100.0, 100.0), (0.0, 100.0)]
            .iter()
            .enumerate()
        {
            s.add_entity(&format!("a{i}"), Entity::Point { x: *x, y: *y })
                .unwrap();
        }
        for i in 0..4 {
            s.add_entity(
                &format!("e{i}"),
                Entity::Line {
                    a: format!("a{i}"),
                    b: format!("a{}", (i + 1) % 4),
                },
            )
            .unwrap();
        }
        s.add_entity("m", Entity::Point { x: 50.0, y: 50.0 })
            .unwrap();
        s.add_entity(
            "h",
            Entity::Circle {
                center: "m".into(),
                radius: 20.0,
            },
        )
        .unwrap();
        s.add_entity("m2", Entity::Point { x: 20.0, y: 20.0 })
            .unwrap();
        s.add_entity(
            "h2",
            Entity::Circle {
                center: "m2".into(),
                radius: 6.0,
            },
        )
        .unwrap();
    });
    let dshape = profile(|s| {
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
    });
    let notched = profile(|s| {
        for (id, x, y) in [
            ("a", 0.0, 0.0),
            ("b", 40.0, 0.0),
            ("c", 40.0, 20.0),
            ("d", 25.0, 20.0),
            ("e", 15.0, 20.0),
            ("f", 0.0, 20.0),
            ("o", 20.0, 20.0),
        ] {
            s.add_entity(id, Entity::Point { x, y }).unwrap();
        }
        for (id, p, q) in [
            ("ab", "a", "b"),
            ("bc", "b", "c"),
            ("cd", "c", "d"),
            ("ef", "e", "f"),
            ("fa", "f", "a"),
        ] {
            s.add_entity(
                id,
                Entity::Line {
                    a: p.into(),
                    b: q.into(),
                },
            )
            .unwrap();
        }
        s.add_entity(
            "notch",
            Entity::Arc {
                center: "o".into(),
                start: "e".into(),
                end: "d".into(),
            },
        )
        .unwrap();
    });
    for (label, p) in [
        ("box", box_),
        ("cylinder", cylinder),
        ("two holes", holed),
        ("D", dshape),
        ("notch", notched),
    ] {
        for reverse in [false, true] {
            let old = extrude(&xy(), &p, 12.5, reverse).unwrap();
            let new = build(&Stepped::extrusion(xy(), p.clone(), 12.5, reverse));
            let tag = format!("{label} reverse={reverse}");
            assert!(new.validate().is_empty(), "{tag}");
            assert_eq!(
                (new.vertices.len(), new.edges.len(), new.faces.len()),
                (old.vertices.len(), old.edges.len(), old.faces.len()),
                "{tag}"
            );
            assert!(
                close(new.volume(), old.volume()) && close(new.area(), old.area()),
                "{tag}"
            );
            let (bo, bn) = (old.bounds().unwrap(), new.bounds().unwrap());
            for (a, b) in [
                (bo.min.x, bn.min.x),
                (bo.min.y, bn.min.y),
                (bo.min.z, bn.min.z),
                (bo.max.x, bn.max.x),
                (bo.max.y, bn.max.y),
                (bo.max.z, bn.max.z),
            ] {
                assert!((a - b).abs() < 1e-9, "{tag}: bounds differ");
            }
            assert_eq!(names(&new), names(&old), "{tag}: face names are unchanged");
            watertight(&new);
        }
    }
}

// ---- 2. independent inside/outside through the mesh ---------------------------------------

fn dist_to_polyline(q: Vec2, pl: &[Vec2]) -> f64 {
    (0..pl.len())
        .map(|i| {
            let (a, b) = (pl[i], pl[(i + 1) % pl.len()]);
            let ab = b - a;
            let t = ((q - a).dot(ab) / ab.length_squared().max(1e-300)).clamp(0.0, 1.0);
            (q - (a + ab * t)).length()
        })
        .fold(f64::MAX, f64::min)
}

/// Möller–Trumbore ray/triangle intersection count along a fixed generic direction.
fn mesh_contains(m: &Mesh, p: Vec3) -> bool {
    let d = Vec3::new(0.314_159, 0.592_653, 0.535_897);
    let mut hits = 0;
    for t in &m.triangles {
        let (a, b, c) = (
            m.positions[t[0] as usize],
            m.positions[t[1] as usize],
            m.positions[t[2] as usize],
        );
        let (e1, e2) = (b - a, c - a);
        let h = d.cross(e2);
        let det = e1.dot(h);
        if det.abs() < 1e-14 {
            continue;
        }
        let f = 1.0 / det;
        let s = p - a;
        let u = f * s.dot(h);
        if !(0.0..=1.0).contains(&u) {
            continue;
        }
        let q = s.cross(e1);
        let v = f * d.dot(q);
        if v < 0.0 || u + v > 1.0 {
            continue;
        }
        if f * e2.dot(q) > 1e-12 {
            hits += 1;
        }
    }
    hits % 2 == 1
}

/// Compares the mesh's inside/outside with the slab model at many probe points away from every
/// boundary: an independent check of the B-rep's topology and orientation.
fn assert_membership(st: &Stepped, solid: &Solid) {
    let mesh = watertight(solid);
    let polys: Vec<Vec<Vec2>> = st
        .slabs
        .iter()
        .flatten()
        .flat_map(|p| {
            let pr = p.prepare();
            pr.boundaries().cloned().collect::<Vec<_>>()
        })
        .collect();
    let prepared: Vec<Vec<_>> = st
        .slabs
        .iter()
        .map(|s| s.iter().map(Profile::prepare).collect())
        .collect();
    let b = solid.bounds().unwrap();
    let mut checked = 0;
    let mut z = b.min.z - 1.3;
    while z < b.max.z + 1.3 {
        let near_level = st.levels.iter().any(|l| (l.w - z).abs() < 0.25);
        if !near_level {
            let mut y = b.min.y - 1.7;
            while y < b.max.y + 1.7 {
                let mut x = b.min.x - 1.9;
                while x < b.max.x + 1.9 {
                    let q = Vec2::new(x, y);
                    if polys.iter().all(|pl| dist_to_polyline(q, pl) > 0.4) {
                        let slab = st.levels.windows(2).position(|w| z > w[0].w && z < w[1].w);
                        let want = slab.is_some_and(|k| prepared[k].iter().any(|p| p.contains(q)));
                        assert_eq!(
                            mesh_contains(&mesh, Vec3::new(x, y, z)),
                            want,
                            "probe ({x:.2}, {y:.2}, {z:.2})"
                        );
                        checked += 1;
                    }
                    x += 3.1;
                }
                y += 2.9;
            }
        }
        z += 1.9;
    }
    assert!(checked > 150, "only {checked} probes");
}

fn plate() -> Stepped {
    Stepped::extrusion(xy(), rect("a", 0.0, 0.0, 100.0, 60.0), 12.0, false)
}

// ---- 3. pockets, counterbores, bosses -------------------------------------------------------

#[test]
fn a_blind_pocket_has_a_floor() {
    let st = plate()
        .cut(&disc("t", 50.0, 30.0, 8.0), 12.0, 7.0, "p1")
        .unwrap();
    assert_eq!(
        st.levels.iter().map(|l| l.w).collect::<Vec<_>>(),
        [0.0, 7.0, 12.0]
    );
    let s = build(&st);
    assert!(
        close(s.volume(), 6000.0 * 12.0 - 64.0 * PI * 5.0),
        "{}",
        s.volume()
    );
    assert!(close(s.volume(), slab_volume(&st)));
    let n = names(&s);
    for want in [
        "cap_end",
        "cap_end:p1",
        "cap_start",
        "side:cut:p1:tc",
        "side:ae0",
    ] {
        assert!(n.iter().any(|x| x == want), "{want} in {n:?}");
    }
    // The floor faces up and is exactly the hole's area; the top is the plate minus the hole.
    let floor = s.faces.iter().find(|f| f.name == "cap_end:p1").unwrap();
    assert!(close(s.face_area(floor), 64.0 * PI) && floor.same_sense);
    let top = s.faces.iter().find(|f| f.name == "cap_end").unwrap();
    assert!(close(s.face_area(top), 6000.0 - 64.0 * PI));
    let wall = s.faces.iter().find(|f| f.name == "side:cut:p1:tc").unwrap();
    assert!(!wall.same_sense, "the pocket wall faces into the cavity");
    assert_membership(&st, &s);
}

#[test]
fn a_counterbored_hole_is_a_through_hole_and_a_wider_shallow_one() {
    let through = plate()
        .cut(&disc("h", 50.0, 30.0, 4.0), 12.0, f64::NEG_INFINITY, "c1")
        .unwrap();
    assert_eq!(through.levels.len(), 2, "a through cut adds no levels");
    let st = through
        .cut(&disc("b", 50.0, 30.0, 8.0), 12.0, 9.0, "c2")
        .unwrap();
    let s = build(&st);
    let exact = 6000.0 * 12.0 - 16.0 * PI * 12.0 - (64.0 - 16.0) * PI * 3.0;
    assert!(close(s.volume(), exact), "{} vs {exact}", s.volume());
    assert!(close(s.volume(), slab_volume(&st)));
    // Two cylinders and a ring-shaped step between them.
    let radii: Vec<f64> = s
        .faces
        .iter()
        .filter_map(|f| match f.surface {
            dc_kernel::Surface::Cylinder { radius, .. } => Some(radius),
            _ => None,
        })
        .collect();
    assert_eq!(radii.len(), 2);
    let step = s.faces.iter().find(|f| f.name == "cap_end:c2").unwrap();
    assert!(close(s.face_area(step), (64.0 - 16.0) * PI));
    assert_membership(&st, &s);
}

#[test]
fn a_boss_stands_on_the_top_face() {
    let st = plate()
        .boss(&rect("t", 30.0, 20.0, 70.0, 40.0), 12.0, 22.0, "b1")
        .unwrap();
    assert_eq!(st.levels.len(), 3);
    let s = build(&st);
    assert!(
        close(s.volume(), 72_000.0 + 40.0 * 20.0 * 10.0),
        "{}",
        s.volume()
    );
    assert!(close(s.volume(), slab_volume(&st)));
    let top = s.faces.iter().find(|f| f.name == "cap_end:b1").unwrap();
    assert!(close(s.face_area(top), 800.0));
    let base_top = s.faces.iter().find(|f| f.name == "cap_end").unwrap();
    assert!(
        close(s.face_area(base_top), 6000.0 - 800.0),
        "the base's top face is a ring around the boss"
    );
    assert_eq!(base_top.loops.len(), 2);
    assert_membership(&st, &s);
}

#[test]
fn a_boss_flush_with_the_part_edge_shares_walls_with_it() {
    // The boss sits in the corner: two of its sides continue the part's sides.
    let st = plate()
        .boss(&rect("t", 0.0, 0.0, 40.0, 20.0), 12.0, 22.0, "b1")
        .unwrap();
    let s = build(&st);
    assert!(
        close(s.volume(), 72_000.0 + 40.0 * 20.0 * 10.0),
        "{}",
        s.volume()
    );
    // The wall below and the boss wall above it are one face, 22 mm tall.
    let tall: Vec<_> = s
        .faces
        .iter()
        .filter(|f| {
            matches!(f.surface, dc_kernel::Surface::Plane { .. })
                && (s.face_area(f) - 22.0 * 40.0).abs() < 1e-6
        })
        .collect();
    assert!(!tall.is_empty(), "{:?}", names(&s));
    assert_membership(&st, &s);
}

#[test]
fn a_pocket_open_to_the_side_is_a_step() {
    let st = plate()
        .cut(&rect("t", 60.0, 20.0, 120.0, 40.0), 12.0, 6.0, "s1")
        .unwrap();
    let s = build(&st);
    assert!(
        close(s.volume(), 72_000.0 - 40.0 * 20.0 * 6.0),
        "{}",
        s.volume()
    );
    assert_membership(&st, &s);
}

#[test]
fn overlapping_tools_at_different_depths_cross_each_others_boundaries() {
    let st = plate()
        .cut(&rect("r", 20.0, 10.0, 60.0, 50.0), 12.0, 8.0, "c1")
        .unwrap()
        .cut(&disc("d", 60.0, 30.0, 15.0), 12.0, 4.0, "c2")
        .unwrap();
    assert_eq!(st.levels.len(), 4);
    let s = build(&st);
    assert!(
        close(s.volume(), slab_volume(&st)),
        "{} vs {}",
        s.volume(),
        slab_volume(&st)
    );
    assert_membership(&st, &s);
}

#[test]
fn a_cut_through_the_middle_of_the_height_splits_the_part_into_two_bodies() {
    let st = plate()
        .cut(&rect("g", -10.0, -10.0, 110.0, 70.0), 8.0, 4.0, "gap")
        .unwrap();
    assert!(st.slabs[1].is_empty(), "the middle slab has no material");
    let s = build(&st);
    assert!(close(s.volume(), 6000.0 * 8.0), "{}", s.volume());
    assert_eq!(
        s.faces
            .iter()
            .filter(|f| f.name.starts_with("cap_end"))
            .count(),
        2,
        "{:?}",
        names(&s)
    );
    assert_membership(&st, &s);
}

#[test]
fn stepped_towers_of_bosses() {
    let st = plate()
        .boss(&rect("a", 10.0, 10.0, 80.0, 50.0), 12.0, 20.0, "b1")
        .unwrap()
        .boss(&rect("b", 20.0, 15.0, 60.0, 45.0), 20.0, 30.0, "b2")
        .unwrap()
        .cut(&disc("h", 40.0, 30.0, 6.0), 30.0, 25.0, "h1")
        .unwrap();
    let s = build(&st);
    assert!(close(s.volume(), slab_volume(&st)));
    assert!(s.validate().is_empty());
    assert_membership(&st, &s);
}

// ---- 4. refusals ------------------------------------------------------------------------------

#[test]
fn operations_that_do_nothing_or_float_are_refused_with_reasons() {
    let p = plate();
    // Depth range entirely outside the part.
    let e = p
        .cut(&disc("t", 50.0, 30.0, 8.0), 100.0, 90.0, "x")
        .unwrap_err();
    assert!(
        matches!(&e, StepError::Nothing(m) if m.contains("do not overlap")),
        "{e}"
    );
    // Right depth, wrong place.
    let e = p
        .cut(&disc("t", 500.0, 500.0, 8.0), 12.0, 7.0, "x")
        .unwrap_err();
    assert!(
        matches!(&e, StepError::Nothing(m) if m.contains("remove nothing")),
        "{e}"
    );
    // Swallowing everything.
    let e = p
        .cut(
            &rect("t", -5.0, -5.0, 200.0, 200.0),
            12.0,
            f64::NEG_INFINITY,
            "x",
        )
        .unwrap_err();
    assert!(
        matches!(&e, StepError::Nothing(m) if m.contains("nothing would be left")),
        "{e}"
    );
    // A boss that floats, and one buried in material.
    let e = p
        .boss(&rect("t", 300.0, 300.0, 340.0, 340.0), 12.0, 22.0, "x")
        .unwrap_err();
    assert!(matches!(e, StepError::NoContact(_)), "{e}");
    let e = p
        .boss(&rect("t", 300.0, 300.0, 340.0, 340.0), 40.0, 50.0, "x")
        .unwrap_err();
    assert!(matches!(e, StepError::NoContact(_)), "{e}");
    let e = p
        .boss(&rect("t", 20.0, 20.0, 40.0, 40.0), 2.0, 6.0, "x")
        .unwrap_err();
    assert!(
        matches!(&e, StepError::Nothing(m) if m.contains("adds nothing")),
        "{e}"
    );
    let e = p
        .boss(&rect("t", 20.0, 20.0, 40.0, 40.0), 12.0, f64::INFINITY, "x")
        .unwrap_err();
    assert!(matches!(e, StepError::Nothing(_)), "{e}");
    // Tangent contact is refused, not guessed.
    let e = p
        .cut(&disc("t", 30.0, 30.0, 30.0), 12.0, 7.0, "x")
        .unwrap_err();
    assert!(matches!(e, StepError::Region(_)), "{e}");
}

#[test]
fn building_is_deterministic_and_levels_are_merged_when_cuts_cancel_out() {
    let a = plate()
        .cut(&disc("t", 50.0, 30.0, 8.0), 12.0, 7.0, "p1")
        .unwrap();
    assert_eq!(
        build(&a).canonical_bytes(),
        build(&a.clone()).canonical_bytes()
    );
    // A through cut applied slab by slab leaves one merged slab, not two identical ones.
    let split = plate()
        .cut(&disc("a", 20.0, 30.0, 5.0), 12.0, 6.0, "c1")
        .unwrap();
    let both = split
        .cut(&disc("a", 20.0, 30.0, 5.0), 6.0, f64::NEG_INFINITY, "c2")
        .unwrap_or(split);
    assert!(both.levels.len() <= 3);
}

/// Names must stay unique without suffixes, and each feature's walls carry its own name, or a
/// face reference could silently point at the wrong wall after a rebuild.
#[test]
fn walls_are_named_after_the_feature_that_made_them_with_no_collisions() {
    let st = plate()
        .boss(&rect("t", 30.0, 20.0, 70.0, 40.0), 12.0, 22.0, "b1")
        .unwrap()
        .cut(&disc("h", 50.0, 30.0, 5.0), 22.0, 16.0, "pk")
        .unwrap();
    let s = build(&st);
    let n = names(&s);
    assert!(
        n.iter().all(|x| !x.contains('#')),
        "no uniqueness suffixes were needed: {n:?}"
    );
    let boss_walls: Vec<_> = n
        .iter()
        .filter(|x| x.starts_with("side:boss:b1:"))
        .collect();
    assert_eq!(boss_walls.len(), 4, "{n:?}");
    assert!(n.iter().any(|x| x == "side:cut:pk:hc"), "{n:?}");
    // And the part's own walls keep their original names.
    for e in ["side:ae0", "side:ae1", "side:ae2", "side:ae3"] {
        assert!(n.iter().any(|x| x == e), "{e} in {n:?}");
    }
}
