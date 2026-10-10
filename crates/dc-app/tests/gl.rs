//! The GPU path's CPU-side maths, checked against the camera that the picker and the software
//! renderer already agree on. (The GL calls themselves need a context; see the Xvfb screenshots.)

use dc_app::gl::{Frame, Geometry, ViewState};
use dc_app::state::AppState;
use dc_math::{det, Vec3};
use dc_render::{FaceRef, ProjectionKind, ViewPreset};
use std::sync::Arc;

fn view(a: &AppState, aspect: f64) -> ViewState {
    ViewState {
        aspect,
        extent: a.scene.bounds().map_or(0.0, |(l, h)| (h - l).length()),
        grid: true,
        origins: true,
        pixels_per_point: 1.0,
        viewport_px_height: 600.0,
        ambient_occlusion: true,
        fxaa: false,
        floor_reflection: true,
        environment: dc_app::environment::Environment::default(),
        local_grid: None,
        edges: true,
        sketches: true,
        selected: None,
        hovered: None,
        realview: None,
        bounds: a.scene.body_bounds(),
    }
}

fn demo() -> AppState {
    let mut a = AppState::new();
    a.load_demo();
    a
}

#[test]
fn the_gpu_matrix_puts_points_where_the_camera_does() {
    let a = demo();
    let g = Arc::new(Geometry::from_scene(&a.scene, 1));
    let (w, h) = (900.0, 600.0);
    let (lo, hi) = a.scene.body_bounds().unwrap();
    let pts = [
        lo,
        hi,
        Vec3::new(lo.x, hi.y, lo.z),
        Vec3::new(hi.x, lo.y, hi.z),
        (lo + hi) * 0.5,
    ];
    for kind in [ProjectionKind::Perspective, ProjectionKind::Orthographic] {
        for preset in [
            ViewPreset::Iso,
            ViewPreset::Front,
            ViewPreset::Top,
            ViewPreset::Right,
        ] {
            let mut fitted = dc_render::Camera::preset(preset);
            fitted.projection = kind;
            fitted.fit_for(lo, hi, w / h);
            let cam = fitted;
            let f = Frame::new(g.clone(), &cam, &view(&a, w / h));
            for p in pts {
                let want = cam.project(p, w, h).expect("in front of the camera");
                let c = f.clip([p.x as f32, p.y as f32, p.z as f32]);
                let (nx, ny, nz) = (c[0] / c[3], c[1] / c[3], c[2] / c[3]);
                let (px, py) = (
                    (f64::from(nx) + 1.0) * 0.5 * w,
                    (1.0 - f64::from(ny)) * 0.5 * h,
                );
                assert!(
                    (px - want.x).abs() < 0.05 && (py - want.y).abs() < 0.05,
                    "{kind:?} {preset:?}: gl ({px:.2},{py:.2}) vs camera ({:.2},{:.2})",
                    want.x,
                    want.y
                );
                assert!(
                    (-1.0..=1.0).contains(&nz),
                    "{kind:?} {preset:?}: depth {nz} inside the clip range"
                );
            }
        }
    }
}

#[test]
fn nearer_points_get_smaller_depth_in_both_projections() {
    let a = demo();
    let g = Arc::new(Geometry::from_scene(&a.scene, 1));
    for kind in [ProjectionKind::Perspective, ProjectionKind::Orthographic] {
        let mut cam = a.camera;
        cam.projection = kind;
        let f = Frame::new(g.clone(), &cam, &view(&a, 1.5));
        let (_, _, fwd) = cam.basis();
        let t = cam.target;
        let near = t - fwd * 10.0;
        let far = t + fwd * 10.0;
        let d = |p: Vec3| {
            let c = f.clip([p.x as f32, p.y as f32, p.z as f32]);
            c[2] / c[3]
        };
        assert!(
            d(near) < d(t) && d(t) < d(far),
            "{kind:?}: {} {} {}",
            d(near),
            d(t),
            d(far)
        );
    }
}

#[test]
fn geometry_holds_every_triangle_and_edge_of_the_scene() {
    let a = demo();
    let g = Arc::new(Geometry::from_scene(&a.scene, 7));
    let f = Frame::new(g, &a.camera, &view(&a, 1.5));
    let tris: usize = a.scene.bodies.iter().map(|b| b.mesh.triangles.len()).sum();
    let edges: usize = a.scene.bodies.iter().map(|b| b.edges.len()).sum();
    let sk: usize = a
        .scene
        .lines
        .iter()
        .map(|l| l.points.len().saturating_sub(1))
        .sum();
    assert_eq!(f.counts(), (tris, edges, sk));
    assert!(tris > 50 && edges > 20);
}

#[test]
fn selection_and_hover_resolve_to_distinct_face_ids_and_unknown_faces_to_none() {
    let a = demo();
    let g = Arc::new(Geometry::from_scene(&a.scene, 1));
    let body = a.scene.bodies[0].id.clone();
    let name = a.scene.bodies[0].face_names[0].clone();
    let other = a.scene.bodies[0].face_names[1].clone();
    let mut v = view(&a, 1.5);
    v.selected = Some(FaceRef {
        body: body.clone(),
        face: name,
    });
    v.hovered = Some(FaceRef {
        body: body.clone(),
        face: other,
    });
    let f = Frame::new(g.clone(), &a.camera, &v);
    let (sel, hov) = f.highlighted();
    assert!(
        sel.is_some() && hov.is_some() && sel != hov,
        "{sel:?} {hov:?}"
    );
    v.selected = Some(FaceRef {
        body,
        face: "no such face".into(),
    });
    v.hovered = None;
    assert_eq!(Frame::new(g, &a.camera, &v).highlighted(), (None, None));
}

#[test]
fn the_shadow_map_covers_every_corner_of_the_part_for_any_material() {
    use dc_app::state::Material;
    let a = demo();
    let g = Arc::new(Geometry::from_scene(&a.scene, 1));
    let (lo, hi) = a.scene.body_bounds().unwrap();
    for m in Material::ALL {
        let mut v = view(&a, 1.5);
        v.realview = Some(m);
        let f = Frame::new(g.clone(), &a.camera, &v);
        for &x in &[lo.x, hi.x] {
            for &y in &[lo.y, hi.y] {
                for &z in &[lo.z, hi.z] {
                    let c = f
                        .light_clip([x as f32, y as f32, z as f32])
                        .expect("realview has a light");
                    for (k, v) in c.iter().take(3).enumerate() {
                        assert!((-1.0..=1.0).contains(v), "{m:?}: light-space {k} = {v}");
                    }
                }
            }
        }
    }
    // Without RealView there is no light at all.
    let f = Frame::new(g, &a.camera, &view(&a, 1.5));
    assert!(f.light_clip([0.0; 3]).is_none());
}

#[test]
fn materials_have_physically_sensible_parameters() {
    use dc_app::state::Material;
    for m in Material::ALL {
        let (c, metal, rough) = m.params();
        assert!(c.iter().all(|x| (0.0..=1.0).contains(x)), "{m:?} colour");
        assert!(
            (0.0..=1.0).contains(&metal) && (0.05..=1.0).contains(&rough),
            "{m:?}"
        );
        assert!(!m.name().is_empty());
        assert!((0.0..=1.0).contains(&m.clearcoat()), "{m:?} clear coat");
    }
}

#[test]
fn a_cylinders_outline_runs_along_the_two_tangent_lines_and_follows_the_eye() {
    // The showcase part has a cylinder (centre 40, 48 on the sketch plane, radius 24). Seen
    // from a given yaw its outline is the two generators where the surface turns edge-on:
    // the points of the cylinder whose angle round the axis is yaw +- 90 degrees.
    let mut a = AppState::new();
    a.load_showcase();
    let g = Arc::new(Geometry::from_scene(&a.scene, 1));
    let (lo, hi) = a.scene.body_bounds().unwrap();
    for (kind, yaw_deg) in [
        (ProjectionKind::Orthographic, 30.0f64),
        (ProjectionKind::Orthographic, 140.0),
        (ProjectionKind::Perspective, 30.0),
    ] {
        let mut cam = dc_render::Camera::preset(ViewPreset::Iso);
        cam.projection = kind;
        cam.fit_for(lo, hi, 1.5);
        cam.yaw = yaw_deg.to_radians();
        cam.pitch = 0.5;
        let f = Frame::new(g.clone(), &cam, &view(&a, 1.5));
        let segs = f.silhouette_segments();
        assert!(
            !segs.is_empty(),
            "{kind:?} {yaw_deg}: a curved part has an outline"
        );
        // Points on the outer wall of the cylinder (not the bore, radius 13), in the part
        // above the plate (z > 14).
        let on_wall: Vec<[f32; 3]> = segs
            .iter()
            .flatten()
            .filter(|p| {
                let r = det::hypot(f64::from(p[0]) - 40.0, f64::from(p[1]) - 48.0);
                (r - 24.0).abs() < 0.6 && p[2] > 14.5
            })
            .copied()
            .collect();
        assert!(
            on_wall.len() >= 2,
            "{kind:?} {yaw_deg}: outline points on the wall"
        );
        // Expected generators: where the radial direction is square to the line of sight. For
        // an orthographic view that is yaw +- 90 degrees; for a perspective one the eye is
        // finite, so it is the tangent from the eye (a little less than 90 degrees).
        let eye = cam.eye();
        let to_eye = (eye.x - 40.0, eye.y - 48.0);
        let eye_az = det::atan2(to_eye.1, to_eye.0);
        let tol = if kind == ProjectionKind::Orthographic {
            4.0
        } else {
            20.0
        };
        for p in &on_wall {
            let az = det::atan2(f64::from(p[1]) - 48.0, f64::from(p[0]) - 40.0);
            let off = (az - eye_az).rem_euclid(std::f64::consts::TAU).to_degrees();
            let from_side = (off - 90.0).abs().min((off - 270.0).abs());
            assert!(
                from_side < tol,
                "{kind:?} {yaw_deg}: point at {off} degrees from the eye"
            );
        }
        // Both sides appear.
        let sides = on_wall
            .iter()
            .map(|p| {
                let az = det::atan2(f64::from(p[1]) - 48.0, f64::from(p[0]) - 40.0);
                (az - eye_az).rem_euclid(std::f64::consts::TAU) < std::f64::consts::PI
            })
            .collect::<Vec<_>>();
        assert!(
            sides.iter().any(|s| *s) && sides.iter().any(|s| !*s),
            "both tangent lines"
        );
    }
}
