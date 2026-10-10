//! The layered model names split faces by their neighbours, not by the order it built them in:
//! moving or resizing a cut changes no name while the topology stays the same.

use dc_features::{build_solid, Stepped};
use dc_math::{Plane, Tolerance, Vec3};
use dc_sketch::{Entity, Profile, Sketch};
use std::collections::BTreeSet;

fn xy() -> Plane {
    Plane::new(Vec3::ZERO, Vec3::X, Vec3::Y, Tolerance::DEFAULT).unwrap()
}

fn rect(p: &str, x0: f64, y0: f64, x1: f64, y1: f64) -> Profile {
    let mut s = Sketch::new();
    for (i, (x, y)) in [(x0, y0), (x1, y0), (x1, y1), (x0, y1)].iter().enumerate() {
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

fn names(w: f64, slot_x: f64, slot_w: f64) -> BTreeSet<String> {
    let st = Stepped::extrusion(xy(), rect("a", 0.0, 0.0, w, 40.0), 12.0, false)
        .cut(
            &rect("t", slot_x, -5.0, slot_x + slot_w, 45.0),
            0.0,
            12.0,
            "slot",
        )
        .unwrap();
    let s = build_solid(&st).unwrap();
    let all: BTreeSet<String> = s.faces.iter().map(|f| f.name.clone()).collect();
    assert_eq!(all.len(), s.faces.len(), "face names are unique");
    all
}

#[test]
fn split_faces_keep_their_names_when_the_cut_moves_or_changes_size() {
    let base = names(100.0, 20.0, 10.0);
    assert_eq!(base, names(100.0, 70.0, 10.0), "moved across the middle");
    assert_eq!(base, names(100.0, 20.0, 25.0), "widened");
    assert_eq!(base, names(140.0, 20.0, 10.0), "longer part");
    assert!(base.iter().any(|n| n.starts_with("cap_end#")), "{base:?}");
}
