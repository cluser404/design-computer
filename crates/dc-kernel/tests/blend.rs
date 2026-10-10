//! Fillets and chamfers of straight edges between flat faces: exact volumes, validity, watertight
//! meshes, the cases that must be refused, and the result working as input to the boolean.

use core::f64::consts::PI;
use dc_kernel::boolean::BoolOp;
use dc_kernel::{blend_edges, BlendError, BlendKind, Solid, TessParams};
use dc_math::{det, Vec3};

fn v(x: f64, y: f64, z: f64) -> Vec3 {
    Vec3::new(x, y, z)
}

fn block() -> Solid {
    Solid::cuboid(v(0.0, 0.0, 0.0), v(20.0, 20.0, 10.0)).unwrap()
}

fn pair(a: &str, b: &str) -> Vec<[String; 2]> {
    vec![[a.to_owned(), b.to_owned()]]
}

fn vol(s: &Solid, want: f64) {
    assert!(
        (s.volume() - want).abs() < 1e-7 * want.abs().max(1.0),
        "volume {} vs {want}",
        s.volume()
    );
}

fn well_formed(s: &Solid) {
    assert!(s.validate().is_empty(), "{:?}", s.validate());
    let names: std::collections::BTreeSet<&String> = s.faces.iter().map(|f| &f.name).collect();
    assert_eq!(names.len(), s.faces.len(), "unique face names");
    let enames: std::collections::BTreeSet<&String> = s.edges.iter().map(|e| &e.name).collect();
    assert_eq!(enames.len(), s.edges.len(), "unique edge names");
    let m = s.tessellate(TessParams::default()).expect("tessellates");
    assert!(m.check().is_empty(), "{:?}", m.check());
    assert!(
        (m.volume() - s.volume()).abs() / s.volume() < 5e-3,
        "mesh {} vs {}",
        m.volume(),
        s.volume()
    );
}

#[test]
fn a_vertical_edge_of_a_block_is_rounded() {
    let r = blend_edges(
        &block(),
        &pair("side:0", "side:1"),
        BlendKind::Fillet,
        3.0,
        "f1",
    )
    .unwrap();
    well_formed(&r);
    vol(&r, 2000.0 * 2.0 - (1.0 - PI / 4.0) * 9.0 * 10.0);
    assert_eq!(r.faces.len(), 7);
    assert!(r.faces.iter().any(|f| f.name == "fillet:f1"));
    // The two faces keep their names; the end faces lose a quarter circle's corner each.
    for n in ["side:0", "side:1", "top", "bottom"] {
        assert!(r.faces.iter().any(|f| f.name == n), "{n}");
    }
    assert!(r
        .edges
        .iter()
        .any(|e| matches!(e.curve, dc_kernel::Curve::Circle { .. })));
}

#[test]
fn a_vertical_edge_of_a_block_is_chamfered() {
    let r = blend_edges(
        &block(),
        &pair("side:0", "side:1"),
        BlendKind::Chamfer,
        3.0,
        "c1",
    )
    .unwrap();
    well_formed(&r);
    vol(&r, 4000.0 - 0.5 * 9.0 * 10.0);
    assert!(r.faces.iter().any(|f| f.name == "chamfer:c1"));
}

#[test]
fn a_top_edge_is_rounded_along_its_whole_length() {
    let r = blend_edges(
        &block(),
        &pair("top", "side:2"),
        BlendKind::Fillet,
        2.0,
        "f",
    )
    .unwrap();
    well_formed(&r);
    vol(&r, 4000.0 - (1.0 - PI / 4.0) * 4.0 * 20.0);
}

#[test]
fn an_inside_corner_gains_material() {
    // An L-shaped part: the block minus a corner block; the inner vertical edge is concave.
    let corner = Solid::cuboid(v(10.0, 10.0, -1.0), v(21.0, 21.0, 11.0)).unwrap();
    let l = block().boolean(&corner, BoolOp::Subtract).unwrap().solid;
    vol(&l, 3000.0);
    // The concave edge is between the two faces the cut made.
    let names: Vec<String> = l.faces.iter().map(|f| f.name.clone()).collect();
    let cut: Vec<&String> = names.iter().filter(|n| n.starts_with("tool:")).collect();
    assert_eq!(cut.len(), 2, "{names:?}");
    let r = blend_edges(&l, &pair(cut[0], cut[1]), BlendKind::Fillet, 3.0, "in").unwrap();
    well_formed(&r);
    vol(&r, 3000.0 + (1.0 - PI / 4.0) * 9.0 * 10.0);
    let fillet = r.faces.iter().find(|f| f.name == "fillet:in").unwrap();
    assert!(!fillet.same_sense, "its outward normal points at the axis");
}

#[test]
fn a_sharp_corner_is_rounded_exactly() {
    // A 60 degree wedge prism: the removed area at the corner is r^2 (cot(phi/2) - (pi - phi)/2).
    let poly = [
        v(0.0, 0.0, 0.0),
        v(30.0, 0.0, 0.0),
        v(15.0, 15.0 * det::sqrt(3.0), 0.0),
    ];
    let wedge = Solid::prism(&poly, &[], v(0.0, 0.0, 10.0)).unwrap();
    let area = 0.5 * 30.0 * 15.0 * det::sqrt(3.0);
    let r = 2.0;
    let phi = PI / 3.0;
    let cut = r * r * (1.0 / det::tan(phi / 2.0) - (PI - phi) / 2.0);
    let names: Vec<String> = wedge.faces.iter().map(|f| f.name.clone()).collect();
    let b = blend_edges(
        &wedge,
        &pair(&names[2], &names[3]),
        BlendKind::Fillet,
        r,
        "w",
    )
    .unwrap();
    well_formed(&b);
    vol(&b, (area - cut) * 10.0);
}

#[test]
fn slanted_end_faces_give_elliptical_arcs() {
    // Sweep the base square along a slanted direction: the vertical-ish edges meet the top and
    // bottom obliquely. The volume change is the corner's cross-section times the edge length.
    let poly = [
        v(0.0, 0.0, 0.0),
        v(20.0, 0.0, 0.0),
        v(20.0, 20.0, 0.0),
        v(0.0, 20.0, 0.0),
    ];
    let dir = v(4.0, 0.0, 10.0);
    let slab = Solid::prism(&poly, &[], dir).unwrap();
    let names: Vec<String> = slab.faces.iter().map(|f| f.name.clone()).collect();
    // The edge at the corner (20, 0): between side:0 and side:1.
    let a = names.iter().find(|n| *n == "side:0").unwrap();
    let b = names.iter().find(|n| *n == "side:1").unwrap();
    let r = blend_edges(&slab, &pair(a, b), BlendKind::Fillet, 2.0, "s").unwrap();
    well_formed(&r);
    let len = dir.length();
    vol(&r, slab.volume() - (1.0 - PI / 4.0) * 4.0 * len);
    assert!(
        r.edges
            .iter()
            .any(|e| matches!(e.curve, dc_kernel::Curve::Ellipse { .. })),
        "an ellipse arc"
    );
}

#[test]
fn two_edges_can_be_blended_in_one_go_if_they_do_not_touch() {
    let pairs = vec![
        ["side:0".to_owned(), "side:1".to_owned()],
        ["side:2".to_owned(), "side:3".to_owned()],
    ];
    let r = blend_edges(&block(), &pairs, BlendKind::Fillet, 2.0, "two").unwrap();
    well_formed(&r);
    vol(&r, 4000.0 - 2.0 * (1.0 - PI / 4.0) * 4.0 * 10.0);
    assert!(
        r.faces.iter().any(|f| f.name == "fillet:two")
            && r.faces.iter().any(|f| f.name == "fillet:two~1")
    );
}

#[test]
fn what_cannot_be_blended_is_refused_with_the_reason() {
    let b = block();
    let e = blend_edges(&b, &pair("side:0", "nowhere"), BlendKind::Fillet, 1.0, "x").unwrap_err();
    assert!(matches!(e, BlendError::NotFound(_)), "{e:?}");
    let e = blend_edges(&b, &pair("side:0", "side:2"), BlendKind::Fillet, 1.0, "x").unwrap_err();
    assert!(
        matches!(e, BlendError::NotFound(_)),
        "faces that do not touch: {e:?}"
    );
    // Too big: the corner is only 20 long.
    let e = blend_edges(&b, &pair("side:0", "side:1"), BlendKind::Fillet, 25.0, "x").unwrap_err();
    assert!(matches!(e, BlendError::TooLarge(_)), "{e:?}");
    // Two edges at one corner.
    let pairs = vec![
        ["side:0".to_owned(), "side:1".to_owned()],
        ["top".to_owned(), "side:1".to_owned()],
    ];
    let e = blend_edges(&b, &pairs, BlendKind::Fillet, 1.0, "x").unwrap_err();
    assert!(matches!(e, BlendError::Unsupported(_)), "{e:?}");
    // An edge that ends on a cylinder.
    let drilled = b
        .boolean(
            &Solid::cylinder(v(10.0, 10.0, -1.0), v(0.0, 0.0, 1.0), 4.0, 12.0).unwrap(),
            BoolOp::Subtract,
        )
        .unwrap()
        .solid;
    let e = blend_edges(
        &drilled,
        &pair("top", "tool:side"),
        BlendKind::Fillet,
        1.0,
        "x",
    )
    .unwrap_err();
    assert!(
        matches!(e, BlendError::Unsupported(_)),
        "an edge of a round hole: {e:?}"
    );
    let e = blend_edges(&b, &pair("side:0", "side:1"), BlendKind::Fillet, -1.0, "x").unwrap_err();
    assert!(matches!(e, BlendError::Unsupported(_)), "{e:?}");
}

#[test]
fn a_rounded_part_goes_on_through_the_boolean() {
    // Round a corner, then drill a hole next to it and cut a notch across the fillet.
    let r = blend_edges(
        &block(),
        &pair("side:0", "side:1"),
        BlendKind::Fillet,
        4.0,
        "f",
    )
    .unwrap();
    let hole = Solid::cylinder(v(16.0, 4.0, -1.0), v(0.0, 0.0, 1.0), 2.0, 12.0).unwrap();
    let a = r.boolean(&hole, BoolOp::Subtract).unwrap().solid;
    well_formed(&a);
    vol(&a, r.volume() - PI * 4.0 * 10.0);
    // (not up to y = 6: that plane would just touch the hole)
    let notch = Solid::cuboid(v(15.0, -1.0, 6.0), v(21.0, 5.5, 11.0)).unwrap();
    let b = a.boolean(&notch, BoolOp::Subtract).unwrap().solid;
    assert!(b.validate().is_empty(), "{:?}", b.validate());
    assert!(b.volume() < a.volume());
}
