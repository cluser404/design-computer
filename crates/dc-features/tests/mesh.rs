//! Tessellation and STL: watertight, outward-facing, and consistent with the exact B-rep.

use dc_features::extrude;
use dc_kernel::{Mesh, Solid, Surface, TessParams};
use dc_sketch::{Entity, PlaneSpec, Sketch};

fn pt(s: &mut Sketch, id: &str, x: f64, y: f64) {
    s.add_entity(id, Entity::Point { x, y }).unwrap();
}
fn line(s: &mut Sketch, id: &str, a: &str, b: &str) {
    s.add_entity(
        id,
        Entity::Line {
            a: a.into(),
            b: b.into(),
        },
    )
    .unwrap();
}
fn circle(s: &mut Sketch, id: &str, c: &str, r: f64) {
    s.add_entity(
        id,
        Entity::Circle {
            center: c.into(),
            radius: r,
        },
    )
    .unwrap();
}
fn rect(s: &mut Sketch, p: &str, x0: f64, y0: f64, x1: f64, y1: f64) {
    for (k, (x, y)) in [(x0, y0), (x1, y0), (x1, y1), (x0, y1)].iter().enumerate() {
        pt(s, &format!("{p}{}", k + 1), *x, *y);
    }
    for k in 0..4 {
        line(
            s,
            &format!("{p}{}", (b'a' + k as u8) as char),
            &format!("{p}{}", k + 1),
            &format!("{p}{}", (k + 1) % 4 + 1),
        );
    }
}
fn solid(s: &Sketch, depth: f64, reverse: bool) -> Solid {
    let r = s.profiles();
    assert!(r.problems.is_empty(), "{:?}", r.problems);
    assert_eq!(r.profiles.len(), 1);
    extrude(&s.named_plane().unwrap(), &r.profiles[0], depth, reverse).unwrap()
}
fn mesh(s: &Solid) -> Mesh {
    let m = s
        .tessellate(TessParams::default())
        .unwrap_or_else(|e| panic!("{e}"));
    let problems = m.check();
    assert!(problems.is_empty(), "not watertight: {problems:?}");
    m
}
fn rel(a: f64, b: f64) -> f64 {
    (a - b).abs() / b.abs().max(1.0)
}

fn shapes() -> Vec<(&'static str, Solid)> {
    let mut out = Vec::new();

    let mut s = Sketch::new();
    rect(&mut s, "r", 0.0, 0.0, 80.0, 40.0);
    out.push(("box", solid(&s, 20.0, false)));
    out.push(("box reversed", solid(&s, 20.0, true)));

    let mut s = Sketch::new();
    pt(&mut s, "c", 5.0, -3.0);
    circle(&mut s, "k", "c", 10.0);
    out.push(("cylinder", solid(&s, 30.0, false)));
    out.push(("cylinder reversed", solid(&s, 30.0, true)));

    let mut s = Sketch::new();
    rect(&mut s, "r", 0.0, 0.0, 100.0, 100.0);
    pt(&mut s, "c", 50.0, 50.0);
    circle(&mut s, "h", "c", 20.0);
    out.push(("block with hole", solid(&s, 10.0, false)));
    out.push(("block with hole reversed", solid(&s, 10.0, true)));

    let mut s = Sketch::new();
    pt(&mut s, "c", 0.0, 0.0);
    circle(&mut s, "outer", "c", 10.0);
    circle(&mut s, "inner", "c", 4.0);
    out.push(("annulus (concentric)", solid(&s, 5.0, false)));

    let mut s = Sketch::new();
    pt(&mut s, "c", 0.0, 0.0);
    pt(&mut s, "l", -10.0, 0.0);
    pt(&mut s, "r", 10.0, 0.0);
    s.add_entity(
        "arc",
        Entity::Arc {
            center: "c".into(),
            start: "r".into(),
            end: "l".into(),
        },
    )
    .unwrap();
    line(&mut s, "base", "l", "r");
    out.push(("half disc", solid(&s, 4.0, false)));

    let mut s = Sketch::new();
    for (id, x, y) in [
        ("a", 0.0, 0.0),
        ("b", 40.0, 0.0),
        ("c", 40.0, 20.0),
        ("d", 25.0, 20.0),
        ("e", 15.0, 20.0),
        ("f", 0.0, 20.0),
        ("o", 20.0, 20.0),
    ] {
        pt(&mut s, id, x, y);
    }
    line(&mut s, "ab", "a", "b");
    line(&mut s, "bc", "b", "c");
    line(&mut s, "cd", "c", "d");
    s.add_entity(
        "notch",
        Entity::Arc {
            center: "o".into(),
            start: "e".into(),
            end: "d".into(),
        },
    )
    .unwrap();
    line(&mut s, "ef", "e", "f");
    line(&mut s, "fa", "f", "a");
    out.push(("block with round notch", solid(&s, 10.0, false)));

    let mut s = Sketch::new();
    rect(&mut s, "r", 0.0, 0.0, 100.0, 100.0);
    for i in 0..5 {
        for j in 0..5 {
            let id = format!("c{i}{j}");
            pt(&mut s, &id, 12.0 + 19.0 * i as f64, 12.0 + 19.0 * j as f64);
            circle(&mut s, &format!("h{i}{j}"), &id, 6.0);
        }
    }
    out.push(("5x5 grid of holes", solid(&s, 8.0, false)));

    let mut s = Sketch::on_plane(PlaneSpec::Yz { offset: 3.0 }).unwrap();
    rect(&mut s, "r", 0.0, 0.0, 30.0, 20.0);
    pt(&mut s, "c", 15.0, 10.0);
    circle(&mut s, "h", "c", 5.0);
    out.push(("holed plate on YZ", solid(&s, 7.0, true)));
    out
}

#[test]
fn every_shape_tessellates_watertight_and_matches_the_exact_measures() {
    for (name, s) in shapes() {
        assert!(s.validate().is_empty(), "{name}");
        let m = mesh(&s);
        assert!(
            m.volume() > 0.0,
            "{name}: mesh is inside out ({})",
            m.volume()
        );
        // A polygon inscribed in a circle is slightly smaller; with the default tolerance the
        // mesh must still be within 0.5% of the exact B-rep value.
        assert!(
            rel(m.volume(), s.volume()) < 5e-3,
            "{name}: mesh volume {} vs exact {}",
            m.volume(),
            s.volume()
        );
        assert!(
            rel(m.area(), s.area()) < 5e-3,
            "{name}: mesh area {} vs exact {}",
            m.area(),
            s.area()
        );
        // The direction of the error is predictable: an inscribed polygon shrinks material bounded
        // by an outward arc (cylinder) and enlarges material around a hole (the hole shrinks).
        match name {
            "cylinder" | "cylinder reversed" | "half disc" => {
                assert!(m.volume() <= s.volume(), "{name}")
            }
            // (A concentric annulus has both kinds of boundary and no predictable sign.)
            "block with hole" | "block with hole reversed" | "5x5 grid of holes" => {
                assert!(m.volume() >= s.volume(), "{name}")
            }
            _ => {}
        }
        assert_eq!(m.face_of.len(), m.triangles.len());
    }
}

#[test]
fn planar_solids_are_exact_and_minimal() {
    let mut s = Sketch::new();
    rect(&mut s, "r", 0.0, 0.0, 80.0, 40.0);
    let b = solid(&s, 20.0, false);
    let m = mesh(&b);
    assert_eq!(m.triangles.len(), 12);
    assert_eq!(m.positions.len(), 8);
    assert!(rel(m.volume(), 64_000.0) < 1e-12 && rel(m.area(), 11_200.0) < 1e-12);
}

#[test]
fn every_triangle_faces_outward() {
    for (name, s) in shapes() {
        let m = mesh(&s);
        for (t, tri) in m.triangles.iter().enumerate() {
            let face = &s.faces[m.face_of[t] as usize];
            let (a, b, c) = (
                m.positions[tri[0] as usize],
                m.positions[tri[1] as usize],
                m.positions[tri[2] as usize],
            );
            let n = (b - a).cross(c - a);
            assert!(
                n.length() > 0.0,
                "{name}: zero-area triangle in '{}'",
                face.name
            );
            let centroid = (a + b + c) / 3.0;
            let expected = match &face.surface {
                Surface::Plane { normal, .. } => *normal * if face.same_sense { 1.0 } else { -1.0 },
                Surface::Cylinder { origin, axis, .. } => {
                    let d = centroid - *origin;
                    (d - *axis * d.dot(*axis)) * if face.same_sense { 1.0 } else { -1.0 }
                }
                // These shapes have no rounded rims; the rim tests check the other surfaces.
                Surface::Cone { .. } | Surface::Torus { .. } => unreachable!("not built here"),
            };
            assert!(
                n.dot(expected) > 0.0,
                "{name}: triangle {t} of '{}' faces the wrong way",
                face.name
            );
        }
    }
}

#[test]
fn finer_tolerance_gives_a_closer_mesh_and_stays_watertight() {
    let (_, s) = shapes()
        .into_iter()
        .find(|(n, _)| *n == "cylinder")
        .unwrap();
    let coarse = s
        .tessellate(TessParams {
            chord_tol: 0.5,
            max_angle: 1.0,
        })
        .unwrap();
    let fine = s
        .tessellate(TessParams {
            chord_tol: 0.001,
            max_angle: 0.1,
        })
        .unwrap();
    assert!(coarse.check().is_empty() && fine.check().is_empty());
    assert!(fine.triangles.len() > coarse.triangles.len());
    assert!(rel(fine.volume(), s.volume()) < rel(coarse.volume(), s.volume()));
    assert!(rel(fine.volume(), s.volume()) < 1e-3);
}

#[test]
fn stl_round_trips_and_is_deterministic() {
    let (_, s) = shapes()
        .into_iter()
        .find(|(n, _)| *n == "block with hole")
        .unwrap();
    let m = mesh(&s);
    let bin = m.to_binary_stl();
    assert_eq!(bin.len(), 84 + 50 * m.triangles.len());
    assert!(bin.starts_with(b"Design Computer STL"));
    assert_eq!(
        u32::from_le_bytes(bin[80..84].try_into().unwrap()) as usize,
        m.triangles.len()
    );
    // Parse it back and recompute the volume from the file alone.
    let f = |o: usize| f32::from_le_bytes(bin[o..o + 4].try_into().unwrap()) as f64;
    let mut vol = 0.0;
    for t in 0..m.triangles.len() {
        let base = 84 + 50 * t;
        let v = |k: usize| {
            dc_math::Vec3::new(
                f(base + 12 + 12 * k),
                f(base + 16 + 12 * k),
                f(base + 20 + 12 * k),
            )
        };
        let (a, b, c) = (v(0), v(1), v(2));
        vol += a.dot(b.cross(c)) / 6.0;
        let n = dc_math::Vec3::new(f(base), f(base + 4), f(base + 8));
        assert!(
            (n.length() - 1.0).abs() < 1e-5,
            "stored normals are unit length"
        );
        assert!(
            n.dot((b - a).cross(c - a)) > 0.0,
            "stored normal agrees with winding"
        );
    }
    assert!(rel(vol, m.volume()) < 1e-5, "{vol} vs {}", m.volume());
    // Same input, same bytes, regardless of when or where.
    assert_eq!(bin, mesh(&s.clone()).to_binary_stl());
    let ascii = m.to_ascii_stl("part");
    assert_eq!(ascii.matches("facet normal").count(), m.triangles.len());
    assert!(ascii.starts_with("solid part\n") && ascii.ends_with("endsolid part\n"));
}

#[test]
fn tessellation_is_deterministic() {
    for (name, s) in shapes() {
        let (a, b) = (mesh(&s), mesh(&s.clone()));
        assert_eq!(a, b, "{name}");
    }
}
