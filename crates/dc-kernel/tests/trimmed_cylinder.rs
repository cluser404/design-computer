//! Faces trimmed by arbitrary loops on a cylinder: exact area and volume from the boundary, an
//! ellipse edge, and a watertight mesh. The solids are built by hand (no boolean needed) so the
//! geometry has an independent closed-form answer.

use dc_kernel::{CoEdge, Curve, Edge, Face, Solid, Surface, TessParams};
use dc_math::{det, Vec3};

const R: f64 = 3.0;
const H0: f64 = 10.0;

fn v(x: f64, y: f64, z: f64) -> Vec3 {
    Vec3::new(x, y, z)
}

/// A cylinder of radius `R` about z from z = 0 up to the plane `z = H0 + k x`.
fn slanted(k: f64) -> Solid {
    let pi2 = 2.0 * core::f64::consts::PI;
    let top_seam = H0 + k * R;
    let u = v(R, 0.0, k * R);
    let a = u.length();
    let ux = u / a;
    let w = v(0.0, 1.0, 0.0);
    let axis = ux.cross(w);
    let edges = vec![
        Edge {
            name: "bottom".into(),
            v0: 0,
            v1: 0,
            curve: Curve::Circle {
                center: v(0.0, 0.0, 0.0),
                axis: v(0.0, 0.0, 1.0),
                x_dir: v(1.0, 0.0, 0.0),
                radius: R,
                t0: 0.0,
                sweep: pi2,
            },
        },
        Edge {
            name: "top".into(),
            v0: 1,
            v1: 1,
            curve: Curve::Ellipse {
                center: v(0.0, 0.0, H0),
                axis,
                x_dir: ux,
                a,
                b: R,
                t0: 0.0,
                sweep: pi2,
            },
        },
        Edge {
            name: "seam".into(),
            v0: 0,
            v1: 1,
            curve: Curve::Line,
        },
    ];
    let c = |edge, reversed| CoEdge { edge, reversed };
    let faces = vec![
        Face {
            name: "bottom".into(),
            surface: Surface::Plane {
                origin: v(0.0, 0.0, 0.0),
                normal: v(0.0, 0.0, -1.0),
                x_dir: v(1.0, 0.0, 0.0),
            },
            same_sense: true,
            loops: vec![vec![c(0, true)]],
        },
        Face {
            name: "top".into(),
            surface: Surface::Plane {
                origin: v(0.0, 0.0, H0),
                normal: axis,
                x_dir: ux,
            },
            same_sense: true,
            loops: vec![vec![c(1, false)]],
        },
        Face {
            name: "side".into(),
            surface: Surface::Cylinder {
                origin: v(0.0, 0.0, 0.0),
                axis: v(0.0, 0.0, 1.0),
                ref_dir: v(1.0, 0.0, 0.0),
                radius: R,
                phi0: 0.0,
                phi1: pi2,
                h0: 0.0,
                h1: top_seam,
            },
            same_sense: true,
            loops: vec![vec![c(0, false), c(2, false), c(1, true), c(2, true)]],
        },
    ];
    Solid {
        vertices: vec![v(R, 0.0, 0.0), v(R, 0.0, top_seam)],
        edges,
        faces,
    }
}

fn close(a: f64, b: f64, tol: f64) -> bool {
    (a - b).abs() <= tol * b.abs().max(1.0)
}

#[test]
fn a_cylinder_cut_by_a_slanted_plane_has_exact_measures() {
    for k in [0.0, 0.25, 0.5, -0.4] {
        let s = slanted(k);
        let issues = s.validate();
        assert!(issues.is_empty(), "k={k}: {issues:?}");
        let pi = core::f64::consts::PI;
        // The slanted top passes through the axis at height H0, so the mean height is H0.
        assert!(
            close(s.volume(), pi * R * R * H0, 1e-9),
            "k={k}: volume {}",
            s.volume()
        );
        let side = 2.0 * pi * R * H0;
        let top = pi * R * R * det::sqrt(1.0 + k * k);
        assert!(
            close(s.area(), side + pi * R * R + top, 1e-9),
            "k={k}: area {}",
            s.area()
        );
        assert!(close(s.face_area(&s.faces[2]), side, 1e-9), "side face");
        assert!(
            close(s.face_area(&s.faces[1]), top, 1e-9),
            "top ellipse face"
        );
    }
}

#[test]
fn the_bounds_of_a_slanted_cylinder_include_the_ellipse_extremes() {
    let s = slanted(0.5);
    let b = s.bounds().unwrap();
    assert!((b.min - v(-R, -R, 0.0)).length() < 1e-9, "{:?}", b.min);
    // Highest point of the top ellipse: H0 + k R at x = R.
    assert!(
        (b.max - v(R, R, H0 + 0.5 * R)).length() < 1e-9,
        "{:?}",
        b.max
    );
}

#[test]
fn an_ellipse_edge_has_the_right_length_and_points() {
    let s = slanted(0.5);
    let a = (R * R + (0.5 * R) * (0.5 * R)).sqrt();
    // Ramanujan's approximation is within 1e-4 relative error here.
    let q = (a - R) / (a + R);
    let h = q * q;
    let want = core::f64::consts::PI * (a + R) * (1.0 + 3.0 * h / (10.0 + (4.0 - 3.0 * h).sqrt()));
    let got = s.edge_length(1);
    assert!((got - want).abs() / want < 1e-4, "{got} vs {want}");
    let p = s.edge_point(1, 0.25);
    assert!((p - v(0.0, R, H0)).length() < 1e-9, "{p:?}");
}

#[test]
fn a_slanted_cylinder_tessellates_watertight_with_the_right_volume() {
    let s = slanted(0.5);
    let m = s.tessellate(TessParams::default()).expect("tessellates");
    assert!(m.check().is_empty(), "{:?}", m.check());
    let exact = s.volume();
    assert!(
        (m.volume() - exact).abs() / exact < 6e-3,
        "mesh {} vs {exact}",
        m.volume()
    );
    assert!(
        (m.area() - s.area()).abs() / s.area() < 6e-3,
        "mesh area {} vs {}",
        m.area(),
        s.area()
    );
}

#[test]
fn the_mesh_converges_to_the_exact_volume_as_the_tolerance_shrinks() {
    let s = slanted(0.5);
    let exact = s.volume();
    let mut last = f64::MAX;
    for chord_tol in [0.05, 0.01, 0.002] {
        let m = s
            .tessellate(TessParams {
                chord_tol,
                max_angle: core::f64::consts::PI,
            })
            .expect("tessellates");
        assert!(m.check().is_empty());
        let err = (m.volume() - exact).abs() / exact;
        assert!(
            err < last,
            "error {err} did not shrink from {last} at chord_tol {chord_tol}"
        );
        last = err;
    }
    assert!(last < 1e-3, "{last}");
}

#[test]
fn the_general_trimmed_path_agrees_with_the_strip_path() {
    for k in [0.0, 0.5, -0.4] {
        let s = slanted(k);
        let strip = s.tessellate(TessParams::default()).unwrap();
        let general = s.tessellate_with(TessParams::default(), true).unwrap();
        assert!(general.check().is_empty(), "k={k}: {:?}", general.check());
        assert!(general.volume() > 0.0, "k={k}: faces point outward");
        // The wall is zipped between its two edges (two thin triangles per segment), not ear
        // clipped into long slivers: the same triangle count as the strip.
        let side = |m: &dc_kernel::Mesh| m.face_of.iter().filter(|&&f| f == 2).count();
        assert_eq!(side(&general), side(&strip), "k={k}: wall triangles");
        // Both are inscribed polylines of the same surface: they agree closely.
        assert!(
            (general.volume() - strip.volume()).abs() / strip.volume() < 5e-3,
            "k={k}: {} vs {}",
            general.volume(),
            strip.volume()
        );
        assert!(
            (general.area() - strip.area()).abs() / strip.area() < 5e-3,
            "k={k}"
        );
    }
}
