//! Structural names for the faces and edges of a solid (AGENTS.md section 4.4: stable references).

use crate::solid::{Id, Solid};
use dc_math::Vec3;
use std::collections::BTreeMap;

fn fnv(s: &str) -> u32 {
    let mut h: u32 = 0x811c_9dc5;
    for b in s.bytes() {
        h ^= u32::from(b);
        h = h.wrapping_mul(0x0100_0193);
    }
    h
}

/// Names for the faces and edges of a provisional solid whose faces carry only their base names
/// (`bases`, possibly repeated). Rules, all structural (no enumeration order):
///
/// * a base used by one face names it;
/// * if several faces share a base (a face that was split), each gets `<base>#<hash>` of the
///   sorted base names of the faces across the edges of its outer loop, so a piece keeps its name
///   when it grows, shrinks or moves while its neighbours stay the same (a feature inside it,
///   like a hole, changes nothing); a clash is broken by position;
/// * an edge is named `<face>|<face>` from the final names of the faces on its two sides; several
///   edges between the same pair get `#<hash>` of the faces around their end vertices (then
///   position).
pub fn structural_names(solid: &Solid, bases: &[String]) -> (Vec<String>, Vec<String>) {
    let nf = solid.faces.len();
    let ne = solid.edges.len();
    // Faces on each edge, and the edges around each vertex.
    let mut edge_faces: Vec<Vec<usize>> = vec![Vec::new(); ne];
    for (fi, f) in solid.faces.iter().enumerate() {
        for c in f.loops.iter().flatten() {
            edge_faces[c.edge as usize].push(fi);
        }
    }
    let centre = |fi: usize| -> [i64; 3] {
        let mut sum = Vec3::new(0.0, 0.0, 0.0);
        let mut n: f64 = 0.0;
        for c in solid.faces[fi].loops.iter().flatten() {
            let e = &solid.edges[c.edge as usize];
            sum += solid.vertex(e.v0) + solid.vertex(e.v1);
            n += 2.0;
        }
        let m = sum / n.max(1.0);
        [
            (m.x * 1e5).round() as i64,
            (m.y * 1e5).round() as i64,
            (m.z * 1e5).round() as i64,
        ]
    };
    // Face names.
    let mut groups: BTreeMap<&str, Vec<usize>> = BTreeMap::new();
    for (i, b) in bases.iter().enumerate() {
        groups.entry(b).or_default().push(i);
    }
    let mut face_names = vec![String::new(); nf];
    for (base, members) in &groups {
        if members.len() == 1 {
            face_names[members[0]] = (*base).to_owned();
            continue;
        }
        let sig = |fi: usize| -> String {
            let mut n: Vec<&str> = Vec::new();
            for c in solid.faces[fi].loops.first().into_iter().flatten() {
                for &g in &edge_faces[c.edge as usize] {
                    if g != fi {
                        n.push(&bases[g]);
                    }
                }
            }
            n.sort_unstable();
            n.dedup();
            n.join("+")
        };
        let order: Vec<usize> = members.clone();
        for &fi in &order {
            face_names[fi] = format!("{base}#{:06x}", fnv(&sig(fi)) & 0xff_ffff);
        }
        // Clashes (equal signatures): number them by position.
        let mut seen: BTreeMap<String, u32> = BTreeMap::new();
        let mut by_pos = order.clone();
        by_pos.sort_by_key(|&fi| centre(fi));
        for fi in by_pos {
            let k = seen.entry(face_names[fi].clone()).or_insert(0);
            if *k > 0 {
                face_names[fi] = format!("{}.{k}", face_names[fi]);
            }
            *k += 1;
        }
    }
    // Edge names.
    let mut star: BTreeMap<Id, Vec<usize>> = BTreeMap::new();
    for (ei, e) in solid.edges.iter().enumerate() {
        for v in [e.v0, e.v1] {
            star.entry(v).or_default().push(ei);
        }
    }
    let vertex_sig = |v: Id| -> String {
        let mut n: Vec<&str> = Vec::new();
        for &ei in &star[&v] {
            for &fi in &edge_faces[ei] {
                n.push(&face_names[fi]);
            }
        }
        n.sort_unstable();
        n.dedup();
        n.join("+")
    };
    let mid = |ei: usize| -> [i64; 3] {
        let p = solid.edge_point(ei as Id, 0.5);
        [
            (p.x * 1e5).round() as i64,
            (p.y * 1e5).round() as i64,
            (p.z * 1e5).round() as i64,
        ]
    };
    let mut base_of: Vec<String> = Vec::with_capacity(ne);
    for faces in &edge_faces {
        let mut n: Vec<&str> = faces.iter().map(|&fi| face_names[fi].as_str()).collect();
        n.sort_unstable();
        n.dedup();
        base_of.push(n.join("|"));
    }
    let mut egroups: BTreeMap<&str, Vec<usize>> = BTreeMap::new();
    for (i, b) in base_of.iter().enumerate() {
        egroups.entry(b).or_default().push(i);
    }
    let mut edge_names = vec![String::new(); ne];
    for (base, members) in &egroups {
        if members.len() == 1 {
            edge_names[members[0]] = (*base).to_owned();
            continue;
        }
        let sig = |ei: usize| -> String {
            let e = &solid.edges[ei];
            let (a, b) = (vertex_sig(e.v0), vertex_sig(e.v1));
            if a <= b {
                format!("{a}/{b}")
            } else {
                format!("{b}/{a}")
            }
        };
        let order = members.clone();
        for &ei in &order {
            edge_names[ei] = format!("{base}#{:06x}", fnv(&sig(ei)) & 0xff_ffff);
        }
        let mut seen: BTreeMap<String, u32> = BTreeMap::new();
        let mut by_pos = order.clone();
        by_pos.sort_by_key(|&ei| mid(ei));
        for ei in by_pos {
            let k = seen.entry(edge_names[ei].clone()).or_insert(0);
            if *k > 0 {
                edge_names[ei] = format!("{}.{k}", edge_names[ei]);
            }
            *k += 1;
        }
    }
    (face_names, edge_names)
}
