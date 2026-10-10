//! Rounding a circular rim: the full circle where a flat end face meets a cylindrical wall (the
//! top of a shaft, the mouth of a hole). A chamfer replaces the edge with a cone, a fillet with a
//! quarter of a torus; the rest of the solid is untouched.
//!
//! Method: the end face's circle moves in by the size (outer rim) or out (a hole's mouth), the
//! wall gets shorter by the size, and one new face joins the two new circles. Only walls that are
//! a single full cylinder patch and a rim that is a whole circle are handled; everything else is
//! refused with the reason rather than approximated.

use crate::solid::{CoEdge, Curve, Edge, Face, Id, Solid, Surface, TWO_PI};
use dc_math::{Tolerance, Vec3};

/// What to put in place of the edge.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RimRound {
    /// A flat bevel (a cone), `size` along the end face and along the wall.
    Chamfer,
    /// A circular arc of radius `size`, tangent to the end face and the wall.
    Fillet,
}

#[derive(Clone, Debug, PartialEq)]
pub enum RimError {
    /// A face with this name does not exist.
    NotFound(String),
    /// The edge is not one this operation can round; the reason says why.
    Unsupported(String),
    /// The size does not fit; the reason says what limits it.
    TooLarge(String),
    /// The result failed the validity check (a bug); the issues found.
    Invalid(Vec<String>),
}

impl Solid {
    /// Rounds the circular rim shared by faces `face_a` and `face_b` (in either order): one must
    /// be a plane, the other a full cylinder. The new face is called `new_face`; its two edges
    /// are called `<new_face>:cap` (against the end face) and `<new_face>:wall`.
    pub fn round_rim(
        &self,
        face_a: &str,
        face_b: &str,
        kind: RimRound,
        size: f64,
        new_face: &str,
    ) -> Result<Solid, RimError> {
        let tol = Tolerance::DEFAULT.linear;
        let find = |n: &str| {
            self.faces
                .iter()
                .position(|f| f.name == n)
                .ok_or_else(|| RimError::NotFound(n.to_owned()))
        };
        let (ia, ib) = (find(face_a)?, find(face_b)?);
        let is_plane = |i: usize| matches!(self.faces[i].surface, Surface::Plane { .. });
        let is_cyl = |i: usize| matches!(self.faces[i].surface, Surface::Cylinder { .. });
        let (ip, iw) = match (is_plane(ia), is_cyl(ib), is_plane(ib), is_cyl(ia)) {
            (true, true, _, _) => (ia, ib),
            (_, _, true, true) => (ib, ia),
            _ => {
                return Err(RimError::Unsupported(format!(
                    "'{face_a}' and '{face_b}' are not a flat face and a cylinder; only the rim between a flat end and a round wall can be rounded across the axis"
                )))
            }
        };
        if !size.is_finite() || size <= 0.0 {
            return Err(RimError::TooLarge(format!(
                "the size must be greater than zero, got {size}"
            )));
        }
        let (plane_face, wall) = (&self.faces[ip], &self.faces[iw]);
        let Surface::Cylinder {
            origin,
            axis,
            ref_dir,
            radius,
            phi0,
            phi1,
            h0,
            h1,
        } = wall.surface.clone()
        else {
            unreachable!()
        };
        let Surface::Plane {
            normal: plane_normal,
            ..
        } = plane_face.surface
        else {
            unreachable!()
        };
        if ((phi1 - phi0) - TWO_PI).abs() > 1e-9 {
            return Err(RimError::Unsupported(format!(
                "'{}' is only part of a cylinder; a rim can be rounded when the wall goes all the way round",
                wall.name
            )));
        }
        // The one edge the two faces share.
        let ids = |f: &Face| -> Vec<Id> { f.loops.iter().flatten().map(|c| c.edge).collect() };
        let (pe, we) = (ids(plane_face), ids(wall));
        let shared: Vec<Id> = pe.iter().copied().filter(|e| we.contains(e)).collect();
        let [rim] = shared[..] else {
            return Err(RimError::Unsupported(format!(
                "'{}' and '{}' meet along {} edges; exactly one whole circle is needed",
                plane_face.name,
                wall.name,
                shared.len()
            )));
        };
        let Curve::Circle {
            center,
            axis: rim_axis,
            x_dir,
            radius: rim_radius,
            t0,
            sweep,
        } = self.edges[rim as usize].curve.clone()
        else {
            return Err(RimError::Unsupported(
                "the edge between the faces is straight, not a circle".into(),
            ));
        };
        let rim_edge = &self.edges[rim as usize];
        if (sweep - TWO_PI).abs() > 1e-9 || rim_edge.v0 != rim_edge.v1 {
            return Err(RimError::Unsupported(
                "the rim is only part of a circle; a whole circle is needed".into(),
            ));
        }
        // Which end of the wall the rim is at, and which way is "away from the wall".
        let h_rim = (center - origin).dot(axis);
        let s = if (h_rim - h1).abs() <= tol {
            1.0
        } else if (h_rim - h0).abs() <= tol {
            -1.0
        } else {
            return Err(RimError::Unsupported(
                "the rim is not at an end of the wall".into(),
            ));
        };
        let plane_out = if plane_face.same_sense {
            plane_normal
        } else {
            -plane_normal
        };
        if plane_out.dot(axis) * s < 0.5 {
            return Err(RimError::Unsupported(
                "this edge is an inside corner (a pocket floor against its wall); only outside edges, where material falls away, can be rounded across the axis".into(),
            ));
        }
        if wall.loops.len() != 1 {
            return Err(RimError::Unsupported(format!(
                "'{}' has holes in it; rounding across the axis needs a plain wall",
                wall.name
            )));
        }
        // The wall may be built of several layers: the straight edge that meets the rim is the
        // top piece of its seam, and the round must fit inside that piece.
        let v_rim_id = self.edges[rim as usize].v0;
        let top_piece = self
            .edges
            .iter()
            .filter(|e| matches!(e.curve, Curve::Line) && (e.v0 == v_rim_id || e.v1 == v_rim_id))
            .map(|e| (self.vertex(e.v1) - self.vertex(e.v0)).length())
            .next();
        if let Some(len) = top_piece {
            if size >= len - tol {
                return Err(RimError::TooLarge(format!(
                    "the top layer of the wall '{}' is only {len:.4} thick; the size must be less than that",
                    wall.name
                )));
            }
        }
        let convex = wall.same_sense;
        let d = size;
        let height = h1 - h0;
        if d >= height - tol {
            return Err(RimError::TooLarge(format!(
                "the wall '{}' is only {height:.4} long; the size must be less than that",
                wall.name
            )));
        }
        if convex && d >= radius - tol {
            return Err(RimError::TooLarge(format!(
                "the rim has radius {radius:.4}; the size must be less than that"
            )));
        }
        let new_r = if convex { radius - d } else { radius + d };
        // Whatever else is on the end face must stay clear of the moved circle.
        let radial = |p: Vec3| {
            let v = p - origin;
            (v - axis * v.dot(axis)).length()
        };
        for (li, lp) in plane_face.loops.iter().enumerate() {
            for c in lp {
                if c.edge == rim {
                    continue;
                }
                let e = &self.edges[c.edge as usize];
                let mut dists: Vec<f64> = Vec::new();
                match e.curve {
                    Curve::Line => {
                        // Nearest and farthest points of a segment from the axis.
                        let (a, b) = (self.vertex(e.v0) - origin, self.vertex(e.v1) - origin);
                        let (a, b) = (a - axis * a.dot(axis), b - axis * b.dot(axis));
                        let ab = b - a;
                        let t = if ab.dot(ab) > 0.0 {
                            (-a.dot(ab) / ab.dot(ab)).clamp(0.0, 1.0)
                        } else {
                            0.0
                        };
                        dists.extend([a.length(), b.length(), (a + ab * t).length()]);
                    }
                    Curve::Circle { .. } | Curve::Ellipse { .. } => {
                        dists.extend(
                            (0..16).map(|k| radial(self.edge_point(c.edge, f64::from(k) / 16.0))),
                        );
                    }
                }
                for r in dists {
                    let clear = if convex {
                        r < new_r - tol
                    } else {
                        r > new_r + tol
                    };
                    if !clear {
                        return Err(RimError::TooLarge(format!(
                            "the rounded edge would reach another edge of '{}' (loop {li}) at distance {r:.4} from the axis",
                            plane_face.name
                        )));
                    }
                }
            }
        }

        // ---- the new geometry ----
        let v_rim = self.vertex(rim_edge.v0);
        let u = (v_rim - center) / rim_radius;
        let c_wall = center - axis * (s * d);
        let mut out = self.clone();
        // The rim's vertex moves to the end face's new circle; the wall's end gets a new one.
        let vc = rim_edge.v0;
        out.vertices[vc as usize] = center + u * new_r;
        let vw = out.vertices.len() as Id;
        out.vertices.push(c_wall + u * radius);

        // The end face's circle moves; a copy of the old circle, shifted down the wall, becomes
        // the wall's new end.
        out.edges[rim as usize].curve = Curve::Circle {
            center,
            axis: rim_axis,
            x_dir,
            radius: new_r,
            t0,
            sweep,
        };
        let name = new_face.to_owned();
        let wall_edge = out.edges.len() as Id;
        out.edges.push(Edge {
            name: format!("{name}:wall"),
            v0: vw,
            v1: vw,
            curve: Curve::Circle {
                center: c_wall,
                axis: rim_axis,
                x_dir,
                radius,
                t0,
                sweep,
            },
        });
        // The wall's straight edge that ended at the old vertex now ends at the new one.
        let mut moved = 0;
        for e in &mut out.edges {
            if matches!(e.curve, Curve::Line) {
                if e.v0 == rim_edge.v0 {
                    e.v0 = vw;
                    moved += 1;
                }
                if e.v1 == rim_edge.v0 {
                    e.v1 = vw;
                    moved += 1;
                }
            }
        }
        if moved != 1 {
            return Err(RimError::Unsupported(
                "the wall's seam could not be found".into(),
            ));
        }
        // Wall: its end edge is the new circle, and it is shorter.
        let rim_in_wall = out.faces[iw].loops[0]
            .iter()
            .position(|c| c.edge == rim)
            .ok_or_else(|| RimError::Unsupported("the wall does not use the rim".into()))?;
        let wall_flag = out.faces[iw].loops[0][rim_in_wall].reversed;
        out.faces[iw].loops[0][rim_in_wall].edge = wall_edge;
        let plane_flag = out.faces[ip]
            .loops
            .iter()
            .flatten()
            .find(|c| c.edge == rim)
            .map(|c| c.reversed)
            .ok_or_else(|| RimError::Unsupported("the end face does not use the rim".into()))?;
        if let Surface::Cylinder { h0: lo, h1: hi, .. } = &mut out.faces[iw].surface {
            if s > 0.0 {
                *hi -= d;
            } else {
                *lo += d;
            }
        }

        // The seam of the new face and the face itself.
        let seam = out.edges.len() as Id;
        let (surface, seam_edge, seam_wall_first) = match kind {
            RimRound::Chamfer => {
                let (lo_h, lo_r, hi_h, hi_r) = if s > 0.0 {
                    (-d, radius, 0.0, new_r)
                } else {
                    (0.0, new_r, d, radius)
                };
                (
                    Surface::Cone {
                        origin: center,
                        axis,
                        ref_dir,
                        h0: lo_h,
                        r0: lo_r,
                        h1: hi_h,
                        r1: hi_r,
                        phi0,
                        phi1,
                    },
                    Edge {
                        name: format!("{name}:seam"),
                        v0: vw,
                        v1: vc,
                        curve: Curve::Line,
                    },
                    true,
                )
            }
            RimRound::Fillet => {
                let major = new_r;
                let half = core::f64::consts::FRAC_PI_2;
                let (th_lo, th_hi, wall_is_lo) = match (convex, s > 0.0) {
                    (true, true) => (0.0, half, true),
                    (true, false) => (-half, 0.0, false),
                    (false, true) => (half, core::f64::consts::PI, false),
                    (false, false) => (-core::f64::consts::PI, -half, true),
                };
                let tube_centre = c_wall + u * major;
                let (v0, v1) = if wall_is_lo { (vw, vc) } else { (vc, vw) };
                (
                    Surface::Torus {
                        origin: c_wall,
                        axis,
                        ref_dir,
                        major,
                        minor: d,
                        phi0,
                        phi1,
                        th0: th_lo,
                        th1: th_hi,
                    },
                    Edge {
                        name: format!("{name}:seam"),
                        v0,
                        v1,
                        curve: Curve::Circle {
                            center: tube_centre,
                            axis: u.cross(axis),
                            x_dir: u,
                            radius: d,
                            t0: th_lo,
                            sweep: th_hi - th_lo,
                        },
                    },
                    wall_is_lo,
                )
            }
        };
        out.edges.push(seam_edge);
        // Round the loop: the wall end circle, the seam from there to the end face's circle, that
        // circle, and the seam back. Each circle is used the other way round from its other face.
        let (up, down) = if seam_wall_first {
            (
                CoEdge {
                    edge: seam,
                    reversed: false,
                },
                CoEdge {
                    edge: seam,
                    reversed: true,
                },
            )
        } else {
            (
                CoEdge {
                    edge: seam,
                    reversed: true,
                },
                CoEdge {
                    edge: seam,
                    reversed: false,
                },
            )
        };
        out.faces.push(Face {
            name,
            surface,
            same_sense: if matches!(kind, RimRound::Fillet) {
                true
            } else {
                convex
            },
            loops: vec![vec![
                CoEdge {
                    edge: wall_edge,
                    reversed: !wall_flag,
                },
                up,
                CoEdge {
                    edge: rim,
                    reversed: !plane_flag,
                },
                down,
            ]],
        });
        // The cap edge of the new face keeps the end face's circle (renamed so it says what it
        // now separates).
        out.edges[rim as usize].name = format!("{new_face}:cap");
        let issues = out.validate();
        if !issues.is_empty() {
            return Err(RimError::Invalid(
                issues.iter().map(|i| format!("{i:?}")).collect(),
            ));
        }
        Ok(out)
    }
}
