//! The OpenGL viewport. Geometry is uploaded once per scene change; each frame only sends a
//! matrix and a few uniforms, so orbiting costs the same for a part with ten faces or ten
//! thousand. If anything about GL setup fails, `GlShared::take_error` reports it and the UI falls
//! back to the software renderer (which is also what agents get, pixel for pixel).

use crate::state::Material;
use dc_math::Vec3;
use dc_render::{raster::grid_step, Camera, FaceRef, ProjectionKind, Scene};
use eframe::glow::{self, HasContext};
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc, Mutex,
};

const BACKGROUND: [f32; 3] = [236.0 / 255.0, 238.0 / 255.0, 242.0 / 255.0];

/// Scene geometry in GPU-ready form. Rebuilt only when the scene changes.
pub struct Geometry {
    pub version: u64,
    /// pos(3) normal(3) colour(3) face-id(1) per vertex, three vertices per triangle.
    tris: Vec<f32>,
    /// pos(3) colour(3) per vertex, two per segment.
    edges: Vec<f32>,
    /// The two face normals at each edge, six floats per edge (see `quads`).
    edge_normals: Vec<f32>,
    sketches: Vec<f32>,
    /// The triangles of curved faces (those whose corner normals differ), for finding the
    /// outline of a curved surface: pos(3) normal(3) per corner, three corners each.
    curved: Vec<f32>,
    /// `(body, face)` for each face id used in `tris`.
    faces: Vec<(String, String)>,
}

fn col(c: [u8; 3]) -> [f32; 3] {
    [
        f32::from(c[0]) / 255.0,
        f32::from(c[1]) / 255.0,
        f32::from(c[2]) / 255.0,
    ]
}

/// The inverse of a column-major 4x4 matrix (identity if it is singular).
fn invert4(m: &[f32; 16]) -> [f32; 16] {
    let a: Vec<f64> = m.iter().map(|v| f64::from(*v)).collect();
    let mut inv = [0f64; 16];
    inv[0] = a[5] * a[10] * a[15] - a[5] * a[11] * a[14] - a[9] * a[6] * a[15]
        + a[9] * a[7] * a[14]
        + a[13] * a[6] * a[11]
        - a[13] * a[7] * a[10];
    inv[4] = -a[4] * a[10] * a[15] + a[4] * a[11] * a[14] + a[8] * a[6] * a[15]
        - a[8] * a[7] * a[14]
        - a[12] * a[6] * a[11]
        + a[12] * a[7] * a[10];
    inv[8] = a[4] * a[9] * a[15] - a[4] * a[11] * a[13] - a[8] * a[5] * a[15]
        + a[8] * a[7] * a[13]
        + a[12] * a[5] * a[11]
        - a[12] * a[7] * a[9];
    inv[12] = -a[4] * a[9] * a[14] + a[4] * a[10] * a[13] + a[8] * a[5] * a[14]
        - a[8] * a[6] * a[13]
        - a[12] * a[5] * a[10]
        + a[12] * a[6] * a[9];
    inv[1] = -a[1] * a[10] * a[15] + a[1] * a[11] * a[14] + a[9] * a[2] * a[15]
        - a[9] * a[3] * a[14]
        - a[13] * a[2] * a[11]
        + a[13] * a[3] * a[10];
    inv[5] = a[0] * a[10] * a[15] - a[0] * a[11] * a[14] - a[8] * a[2] * a[15]
        + a[8] * a[3] * a[14]
        + a[12] * a[2] * a[11]
        - a[12] * a[3] * a[10];
    inv[9] = -a[0] * a[9] * a[15] + a[0] * a[11] * a[13] + a[8] * a[1] * a[15]
        - a[8] * a[3] * a[13]
        - a[12] * a[1] * a[11]
        + a[12] * a[3] * a[9];
    inv[13] = a[0] * a[9] * a[14] - a[0] * a[10] * a[13] - a[8] * a[1] * a[14]
        + a[8] * a[2] * a[13]
        + a[12] * a[1] * a[10]
        - a[12] * a[2] * a[9];
    inv[2] = a[1] * a[6] * a[15] - a[1] * a[7] * a[14] - a[5] * a[2] * a[15]
        + a[5] * a[3] * a[14]
        + a[13] * a[2] * a[7]
        - a[13] * a[3] * a[6];
    inv[6] = -a[0] * a[6] * a[15] + a[0] * a[7] * a[14] + a[4] * a[2] * a[15]
        - a[4] * a[3] * a[14]
        - a[12] * a[2] * a[7]
        + a[12] * a[3] * a[6];
    inv[10] = a[0] * a[5] * a[15] - a[0] * a[7] * a[13] - a[4] * a[1] * a[15]
        + a[4] * a[3] * a[13]
        + a[12] * a[1] * a[7]
        - a[12] * a[3] * a[5];
    inv[14] = -a[0] * a[5] * a[14] + a[0] * a[6] * a[13] + a[4] * a[1] * a[14]
        - a[4] * a[2] * a[13]
        - a[12] * a[1] * a[6]
        + a[12] * a[2] * a[5];
    inv[3] = -a[1] * a[6] * a[11] + a[1] * a[7] * a[10] + a[5] * a[2] * a[11]
        - a[5] * a[3] * a[10]
        - a[9] * a[2] * a[7]
        + a[9] * a[3] * a[6];
    inv[7] = a[0] * a[6] * a[11] - a[0] * a[7] * a[10] - a[4] * a[2] * a[11]
        + a[4] * a[3] * a[10]
        + a[8] * a[2] * a[7]
        - a[8] * a[3] * a[6];
    inv[11] = -a[0] * a[5] * a[11] + a[0] * a[7] * a[9] + a[4] * a[1] * a[11]
        - a[4] * a[3] * a[9]
        - a[8] * a[1] * a[7]
        + a[8] * a[3] * a[5];
    inv[15] = a[0] * a[5] * a[10] - a[0] * a[6] * a[9] - a[4] * a[1] * a[10]
        + a[4] * a[2] * a[9]
        + a[8] * a[1] * a[6]
        - a[8] * a[2] * a[5];
    let det = a[0] * inv[0] + a[1] * inv[4] + a[2] * inv[8] + a[3] * inv[12];
    let mut out = [0f32; 16];
    if det.abs() < 1e-300 {
        for (i, o) in out.iter_mut().enumerate() {
            *o = if i % 5 == 0 { 1.0 } else { 0.0 };
        }
        return out;
    }
    for i in 0..16 {
        out[i] = (inv[i] / det) as f32;
    }
    out
}

/// Segments (`pos(3) colour(3)` per end, two ends) as the quads `THICK_VS` expands: six vertices
/// of `pos(3) other(3) colour(3) side(1) end(1) normal1(3) normal2(3)`. `normals` (6 per segment)
/// are the outward normals of the two faces at an edge; without them they are zero, which the
/// shader reads as "draw at the plain weight".
fn quads(lines: &[f32], normals: Option<&[f32]>) -> Vec<f32> {
    let mut out = Vec::with_capacity(lines.len() / 12 * 102);
    for (i, seg) in lines.chunks_exact(12).enumerate() {
        let (a, b) = (&seg[0..6], &seg[6..12]);
        let n: [f32; 6] = normals
            .and_then(|n| n.get(i * 6..i * 6 + 6))
            .map_or([0.0; 6], |s| [s[0], s[1], s[2], s[3], s[4], s[5]]);
        // (this end, the other end, side, which end). The quad's corners are a+n, a-n, b+n, b-n.
        for (p, q, side, end) in [
            (a, b, 1.0f32, -1.0f32),
            (a, b, -1.0, -1.0),
            (b, a, 1.0, 1.0),
            (a, b, -1.0, -1.0),
            (b, a, -1.0, 1.0),
            (b, a, 1.0, 1.0),
        ] {
            out.extend_from_slice(&p[0..3]);
            out.extend_from_slice(&q[0..3]);
            out.extend_from_slice(&p[3..6]);
            out.push(side);
            out.push(end);
            out.extend_from_slice(&n);
        }
    }
    out
}

fn push_line(out: &mut Vec<f32>, a: Vec3, b: Vec3, c: [f32; 3]) {
    for p in [a, b] {
        out.extend_from_slice(&[p.x as f32, p.y as f32, p.z as f32, c[0], c[1], c[2]]);
    }
}

/// The outline of curved surfaces as seen from here: where the surface turns edge-on to the
/// viewer (its normal is square to the line of sight). In each curved triangle the sign of
/// `normal . to_eye` is known at the three corners; where it changes, the contour crosses the
/// triangle, and the two crossing points (found by linear interpolation along the edges) are
/// one segment of it. Same layout as `Geometry::edges`.
fn silhouette_lines(g: &Geometry, eye: [f32; 3], persp: bool, toward: [f32; 3]) -> Vec<f32> {
    let colour = col([38, 40, 48]);
    let mut out = Vec::new();
    for t in g.curved.chunks_exact(18) {
        let mut s = [0f32; 3];
        for (k, c) in t.chunks_exact(6).enumerate() {
            let e = if persp {
                let d = [eye[0] - c[0], eye[1] - c[1], eye[2] - c[2]];
                let l = (d[0] * d[0] + d[1] * d[1] + d[2] * d[2]).sqrt().max(1e-9);
                [d[0] / l, d[1] / l, d[2] / l]
            } else {
                toward
            };
            s[k] = c[3] * e[0] + c[4] * e[1] + c[5] * e[2];
        }
        let mut hits: Vec<[f32; 3]> = Vec::with_capacity(2);
        for (i, j) in [(0usize, 1usize), (1, 2), (2, 0)] {
            if (s[i] > 0.0) != (s[j] > 0.0) {
                let u = s[i] / (s[i] - s[j]);
                let (a, b) = (&t[i * 6..i * 6 + 3], &t[j * 6..j * 6 + 3]);
                hits.push([
                    a[0] + (b[0] - a[0]) * u,
                    a[1] + (b[1] - a[1]) * u,
                    a[2] + (b[2] - a[2]) * u,
                ]);
            }
        }
        if let [a, b] = hits[..] {
            out.extend_from_slice(&[a[0], a[1], a[2], colour[0], colour[1], colour[2]]);
            out.extend_from_slice(&[b[0], b[1], b[2], colour[0], colour[1], colour[2]]);
        }
    }
    out
}

impl Geometry {
    pub fn from_scene(scene: &Scene, version: u64) -> Geometry {
        let mut g = Geometry {
            version,
            tris: Vec::new(),
            edges: Vec::new(),
            edge_normals: Vec::new(),
            sketches: Vec::new(),
            curved: Vec::new(),
            faces: Vec::new(),
        };
        for body in &scene.bodies {
            let base = g.faces.len();
            for name in &body.face_names {
                g.faces.push((body.id.clone(), name.clone()));
            }
            let c = col(body.color);
            for (ti, tri) in body.mesh.triangles.iter().enumerate() {
                let gid = (base + body.mesh.face_of[ti] as usize) as f32;
                let ns = &body.corner_normals[ti];
                if (ns[1] - ns[0]).length() > 1e-4 || (ns[2] - ns[0]).length() > 1e-4 {
                    for (&v, n) in tri.iter().zip(ns) {
                        let p = body.mesh.positions[v as usize];
                        g.curved.extend_from_slice(&[
                            p.x as f32, p.y as f32, p.z as f32, n.x as f32, n.y as f32, n.z as f32,
                        ]);
                    }
                }
                for (&v, n) in tri.iter().zip(&body.corner_normals[ti]) {
                    let p = body.mesh.positions[v as usize];
                    g.tris.extend_from_slice(&[
                        p.x as f32, p.y as f32, p.z as f32, n.x as f32, n.y as f32, n.z as f32,
                        c[0], c[1], c[2], gid,
                    ]);
                }
            }
            for (e, n) in body.edges.iter().zip(&body.edge_normals) {
                push_line(&mut g.edges, e[0], e[1], col([38, 40, 48]));
                for v in n {
                    g.edge_normals
                        .extend_from_slice(&[v.x as f32, v.y as f32, v.z as f32]);
                }
            }
        }
        for l in &scene.lines {
            for w in l.points.windows(2) {
                push_line(&mut g.sketches, w[0], w[1], col(l.color));
            }
        }
        g
    }

    fn face_id(&self, f: &Option<FaceRef>) -> f32 {
        f.as_ref()
            .and_then(|f| {
                self.faces
                    .iter()
                    .position(|(b, n)| *b == f.body && *n == f.face)
            })
            .map_or(-1.0, |i| i as f32)
    }
}

/// What the UI knows about the view besides the camera.
pub struct ViewState {
    /// Viewport width / height.
    pub aspect: f64,
    /// Diagonal of the scene's bounding box, to size the depth range.
    pub extent: f64,
    /// The world grid (not shown by the app; agents can ask for it).
    pub grid: bool,
    /// The model origin triad.
    pub origins: bool,
    /// Physical pixels per point (the screen's scale), so lines keep their weight on a sharp display.
    pub pixels_per_point: f32,
    /// The viewport's height in physical pixels (marks that keep one size on screen need it).
    pub viewport_px_height: f32,
    /// Ambient occlusion: concave corners and pockets are darkened, as in SolidWorks and Inventor.
    pub ambient_occlusion: bool,
    /// An extra smoothing pass over the finished image (on top of the multisampling).
    pub fxaa: bool,
    /// The part's soft reflection in the ground (RealView only).
    pub floor_reflection: bool,
    /// What RealView's shiny surfaces reflect.
    pub environment: crate::environment::Environment,
    /// The grid and axes of the sketch plane being edited.
    pub local_grid: Option<dc_render::raster::GridPlane>,
    pub edges: bool,
    pub sketches: bool,
    pub selected: Option<FaceRef>,
    pub hovered: Option<FaceRef>,
    /// `Some` draws with realistic shading in this material.
    pub realview: Option<Material>,
    /// Bounding box of the solid bodies (sizes the light and the ground).
    pub bounds: Option<(Vec3, Vec3)>,
}

/// Realistic-shading inputs for one frame.
pub struct Real {
    light: [f32; 16],
    eye: [f32; 3],
    key: [f32; 3],
    albedo: [f32; 3],
    metallic: f32,
    roughness: f32,
    /// A clear glossy layer over the base, 0..1.
    clearcoat: f32,
    center: [f32; 3],
    radius: f32,
    normal_bias: f32,
    /// Ground quad under the part (a shadow catcher), same layout as the triangles.
    floor: Vec<f32>,
    /// Height of the ground, for the reflection of the part in it.
    floor_z: f32,
}

/// Direction toward the key light, fixed in the world so shadows stay put while you orbit.
const KEY: [f64; 3] = [0.55, -0.60, 0.52];

/// Everything one frame needs besides the uploaded geometry.
pub struct Frame {
    pub geom: Arc<Geometry>,
    mvp: [f32; 16],
    basis: [[f32; 3]; 3],
    selected: f32,
    hovered: f32,
    /// Grid and axes (they depend on the zoom), same layout as `Geometry::edges`.
    overlay: Vec<f32>,
    /// Outlines of curved surfaces for this view, same layout as `Geometry::edges`.
    silhouette: Vec<f32>,
    ppp: f32,
    /// Thinner lines for a model that is small on screen, so its outlines do not eat the shape.
    edge_scale: f32,
    /// Screen to world, for ambient occlusion (column-major), and where the eye is.
    inv_mvp: [f32; 16],
    eye3: [f32; 3],
    /// How far, in world units, a surface can be shadowed by its neighbours.
    ao_radius: f32,
    ao: bool,
    fxaa: bool,
    persp: bool,
    /// Draw the part's reflection in the ground (RealView).
    reflection: bool,
    environment: crate::environment::Environment,
    /// The camera-following lights of RealView (fill, rim, head), as world directions.
    rig: [[f32; 3]; 3],
    pub edges: bool,
    pub sketches: bool,
    real: Option<Real>,
}

impl Frame {
    pub fn new(geom: Arc<Geometry>, camera: &Camera, v: &ViewState) -> Frame {
        let ViewState {
            aspect,
            extent,
            grid,
            origins,
            pixels_per_point,
            viewport_px_height,
            ambient_occlusion,
            fxaa,
            floor_reflection,
            environment,
            local_grid,
            edges,
            sketches,
            selected,
            hovered,
            realview,
            bounds,
        } = v;
        let (aspect, extent) = (*aspect, *extent);
        let (grid, edges, sketches) = (*grid, *edges, *sketches);
        let (local_grid, origins) = (*local_grid, *origins);
        let ppp = (*pixels_per_point).max(0.5);
        let (r, u, f) = camera.basis();
        let eye = camera.eye();
        let (re, ue, fe) = (r.dot(eye), u.dot(eye), f.dot(eye));
        let v = |a: Vec3, c: f64| [a.x, a.y, a.z, c];
        // Rows of the clip-space matrix: x, y, z, w.
        let rows: [[f64; 4]; 4] = match camera.projection {
            ProjectionKind::Perspective => {
                let t = camera.tan_half();
                let (n, fa) = (camera.near(), camera.distance * 50.0 + extent * 4.0);
                let a = (fa + n) / (fa - n);
                let b = -2.0 * fa * n / (fa - n);
                let (sx, sy) = (1.0 / (t * aspect), 1.0 / t);
                [
                    scale(v(r, -re), sx),
                    scale(v(u, -ue), sy),
                    {
                        let z = v(f, -fe);
                        [z[0] * a, z[1] * a, z[2] * a, z[3] * a + b]
                    },
                    v(f, -fe),
                ]
            }
            ProjectionKind::Orthographic => {
                let hh = camera.half_height();
                let big = camera.distance * 20.0 + extent * 4.0;
                [
                    scale(v(r, -re), 1.0 / (hh * aspect)),
                    scale(v(u, -ue), 1.0 / hh),
                    scale(v(f, -fe - camera.distance), 1.0 / big),
                    [0.0, 0.0, 0.0, 1.0],
                ]
            }
        };
        let mut mvp = [0f32; 16];
        for (i, row) in rows.iter().enumerate() {
            for (j, x) in row.iter().enumerate() {
                mvp[j * 4 + i] = *x as f32; // column-major
            }
        }
        let inv_mvp = invert4(&mvp);
        // How much of the world one pixel covers: marks that should keep one size on screen
        // (the origin triad, a sketch's origin arrows) are sized with it.
        let world_per_px = 2.0 * camera.half_height() / f64::from((*viewport_px_height).max(1.0));
        let model_px = extent / world_per_px.max(1e-12);
        let edge_scale = (model_px / 650.0).clamp(0.6, 1.0) as f32;
        let mut overlay = Vec::new();
        let step = grid_step(camera.distance);
        // A grid on a plane (origin, two directions), with its two axes if asked.
        let mut plane_grid = |origin: Vec3,
                              ex: Vec3,
                              ey: Vec3,
                              grid_lines: bool,
                              z_axis: Option<Vec3>,
                              reach: f64| {
            let at = |u: f64, v: f64| origin + ex * u + ey * v;
            if grid_lines {
                let n = 20;
                let ext = step * f64::from(n);
                for i in -n..=n {
                    let c = f64::from(i) * step;
                    let colr = if i % 5 == 0 {
                        col([176, 182, 194])
                    } else {
                        col([204, 208, 217])
                    };
                    push_line(&mut overlay, at(c, -ext), at(c, ext), colr);
                    push_line(&mut overlay, at(-ext, c), at(ext, c), colr);
                }
            }
            let len = reach;
            // A plane's axes run both ways; the world's are short stubs from the origin.
            let back = if z_axis.is_none() { -len } else { 0.0 };
            push_line(
                &mut overlay,
                at(back, 0.0),
                at(len, 0.0),
                col([214, 60, 60]),
            );
            push_line(
                &mut overlay,
                at(0.0, back),
                at(0.0, len),
                col([60, 170, 80]),
            );
            if let Some(ez) = z_axis {
                push_line(&mut overlay, origin, origin + ez * len, col([70, 100, 220]));
            }
        };
        // The world's grid (only when asked for) and the model origin.
        if grid || origins {
            plane_grid(
                Vec3::ZERO,
                Vec3::new(1.0, 0.0, 0.0),
                Vec3::new(0.0, 1.0, 0.0),
                grid,
                Some(Vec3::new(0.0, 0.0, 1.0)),
                if origins {
                    world_per_px * 46.0 * f64::from(ppp)
                } else {
                    0.0
                },
            );
        }
        // The plane being sketched on: its origin marker always, its grid when switched on.
        if let Some(l) = local_grid {
            if l.grid {
                plane_grid(l.origin, l.x, l.y, true, None, 0.0);
            }
            for [p, q] in l.origin_marker(world_per_px * 40.0 * f64::from(ppp)) {
                push_line(&mut overlay, p, q, col([214, 60, 60]));
            }
        }
        let b = |a: Vec3| [a.x as f32, a.y as f32, a.z as f32];
        let real = realview
            .zip(*bounds)
            .map(|(m, b)| real_inputs(m, b, camera));
        let eye32 = [eye.x as f32, eye.y as f32, eye.z as f32];
        let toward = [-f.x as f32, -f.y as f32, -f.z as f32];
        let silhouette = if edges {
            silhouette_lines(
                &geom,
                eye32,
                camera.projection == ProjectionKind::Perspective,
                toward,
            )
        } else {
            Vec::new()
        };
        Frame {
            real,
            silhouette,
            selected: geom.face_id(selected),
            hovered: geom.face_id(hovered),
            geom,
            mvp,
            basis: [b(r), b(u), b(f)],
            overlay,
            ppp,
            edge_scale,
            inv_mvp,
            eye3: [eye.x as f32, eye.y as f32, eye.z as f32],
            ao_radius: (extent * 0.07).max(1e-3) as f32,
            ao: *ambient_occlusion,
            fxaa: *fxaa,
            persp: camera.projection == ProjectionKind::Perspective,
            reflection: *floor_reflection,
            environment: *environment,
            rig: {
                // x right, y up, z towards the viewer, in the camera's own frame.
                let to_world = |c: [f64; 3]| {
                    let n = (c[0] * c[0] + c[1] * c[1] + c[2] * c[2]).sqrt();
                    let w = r * (c[0] / n) + u * (c[1] / n) - f * (c[2] / n);
                    [w.x as f32, w.y as f32, w.z as f32]
                };
                [
                    to_world([0.75, 0.10, 0.65]),
                    to_world([-0.25, 0.85, -0.45]),
                    to_world([-0.20, 0.25, 0.95]),
                ]
            },
            edges,
            sketches,
        }
    }
}

impl Frame {
    /// A world point in clip space, exactly as the vertex shader computes it.
    pub fn clip(&self, p: [f32; 3]) -> [f32; 4] {
        let m = &self.mvp;
        let row = |i: usize| m[i] * p[0] + m[4 + i] * p[1] + m[8 + i] * p[2] + m[12 + i];
        [row(0), row(1), row(2), row(3)]
    }

    /// The face ids (selected, hovered) the shader will tint; `None` if not in the scene.
    pub fn highlighted(&self) -> (Option<usize>, Option<usize>) {
        let id = |f: f32| (f >= 0.0).then_some(f as usize);
        (id(self.selected), id(self.hovered))
    }

    /// A world point in the light's clip space (for checking that the shadow map covers the part).
    pub fn light_clip(&self, p: [f32; 3]) -> Option<[f32; 4]> {
        let m = &self.real.as_ref()?.light;
        let row = |i: usize| m[i] * p[0] + m[4 + i] * p[1] + m[8 + i] * p[2] + m[12 + i];
        Some([row(0), row(1), row(2), row(3)])
    }

    /// Segments of the outline of curved surfaces in this view (each is two points).
    pub fn silhouette_segments(&self) -> Vec<[[f32; 3]; 2]> {
        self.silhouette
            .chunks_exact(12)
            .map(|s| [[s[0], s[1], s[2]], [s[6], s[7], s[8]]])
            .collect()
    }

    /// Triangles, edge segments and sketch segments uploaded for this scene.
    pub fn counts(&self) -> (usize, usize, usize) {
        let g = &self.geom;
        (g.tris.len() / 30, g.edges.len() / 12, g.sketches.len() / 12)
    }
}

fn real_inputs(m: Material, (lo, hi): (Vec3, Vec3), camera: &Camera) -> Real {
    let center = (lo + hi) * 0.5;
    let radius = ((hi - lo).length() * 0.5).max(1e-6);
    let key = Vec3::new(KEY[0], KEY[1], KEY[2]);
    let l = key / key.length();
    let helper = if l.z.abs() > 0.95 {
        Vec3::new(0.0, 1.0, 0.0)
    } else {
        Vec3::Z
    };
    let r = helper.cross(l);
    let r = r / r.length();
    let u = l.cross(r);
    // Orthographic light looking along -l, covering the part and the shadow it throws.
    let big = radius * 1.8;
    let row = |a: Vec3, k: f64| [a.x * k, a.y * k, a.z * k, -a.dot(center) * k];
    let rows = [
        row(r, 1.0 / big),
        row(u, 1.0 / big),
        // Nearer the light means a smaller depth.
        row(l, -1.0 / (2.0 * big)),
        [0.0, 0.0, 0.0, 1.0],
    ];
    let mut light = [0f32; 16];
    for (i, row) in rows.iter().enumerate() {
        for (j, x) in row.iter().enumerate() {
            light[j * 4 + i] = *x as f32;
        }
    }
    let (albedo, metallic, roughness) = m.params();
    let clearcoat = m.clearcoat();
    let fz = (lo.z - radius * 0.004) as f32;
    let half = (radius * 3.0) as f32;
    let (cx, cy) = (center.x as f32, center.y as f32);
    let mut floor = Vec::new();
    let quad = [
        (-1.0f32, -1.0f32),
        (1.0, -1.0),
        (1.0, 1.0),
        (-1.0, -1.0),
        (1.0, 1.0),
        (-1.0, 1.0),
    ];
    for (qx, qy) in quad {
        floor.extend_from_slice(&[
            cx + qx * half,
            cy + qy * half,
            fz,
            0.0,
            0.0,
            1.0,
            0.0,
            0.0,
            0.0,
            -1.0,
        ]);
    }
    // Shading looks from a nearby eye even in an orthographic view: with a parallel view every
    // point of a flat face would see the same reflection and the face would be one flat tone.
    // From a finite eye the reflection moves across the face, through the studio's light
    // strips, as it does on real metal.
    let e = if camera.projection == ProjectionKind::Orthographic {
        let dir = camera.eye() - camera.target;
        let dir = dir / dir.length().max(1e-9);
        camera.target + dir * (radius * 2.4).max(1e-6)
    } else {
        camera.eye()
    };
    Real {
        light,
        eye: [e.x as f32, e.y as f32, e.z as f32],
        key: [l.x as f32, l.y as f32, l.z as f32],
        albedo,
        metallic,
        roughness,
        clearcoat,
        center: [cx, cy, center.z as f32],
        radius: radius as f32,
        normal_bias: (radius * 0.004) as f32,
        floor,
        floor_z: fz,
    }
}

fn scale(r: [f64; 4], s: f64) -> [f64; 4] {
    [r[0] * s, r[1] * s, r[2] * s, r[3] * s]
}

// ---- shaders ---------------------------------------------------------------------------------

/// The classic (flat-lit) look; same lighting and colours as the software renderer.
const TRI_VS: &str = r"
uniform mat4 u_mvp;
uniform vec3 u_right;
uniform vec3 u_up;
uniform vec3 u_fwd;
uniform float u_sel;
uniform float u_hov;
in vec3 a_pos;
in vec3 a_nrm;
in vec3 a_col;
in float a_gid;
out vec3 v_col;
void main() {
    vec3 n = vec3(dot(a_nrm, u_right), dot(a_nrm, u_up), dot(a_nrm, u_fwd));
    // A key light, a fill from the other side, and a top light: neighbouring faces get distinct
    // tones, as in a CAD light rig (all in camera space, so they follow the view).
    float key = max(dot(n, vec3(-0.3095, 0.4421, -0.8421)), 0.0);
    float fill = max(dot(n, vec3(0.7071, 0.1000, -0.6999)), 0.0);
    float top = max(dot(n, vec3(0.0, 0.9806, -0.1961)), 0.0);
    vec3 c = a_col * clamp(0.30 + 0.42 * key + 0.32 * fill + 0.12 * top, 0.0, 1.0);
    if (abs(a_gid - u_sel) < 0.5) {
        c = mix(c, vec3(1.0, 0.549, 0.0), 0.6);
    } else if (abs(a_gid - u_hov) < 0.5) {
        c = mix(c, vec3(1.0, 0.863, 0.0), 0.4);
    }
    v_col = c;
    gl_Position = u_mvp * vec4(a_pos, 1.0);
}
";

/// Lines as screen-space quads: an exact width in pixels, square ends, and a soft edge so they
/// are anti-aliased whatever the driver does about multisampling. Each segment is six vertices
/// that all know both of its ends and which side of the line they sit on (+1 / -1).
const THICK_VS: &str = r"
uniform mat4 u_mvp;
uniform vec2 u_px;
uniform float u_w;
uniform vec3 u_eye3;
uniform vec3 u_toward;
uniform float u_persp;
in vec3 a_pos;
in vec3 a_other;
in vec3 a_col;
in float a_side;
in float a_end;
in vec3 a_n1;
in vec3 a_n2;
out vec3 v_col;
out float v_across;
out float v_half;
out float v_alpha;
void main() {
    // Edge weight: a silhouette (one face towards the viewer, the other away) is the heaviest,
    // a crease the plain weight, and a smooth seam between nearly parallel faces is light.
    float k = 1.0;
    float alpha = 1.0;
    if (dot(a_n1, a_n1) > 0.5) {
        vec3 to_eye = u_persp > 0.5 ? normalize(u_eye3 - a_pos) : u_toward;
        bool silhouette = (dot(a_n1, to_eye) > 0.0) != (dot(a_n2, to_eye) > 0.0);
        float smooth_seam = dot(a_n1, a_n2);
        if (silhouette) {
            k = 1.45;
        } else if (smooth_seam > 0.97) {
            k = 0.6;
            alpha = 0.6;
        }
    }
    // Half the width, plus half a pixel of soft edge.
    float half_w = u_w * k * 0.5 + 0.5;
    vec4 ca = u_mvp * vec4(a_pos, 1.0);
    vec4 cb = u_mvp * vec4(a_other, 1.0);
    vec2 sa = ca.xy / ca.w * u_px * 0.5;
    vec2 sb = cb.xy / cb.w * u_px * 0.5;
    vec2 out_dir = sa - sb;
    float len = length(out_dir);
    out_dir = len > 1e-6 ? out_dir / len : vec2(1.0, 0.0);
    vec2 nrm = vec2(-out_dir.y, out_dir.x);
    // Out past the end by a third of the half width (short caps: long ones poke out of corners
    // where several lines meet), and sideways by the half width.
    // (nrm flips with the end it is measured at, a_end undoes that: one common side.)
    vec2 off = out_dir * half_w * 0.3 + nrm * a_end * a_side * half_w;
    vec2 ndc = (sa + off) * 2.0 / u_px;
    v_col = a_col;
    v_across = a_side;
    v_half = half_w;
    v_alpha = alpha;
    // A little towards the viewer, so a line wins against the faces it borders (whose depth
    // slopes away at glancing angles) but not against anything really in front of it.
    gl_Position = vec4(ndc * ca.w, ca.z - 0.00004 * ca.w, ca.w);
}
";

const THICK_FS: &str = r"
in vec3 v_col;
in float v_across;
in float v_half;
in float v_alpha;
out vec4 o_color;
void main() {
    float cover = clamp((1.0 - abs(v_across)) * v_half, 0.0, 1.0);
    o_color = vec4(v_col, cover * v_alpha);
}
";

/// What the surface shaders share for ambient occlusion: the blurred AO texture, looked up by
/// where on the viewport the pixel is.
const AO_LOOKUP: &str = r"
uniform sampler2D u_ao;
uniform vec4 u_vp;
uniform float u_ao_on;
float ambient_occlusion() {
    if (u_ao_on < 0.5) { return 1.0; }
    return texture(u_ao, (gl_FragCoord.xy - u_vp.xy) / u_vp.zw).r;
}
";

/// The classic look with ambient occlusion: the occlusion darkens the colour, more where it is
/// strong, never to black.
const TRI_FS: &str = r"
in vec3 v_col;
out vec4 o_color;
void main() {
    float ao = ambient_occlusion();
    o_color = vec4(v_col * mix(0.74, 1.0, ao), 1.0);
}
";

/// The viewport gradient, drawn inside the scene buffer so the whole frame is one opaque image.
const BG_FS: &str = r"
uniform vec2 u_px;
out vec4 o_color;
float sm(float a, float b, float x) {
    float t = clamp((x - a) / (b - a), 0.0, 1.0);
    return t * t * (3.0 - 2.0 * t);
}
void main() {
    // White with a soft lavender band across the middle (SolidWorks' three point faded look).
    // The same formula, with its derivation, is `backdrop::background_at`.
    float u = gl_FragCoord.x / u_px.x;
    float v = 1.0 - gl_FragCoord.y / u_px.y;
    float d = v - (0.5298 + (0.4889 - 0.5298) * u);
    float k;
    if (d > 0.0) {
        k = 1.0 - sm(0.0, 0.2028, d);
    } else {
        float e = 0.246 + 1.0032 * sm(0.3656, 1.3, u);
        float t = clamp(-d / e, 0.0, 1.0);
        k = pow(1.0 - sm(0.0, 1.0, t), 0.8653);
    }
    k *= 0.6665 + (1.0 - 0.6665) * u;
    vec3 white = vec3(254.0) / 255.0;
    vec3 peak = vec3(218.0, 222.0, 233.0) / 255.0;
    o_color = vec4(mix(white, peak, k), 1.0);
}
";

/// Puts the resolved scene image into the window.
const COPY_FS: &str = r"
uniform sampler2D u_img;
uniform vec4 u_vp;
uniform float u_fxaa;
out vec4 o_color;

float luma(vec3 c) { return dot(c, vec3(0.299, 0.587, 0.114)); }

void main() {
    vec2 px = gl_FragCoord.xy - u_vp.xy;
    if (u_fxaa < 0.5) {
        o_color = vec4(texelFetch(u_img, ivec2(px), 0).rgb, 1.0);
        return;
    }
    // FXAA (after Lottes): find the edge direction from the luma of the four diagonal
    // neighbours and average along it, keeping the result only if it stays in range.
    vec2 inv = 1.0 / u_vp.zw;
    vec2 uv = px * inv;
    vec3 m = texture(u_img, uv).rgb;
    float lm = luma(m);
    float lnw = luma(texture(u_img, uv + vec2(-1.0, -1.0) * inv).rgb);
    float lne = luma(texture(u_img, uv + vec2(1.0, -1.0) * inv).rgb);
    float lsw = luma(texture(u_img, uv + vec2(-1.0, 1.0) * inv).rgb);
    float lse = luma(texture(u_img, uv + vec2(1.0, 1.0) * inv).rgb);
    float lmin = min(lm, min(min(lnw, lne), min(lsw, lse)));
    float lmax = max(lm, max(max(lnw, lne), max(lsw, lse)));
    vec2 dir = vec2(-((lnw + lne) - (lsw + lse)), (lnw + lsw) - (lne + lse));
    float reduce = max((lnw + lne + lsw + lse) * 0.25 * 0.125, 1.0 / 128.0);
    float rcp = 1.0 / (min(abs(dir.x), abs(dir.y)) + reduce);
    dir = clamp(dir * rcp, vec2(-6.0), vec2(6.0)) * inv;
    vec3 a = 0.5 * (texture(u_img, uv + dir * (1.0 / 3.0 - 0.5)).rgb
                  + texture(u_img, uv + dir * (2.0 / 3.0 - 0.5)).rgb);
    vec3 b = a * 0.5 + 0.25 * (texture(u_img, uv + dir * -0.5).rgb
                             + texture(u_img, uv + dir * 0.5).rgb);
    float lb = luma(b);
    o_color = vec4((lb < lmin || lb > lmax) ? a : b, 1.0);
}
";

/// A full-screen triangle with no vertex data.
const FULL_VS: &str = r"
void main() {
    vec2 p = vec2((gl_VertexID << 1) & 2, gl_VertexID & 2);
    gl_Position = vec4(p * 2.0 - 1.0, 0.0, 1.0);
}
";

/// Screen-space ambient occlusion (an Alchemy-style estimate): for each pixel, rebuild its world
/// position from the depth buffer, find its normal, and look at neighbours within a world radius;
/// those standing up out of its tangent plane close by occlude it.
const AO_FS: &str = r"
uniform sampler2D u_depth;
uniform mat4 u_inv;
uniform vec2 u_px;
uniform vec3 u_eye;
uniform float u_radius;
out vec4 o_color;

vec3 world(ivec2 p) {
    float d = texelFetch(u_depth, p, 0).r;
    vec4 n = vec4((vec2(p) + 0.5) / u_px * 2.0 - 1.0, d * 2.0 - 1.0, 1.0);
    vec4 w = u_inv * n;
    return w.xyz / w.w;
}

void main() {
    // The output is half the size of the depth image: each output pixel stands for the 2 x 2
    // block whose upper-left depth sample is read here.
    ivec2 p = ivec2(gl_FragCoord.xy) * 2;
    float depth = texelFetch(u_depth, p, 0).r;
    if (depth >= 0.99999) { o_color = vec4(1.0); return; }
    vec3 P = world(p);
    vec3 xp = world(p + ivec2(1, 0));
    vec3 xm = world(p - ivec2(1, 0));
    vec3 yp = world(p + ivec2(0, 1));
    vec3 ym = world(p - ivec2(0, 1));
    // The neighbour on the same surface (the nearer one) on each axis.
    vec3 dx = length(xp - P) < length(P - xm) ? xp - P : P - xm;
    vec3 dy = length(yp - P) < length(P - ym) ? yp - P : P - ym;
    vec3 N = normalize(cross(dx, dy));
    if (dot(N, u_eye - P) < 0.0) { N = -N; }
    float pixel = max(length(xp - P), 1e-6);
    float rpx = clamp(u_radius / pixel, 3.0, 48.0);
    float noise = fract(52.9829189 * fract(dot(gl_FragCoord.xy, vec2(0.06711056, 0.00583715))));
    const int K = 10;
    float occ = 0.0;
    for (int i = 0; i < K; i++) {
        float t = (float(i) + 0.5) / float(K);
        float ang = float(i) * 2.39996323 + noise * 6.2831853;
        ivec2 q = p + ivec2(floor(vec2(cos(ang), sin(ang)) * rpx * t + 0.5));
        if (q == p || q.x < 0 || q.y < 0 || q.x >= int(u_px.x) || q.y >= int(u_px.y)) { continue; }
        if (texelFetch(u_depth, q, 0).r >= 0.99999) { continue; }
        vec3 v = world(q) - P;
        float dist = length(v);
        float reach = 1.0 - smoothstep(0.6 * u_radius, 1.2 * u_radius, dist);
        vec3 vs = v / u_radius;
        occ += reach * max(dot(N, vs) - 0.04, 0.0) / (dot(vs, vs) + 0.02);
    }
    float ao = clamp(1.0 - occ * 2.0 / float(K) * 1.0, 0.0, 1.0);
    o_color = vec4(vec3(ao), 1.0);
}
";

/// Blurs the occlusion over a few pixels, but not across depth steps (so it does not bleed over
/// silhouettes).
const AO_BLUR_FS: &str = r"
uniform sampler2D u_ao_in;
uniform sampler2D u_depth;
out vec4 o_color;
void main() {
    // Half-size occlusion image, full-size depth image (see AO_FS).
    ivec2 p = ivec2(gl_FragCoord.xy);
    ivec2 half_max = textureSize(u_ao_in, 0) - 1;
    ivec2 full_max = textureSize(u_depth, 0) - 1;
    float d0 = texelFetch(u_depth, min(p * 2, full_max), 0).r;
    float sum = 0.0;
    float w = 0.0;
    for (int j = -2; j <= 2; j++) {
        for (int i = -2; i <= 2; i++) {
            ivec2 q = clamp(p + ivec2(i, j), ivec2(0), half_max);
            float d = texelFetch(u_depth, min(q * 2, full_max), 0).r;
            float k = abs(d - d0) < 0.0012 ? 1.0 : 0.0;
            sum += texelFetch(u_ao_in, q, 0).r * k;
            w += k;
        }
    }
    o_color = vec4(vec3(sum / max(w, 1.0)), 1.0);
}
";

/// Depth-only pass from the light.
const SHADOW_VS: &str = r"
uniform mat4 u_light;
in vec3 a_pos;
void main() {
    gl_Position = u_light * vec4(a_pos, 1.0);
}
";

const SHADOW_FS: &str = r"
out vec4 o_color;
void main() {
    o_color = vec4(1.0);
}
";

/// RealView surfaces and the ground share this vertex stage.
const REAL_VS: &str = r"
uniform mat4 u_mvp;
uniform mat4 u_light;
uniform float u_nbias;
uniform float u_mirror;
uniform float u_floor_z;
in vec3 a_pos;
in vec3 a_nrm;
in vec3 a_col;
in float a_gid;
out vec3 v_pos;
out vec3 v_nrm;
out vec4 v_sh;
out float v_h;
flat out float v_gid;
void main() {
    // The reflection in the ground: the same part, mirrored in the ground plane. (Shadows are
    // still looked up for the real position, so the image is lit as the part is.)
    vec3 p = a_pos;
    vec3 n = a_nrm;
    v_h = a_pos.z - u_floor_z;
    if (u_mirror > 0.5) {
        p.z = 2.0 * u_floor_z - p.z;
        n.z = -n.z;
    }
    v_pos = p;
    v_nrm = n;
    v_gid = a_gid;
    v_sh = u_light * vec4(a_pos + a_nrm * u_nbias, 1.0);
    gl_Position = u_mvp * vec4(p, 1.0);
}
";

const SHADOW_FN: &str = r"
uniform sampler2D u_shadow;
uniform float u_shadow_on;
uniform float u_texel;
uniform float u_dbias;
uniform float u_light_size;
float shadow_at(vec4 sh) {
    if (u_shadow_on < 0.5) { return 1.0; }
    vec3 c = sh.xyz * 0.5 + 0.5;
    if (c.x < 0.0 || c.x > 1.0 || c.y < 0.0 || c.y > 1.0 || c.z > 1.0) { return 1.0; }
    // Percentage-closer soft shadows: how soft the edge is depends on how far the shadow
    // falls from what casts it. A blocker search first finds how far above this point the
    // casting surface is; the filter is then as wide as a light of u_light_size would make
    // the penumbra, so a shadow is sharp where the part touches the ground and fades with
    // distance, as in a photograph. The taps lie on a disk and are turned by a per-pixel
    // angle, so the sampling shows as fine grain, not steps.
    float ang = 6.2831853 * fract(52.9829189 * fract(dot(gl_FragCoord.xy, vec2(0.06711056, 0.00583715))));
    mat2 rot = mat2(cos(ang), sin(ang), -sin(ang), cos(ang));
    float sum = 0.0;
    float n = 0.0;
    for (int i = 0; i < 12; i++) {
        float r = sqrt((float(i) + 0.5) / 12.0);
        float t = float(i) * 2.39996323;
        vec2 o = rot * (r * vec2(cos(t), sin(t))) * 0.03;
        float d = texture(u_shadow, c.xy + o).r;
        if (c.z - u_dbias > d) { sum += d; n += 1.0; }
    }
    if (n < 0.5) { return 1.0; }
    float gap = c.z - sum / n;
    float rad = clamp(gap * u_light_size, 1.5 * u_texel, 0.03);
    float bias = u_dbias + rad * 0.5;
    float lit = 0.0;
    for (int i = 0; i < 28; i++) {
        float r = sqrt((float(i) + 0.5) / 28.0);
        float t = float(i) * 2.39996323;
        vec2 o = rot * (r * vec2(cos(t), sin(t))) * rad;
        float d = texture(u_shadow, c.xy + o).r;
        lit += (c.z - bias > d) ? 0.0 : 1.0;
    }
    return lit / 28.0;
}
";

/// Physically based shading under a procedural studio: a sky/ground gradient plus three softboxes,
/// the biggest of which is the shadow-casting key light.
const REAL_FS: &str = r"
uniform vec3 u_eye;
uniform vec3 u_key;
uniform vec3 u_albedo;
uniform float u_metal;
uniform float u_rough;
uniform float u_coat;
uniform float u_sel;
uniform float u_hov;
uniform float u_mirror;
uniform float u_reflect;
uniform float u_rad;
in vec3 v_pos;
in vec3 v_nrm;
in vec4 v_sh;
in float v_h;
flat in float v_gid;
out vec4 o_color;

// The fill, rim and head lights follow the camera (set per frame); only the key stays in the world.
uniform vec3 u_fill;
uniform vec3 u_rim;
uniform vec3 u_head;
uniform sampler2D u_envmap;
uniform float u_env_on;
uniform float u_env_lods;
uniform float u_env_gain;
// Equirectangular lookup: azimuth round, elevation down the image (row 0 is straight up).
vec3 env_lod(vec3 d, float lod) {
    float u = atan(d.y, d.x) * 0.15915494 + 0.5;
    float v = acos(clamp(d.z, -1.0, 1.0)) * 0.31830989;
    return textureLod(u_envmap, vec2(u, v), lod).rgb * u_env_gain;
}
// Level k of the texture is the panorama prefiltered for roughness k / u_env_lods.
vec3 env_panorama(vec3 d, float rough) {
    float lod = clamp(rough * u_env_lods, 0.0, u_env_lods);
    return env_lod(d, lod);
}
vec3 fill_dir() { return u_fill; }
vec3 rim_dir() { return u_rim; }

// A tall softbox standing in the studio: bright over a range of azimuths (the direction round
// the vertical axis) and elevations. Upright curved faces, whose reflection sweeps round the
// azimuth, show these as bright stripes, which is what makes a cylinder look like metal.
float tall_light(vec3 d, vec3 l, float width, float z_lo, float z_hi) {
    vec2 a = normalize(d.xy + vec2(1e-5, 0.0));
    vec2 b = normalize(l.xy + vec2(1e-5, 0.0));
    float az = exp((dot(a, b) - 1.0) / width);
    // Straight up or down every azimuth meets in one point: the strips fade out towards it,
    // or all the light would converge on a dot (looking at a face from underneath).
    az *= smoothstep(0.08, 0.55, length(d.xy));
    float up = smoothstep(z_lo - 0.25, z_lo, d.z) * (1.0 - smoothstep(z_hi, z_hi + 0.25, d.z));
    return az * up;
}

vec3 env(vec3 d, float rough) {
    if (u_env_on > 0.5) { return env_panorama(d, rough); }
    float z = d.z;
    // A studio: a dim sky above, a lighter horizon, a mid-grey floor.
    vec3 sky = vec3(0.06, 0.075, 0.11);
    vec3 hor = vec3(0.22, 0.24, 0.28);
    vec3 gnd = vec3(0.16, 0.165, 0.175);
    vec3 c = z > 0.0 ? mix(hor, sky, pow(z, 0.6)) : mix(hor, gnd, pow(-z, 0.5));
    float w = 0.04 + rough * rough * 1.1;
    // A long horizontal strip a little above the horizon, and a floor bounce band below it.
    c += vec3(0.95, 0.97, 1.0) * 0.55 * exp(-pow((z - 0.22) / (0.08 + w * 0.6), 2.0));
    c += vec3(0.84, 0.83, 0.82) * 0.45 * exp(-pow((z + 0.45) / (0.18 + w), 2.0));
    // The overhead softbox and the key (a bright spot, for the sharp highlight).
    c += vec3(1.0, 0.98, 0.96) * 0.30 * exp((dot(d, vec3(0.05, 0.1, 0.99)) - 1.0) / (w * 5.0));
    c += vec3(1.0, 0.97, 0.93) * 3.2 * exp((dot(d, u_key) - 1.0) / (w * 1.6));
    // Tall softboxes that follow the camera: a wide fill from the right, a rim from behind and a
    // narrow one in front, standing on the floor and reaching above the horizon.
    c += vec3(0.88, 0.93, 1.0) * 1.7 * tall_light(d, fill_dir(), 0.05 + w * 1.4, -0.95, 0.55);
    c += vec3(1.0, 0.99, 0.97) * 1.5 * tall_light(d, rim_dir(), 0.03 + w * 1.2, -0.7, 0.85);
    c += vec3(1.0, 0.98, 0.96) * 0.9 * tall_light(d, u_head, 0.02 + w, -0.9, 0.35);
    return c;
}

vec3 irradiance(vec3 n) {
    // The panorama's blurriest copy is the light arriving from everywhere round n.
    if (u_env_on > 0.5) { return env_lod(n, u_env_lods) * 0.55; }
    float h = n.z * 0.5 + 0.5;
    vec3 a = mix(vec3(0.16, 0.165, 0.18), vec3(0.34, 0.37, 0.43), h);
    return a + vec3(0.85, 0.90, 1.0) * 0.35 * max(dot(n, fill_dir()), 0.0);
}

vec3 env_brdf(vec3 f0, float rough, float nv) {
    const vec4 c0 = vec4(-1.0, -0.0275, -0.572, 0.022);
    const vec4 c1 = vec4(1.0, 0.0425, 1.04, -0.04);
    vec4 r = rough * c0 + c1;
    float a004 = min(r.x * r.x, exp2(-9.28 * nv)) * r.x + r.y;
    vec2 ab = vec2(-1.04, 1.04) * a004 + r.zw;
    return f0 * ab.x + ab.y;
}

vec3 aces(vec3 x) {
    return clamp((x * (2.51 * x + 0.03)) / (x * (2.43 * x + 0.59) + 0.14), 0.0, 1.0);
}

void main() {
    vec3 n = normalize(v_nrm);
    vec3 v = normalize(u_eye - v_pos);
    if (dot(n, v) < 0.0) { n = -n; }
    float nv = max(dot(n, v), 1e-3);
    float rough = max(u_rough, 0.05);
    vec3 f0 = mix(vec3(0.04), u_albedo, u_metal);
    vec3 diffuse_colour = u_albedo * (1.0 - u_metal);
    float nl = max(dot(n, u_key), 0.0);
    // A surface turned away from the light is in its own shadow: no need to look it up.
    float sh = nl > 0.0 ? shadow_at(v_sh) : 0.0;
    vec3 direct = diffuse_colour * nl * sh * vec3(1.0, 0.98, 0.94) * 1.15;
    float ao = ambient_occlusion();
    // Occlusion hides the sky from corners and pockets: it dims the ambient light and the
    // environment's reflection there (the direct light is shadow-mapped already).
    vec3 ambient = diffuse_colour * irradiance(n) * 0.85 * ao;
    vec3 spec = env(reflect(-v, n), rough) * env_brdf(f0, rough, nv) * mix(0.72, 1.0, sh) * mix(0.45, 1.0, ao);
    // Plain gamma with a gentle S-curve: no filmic lift, so shaded sides stay shaded and the
    // lit faces keep their punch (the ACES curve this replaced washed everything towards grey).
    vec3 base = direct + ambient + spec;
    if (u_coat > 0.0) {
        // A clear lacquer over the base: a sharp, uncoloured reflection of the studio (a
        // dielectric, so weak head on and strong at grazing angles), and the base seen through
        // it is dimmed by what the coat reflects.
        float fc = (0.04 + 0.96 * pow(1.0 - nv, 5.0)) * u_coat;
        vec3 coat = env(reflect(-v, n), 0.04) * fc * mix(0.72, 1.0, sh) * mix(0.45, 1.0, ao);
        base = base * (1.0 - fc) + coat;
    }
    vec3 c = pow(clamp(base * 0.74, 0.0, 1.0), vec3(1.0 / 2.2));
    c = mix(c, c * c * (3.0 - 2.0 * c), 0.45);
    if (u_mirror > 0.5) {
        // The reflection: faint, fading with height above the ground, premultiplied.
        float a = u_reflect * exp(-3.6 * max(v_h, 0.0) / u_rad);
        o_color = vec4(c * a, a);
        return;
    }
    if (abs(v_gid - u_sel) < 0.5) {
        c = mix(c, vec3(1.0, 0.549, 0.0), 0.55);
    } else if (abs(v_gid - u_hov) < 0.5) {
        c = mix(c, vec3(1.0, 0.863, 0.0), 0.35);
    }
    o_color = vec4(c, 1.0);
}
";

/// A transparent ground that only shows the shadow of the part, plus a soft contact darkening.
const FLOOR_FS: &str = r"
uniform vec3 u_center;
uniform float u_radius;
uniform sampler2D u_refl;
uniform float u_refl_on;
uniform float u_reflect;
in vec3 v_pos;
in vec3 v_nrm;
in vec4 v_sh;
flat in float v_gid;
out vec4 o_color;

// The reflection image averaged over a disk of the given radius in pixels.
vec4 reflection_blur(vec2 uv, float radius_px, float ang, int taps) {
    vec4 sum = vec4(0.0);
    mat2 rot = mat2(cos(ang), sin(ang), -sin(ang), cos(ang));
    for (int i = 0; i < 16; i++) {
        if (i >= taps) { break; }
        float r = sqrt((float(i) + 0.5) / float(taps));
        float t = float(i) * 2.39996323;
        vec2 o = rot * (r * vec2(cos(t), sin(t))) * radius_px / u_vp.zw;
        sum += texture(u_refl, uv + o);
    }
    return sum / float(taps);
}

void main() {
    float d = length(v_pos.xy - u_center.xy) / u_radius;
    float fade = 1.0 - smoothstep(1.3, 2.4, d);
    // Most of the screen is far from the part: nothing to add there (this pass was the
    // single most expensive one before this test).
    if (fade < 0.004) { o_color = vec4(0.0); return; }
    float s = shadow_at(v_sh);
    // The part's occlusion on the ground: it darkens where the part meets it.
    float ao = ambient_occlusion();
    float a = ((1.0 - s) * 0.30 + 0.06 * (1.0 - smoothstep(0.3, 1.2, d)) + (1.0 - ao) * 0.30) * fade;
    vec4 refl = vec4(0.0);
    if (u_refl_on > 0.5) {
        // A glossy floor: the mirror image of the part gets blurrier the higher the reflected
        // point is above the ground. The image fades with that height (its alpha), so the
        // alpha tells how high it is: three blurs, mixed by it.
        vec2 uv = (gl_FragCoord.xy - u_vp.xy) / u_vp.zw;
        float ang = 6.2831853 * fract(52.9829189 * fract(dot(gl_FragCoord.xy, vec2(0.06711056, 0.00583715))));
        // Is there any reflection within reach of the widest blur? Nine taps tell, and where
        // there is none the three blurs (34 taps) are skipped.
        float reach = texture(u_refl, uv).a;
        for (int i = 0; i < 8; i++) {
            float a = float(i) * 0.7853982;
            reach += texture(u_refl, uv + vec2(cos(a), sin(a)) * 16.0 / u_vp.zw).a;
        }
        if (reach > 0.001) {
        vec4 b1 = reflection_blur(uv, 1.5, ang, 6);
        float t = 1.0 - clamp(b1.a / u_reflect, 0.0, 1.0);
        vec4 b2 = reflection_blur(uv, 6.0, ang, 12);
        vec4 b3 = reflection_blur(uv, 18.0, ang, 16);
        refl = (t < 0.5 ? mix(b1, b2, t * 2.0) : mix(b2, b3, t * 2.0 - 1.0)) * fade;
        }
    }
    // The shadow is premultiplied; the reflection shows through where it does not cover.
    o_color = vec4(vec3(0.03, 0.04, 0.07) * a + refl.rgb * (1.0 - a), a + refl.a * (1.0 - a));
}
";

struct Program {
    prog: glow::Program,
}

impl Program {
    unsafe fn new(
        gl: &glow::Context,
        vs: &str,
        fs: &str,
        attribs: &[(&str, u32)],
    ) -> Result<Program, String> {
        let header = "#version 140\n";
        let prog = gl.create_program()?;
        let mut shaders = Vec::new();
        for (kind, src) in [(glow::VERTEX_SHADER, vs), (glow::FRAGMENT_SHADER, fs)] {
            let sh = gl.create_shader(kind)?;
            gl.shader_source(sh, &format!("{header}{src}"));
            gl.compile_shader(sh);
            if !gl.get_shader_compile_status(sh) {
                return Err(format!(
                    "shader compile failed: {}",
                    gl.get_shader_info_log(sh)
                ));
            }
            gl.attach_shader(prog, sh);
            shaders.push(sh);
        }
        for (name, loc) in attribs {
            gl.bind_attrib_location(prog, *loc, name);
        }
        gl.link_program(prog);
        let ok = gl.get_program_link_status(prog);
        for sh in shaders {
            gl.detach_shader(prog, sh);
            gl.delete_shader(sh);
        }
        if !ok {
            return Err(format!(
                "shader link failed: {}",
                gl.get_program_info_log(prog)
            ));
        }
        Ok(Program { prog })
    }

    unsafe fn loc(&self, gl: &glow::Context, name: &str) -> Option<glow::UniformLocation> {
        gl.get_uniform_location(self.prog, name)
    }
}

/// A vertex buffer with its own vertex array.
struct Buffer {
    vao: glow::VertexArray,
    vbo: glow::Buffer,
    count: i32,
}

impl Buffer {
    /// `layout`: `(attribute location, floats)` in order; stride is their sum.
    unsafe fn new(gl: &glow::Context, layout: &[(u32, i32)]) -> Result<Buffer, String> {
        let vao = gl.create_vertex_array()?;
        let vbo = gl.create_buffer()?;
        gl.bind_vertex_array(Some(vao));
        gl.bind_buffer(glow::ARRAY_BUFFER, Some(vbo));
        let stride: i32 = layout.iter().map(|l| l.1).sum::<i32>() * 4;
        let mut off = 0;
        for (loc, n) in layout {
            gl.enable_vertex_attrib_array(*loc);
            gl.vertex_attrib_pointer_f32(*loc, *n, glow::FLOAT, false, stride, off);
            off += n * 4;
        }
        gl.bind_vertex_array(None);
        Ok(Buffer { vao, vbo, count: 0 })
    }

    unsafe fn set(
        &mut self,
        gl: &glow::Context,
        data: &[f32],
        floats_per_vertex: usize,
        usage: u32,
    ) {
        gl.bind_buffer(glow::ARRAY_BUFFER, Some(self.vbo));
        gl.buffer_data_u8_slice(glow::ARRAY_BUFFER, bytes(data), usage);
        self.count = (data.len() / floats_per_vertex) as i32;
    }
}

/// How strong the ground's reflection is at the contact line (it fades with height).
const REFLECTION: f32 = 0.38;

/// Prefiltered roughness levels of the environment texture (level 0 is the sharp image).
const ENV_LEVELS: usize = 8;

fn bytes(d: &[f32]) -> &[u8] {
    // SAFETY: f32 has no padding or invalid bit patterns; the slice covers exactly the same memory.
    unsafe { std::slice::from_raw_parts(d.as_ptr().cast::<u8>(), std::mem::size_of_val(d)) }
}

const SHADOW_SIZE: i32 = 2048;
const THICK_LAYOUT: [(u32, i32); 7] = [(0, 3), (1, 3), (2, 3), (3, 1), (4, 1), (5, 3), (6, 3)];
const TRI_LAYOUT: [(u32, i32); 4] = [(0, 3), (1, 3), (2, 3), (3, 1)];
const TRI_ATTRIBS: [(&str, u32); 4] = [("a_pos", 0), ("a_nrm", 1), ("a_col", 2), ("a_gid", 3)];

struct Shadow {
    fbo: glow::Framebuffer,
    tex: glow::Texture,
}

/// Depth of the part seen from the camera, the raw occlusion, and the blurred occlusion.
struct AoTargets {
    size: (i32, i32),
    depth_tex: glow::Texture,
    depth_fbo: glow::Framebuffer,
    raw_tex: glow::Texture,
    raw_fbo: glow::Framebuffer,
    blur_tex: glow::Texture,
    blur_fbo: glow::Framebuffer,
}

/// The scene is drawn into this multisampled buffer, resolved, and copied into the window, so the
/// anti-aliasing no longer depends on what the window itself was given.
struct SceneTarget {
    size: (i32, i32),
    msaa_fbo: glow::Framebuffer,
    color_rb: glow::Renderbuffer,
    depth_rb: glow::Renderbuffer,
    resolve_fbo: glow::Framebuffer,
    resolve_tex: glow::Texture,
}

/// The part's mirror image in the ground, drawn off screen so the floor can blur it.
struct ReflTarget {
    size: (i32, i32),
    fbo: glow::Framebuffer,
    tex: glow::Texture,
    depth_rb: glow::Renderbuffer,
}

/// Where a frame's time goes, measured only under `--bench` (it waits for the GPU after every
/// pass, which makes frames slower).
struct Profile {
    last: std::time::Instant,
    totals: Vec<(&'static str, f32)>,
}

struct Gpu {
    prof: Option<Profile>,
    tri: Program,
    thick: Program,
    real: Program,
    floor: Program,
    shadow_prog: Program,
    ao_prog: Option<Program>,
    blur_prog: Option<Program>,
    bg_prog: Option<Program>,
    copy_prog: Option<Program>,
    scene: Option<SceneTarget>,
    scene_broken: bool,
    refl: Option<ReflTarget>,
    /// The framebuffer the scene is being drawn into (to return to after an off-screen pass).
    cur_fbo: Option<glow::Framebuffer>,
    /// The environment panorama in use (and which one it is), if RealView has one.
    envs: Vec<(glow::Texture, crate::environment::Environment)>,
    /// Which of `envs` the shader uses (the last one that was ready: while the one asked for is
    /// still being prefiltered the previous one stays on screen).
    env_shown: Option<usize>,
    /// The environment being prefiltered on another thread, and where its levels will arrive.
    env_job: Option<(
        crate::environment::Environment,
        std::sync::mpsc::Receiver<Vec<crate::environment::Panorama>>,
    )>,
    /// Environments whose prefilter thread died; they are not retried.
    env_failed: Vec<crate::environment::Environment>,
    /// Wakes the UI when a prefilter finishes, so the frame that uses it is drawn.
    repaint: Option<Repaint>,
    /// How many blurrier copies of the panorama the shader may read (log2 of its width, less 2).
    env_lods: f32,
    empty_vao: glow::VertexArray,
    /// Targets for the occlusion passes, sized to the viewport; `None` until first used, or if
    /// the driver would not make them.
    ao: Option<AoTargets>,
    /// Whether this frame's occlusion texture is ready for the surface shaders.
    ao_active: bool,
    shadow: Option<Shadow>,
    tris: Buffer,
    edges: Buffer,
    sketches: Buffer,
    overlay: Buffer,
    silhouettes: Buffer,
    floor_quad: Buffer,
    uploaded: Option<u64>,
}

impl Gpu {
    unsafe fn new(gl: &glow::Context) -> Result<Gpu, String> {
        if eframe::egui_glow::ShaderVersion::get(gl) != eframe::egui_glow::ShaderVersion::Gl140 {
            return Err("the GL viewport needs desktop OpenGL 3.1 or newer".into());
        }
        let tri = Program::new(gl, TRI_VS, &format!("{AO_LOOKUP}{TRI_FS}"), &TRI_ATTRIBS)?;
        let thick = Program::new(
            gl,
            THICK_VS,
            THICK_FS,
            &[
                ("a_pos", 0),
                ("a_other", 1),
                ("a_col", 2),
                ("a_side", 3),
                ("a_end", 4),
                ("a_n1", 5),
                ("a_n2", 6),
            ],
        )?;
        // RealView is optional: if its shaders do not build, the classic look still works.
        let real = Program::new(
            gl,
            REAL_VS,
            &format!("{SHADOW_FN}{AO_LOOKUP}{REAL_FS}"),
            &TRI_ATTRIBS,
        )?;
        let floor = Program::new(
            gl,
            REAL_VS,
            &format!("{SHADOW_FN}{AO_LOOKUP}{FLOOR_FS}"),
            &TRI_ATTRIBS,
        )?;
        let shadow_prog = Program::new(gl, SHADOW_VS, SHADOW_FS, &[("a_pos", 0)])?;
        // Ambient occlusion is optional too: without it the surfaces just are not darkened.
        let ao_prog = Program::new(gl, FULL_VS, AO_FS, &[]).ok();
        let blur_prog = Program::new(gl, FULL_VS, AO_BLUR_FS, &[]).ok();
        let bg_prog = Program::new(gl, FULL_VS, BG_FS, &[]).ok();
        let copy_prog = Program::new(gl, FULL_VS, COPY_FS, &[]).ok();
        let empty_vao = gl.create_vertex_array()?;
        Ok(Gpu {
            tri,
            thick,
            real,
            floor,
            shadow_prog,
            ao_prog,
            blur_prog,
            bg_prog,
            copy_prog,
            scene: None,
            scene_broken: false,
            prof: None,
            refl: None,
            cur_fbo: None,
            envs: Vec::new(),
            env_shown: None,
            env_job: None,
            env_failed: Vec::new(),
            repaint: None,
            env_lods: 7.0,
            empty_vao,
            ao: None,
            ao_active: false,
            shadow: Self::shadow_target(gl),
            tris: Buffer::new(gl, &TRI_LAYOUT)?,
            edges: Buffer::new(gl, &THICK_LAYOUT)?,
            sketches: Buffer::new(gl, &THICK_LAYOUT)?,
            overlay: Buffer::new(gl, &THICK_LAYOUT)?,
            silhouettes: Buffer::new(gl, &THICK_LAYOUT)?,
            floor_quad: Buffer::new(gl, &TRI_LAYOUT)?,
            uploaded: None,
        })
    }

    /// A depth-only framebuffer for the light's view; `None` if the driver will not make one
    /// (RealView then draws without shadows).
    unsafe fn shadow_target(gl: &glow::Context) -> Option<Shadow> {
        let tex = gl.create_texture().ok()?;
        gl.bind_texture(glow::TEXTURE_2D, Some(tex));
        gl.tex_image_2d(
            glow::TEXTURE_2D,
            0,
            glow::DEPTH_COMPONENT24 as i32,
            SHADOW_SIZE,
            SHADOW_SIZE,
            0,
            glow::DEPTH_COMPONENT,
            glow::UNSIGNED_INT,
            glow::PixelUnpackData::Slice(None),
        );
        for (p, v) in [
            (glow::TEXTURE_MIN_FILTER, glow::NEAREST),
            (glow::TEXTURE_MAG_FILTER, glow::NEAREST),
            (glow::TEXTURE_WRAP_S, glow::CLAMP_TO_EDGE),
            (glow::TEXTURE_WRAP_T, glow::CLAMP_TO_EDGE),
        ] {
            gl.tex_parameter_i32(glow::TEXTURE_2D, p, v as i32);
        }
        let fbo = gl.create_framebuffer().ok()?;
        gl.bind_framebuffer(glow::FRAMEBUFFER, Some(fbo));
        gl.framebuffer_texture_2d(
            glow::FRAMEBUFFER,
            glow::DEPTH_ATTACHMENT,
            glow::TEXTURE_2D,
            Some(tex),
            0,
        );
        gl.draw_buffers(&[glow::NONE]);
        gl.read_buffer(glow::NONE);
        let ok = gl.check_framebuffer_status(glow::FRAMEBUFFER) == glow::FRAMEBUFFER_COMPLETE;
        gl.bind_framebuffer(glow::FRAMEBUFFER, None);
        ok.then_some(Shadow { fbo, tex })
    }

    /// Closes the pass that just ended (benchmark only): waits for the GPU and adds the time.
    unsafe fn mark(&mut self, gl: &glow::Context, name: &'static str) {
        if let Some(p) = &mut self.prof {
            gl.finish();
            let now = std::time::Instant::now();
            let ms = (now - p.last).as_secs_f32() * 1000.0;
            p.last = now;
            match p.totals.iter_mut().find(|(n, _)| *n == name) {
                Some(e) => e.1 += ms,
                None => p.totals.push((name, ms)),
            }
        }
    }

    unsafe fn draw(
        &mut self,
        gl: &glow::Context,
        vp: [i32; 4],
        target: Option<glow::Framebuffer>,
        f: &Frame,
    ) {
        self.mark(gl, "(between frames)");
        if self.uploaded != Some(f.geom.version) {
            self.tris.set(gl, &f.geom.tris, 10, glow::STATIC_DRAW);
            self.edges.set(
                gl,
                &quads(&f.geom.edges, Some(&f.geom.edge_normals)),
                17,
                glow::STATIC_DRAW,
            );
            self.sketches
                .set(gl, &quads(&f.geom.sketches, None), 17, glow::STATIC_DRAW);
            self.uploaded = Some(f.geom.version);
        }
        self.overlay
            .set(gl, &quads(&f.overlay, None), 17, glow::STREAM_DRAW);
        self.silhouettes
            .set(gl, &quads(&f.silhouette, None), 17, glow::STREAM_DRAW);
        self.mark(gl, "upload");
        let shadows = match (&f.real, &self.shadow) {
            (Some(r), Some(s)) => {
                self.shadow_pass(gl, vp, target, r, s);
                true
            }
            _ => false,
        };

        self.mark(gl, "shadow map");
        // The ground (RealView) is part of the depth the occlusion works from.
        if let Some(r) = &f.real {
            self.floor_quad.set(gl, &r.floor, 10, glow::STREAM_DRAW);
        }
        self.ao_active = f.ao && self.ao_passes(gl, vp, target, f);

        self.mark(gl, "ambient occlusion");
        // The scene goes into a multisampled buffer of its own when the driver allows it.
        let vp_window = vp;
        let offscreen = self.scene_begin(gl, vp);
        let vp = if offscreen {
            [0, 0, vp[2].max(1), vp[3].max(1)]
        } else {
            vp
        };

        self.cur_fbo = if offscreen {
            self.scene.as_ref().map(|s| s.msaa_fbo)
        } else {
            target
        };
        self.mark(gl, "scene begin");
        gl.enable(glow::DEPTH_TEST);
        gl.depth_func(glow::LEQUAL);
        gl.depth_mask(true);
        gl.disable(glow::CULL_FACE);
        gl.clear(glow::DEPTH_BUFFER_BIT);

        // Surfaces, pushed slightly back so their edges win the depth test.
        gl.enable(glow::POLYGON_OFFSET_FILL);
        gl.polygon_offset(1.0, 1.0);
        match &f.real {
            Some(r) => self.draw_real(gl, f, r, shadows, vp),
            None => {
                let p = &self.tri;
                gl.use_program(Some(p.prog));
                gl.uniform_matrix_4_f32_slice(p.loc(gl, "u_mvp").as_ref(), false, &f.mvp);
                gl.uniform_3_f32_slice(p.loc(gl, "u_right").as_ref(), &f.basis[0]);
                gl.uniform_3_f32_slice(p.loc(gl, "u_up").as_ref(), &f.basis[1]);
                gl.uniform_3_f32_slice(p.loc(gl, "u_fwd").as_ref(), &f.basis[2]);
                gl.uniform_1_f32(p.loc(gl, "u_sel").as_ref(), f.selected);
                gl.uniform_1_f32(p.loc(gl, "u_hov").as_ref(), f.hovered);
                self.bind_ao(gl, p, vp);
                gl.bind_vertex_array(Some(self.tris.vao));
                gl.draw_arrays(glow::TRIANGLES, 0, self.tris.count);
            }
        }
        gl.disable(glow::POLYGON_OFFSET_FILL);

        // Lines: anti-aliased quads of exact pixel width, blended over what is behind them.
        gl.use_program(Some(self.thick.prog));
        gl.uniform_matrix_4_f32_slice(self.thick.loc(gl, "u_mvp").as_ref(), false, &f.mvp);
        gl.uniform_3_f32_slice(self.thick.loc(gl, "u_eye3").as_ref(), &f.eye3);
        gl.uniform_3_f32_slice(
            self.thick.loc(gl, "u_toward").as_ref(),
            &[-f.basis[2][0], -f.basis[2][1], -f.basis[2][2]],
        );
        gl.uniform_1_f32(
            self.thick.loc(gl, "u_persp").as_ref(),
            if f.persp { 1.0 } else { 0.0 },
        );
        gl.uniform_2_f32(
            self.thick.loc(gl, "u_px").as_ref(),
            vp[2].max(1) as f32,
            vp[3].max(1) as f32,
        );
        gl.enable(glow::BLEND);
        gl.blend_func(glow::SRC_ALPHA, glow::ONE_MINUS_SRC_ALPHA);
        gl.depth_mask(false);
        // (buffer, width in points): grid and axes light, model edges firm, sketches a touch more.
        let mut lines = vec![(&self.overlay, 1.0)];
        if f.edges {
            lines.push((&self.edges, 1.25));
            lines.push((&self.silhouettes, 1.25));
        }
        if f.sketches {
            lines.push((&self.sketches, 1.3));
        }
        for (b, width) in lines {
            gl.uniform_1_f32(
                self.thick.loc(gl, "u_w").as_ref(),
                width * f.edge_scale * f.ppp,
            );
            gl.bind_vertex_array(Some(b.vao));
            gl.draw_arrays(glow::TRIANGLES, 0, b.count);
        }
        gl.depth_mask(true);
        gl.disable(glow::BLEND);

        gl.bind_vertex_array(None);
        gl.use_program(None);
        gl.disable(glow::DEPTH_TEST);
        self.mark(gl, "lines");
        if offscreen {
            self.scene_end(gl, vp_window, target, f.fxaa);
        }
        self.mark(gl, "resolve + copy");
    }

    unsafe fn scene_buffers(gl: &glow::Context, w: i32, h: i32) -> Option<SceneTarget> {
        let samples = gl.get_parameter_i32(glow::MAX_SAMPLES).clamp(0, 4);
        let color_rb = gl.create_renderbuffer().ok()?;
        gl.bind_renderbuffer(glow::RENDERBUFFER, Some(color_rb));
        gl.renderbuffer_storage_multisample(glow::RENDERBUFFER, samples, glow::RGBA8, w, h);
        let depth_rb = gl.create_renderbuffer().ok()?;
        gl.bind_renderbuffer(glow::RENDERBUFFER, Some(depth_rb));
        gl.renderbuffer_storage_multisample(
            glow::RENDERBUFFER,
            samples,
            glow::DEPTH_COMPONENT24,
            w,
            h,
        );
        gl.bind_renderbuffer(glow::RENDERBUFFER, None);
        let msaa_fbo = gl.create_framebuffer().ok()?;
        gl.bind_framebuffer(glow::FRAMEBUFFER, Some(msaa_fbo));
        gl.framebuffer_renderbuffer(
            glow::FRAMEBUFFER,
            glow::COLOR_ATTACHMENT0,
            glow::RENDERBUFFER,
            Some(color_rb),
        );
        gl.framebuffer_renderbuffer(
            glow::FRAMEBUFFER,
            glow::DEPTH_ATTACHMENT,
            glow::RENDERBUFFER,
            Some(depth_rb),
        );
        let ok1 = gl.check_framebuffer_status(glow::FRAMEBUFFER) == glow::FRAMEBUFFER_COMPLETE;
        let (resolve_tex, resolve_fbo) = Self::ao_texture(gl, w, h, false, true)?;
        gl.bind_framebuffer(glow::FRAMEBUFFER, None);
        ok1.then_some(SceneTarget {
            size: (w, h),
            msaa_fbo,
            color_rb,
            depth_rb,
            resolve_fbo,
            resolve_tex,
        })
    }

    /// Starts drawing the scene into its own buffer (with the gradient background). `false`
    /// means the window itself is drawn into, as before.
    unsafe fn scene_begin(&mut self, gl: &glow::Context, vp: [i32; 4]) -> bool {
        if self.scene_broken || self.bg_prog.is_none() || self.copy_prog.is_none() {
            return false;
        }
        let (w, h) = (vp[2].max(1), vp[3].max(1));
        if self.scene.as_ref().is_none_or(|t| t.size != (w, h)) {
            if let Some(old) = self.scene.take() {
                gl.delete_framebuffer(old.msaa_fbo);
                gl.delete_framebuffer(old.resolve_fbo);
                gl.delete_renderbuffer(old.color_rb);
                gl.delete_renderbuffer(old.depth_rb);
                gl.delete_texture(old.resolve_tex);
            }
            self.scene = Self::scene_buffers(gl, w, h);
        }
        let (Some(t), Some(bg)) = (&self.scene, &self.bg_prog) else {
            self.scene_broken = true;
            return false;
        };
        gl.bind_framebuffer(glow::FRAMEBUFFER, Some(t.msaa_fbo));
        gl.viewport(0, 0, w, h);
        gl.disable(glow::SCISSOR_TEST);
        gl.disable(glow::BLEND);
        gl.disable(glow::DEPTH_TEST);
        gl.depth_mask(true);
        gl.clear_color(0.0, 0.0, 0.0, 1.0);
        gl.clear(glow::COLOR_BUFFER_BIT | glow::DEPTH_BUFFER_BIT);
        gl.use_program(Some(bg.prog));
        gl.uniform_2_f32(bg.loc(gl, "u_px").as_ref(), w as f32, h as f32);
        gl.bind_vertex_array(Some(self.empty_vao));
        gl.draw_arrays(glow::TRIANGLES, 0, 3);
        gl.bind_vertex_array(None);
        true
    }

    /// Resolves the scene buffer and copies it into the window at the callback's rectangle.
    unsafe fn scene_end(
        &self,
        gl: &glow::Context,
        vp: [i32; 4],
        target: Option<glow::Framebuffer>,
        fxaa: bool,
    ) {
        let (Some(t), Some(copy)) = (&self.scene, &self.copy_prog) else {
            return;
        };
        let (w, h) = t.size;
        gl.bind_framebuffer(glow::READ_FRAMEBUFFER, Some(t.msaa_fbo));
        gl.bind_framebuffer(glow::DRAW_FRAMEBUFFER, Some(t.resolve_fbo));
        gl.blit_framebuffer(
            0,
            0,
            w,
            h,
            0,
            0,
            w,
            h,
            glow::COLOR_BUFFER_BIT,
            glow::NEAREST,
        );
        gl.bind_framebuffer(glow::FRAMEBUFFER, target);
        gl.viewport(vp[0], vp[1], vp[2], vp[3]);
        gl.enable(glow::SCISSOR_TEST);
        gl.disable(glow::DEPTH_TEST);
        gl.disable(glow::BLEND);
        gl.use_program(Some(copy.prog));
        gl.active_texture(glow::TEXTURE0);
        gl.bind_texture(glow::TEXTURE_2D, Some(t.resolve_tex));
        gl.uniform_1_i32(copy.loc(gl, "u_img").as_ref(), 0);
        gl.uniform_1_f32(
            copy.loc(gl, "u_fxaa").as_ref(),
            if fxaa { 1.0 } else { 0.0 },
        );
        gl.uniform_4_f32(
            copy.loc(gl, "u_vp").as_ref(),
            vp[0] as f32,
            vp[1] as f32,
            vp[2].max(1) as f32,
            vp[3].max(1) as f32,
        );
        gl.bind_vertex_array(Some(self.empty_vao));
        gl.draw_arrays(glow::TRIANGLES, 0, 3);
        gl.bind_vertex_array(None);
        gl.bind_texture(glow::TEXTURE_2D, None);
        gl.use_program(None);
    }

    /// Brings the panorama of `env` to the GPU without stalling the UI. Prefiltering takes
    /// about a second, so it runs on another thread; until it is done the environment shown
    /// before stays (or the analytic studio, if there was none), and a repaint is requested
    /// when it finishes. Finished environments are kept, so switching back is instant.
    /// Returns the texture and environment to draw with, if any.
    unsafe fn ensure_environment(
        &mut self,
        gl: &glow::Context,
        env: crate::environment::Environment,
    ) -> Option<(glow::Texture, crate::environment::Environment)> {
        use std::sync::mpsc::TryRecvError;
        // Collect a finished job (whichever environment it was for).
        if let Some((done, rx)) = &self.env_job {
            let done = *done;
            match rx.try_recv() {
                Ok(levels) => {
                    self.env_job = None;
                    if let Some(tex) = self.upload_environment(gl, &levels) {
                        self.envs.push((tex, done));
                        if done == env {
                            self.env_shown = Some(self.envs.len() - 1);
                        }
                    } else {
                        self.env_failed.push(done);
                    }
                }
                Err(TryRecvError::Empty) => {}
                Err(TryRecvError::Disconnected) => {
                    self.env_job = None;
                    self.env_failed.push(done);
                }
            }
        }
        if !env.has_image() {
            return None;
        }
        if let Some(i) = self.envs.iter().position(|(_, e)| *e == env) {
            self.env_shown = Some(i);
        } else if self.env_job.is_none() && !self.env_failed.contains(&env) {
            let (tx, rx) = std::sync::mpsc::channel();
            let repaint = self.repaint.clone();
            std::thread::spawn(move || {
                // One level per roughness, each the environment averaged over that roughness's
                // GGX lobe (see `prefilter`); the shader blends two of them.
                let levels =
                    crate::prefilter::prefilter(&crate::environment::panorama(env), ENV_LEVELS);
                let _ = tx.send(levels);
                if let Some(r) = repaint {
                    r.0();
                }
            });
            self.env_job = Some((env, rx));
        }
        self.env_shown.map(|i| self.envs[i])
    }

    unsafe fn upload_environment(
        &mut self,
        gl: &glow::Context,
        levels: &[crate::environment::Panorama],
    ) -> Option<glow::Texture> {
        let tex = gl.create_texture().ok()?;
        gl.bind_texture(glow::TEXTURE_2D, Some(tex));
        for (k, l) in levels.iter().enumerate() {
            gl.tex_image_2d(
                glow::TEXTURE_2D,
                k as i32,
                glow::RGBA16F as i32,
                l.width as i32,
                l.height as i32,
                0,
                glow::RGBA,
                glow::FLOAT,
                glow::PixelUnpackData::Slice(Some(bytes(&l.rgba))),
            );
        }
        gl.tex_parameter_i32(glow::TEXTURE_2D, glow::TEXTURE_BASE_LEVEL, 0);
        gl.tex_parameter_i32(
            glow::TEXTURE_2D,
            glow::TEXTURE_MAX_LEVEL,
            levels.len() as i32 - 1,
        );
        self.env_lods = (levels.len() - 1) as f32;
        for (p, v) in [
            (glow::TEXTURE_MIN_FILTER, glow::LINEAR_MIPMAP_LINEAR),
            (glow::TEXTURE_MAG_FILTER, glow::LINEAR),
            (glow::TEXTURE_WRAP_S, glow::REPEAT),
            (glow::TEXTURE_WRAP_T, glow::CLAMP_TO_EDGE),
        ] {
            gl.tex_parameter_i32(glow::TEXTURE_2D, p, v as i32);
        }
        gl.bind_texture(glow::TEXTURE_2D, None);
        Some(tex)
    }

    unsafe fn refl_target(gl: &glow::Context, w: i32, h: i32) -> Option<ReflTarget> {
        let tex = gl.create_texture().ok()?;
        gl.bind_texture(glow::TEXTURE_2D, Some(tex));
        gl.tex_image_2d(
            glow::TEXTURE_2D,
            0,
            glow::RGBA16F as i32,
            w,
            h,
            0,
            glow::RGBA,
            glow::FLOAT,
            glow::PixelUnpackData::Slice(None),
        );
        for (p, v) in [
            (glow::TEXTURE_MIN_FILTER, glow::LINEAR),
            (glow::TEXTURE_MAG_FILTER, glow::LINEAR),
            (glow::TEXTURE_WRAP_S, glow::CLAMP_TO_EDGE),
            (glow::TEXTURE_WRAP_T, glow::CLAMP_TO_EDGE),
        ] {
            gl.tex_parameter_i32(glow::TEXTURE_2D, p, v as i32);
        }
        let depth_rb = gl.create_renderbuffer().ok()?;
        gl.bind_renderbuffer(glow::RENDERBUFFER, Some(depth_rb));
        gl.renderbuffer_storage(glow::RENDERBUFFER, glow::DEPTH_COMPONENT24, w, h);
        gl.bind_renderbuffer(glow::RENDERBUFFER, None);
        let fbo = gl.create_framebuffer().ok()?;
        gl.bind_framebuffer(glow::FRAMEBUFFER, Some(fbo));
        gl.framebuffer_texture_2d(
            glow::FRAMEBUFFER,
            glow::COLOR_ATTACHMENT0,
            glow::TEXTURE_2D,
            Some(tex),
            0,
        );
        gl.framebuffer_renderbuffer(
            glow::FRAMEBUFFER,
            glow::DEPTH_ATTACHMENT,
            glow::RENDERBUFFER,
            Some(depth_rb),
        );
        let ok = gl.check_framebuffer_status(glow::FRAMEBUFFER) == glow::FRAMEBUFFER_COMPLETE;
        gl.bind_framebuffer(glow::FRAMEBUFFER, None);
        gl.bind_texture(glow::TEXTURE_2D, None);
        ok.then_some(ReflTarget {
            size: (w, h),
            fbo,
            tex,
            depth_rb,
        })
    }

    /// Points a surface program at the occlusion texture (or tells it there is none).
    unsafe fn bind_ao(&self, gl: &glow::Context, p: &Program, vp: [i32; 4]) {
        gl.uniform_1_f32(
            p.loc(gl, "u_ao_on").as_ref(),
            if self.ao_active { 1.0 } else { 0.0 },
        );
        if let (true, Some(t)) = (self.ao_active, &self.ao) {
            gl.uniform_1_i32(p.loc(gl, "u_ao").as_ref(), 1);
            gl.uniform_4_f32(
                p.loc(gl, "u_vp").as_ref(),
                vp[0] as f32,
                vp[1] as f32,
                vp[2].max(1) as f32,
                vp[3].max(1) as f32,
            );
            gl.active_texture(glow::TEXTURE1);
            gl.bind_texture(glow::TEXTURE_2D, Some(t.blur_tex));
            gl.active_texture(glow::TEXTURE0);
        }
    }

    unsafe fn ao_texture(
        gl: &glow::Context,
        w: i32,
        h: i32,
        depth: bool,
        linear: bool,
    ) -> Option<(glow::Texture, glow::Framebuffer)> {
        let tex = gl.create_texture().ok()?;
        gl.bind_texture(glow::TEXTURE_2D, Some(tex));
        if depth {
            gl.tex_image_2d(
                glow::TEXTURE_2D,
                0,
                glow::DEPTH_COMPONENT24 as i32,
                w,
                h,
                0,
                glow::DEPTH_COMPONENT,
                glow::UNSIGNED_INT,
                glow::PixelUnpackData::Slice(None),
            );
        } else {
            gl.tex_image_2d(
                glow::TEXTURE_2D,
                0,
                glow::RGBA8 as i32,
                w,
                h,
                0,
                glow::RGBA,
                glow::UNSIGNED_BYTE,
                glow::PixelUnpackData::Slice(None),
            );
        }
        let filter = if linear { glow::LINEAR } else { glow::NEAREST };
        for (p, v) in [
            (glow::TEXTURE_MIN_FILTER, filter),
            (glow::TEXTURE_MAG_FILTER, filter),
            (glow::TEXTURE_WRAP_S, glow::CLAMP_TO_EDGE),
            (glow::TEXTURE_WRAP_T, glow::CLAMP_TO_EDGE),
        ] {
            gl.tex_parameter_i32(glow::TEXTURE_2D, p, v as i32);
        }
        let fbo = gl.create_framebuffer().ok()?;
        gl.bind_framebuffer(glow::FRAMEBUFFER, Some(fbo));
        if depth {
            gl.framebuffer_texture_2d(
                glow::FRAMEBUFFER,
                glow::DEPTH_ATTACHMENT,
                glow::TEXTURE_2D,
                Some(tex),
                0,
            );
            gl.draw_buffers(&[glow::NONE]);
            gl.read_buffer(glow::NONE);
        } else {
            gl.framebuffer_texture_2d(
                glow::FRAMEBUFFER,
                glow::COLOR_ATTACHMENT0,
                glow::TEXTURE_2D,
                Some(tex),
                0,
            );
        }
        let ok = gl.check_framebuffer_status(glow::FRAMEBUFFER) == glow::FRAMEBUFFER_COMPLETE;
        gl.bind_framebuffer(glow::FRAMEBUFFER, None);
        ok.then_some((tex, fbo))
    }

    /// Makes (or resizes) the occlusion targets. `None` if the driver cannot.
    unsafe fn ao_targets(gl: &glow::Context, w: i32, h: i32) -> Option<AoTargets> {
        let (depth_tex, depth_fbo) = Self::ao_texture(gl, w, h, true, false)?;
        // The occlusion itself is half the size: it is soft, and this is the most expensive pass.
        let (hw, hh) = ((w + 1) / 2, (h + 1) / 2);
        let (raw_tex, raw_fbo) = Self::ao_texture(gl, hw, hh, false, false)?;
        let (blur_tex, blur_fbo) = Self::ao_texture(gl, hw, hh, false, true)?;
        Some(AoTargets {
            size: (w, h),
            depth_tex,
            depth_fbo,
            raw_tex,
            raw_fbo,
            blur_tex,
            blur_fbo,
        })
    }

    unsafe fn free_ao(gl: &glow::Context, t: &AoTargets) {
        for tex in [t.depth_tex, t.raw_tex, t.blur_tex] {
            gl.delete_texture(tex);
        }
        for fbo in [t.depth_fbo, t.raw_fbo, t.blur_fbo] {
            gl.delete_framebuffer(fbo);
        }
    }

    /// Ambient occlusion: the part's depth from the camera, an occlusion estimate from it, and a
    /// depth-aware blur. Returns whether the blurred result is ready for the surface shaders.
    unsafe fn ao_passes(
        &mut self,
        gl: &glow::Context,
        vp: [i32; 4],
        target: Option<glow::Framebuffer>,
        f: &Frame,
    ) -> bool {
        let (w, h) = (vp[2].max(1), vp[3].max(1));
        if self.ao_prog.is_none() || self.blur_prog.is_none() || self.tris.count == 0 {
            return false;
        }
        if self.ao.as_ref().is_none_or(|t| t.size != (w, h)) {
            if let Some(old) = self.ao.take() {
                Self::free_ao(gl, &old);
            }
            self.ao = Self::ao_targets(gl, w, h);
        }
        let (Some(t), Some(ao_p), Some(blur_p)) = (&self.ao, &self.ao_prog, &self.blur_prog) else {
            self.ao_prog = None; // do not try again every frame
            return false;
        };
        gl.disable(glow::SCISSOR_TEST);
        gl.viewport(0, 0, w, h);

        // 1. The depth of the part from the camera.
        gl.bind_framebuffer(glow::FRAMEBUFFER, Some(t.depth_fbo));
        gl.enable(glow::DEPTH_TEST);
        gl.depth_func(glow::LESS);
        gl.depth_mask(true);
        gl.disable(glow::CULL_FACE);
        gl.clear(glow::DEPTH_BUFFER_BIT);
        gl.use_program(Some(self.shadow_prog.prog));
        gl.uniform_matrix_4_f32_slice(self.shadow_prog.loc(gl, "u_light").as_ref(), false, &f.mvp);
        gl.bind_vertex_array(Some(self.tris.vao));
        gl.draw_arrays(glow::TRIANGLES, 0, self.tris.count);
        if f.real.is_some() && self.floor_quad.count > 0 {
            gl.bind_vertex_array(Some(self.floor_quad.vao));
            gl.draw_arrays(glow::TRIANGLES, 0, self.floor_quad.count);
        }
        gl.disable(glow::DEPTH_TEST);

        // 2. Occlusion from the depth (into half-size images).
        gl.viewport(0, 0, (w + 1) / 2, (h + 1) / 2);
        gl.bind_vertex_array(Some(self.empty_vao));
        gl.bind_framebuffer(glow::FRAMEBUFFER, Some(t.raw_fbo));
        gl.use_program(Some(ao_p.prog));
        gl.active_texture(glow::TEXTURE0);
        gl.bind_texture(glow::TEXTURE_2D, Some(t.depth_tex));
        gl.uniform_1_i32(ao_p.loc(gl, "u_depth").as_ref(), 0);
        gl.uniform_matrix_4_f32_slice(ao_p.loc(gl, "u_inv").as_ref(), false, &f.inv_mvp);
        gl.uniform_2_f32(ao_p.loc(gl, "u_px").as_ref(), w as f32, h as f32);
        gl.uniform_3_f32_slice(ao_p.loc(gl, "u_eye").as_ref(), &f.eye3);
        gl.uniform_1_f32(ao_p.loc(gl, "u_radius").as_ref(), f.ao_radius);
        gl.draw_arrays(glow::TRIANGLES, 0, 3);

        // 3. Blur it, not across depth steps.
        gl.bind_framebuffer(glow::FRAMEBUFFER, Some(t.blur_fbo));
        gl.use_program(Some(blur_p.prog));
        gl.uniform_1_i32(blur_p.loc(gl, "u_depth").as_ref(), 0);
        gl.uniform_1_i32(blur_p.loc(gl, "u_ao_in").as_ref(), 1);
        gl.active_texture(glow::TEXTURE1);
        gl.bind_texture(glow::TEXTURE_2D, Some(t.raw_tex));
        gl.draw_arrays(glow::TRIANGLES, 0, 3);
        gl.bind_texture(glow::TEXTURE_2D, None);
        gl.active_texture(glow::TEXTURE0);
        gl.bind_texture(glow::TEXTURE_2D, None);

        gl.bind_vertex_array(None);
        gl.bind_framebuffer(glow::FRAMEBUFFER, target);
        gl.viewport(vp[0], vp[1], vp[2], vp[3]);
        gl.enable(glow::SCISSOR_TEST);
        true
    }

    /// Renders the part's depth as the light sees it.
    unsafe fn shadow_pass(
        &self,
        gl: &glow::Context,
        vp: [i32; 4],
        target: Option<glow::Framebuffer>,
        r: &Real,
        s: &Shadow,
    ) {
        gl.bind_framebuffer(glow::FRAMEBUFFER, Some(s.fbo));
        gl.viewport(0, 0, SHADOW_SIZE, SHADOW_SIZE);
        gl.disable(glow::SCISSOR_TEST);
        gl.enable(glow::DEPTH_TEST);
        gl.depth_func(glow::LESS);
        gl.depth_mask(true);
        gl.clear(glow::DEPTH_BUFFER_BIT);
        gl.enable(glow::POLYGON_OFFSET_FILL);
        gl.polygon_offset(2.0, 4.0);
        gl.use_program(Some(self.shadow_prog.prog));
        gl.uniform_matrix_4_f32_slice(
            self.shadow_prog.loc(gl, "u_light").as_ref(),
            false,
            &r.light,
        );
        gl.bind_vertex_array(Some(self.tris.vao));
        gl.draw_arrays(glow::TRIANGLES, 0, self.tris.count);
        gl.disable(glow::POLYGON_OFFSET_FILL);
        gl.bind_framebuffer(glow::FRAMEBUFFER, target);
        gl.viewport(vp[0], vp[1], vp[2], vp[3]);
        gl.enable(glow::SCISSOR_TEST);
    }

    unsafe fn draw_real(
        &mut self,
        gl: &glow::Context,
        f: &Frame,
        r: &Real,
        shadows: bool,
        vp: [i32; 4],
    ) {
        let tex = self.shadow.as_ref().map(|s| s.tex);
        let set_common = |p: &Program| {
            gl.use_program(Some(p.prog));
            gl.uniform_matrix_4_f32_slice(p.loc(gl, "u_mvp").as_ref(), false, &f.mvp);
            gl.uniform_matrix_4_f32_slice(p.loc(gl, "u_light").as_ref(), false, &r.light);
            gl.uniform_1_f32(p.loc(gl, "u_nbias").as_ref(), r.normal_bias);
            gl.uniform_1_f32(
                p.loc(gl, "u_shadow_on").as_ref(),
                if shadows { 1.0 } else { 0.0 },
            );
            gl.uniform_1_f32(p.loc(gl, "u_texel").as_ref(), 1.0 / SHADOW_SIZE as f32);
            gl.uniform_1_f32(p.loc(gl, "u_dbias").as_ref(), 0.0015);
            gl.uniform_1_f32(p.loc(gl, "u_light_size").as_ref(), 0.30);
            gl.uniform_1_i32(p.loc(gl, "u_shadow").as_ref(), 0);
        };
        gl.active_texture(glow::TEXTURE0);
        gl.bind_texture(glow::TEXTURE_2D, tex);

        // The environment the shiny surfaces reflect.
        let shown = self.ensure_environment(gl, f.environment);
        let env_on = shown.is_some();
        if let Some((t, _)) = shown {
            gl.active_texture(glow::TEXTURE2);
            gl.bind_texture(glow::TEXTURE_2D, Some(t));
            gl.active_texture(glow::TEXTURE0);
        }

        let p = &self.real;
        set_common(p);
        gl.uniform_1_i32(p.loc(gl, "u_envmap").as_ref(), 2);
        gl.uniform_1_f32(
            p.loc(gl, "u_env_on").as_ref(),
            if env_on { 1.0 } else { 0.0 },
        );
        gl.uniform_1_f32(p.loc(gl, "u_env_lods").as_ref(), self.env_lods);
        let gain = if matches!(
            shown.map(|s| s.1).unwrap_or_default(),
            crate::environment::Environment::PhotoStudio
                | crate::environment::Environment::BrownStudio
        ) {
            2.1
        } else {
            1.0
        };
        gl.uniform_1_f32(p.loc(gl, "u_env_gain").as_ref(), gain);
        self.bind_ao(gl, p, vp);
        gl.uniform_1_f32(p.loc(gl, "u_mirror").as_ref(), 0.0);
        gl.uniform_3_f32_slice(p.loc(gl, "u_fill").as_ref(), &f.rig[0]);
        gl.uniform_3_f32_slice(p.loc(gl, "u_rim").as_ref(), &f.rig[1]);
        gl.uniform_3_f32_slice(p.loc(gl, "u_head").as_ref(), &f.rig[2]);
        gl.uniform_3_f32_slice(p.loc(gl, "u_eye").as_ref(), &r.eye);
        gl.uniform_3_f32_slice(p.loc(gl, "u_key").as_ref(), &r.key);
        gl.uniform_3_f32_slice(p.loc(gl, "u_albedo").as_ref(), &r.albedo);
        gl.uniform_1_f32(p.loc(gl, "u_metal").as_ref(), r.metallic);
        gl.uniform_1_f32(p.loc(gl, "u_rough").as_ref(), r.roughness);
        gl.uniform_1_f32(p.loc(gl, "u_coat").as_ref(), r.clearcoat);
        gl.uniform_1_f32(p.loc(gl, "u_sel").as_ref(), f.selected);
        gl.uniform_1_f32(p.loc(gl, "u_hov").as_ref(), f.hovered);
        gl.bind_vertex_array(Some(self.tris.vao));
        gl.draw_arrays(glow::TRIANGLES, 0, self.tris.count);

        self.mark(gl, "real surfaces");
        // Seen from below the ground there is nothing to reflect in (and no shadow to catch).
        let above = f.eye3[2] > r.floor_z;

        // The part's reflection in the ground: the same shader on the part mirrored in the ground
        // plane, drawn into a texture of its own (faint, fading with height above the ground) for
        // the floor to blur and lay over the scene. It is drawn twice: first only its depth, so
        // that just the surfaces nearest the viewer are left (otherwise every hidden face of the
        // mirror image shows through, like an X-ray), then its colour.
        let mut refl_on = false;
        if f.reflection && above {
            // Half size: it is blurred afterwards anyway.
            let (w, h) = ((vp[2].max(1) + 1) / 2, (vp[3].max(1) + 1) / 2);
            if self.refl.as_ref().is_none_or(|t| t.size != (w, h)) {
                if let Some(old) = self.refl.take() {
                    gl.delete_framebuffer(old.fbo);
                    gl.delete_texture(old.tex);
                    gl.delete_renderbuffer(old.depth_rb);
                }
                self.refl = Self::refl_target(gl, w, h);
            }
            if let Some(t) = &self.refl {
                let p = &self.real;
                gl.use_program(Some(p.prog));
                gl.bind_framebuffer(glow::FRAMEBUFFER, Some(t.fbo));
                gl.viewport(0, 0, w, h);
                gl.disable(glow::SCISSOR_TEST);
                gl.clear_color(0.0, 0.0, 0.0, 0.0);
                gl.depth_mask(true);
                gl.clear(glow::COLOR_BUFFER_BIT | glow::DEPTH_BUFFER_BIT);
                gl.uniform_1_f32(p.loc(gl, "u_mirror").as_ref(), 1.0);
                gl.uniform_1_f32(p.loc(gl, "u_floor_z").as_ref(), r.floor_z);
                gl.uniform_1_f32(p.loc(gl, "u_reflect").as_ref(), REFLECTION);
                gl.uniform_1_f32(p.loc(gl, "u_rad").as_ref(), r.radius);
                gl.uniform_1_f32(p.loc(gl, "u_ao_on").as_ref(), 0.0);
                gl.bind_vertex_array(Some(self.tris.vao));
                gl.color_mask(false, false, false, false);
                gl.draw_arrays(glow::TRIANGLES, 0, self.tris.count);
                gl.color_mask(true, true, true, true);
                gl.depth_mask(false);
                gl.enable(glow::BLEND);
                gl.blend_func(glow::ONE, glow::ONE_MINUS_SRC_ALPHA);
                gl.draw_arrays(glow::TRIANGLES, 0, self.tris.count);
                gl.depth_mask(true);
                gl.disable(glow::BLEND);
                gl.uniform_1_f32(p.loc(gl, "u_mirror").as_ref(), 0.0);
                self.bind_ao(gl, p, vp);
                gl.bind_framebuffer(glow::FRAMEBUFFER, self.cur_fbo);
                gl.viewport(vp[0], vp[1], vp[2], vp[3]);
                refl_on = true;
            }
        }

        self.mark(gl, "mirror image");
        // The ground: depth-tested against the part, never writing depth.
        let p = &self.floor;
        set_common(p);
        self.bind_ao(gl, p, vp);
        gl.uniform_4_f32(
            p.loc(gl, "u_vp").as_ref(),
            vp[0] as f32,
            vp[1] as f32,
            vp[2].max(1) as f32,
            vp[3].max(1) as f32,
        );
        gl.uniform_1_f32(
            p.loc(gl, "u_refl_on").as_ref(),
            if refl_on { 1.0 } else { 0.0 },
        );
        gl.uniform_1_f32(p.loc(gl, "u_reflect").as_ref(), REFLECTION);
        if let (true, Some(t)) = (refl_on, &self.refl) {
            gl.uniform_1_i32(p.loc(gl, "u_refl").as_ref(), 3);
            gl.active_texture(glow::TEXTURE3);
            gl.bind_texture(glow::TEXTURE_2D, Some(t.tex));
            gl.active_texture(glow::TEXTURE0);
        }
        gl.uniform_3_f32_slice(p.loc(gl, "u_center").as_ref(), &r.center);
        gl.uniform_1_f32(p.loc(gl, "u_radius").as_ref(), r.radius);
        // (The shadow comes out premultiplied: its colour is already scaled by its alpha.)
        gl.enable(glow::BLEND);
        gl.blend_func(glow::ONE, glow::ONE_MINUS_SRC_ALPHA);
        gl.depth_mask(false);
        if above {
            gl.bind_vertex_array(Some(self.floor_quad.vao));
            gl.draw_arrays(glow::TRIANGLES, 0, self.floor_quad.count);
        }
        gl.depth_mask(true);
        gl.disable(glow::BLEND);
        gl.bind_texture(glow::TEXTURE_2D, None);
        self.mark(gl, "floor");
    }
}

/// Wakes the UI thread (egui's `request_repaint`), callable from any thread.
#[derive(Clone)]
struct Repaint(Arc<dyn Fn() + Send + Sync>);

/// GL resources shared between the UI thread and the paint callback.
#[derive(Default)]
pub struct GlShared {
    gpu: Mutex<Option<Gpu>>,
    repaint: Mutex<Option<Repaint>>,
    /// An environment is being prefiltered in the background.
    env_pending: AtomicBool,
    /// Frame timing for `--bench`: the GPU is waited for (glFinish) around each frame.
    bench: AtomicBool,
    bench_ms: Mutex<Vec<f32>>,
    bench_passes: Mutex<Vec<(&'static str, f32)>>,
    renderer: Mutex<String>,
    error: Mutex<Option<String>>,
    failed: AtomicBool,
}

impl GlShared {
    /// Lets background work (environment prefiltering) wake the UI when it is done. Only the
    /// first call has an effect.
    pub fn set_repaint(&self, wake: impl Fn() + Send + Sync + 'static) {
        let mut r = self.repaint.lock().unwrap_or_else(|e| e.into_inner());
        if r.is_none() {
            *r = Some(Repaint(Arc::new(wake)));
        }
    }

    /// Starts timing every frame the GPU draws (`--bench`).
    pub fn start_bench(&self) {
        self.bench.store(true, Ordering::Relaxed);
    }

    /// Frames timed so far.
    pub fn bench_frames(&self) -> usize {
        self.bench_ms
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .len()
    }

    /// A summary of the timed frames (the first few, which include setup, are left out).
    pub fn bench_report(&self) -> String {
        let all = self
            .bench_ms
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .clone();
        let mut v: Vec<f32> = all.iter().skip(5).copied().collect();
        if v.is_empty() {
            return "no frames were timed".into();
        }
        v.sort_by(|a, b| a.total_cmp(b));
        let mean = v.iter().sum::<f32>() / v.len() as f32;
        let pct = |p: f32| v[((v.len() - 1) as f32 * p).round() as usize];
        let passes = self
            .bench_passes
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .iter()
            .filter(|(n, _)| *n != "(between frames)")
            .map(|(n, ms)| format!("  {n}: {:.2} ms per frame", ms / all.len() as f32))
            .collect::<Vec<_>>()
            .join("\n");
        format!(
            "{passes}\nGPU: {}\nframes timed: {}\nframe time (ms, CPU + GPU, waited for): mean {:.2}  median {:.2}  p95 {:.2}  max {:.2}  ({:.0} fps at the mean)",
            self.renderer.lock().unwrap_or_else(|e| e.into_inner()),
            v.len(),
            mean,
            pct(0.5),
            pct(0.95),
            v[v.len() - 1],
            1000.0 / mean.max(1e-6)
        )
    }

    /// Is an environment still being prepared? (A screenshot waits for it.)
    pub fn environment_pending(&self) -> bool {
        self.env_pending.load(Ordering::Relaxed)
    }

    pub fn background() -> [f32; 3] {
        BACKGROUND
    }

    /// Called from the paint callback with the live GL context. `vp` is the callback's viewport in
    /// framebuffer pixels (left, from-bottom, width, height) and `target` the bound framebuffer.
    pub fn paint(
        &self,
        gl: &glow::Context,
        vp: [i32; 4],
        target: Option<glow::Framebuffer>,
        frame: &Frame,
    ) {
        if self.failed.load(Ordering::Relaxed) {
            return;
        }
        let mut guard = self.gpu.lock().unwrap_or_else(|e| e.into_inner());
        if guard.is_none() {
            match unsafe { Gpu::new(gl) } {
                Ok(g) => *guard = Some(g),
                Err(e) => {
                    *self.error.lock().unwrap_or_else(|e| e.into_inner()) = Some(e);
                    self.failed.store(true, Ordering::Relaxed);
                    return;
                }
            }
        }
        if let Some(g) = guard.as_mut() {
            g.repaint = self
                .repaint
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .clone();
            if self.bench.load(Ordering::Relaxed) {
                use glow::HasContext;
                let renderer = unsafe { gl.get_parameter_string(glow::RENDERER) };
                *self.renderer.lock().unwrap_or_else(|e| e.into_inner()) = renderer;
                unsafe { gl.finish() };
                if g.prof.is_none() {
                    g.prof = Some(Profile {
                        last: std::time::Instant::now(),
                        totals: Vec::new(),
                    });
                }
                if let Some(p) = &mut g.prof {
                    p.last = std::time::Instant::now();
                }
                let t = std::time::Instant::now();
                unsafe { g.draw(gl, vp, target, frame) };
                unsafe { gl.finish() };
                if let Some(p) = &g.prof {
                    *self.bench_passes.lock().unwrap_or_else(|e| e.into_inner()) = p.totals.clone();
                }
                self.bench_ms
                    .lock()
                    .unwrap_or_else(|e| e.into_inner())
                    .push(t.elapsed().as_secs_f32() * 1000.0);
            } else {
                unsafe { g.draw(gl, vp, target, frame) };
            }
            self.env_pending
                .store(g.env_job.is_some(), Ordering::Relaxed);
        }
    }

    /// A setup failure, once. After this the caller should use the software path.
    pub fn take_error(&self) -> Option<String> {
        self.error.lock().unwrap_or_else(|e| e.into_inner()).take()
    }

    pub fn failed(&self) -> bool {
        self.failed.load(Ordering::Relaxed)
    }
}
