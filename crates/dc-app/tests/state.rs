//! The app's behaviour, tested without a window.

use dc_app::state::{AppState, Level, Selected};
use dc_doc::{CommandKind, DiagCode};
use dc_render::{ProjectionKind, ViewPreset};

fn demo() -> AppState {
    let mut s = AppState::new();
    s.load_demo();
    s
}
fn tmp(name: &str) -> std::path::PathBuf {
    let d = std::env::temp_dir().join(format!("dc-app-test-{}-{name}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    d
}
fn volume(s: &AppState, f: &str) -> f64 {
    s.feature_rows()
        .into_iter()
        .find(|r| r.id == f)
        .unwrap()
        .volume
}

#[test]
fn a_fresh_app_is_empty_and_still_renders() {
    let s = AppState::new();
    assert!(s.scene.bodies.is_empty() && s.sketch_rows().is_empty() && s.feature_rows().is_empty());
    assert!(!s.dirty());
    let img = s.render(160, 120, 1);
    assert_eq!(img.rgb.len(), 160 * 120 * 3);
}

#[test]
fn the_demo_is_built_through_ordinary_commands() {
    let s = demo();
    let ids: Vec<_> = s.feature_rows().into_iter().map(|r| r.id).collect();
    assert_eq!(ids, ["boss", "hole", "plate", "pocket_cut"]);
    assert_eq!(
        s.scene
            .bodies
            .iter()
            .map(|b| b.id.as_str())
            .collect::<Vec<_>>(),
        ["hole"],
        "only the final body is drawn"
    );
    assert!(
        s.authority.revision().0 >= 4,
        "every step is in the history"
    );
    assert!(s.dirty(), "an unsaved document is dirty");
    // Sketches, including three placed on faces, are all there and solved.
    let sketches = s.sketch_rows();
    assert_eq!(sketches.len(), 4);
    assert!(sketches
        .iter()
        .all(|r| r.info.status == dc_sketch::SolveStatus::Solved));
    // The camera was fitted around the part.
    let (lo, hi) = s.scene.body_bounds().unwrap();
    let c = s.camera.target;
    assert!(c.x > lo.x && c.x < hi.x && c.y > lo.y && c.y < hi.y);
    assert!(s.messages.iter().all(|m| m.level == Level::Info));
}

#[test]
fn edits_are_commands_and_undo_restores_the_exact_state() {
    let mut s = demo();
    let h0 = s.authority.state_hash();
    let v0 = volume(&s, "plate");
    s.set_depth("plate", 20.0).unwrap();
    assert!(
        (volume(&s, "plate") - v0).abs() > 100.0,
        "the plate got thicker"
    );
    // Everything stacked on the plate followed it up (the boss and pocket ride on its top face).
    let (_, hi) = s.scene.body_bounds().unwrap();
    assert!(
        (hi.z - (20.0 + 14.0)).abs() < 1e-9,
        "top of the boss is now at {}",
        hi.z
    );
    s.set_dimension("base", "width", 130.0).unwrap();
    assert!(s
        .authority
        .log()
        .iter()
        .any(|e| matches!(e.command.kind, CommandKind::SketchSetValue { .. })));
    s.undo().unwrap();
    s.undo().unwrap();
    assert_eq!(
        s.authority.state_hash(),
        h0,
        "two undos give back the bit-identical document"
    );
    s.redo().unwrap();
    assert_ne!(s.authority.state_hash(), h0);
    assert_eq!(
        s.authority.log().last().unwrap().command.actor,
        s.actor,
        "attributed to the human"
    );
}

#[test]
fn a_rejected_edit_becomes_a_readable_message_and_changes_nothing() {
    let mut s = demo();
    let h = s.authority.state_hash();
    let n = s.messages.len();
    let e = s.set_depth("plate", 0.0).unwrap_err();
    assert_eq!(e.code, DiagCode::InvalidValue);
    let m = s.messages.last().unwrap();
    assert_eq!((m.level, s.messages.len()), (Level::Error, n + 1));
    assert!(m.text.contains("depth"), "{}", m.text);
    assert_eq!(s.authority.state_hash(), h);
    // Deleting something other things stand on explains what, and offers a way forward.
    s.select(Selected::Feature("plate".into()));
    assert_eq!(s.delete_selected().unwrap_err().code, DiagCode::InUse);
    let m = s.messages.last().unwrap();
    assert!(
        m.text.contains("feature/"),
        "names what depends on it: {}",
        m.text
    );
    assert!(m.hint.is_some());
    // A leaf can go, and the selection does not dangle afterwards.
    s.select(Selected::Feature("hole".into()));
    s.delete_selected().unwrap();
    assert_eq!(s.selected, Selected::None);
    assert_eq!(
        s.scene
            .bodies
            .iter()
            .map(|b| b.id.as_str())
            .collect::<Vec<_>>(),
        ["pocket_cut"],
        "the previous stage is now the final body"
    );
}

#[test]
fn dirty_follows_the_content_not_the_history() {
    let mut s = demo();
    let dir = tmp("dirty");
    let path = dir.join("part.dcad");
    s.save(&path).unwrap();
    assert!(!s.dirty());
    s.set_depth("plate", 18.0).unwrap();
    assert!(s.dirty());
    s.undo().unwrap();
    assert!(
        !s.dirty(),
        "undoing back to the saved content is clean again"
    );
}

#[test]
fn the_console_takes_the_same_json_an_agent_sends() {
    let mut s = AppState::new();
    s.run_console(r#"{"type":"param.set","name":"x","value":3}"#);
    assert_eq!(s.authority.state().get("x"), Some(3.0));
    s.run_console("undo");
    assert_eq!(s.authority.state().get("x"), None);
    s.run_console("redo");
    assert_eq!(s.authority.state().get("x"), Some(3.0));
    let n = s.messages.len();
    s.run_console("this is not json");
    let m = s.messages.last().unwrap();
    assert_eq!((m.level, s.messages.len()), (Level::Error, n + 1));
    assert!(m.hint.as_deref().unwrap().contains("param.set"));
    s.run_console(r#"{"type":"param.set","name":"bad name!","value":1}"#);
    assert_eq!(
        s.messages.last().unwrap().level,
        Level::Error,
        "engine errors surface too"
    );
    s.run_console("");
    s.run_console("fit");
}

#[test]
fn clicking_selects_the_face_under_the_cursor() {
    let mut s = demo();
    s.set_view(ViewPreset::Top);
    s.camera.projection = ProjectionKind::Orthographic;
    let (w, h) = (400.0, 300.0);
    s.click_at(w / 2.0, h / 2.0, w, h);
    match &s.selected {
        Selected::Face { feature, face } => {
            assert_eq!((feature.as_str(), face.as_str()), ("hole", "cap_end"))
        }
        other => panic!("{other:?}"),
    }
    assert!(s.selected_face_ref().is_some());
    // The highlight shows up in the render.
    let with = s.render(400, 300, 1);
    s.select(Selected::None);
    assert_ne!(with, s.render(400, 300, 1));
    s.click_at(2.0, 2.0, w, h);
    assert_eq!(
        s.selected,
        Selected::None,
        "clicking empty space clears the selection"
    );
}

#[test]
fn hovering_only_bumps_the_view_when_something_changed() {
    let mut s = demo();
    s.set_view(ViewPreset::Top);
    let (w, h) = (400.0, 300.0);
    s.hover_at(Some((w / 2.0, h / 2.0, w, h)));
    let v = s.view_version;
    assert!(s.hovered.is_some());
    s.hover_at(Some((w / 2.0 + 1.0, h / 2.0, w, h)));
    assert_eq!(
        s.view_version, v,
        "same face under the cursor: no redraw needed"
    );
    s.hover_at(None);
    assert!(s.hovered.is_none() && s.view_version > v);
}

#[test]
fn camera_gestures_have_the_conventional_directions() {
    let mut s = demo();
    s.set_view(ViewPreset::Front);
    let (yaw, pitch) = (s.camera.yaw, s.camera.pitch);
    s.orbit_px(100.0, 0.0);
    assert!(
        s.camera.yaw < yaw,
        "dragging right swings the eye to the left, so the part turns right"
    );
    s.orbit_px(0.0, 50.0);
    assert!(
        s.camera.pitch > pitch,
        "dragging down brings the top toward you"
    );
    s.orbit_px(0.0, 1e6);
    assert_eq!(
        s.camera.pitch,
        core::f64::consts::FRAC_PI_2,
        "no flipping over the pole"
    );
    let d = s.camera.distance;
    s.zoom_scroll(120.0);
    assert!(s.camera.distance < d, "scrolling up zooms in");
    s.zoom_scroll(-120.0);
    assert!((s.camera.distance - d).abs() < 1e-9, "and back out exactly");
    let t = s.camera.target;
    s.pan_px(30.0, -20.0, 600.0);
    assert!((s.camera.target - t).length() > 0.0);
    s.camera.projection = ProjectionKind::Orthographic;
    s.set_view(ViewPreset::Right);
    assert_eq!(
        s.camera.projection,
        ProjectionKind::Orthographic,
        "changing view keeps the projection"
    );
    s.toggle_projection();
    assert_eq!(s.camera.projection, ProjectionKind::Perspective);
}

#[test]
fn files_round_trip_and_bad_files_are_explained() {
    let mut s = demo();
    let dir = tmp("files");
    let path = dir.join("part.dcad");
    s.save(&path).unwrap();
    assert!(!dir.join("part.dcad.tmp").exists());
    let hash = s.authority.state_hash();
    let mut t = AppState::new();
    t.open(&path).unwrap();
    assert_eq!(t.authority.state_hash(), hash);
    assert!(!t.dirty() && t.path.as_deref() == Some(path.as_path()));
    assert_eq!(t.scene.bodies.len(), 1, "the scene is rebuilt on open");
    // The opened document is fully live: undo works because the history came with it.
    t.undo().unwrap();
    assert_ne!(t.authority.state_hash(), hash);
    // Errors say what is wrong and where.
    let e = t.open(&dir.join("missing.dcad")).unwrap_err();
    assert!(e.contains("missing.dcad"), "{e}");
    let bad = dir.join("bad.dcad");
    std::fs::write(&bad, b"not a zip at all").unwrap();
    assert!(t.open(&bad).unwrap_err().contains("bad.dcad"));
    assert_ne!(
        t.authority.state_hash(),
        hash,
        "a failed open leaves the current document alone"
    );
    // Unreadable save target.
    assert!(t.save(&dir.join("no/such/dir/x.dcad")).is_err());
}

#[test]
fn the_panels_read_everything_through_queries() {
    let s = demo();
    let rows = s.feature_rows();
    let boss = rows.iter().find(|r| r.id == "boss").unwrap();
    assert_eq!(
        (boss.kind.as_str(), boss.target.as_deref(), boss.depth),
        ("boss", Some("plate"), Some(14.0))
    );
    let hole = rows.iter().find(|r| r.id == "hole").unwrap();
    assert_eq!(
        (hole.target.as_deref(), hole.depth),
        (Some("pocket_cut"), None),
        "through-all has no depth"
    );
    let faces = s.faces_of("hole");
    assert!(
        faces.iter().any(|f| f.name == "cap_end") && faces.iter().any(|f| f.radius == Some(7.0))
    );
    let cons = s.constraints_of("base");
    assert!(cons
        .iter()
        .any(|(id, c)| id == "width" && c.value() == Some(100.0)));
}

#[test]
fn the_view_stays_fitted_to_the_window_until_the_user_moves_the_camera() {
    let mut s = demo();
    let corners = |s: &AppState, w: f64, h: f64| {
        let (lo, hi) = s.scene.body_bounds().unwrap();
        let mut all_in = true;
        for &x in &[lo.x, hi.x] {
            for &y in &[lo.y, hi.y] {
                for &z in &[lo.z, hi.z] {
                    let p = s.camera.project(dc_math::Vec3::new(x, y, z), w, h).unwrap();
                    all_in &= (0.0..=w).contains(&p.x) && (0.0..=h).contains(&p.y);
                }
            }
        }
        all_in
    };
    // The window opens wide, then is resized to tall and narrow: the part still fits both times.
    s.set_viewport_aspect(1.6);
    assert!(corners(&s, 800.0, 500.0));
    s.set_viewport_aspect(0.45);
    assert!(
        corners(&s, 450.0, 1000.0),
        "resizing refits while the user has not touched the camera"
    );
    // Once the user orbits, their camera is respected.
    s.orbit_px(30.0, 0.0);
    let cam = s.camera;
    s.set_viewport_aspect(2.0);
    assert_eq!(s.camera, cam, "no refit after the user took over");
    // Fit (or choosing a view) hands control back to auto-fit.
    s.fit();
    s.set_viewport_aspect(0.8);
    assert!(corners(&s, 800.0, 1000.0));
}

#[test]
fn the_showcase_part_builds_from_ordinary_commands_and_has_the_things_worth_looking_at() {
    let mut s = AppState::new();
    s.load_showcase();
    let last = s
        .messages
        .last()
        .map(|m| m.text.clone())
        .unwrap_or_default();
    assert!(
        last.contains("loaded the showcase"),
        "{:?}",
        s.messages
            .iter()
            .map(|m| (m.text.clone(), m.hint.clone()))
            .collect::<Vec<_>>()
    );
    let ids: Vec<_> = s.feature_rows().into_iter().map(|r| r.id).collect();
    for want in [
        "plate",
        "cylinder",
        "step1",
        "step2",
        "rib",
        "bore",
        "pocket",
        "hole_a",
        "hole_b",
        "hole_c",
        "cbore",
        "cbore_hole",
        "step_hole",
    ] {
        assert!(ids.iter().any(|i| i == want), "{want} in {ids:?}");
    }
    // One body (the result), 140 x 90 on the plate, 14 + 30 high at the cylinder.
    assert_eq!(s.scene.bodies.len(), 1);
    let (lo, hi) = s.scene.body_bounds().unwrap();
    assert!((hi.x - lo.x - 140.0).abs() < 1e-6 && (hi.y - lo.y - 90.0).abs() < 1e-6);
    assert!((hi.z - lo.z - 44.0).abs() < 1e-6, "{lo:?} {hi:?}");
    // It renders, and it is not the small demo.
    assert!(s.scene.bodies[0].mesh.triangles.len() > 500);
    assert_eq!(s.render(120, 90, 1).rgb.len(), 120 * 90 * 3);
}
