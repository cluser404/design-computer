//! The Extrude / Cut / Boss dialogs: the draft previews what it would build, changes nothing
//! until accepted, and accepting is one command.

use dc_app::featuring::FeatureKind;
use dc_app::sketching::Tool;
use dc_app::state::{AppState, Selected};
use dc_sketch::PlaneSpec;

const W: f64 = 800.0;
const H: f64 = 600.0;

fn click(a: &mut AppState, p: (f64, f64)) {
    let (x, y) = a.sketch_to_screen(p, W, H).expect("on screen");
    a.sketch_hover(Some((x, y, W, H)));
    a.sketch_click(x, y, W, H);
}

/// A sketch with a 40 x 20 rectangle and a circle inside it, left finished.
fn a_plate_sketch() -> AppState {
    let mut a = AppState::new();
    a.begin_sketch(PlaneSpec::Xy { offset: 0.0 }).unwrap();
    a.sketching.as_mut().unwrap().snap_grid = false;
    a.set_tool(Tool::Rectangle);
    click(&mut a, (0.0, 0.0));
    click(&mut a, (40.0, 20.0));
    a.set_tool(Tool::Circle);
    click(&mut a, (20.0, 10.0));
    click(&mut a, (25.0, 10.0));
    a.end_sketch();
    a
}

fn volume(a: &AppState, feature: &str) -> f64 {
    a.feature_rows()
        .into_iter()
        .find(|r| r.id == feature)
        .map(|r| r.volume)
        .unwrap_or(f64::NAN)
}

#[test]
fn an_extrude_draft_previews_without_touching_the_document_and_commits_as_one_command() {
    let mut a = a_plate_sketch();
    let rev = a.authority.revision();
    assert!(a.begin_feature(FeatureKind::Extrude));
    let d = a.feature_draft.clone().unwrap();
    assert_eq!(d.name, "extrude1");
    assert!(d.profiles.is_empty(), "no region named means every region");
    // The preview shows a body that the document does not have yet.
    assert_eq!(a.authority.revision(), rev, "a draft logs nothing");
    assert!(a.feature_rows().is_empty());
    assert_eq!(a.scene.bodies.len(), 1, "the drafted part is drawn");
    assert!(a.draft_problem().is_none());
    // Changing the depth changes the preview; the numbers match the real feature after accepting.
    a.feature_draft.as_mut().unwrap().depth = 8.0;
    a.update_feature_preview();
    let previewed = a.scene.bodies[0]
        .mesh
        .positions
        .iter()
        .map(|p| p.z)
        .fold(0.0, f64::max);
    assert!(
        (previewed - 8.0).abs() < 1e-9,
        "the preview is 8 mm tall: {previewed}"
    );
    assert!(a.commit_feature_draft());
    assert_eq!(
        a.authority.revision().0,
        rev.0 + 1,
        "one command, one undo step"
    );
    assert!(a.feature_draft.is_none() && a.feature_preview.is_none());
    assert_eq!(a.selected, Selected::Feature("extrude1".into()));
    // 40 x 20 x 8 (the circle is a hole of the profile that holds it, not a second profile
    // unless it is its own).
    assert!(volume(&a, "extrude1") > 0.0);
    a.undo().unwrap();
    assert!(a.feature_rows().is_empty());
}

#[test]
fn a_draft_that_cannot_be_built_says_why_and_cancelling_restores_the_scene() {
    let mut a = a_plate_sketch();
    assert!(a.begin_feature(FeatureKind::Extrude));
    a.feature_draft.as_mut().unwrap().depth = 0.0;
    a.update_feature_preview();
    assert!(a.draft_problem().unwrap().contains("depth"));
    assert!(
        !a.commit_feature_draft(),
        "an impossible draft is not committed"
    );
    assert!(a.feature_draft.is_some());
    let n = a.scene.bodies.len();
    a.cancel_feature_draft();
    assert!(a.feature_draft.is_none());
    assert!(a.scene.bodies.len() <= n);
    assert!(
        a.scene.bodies.is_empty(),
        "no part exists, so nothing is drawn"
    );
    assert!(a.feature_rows().is_empty());
}

#[test]
fn cut_and_boss_need_a_part_and_work_on_the_latest_one() {
    let mut a = AppState::new();
    a.begin_sketch(PlaneSpec::Xy { offset: 0.0 }).unwrap();
    a.sketching.as_mut().unwrap().snap_grid = false;
    a.set_tool(Tool::Rectangle);
    click(&mut a, (0.0, 0.0));
    click(&mut a, (40.0, 20.0));
    // Still sketching: Cut and Boss have no part yet.
    assert!(!a.begin_feature(FeatureKind::Cut));
    assert!(!a.begin_feature(FeatureKind::Boss));
    // Starting a feature leaves the sketch and makes the plate.
    assert!(a.begin_feature(FeatureKind::Extrude));
    assert!(a.sketching.is_none());
    a.feature_draft.as_mut().unwrap().depth = 10.0;
    assert!(a.commit_feature_draft());
    let plate = volume(&a, "extrude1");
    // A hole sketched on the same plane.
    a.begin_sketch(PlaneSpec::Xy { offset: 0.0 }).unwrap();
    a.sketching.as_mut().unwrap().snap_grid = false;
    a.set_tool(Tool::Circle);
    click(&mut a, (20.0, 10.0));
    click(&mut a, (25.0, 10.0));
    assert!(a.begin_feature(FeatureKind::Cut));
    let d = a.feature_draft.clone().unwrap();
    assert_eq!(d.target.as_deref(), Some("extrude1"));
    assert_eq!(d.sketch, "sketch2");
    a.feature_draft.as_mut().unwrap().end = dc_app::featuring::EndCondition::ThroughAll;
    a.update_feature_preview();
    assert!(a.draft_problem().is_none(), "{:?}", a.draft_problem());
    let rev = a.authority.revision();
    assert!(a.commit_feature_draft());
    assert_eq!(a.authority.revision().0, rev.0 + 1);
    let cut = volume(&a, "cut1");
    assert!(cut < plate, "material was removed: {cut} < {plate}");
    // A cut that misses the part is explained, not committed.
    assert!(a.begin_feature(FeatureKind::Cut));
    a.feature_draft.as_mut().unwrap().depth = 5.0;
    a.feature_draft.as_mut().unwrap().end = dc_app::featuring::EndCondition::Blind;
    a.feature_draft.as_mut().unwrap().reverse = true;
    a.update_feature_preview();
    let why = a
        .draft_problem()
        .expect("a cut below the part must be explained");
    assert!(why.contains("reverse") || why.contains("part"), "{why}");
    assert!(!a.commit_feature_draft());
    assert!(a.authority.state().feature("cut2").is_none());
}

#[test]
fn the_sketch_choice_resets_the_profile_and_a_sketch_without_a_closed_shape_is_refused() {
    let mut a = a_plate_sketch();
    // A second sketch with only an open line.
    a.begin_sketch(PlaneSpec::Xy { offset: 5.0 }).unwrap();
    a.sketching.as_mut().unwrap().snap_grid = false;
    a.set_tool(Tool::Line);
    click(&mut a, (0.0, 0.0));
    click(&mut a, (10.0, 5.0));
    a.end_sketch();
    assert_eq!(a.extrudable_sketches(), vec!["sketch1".to_owned()]);
    assert!(a.begin_feature(FeatureKind::Extrude));
    assert_eq!(a.feature_draft.as_ref().unwrap().sketch, "sketch1");
    a.cancel_feature_draft();
    // Selecting the open sketch does not make it usable.
    a.select(Selected::Sketch("sketch2".into()));
    assert!(a.begin_feature(FeatureKind::Extrude));
    assert_eq!(a.feature_draft.as_ref().unwrap().sketch, "sketch1");
}

fn z_range(a: &AppState) -> (f64, f64) {
    let zs = a.scene.bodies[0].mesh.positions.iter().map(|p| p.z);
    (
        zs.clone().fold(f64::MAX, f64::min),
        zs.fold(f64::MIN, f64::max),
    )
}

#[test]
fn the_dialog_offers_mid_plane_a_second_direction_and_the_cut_end_conditions() {
    use dc_app::featuring::EndCondition;
    let mut a = a_plate_sketch();
    assert!(a.begin_feature(FeatureKind::Extrude));
    // Cut-only end conditions are not offered to an extrusion.
    assert!(!EndCondition::for_kind(FeatureKind::Extrude).contains(&EndCondition::ThroughAll));
    assert!(EndCondition::for_kind(FeatureKind::Cut).contains(&EndCondition::ThroughBoth));
    let rect = a
        .profiles_of("sketch1")
        .into_iter()
        .max_by(|x, y| x.area.total_cmp(&y.area))
        .unwrap();
    {
        let d = a.feature_draft.as_mut().unwrap();
        d.profiles = vec![rect.id.clone()];
        d.depth = 10.0;
        d.end = EndCondition::MidPlane;
    }
    a.update_feature_preview();
    let (lo, hi) = z_range(&a);
    assert!(
        (lo + 5.0).abs() < 1e-9 && (hi - 5.0).abs() < 1e-9,
        "{lo} {hi}"
    );
    // A second direction instead.
    {
        let d = a.feature_draft.as_mut().unwrap();
        d.end = EndCondition::Blind;
        d.second = true;
        d.depth2 = 3.0;
    }
    a.update_feature_preview();
    let (lo, hi) = z_range(&a);
    assert!(
        (lo + 3.0).abs() < 1e-9 && (hi - 10.0).abs() < 1e-9,
        "{lo} {hi}"
    );
    // A bad second depth is explained.
    a.feature_draft.as_mut().unwrap().depth2 = 0.0;
    a.update_feature_preview();
    assert!(a.draft_problem().unwrap().contains("second direction"));
    a.feature_draft.as_mut().unwrap().depth2 = 3.0;
    a.update_feature_preview();
    assert!(a.commit_feature_draft());
    let spec = a
        .authority
        .state()
        .feature("extrude1")
        .unwrap()
        .spec
        .clone();
    assert!(serde_json::to_string(&spec).unwrap().contains("depth2"));
}

#[test]
fn the_model_origin_shows_by_default_and_a_sketch_marks_its_own_origin_with_the_grid_off_until_asked(
) {
    let mut a = AppState::new();
    assert!(
        a.show_origins,
        "the model origin is shown, as in SolidWorks"
    );
    assert!(!a.show_sketch_grid, "no grid until switched on");
    assert!(a.local_grid().is_none(), "nothing to sketch on");
    let part = a.render(120, 90, 1);
    a.show_origins = false;
    assert_ne!(
        part.rgb,
        a.render(120, 90, 1).rgb,
        "the origin triad is drawn"
    );
    a.show_origins = true;
    // A sketch on the XZ plane, 7 mm out: its origin marker and grid lie in that plane.
    a.begin_sketch(PlaneSpec::Xz { offset: 7.0 }).unwrap();
    let g = a.local_grid().expect("the sketch plane is marked");
    assert!(!g.grid, "the grid is off by default");
    let n = g.x.cross(g.y);
    assert!(n.z.abs() < 1e-9, "the plane is vertical: normal {n:?}");
    assert!(
        n.dot(g.origin).abs() > 6.9,
        "and sits 7 mm from the world origin: {g:?}"
    );
    assert_eq!(
        g.origin_marker(2.0).len(),
        6,
        "two arrows, a shaft and two barbs each"
    );
    let marked = a.render(120, 90, 1);
    a.show_sketch_grid = true;
    assert!(a.local_grid().unwrap().grid);
    assert_ne!(marked.rgb, a.render(120, 90, 1).rgb, "the grid adds lines");
    a.end_sketch();
    assert!(
        a.local_grid().is_none(),
        "leaving the sketch leaves its marker and grid"
    );
}

#[test]
fn a_new_part_opens_isometric_and_orthographic_and_perspective_is_a_switch() {
    use dc_render::{Camera, ProjectionKind, ViewPreset};
    let mut a = AppState::new();
    let iso = Camera::preset(ViewPreset::Iso);
    assert_eq!(
        (a.camera.yaw, a.camera.pitch),
        (iso.yaw, iso.pitch),
        "isometric"
    );
    assert_eq!(
        a.camera.projection,
        ProjectionKind::Orthographic,
        "no perspective by default"
    );
    a.toggle_projection();
    assert_eq!(a.camera.projection, ProjectionKind::Perspective);
    // Standard views keep the chosen projection; the other way too.
    a.set_view(ViewPreset::Front);
    assert_eq!(a.camera.projection, ProjectionKind::Perspective);
    a.toggle_projection();
    a.set_view(ViewPreset::Iso);
    assert_eq!(a.camera.projection, ProjectionKind::Orthographic);
    // Sketching turns square-on and leaves the view as it was.
    a.toggle_projection();
    a.begin_sketch(PlaneSpec::Xy { offset: 0.0 }).unwrap();
    a.end_sketch();
    assert_eq!(
        a.camera.projection,
        ProjectionKind::Perspective,
        "restored on leaving the sketch"
    );
}

#[test]
fn a_sketch_a_feature_was_made_from_is_put_away_until_it_is_selected_or_edited() {
    let mut a = a_plate_sketch();
    // Nothing uses the sketch yet: its outline is drawn.
    a.refresh_scene();
    assert!(!a.scene.lines.is_empty(), "an unused sketch shows");
    assert!(a.begin_feature(FeatureKind::Extrude));
    assert!(a.commit_feature_draft());
    a.refresh_scene();
    assert!(a.scene.lines.is_empty(), "used by a feature: put away");
    // Selecting it brings it back; selecting something else puts it away again.
    a.select(Selected::Sketch("sketch1".into()));
    a.refresh_scene();
    assert!(!a.scene.lines.is_empty());
    a.select(Selected::None);
    a.refresh_scene();
    assert!(a.scene.lines.is_empty());
    // Editing it shows it too.
    assert!(a.edit_sketch("sketch1"));
    a.refresh_scene();
    assert!(!a.scene.lines.is_empty());
}

#[test]
fn clicking_inside_a_region_of_the_sketch_in_the_3d_view_picks_that_profile_for_the_draft() {
    // A 40 x 20 plate with a round hole in it, and a separate disc beside it.
    let mut a = a_plate_sketch();
    a.begin_sketch(PlaneSpec::Xy { offset: 0.0 }).unwrap();
    a.edit_sketch("sketch1");
    a.sketching.as_mut().unwrap().snap_grid = false;
    a.set_tool(Tool::Circle);
    click(&mut a, (60.0, 10.0));
    click(&mut a, (65.0, 10.0));
    a.end_sketch();
    assert!(a.begin_feature(FeatureKind::Extrude));
    let profiles = a.profiles_of("sketch1");
    assert_eq!(
        profiles.len(),
        2,
        "the plate with its hole, and the disc: {profiles:?}"
    );
    let plate = profiles
        .iter()
        .find(|p| !p.holes.is_empty())
        .unwrap()
        .id
        .clone();
    let disc = profiles
        .iter()
        .find(|p| p.holes.is_empty())
        .unwrap()
        .id
        .clone();
    a.set_view(dc_render::ViewPreset::Iso);
    let (w, h) = (800.0, 600.0);
    let screen = |a: &AppState, p: (f64, f64)| {
        let plane = a.authority.state().sketch_plane("sketch1").unwrap();
        let q = a
            .camera
            .project(plane.to_3d(dc_math::Vec2::new(p.0, p.1)), w, h)
            .expect("on screen");
        (q.x, q.y)
    };
    // The disc, then the plate (beside its hole), each picked by clicking inside it.
    let (x, y) = screen(&a, (60.0, 10.0));
    assert!(a.draft_click(x, y, w, h));
    assert_eq!(
        a.feature_draft.as_ref().unwrap().profiles,
        std::slice::from_ref(&disc)
    );
    // A second region is added to the choice.
    let (x, y) = screen(&a, (5.0, 4.0));
    assert!(a.draft_click(x, y, w, h));
    assert_eq!(
        a.feature_draft.as_ref().unwrap().profiles,
        [disc.clone(), plate.clone()]
    );
    // Clicking a chosen region again takes it out; the last one stays.
    let (x, y) = screen(&a, (60.0, 10.0));
    assert!(a.draft_click(x, y, w, h));
    assert_eq!(
        a.feature_draft.as_ref().unwrap().profiles,
        std::slice::from_ref(&plate)
    );
    let (x, y) = screen(&a, (5.0, 4.0));
    assert!(a.draft_click(x, y, w, h));
    assert_eq!(
        a.feature_draft.as_ref().unwrap().profiles,
        std::slice::from_ref(&plate)
    );
    // The hole is not part of the plate's region: a click there picks nothing, and so does
    // empty space; the draft keeps what it had.
    for p in [(20.0, 10.0), (90.0, 40.0)] {
        let (x, y) = screen(&a, p);
        assert!(!a.draft_click(x, y, w, h), "{p:?}");
        assert_eq!(
            a.feature_draft.as_ref().unwrap().profiles,
            std::slice::from_ref(&plate)
        );
    }
    // The ordinary click routes through the same pick, and logs nothing.
    let rev = a.authority.revision();
    let (x, y) = screen(&a, (60.0, 10.0));
    a.click_at(x, y, w, h);
    assert_eq!(
        a.feature_draft.as_ref().unwrap().profiles,
        [plate.clone(), disc.clone()]
    );
    assert_eq!(a.authority.revision(), rev);
}
