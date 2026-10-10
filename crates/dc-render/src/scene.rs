//! What gets drawn: tessellated bodies and 3D polylines, built from solids and sketches.

use dc_doc::Authority;
use dc_kernel::{Mesh, MeshError, Solid, TessParams};
use dc_math::{det, Vec3};
use dc_sketch::Entity;
use std::collections::BTreeMap;
use std::sync::Arc;

#[derive(Clone, Debug)]
pub struct Body {
    /// The feature this body is the result of.
    pub id: String,
    pub mesh: Arc<Mesh>,
    /// Face names, indexed like `Mesh::face_of` values.
    pub face_names: Vec<String>,
    /// Sharp and face-boundary edges, in world coordinates.
    pub edges: Vec<[Vec3; 2]>,
    /// For each edge, the outward normals of the two faces that meet there (the same twice for an
    /// open boundary): what tells a silhouette from a crease from a smooth seam.
    pub edge_normals: Vec<[Vec3; 2]>,
    /// For each edge, the indices of the two faces that meet there (the same twice for an open
    /// boundary): what names it.
    pub edge_faces: Vec<[u32; 2]>,
    /// Per triangle, the shading normal at each corner. Smooth within a face (so cylinders look
    /// round) but never across faces (so edges stay crisp).
    pub corner_normals: Vec<[Vec3; 3]>,
    pub color: [u8; 3],
}

#[derive(Clone, Debug)]
pub struct Polyline {
    pub id: String,
    pub points: Vec<Vec3>,
    pub color: [u8; 3],
}

#[derive(Clone, Debug, Default)]
pub struct Scene {
    pub bodies: Vec<Body>,
    pub lines: Vec<Polyline>,
}

impl Scene {
    /// Bounding box of everything drawn, if anything is.
    pub fn bounds(&self) -> Option<(Vec3, Vec3)> {
        let mut it = self
            .bodies
            .iter()
            .flat_map(|b| b.mesh.positions.iter().copied())
            .chain(self.lines.iter().flat_map(|l| l.points.iter().copied()));
        let first = it.next()?;
        let (mut lo, mut hi) = (first, first);
        for p in it {
            lo = Vec3::new(lo.x.min(p.x), lo.y.min(p.y), lo.z.min(p.z));
            hi = Vec3::new(hi.x.max(p.x), hi.y.max(p.y), hi.z.max(p.z));
        }
        Some((lo, hi))
    }

    /// Like [`bounds`](Self::bounds) but only the solid bodies (for fitting the view to the part).
    pub fn body_bounds(&self) -> Option<(Vec3, Vec3)> {
        let mut it = self
            .bodies
            .iter()
            .flat_map(|b| b.mesh.positions.iter().copied());
        let first = it.next()?;
        let (mut lo, mut hi) = (first, first);
        for p in it {
            lo = Vec3::new(lo.x.min(p.x), lo.y.min(p.y), lo.z.min(p.z));
            hi = Vec3::new(hi.x.max(p.x), hi.y.max(p.y), hi.z.max(p.z));
        }
        Some((lo, hi))
    }
}

/// Edges between triangles of different faces (and open boundaries), each once.
pub fn feature_edges(mesh: &Mesh) -> Vec<[Vec3; 2]> {
    feature_edges_with_normals(mesh).0
}

/// The edge list of [`feature_edges_with_normals`] together with the faces on each side.
pub struct EdgeSet {
    pub edges: Vec<[Vec3; 2]>,
    pub normals: Vec<[Vec3; 2]>,
    pub faces: Vec<[u32; 2]>,
}

/// [`feature_edges`], with the outward normals of the two faces at each edge.
pub fn feature_edges_with_normals(mesh: &Mesh) -> (Vec<[Vec3; 2]>, Vec<[Vec3; 2]>) {
    let e = feature_edge_set(mesh);
    (e.edges, e.normals)
}

/// [`feature_edges_with_normals`] with the two faces of each edge as well.
pub fn feature_edge_set(mesh: &Mesh) -> EdgeSet {
    let normal_of = |t: usize| {
        let tri = &mesh.triangles[t];
        let (a, b, c) = (
            mesh.positions[tri[0] as usize],
            mesh.positions[tri[1] as usize],
            mesh.positions[tri[2] as usize],
        );
        let n = (b - a).cross(c - a);
        let l = n.length();
        if l > 1e-300 {
            n / l
        } else {
            Vec3::Z
        }
    };
    // (face of the first triangle, whether a second triangle has another face, first, second)
    type Seen = (u32, bool, usize, Option<usize>);
    let mut seen: BTreeMap<(u32, u32), Seen> = BTreeMap::new();
    for (t, tri) in mesh.triangles.iter().enumerate() {
        let face = mesh.face_of[t];
        for k in 0..3 {
            let (a, b) = (tri[k], tri[(k + 1) % 3]);
            let key = (a.min(b), a.max(b));
            match seen.get_mut(&key) {
                None => {
                    seen.insert(key, (face, false, t, None));
                }
                Some(e) => {
                    if e.0 != face {
                        e.1 = true;
                    }
                    if e.3.is_none() {
                        e.3 = Some(t);
                    }
                }
            }
        }
    }
    // Edges seen once (open) are also shown; edges seen twice only if the faces differ.
    let mut count: BTreeMap<(u32, u32), u32> = BTreeMap::new();
    for tri in &mesh.triangles {
        for k in 0..3 {
            let (a, b) = (tri[k], tri[(k + 1) % 3]);
            *count.entry((a.min(b), a.max(b))).or_insert(0) += 1;
        }
    }
    let mut edges = Vec::new();
    let mut normals = Vec::new();
    let mut faces = Vec::new();
    for ((a, b), (_, differ, t1, t2)) in seen {
        if differ || count[&(a, b)] == 1 {
            edges.push([mesh.positions[a as usize], mesh.positions[b as usize]]);
            let n1 = normal_of(t1);
            normals.push([n1, t2.map_or(n1, normal_of)]);
            faces.push([
                mesh.face_of[t1],
                t2.map_or(mesh.face_of[t1], |t| mesh.face_of[t]),
            ]);
        }
    }
    EdgeSet {
        edges,
        normals,
        faces,
    }
}

fn corner_normals(mesh: &Mesh) -> Vec<[Vec3; 3]> {
    let mut sum: BTreeMap<(u32, u32), Vec3> = BTreeMap::new();
    let tri_normal = |t: &[u32; 3]| {
        let (a, b, c) = (
            mesh.positions[t[0] as usize],
            mesh.positions[t[1] as usize],
            mesh.positions[t[2] as usize],
        );
        (b - a).cross(c - a)
    };
    for (t, tri) in mesh.triangles.iter().enumerate() {
        let n = tri_normal(tri); // area-weighted
        for &v in tri {
            let e = sum.entry((v, mesh.face_of[t])).or_insert(Vec3::ZERO);
            *e += n;
        }
    }
    mesh.triangles
        .iter()
        .enumerate()
        .map(|(t, tri)| {
            let face = mesh.face_of[t];
            let flat = tri_normal(tri);
            let norm = |v: Vec3, fallback: Vec3| {
                let l = v.length();
                if l > 1e-12 {
                    v / l
                } else {
                    let f = fallback.length();
                    if f > 1e-12 {
                        fallback / f
                    } else {
                        Vec3::Z
                    }
                }
            };
            [
                norm(sum[&(tri[0], face)], flat),
                norm(sum[&(tri[1], face)], flat),
                norm(sum[&(tri[2], face)], flat),
            ]
        })
        .collect()
}

impl Body {
    /// The name of edge `i`: the names of the two faces that meet there, sorted and joined by
    /// `|` (`cap_end|side:c1`). It survives rebuilds as long as the faces keep their names.
    pub fn edge_name(&self, i: usize) -> String {
        let [a, b] = self.edge_faces[i];
        let (mut x, mut y) = (
            self.face_names[a as usize].clone(),
            self.face_names[b as usize].clone(),
        );
        if x > y {
            std::mem::swap(&mut x, &mut y);
        }
        format!("{x}|{y}")
    }

    /// Every drawn segment of the edge called `name`.
    pub fn edge_segments(&self, name: &str) -> Vec<[Vec3; 2]> {
        (0..self.edges.len())
            .filter(|&i| self.edge_name(i) == name)
            .map(|i| self.edges[i])
            .collect()
    }

    pub fn from_solid(id: &str, solid: &Solid, color: [u8; 3]) -> Result<Body, MeshError> {
        let mesh = solid.tessellate(TessParams::default())?;
        let EdgeSet {
            edges,
            normals: edge_normals,
            faces: edge_faces,
        } = feature_edge_set(&mesh);
        let corner_normals = corner_normals(&mesh);
        Ok(Body {
            id: id.to_owned(),
            face_names: solid.faces.iter().map(|f| f.name.clone()).collect(),
            mesh: Arc::new(mesh),
            edges,
            edge_normals,
            edge_faces,
            corner_normals,
            color,
        })
    }
}

const PALETTE: [[u8; 3]; 5] = [
    [122, 162, 214],
    [214, 154, 96],
    [124, 192, 134],
    [192, 124, 172],
    [204, 192, 104],
];

/// Everything in a document worth drawing: the final bodies, and every sketch as 3D outlines.
pub fn scene_of_authority(a: &Authority) -> Scene {
    let mut scene = Scene::default();
    for (i, f) in a.state().result_features().iter().enumerate() {
        if let Some(solid) = a.solid(f) {
            if let Ok(b) = Body::from_solid(f, &solid, PALETTE[i % PALETTE.len()]) {
                scene.bodies.push(b);
            }
        }
    }
    let st = a.state();
    for id in st.sketch_ids() {
        let (Ok(plane), Some(slot)) = (st.sketch_plane(id), st.sketch(id)) else {
            continue;
        };
        let sk = &slot.sketch;
        let at = |p: &str| {
            sk.point(p)
                .map(|(x, y)| plane.to_3d(dc_math::Vec2::new(x, y)))
        };
        for (eid, e) in sk.entities() {
            if sk.is_construction(eid) {
                continue;
            }
            let pts: Vec<Vec3> = match e {
                Entity::Point { .. } => continue,
                Entity::Line { a, b } => match (at(a), at(b)) {
                    (Some(p), Some(q)) => vec![p, q],
                    _ => continue,
                },
                Entity::Circle { center, radius } => {
                    let Some((cx, cy)) = sk.point(center) else {
                        continue;
                    };
                    (0..=72)
                        .map(|k| {
                            let t = core::f64::consts::TAU * f64::from(k) / 72.0;
                            plane.to_3d(dc_math::Vec2::new(
                                cx + radius * det::cos(t),
                                cy + radius * det::sin(t),
                            ))
                        })
                        .collect()
                }
                Entity::Arc { center, start, end } => {
                    let (Some((cx, cy)), Some((sx, sy)), Some((ex, ey))) =
                        (sk.point(center), sk.point(start), sk.point(end))
                    else {
                        continue;
                    };
                    let r = dc_math::det::sqrt((sx - cx) * (sx - cx) + (sy - cy) * (sy - cy));
                    let a0 = det::atan2(sy - cy, sx - cx);
                    let mut sweep = det::atan2(ey - cy, ex - cx) - a0;
                    while sweep <= 0.0 {
                        sweep += core::f64::consts::TAU;
                    }
                    let n = ((sweep / core::f64::consts::TAU) * 72.0).ceil().max(4.0) as u32;
                    (0..=n)
                        .map(|k| {
                            let t = a0 + sweep * f64::from(k) / f64::from(n);
                            plane.to_3d(dc_math::Vec2::new(
                                cx + r * det::cos(t),
                                cy + r * det::sin(t),
                            ))
                        })
                        .collect()
                }
            };
            scene.lines.push(Polyline {
                id: format!("sketch/{id}/{eid}"),
                points: pts,
                color: [0, 150, 175],
            });
        }
    }
    scene
}

#[cfg(test)]
mod tests {
    use super::*;
    use dc_features::{build_solid, Stepped};
    use dc_math::{Plane, Tolerance};
    use dc_sketch::Sketch;

    pub(crate) fn box_solid(w: f64, d: f64, h: f64) -> Solid {
        let mut s = Sketch::new();
        for (i, (x, y)) in [(0.0, 0.0), (w, 0.0), (w, d), (0.0, d)].iter().enumerate() {
            s.add_entity(&format!("p{i}"), Entity::Point { x: *x, y: *y })
                .unwrap();
        }
        for i in 0..4 {
            s.add_entity(
                &format!("e{i}"),
                Entity::Line {
                    a: format!("p{i}"),
                    b: format!("p{}", (i + 1) % 4),
                },
            )
            .unwrap();
        }
        let p = s.profiles().profiles.remove(0);
        let plane = Plane::new(Vec3::ZERO, Vec3::X, Vec3::Y, Tolerance::DEFAULT).unwrap();
        build_solid(&Stepped::extrusion(plane, p, h, false)).unwrap()
    }

    #[test]
    fn a_box_has_twelve_feature_edges_and_flat_normals() {
        let b = Body::from_solid("box", &box_solid(80.0, 40.0, 20.0), [100, 100, 100]).unwrap();
        assert_eq!(
            b.edges.len(),
            12,
            "the twelve edges of a box, and not the diagonals inside faces"
        );
        assert_eq!(b.mesh.triangles.len(), 12);
        for (t, tri) in b.corner_normals.iter().enumerate() {
            let f = b.mesh.face_of[t];
            // All three corners of a flat face share one normal, and it is a unit axis vector.
            assert!(
                tri.iter()
                    .all(|n| (*n - tri[0]).length() < 1e-12 && (n.length() - 1.0).abs() < 1e-12),
                "face {f}"
            );
        }
    }

    #[test]
    fn every_edge_knows_the_two_faces_that_meet_there() {
        let b = Body::from_solid("box", &box_solid(30.0, 20.0, 10.0), [1, 2, 3]).unwrap();
        assert_eq!(b.edges.len(), b.edge_normals.len());
        assert_eq!(b.edges.len(), 12);
        for ([p, q], [n1, n2]) in b.edges.iter().zip(&b.edge_normals) {
            // Unit normals, at right angles (a box has only creases), both perpendicular to the edge.
            assert!((n1.length() - 1.0).abs() < 1e-9 && (n2.length() - 1.0).abs() < 1e-9);
            assert!(n1.dot(*n2).abs() < 1e-9, "{n1:?} {n2:?}");
            let d = *q - *p;
            assert!(n1.dot(d).abs() < 1e-9 && n2.dot(d).abs() < 1e-9);
            // Outward: the midpoint pushed along either normal leaves the box.
            let m = (*p + *q) * 0.5;
            for n in [n1, n2] {
                let out = m + *n * 0.5;
                let inside = (0.0..=30.0).contains(&out.x)
                    && (0.0..=20.0).contains(&out.y)
                    && (0.0..=10.0).contains(&out.z);
                assert!(!inside, "{n:?} points into the box at {m:?}");
            }
        }
    }

    #[test]
    fn cylinders_are_shaded_smoothly_but_keep_a_crisp_rim() {
        let mut s = Sketch::new();
        s.add_entity("m", Entity::Point { x: 0.0, y: 0.0 }).unwrap();
        s.add_entity(
            "c",
            Entity::Circle {
                center: "m".into(),
                radius: 10.0,
            },
        )
        .unwrap();
        let p = s.profiles().profiles.remove(0);
        let plane = Plane::new(Vec3::ZERO, Vec3::X, Vec3::Y, Tolerance::DEFAULT).unwrap();
        let solid = build_solid(&Stepped::extrusion(plane, p, 5.0, false)).unwrap();
        let b = Body::from_solid("cyl", &solid, [1, 2, 3]).unwrap();
        // Side normals point radially (horizontal, varying), caps point along z.
        let side = solid
            .faces
            .iter()
            .position(|f| f.name.starts_with("side"))
            .unwrap() as u32;
        let mut distinct = std::collections::BTreeSet::new();
        for (t, tri) in b.corner_normals.iter().enumerate() {
            if b.mesh.face_of[t] == side {
                for n in tri {
                    assert!(n.z.abs() < 1e-9 && (n.length() - 1.0).abs() < 1e-9);
                    distinct.insert(((n.x * 1e6) as i64, (n.y * 1e6) as i64));
                }
            }
        }
        assert!(
            distinct.len() > 8,
            "smooth shading has many different normals, got {}",
            distinct.len()
        );
        // Only the two circular rims are drawn: no seam, no interior triangulation lines.
        assert!(
            b.edges.iter().all(|e| (e[0].z - e[1].z).abs() < 1e-9),
            "a vertical seam line was drawn"
        );
        assert!(b.edges.len() < 200, "{} edges", b.edges.len());
    }
}
