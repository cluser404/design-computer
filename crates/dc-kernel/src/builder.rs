//! Assembling a [`Solid`] from faces given as vertex cycles, plus prism and cuboid constructors.
//!
//! The builder shares one edge between the two faces that use it (an undirected vertex pair), so
//! the result passes the "every edge used once in each direction" check by construction when the
//! faces are consistently oriented. Edge names are derived from the faces on either side
//! (`<face>|<face>`, sorted), with `#k` added only to keep names unique.

use crate::solid::{CoEdge, Curve, Edge, Face, Id, Solid, Surface};
use dc_math::{Tolerance, Vec3};
use std::collections::BTreeMap;

#[derive(Default)]
pub struct SolidBuilder {
    pub vertices: Vec<Vec3>,
    edge_ids: BTreeMap<(Id, Id), Id>,
    edge_ends: Vec<(Id, Id)>,
    /// Faces whose edge names are still to be derived.
    faces: Vec<Face>,
}

impl SolidBuilder {
    pub fn new() -> SolidBuilder {
        SolidBuilder::default()
    }

    pub fn vertex(&mut self, p: Vec3) -> Id {
        self.vertices.push(p);
        (self.vertices.len() - 1) as Id
    }

    fn edge(&mut self, a: Id, b: Id) -> CoEdge {
        let (lo, hi) = if a < b { (a, b) } else { (b, a) };
        let next = self.edge_ends.len() as Id;
        let id = *self.edge_ids.entry((lo, hi)).or_insert_with(|| next);
        if id == next {
            self.edge_ends.push((lo, hi));
        }
        CoEdge {
            edge: id,
            reversed: a > b,
        }
    }

    /// Adds a face whose loops are cycles of vertex IDs (the first loop is the outer boundary).
    pub fn face(&mut self, name: String, surface: Surface, same_sense: bool, loops: &[Vec<Id>]) {
        let loops = loops
            .iter()
            .map(|lp| {
                (0..lp.len())
                    .map(|k| self.edge(lp[k], lp[(k + 1) % lp.len()]))
                    .collect()
            })
            .collect();
        self.faces.push(Face {
            name,
            surface,
            same_sense,
            loops,
        });
    }

    pub fn finish(self) -> Solid {
        let mut names: Vec<Vec<String>> = vec![Vec::new(); self.edge_ends.len()];
        for f in &self.faces {
            for c in f.loops.iter().flatten() {
                names[c.edge as usize].push(f.name.clone());
            }
        }
        let mut seen: BTreeMap<String, u32> = BTreeMap::new();
        let edges = self
            .edge_ends
            .iter()
            .zip(names)
            .map(|(&(v0, v1), mut n)| {
                n.sort();
                n.dedup();
                let base = n.join("|");
                let k = seen.entry(base.clone()).or_insert(0);
                let name = if *k == 0 {
                    base.clone()
                } else {
                    format!("{base}#{k}")
                };
                *k += 1;
                Edge {
                    name,
                    v0,
                    v1,
                    curve: Curve::Line,
                }
            })
            .collect();
        Solid {
            vertices: self.vertices,
            edges,
            faces: self.faces,
        }
    }
}

/// Newell normal of a polygon (not normalised; its length is twice the area).
pub fn newell(poly: &[Vec3]) -> Vec3 {
    let mut n = Vec3::new(0.0, 0.0, 0.0);
    for k in 0..poly.len() {
        let (a, b) = (poly[k], poly[(k + 1) % poly.len()]);
        n += a.cross(b);
    }
    n
}

impl Solid {
    /// A prism: the planar polygon `outer` (with optional `holes`) swept along `dir`. Faces are
    /// named `bottom`, `top`, `side:<i>` (outer edge i) and `hole<h>:<i>`. Planar polygons only;
    /// `dir` must not lie in the polygon's plane. Returns `None` for degenerate input.
    pub fn prism(outer: &[Vec3], holes: &[Vec<Vec3>], dir: Vec3) -> Option<Solid> {
        let tol = Tolerance::DEFAULT;
        if outer.len() < 3 {
            return None;
        }
        let mut outer = outer.to_vec();
        let mut n = newell(&outer);
        if n.length() <= tol.linear {
            return None;
        }
        if n.dot(dir) < 0.0 {
            outer.reverse();
            n = -n;
        }
        let n = n.normalized(tol)?;
        if n.dot(dir) <= tol.linear {
            return None;
        }
        // Holes run clockwise about `n` (as seen from the top).
        let holes: Vec<Vec<Vec3>> = holes
            .iter()
            .map(|h| {
                let mut h = h.clone();
                if newell(&h).dot(n) > 0.0 {
                    h.reverse();
                }
                h
            })
            .collect();
        let mut b = SolidBuilder::new();
        let mut rings: Vec<(Vec<Id>, Vec<Id>)> = Vec::new();
        for poly in std::iter::once(&outer).chain(holes.iter()) {
            let bottom: Vec<Id> = poly.iter().map(|&p| b.vertex(p)).collect();
            let top: Vec<Id> = poly.iter().map(|&p| b.vertex(p + dir)).collect();
            rings.push((bottom, top));
        }
        let x_dir = (outer[1] - outer[0]).normalized(tol)?;
        let plane = |origin: Vec3, normal: Vec3| Surface::Plane {
            origin,
            normal,
            x_dir,
        };
        // Bottom: outward normal -n, so every loop runs the other way round.
        let bottom_loops: Vec<Vec<Id>> = rings
            .iter()
            .map(|(bt, _)| bt.iter().rev().copied().collect())
            .collect();
        b.face("bottom".into(), plane(outer[0], -n), true, &bottom_loops);
        let top_loops: Vec<Vec<Id>> = rings.iter().map(|(_, tp)| tp.clone()).collect();
        b.face("top".into(), plane(outer[0] + dir, n), true, &top_loops);
        for (ri, ((bt, tp), poly)) in rings
            .iter()
            .zip(std::iter::once(&outer).chain(holes.iter()))
            .enumerate()
        {
            for i in 0..poly.len() {
                let j = (i + 1) % poly.len();
                let e = poly[j] - poly[i];
                let out = e.cross(dir).normalized(tol)?;
                let name = if ri == 0 {
                    format!("side:{i}")
                } else {
                    format!("hole{}:{i}", ri - 1)
                };
                b.face(
                    name,
                    Surface::Plane {
                        origin: poly[i],
                        normal: out,
                        x_dir: e.normalized(tol)?,
                    },
                    true,
                    &[vec![bt[i], bt[j], tp[j], tp[i]]],
                );
            }
        }
        Some(b.finish())
    }

    /// An axis-aligned box from two opposite corners.
    pub fn cuboid(min: Vec3, max: Vec3) -> Option<Solid> {
        let (a, b) = (min, max);
        let outer = [
            Vec3::new(a.x, a.y, a.z),
            Vec3::new(b.x, a.y, a.z),
            Vec3::new(b.x, b.y, a.z),
            Vec3::new(a.x, b.y, a.z),
        ];
        Solid::prism(&outer, &[], Vec3::new(0.0, 0.0, b.z - a.z))
    }
}

impl Solid {
    /// A right circular cylinder: base circle centred at `base`, axis direction `axis`, radius
    /// `r`, height `h`. Faces `bottom`, `top` and `side` (which has a seam edge along the
    /// reference direction). `None` for degenerate input.
    pub fn cylinder(base: Vec3, axis: Vec3, r: f64, h: f64) -> Option<Solid> {
        let tol = Tolerance::DEFAULT;
        let a = axis.normalized(tol)?;
        if r <= tol.linear || h <= tol.linear {
            return None;
        }
        // Any direction across the axis.
        let helper = if a.x.abs() < 0.9 {
            Vec3::new(1.0, 0.0, 0.0)
        } else {
            Vec3::new(0.0, 1.0, 0.0)
        };
        let e1 = (helper - a * helper.dot(a)).normalized(tol)?;
        let two_pi = 2.0 * core::f64::consts::PI;
        let top_c = base + a * h;
        let vertices = vec![base + e1 * r, top_c + e1 * r];
        let circle = |center: Vec3, name: &str| Edge {
            name: name.into(),
            v0: if name == "bottom" { 0 } else { 1 },
            v1: if name == "bottom" { 0 } else { 1 },
            curve: Curve::Circle {
                center,
                axis: a,
                x_dir: e1,
                radius: r,
                t0: 0.0,
                sweep: two_pi,
            },
        };
        let edges = vec![
            circle(base, "bottom"),
            circle(top_c, "top"),
            Edge {
                name: "side".into(),
                v0: 0,
                v1: 1,
                curve: Curve::Line,
            },
        ];
        let c = |edge: Id, reversed: bool| CoEdge { edge, reversed };
        let faces = vec![
            Face {
                name: "bottom".into(),
                surface: Surface::Plane {
                    origin: base,
                    normal: -a,
                    x_dir: e1,
                },
                same_sense: true,
                loops: vec![vec![c(0, true)]],
            },
            Face {
                name: "top".into(),
                surface: Surface::Plane {
                    origin: top_c,
                    normal: a,
                    x_dir: e1,
                },
                same_sense: true,
                loops: vec![vec![c(1, false)]],
            },
            Face {
                name: "side".into(),
                surface: Surface::Cylinder {
                    origin: base,
                    axis: a,
                    ref_dir: e1,
                    radius: r,
                    phi0: 0.0,
                    phi1: two_pi,
                    h0: 0.0,
                    h1: h,
                },
                same_sense: true,
                loops: vec![vec![c(0, false), c(2, false), c(1, true), c(2, true)]],
            },
        ];
        Some(Solid {
            vertices,
            edges,
            faces,
        })
    }
}

impl Solid {
    /// A solid between two planar polygons with the same number of corners, joined by flat
    /// faces (`bottom[i]`, `bottom[i+1]`, `top[i+1]`, `top[i]`). Each polygon may be given in
    /// either winding; the top is turned and shifted round so corner `i` of one goes with
    /// corner `i` of the other, whichever order they were given in (the shift that keeps
    /// matching corners closest). Faces: `cap_start`, `cap_end` and `side:<names[i]>`. Refuses
    /// polygons that are not planar or whose corresponding edges do not make flat side faces
    /// (the profiles' edges must be parallel, or meet).
    pub fn loft(bottom: &[Vec3], top: &[Vec3], names: &[String]) -> Result<Solid, String> {
        let tol = Tolerance::DEFAULT;
        let k = bottom.len();
        if k < 3 || top.len() != k || names.len() != k {
            return Err(format!(
                "a loft joins two outlines with the same number of corners (at least 3), got {} and {}",
                bottom.len(),
                top.len()
            ));
        }
        let planar = |p: &[Vec3]| -> Result<Vec3, String> {
            let n = newell(p).normalized(tol).ok_or("an outline has no area")?;
            let c = p.iter().fold(Vec3::new(0.0, 0.0, 0.0), |s, q| s + *q) / p.len() as f64;
            if p.iter().any(|q| (*q - c).dot(n).abs() > 1e-6) {
                return Err("an outline is not flat".into());
            }
            Ok(n)
        };
        planar(bottom)?;
        planar(top)?;
        let centroid =
            |p: &[Vec3]| p.iter().fold(Vec3::new(0.0, 0.0, 0.0), |s, q| s + *q) / p.len() as f64;
        let h = (centroid(top) - centroid(bottom))
            .normalized(tol)
            .ok_or("the two outlines are in the same place")?;
        // Both outlines counter-clockwise about the direction from bottom to top.
        let mut b = bottom.to_vec();
        let mut nb: Vec<String> = names.to_vec();
        if newell(&b).dot(h) < 0.0 {
            b.reverse();
            nb.reverse();
        }
        let mut t = top.to_vec();
        if newell(&t).dot(h) < 0.0 {
            t.reverse();
        }
        // The shift of the top's corners that keeps matching corners closest.
        let best = (0..k)
            .min_by(|&s, &r| {
                let cost = |s: usize| -> f64 {
                    (0..k)
                        .map(|i| {
                            (t[(i + s) % k] - b[i] - h * (t[(i + s) % k] - b[i]).dot(h)).length()
                        })
                        .sum()
                };
                cost(s).total_cmp(&cost(r))
            })
            .expect("k >= 3");
        let t: Vec<Vec3> = (0..k).map(|i| t[(i + best) % k]).collect();
        // Every side must be flat.
        for i in 0..k {
            let j = (i + 1) % k;
            let (e1, e2, e3) = (b[j] - b[i], t[i] - b[i], t[j] - b[i]);
            let vol = e1.cross(e2).dot(e3);
            let scale = e1.length() * e2.length() * e3.length();
            if scale > 0.0 && vol.abs() > 1e-9 * scale {
                return Err(format!(
                    "side '{}' is twisted: its four corners are not in one plane (corresponding edges of the two outlines must be parallel or meet)",
                    nb[i]
                ));
            }
        }
        let mut s = SolidBuilder::new();
        let bv: Vec<Id> = b.iter().map(|&p| s.vertex(p)).collect();
        let tv: Vec<Id> = t.iter().map(|&p| s.vertex(p)).collect();
        let x_dir = (b[1] - b[0]).normalized(tol).ok_or("a zero-length edge")?;
        let nbot = -newell(&b).normalized(tol).ok_or("an outline has no area")?;
        let ntop = newell(&t).normalized(tol).ok_or("an outline has no area")?;
        s.face(
            "cap_start".into(),
            Surface::Plane {
                origin: b[0],
                normal: nbot,
                x_dir,
            },
            true,
            &[bv.iter().rev().copied().collect()],
        );
        s.face(
            "cap_end".into(),
            Surface::Plane {
                origin: t[0],
                normal: ntop,
                x_dir,
            },
            true,
            std::slice::from_ref(&tv),
        );
        for i in 0..k {
            let j = (i + 1) % k;
            let n = (b[j] - b[i])
                .cross(t[j] - b[j])
                .normalized(tol)
                .ok_or_else(|| format!("side '{}' has no area", nb[i]))?;
            s.face(
                format!("side:{}", nb[i]),
                Surface::Plane {
                    origin: b[i],
                    normal: n,
                    x_dir: (b[j] - b[i]).normalized(tol).ok_or("a zero-length edge")?,
                },
                true,
                &[vec![bv[i], bv[j], tv[j], tv[i]]],
            );
        }
        let solid = s.finish();
        let issues = solid.validate();
        if issues.is_empty() {
            Ok(solid)
        } else {
            Err(format!(
                "the loft is not a valid solid: {}",
                issues
                    .iter()
                    .map(|i| i.message.clone())
                    .collect::<Vec<_>>()
                    .join("; ")
            ))
        }
    }
}
