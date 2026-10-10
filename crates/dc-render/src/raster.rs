//! A deterministic software rasterizer: z-buffered triangles with smooth per-face shading, depth
//! tested lines for edges, sketches and the grid, optional supersampling, and a flat-colour
//! "face id" mode that lets a viewer (or an agent) map pixels back to named faces.

use crate::camera::{Camera, Projected};
use crate::png;
use crate::scene::{Body, Scene};
use dc_math::Vec3;
use std::collections::BTreeSet;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Image {
    pub width: u32,
    pub height: u32,
    /// `width * height * 3` bytes, row-major, top row first.
    pub rgb: Vec<u8>,
}

impl Image {
    pub fn pixel(&self, x: u32, y: u32) -> [u8; 3] {
        let i = (y as usize * self.width as usize + x as usize) * 3;
        [self.rgb[i], self.rgb[i + 1], self.rgb[i + 2]]
    }

    pub fn to_png(&self) -> Vec<u8> {
        png::encode_rgb(self.width, self.height, &self.rgb)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RenderMode {
    /// What a person sees: lit surfaces, edges, grid.
    Shaded,
    /// Every face a distinct flat colour, no lighting or lines, with a legend. For mapping pixels
    /// to faces.
    FaceIds,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FaceRef {
    pub body: String,
    pub face: String,
}

/// The plane being sketched on: its origin is always marked (two small red arrows, as in
/// SolidWorks); its grid is drawn only when `grid` is set.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct GridPlane {
    pub origin: Vec3,
    pub x: Vec3,
    pub y: Vec3,
    pub grid: bool,
}

impl GridPlane {
    /// The sketch origin marker as line segments in the plane's 2D frame: one arrow along each
    /// axis, `len` long.
    pub fn origin_marker(&self, len: f64) -> Vec<[Vec3; 2]> {
        let at = |u: f64, v: f64| self.origin + self.x * u + self.y * v;
        let (b, w) = (len * 0.22, len * 0.09);
        vec![
            [at(0.0, 0.0), at(len, 0.0)],
            [at(len, 0.0), at(len - b, w)],
            [at(len, 0.0), at(len - b, -w)],
            [at(0.0, 0.0), at(0.0, len)],
            [at(0.0, len), at(w, len - b)],
            [at(0.0, len), at(-w, len - b)],
        ]
    }
}

#[derive(Clone, Debug)]
pub struct RenderOptions {
    pub width: u32,
    pub height: u32,
    pub camera: Camera,
    pub mode: RenderMode,
    pub edges: bool,
    /// The world grid (in the XY plane) and the world axes.
    pub grid: bool,
    pub axes: bool,
    /// The grid and axes of a plane, normally the sketch being edited.
    pub local_grid: Option<GridPlane>,
    pub sketches: bool,
    /// Darken concave corners and pockets (screen-space ambient occlusion on the depth buffer).
    /// Off by default so existing images stay as they are.
    pub ambient_occlusion: bool,
    pub background: [u8; 3],
    /// 1 = none, 2 = 2x2 samples per pixel. Ignored (forced to 1) in face-id mode.
    pub supersample: u32,
    pub selected: Option<FaceRef>,
    pub hovered: Option<FaceRef>,
}

impl RenderOptions {
    pub fn new(width: u32, height: u32, camera: Camera) -> Self {
        RenderOptions {
            width,
            height,
            camera,
            mode: RenderMode::Shaded,
            edges: true,
            grid: true,
            axes: true,
            local_grid: None,
            sketches: true,
            ambient_occlusion: false,
            background: [236, 238, 242],
            supersample: 1,
            selected: None,
            hovered: None,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LegendEntry {
    pub color: [u8; 3],
    pub body: String,
    pub face: String,
}

pub struct RenderOutput {
    pub image: Image,
    /// Face-id mode only: which colour is which face (every face of every body).
    pub legend: Vec<LegendEntry>,
}

struct Target {
    w: usize,
    h: usize,
    rgb: Vec<u8>,
    /// Larger = closer; `-inf` = nothing drawn.
    key: Vec<f64>,
}

#[derive(Clone, Copy)]
struct Vtx {
    x: f64,
    y: f64,
    key: f64,
    shade: f64,
}

impl Target {
    fn new(w: usize, h: usize, bg: [u8; 3]) -> Target {
        let mut rgb = Vec::with_capacity(w * h * 3);
        for _ in 0..w * h {
            rgb.extend_from_slice(&bg);
        }
        Target {
            w,
            h,
            rgb,
            key: vec![f64::NEG_INFINITY; w * h],
        }
    }

    fn put(&mut self, x: usize, y: usize, key: f64, c: [u8; 3]) {
        let i = y * self.w + x;
        if key > self.key[i] {
            self.key[i] = key;
            self.rgb[i * 3..i * 3 + 3].copy_from_slice(&c);
        }
    }

    /// Like `put` but writes only if strictly in front, with a bias, and never updates depth
    /// (lines do not occlude).
    fn put_line(&mut self, x: usize, y: usize, key: f64, bias: f64, c: [u8; 3]) {
        let i = y * self.w + x;
        if key + bias >= self.key[i] {
            self.rgb[i * 3..i * 3 + 3].copy_from_slice(&c);
        }
    }

    fn fill(&mut self, a: Vtx, b: Vtx, c: Vtx, color: impl Fn(f64) -> [u8; 3]) {
        let area = (b.x - a.x) * (c.y - a.y) - (b.y - a.y) * (c.x - a.x);
        if area.abs() < 1e-12 {
            return;
        }
        let (min_x, max_x) = (a.x.min(b.x).min(c.x), a.x.max(b.x).max(c.x));
        let (min_y, max_y) = (a.y.min(b.y).min(c.y), a.y.max(b.y).max(c.y));
        let x0 = (min_x.floor().max(0.0)) as usize;
        let y0 = (min_y.floor().max(0.0)) as usize;
        let x1 = (max_x.ceil().min(self.w as f64 - 1.0)).max(-1.0);
        let y1 = (max_y.ceil().min(self.h as f64 - 1.0)).max(-1.0);
        if x1 < 0.0 || y1 < 0.0 {
            return;
        }
        let (x1, y1) = (x1 as usize, y1 as usize);
        let inv = 1.0 / area;
        for py in y0..=y1 {
            for px in x0..=x1 {
                let (sx, sy) = (px as f64 + 0.5, py as f64 + 0.5);
                let w0 = ((b.x - sx) * (c.y - sy) - (b.y - sy) * (c.x - sx)) * inv;
                let w1 = ((c.x - sx) * (a.y - sy) - (c.y - sy) * (a.x - sx)) * inv;
                let w2 = 1.0 - w0 - w1;
                const E: f64 = -1e-9;
                if w0 >= E && w1 >= E && w2 >= E {
                    let key = w0 * a.key + w1 * b.key + w2 * c.key;
                    let shade = w0 * a.shade + w1 * b.shade + w2 * c.shade;
                    self.put(px, py, key, color(shade));
                }
            }
        }
    }

    fn line(
        &mut self,
        a: Projected,
        b: Projected,
        color: [u8; 3],
        bias_rel: f64,
        thickness: usize,
    ) {
        // Clip the segment to the viewport first (Liang-Barsky), so long lines that run far off
        // screen are drawn correctly and cost only their visible length.
        let (dx, dy) = (b.x - a.x, b.y - a.y);
        let (mut t0, mut t1) = (0.0f64, 1.0f64);
        for (p, q) in [
            (-dx, a.x),
            (dx, self.w as f64 - 1.0 - a.x),
            (-dy, a.y),
            (dy, self.h as f64 - 1.0 - a.y),
        ] {
            if p == 0.0 {
                if q < 0.0 {
                    return;
                }
            } else {
                let r = q / p;
                if p < 0.0 {
                    t0 = t0.max(r);
                } else {
                    t1 = t1.min(r);
                }
            }
        }
        if t0 > t1 {
            return;
        }
        let at = |t: f64| (a.x + dx * t, a.y + dy * t, a.key + (b.key - a.key) * t);
        let ((x0, y0, k0), (x1, y1, k1)) = (at(t0), at(t1));
        let steps = (x1 - x0).abs().max((y1 - y0).abs()).ceil().max(1.0) as usize;
        for s in 0..=steps {
            let t = s as f64 / steps as f64;
            let (x, y, key) = (x0 + (x1 - x0) * t, y0 + (y1 - y0) * t, k0 + (k1 - k0) * t);
            let bias = bias_rel * key.abs() + 1e-12;
            for ox in 0..thickness {
                for oy in 0..thickness {
                    let (px, py) = (x.floor() as i64 + ox as i64, y.floor() as i64 + oy as i64);
                    if px >= 0 && py >= 0 && (px as usize) < self.w && (py as usize) < self.h {
                        self.put_line(px as usize, py as usize, key, bias, color);
                    }
                }
            }
        }
    }
}

fn mix(a: [u8; 3], b: [u8; 3], t: f64) -> [u8; 3] {
    let m = |x: u8, y: u8| {
        (f64::from(x) * (1.0 - t) + f64::from(y) * t)
            .round()
            .clamp(0.0, 255.0) as u8
    };
    [m(a[0], b[0]), m(a[1], b[1]), m(a[2], b[2])]
}

fn hsv(h: f64, s: f64, v: f64) -> [u8; 3] {
    let h6 = (h - h.floor()) * 6.0;
    let (i, f) = (h6.floor() as i32, h6 - h6.floor());
    let (p, q, t) = (v * (1.0 - s), v * (1.0 - s * f), v * (1.0 - s * (1.0 - f)));
    let (r, g, b) = match i {
        0 => (v, t, p),
        1 => (q, v, p),
        2 => (p, v, t),
        3 => (p, q, v),
        4 => (t, p, v),
        _ => (v, p, q),
    };
    [
        (r * 255.0).round() as u8,
        (g * 255.0).round() as u8,
        (b * 255.0).round() as u8,
    ]
}

/// Distinct colours, deterministically, for face-id mode. Never black or the background.
fn id_colors(n: usize, avoid: [u8; 3]) -> Vec<[u8; 3]> {
    let mut used: BTreeSet<[u8; 3]> = BTreeSet::new();
    used.insert([0, 0, 0]);
    used.insert(avoid);
    let mut out = Vec::with_capacity(n);
    for i in 0..n {
        let f = |k: f64| (i as f64 * k).fract();
        let mut c = hsv(
            f(0.618_033_988_75),
            0.55 + 0.35 * f(0.37),
            0.72 + 0.23 * f(0.71),
        );
        while !used.insert(c) {
            c[2] = c[2].wrapping_add(1);
        }
        out.push(c);
    }
    out
}

/// Intensity from a camera-space normal: a fixed key light up and to the left, plus ambient.
fn lambert(n: (f64, f64, f64)) -> f64 {
    const L: (f64, f64, f64) = (-0.309_5, 0.442_1, -0.842_1); // toward the light, camera space
    let d = n.0 * L.0 + n.1 * L.1 + n.2 * L.2;
    0.32 + 0.68 * d.max(0.0)
}

/// Grid spacing that keeps roughly six cells across the view at this camera distance.
pub fn grid_step(distance: f64) -> f64 {
    let target = distance / 6.0;
    let mut s = 1.0;
    while s * 5.0 < target {
        s *= 10.0;
    }
    while s > target * 2.0 && s > 1e-6 {
        s /= 10.0;
    }
    for m in [1.0, 2.0, 5.0] {
        if s * m >= target * 0.7 {
            return s * m;
        }
    }
    s * 10.0
}

struct Ctx<'a> {
    cam: &'a Camera,
    w: f64,
    h: f64,
}

impl Ctx<'_> {
    /// Draws a world-space segment, clipped to the near plane.
    fn segment(
        &self,
        t: &mut Target,
        p: Vec3,
        q: Vec3,
        color: [u8; 3],
        bias: f64,
        thickness: usize,
    ) {
        let (mut a, mut b) = (self.cam.camera_space(p), self.cam.camera_space(q));
        if self.cam.projection == crate::camera::ProjectionKind::Perspective {
            let near = self.cam.near();
            if a.2 < near && b.2 < near {
                return;
            }
            if a.2 < near || b.2 < near {
                let s = (near - a.2) / (b.2 - a.2);
                let m = (a.0 + (b.0 - a.0) * s, a.1 + (b.1 - a.1) * s, near);
                if a.2 < near {
                    a = m;
                } else {
                    b = m;
                }
            }
        }
        if let (Some(pa), Some(pb)) = (
            self.cam.project_camera(a, self.w, self.h),
            self.cam.project_camera(b, self.w, self.h),
        ) {
            t.line(pa, pb, color, bias, thickness);
        }
    }
}

/// Draws one triangle given its camera-space corners and per-corner shade values.
fn tri(
    ctx: &Ctx<'_>,
    t: &mut Target,
    c: [(f64, f64, f64); 3],
    shade: [f64; 3],
    color: impl Fn(f64) -> [u8; 3],
) {
    // Clip against the near plane (perspective only), keeping shade interpolated.
    let mut poly: Vec<((f64, f64, f64), f64)> = c.iter().copied().zip(shade).collect();
    if ctx.cam.projection == crate::camera::ProjectionKind::Perspective {
        let near = ctx.cam.near();
        let mut out = Vec::with_capacity(4);
        for i in 0..poly.len() {
            let (a, b) = (poly[i], poly[(i + 1) % poly.len()]);
            let (ain, bin) = (a.0 .2 >= near, b.0 .2 >= near);
            if ain {
                out.push(a);
            }
            if ain != bin {
                let s = (near - a.0 .2) / (b.0 .2 - a.0 .2);
                out.push((
                    (
                        a.0 .0 + (b.0 .0 - a.0 .0) * s,
                        a.0 .1 + (b.0 .1 - a.0 .1) * s,
                        near,
                    ),
                    a.1 + (b.1 - a.1) * s,
                ));
            }
        }
        poly = out;
    }
    if poly.len() < 3 {
        return;
    }
    let v: Vec<Vtx> = poly
        .iter()
        .filter_map(|(p, s)| {
            ctx.cam.project_camera(*p, ctx.w, ctx.h).map(|q| Vtx {
                x: q.x,
                y: q.y,
                key: q.key,
                shade: *s,
            })
        })
        .collect();
    if v.len() < 3 {
        return;
    }
    for k in 1..v.len() - 1 {
        t.fill(v[0], v[k], v[k + 1], &color);
    }
}

fn draw_body(
    ctx: &Ctx<'_>,
    t: &mut Target,
    body: &Body,
    opts: &RenderOptions,
    ids: Option<&[[u8; 3]]>,
) {
    let cam = ctx.cam;
    let (right, up, forward) = cam.basis();
    let cs: Vec<(f64, f64, f64)> = body
        .mesh
        .positions
        .iter()
        .map(|p| cam.camera_space(*p))
        .collect();
    let ortho = cam.projection == crate::camera::ProjectionKind::Orthographic;
    let sel = |r: &Option<crate::raster::FaceRef>, face: &str| {
        r.as_ref()
            .is_some_and(|r| r.body == body.id && r.face == face)
    };
    for (ti, tr) in body.mesh.triangles.iter().enumerate() {
        let c = [cs[tr[0] as usize], cs[tr[1] as usize], cs[tr[2] as usize]];
        let n = {
            let (u, v) = (
                (c[1].0 - c[0].0, c[1].1 - c[0].1, c[1].2 - c[0].2),
                (c[2].0 - c[0].0, c[2].1 - c[0].1, c[2].2 - c[0].2),
            );
            (
                u.1 * v.2 - u.2 * v.1,
                u.2 * v.0 - u.0 * v.2,
                u.0 * v.1 - u.1 * v.0,
            )
        };
        // Back-face culling: bodies are closed solids with outward-facing triangles. Camera space
        // (x right, y up, z forward) is left-handed, so a cross product taken there points the
        // opposite way to the world normal: a front-facing triangle has n . view > 0.
        let view = if ortho { (0.0, 0.0, 1.0) } else { c[0] };
        if n.0 * view.0 + n.1 * view.1 + n.2 * view.2 <= 0.0 {
            continue;
        }
        let face = body.mesh.face_of[ti] as usize;
        if let Some(ids) = ids {
            let col = ids[face];
            tri(ctx, t, c, [0.0; 3], move |_| col);
            continue;
        }
        let name = &body.face_names[face];
        let base = body.color;
        let tint = if sel(&opts.selected, name) {
            Some(([255, 140, 0], 0.6))
        } else if sel(&opts.hovered, name) {
            Some(([255, 220, 0], 0.4))
        } else {
            None
        };
        let shade =
            body.corner_normals[ti].map(|w| lambert((w.dot(right), w.dot(up), w.dot(forward))));
        tri(ctx, t, c, shade, move |s| {
            let lit = [
                (f64::from(base[0]) * s).round().clamp(0.0, 255.0) as u8,
                (f64::from(base[1]) * s).round().clamp(0.0, 255.0) as u8,
                (f64::from(base[2]) * s).round().clamp(0.0, 255.0) as u8,
            ];
            match tint {
                Some((c, k)) => mix(lit, c, k),
                None => lit,
            }
        });
    }
}

/// Screen-space ambient occlusion on the finished depth buffer, applied to the colours: each
/// pixel looks at neighbours within `radius` (camera-space units) and is darkened by those that
/// stand up out of its own surface close by. The same estimate as the GPU view uses, blurred so
/// the sampling pattern does not show, never darker than half.
fn occlude(t: &mut Target, cam: &Camera, radius: f64) {
    use dc_math::det;
    let (w, h) = (t.w, t.h);
    if w < 4 || h < 4 || radius <= 0.0 {
        return;
    }
    let aspect = w as f64 / h as f64;
    let ortho = cam.projection == crate::camera::ProjectionKind::Orthographic;
    // Camera-space position of a pixel that has something drawn in it.
    let pos = |x: usize, y: usize, key: &[f64]| -> Option<(f64, f64, f64)> {
        let k = key[y * w + x];
        if k == f64::NEG_INFINITY {
            return None;
        }
        let nx = (x as f64 + 0.5) / w as f64 * 2.0 - 1.0;
        let ny = 1.0 - (y as f64 + 0.5) / h as f64 * 2.0;
        Some(if ortho {
            let hh = cam.half_height();
            (nx * hh * aspect, ny * hh, -k)
        } else {
            let z = 1.0 / k;
            let tn = cam.tan_half();
            (nx * z * tn * aspect, ny * z * tn, z)
        })
    };
    let sub = |a: (f64, f64, f64), b: (f64, f64, f64)| (a.0 - b.0, a.1 - b.1, a.2 - b.2);
    let dot = |a: (f64, f64, f64), b: (f64, f64, f64)| a.0 * b.0 + a.1 * b.1 + a.2 * b.2;
    let len = |a: (f64, f64, f64)| det::sqrt(dot(a, a));
    let mut raw = vec![1.0f32; w * h];
    for y in 1..h - 1 {
        for x in 1..w - 1 {
            let Some(p) = pos(x, y, &t.key) else { continue };
            let (Some(xp), Some(xm), Some(yp), Some(ym)) = (
                pos(x + 1, y, &t.key),
                pos(x - 1, y, &t.key),
                pos(x, y + 1, &t.key),
                pos(x, y - 1, &t.key),
            ) else {
                continue;
            };
            let dx = if len(sub(xp, p)) < len(sub(p, xm)) {
                sub(xp, p)
            } else {
                sub(p, xm)
            };
            let dy = if len(sub(yp, p)) < len(sub(p, ym)) {
                sub(yp, p)
            } else {
                sub(p, ym)
            };
            let n = (
                dx.1 * dy.2 - dx.2 * dy.1,
                dx.2 * dy.0 - dx.0 * dy.2,
                dx.0 * dy.1 - dx.1 * dy.0,
            );
            let nl = len(n);
            if nl < 1e-12 {
                continue;
            }
            let mut n = (n.0 / nl, n.1 / nl, n.2 / nl);
            // Face the viewer: camera space looks along +z.
            if n.2 > 0.0 {
                n = (-n.0, -n.1, -n.2);
            }
            let pixel = len(sub(xp, p)).max(1e-9);
            let rpx = (radius / pixel).clamp(3.0, 40.0);
            let noise = {
                let v = 52.982_918_9 * ((x as f64 * 0.067_110_56 + y as f64 * 0.005_837_15) % 1.0);
                v % 1.0
            };
            const K: usize = 12;
            let mut occ = 0.0;
            for i in 0..K {
                let tt = (i as f64 + 0.5) / K as f64;
                let ang = i as f64 * 2.399_963_23 + noise * core::f64::consts::TAU;
                let qx = (x as f64 + det::cos(ang) * rpx * tt + 0.5).floor();
                let qy = (y as f64 + det::sin(ang) * rpx * tt + 0.5).floor();
                if qx < 0.0 || qy < 0.0 || qx >= w as f64 || qy >= h as f64 {
                    continue;
                }
                let Some(q) = pos(qx as usize, qy as usize, &t.key) else {
                    continue;
                };
                let v = sub(q, p);
                let d = len(v);
                if d < 1e-12 {
                    continue;
                }
                let reach = 1.0 - ((d - 0.6 * radius) / (0.6 * radius)).clamp(0.0, 1.0);
                let vs = (v.0 / radius, v.1 / radius, v.2 / radius);
                occ += reach * (dot(n, vs) - 0.04).max(0.0) / (dot(vs, vs) + 0.02);
            }
            raw[y * w + x] = (1.0 - occ * 2.0 / K as f64).clamp(0.0, 1.0) as f32;
        }
    }
    // A 5 x 5 blur that does not cross depth steps.
    let mut ao = raw.clone();
    for y in 2..h - 2 {
        for x in 2..w - 2 {
            let i = y * w + x;
            if t.key[i] == f64::NEG_INFINITY {
                continue;
            }
            let (mut sum, mut cnt) = (0.0f32, 0.0f32);
            for j in y - 2..=y + 2 {
                for k in x - 2..=x + 2 {
                    let q = j * w + k;
                    let same =
                        t.key[q] != f64::NEG_INFINITY && (t.key[q] - t.key[i]).abs() < radius * 0.5;
                    if same {
                        sum += raw[q];
                        cnt += 1.0;
                    }
                }
            }
            if cnt > 0.0 {
                ao[i] = sum / cnt;
            }
        }
    }
    for (i, occlusion) in ao.iter().enumerate() {
        if t.key[i] == f64::NEG_INFINITY {
            continue;
        }
        let k = 0.5 + 0.5 * f64::from(*occlusion);
        for c in 0..3 {
            t.rgb[i * 3 + c] = (f64::from(t.rgb[i * 3 + c]) * k).round().clamp(0.0, 255.0) as u8;
        }
    }
}

/// Renders the scene.
pub fn render(scene: &Scene, opts: &RenderOptions) -> RenderOutput {
    let ss = if opts.mode == RenderMode::FaceIds {
        1
    } else {
        opts.supersample.clamp(1, 4)
    } as usize;
    let (w, h) = (opts.width as usize * ss, opts.height as usize * ss);
    let ctx = Ctx {
        cam: &opts.camera,
        w: w as f64,
        h: h as f64,
    };
    let bg = if opts.mode == RenderMode::FaceIds {
        [255, 255, 255]
    } else {
        opts.background
    };
    let mut t = Target::new(w, h, bg);

    let mut legend = Vec::new();
    let mut id_table: Vec<Vec<[u8; 3]>> = Vec::new();
    if opts.mode == RenderMode::FaceIds {
        let total: usize = scene.bodies.iter().map(|b| b.face_names.len()).sum();
        let mut colors = id_colors(total, bg).into_iter();
        for b in &scene.bodies {
            let cols: Vec<[u8; 3]> = b
                .face_names
                .iter()
                .map(|_| colors.next().expect("enough colours"))
                .collect();
            for (c, n) in cols.iter().zip(&b.face_names) {
                legend.push(LegendEntry {
                    color: *c,
                    body: b.id.clone(),
                    face: n.clone(),
                });
            }
            id_table.push(cols);
        }
    }

    for (bi, b) in scene.bodies.iter().enumerate() {
        let ids = (opts.mode == RenderMode::FaceIds).then(|| id_table[bi].as_slice());
        draw_body(&ctx, &mut t, b, opts, ids);
    }

    if opts.mode == RenderMode::Shaded && opts.ambient_occlusion {
        if let Some((lo, hi)) = scene.body_bounds() {
            occlude(&mut t, &opts.camera, (hi - lo).length() * 0.07);
        }
    }

    if opts.mode == RenderMode::Shaded {
        let thick = ss;
        if opts.grid || opts.axes {
            let step = grid_step(opts.camera.distance);
            if opts.grid {
                let n = 20;
                let ext = step * f64::from(n);
                for i in -n..=n {
                    let c = f64::from(i) * step;
                    let col = if i % 5 == 0 {
                        [176, 182, 194]
                    } else {
                        [204, 208, 217]
                    };
                    ctx.segment(
                        &mut t,
                        Vec3::new(c, -ext, 0.0),
                        Vec3::new(c, ext, 0.0),
                        col,
                        -3e-4,
                        thick,
                    );
                    ctx.segment(
                        &mut t,
                        Vec3::new(-ext, c, 0.0),
                        Vec3::new(ext, c, 0.0),
                        col,
                        -3e-4,
                        thick,
                    );
                }
            }
            if opts.axes {
                let len = opts.camera.distance * 0.12;
                ctx.segment(
                    &mut t,
                    Vec3::ZERO,
                    Vec3::new(len, 0.0, 0.0),
                    [214, 60, 60],
                    3e-4,
                    thick * 2,
                );
                ctx.segment(
                    &mut t,
                    Vec3::ZERO,
                    Vec3::new(0.0, len, 0.0),
                    [60, 170, 80],
                    3e-4,
                    thick * 2,
                );
                ctx.segment(
                    &mut t,
                    Vec3::ZERO,
                    Vec3::new(0.0, 0.0, len),
                    [70, 100, 220],
                    3e-4,
                    thick * 2,
                );
            }
        }
        if let Some(g) = &opts.local_grid {
            let step = grid_step(opts.camera.distance);
            if g.grid {
                let n = 20;
                let ext = step * f64::from(n);
                let at = |u: f64, v: f64| g.origin + g.x * u + g.y * v;
                for i in -n..=n {
                    let c = f64::from(i) * step;
                    let col = if i % 5 == 0 {
                        [176, 182, 194]
                    } else {
                        [204, 208, 217]
                    };
                    ctx.segment(&mut t, at(c, -ext), at(c, ext), col, -3e-4, thick);
                    ctx.segment(&mut t, at(-ext, c), at(ext, c), col, -3e-4, thick);
                }
            }
            for [p, q] in g.origin_marker(opts.camera.distance * 0.06) {
                ctx.segment(&mut t, p, q, [214, 60, 60], 3e-4, thick * 2);
            }
        }
        if opts.edges {
            for b in &scene.bodies {
                for e in &b.edges {
                    ctx.segment(&mut t, e[0], e[1], [38, 40, 48], 3e-4, thick);
                }
            }
        }
        if opts.sketches {
            for l in &scene.lines {
                for s in l.points.windows(2) {
                    ctx.segment(&mut t, s[0], s[1], l.color, 6e-4, thick);
                }
            }
        }
    }

    let image = if ss == 1 {
        Image {
            width: opts.width,
            height: opts.height,
            rgb: t.rgb,
        }
    } else {
        let (ow, oh) = (opts.width as usize, opts.height as usize);
        let mut out = Vec::with_capacity(ow * oh * 3);
        for y in 0..oh {
            for x in 0..ow {
                let mut acc = [0u32; 3];
                for sy in 0..ss {
                    for sx in 0..ss {
                        let i = ((y * ss + sy) * w + x * ss + sx) * 3;
                        for (k, a) in acc.iter_mut().enumerate() {
                            *a += u32::from(t.rgb[i + k]);
                        }
                    }
                }
                let n = (ss * ss) as u32;
                out.extend(acc.iter().map(|a| ((a + n / 2) / n) as u8));
            }
        }
        Image {
            width: opts.width,
            height: opts.height,
            rgb: out,
        }
    };
    RenderOutput { image, legend }
}
