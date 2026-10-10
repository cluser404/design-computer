//! Rendering checked at the pixel level, and picking checked against the rasterizer.

use dc_doc::{Actor, Authority, Command, CommandKind as K};
use dc_features::{build_solid, Stepped};
use dc_math::{Plane, Tolerance, Vec3};
use dc_render::{
    pick, render, scene_of_authority, Body, Camera, ProjectionKind, RenderMode, RenderOptions,
    Scene, ViewPreset,
};
use dc_sketch::{Entity, Profile, Sketch};

fn xy() -> Plane {
    Plane::new(Vec3::ZERO, Vec3::X, Vec3::Y, Tolerance::DEFAULT).unwrap()
}
fn rect(x0: f64, y0: f64, x1: f64, y1: f64) -> Profile {
    let mut s = Sketch::new();
    for (i, (x, y)) in [(x0, y0), (x1, y0), (x1, y1), (x0, y1)].iter().enumerate() {
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
    s.profiles().profiles.remove(0)
}
fn disc(cx: f64, cy: f64, r: f64) -> Profile {
    let mut s = Sketch::new();
    s.add_entity("m", Entity::Point { x: cx, y: cy }).unwrap();
    s.add_entity(
        "c",
        Entity::Circle {
            center: "m".into(),
            radius: r,
        },
    )
    .unwrap();
    s.profiles().profiles.remove(0)
}
fn body(id: &str, st: &Stepped, color: [u8; 3]) -> Body {
    Body::from_solid(id, &build_solid(st).unwrap(), color).unwrap()
}
fn box_scene() -> Scene {
    let st = Stepped::extrusion(xy(), rect(0.0, 0.0, 80.0, 40.0), 20.0, false);
    Scene {
        bodies: vec![body("box", &st, [120, 160, 210])],
        lines: vec![],
    }
}
fn opts(cam: Camera, mode: RenderMode) -> RenderOptions {
    let mut o = RenderOptions::new(240, 180, cam);
    o.mode = mode;
    o.grid = false;
    o.axes = false;
    o
}
fn fitted(scene: &Scene, preset: ViewPreset, kind: ProjectionKind) -> Camera {
    let mut c = Camera::preset(preset);
    c.projection = kind;
    let (lo, hi) = scene.bounds().unwrap();
    c.fit(lo, hi);
    c
}
/// The face name at the centre pixel of a face-id render.
fn centre_face(scene: &Scene, cam: Camera) -> String {
    let out = render(scene, &opts(cam, RenderMode::FaceIds));
    let c = out.image.pixel(out.image.width / 2, out.image.height / 2);
    out.legend
        .iter()
        .find(|l| l.color == c)
        .map(|l| l.face.clone())
        .unwrap_or_else(|| format!("none ({c:?})"))
}

#[test]
fn every_standard_view_sees_the_face_it_should() {
    let s = box_scene();
    // e0 is the wall at y=0 (normal -y), e1 at x=80 (+x), e2 at y=40 (+y), e3 at x=0 (-x).
    for (preset, want) in [
        (ViewPreset::Front, "side:e0"),
        (ViewPreset::Back, "side:e2"),
        (ViewPreset::Right, "side:e1"),
        (ViewPreset::Left, "side:e3"),
        (ViewPreset::Top, "cap_end"),
        (ViewPreset::Bottom, "cap_start"),
    ] {
        for kind in [ProjectionKind::Orthographic, ProjectionKind::Perspective] {
            assert_eq!(
                centre_face(&s, fitted(&s, preset, kind)),
                want,
                "{preset:?} {kind:?}"
            );
        }
    }
}

#[test]
fn face_id_renders_come_with_a_complete_unique_legend() {
    let s = box_scene();
    let out = render(
        &s,
        &opts(
            fitted(&s, ViewPreset::Iso, ProjectionKind::Perspective),
            RenderMode::FaceIds,
        ),
    );
    assert_eq!(out.legend.len(), 6);
    let mut cols: Vec<_> = out.legend.iter().map(|l| l.color).collect();
    cols.sort();
    cols.dedup();
    assert_eq!(cols.len(), 6, "every face has its own colour");
    assert!(cols
        .iter()
        .all(|c| *c != [0, 0, 0] && *c != [255, 255, 255]));
    // Three faces are visible from the iso view; every other pixel is background.
    let mut seen = std::collections::BTreeSet::new();
    for y in 0..out.image.height {
        for x in 0..out.image.width {
            let p = out.image.pixel(x, y);
            if p != [255, 255, 255] {
                let l = out
                    .legend
                    .iter()
                    .find(|l| l.color == p)
                    .unwrap_or_else(|| panic!("pixel {p:?} is not in the legend"));
                seen.insert(l.face.clone());
            }
        }
    }
    assert_eq!(seen.len(), 3, "{seen:?}");
}

#[test]
fn shaded_renders_have_background_lit_faces_and_dark_edges() {
    let s = box_scene();
    let cam = fitted(&s, ViewPreset::Iso, ProjectionKind::Perspective);
    let o = opts(cam, RenderMode::Shaded);
    let out = render(&s, &o);
    let img = &out.image;
    assert_eq!(img.pixel(2, 2), o.background, "corner is background");
    assert_eq!(img.rgb.len(), 240 * 180 * 3);
    // Different faces get different brightness (lighting works), and none is the background.
    let faces = render(&s, &opts(cam, RenderMode::FaceIds));
    let mut tones = std::collections::BTreeMap::new();
    for y in (0..img.height).step_by(3) {
        for x in (0..img.width).step_by(3) {
            let id = faces.image.pixel(x, y);
            if let Some(l) = faces.legend.iter().find(|l| l.color == id) {
                tones
                    .entry(l.face.clone())
                    .or_insert_with(Vec::new)
                    .push(img.pixel(x, y));
            }
        }
    }
    assert_eq!(tones.len(), 3);
    let means: Vec<u32> = tones
        .values()
        .map(|v| {
            v.iter()
                .map(|p| u32::from(p[0]) + u32::from(p[1]) + u32::from(p[2]))
                .sum::<u32>()
                / v.len() as u32
        })
        .collect();
    assert!(
        means.windows(2).any(|w| w[0] != w[1]) && means.iter().all(|m| *m > 60),
        "{means:?}"
    );
    // Edges: dark pixels exist; and with edges off there are none.
    let dark = |i: &dc_render::Image| {
        (0..i.height)
            .flat_map(|y| (0..i.width).map(move |x| (x, y)))
            .filter(|&(x, y)| i.pixel(x, y).iter().all(|c| *c < 70))
            .count()
    };
    assert!(dark(img) > 150, "only {} edge pixels", dark(img));
    let mut no_edges = o.clone();
    no_edges.edges = false;
    assert_eq!(
        dark(&render(&s, &no_edges).image),
        0,
        "edges off means no dark lines"
    );
}

#[test]
fn nearer_bodies_hide_farther_ones() {
    let a = Stepped::extrusion(xy(), rect(0.0, 0.0, 30.0, 10.0), 20.0, false); // y 0..10: in front
    let b = Stepped::extrusion(xy(), rect(0.0, 30.0, 30.0, 40.0), 20.0, false); // y 30..40: behind
    let s = Scene {
        bodies: vec![
            body("near", &a, [200, 60, 60]),
            body("far", &b, [60, 60, 200]),
        ],
        lines: vec![],
    };
    for kind in [ProjectionKind::Orthographic, ProjectionKind::Perspective] {
        let cam = fitted(&s, ViewPreset::Front, kind);
        let out = render(&s, &opts(cam, RenderMode::FaceIds));
        let c = out.image.pixel(out.image.width / 2, out.image.height / 2);
        assert_eq!(
            out.legend.iter().find(|l| l.color == c).unwrap().body,
            "near",
            "{kind:?}"
        );
        // From the back, the other way round.
        let cam = fitted(&s, ViewPreset::Back, kind);
        let out = render(&s, &opts(cam, RenderMode::FaceIds));
        let c = out.image.pixel(out.image.width / 2, out.image.height / 2);
        assert_eq!(
            out.legend.iter().find(|l| l.color == c).unwrap().body,
            "far",
            "{kind:?}"
        );
    }
}

#[test]
fn highlighting_a_face_changes_only_that_face() {
    let s = box_scene();
    let cam = fitted(&s, ViewPreset::Iso, ProjectionKind::Perspective);
    let plain = render(&s, &opts(cam, RenderMode::Shaded)).image;
    let mut o = opts(cam, RenderMode::Shaded);
    o.selected = Some(dc_render::raster::FaceRef {
        body: "box".into(),
        face: "cap_end".into(),
    });
    let lit = render(&s, &o).image;
    let ids = render(&s, &opts(cam, RenderMode::FaceIds));
    let top = ids
        .legend
        .iter()
        .find(|l| l.face == "cap_end")
        .unwrap()
        .color;
    let (mut changed, mut outside) = (0, 0);
    for y in 0..plain.height {
        for x in 0..plain.width {
            if plain.pixel(x, y) != lit.pixel(x, y) {
                changed += 1;
                if ids.image.pixel(x, y) != top {
                    outside += 1;
                }
            }
        }
    }
    assert!(
        changed > 500,
        "the selected face visibly changes ({changed} px)"
    );
    assert_eq!(outside, 0, "no pixel outside the selected face changed");
}

#[test]
fn rendering_is_deterministic_and_supersampling_keeps_the_size() {
    let s = box_scene();
    let cam = fitted(&s, ViewPreset::Iso, ProjectionKind::Perspective);
    let o = opts(cam, RenderMode::Shaded);
    assert_eq!(render(&s, &o).image, render(&s, &o).image);
    let mut o2 = o.clone();
    o2.supersample = 2;
    let ss = render(&s, &o2).image;
    assert_eq!((ss.width, ss.height), (240, 180));
    assert_eq!(ss.rgb.len(), 240 * 180 * 3);
    // Anti-aliasing produces intermediate tones along edges that the 1x render does not have.
    let base = render(&s, &o).image;
    let distinct = |i: &dc_render::Image| {
        i.rgb
            .chunks(3)
            .map(|c| [c[0], c[1], c[2]])
            .collect::<std::collections::BTreeSet<_>>()
            .len()
    };
    assert!(distinct(&ss) > distinct(&base));
    // The PNG is a valid, compact file.
    let png = ss.to_png();
    assert_eq!(&png[1..4], b"PNG");
    assert!(png.len() < ss.rgb.len() / 4, "{} bytes", png.len());
}

#[test]
fn a_camera_inside_or_beside_the_geometry_does_not_break_rendering() {
    let s = box_scene();
    let mut cam = Camera::preset(ViewPreset::Front);
    for (target, dist) in [
        (Vec3::new(40.0, 20.0, 10.0), 0.01),
        (Vec3::new(40.0, 20.0, 10.0), 1.0),
        (Vec3::new(40.0, 0.0, 10.0), 0.5),
        (Vec3::new(1e5, 1e5, 1e5), 10.0),
    ] {
        cam.target = target;
        cam.distance = dist;
        let mut o = opts(cam, RenderMode::Shaded);
        o.grid = true;
        o.axes = true;
        let out = render(&s, &o);
        assert_eq!(out.image.rgb.len(), 240 * 180 * 3);
    }
}

/// The ray caster and the rasterizer are separate implementations: they must agree on which face
/// is under each pixel, on a shape with curved and flat faces, pockets and a boss.
#[test]
fn picking_agrees_with_the_rasterizer() {
    let st = Stepped::extrusion(xy(), rect(0.0, 0.0, 100.0, 60.0), 12.0, false)
        .cut(&disc(25.0, 30.0, 9.0), 12.0, 6.0, "pk")
        .unwrap()
        .boss(&rect(50.0, 15.0, 90.0, 45.0), 12.0, 22.0, "bs")
        .unwrap();
    let s = Scene {
        bodies: vec![body("part", &st, [150, 150, 150])],
        lines: vec![],
    };
    for kind in [ProjectionKind::Perspective, ProjectionKind::Orthographic] {
        for preset in [ViewPreset::Iso, ViewPreset::Top, ViewPreset::Front] {
            let cam = fitted(&s, preset, kind);
            let out = render(&s, &opts(cam, RenderMode::FaceIds));
            let img = &out.image;
            let (mut checked, mut agree) = (0, 0);
            for y in (2..img.height - 2).step_by(5) {
                for x in (2..img.width - 2).step_by(5) {
                    let id = img.pixel(x, y);
                    // Skip pixels next to a face boundary, where sampling positions legitimately differ.
                    let uniform = (-2i32..=2).all(|dy| {
                        (-2i32..=2).all(|dx| {
                            img.pixel((x as i32 + dx) as u32, (y as i32 + dy) as u32) == id
                        })
                    });
                    if !uniform {
                        continue;
                    }
                    let picked = pick(
                        &s,
                        &cam,
                        f64::from(x) + 0.5,
                        f64::from(y) + 0.5,
                        f64::from(img.width),
                        f64::from(img.height),
                    );
                    let want = out
                        .legend
                        .iter()
                        .find(|l| l.color == id)
                        .map(|l| l.face.clone());
                    checked += 1;
                    if picked.map(|p| p.face) == want {
                        agree += 1;
                    }
                }
            }
            assert!(
                checked > 300,
                "{preset:?} {kind:?}: only {checked} pixels checked"
            );
            assert_eq!(
                agree,
                checked,
                "{preset:?} {kind:?}: picking and rendering disagree on {} of {checked} pixels",
                checked - agree
            );
        }
    }
}

#[test]
fn picking_reports_where_and_how_far_and_misses_cleanly() {
    let s = box_scene();
    let cam = fitted(&s, ViewPreset::Front, ProjectionKind::Orthographic);
    let p = pick(&s, &cam, 120.0, 90.0, 240.0, 180.0).expect("the centre is on the box");
    assert_eq!((p.body.as_str(), p.face.as_str()), ("box", "side:e0"));
    assert!(
        (p.point.y - 0.0).abs() < 1e-9,
        "front face is at y = 0, got {}",
        p.point.y
    );
    assert!(
        (p.point.x - 40.0).abs() < 1e-6 && (p.point.z - 10.0).abs() < 1e-6,
        "{:?}",
        p.point
    );
    assert!(
        pick(&s, &cam, 2.0, 2.0, 240.0, 180.0).is_none(),
        "the corner is empty space"
    );
}

#[test]
fn a_document_becomes_a_scene_showing_only_final_bodies_and_the_sketches() {
    let mut a = Authority::default();
    let who = Actor::Human("ann".into());
    let mut n = 0;
    let mut run = |a: &mut Authority, kind: K| {
        n += 1;
        a.submit(Command {
            id: format!("c{n}"),
            actor: who.clone(),
            base_revision: a.revision(),
            kind,
        })
        .unwrap();
    };
    run(
        &mut a,
        K::SketchCreate {
            sketch: "s".into(),
            plane: Default::default(),
        },
    );
    for (i, (x, y)) in [(0.0, 0.0), (60.0, 0.0), (60.0, 40.0), (0.0, 40.0)]
        .iter()
        .enumerate()
    {
        run(
            &mut a,
            K::SketchAddEntity {
                sketch: "s".into(),
                entity_id: format!("p{i}"),
                entity: Entity::Point { x: *x, y: *y },
            },
        );
    }
    for i in 0..4 {
        run(
            &mut a,
            K::SketchAddEntity {
                sketch: "s".into(),
                entity_id: format!("e{i}"),
                entity: Entity::Line {
                    a: format!("p{i}"),
                    b: format!("p{}", (i + 1) % 4),
                },
            },
        );
    }
    run(
        &mut a,
        K::FeatureExtrude {
            feature: "base".into(),
            sketch: "s".into(),
            profiles: vec!["profile.e0".into()],
            depth: 10.0,
            reverse: false,
            extent: Default::default(),
        },
    );
    run(
        &mut a,
        K::SketchCreate {
            sketch: "t".into(),
            plane: dc_sketch::PlaneSpec::Face {
                feature: "base".into(),
                face: "cap_end".into(),
                offset: 0.0,
            },
        },
    );
    run(
        &mut a,
        K::SketchAddEntity {
            sketch: "t".into(),
            entity_id: "m".into(),
            entity: Entity::Point { x: 30.0, y: 20.0 },
        },
    );
    run(
        &mut a,
        K::SketchAddEntity {
            sketch: "t".into(),
            entity_id: "c".into(),
            entity: Entity::Circle {
                center: "m".into(),
                radius: 6.0,
            },
        },
    );
    run(
        &mut a,
        K::FeatureCut {
            feature: "hole".into(),
            target: "base".into(),
            sketch: "t".into(),
            profiles: vec!["profile.c".into()],
            depth: None,
            reverse: true,
            extent: Default::default(),
            pattern: None,
        },
    );
    let scene = scene_of_authority(&a);
    assert_eq!(
        scene
            .bodies
            .iter()
            .map(|b| b.id.as_str())
            .collect::<Vec<_>>(),
        ["hole"],
        "the base is a stage of the cut: not drawn twice"
    );
    let ids: Vec<&str> = scene.lines.iter().map(|l| l.id.as_str()).collect();
    assert!(
        ids.contains(&"sketch/s/e0") && ids.contains(&"sketch/t/c"),
        "{ids:?}"
    );
    // The face-attached sketch is drawn on the top face, z = 10.
    let c = scene.lines.iter().find(|l| l.id == "sketch/t/c").unwrap();
    assert!(c.points.iter().all(|p| (p.z - 10.0).abs() < 1e-9));
    // And the whole thing renders with sketches over the faces they sit on.
    let cam = fitted(&scene, ViewPreset::Top, ProjectionKind::Orthographic);
    let mut o = opts(cam, RenderMode::Shaded);
    o.sketches = true;
    let with = render(&scene, &o).image;
    o.sketches = false;
    let without = render(&scene, &o).image;
    assert_ne!(with, without, "sketch lines are visible");
}

/// Writes sample images next to the build output so they can be looked at (and opened with any
/// other PNG reader, which is how the encoder was cross-checked).
#[test]
fn writes_sample_images() {
    let dir = std::path::Path::new(env!("CARGO_TARGET_TMPDIR")).join("render-samples");
    std::fs::create_dir_all(&dir).unwrap();
    let st = Stepped::extrusion(xy(), rect(0.0, 0.0, 100.0, 60.0), 12.0, false)
        .cut(&disc(25.0, 30.0, 9.0), 12.0, 6.0, "pk")
        .unwrap()
        .cut(&disc(25.0, 30.0, 4.0), 6.0, f64::NEG_INFINITY, "thru")
        .unwrap()
        .boss(&rect(50.0, 15.0, 90.0, 45.0), 12.0, 24.0, "bs")
        .unwrap()
        .cut(&disc(70.0, 30.0, 8.0), 24.0, 16.0, "pk2")
        .unwrap();
    let s = Scene {
        bodies: vec![body("part", &st, [122, 162, 214])],
        lines: vec![],
    };
    let cam = fitted(&s, ViewPreset::Iso, ProjectionKind::Perspective);
    let mut o = RenderOptions::new(800, 600, cam);
    o.supersample = 2;
    std::fs::write(dir.join("iso.png"), render(&s, &o).image.to_png()).unwrap();
    o.camera = fitted(&s, ViewPreset::Top, ProjectionKind::Orthographic);
    std::fs::write(dir.join("top.png"), render(&s, &o).image.to_png()).unwrap();
    o.mode = RenderMode::FaceIds;
    o.camera = cam;
    o.supersample = 1;
    std::fs::write(dir.join("faces.png"), render(&s, &o).image.to_png()).unwrap();
    println!("{}", dir.display());
}

/// A wide plate with a block standing on it: the foot of the block is a concave corner.
fn plate_and_block() -> Scene {
    let plate = Stepped::extrusion(xy(), rect(0.0, 0.0, 100.0, 60.0), 10.0, false);
    let tall = plate
        .boss(&rect(30.0, 15.0, 60.0, 45.0), 10.0, 30.0, "b")
        .unwrap();
    Scene {
        bodies: vec![body("part", &tall, [180, 180, 190])],
        lines: vec![],
    }
}

#[test]
fn ambient_occlusion_darkens_concave_corners_only_and_is_off_by_default() {
    let scene = plate_and_block();
    let cam = fitted(&scene, ViewPreset::Iso, ProjectionKind::Orthographic);
    let mut plain = opts(cam, RenderMode::Shaded);
    plain.edges = false;
    let before = render(&scene, &plain).image;
    // Off by default: nothing changes unless asked.
    assert!(!RenderOptions::new(8, 8, cam).ambient_occlusion);
    let mut with = plain.clone();
    with.ambient_occlusion = true;
    let after = render(&scene, &with).image;
    assert_eq!((before.width, before.height), (after.width, after.height));
    let lum = |im: &dc_render::Image, x: u32, y: u32| {
        let p = im.pixel(x, y);
        u32::from(p[0]) + u32::from(p[1]) + u32::from(p[2])
    };
    let (mut darker, mut lighter, mut open_changed) = (0, 0, 0);
    for y in 0..before.height {
        for x in 0..before.width {
            let (a, b) = (lum(&before, x, y), lum(&after, x, y));
            if b + 6 < a {
                darker += 1;
            }
            if b > a + 6 {
                lighter += 1;
            }
            // The background is never touched.
            if before.pixel(x, y) == [236, 238, 242] && a != b {
                open_changed += 1;
            }
        }
    }
    assert!(
        darker > 100,
        "the foot of the block and the plate beside it are darker: {darker}"
    );
    assert_eq!(lighter, 0, "occlusion only ever darkens");
    assert_eq!(open_changed, 0, "background stays");
    // Never below half: every pixel keeps at least half its brightness.
    for y in 0..before.height {
        for x in 0..before.width {
            assert!(lum(&after, x, y) * 2 + 8 >= lum(&before, x, y));
        }
    }
    // A lone flat face has no corner to darken.
    let flat = box_scene();
    let mut o = opts(
        fitted(&flat, ViewPreset::Top, ProjectionKind::Orthographic),
        RenderMode::Shaded,
    );
    o.edges = false;
    let base = render(&flat, &o).image;
    o.ambient_occlusion = true;
    let occluded = render(&flat, &o).image;
    let mid = (base.width / 2, base.height / 2);
    assert_eq!(base.pixel(mid.0, mid.1), occluded.pixel(mid.0, mid.1));
}

#[test]
fn edges_and_sketch_lines_can_be_picked_and_hidden_ones_cannot() {
    use dc_render::{pick_hit, Filter, Hit, Polyline};
    let mut s = box_scene();
    // A sketch line drawn along the top face's front edge region, 5 mm in from it.
    s.lines.push(Polyline {
        id: "sketch/s1/e9".into(),
        points: vec![Vec3::new(10.0, 5.0, 20.0), Vec3::new(70.0, 5.0, 20.0)],
        color: [0, 150, 175],
    });
    let (w, h) = (800.0, 600.0);
    let cam = fitted(&s, ViewPreset::Iso, ProjectionKind::Orthographic);
    let px = |p: Vec3| {
        let q = cam.project(p, w, h).unwrap();
        (q.x, q.y)
    };
    // The middle of the top front edge: the face names are the top cap and the front wall.
    let (x, y) = px(Vec3::new(40.0, 0.0, 20.0));
    match pick_hit(&s, &cam, x, y, w, h, Filter::ALL, 6.0) {
        Some(Hit::Edge { body, name, .. }) => {
            assert_eq!(body, "box");
            assert_eq!(name, "cap_end|side:e0");
        }
        other => panic!("{other:?}"),
    }
    // With edges filtered out the same pixel is a face.
    let only_faces = Filter {
        edges: false,
        sketch_lines: false,
        ..Filter::ALL
    };
    assert!(matches!(
        pick_hit(&s, &cam, x, y, w, h, only_faces, 6.0),
        Some(Hit::Face(_))
    ));
    // The sketch line.
    let (x, y) = px(Vec3::new(40.0, 5.0, 20.0));
    match pick_hit(&s, &cam, x, y, w, h, Filter::ALL, 6.0) {
        Some(Hit::SketchLine { sketch, entity, .. }) => {
            assert_eq!((sketch.as_str(), entity.as_str()), ("s1", "e9"))
        }
        other => panic!("{other:?}"),
    }
    assert!(pick_hit(&s, &cam, x, y, w, h, Filter::EDGES, 6.0).is_none());
    // The middle of a face is a face; an edge on the far side of the box is hidden.
    let (x, y) = px(Vec3::new(40.0, 20.0, 20.0));
    assert!(matches!(
        pick_hit(&s, &cam, x, y, w, h, Filter::ALL, 6.0),
        Some(Hit::Face(_))
    ));
    let (x, y) = px(Vec3::new(40.0, 40.0, 0.0));
    let far = pick_hit(&s, &cam, x, y, w, h, Filter::EDGES, 4.0);
    assert!(
        far.is_none(),
        "an edge behind the part is not pickable: {far:?}"
    );
}

#[test]
fn a_rubber_band_takes_what_is_wholly_inside_or_what_it_touches_and_only_what_is_visible() {
    use dc_render::{box_hits, Filter, Hit, Polyline};
    let mut s = box_scene();
    s.lines.push(Polyline {
        id: "sketch/s1/e9".into(),
        points: vec![Vec3::new(10.0, 5.0, 20.0), Vec3::new(70.0, 5.0, 20.0)],
        color: [0, 150, 175],
    });
    let (w, h) = (800.0, 600.0);
    let cam = fitted(&s, ViewPreset::Iso, ProjectionKind::Orthographic);
    let px = |p: Vec3| {
        let q = cam.project(p, w, h).unwrap();
        (q.x, q.y)
    };
    let names = |hits: &[Hit]| -> Vec<String> {
        let mut v: Vec<String> = hits
            .iter()
            .map(|h| match h {
                Hit::Face(f) => format!("face {}", f.face),
                Hit::Edge { name, .. } => format!("edge {name}"),
                Hit::SketchLine { entity, .. } => format!("line {entity}"),
            })
            .collect();
        v.sort();
        v
    };
    // A box round the whole part: window mode takes the three visible faces, the nine visible
    // edges and the sketch line, but not the three hidden faces or their edges.
    let all = (0.0, 0.0, w, h);
    let hits = box_hits(&s, &cam, all, false, w, h, Filter::ALL);
    let n = names(&hits);
    assert_eq!(
        n.iter().filter(|x| x.starts_with("face")).count(),
        3,
        "{n:?}"
    );
    assert_eq!(
        n.iter().filter(|x| x.starts_with("edge")).count(),
        9,
        "{n:?}"
    );
    assert!(n.contains(&"line e9".to_owned()), "{n:?}");
    assert!(n.contains(&"face cap_end".to_owned()), "{n:?}");
    // A small box on the middle of the top face: nothing is wholly inside it, but touching
    // takes the top face (and nothing else).
    let (cx, cy) = px(Vec3::new(40.0, 20.0, 20.0));
    let small = (cx - 5.0, cy - 5.0, cx + 5.0, cy + 5.0);
    assert!(names(&box_hits(&s, &cam, small, false, w, h, Filter::ALL)).is_empty());
    let touched = names(&box_hits(&s, &cam, small, true, w, h, Filter::ALL));
    assert_eq!(touched, vec!["face cap_end".to_owned()], "{touched:?}");
    // Around the front-top edge only (a thin box): window takes that edge, crossing also the faces it borders.
    let (ex, ey) = px(Vec3::new(40.0, 0.0, 20.0));
    let (ax, ay) = px(Vec3::new(0.0, 0.0, 20.0));
    let (bx, by) = px(Vec3::new(80.0, 0.0, 20.0));
    let thin = (
        ax.min(bx) - 3.0,
        ay.min(by) - 3.0,
        ax.max(bx) + 3.0,
        ay.max(by) + 3.0,
    );
    let _ = (ex, ey);
    let window = names(&box_hits(&s, &cam, thin, false, w, h, Filter::EDGES));
    assert!(
        window.contains(&"edge cap_end|side:e0".to_owned()),
        "{window:?}"
    );
    // The filter limits what is offered.
    let faces_only = Filter {
        edges: false,
        sketch_lines: false,
        ..Filter::ALL
    };
    let f = names(&box_hits(&s, &cam, all, false, w, h, faces_only));
    assert!(
        f.iter().all(|x| x.starts_with("face")) && f.len() == 3,
        "{f:?}"
    );
    // A box on empty space takes nothing.
    assert!(box_hits(&s, &cam, (0.0, 0.0, 5.0, 5.0), true, w, h, Filter::ALL).is_empty());
}
