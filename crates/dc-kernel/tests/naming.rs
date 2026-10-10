//! Persistent names: unique, structural, and stable while the geometry changes but the topology
//! does not. (AGENTS.md section 4.4 "stable references".)

use dc_kernel::boolean::BoolOp;
use dc_kernel::Solid;
use dc_math::Vec3;
use std::collections::BTreeSet;

fn v(x: f64, y: f64, z: f64) -> Vec3 {
    Vec3::new(x, y, z)
}

fn block(w: f64) -> Solid {
    Solid::cuboid(v(0.0, 0.0, 0.0), v(w, 20.0, 10.0)).unwrap()
}

fn names(s: &Solid) -> (BTreeSet<String>, BTreeSet<String>) {
    (
        s.faces.iter().map(|f| f.name.clone()).collect(),
        s.edges.iter().map(|e| e.name.clone()).collect(),
    )
}

fn assert_unique(s: &Solid) {
    let (f, e) = names(s);
    assert_eq!(
        f.len(),
        s.faces.len(),
        "face names are unique: {:?}",
        s.faces.iter().map(|f| &f.name).collect::<Vec<_>>()
    );
    assert_eq!(e.len(), s.edges.len(), "edge names are unique");
}

fn run(a: &Solid, b: &Solid, op: BoolOp) -> Solid {
    let r = a.boolean(b, op).unwrap();
    assert_unique(&r.solid);
    r.solid
}

/// A slot through the whole block across its width, at x in `x0..x0+width`: top and bottom split.
fn slotted(x0: f64, width: f64, total: f64) -> Solid {
    let tool = Solid::cuboid(v(x0, -1.0, -1.0), v(x0 + width, 21.0, 11.0)).unwrap();
    run(&block(total), &tool, BoolOp::Subtract)
}

#[test]
fn pieces_of_a_split_face_keep_their_names_when_the_cut_moves_or_changes_size() {
    let a = names(&slotted(10.0, 2.0, 40.0));
    // Moved so the other piece is now the bigger one, and a different slot width and part size.
    assert_eq!(
        a,
        names(&slotted(25.0, 2.0, 40.0)),
        "slot moved across the middle"
    );
    assert_eq!(a, names(&slotted(10.0, 5.0, 40.0)), "slot widened");
    assert_eq!(a, names(&slotted(10.0, 2.0, 60.0)), "part longer");
    // Split pieces carry the base name plus a suffix; unsplit faces keep the plain name.
    let (faces, _) = a;
    assert!(faces.iter().any(|n| n.starts_with("top#")), "{faces:?}");
    assert!(
        faces.contains("side:1") && faces.contains("side:3"),
        "{faces:?}"
    );
}

#[test]
fn a_round_hole_that_moves_or_grows_changes_no_name() {
    let mk = |x: f64, r: f64| {
        let tool = Solid::cylinder(v(x, 10.0, -1.0), v(0.0, 0.0, 1.0), r, 12.0).unwrap();
        names(&run(&block(40.0), &tool, BoolOp::Subtract))
    };
    let base = mk(20.0, 3.0);
    assert_eq!(base, mk(12.0, 3.0));
    assert_eq!(base, mk(20.0, 5.5));
}

#[test]
fn names_do_not_depend_on_the_order_of_the_input_faces() {
    let mut a = block(40.0);
    let mut tool = Solid::cuboid(v(10.0, -1.0, -1.0), v(12.0, 21.0, 11.0)).unwrap();
    let reference = run(&a, &tool, BoolOp::Subtract);
    a.faces.reverse();
    tool.faces.rotate_left(2);
    let shuffled = run(&a, &tool, BoolOp::Subtract);
    assert_eq!(names(&reference), names(&shuffled));
    // The faces with the same name have the same area.
    for f in &reference.faces {
        let g = shuffled.faces.iter().find(|g| g.name == f.name).unwrap();
        assert!(
            (reference.face_area(f) - shuffled.face_area(g)).abs() < 1e-9,
            "{}",
            f.name
        );
    }
    assert!((reference.volume() - shuffled.volume()).abs() < 1e-9);
}

#[test]
fn the_result_is_canonical_so_the_same_inputs_give_the_same_bytes() {
    let a = block(40.0);
    let tool = Solid::cuboid(v(10.0, -1.0, -1.0), v(12.0, 21.0, 11.0)).unwrap();
    let r1 = run(&a, &tool, BoolOp::Subtract);
    let r2 = run(&a, &tool, BoolOp::Subtract);
    assert_eq!(r1.canonical_bytes(), r2.canonical_bytes());
    // Faces and edges come out ordered by name.
    let sorted = |names: Vec<&String>| names.windows(2).all(|w| w[0] <= w[1]);
    assert!(sorted(r1.faces.iter().map(|f| &f.name).collect()));
    assert!(sorted(r1.edges.iter().map(|e| &e.name).collect()));
}

#[test]
fn edges_between_the_same_two_faces_get_distinct_structural_names() {
    // Two notches on the same side of a block: the top face and the side face meet in three
    // separate stretches of edge.
    let mut s = block(60.0);
    for x in [10.0, 40.0] {
        let tool = Solid::cuboid(v(x, -1.0, 5.0), v(x + 6.0, 6.0, 11.0)).unwrap();
        s = run(&s, &tool, BoolOp::Subtract);
    }
    assert_unique(&s);
    let dup: Vec<&String> = s
        .edges
        .iter()
        .map(|e| &e.name)
        .filter(|n| n.contains('#'))
        .collect();
    assert!(
        !dup.is_empty(),
        "some edge names carry a structural suffix: {:?}",
        s.edges.iter().map(|e| &e.name).collect::<Vec<_>>()
    );
}

#[test]
fn a_chain_of_operations_never_repeats_a_name() {
    let mut s = block(80.0);
    for (i, x) in [10.0, 25.0, 40.0, 55.0, 70.0].iter().enumerate() {
        let tool = Solid::cylinder(v(*x, 10.0, -1.0), v(0.0, 0.0, 1.0), 3.0, 12.0).unwrap();
        let r = s
            .boolean_named(&tool, BoolOp::Subtract, &format!("hole{i}"))
            .unwrap();
        s = r.solid;
        assert_unique(&s);
    }
    assert_eq!(s.faces.len(), 6 + 5);
}
