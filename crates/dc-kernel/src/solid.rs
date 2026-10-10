use dc_math::{det, Vec3};

pub type Id = u32;

pub(crate) const TWO_PI: f64 = 2.0 * core::f64::consts::PI;

/// Edge geometry. Direction runs from the edge's `v0` to its `v1`.
#[derive(Clone, Debug, PartialEq)]
pub enum Curve {
    Line,
    /// Counter-clockwise (about `axis`) from parameter `t0` by `sweep`:
    /// `p(t) = center + radius * (cos t * x_dir + sin t * (axis × x_dir))`.
    /// A full circle has `sweep = 2π` and `v0 == v1`.
    Circle {
        center: Vec3,
        axis: Vec3,
        x_dir: Vec3,
        radius: f64,
        t0: f64,
        sweep: f64,
    },
    /// An ellipse (a circle seen obliquely, e.g. a plane cutting a cylinder), counter-clockwise
    /// about `axis` from parameter `t0` by `sweep`:
    /// `p(t) = center + a * cos t * x_dir + b * sin t * (axis × x_dir)`, with `x_dir` along the
    /// major or minor axis (`a` and `b` are the two semi-axes, either may be larger).
    Ellipse {
        center: Vec3,
        axis: Vec3,
        x_dir: Vec3,
        a: f64,
        b: f64,
        t0: f64,
        sweep: f64,
    },
}

#[derive(Clone, Debug, PartialEq)]
pub struct Edge {
    /// Deterministic name derived from how the edge was made (e.g. `base:l1`).
    pub name: String,
    pub v0: Id,
    pub v1: Id,
    pub curve: Curve,
}

/// A use of an edge by a face loop, in the loop's direction.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CoEdge {
    pub edge: Id,
    pub reversed: bool,
}

#[derive(Clone, Debug, PartialEq)]
pub enum Surface {
    /// Infinite plane; the in-plane frame is `x_dir` and `normal × x_dir`.
    Plane {
        origin: Vec3,
        normal: Vec3,
        x_dir: Vec3,
    },
    /// Rectangular patch of a cylinder: angle `phi0..phi1` (about `axis`, from `ref_dir`) and
    /// axial offset `h0..h1` from `origin`. The surface normal points radially outward.
    Cylinder {
        origin: Vec3,
        axis: Vec3,
        ref_dir: Vec3,
        radius: f64,
        phi0: f64,
        phi1: f64,
        h0: f64,
        h1: f64,
    },
    /// Patch of a cone: angle `phi0..phi1` about `axis` (from `ref_dir`), axial offsets `h0..h1`
    /// from `origin`, radius `r0` at `h0` and `r1` at `h1` (linear between). The surface normal
    /// points away from the axis (and along the slope).
    Cone {
        origin: Vec3,
        axis: Vec3,
        ref_dir: Vec3,
        h0: f64,
        r0: f64,
        h1: f64,
        r1: f64,
        phi0: f64,
        phi1: f64,
    },
    /// Patch of a torus (a round edge): the tube's centre circle has radius `major` about `axis`
    /// through `origin`; a point is `origin + (major + minor cos th) e(phi) + minor sin th axis`,
    /// for `phi0..phi1` (from `ref_dir`) and `th0..th1`. The surface normal points away from the
    /// tube's centre.
    Torus {
        origin: Vec3,
        axis: Vec3,
        ref_dir: Vec3,
        major: f64,
        minor: f64,
        phi0: f64,
        phi1: f64,
        th0: f64,
        th1: f64,
    },
}

/// 8-point Gauss-Legendre nodes and weights on `[-1, 1]`: exact for polynomials up to degree 15
/// and, for the smooth trigonometric integrands used here, exact to rounding.
const GL8: [(f64, f64); 8] = [
    (-0.960_289_856_497_536_3, 0.101_228_536_290_376_26),
    (-0.796_666_477_413_626_7, 0.222_381_034_453_374_47),
    (-0.525_532_409_916_329, 0.313_706_645_877_887_3),
    (-0.183_434_642_495_649_8, 0.362_683_783_378_362),
    (0.183_434_642_495_649_8, 0.362_683_783_378_362),
    (0.525_532_409_916_329, 0.313_706_645_877_887_3),
    (0.796_666_477_413_626_7, 0.222_381_034_453_374_47),
    (0.960_289_856_497_536_3, 0.101_228_536_290_376_26),
];

/// Integral of `f` over `[a, b]` by 8-point Gauss-Legendre on each of `parts` equal pieces.
pub(crate) fn gauss(a: f64, b: f64, parts: usize, f: &mut dyn FnMut(f64) -> f64) -> f64 {
    let w = (b - a) / parts as f64;
    let mut total = 0.0;
    for k in 0..parts {
        let (lo, hi) = (a + w * k as f64, a + w * (k + 1) as f64);
        let (mid, half) = (0.5 * (lo + hi), 0.5 * (hi - lo));
        for (x, wt) in GL8 {
            total += wt * half * f(mid + half * x);
        }
    }
    total
}

#[derive(Clone, Debug, PartialEq)]
pub struct Face {
    /// Deterministic name (`cap_start`, `cap_end`, `side:<sketch entity>`).
    pub name: String,
    pub surface: Surface,
    /// True if the solid's outward normal equals the surface normal; false if opposite.
    pub same_sense: bool,
    /// `loops[0]` is the outer boundary, the rest are holes. Every loop runs counter-clockwise
    /// when seen from outside the solid (outer) or clockwise (holes).
    pub loops: Vec<Vec<CoEdge>>,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Bounds {
    pub min: Vec3,
    pub max: Vec3,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Solid {
    pub vertices: Vec<Vec3>,
    pub edges: Vec<Edge>,
    pub faces: Vec<Face>,
}

/// Why a face could not be turned into a sketch plane.
#[derive(Clone, Debug, PartialEq)]
pub enum FaceFrameError {
    NotFound { available: Vec<String> },
    NotPlanar,
}

impl Curve {
    /// `(center, u, w, semi-axis along u, semi-axis along w, t0, sweep)` of a circle or ellipse.
    pub(crate) fn frame_public(&self) -> Option<(Vec3, Vec3, Vec3, f64, f64, f64, f64)> {
        self.frame()
    }

    fn frame(&self) -> Option<(Vec3, Vec3, Vec3, f64, f64, f64, f64)> {
        match self {
            Curve::Line => None,
            Curve::Circle {
                center,
                axis,
                x_dir,
                radius,
                t0,
                sweep,
            } => Some((
                *center,
                *x_dir,
                axis.cross(*x_dir),
                *radius,
                *radius,
                *t0,
                *sweep,
            )),
            Curve::Ellipse {
                center,
                axis,
                x_dir,
                a,
                b,
                t0,
                sweep,
            } => Some((*center, *x_dir, axis.cross(*x_dir), *a, *b, *t0, *sweep)),
        }
    }

    /// Point on a circle or ellipse at parameter `t`. `None` for lines.
    pub fn circle_point(&self, t: f64) -> Option<Vec3> {
        let (c, u, w, a, b, _, _) = self.frame()?;
        Some(c + u * (a * det::cos(t)) + w * (b * det::sin(t)))
    }

    /// Derivative of [`circle_point`](Self::circle_point) with respect to `t`.
    pub fn circle_tangent(&self, t: f64) -> Option<Vec3> {
        let (_, u, w, a, b, _, _) = self.frame()?;
        Some(u * (-a * det::sin(t)) + w * (b * det::cos(t)))
    }

    /// True for circles and ellipses (edges that are sampled, not straight).
    pub fn is_curved(&self) -> bool {
        !matches!(self, Curve::Line)
    }

    /// The longer semi-axis (the radius of a circle); 0 for a line.
    pub fn max_radius(&self) -> f64 {
        self.frame().map_or(0.0, |(_, _, _, a, b, _, _)| a.max(b))
    }

    /// Parameter range `(t0, sweep)` of a circle or ellipse.
    pub fn range(&self) -> Option<(f64, f64)> {
        self.frame().map(|(_, _, _, _, _, t0, sweep)| (t0, sweep))
    }
}

impl Surface {
    /// Distance from the surface (and, for cylinders, from its axial extent) to `p`.
    /// Zero (within tolerance) means `p` lies on the face's surface patch.
    pub fn deviation(&self, p: Vec3) -> f64 {
        match self {
            Surface::Plane { origin, normal, .. } => (p - *origin).dot(*normal).abs(),
            // A cylinder face is bounded by its loops, not by an extent of the surface, so only
            // the distance from the (unbounded) cylinder counts.
            Surface::Cylinder {
                origin,
                axis,
                radius,
                ..
            } => {
                let d = p - *origin;
                let h = d.dot(*axis);
                let radial = (d - *axis * h).length();
                (radial - radius).abs()
            }
            Surface::Cone {
                origin,
                axis,
                h0,
                r0,
                h1,
                r1,
                ..
            } => {
                let d = p - *origin;
                let h = d.dot(*axis);
                let radial = (d - *axis * h).length();
                let slope = (r1 - r0) / (h1 - h0);
                let at = r0 + slope * (h - h0);
                // Distance to the generating line, and how far past either end of the patch.
                let line = (radial - at).abs() / det::sqrt(1.0 + slope * slope);
                let past = (*h0 - h).max(h - *h1).max(0.0);
                line.max(past)
            }
            Surface::Torus {
                origin,
                axis,
                major,
                minor,
                ..
            } => {
                let d = p - *origin;
                let h = d.dot(*axis);
                let radial = (d - *axis * h).length();
                let tube = det::sqrt((radial - major) * (radial - major) + h * h);
                (tube - minor).abs()
            }
        }
    }
}

impl Solid {
    /// The two line integrals that give a cylinder face's area and volume from its boundary
    /// alone (so the face may be trimmed by any loops, not only a rectangle of the surface):
    /// with `phi` the angle about the axis and `h` the height along it, `L1 = Σ ∫ h dphi` and
    /// `L2 = Σ ∫ G(phi) dh` with `G = o1 sin phi - o2 cos phi` (`o1`, `o2` the origin's
    /// components along `ref_dir` and `axis × ref_dir`), summed over every coedge in loop
    /// direction. Then area = `-s r L1` and the volume share is `r/3 (L2 - r L1)`, where `s` is
    /// +1 for `same_sense`. See `docs/kernel/boolean.md`.
    fn cylinder_integrals(&self, face: &Face) -> (f64, f64) {
        let Surface::Cylinder {
            origin,
            axis,
            ref_dir,
            ..
        } = &face.surface
        else {
            return (0.0, 0.0);
        };
        let e2 = axis.cross(*ref_dir);
        let (o1, o2) = (origin.dot(*ref_dir), origin.dot(e2));
        let g = |phi: f64| o1 * det::sin(phi) - o2 * det::cos(phi);
        let phi_of = |p: Vec3| {
            let d = p - *origin;
            det::atan2(d.dot(e2), d.dot(*ref_dir))
        };
        let h_of = |p: Vec3| (p - *origin).dot(*axis);
        let (mut l1, mut l2) = (0.0, 0.0);
        for c in face.loops.iter().flatten() {
            let e = &self.edges[c.edge as usize];
            let sign = if c.reversed { -1.0 } else { 1.0 };
            match &e.curve {
                Curve::Line => {
                    // A line on a cylinder is a ruling: constant angle, height changes.
                    let (a, b) = {
                        let (s, t) = self.coedge_ends(*c);
                        (self.vertex(s), self.vertex(t))
                    };
                    l2 += g(phi_of(a)) * (h_of(b) - h_of(a));
                }
                Curve::Circle {
                    center,
                    axis: ca,
                    sweep,
                    ..
                } if ca.cross(*axis).length() <= 1e-9 => {
                    let turn = if ca.dot(*axis) > 0.0 { *sweep } else { -*sweep };
                    l1 += h_of(*center) * turn * sign;
                }
                curve => {
                    let (t0, sweep) = curve.range().expect("curved edge");
                    let i1 = gauss(t0, t0 + sweep, 16, &mut |t| {
                        let p = curve.circle_point(t).expect("point");
                        let dp = curve.circle_tangent(t).expect("tangent");
                        let d = p - *origin;
                        let (x, y) = (d.dot(*ref_dir), d.dot(e2));
                        let (dx, dy) = (dp.dot(*ref_dir), dp.dot(e2));
                        h_of(p) * (x * dy - y * dx) / (x * x + y * y)
                    });
                    let i2 = gauss(t0, t0 + sweep, 16, &mut |t| {
                        let p = curve.circle_point(t).expect("point");
                        let dp = curve.circle_tangent(t).expect("tangent");
                        g(phi_of(p)) * dp.dot(*axis)
                    });
                    l1 += i1 * sign;
                    l2 += i2 * sign;
                }
            }
        }
        (l1, l2)
    }

    pub fn vertex(&self, id: Id) -> Vec3 {
        self.vertices[id as usize]
    }

    /// Start and end vertex of a coedge in loop direction.
    pub fn coedge_ends(&self, c: CoEdge) -> (Id, Id) {
        let e = &self.edges[c.edge as usize];
        if c.reversed {
            (e.v1, e.v0)
        } else {
            (e.v0, e.v1)
        }
    }

    /// Point on an edge at normalized parameter `f` in `0..=1`, in the edge's own direction.
    pub fn edge_point(&self, edge: Id, f: f64) -> Vec3 {
        let e = &self.edges[edge as usize];
        match &e.curve {
            Curve::Line => {
                let (a, b) = (self.vertex(e.v0), self.vertex(e.v1));
                a + (b - a) * f
            }
            c => {
                let (t0, sweep) = c.range().expect("a curved edge has a range");
                c.circle_point(t0 + sweep * f).expect("circle has points")
            }
        }
    }

    pub fn edge_length(&self, edge: Id) -> f64 {
        let e = &self.edges[edge as usize];
        match &e.curve {
            Curve::Line => (self.vertex(e.v1) - self.vertex(e.v0)).length(),
            Curve::Circle { radius, sweep, .. } => radius * sweep,
            c @ Curve::Ellipse { t0, sweep, .. } => gauss(*t0, t0 + sweep, 8, &mut |t| {
                c.circle_tangent(t).expect("ellipse").length()
            }),
        }
    }

    /// The frame of a planar face, for placing a sketch on it: a point on the face's plane, the
    /// outward unit normal, and an in-plane x direction (the y axis is `normal × x`, so the
    /// frame is right-handed about the outward normal).
    pub fn planar_face_frame(&self, name: &str) -> Result<(Vec3, Vec3, Vec3), FaceFrameError> {
        let face =
            self.faces
                .iter()
                .find(|f| f.name == name)
                .ok_or_else(|| FaceFrameError::NotFound {
                    available: self.faces.iter().map(|f| f.name.clone()).collect(),
                })?;
        match &face.surface {
            Surface::Plane {
                origin,
                normal,
                x_dir,
            } => {
                let out = if face.same_sense { *normal } else { -*normal };
                Ok((*origin, out, *x_dir))
            }
            Surface::Cylinder { .. } | Surface::Cone { .. } | Surface::Torus { .. } => {
                Err(FaceFrameError::NotPlanar)
            }
        }
    }

    /// Combines two solids into one body list (used when an operation yields several pieces).
    /// Indices of `other` are shifted; nothing else changes.
    pub fn merged(mut self, other: Solid) -> Solid {
        let (dv, de) = (self.vertices.len() as Id, self.edges.len() as Id);
        self.vertices.extend(other.vertices);
        self.edges.extend(other.edges.into_iter().map(|mut e| {
            e.v0 += dv;
            e.v1 += dv;
            e
        }));
        self.faces.extend(other.faces.into_iter().map(|mut f| {
            for c in f.loops.iter_mut().flatten() {
                c.edge += de;
            }
            f
        }));
        self
    }

    /// Exact face area.
    pub fn face_area(&self, face: &Face) -> f64 {
        match &face.surface {
            Surface::Cylinder { radius, .. } => {
                let (l1, _) = self.cylinder_integrals(face);
                -(if face.same_sense { 1.0 } else { -1.0 }) * radius * l1
            }
            Surface::Cone {
                h0,
                r0,
                h1,
                r1,
                phi0,
                phi1,
                ..
            } => {
                let slant = det::sqrt((h1 - h0) * (h1 - h0) + (r1 - r0) * (r1 - r0));
                (phi1 - phi0) * (r0 + r1) * 0.5 * slant
            }
            Surface::Torus {
                major,
                minor,
                phi0,
                phi1,
                th0,
                th1,
                ..
            } => {
                minor
                    * (phi1 - phi0)
                    * (major * (th1 - th0) + minor * (det::sin(*th1) - det::sin(*th0)))
            }
            Surface::Plane {
                origin,
                normal,
                x_dir,
            } => {
                let y_dir = normal.cross(*x_dir);
                let to2 = |p: Vec3| {
                    let d = p - *origin;
                    (d.dot(*x_dir), d.dot(y_dir))
                };
                let cross = |a: (f64, f64), b: (f64, f64)| a.0 * b.1 - a.1 * b.0;
                let mut total = 0.0;
                for lp in &face.loops {
                    for &c in lp {
                        let e = &self.edges[c.edge as usize];
                        let term = match &e.curve {
                            Curve::Line => {
                                0.5 * cross(to2(self.vertex(e.v0)), to2(self.vertex(e.v1)))
                            }
                            curve @ (Curve::Circle { .. } | Curve::Ellipse { .. }) => {
                                let (center, ux, w, ra, rb, t0, sweep) =
                                    curve.frame_public().expect("curved");
                                let (c2, u2, w2) = (
                                    to2(center),
                                    (ux.dot(*x_dir), ux.dot(y_dir)),
                                    (w.dot(*x_dir), w.dot(y_dir)),
                                );
                                let s = cross(u2, w2);
                                let t1 = t0 + sweep;
                                0.5 * (s * ra * rb * sweep
                                    + cross(c2, u2) * ra * (det::cos(t1) - det::cos(t0))
                                    + cross(c2, w2) * rb * (det::sin(t1) - det::sin(t0)))
                            }
                        };
                        total += if c.reversed { -term } else { term };
                    }
                }
                if face.same_sense {
                    total
                } else {
                    -total
                }
            }
        }
    }

    pub fn area(&self) -> f64 {
        self.faces.iter().map(|f| self.face_area(f)).sum()
    }

    /// Exact volume by the divergence theorem. Negative if the solid is inside out.
    pub fn volume(&self) -> f64 {
        let mut v = 0.0;
        for f in &self.faces {
            let s = if f.same_sense { 1.0 } else { -1.0 };
            v += match &f.surface {
                Surface::Plane { origin, normal, .. } => {
                    (origin.dot(*normal) * s) * self.face_area(f) / 3.0
                }
                Surface::Cylinder { radius, .. } => {
                    let (l1, l2) = self.cylinder_integrals(f);
                    radius / 3.0 * (l2 - radius * l1)
                }
                Surface::Cone {
                    origin,
                    axis,
                    ref_dir,
                    h0,
                    r0,
                    h1,
                    r1,
                    phi0,
                    phi1,
                } => {
                    // (1/3) of the flux of p through the surface; see `Face` normals above.
                    let e2 = axis.cross(*ref_dir);
                    let (oa, o1, o2) = (origin.dot(*axis), origin.dot(*ref_dir), origin.dot(e2));
                    let slope = (r1 - r0) / (h1 - h0);
                    let ie1 = det::sin(*phi1) - det::sin(*phi0);
                    let ie2 = -(det::cos(*phi1) - det::cos(*phi0));
                    let dphi = phi1 - phi0;
                    let flux = gauss(*h0, *h1, 2, &mut |h| {
                        let r = r0 + slope * (h - h0);
                        r * ((o1 * ie1 + o2 * ie2) + dphi * (-slope * oa + r - slope * h))
                    });
                    s * flux / 3.0
                }
                Surface::Torus {
                    origin,
                    axis,
                    ref_dir,
                    major,
                    minor,
                    phi0,
                    phi1,
                    th0,
                    th1,
                } => {
                    let e2 = axis.cross(*ref_dir);
                    let (oa, o1, o2) = (origin.dot(*axis), origin.dot(*ref_dir), origin.dot(e2));
                    let ie1 = det::sin(*phi1) - det::sin(*phi0);
                    let ie2 = -(det::cos(*phi1) - det::cos(*phi0));
                    let dphi = phi1 - phi0;
                    let flux = gauss(*th0, *th1, 2, &mut |th| {
                        let (c, sn) = (det::cos(th), det::sin(th));
                        let rho = major + minor * c;
                        let z = minor * sn;
                        minor
                            * rho
                            * (c * (o1 * ie1 + o2 * ie2) + dphi * (sn * oa + rho * c + z * sn))
                    });
                    s * flux / 3.0
                }
            };
        }
        v
    }

    /// Exact axis-aligned bounding box (includes arc extremes, not just vertices).
    pub fn bounds(&self) -> Option<Bounds> {
        let mut it = self.vertices.iter();
        let first = *it.next()?;
        let (mut lo, mut hi) = (first, first);
        let mut grow = |p: Vec3| {
            lo = Vec3::new(lo.x.min(p.x), lo.y.min(p.y), lo.z.min(p.z));
            hi = Vec3::new(hi.x.max(p.x), hi.y.max(p.y), hi.z.max(p.z));
        };
        for &p in it {
            grow(p);
        }
        for e in &self.edges {
            let Some((_, u, w, ra, rb, t0, sweep)) = e.curve.frame() else {
                continue;
            };
            // Per coordinate axis the extremes sit where d/dt of the coordinate is zero.
            for k in 0..3 {
                let (uk, wk) = ([u.x, u.y, u.z][k], [w.x, w.y, w.z][k]);
                let base = det::atan2(rb * wk, ra * uk);
                for t in [base, base + core::f64::consts::PI] {
                    let d = (t - t0) - TWO_PI * ((t - t0) / TWO_PI).floor();
                    if d <= sweep {
                        if let Some(p) = e.curve.circle_point(t0 + d) {
                            grow(p);
                        }
                    }
                }
            }
        }
        Some(Bounds { min: lo, max: hi })
    }

    /// Canonical bytes of the whole solid (names, topology, exact float bits), for hashing.
    /// Two solids built by the same commands on any platform produce identical bytes.
    pub fn canonical_bytes(&self) -> Vec<u8> {
        let mut o = Vec::new();
        let mut f = |o: &mut Vec<u8>, v: f64| o.extend_from_slice(&v.to_bits().to_le_bytes());
        let u = |o: &mut Vec<u8>, v: u64| o.extend_from_slice(&v.to_le_bytes());
        let vec = |o: &mut Vec<u8>, p: Vec3, f: &mut dyn FnMut(&mut Vec<u8>, f64)| {
            f(o, p.x);
            f(o, p.y);
            f(o, p.z);
        };
        u(&mut o, self.vertices.len() as u64);
        for p in &self.vertices {
            vec(&mut o, *p, &mut f);
        }
        u(&mut o, self.edges.len() as u64);
        for e in &self.edges {
            o.extend_from_slice(e.name.as_bytes());
            o.push(0);
            u(&mut o, u64::from(e.v0));
            u(&mut o, u64::from(e.v1));
            match &e.curve {
                Curve::Line => o.push(0),
                Curve::Circle {
                    center,
                    axis,
                    x_dir,
                    radius,
                    t0,
                    sweep,
                } => {
                    o.push(1);
                    vec(&mut o, *center, &mut f);
                    vec(&mut o, *axis, &mut f);
                    vec(&mut o, *x_dir, &mut f);
                    f(&mut o, *radius);
                    f(&mut o, *t0);
                    f(&mut o, *sweep);
                }
                Curve::Ellipse {
                    center,
                    axis,
                    x_dir,
                    a,
                    b,
                    t0,
                    sweep,
                } => {
                    o.push(2);
                    vec(&mut o, *center, &mut f);
                    vec(&mut o, *axis, &mut f);
                    vec(&mut o, *x_dir, &mut f);
                    for x in [a, b, t0, sweep] {
                        f(&mut o, *x);
                    }
                }
            }
        }
        u(&mut o, self.faces.len() as u64);
        for fa in &self.faces {
            o.extend_from_slice(fa.name.as_bytes());
            o.push(0);
            o.push(u8::from(fa.same_sense));
            match &fa.surface {
                Surface::Plane {
                    origin,
                    normal,
                    x_dir,
                } => {
                    o.push(0);
                    vec(&mut o, *origin, &mut f);
                    vec(&mut o, *normal, &mut f);
                    vec(&mut o, *x_dir, &mut f);
                }
                Surface::Cylinder {
                    origin,
                    axis,
                    ref_dir,
                    radius,
                    phi0,
                    phi1,
                    h0,
                    h1,
                } => {
                    o.push(1);
                    vec(&mut o, *origin, &mut f);
                    vec(&mut o, *axis, &mut f);
                    vec(&mut o, *ref_dir, &mut f);
                    for x in [radius, phi0, phi1, h0, h1] {
                        f(&mut o, *x);
                    }
                }
                Surface::Cone {
                    origin,
                    axis,
                    ref_dir,
                    h0,
                    r0,
                    h1,
                    r1,
                    phi0,
                    phi1,
                } => {
                    o.push(2);
                    vec(&mut o, *origin, &mut f);
                    vec(&mut o, *axis, &mut f);
                    vec(&mut o, *ref_dir, &mut f);
                    for x in [h0, r0, h1, r1, phi0, phi1] {
                        f(&mut o, *x);
                    }
                }
                Surface::Torus {
                    origin,
                    axis,
                    ref_dir,
                    major,
                    minor,
                    phi0,
                    phi1,
                    th0,
                    th1,
                } => {
                    o.push(3);
                    vec(&mut o, *origin, &mut f);
                    vec(&mut o, *axis, &mut f);
                    vec(&mut o, *ref_dir, &mut f);
                    for x in [major, minor, phi0, phi1, th0, th1] {
                        f(&mut o, *x);
                    }
                }
            }
            u(&mut o, fa.loops.len() as u64);
            for lp in &fa.loops {
                u(&mut o, lp.len() as u64);
                for c in lp {
                    u(&mut o, u64::from(c.edge));
                    o.push(u8::from(c.reversed));
                }
            }
        }
        o
    }
}
