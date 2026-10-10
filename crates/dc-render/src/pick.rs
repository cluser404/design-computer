//! Picking: which face is under a pixel. An exact ray cast against the triangles, independent of
//! the rasterizer (the tests check the two agree).

use crate::camera::Camera;
use crate::scene::Scene;
use dc_math::Vec3;

#[derive(Clone, Debug, PartialEq)]
pub struct Pick {
    /// The body (feature) hit.
    pub body: String,
    pub face_index: usize,
    pub face: String,
    pub point: Vec3,
    /// Distance from the ray origin.
    pub distance: f64,
}

/// The nearest face under pixel `(px, py)` of a `w` x `h` viewport, if any.
pub fn pick(scene: &Scene, camera: &Camera, px: f64, py: f64, w: f64, h: f64) -> Option<Pick> {
    let (o, d) = camera.ray(px, py, w, h);
    let mut best: Option<(f64, usize, usize)> = None;
    for (bi, b) in scene.bodies.iter().enumerate() {
        let m = &b.mesh;
        for (ti, tr) in m.triangles.iter().enumerate() {
            let (a, bb, c) = (
                m.positions[tr[0] as usize],
                m.positions[tr[1] as usize],
                m.positions[tr[2] as usize],
            );
            let (e1, e2) = (bb - a, c - a);
            let hv = d.cross(e2);
            let det = e1.dot(hv);
            if det.abs() < 1e-14 {
                continue;
            }
            let f = 1.0 / det;
            let s = o - a;
            let u = f * s.dot(hv);
            // A little slack so a ray exactly on an edge shared by two triangles hits one of
            // them: with exact bounds rounding can make both miss and the ray sees through.
            const SLACK: f64 = 1e-9;
            if !(-SLACK..=1.0 + SLACK).contains(&u) {
                continue;
            }
            let q = s.cross(e1);
            let v = f * d.dot(q);
            if v < -SLACK || u + v > 1.0 + SLACK {
                continue;
            }
            let t = f * e2.dot(q);
            if t > 1e-9 && best.is_none_or(|(bt, _, _)| t < bt) {
                best = Some((t, bi, ti));
            }
        }
    }
    best.map(|(t, bi, ti)| {
        let b = &scene.bodies[bi];
        let face_index = b.mesh.face_of[ti] as usize;
        Pick {
            body: b.id.clone(),
            face_index,
            face: b.face_names[face_index].clone(),
            point: o + d * t,
            distance: t,
        }
    })
}

/// What kinds of things a pick may return (SolidWorks' selection filter).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Filter {
    pub faces: bool,
    pub edges: bool,
    pub sketch_lines: bool,
}

impl Filter {
    pub const ALL: Filter = Filter {
        faces: true,
        edges: true,
        sketch_lines: true,
    };
    pub const EDGES: Filter = Filter {
        faces: false,
        edges: true,
        sketch_lines: false,
    };
    pub const SKETCH_LINES: Filter = Filter {
        faces: false,
        edges: false,
        sketch_lines: true,
    };
}

/// Whatever is under the cursor.
#[derive(Clone, Debug, PartialEq)]
pub enum Hit {
    Face(Pick),
    /// An edge of a body, named by its two faces (see `Body::edge_name`).
    Edge {
        body: String,
        name: String,
        point: Vec3,
    },
    /// A line, arc or circle of a sketch drawn in the view.
    SketchLine {
        sketch: String,
        entity: String,
        point: Vec3,
    },
}

/// The nearest point to pixel `(px, py)` on the segment `a`-`b` seen from `camera`: its pixel
/// distance and its camera depth. `None` if an end is behind the camera.
#[allow(clippy::too_many_arguments)]
fn segment_distance(
    camera: &Camera,
    a: Vec3,
    b: Vec3,
    px: f64,
    py: f64,
    w: f64,
    h: f64,
) -> Option<(f64, f64)> {
    let (pa, pb) = (camera.project(a, w, h)?, camera.project(b, w, h)?);
    let (dx, dy) = (pb.x - pa.x, pb.y - pa.y);
    let len2 = dx * dx + dy * dy;
    let t = if len2 < 1e-12 {
        0.0
    } else {
        (((px - pa.x) * dx + (py - pa.y) * dy) / len2).clamp(0.0, 1.0)
    };
    let (cx, cy) = (pa.x + t * dx, pa.y + t * dy);
    let dist = ((px - cx) * (px - cx) + (py - cy) * (py - cy)).sqrt();
    Some((dist, pa.depth + t * (pb.depth - pa.depth)))
}

/// The thing under the pixel that `filter` allows. Edges and sketch lines win over a face when
/// the pointer is within `tol_px` of them (and they are not hidden behind it); otherwise the
/// face is returned. When only sketch lines are allowed they are picked through the part (a
/// revolve axis is chosen with the preview in the way). Among edges and lines the nearest to the pointer wins, edges first on a tie.
#[allow(clippy::too_many_arguments)]
pub fn pick_hit(
    scene: &Scene,
    camera: &Camera,
    px: f64,
    py: f64,
    w: f64,
    h: f64,
    filter: Filter,
    tol_px: f64,
) -> Option<Hit> {
    // The face under the pointer also decides what hides an edge. It is needed even when faces
    // are filtered out of the answer.
    let face = pick(scene, camera, px, py, w, h);
    let face_depth = face
        .as_ref()
        .map(|f| camera.camera_space(f.point).2)
        .unwrap_or(f64::INFINITY);
    let eps = 1e-3 * face_depth.abs().max(1.0) + 1e-6;
    let mut best: Option<(f64, Hit)> = None;
    let mut offer = |dist: f64, hit: Hit| {
        if dist <= tol_px && best.as_ref().is_none_or(|(d, _)| dist < *d) {
            best = Some((dist, hit));
        }
    };
    if filter.edges {
        for b in &scene.bodies {
            for (i, e) in b.edges.iter().enumerate() {
                let Some((d, depth)) = segment_distance(camera, e[0], e[1], px, py, w, h) else {
                    continue;
                };
                if d <= tol_px && depth <= face_depth + eps {
                    offer(
                        d,
                        Hit::Edge {
                            body: b.id.clone(),
                            name: b.edge_name(i),
                            point: (e[0] + e[1]) * 0.5,
                        },
                    );
                }
            }
        }
    }
    if filter.sketch_lines {
        for l in &scene.lines {
            let mut parts = l.id.splitn(3, '/');
            let (Some("sketch"), Some(sk), Some(ent)) = (parts.next(), parts.next(), parts.next())
            else {
                continue;
            };
            for seg in l.points.windows(2) {
                let Some((d, depth)) = segment_distance(camera, seg[0], seg[1], px, py, w, h)
                else {
                    continue;
                };
                // A sketch drawn on a face is level with it: give it the same slack.
                let xray = !filter.faces && !filter.edges;
                if d <= tol_px && (xray || depth <= face_depth + eps.max(1e-2 * face_depth.abs())) {
                    // Edges of the part are offered first, so a lower distance is needed to
                    // beat one: lines lie on the part's faces and would otherwise shadow them.
                    offer(
                        d + 0.5,
                        Hit::SketchLine {
                            sketch: sk.to_owned(),
                            entity: ent.to_owned(),
                            point: (seg[0] + seg[1]) * 0.5,
                        },
                    );
                }
            }
        }
    }
    if let Some((_, hit)) = best {
        return Some(hit);
    }
    if filter.faces {
        return face.map(Hit::Face);
    }
    None
}

/// A pixel rectangle `(x0, y0, x1, y1)` with `x0 <= x1` and `y0 <= y1`.
pub type PixelRect = (f64, f64, f64, f64);

fn inside(r: PixelRect, x: f64, y: f64) -> bool {
    x >= r.0 && x <= r.2 && y >= r.1 && y <= r.3
}

/// Does the segment touch the rectangle?
fn segment_touches(r: PixelRect, a: (f64, f64), b: (f64, f64)) -> bool {
    if inside(r, a.0, a.1) || inside(r, b.0, b.1) {
        return true;
    }
    // Liang-Barsky clip of the segment against the rectangle.
    let (dx, dy) = (b.0 - a.0, b.1 - a.1);
    let (mut t0, mut t1) = (0.0f64, 1.0f64);
    for (p, q) in [
        (-dx, a.0 - r.0),
        (dx, r.2 - a.0),
        (-dy, a.1 - r.1),
        (dy, r.3 - a.1),
    ] {
        if p.abs() < 1e-12 {
            if q < 0.0 {
                return false;
            }
        } else {
            let t = q / p;
            if p < 0.0 {
                t0 = t0.max(t);
            } else {
                t1 = t1.min(t);
            }
            if t0 > t1 {
                return false;
            }
        }
    }
    true
}

/// Everything the filter allows that a rubber-band rectangle takes, as SolidWorks does it: with
/// `crossing` false, only what lies wholly inside the rectangle; with `crossing` true, whatever
/// touches it. Only what can be seen counts for faces and edges (a face or edge hidden behind
/// another is left out); sketch lines are taken wherever they are. Deterministic: bodies in
/// scene order, then faces, then edges, then sketch lines.
pub fn box_hits(
    scene: &Scene,
    camera: &Camera,
    rect: PixelRect,
    crossing: bool,
    w: f64,
    h: f64,
    filter: Filter,
) -> Vec<Hit> {
    let mut out = Vec::new();
    // What is seen at a grid of pixels inside the rectangle: the faces a touching box takes.
    let seen_in_rect: Vec<(String, usize)> = if crossing && filter.faces {
        let mut v: Vec<(String, usize)> = Vec::new();
        let n = 9;
        for i in 0..n {
            for j in 0..n {
                let x = rect.0 + (rect.2 - rect.0) * (f64::from(i) + 0.5) / f64::from(n);
                let y = rect.1 + (rect.3 - rect.1) * (f64::from(j) + 0.5) / f64::from(n);
                if let Some(k) = pick(scene, camera, x, y, w, h) {
                    if !v.iter().any(|(b, f)| *b == k.body && *f == k.face_index) {
                        v.push((k.body, k.face_index));
                    }
                }
            }
        }
        v
    } else {
        Vec::new()
    };
    for b in &scene.bodies {
        let m = &b.mesh;
        let proj: Vec<Option<(f64, f64)>> = m
            .positions
            .iter()
            .map(|&p| camera.project(p, w, h).map(|q| (q.x, q.y)))
            .collect();
        if filter.faces {
            for (fi, name) in b.face_names.iter().enumerate() {
                let tris: Vec<usize> = (0..m.triangles.len())
                    .filter(|&t| m.face_of[t] as usize == fi)
                    .collect();
                if tris.is_empty() {
                    continue;
                }
                // The face's vertices, for the "wholly inside" rule.
                let mut whole = true;
                let mut any_vertex = false;
                for &t in &tris {
                    for &v in &m.triangles[t] {
                        match proj[v as usize] {
                            Some((x, y)) => {
                                any_vertex = true;
                                whole &= inside(rect, x, y);
                            }
                            None => whole = false,
                        }
                    }
                }
                if !any_vertex || (!crossing && !whole) {
                    continue;
                }
                // Visible: a sample triangle's centre is seen as this face, and (crossing) in the box.
                let step = (tris.len() / 12).max(1);
                let visible = tris.iter().step_by(step).any(|&t| {
                    let tr = m.triangles[t];
                    let c = (m.positions[tr[0] as usize]
                        + m.positions[tr[1] as usize]
                        + m.positions[tr[2] as usize])
                        / 3.0;
                    let Some(p) = camera.project(c, w, h) else {
                        return false;
                    };
                    if crossing && !inside(rect, p.x, p.y) {
                        return false;
                    }
                    pick(scene, camera, p.x, p.y, w, h)
                        .is_some_and(|k| k.body == b.id && k.face_index == fi)
                });
                let visible = visible
                    || (crossing && seen_in_rect.iter().any(|(bb, f)| *bb == b.id && *f == fi));
                if visible {
                    let t = m.triangles[tris[0]];
                    out.push(Hit::Face(Pick {
                        body: b.id.clone(),
                        face_index: fi,
                        face: name.clone(),
                        point: m.positions[t[0] as usize],
                        distance: 0.0,
                    }));
                }
            }
        }
        if filter.edges {
            // Segments grouped by name, in first-seen order.
            let mut names: Vec<String> = Vec::new();
            let mut groups: Vec<Vec<usize>> = Vec::new();
            for i in 0..b.edges.len() {
                let n = b.edge_name(i);
                match names.iter().position(|x| *x == n) {
                    Some(k) => groups[k].push(i),
                    None => {
                        names.push(n);
                        groups.push(vec![i]);
                    }
                }
            }
            for (name, segs) in names.into_iter().zip(groups) {
                let mut seen = 0;
                let mut all_in = true;
                let mut any_touch = false;
                let mut point = b.edges[segs[0]][0];
                for &i in &segs {
                    let e = b.edges[i];
                    let (Some(pa), Some(pb)) =
                        (camera.project(e[0], w, h), camera.project(e[1], w, h))
                    else {
                        all_in = false;
                        continue;
                    };
                    // Hidden behind a face at its middle: not offered.
                    let mid = (e[0] + e[1]) * 0.5;
                    let (mx, my) = ((pa.x + pb.x) * 0.5, (pa.y + pb.y) * 0.5);
                    let md = camera.camera_space(mid).2;
                    let front = pick(scene, camera, mx, my, w, h)
                        .map(|f| camera.camera_space(f.point).2)
                        .unwrap_or(f64::INFINITY);
                    if md > front + 1e-3 * front.abs().max(1.0) + 1e-6 {
                        continue;
                    }
                    seen += 1;
                    point = mid;
                    all_in &= inside(rect, pa.x, pa.y) && inside(rect, pb.x, pb.y);
                    any_touch |= segment_touches(rect, (pa.x, pa.y), (pb.x, pb.y));
                }
                if seen > 0 && ((crossing && any_touch) || (!crossing && all_in)) {
                    out.push(Hit::Edge {
                        body: b.id.clone(),
                        name,
                        point,
                    });
                }
            }
        }
    }
    if filter.sketch_lines {
        for l in &scene.lines {
            let mut parts = l.id.splitn(3, '/');
            let (Some("sketch"), Some(sk), Some(ent)) = (parts.next(), parts.next(), parts.next())
            else {
                continue;
            };
            let pts: Vec<Option<(f64, f64)>> = l
                .points
                .iter()
                .map(|&p| camera.project(p, w, h).map(|q| (q.x, q.y)))
                .collect();
            let all_in = pts
                .iter()
                .all(|p| p.is_some_and(|(x, y)| inside(rect, x, y)));
            let any_touch = pts.windows(2).any(|s| match (s[0], s[1]) {
                (Some(a), Some(b)) => segment_touches(rect, a, b),
                _ => false,
            });
            if (crossing && any_touch) || (!crossing && all_in && !pts.is_empty()) {
                out.push(Hit::SketchLine {
                    sketch: sk.to_owned(),
                    entity: ent.to_owned(),
                    point: l.points[0],
                });
            }
        }
    }
    out
}
