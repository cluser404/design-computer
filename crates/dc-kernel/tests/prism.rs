use dc_kernel::Solid;
use dc_math::Vec3;

fn v(x: f64, y: f64, z: f64) -> Vec3 {
    Vec3::new(x, y, z)
}

#[test]
fn cuboid_is_valid_with_exact_measures() {
    let s = Solid::cuboid(v(0.0, 0.0, 0.0), v(2.0, 3.0, 4.0)).unwrap();
    assert!(s.validate().is_empty(), "{:?}", s.validate());
    assert!((s.volume() - 24.0).abs() < 1e-12);
    assert!((s.area() - 2.0 * (6.0 + 8.0 + 12.0)).abs() < 1e-12);
    assert_eq!(s.faces.len(), 6);
    assert_eq!(s.edges.len(), 12);
}

#[test]
fn prism_with_hole_and_slanted_direction_is_valid() {
    let outer = [
        v(0.0, 0.0, 0.0),
        v(10.0, 0.0, 0.0),
        v(10.0, 10.0, 0.0),
        v(0.0, 10.0, 0.0),
    ];
    let hole = vec![
        v(3.0, 3.0, 0.0),
        v(7.0, 3.0, 0.0),
        v(7.0, 7.0, 0.0),
        v(3.0, 7.0, 0.0),
    ];
    // A slanted sweep keeps the volume at base area times the height.
    let s = Solid::prism(&outer, &[hole], v(2.0, 1.0, 5.0)).unwrap();
    assert!(s.validate().is_empty(), "{:?}", s.validate());
    assert!(
        (s.volume() - (100.0 - 16.0) * 5.0).abs() < 1e-9,
        "{}",
        s.volume()
    );
    assert_eq!(s.faces.len(), 2 + 8);
}

#[test]
fn prism_accepts_either_winding_and_direction() {
    let tri = [v(0.0, 0.0, 0.0), v(4.0, 0.0, 0.0), v(0.0, 3.0, 0.0)];
    let up = Solid::prism(&tri, &[], v(0.0, 0.0, 2.0)).unwrap();
    let down = Solid::prism(&tri, &[], v(0.0, 0.0, -2.0)).unwrap();
    for s in [&up, &down] {
        assert!(s.validate().is_empty(), "{:?}", s.validate());
        assert!((s.volume() - 12.0).abs() < 1e-12);
    }
    let mut rev = tri.to_vec();
    rev.reverse();
    let r = Solid::prism(&rev, &[], v(0.0, 0.0, 2.0)).unwrap();
    assert!((r.volume() - 12.0).abs() < 1e-12);
    assert!(
        Solid::prism(&tri, &[], v(1.0, 1.0, 0.0)).is_none(),
        "in-plane sweep is refused"
    );
}

#[test]
fn a_loft_between_a_square_and_a_smaller_parallel_square_is_a_frustum() {
    let sq = |s: f64, z: f64| [v(-s, -s, z), v(s, -s, z), v(s, s, z), v(-s, s, z)];
    let names: Vec<String> = (0..4).map(|i| format!("e{i}")).collect();
    let f = Solid::loft(&sq(5.0, 0.0), &sq(2.0, 6.0), &names).unwrap();
    assert!(f.validate().is_empty(), "{:?}", f.validate());
    // Frustum volume: h/3 (A1 + A2 + sqrt(A1 A2)).
    let (a1, a2) = (100.0, 16.0);
    assert!(
        (f.volume() - 6.0 / 3.0 * (a1 + a2 + (a1 * a2f()).sqrt())).abs() < 1e-9,
        "{}",
        f.volume()
    );
    assert_eq!(f.faces.len(), 6);
    // The same with the top turned and given the other way round.
    let mut top = sq(2.0, 6.0).to_vec();
    top.reverse();
    top.rotate_left(1);
    let g = Solid::loft(&sq(5.0, 0.0), &top, &names).unwrap();
    assert!((g.volume() - f.volume()).abs() < 1e-9);
    let m = g.tessellate(dc_kernel::TessParams::default()).unwrap();
    assert!(m.check().is_empty());
}

fn a2f() -> f64 {
    16.0
}

#[test]
fn a_twisted_loft_is_refused_with_the_reason() {
    let names: Vec<String> = (0..4).map(|i| format!("e{i}")).collect();
    let bottom = [
        v(-5.0, -5.0, 0.0),
        v(5.0, -5.0, 0.0),
        v(5.0, 5.0, 0.0),
        v(-5.0, 5.0, 0.0),
    ];
    // The top square turned by 30 degrees: its edges are no longer parallel to the bottom's.
    let (c, s) = (0.866_025_403_784_438_6, 0.5);
    let turned = |x: f64, y: f64| v(x * c - y * s, x * s + y * c, 6.0);
    let top = [
        turned(-3.0, -3.0),
        turned(3.0, -3.0),
        turned(3.0, 3.0),
        turned(-3.0, 3.0),
    ];
    let e = Solid::loft(&bottom, &top, &names).unwrap_err();
    assert!(e.contains("twisted"), "{e}");
    let e = Solid::loft(&bottom, &top[..3], &names).unwrap_err();
    assert!(e.contains("same number"), "{e}");
}
