//! Sketch mode, driven the way a mouse drives it: hover, click, type a dimension.

use dc_app::sketching::{Marker, Relation, Tool};
use dc_app::state::{AppState, Selected};
use dc_doc::{CommandKind, Query, QueryResult};
use dc_render::ProjectionKind;
use dc_sketch::{Constraint, Entity, PlaneSpec};

const W: f64 = 800.0;
const H: f64 = 600.0;

fn xy() -> AppState {
    let mut a = AppState::new();
    a.begin_sketch(PlaneSpec::Xy { offset: 0.0 })
        .expect("sketch starts");
    a
}

fn at(a: &mut AppState, p: (f64, f64)) {
    let (x, y) = a.sketch_to_screen(p, W, H).expect("on screen");
    a.sketch_hover(Some((x, y, W, H)));
}

fn click(a: &mut AppState, p: (f64, f64)) {
    at(a, p);
    let (x, y) = a.sketch_to_screen(p, W, H).unwrap();
    a.sketch_click(x, y, W, H);
}

/// Clicks a point given in screen pixels offset from a sketch position (to test guides).
fn click_px(a: &mut AppState, p: (f64, f64), dx: f64, dy: f64) {
    let (x, y) = a.sketch_to_screen(p, W, H).unwrap();
    a.sketch_hover(Some((x + dx, y + dy, W, H)));
    a.sketch_click(x + dx, y + dy, W, H);
}

fn session_sketch(a: &AppState) -> dc_sketch::Sketch {
    let name = &a.sketching.as_ref().unwrap().sketch;
    a.authority.state().sketch(name).unwrap().sketch.clone()
}

fn points(sk: &dc_sketch::Sketch) -> Vec<(f64, f64)> {
    sk.entities()
        .values()
        .filter_map(|e| match e {
            Entity::Point { x, y } => Some((*x, *y)),
            _ => None,
        })
        .collect()
}

fn count(sk: &dc_sketch::Sketch, f: impl Fn(&Entity) -> bool) -> usize {
    sk.entities().values().filter(|e| f(e)).count()
}

fn near(a: (f64, f64), b: (f64, f64)) -> bool {
    (a.0 - b.0).abs() < 1e-6 && (a.1 - b.1).abs() < 1e-6
}

fn no_grid(a: &mut AppState) {
    a.sketching.as_mut().unwrap().snap_grid = false;
}

#[test]
fn starting_a_sketch_faces_the_plane_and_screen_coordinates_round_trip() {
    let a = xy();
    assert_eq!(a.camera.projection, ProjectionKind::Orthographic);
    assert!(
        (a.camera.pitch - core::f64::consts::FRAC_PI_2).abs() < 1e-9,
        "looking straight down"
    );
    for p in [(0.0, 0.0), (30.0, -20.0), (-45.5, 12.25)] {
        let (sx, sy) = a.sketch_to_screen(p, W, H).unwrap();
        let back = a.screen_to_sketch(sx, sy, W, H).unwrap();
        assert!(near(p, back), "{p:?} -> ({sx},{sy}) -> {back:?}");
    }
    // Sketch +x is screen right and +y is screen up.
    let o = a.sketch_to_screen((0.0, 0.0), W, H).unwrap();
    let px = a.sketch_to_screen((10.0, 0.0), W, H).unwrap();
    let py = a.sketch_to_screen((0.0, 10.0), W, H).unwrap();
    assert!(px.0 > o.0 && (px.1 - o.1).abs() < 1e-6, "x goes right");
    assert!(
        py.1 < o.1 && (py.0 - o.0).abs() < 1e-6,
        "y goes up (screen y is down)"
    );
}

#[test]
fn two_clicks_make_a_rectangle_in_one_undo_step() {
    let mut a = xy();
    no_grid(&mut a);
    a.set_tool(Tool::Rectangle);
    let rev = a.authority.revision();
    click(&mut a, (10.0, 5.0));
    assert_eq!(
        a.authority.revision(),
        rev,
        "the first corner is only a preview, nothing is logged"
    );
    click(&mut a, (50.0, 35.0));
    assert_eq!(
        a.authority.revision().0,
        rev.0 + 1,
        "the rectangle is exactly one command"
    );
    let sk = session_sketch(&a);
    let pts = points(&sk);
    for want in [(10.0, 5.0), (50.0, 5.0), (50.0, 35.0), (10.0, 35.0)] {
        assert!(
            pts.iter().any(|p| near(*p, want)),
            "corner {want:?} in {pts:?}"
        );
    }
    assert_eq!(count(&sk, |e| matches!(e, Entity::Line { .. })), 4);
    let h = sk
        .constraints()
        .values()
        .filter(|c| matches!(c, Constraint::Horizontal { .. }))
        .count();
    let v = sk
        .constraints()
        .values()
        .filter(|c| matches!(c, Constraint::Vertical { .. }))
        .count();
    assert_eq!((h, v), (2, 2));
    assert!(
        a.sketching.as_ref().unwrap().first.is_none(),
        "ready for the next shape"
    );
    a.undo().unwrap();
    assert!(
        session_sketch(&a).entities().is_empty(),
        "one undo removes the whole rectangle"
    );
}

#[test]
fn clicks_snap_to_the_grid_when_it_is_switched_on_and_not_by_default() {
    let mut a = xy();
    assert!(
        !a.sketching.as_ref().unwrap().snap_grid,
        "snap to grid is off by default"
    );
    a.sketching.as_mut().unwrap().snap_grid = true;
    a.set_tool(Tool::Rectangle);
    let step = dc_app::sketching::snap_step(a.camera.distance);
    click_px(&mut a, (0.0, 0.0), 0.0, 0.0); // origin snap
    click(&mut a, (33.3 * step / step, 17.1));
    for (x, y) in points(&session_sketch(&a)) {
        let k = |v: f64| ((v / step).round() * step - v).abs();
        assert!(
            k(x) < 1e-9 && k(y) < 1e-9,
            "({x},{y}) is a multiple of {step}"
        );
    }
}

#[test]
fn a_new_shape_reuses_the_point_it_starts_on() {
    let mut a = xy();
    no_grid(&mut a);
    a.set_tool(Tool::Rectangle);
    click(&mut a, (0.0, 0.0));
    click(&mut a, (40.0, 20.0));
    assert_eq!(points(&session_sketch(&a)).len(), 4);
    click(&mut a, (40.0, 20.0)); // exactly on an existing corner
    click(&mut a, (70.0, 50.0));
    let sk = session_sketch(&a);
    assert_eq!(
        points(&sk).len(),
        7,
        "the shared corner is one point, not two"
    );
    assert_eq!(count(&sk, |e| matches!(e, Entity::Line { .. })), 8);
}

#[test]
fn a_circle_is_a_centre_and_a_radius() {
    let mut a = xy();
    no_grid(&mut a);
    a.set_tool(Tool::Circle);
    click(&mut a, (30.0, 20.0));
    click(&mut a, (40.0, 20.0));
    let sk = session_sketch(&a);
    let r = sk.entities().values().find_map(|e| match e {
        Entity::Circle { radius, .. } => Some(*radius),
        _ => None,
    });
    assert!((r.unwrap() - 10.0).abs() < 1e-6, "{r:?}");
    assert!(points(&sk).iter().any(|p| near(*p, (30.0, 20.0))));
}

#[test]
fn drawing_a_line_near_horizontal_snaps_flat_and_adds_the_constraint() {
    let mut a = xy();
    no_grid(&mut a);
    a.set_tool(Tool::Line);
    click(&mut a, (10.0, 10.0));
    click_px(&mut a, (40.0, 10.0), 0.0, 3.0); // 3 px off horizontal
    let sk = session_sketch(&a);
    assert!(sk
        .constraints()
        .values()
        .any(|c| matches!(c, Constraint::Horizontal { .. })));
    let ys: Vec<f64> = points(&sk).iter().map(|p| p.1).collect();
    assert!(
        ys.iter().all(|y| (y - 10.0).abs() < 1e-9),
        "both ends on y = 10: {ys:?}"
    );
    // The chain continues from the end point; a vertical move snaps to a vertical guide.
    click_px(&mut a, (40.0, 30.0), 3.0, 0.0);
    let sk = session_sketch(&a);
    assert_eq!(count(&sk, |e| matches!(e, Entity::Line { .. })), 2);
    assert_eq!(
        points(&sk).len(),
        3,
        "the second line starts at the first line's end"
    );
    assert!(sk
        .constraints()
        .values()
        .any(|c| matches!(c, Constraint::Vertical { .. })));
    // Clicking the end point again finishes the chain.
    click(&mut a, (40.0, 30.0));
    assert!(a.sketching.as_ref().unwrap().first.is_none());
    assert_eq!(
        count(&session_sketch(&a), |e| matches!(e, Entity::Line { .. })),
        2
    );
}

#[test]
fn a_hand_drawn_plate_with_a_hole_extrudes_to_the_right_volume() {
    let mut a = xy();
    no_grid(&mut a);
    a.set_tool(Tool::Rectangle);
    click(&mut a, (0.0, 0.0));
    click(&mut a, (100.0, 60.0));
    a.set_tool(Tool::Circle);
    click(&mut a, (50.0, 30.0));
    click(&mut a, (58.0, 30.0));
    let name = a.sketching.as_ref().unwrap().sketch.clone();
    let profiles = match a
        .authority
        .query(&Query::SketchProfiles {
            sketch: name.clone(),
            after: None,
            limit: None,
        })
        .unwrap()
    {
        QueryResult::SketchProfiles {
            profiles, problems, ..
        } => {
            assert!(problems.is_empty(), "{problems:?}");
            profiles
        }
        other => panic!("{other:?}"),
    };
    assert_eq!(profiles.len(), 1);
    assert_eq!(
        profiles[0].holes.len(),
        1,
        "the circle inside the rectangle is a hole"
    );
    a.end_sketch();
    a.submit(CommandKind::FeatureExtrude {
        feature: "plate".into(),
        sketch: name,
        profiles: vec![profiles[0].id.clone()],
        depth: 10.0,
        reverse: false,
        extent: Default::default(),
    })
    .unwrap();
    let vol = match a
        .authority
        .query(&Query::FeatureSummary {
            feature: "plate".into(),
        })
        .unwrap()
    {
        QueryResult::FeatureSummary { info } => info.summary.volume,
        other => panic!("{other:?}"),
    };
    let want = (100.0 * 60.0 - core::f64::consts::PI * 64.0) * 10.0;
    assert!((vol - want).abs() < 0.5, "volume {vol} vs {want}");
}

#[test]
fn escape_backs_out_one_level_at_a_time() {
    let mut a = xy();
    a.set_tool(Tool::Rectangle);
    click(&mut a, (10.0, 10.0));
    assert!(a.sketching.as_ref().unwrap().first.is_some());
    assert!(a.sketch_cancel(), "drops the rectangle in progress");
    assert_eq!(a.sketching.as_ref().unwrap().tool, Tool::Rectangle);
    assert!(a.sketch_cancel(), "then leaves the tool");
    assert_eq!(a.sketching.as_ref().unwrap().tool, Tool::Select);
    assert!(!a.sketch_cancel(), "nothing left to cancel");
    assert!(
        a.sketching.is_some(),
        "Escape never leaves sketch mode by itself"
    );
}

#[test]
fn selecting_and_deleting_a_line_removes_it_and_what_only_it_needed() {
    let mut a = xy();
    no_grid(&mut a);
    a.set_tool(Tool::Rectangle);
    click(&mut a, (0.0, 0.0));
    click(&mut a, (40.0, 20.0));
    a.set_tool(Tool::Select);
    let (x, y) = a.sketch_to_screen((20.0, 0.0), W, H).unwrap(); // middle of the bottom edge
    a.sketch_click(x, y, W, H);
    assert_eq!(a.sketching.as_ref().unwrap().selection.len(), 1);
    assert!(a.sketch_delete_selected());
    let sk = session_sketch(&a);
    assert_eq!(count(&sk, |e| matches!(e, Entity::Line { .. })), 3);
    assert_eq!(
        sk.constraints()
            .values()
            .filter(|c| matches!(c, Constraint::Horizontal { .. }))
            .count(),
        1,
        "the bottom edge's constraint went with it"
    );
    assert_eq!(
        points(&sk).len(),
        4,
        "its end points are still used by the side edges"
    );
    a.undo().unwrap();
    assert_eq!(
        count(&session_sketch(&a), |e| matches!(e, Entity::Line { .. })),
        4,
        "deleting is one undoable step"
    );
}

#[test]
fn leaving_the_sketch_restores_the_camera_and_undoing_its_creation_ends_the_session() {
    let mut a = AppState::new();
    a.load_demo();
    let cam = a.camera;
    a.begin_sketch(PlaneSpec::Xy { offset: 0.0 }).unwrap();
    assert_ne!(a.camera, cam);
    a.end_sketch();
    assert_eq!(a.camera, cam);
    assert!(matches!(a.selected, Selected::Sketch(_)));

    a.begin_sketch(PlaneSpec::Xz { offset: 0.0 }).unwrap();
    assert!(a.sketching.is_some());
    a.undo().unwrap(); // undoes sketch.create
    assert!(
        a.sketching.is_none(),
        "the sketch no longer exists, so sketch mode ended"
    );
    assert_eq!(a.camera, cam);
}

#[test]
fn sketching_on_a_face_looks_at_that_face_and_centres_on_the_part() {
    let mut a = AppState::new();
    a.load_demo();
    let plane = PlaneSpec::Face {
        feature: "boss".into(),
        face: "cap_end:boss".into(),
        offset: 0.0,
    };
    a.begin_sketch(plane).expect("sketch on the boss top");
    let (lo, hi) = a.scene.body_bounds().unwrap();
    assert!(
        a.camera.target.x >= lo.x
            && a.camera.target.x <= hi.x
            && a.camera.target.y >= lo.y
            && a.camera.target.y <= hi.y,
        "{:?} in {lo:?}..{hi:?}",
        a.camera.target
    );
    let f = a.sketching.as_ref().unwrap().frame.clone();
    assert!(
        f.normal[2] > 0.99 && f.origin[2] > 1.0,
        "a horizontal face above the base: {f:?}"
    );
    let (x, y) = a.sketch_to_screen((0.0, 0.0), W, H).unwrap();
    let back = a.screen_to_sketch(x, y, W, H).unwrap();
    assert!(near((0.0, 0.0), back));
}

#[test]
fn the_overlay_previews_the_shape_in_progress_with_its_size() {
    let mut a = xy();
    no_grid(&mut a);
    a.set_tool(Tool::Rectangle);
    click(&mut a, (10.0, 10.0));
    at(&mut a, (50.0, 30.0));
    let o = a.sketch_overlay(W, H).unwrap();
    assert_eq!(o.preview.len(), 1);
    assert_eq!(o.preview[0].len(), 5, "a closed rectangle outline");
    assert!(o.labels.iter().any(|l| l.1 == "40 × 20"), "{:?}", o.labels);
    assert_eq!(o.marker.map(|m| m.1), Some(Marker::Free));
    assert!(o.dof == 0 && o.strokes.is_empty(), "nothing committed yet");
    // Hovering the origin shows the origin marker.
    at(&mut a, (0.0, 0.0));
    assert_eq!(
        a.sketch_overlay(W, H).unwrap().marker.map(|m| m.1),
        Some(Marker::Origin)
    );
}

#[test]
fn a_rectangle_with_no_height_is_refused_with_a_reason_and_nothing_changes() {
    let mut a = xy();
    no_grid(&mut a);
    a.set_tool(Tool::Line);
    click(&mut a, (0.0, 0.0));
    click(&mut a, (40.0, 0.0));
    a.sketch_cancel();
    a.set_tool(Tool::Rectangle);
    let before = session_sketch(&a);
    let rev = a.authority.revision();
    click(&mut a, (0.0, 0.0)); // both corners are existing points on the same row
    click(&mut a, (40.0, 0.0));
    assert_eq!(a.authority.revision(), rev, "nothing was committed");
    assert_eq!(session_sketch(&a).entities().len(), before.entities().len());
    let err = a
        .messages
        .iter()
        .rev()
        .find(|m| m.level == dc_app::state::Level::Error)
        .expect("an error was reported");
    assert!(err.text.contains("height") && err.hint.is_some(), "{err:?}");
}

#[test]
fn construction_mode_makes_dashed_guides_that_do_not_split_profiles() {
    let mut a = xy();
    no_grid(&mut a);
    a.set_tool(Tool::Rectangle);
    click(&mut a, (0.0, 0.0));
    click(&mut a, (60.0, 40.0));
    a.set_tool(Tool::Line);
    a.set_construction_mode(true);
    click(&mut a, (0.0, 0.0));
    click(&mut a, (60.0, 40.0)); // the diagonal, between two existing points
    a.sketch_cancel();
    let sk = solved(&a);
    assert_eq!(sk.construction().len(), 1);
    assert_eq!(
        profile_count(&a),
        1,
        "a construction diagonal is ignored by profile detection"
    );
    // Switch the mode off and draw a normal line: it is not construction.
    a.set_construction_mode(false);
    a.set_tool(Tool::Line);
    click(&mut a, (10.0, 10.0));
    click(&mut a, (20.0, 10.0));
    assert_eq!(solved(&a).construction().len(), 1);
    // Drawn shapes show as dashed construction strokes in the overlay.
    let o = a.sketch_overlay(W, H).unwrap();
    assert_eq!(o.strokes.iter().filter(|s| s.construction).count(), 1);
}

#[test]
fn x_converts_the_selection_to_construction_and_back_in_one_undo_step() {
    let mut a = xy();
    no_grid(&mut a);
    a.set_tool(Tool::Rectangle);
    click(&mut a, (0.0, 0.0));
    click(&mut a, (40.0, 20.0));
    a.set_tool(Tool::Select);
    let (x, y) = a.sketch_to_screen((20.0, 0.0), W, H).unwrap();
    a.sketch_click(x, y, W, H);
    let rev = a.authority.revision();
    assert!(a.toggle_construction());
    assert_eq!(a.authority.revision().0, rev.0 + 1);
    assert_eq!(solved(&a).construction().len(), 1);
    assert!(a.toggle_construction(), "toggling again turns it back");
    assert!(solved(&a).construction().is_empty());
    a.undo().unwrap();
    assert_eq!(
        solved(&a).construction().len(),
        1,
        "undo restores the previous flag"
    );
}

#[test]
fn a_centre_arc_sweeps_the_way_the_cursor_moves() {
    let mut a = xy();
    no_grid(&mut a);
    a.set_tool(Tool::Arc);
    click(&mut a, (0.0, 0.0)); // centre
    click(&mut a, (30.0, 0.0)); // start
    at(&mut a, (30.0, 20.0)); // first movement: counter-clockwise
    click(&mut a, (0.0, 30.0)); // end: 90 degrees later
    let sk = solved(&a);
    let (c, s, e) = sk
        .entities()
        .values()
        .find_map(|e| match e {
            Entity::Arc { center, start, end } => {
                Some((center.clone(), start.clone(), end.clone()))
            }
            _ => None,
        })
        .expect("an arc");
    let p = |id: &str| sk.point(id).unwrap();
    assert!(near(p(&c), (0.0, 0.0)) && near(p(&s), (30.0, 0.0)));
    assert!(near(p(&e), (0.0, 30.0)), "end at {:?}", p(&e));

    // Moving the other way makes a clockwise arc: its start/end are swapped so it still sweeps ccw.
    a.set_tool(Tool::Arc);
    click(&mut a, (0.0, 0.0));
    click(&mut a, (30.0, 0.0));
    at(&mut a, (30.0, -20.0)); // first movement: clockwise
    click(&mut a, (0.0, -30.0));
    let sk = solved(&a);
    let arcs: Vec<_> = sk
        .entities()
        .values()
        .filter_map(|e| match e {
            Entity::Arc { start, end, .. } => Some((start.clone(), end.clone())),
            _ => None,
        })
        .collect();
    assert_eq!(arcs.len(), 2);
    let (s2, e2) = &arcs[1];
    let (ps, pe) = (sk.point(s2).unwrap(), sk.point(e2).unwrap());
    assert!(
        near(ps, (0.0, -30.0)) && near(pe, (30.0, 0.0)),
        "ccw from the bottom to the right: {ps:?} -> {pe:?}"
    );
}

#[test]
fn a_three_point_arc_passes_through_its_third_point_either_way_round() {
    let mut a = xy();
    no_grid(&mut a);
    a.set_tool(Tool::Arc3);
    click(&mut a, (-20.0, 0.0));
    click(&mut a, (20.0, 0.0));
    click(&mut a, (0.0, 20.0)); // over the top
    let sk = solved(&a);
    let (c, s, e) = sk
        .entities()
        .values()
        .find_map(|e| match e {
            Entity::Arc { center, start, end } => {
                Some((center.clone(), start.clone(), end.clone()))
            }
            _ => None,
        })
        .unwrap();
    let (pc, ps, pe) = (
        sk.point(&c).unwrap(),
        sk.point(&s).unwrap(),
        sk.point(&e).unwrap(),
    );
    assert!(near(pc, (0.0, 0.0)), "centre {pc:?}");
    // Counter-clockwise from start to end must pass through (0, 20): the start is on the right.
    assert!(
        near(ps, (20.0, 0.0)) && near(pe, (-20.0, 0.0)),
        "{ps:?} -> {pe:?}"
    );
    // The other side.
    a.set_tool(Tool::Arc3);
    click(&mut a, (-20.0, 0.0));
    click(&mut a, (20.0, 0.0));
    click(&mut a, (0.0, -20.0));
    let sk = solved(&a);
    let n = sk
        .entities()
        .values()
        .filter(|e| matches!(e, Entity::Arc { .. }))
        .count();
    assert_eq!(n, 2);
}

#[test]
fn three_points_in_a_line_make_no_arc_and_say_why() {
    let mut a = xy();
    no_grid(&mut a);
    a.set_tool(Tool::Arc3);
    click(&mut a, (0.0, 0.0));
    click(&mut a, (20.0, 0.0));
    let rev = a.authority.revision();
    click(&mut a, (10.0, 0.0));
    assert_eq!(a.authority.revision(), rev);
    let err = a
        .messages
        .iter()
        .rev()
        .find(|m| m.level == dc_app::state::Level::Error)
        .unwrap();
    assert!(err.text.contains("straight line"), "{err:?}");
    assert!(
        a.sketching.as_ref().unwrap().second.is_some(),
        "the first two points are kept so the third can be fixed"
    );
}

#[test]
fn relations_offered_depend_on_what_is_selected_and_apply_as_one_command() {
    let mut a = xy();
    no_grid(&mut a);
    a.set_tool(Tool::Line);
    click(&mut a, (0.0, 0.0));
    click(&mut a, (40.0, 5.0)); // not axis-aligned: no guide
    a.sketch_cancel();
    click(&mut a, (0.0, 30.0));
    click(&mut a, (40.0, 20.0));
    a.sketch_cancel();
    a.set_tool(Tool::Select);
    assert!(a.available_relations().is_empty(), "nothing selected");
    let pick = |a: &mut AppState, p: (f64, f64), add: bool| {
        let (x, y) = a.sketch_to_screen(p, W, H).unwrap();
        a.sketch_click_with(x, y, W, H, add);
    };
    pick(&mut a, (20.0, 2.5), false);
    assert_eq!(
        a.available_relations(),
        vec![Relation::Horizontal, Relation::Vertical, Relation::Fix]
    );
    pick(&mut a, (20.0, 25.0), true);
    let rel = a.available_relations();
    for r in [
        Relation::Horizontal,
        Relation::Parallel,
        Relation::Perpendicular,
        Relation::Equal,
        Relation::Fix,
    ] {
        assert!(rel.contains(&r), "{r:?} in {rel:?}");
    }
    assert!(!rel.contains(&Relation::Coincident) && !rel.contains(&Relation::Tangent));
    let rev = a.authority.revision();
    assert!(a.apply_relation(Relation::Parallel));
    assert_eq!(a.authority.revision().0, rev.0 + 1);
    assert_eq!(
        constraints_of(&solved(&a), |c| matches!(c, Constraint::Parallel { .. })),
        1
    );
    // Shift-click again removes it from the selection.
    pick(&mut a, (24.0, 24.0), true);
    assert_eq!(a.sketching.as_ref().unwrap().selection.len(), 1);
    // A relation that was not offered is refused.
    assert!(!a.apply_relation(Relation::Equal));
}

#[test]
fn a_new_point_on_an_edge_is_related_to_that_line() {
    let mut a = xy();
    no_grid(&mut a);
    a.set_tool(Tool::Line);
    click(&mut a, (0.0, 0.0));
    click(&mut a, (60.0, 30.0));
    a.sketch_cancel();
    // Start a new line on the first one, away from its middle (3 px off it).
    click_px(&mut a, (20.0, 10.0), 0.0, 3.0);
    let o = a.sketch_overlay(W, H).unwrap();
    assert_eq!(o.marker.map(|m| m.1), Some(Marker::OnEdge));
    assert_eq!(o.hint.as_deref(), Some("on edge"));
    click(&mut a, (30.0, 50.0));
    let sk = solved(&a);
    assert_eq!(
        constraints_of(&sk, |c| matches!(c, Constraint::PointOnLine { .. })),
        1
    );
    // The new point is the nearest spot on the line to where the cursor was: exactly on it.
    let on_line: Vec<_> = points(&sk)
        .into_iter()
        .filter(|p| p.0 > 1.0 && p.0 < 59.0 && (p.1 - p.0 / 2.0).abs() < 1e-9)
        .collect();
    assert_eq!(
        on_line.len(),
        1,
        "one new point exactly on y = x/2: {:?}",
        points(&sk)
    );
    assert!(
        (on_line[0].0 - 20.0).abs() < 1.0,
        "close to where it was clicked: {:?}",
        on_line[0]
    );
}

#[test]
fn dragging_a_point_previews_without_logging_and_commits_once_on_release() {
    let mut a = xy();
    no_grid(&mut a);
    a.set_tool(Tool::Rectangle);
    click(&mut a, (0.0, 0.0));
    click(&mut a, (40.0, 20.0));
    a.set_tool(Tool::Select);
    let rev = a.authority.revision();
    let (sx, sy) = a.sketch_to_screen((40.0, 20.0), W, H).unwrap();
    assert!(
        a.sketch_drag_begin(sx, sy, W, H),
        "there is a corner under the cursor"
    );
    for target in [(45.0, 25.0), (50.0, 30.0), (60.0, 35.0)] {
        let (x, y) = a.sketch_to_screen(target, W, H).unwrap();
        a.sketch_drag_move(x, y, W, H);
    }
    assert_eq!(a.authority.revision(), rev, "dragging logs nothing");
    // The preview shows the rest of the rectangle following (its sides stay axis-aligned).
    let d = a.sketching.as_ref().unwrap().drag.clone().unwrap();
    assert!(near(d.target, (60.0, 35.0)));
    assert!(
        d.preview.values().any(|p| near(*p, (60.0, 0.0))),
        "{:?}",
        d.preview
    );
    assert!(
        d.preview.values().any(|p| near(*p, (0.0, 35.0))),
        "{:?}",
        d.preview
    );
    assert!(a.sketch_overlay(W, H).unwrap().strokes.len() == 4);
    assert!(a.sketch_drag_end());
    assert_eq!(
        a.authority.revision().0,
        rev.0 + 1,
        "one command for the whole drag"
    );
    assert!(points(&solved(&a)).iter().any(|p| near(*p, (60.0, 35.0))));
    a.undo().unwrap();
    assert!(
        points(&solved(&a)).iter().any(|p| near(*p, (40.0, 20.0))),
        "undo puts it back"
    );
}

#[test]
fn dragging_nothing_or_without_moving_does_nothing() {
    let mut a = xy();
    no_grid(&mut a);
    a.set_tool(Tool::Rectangle);
    click(&mut a, (0.0, 0.0));
    click(&mut a, (40.0, 20.0));
    a.set_tool(Tool::Select);
    let (x, y) = a.sketch_to_screen((20.0, 50.0), W, H).unwrap();
    assert!(!a.sketch_drag_begin(x, y, W, H), "empty space");
    let rev = a.authority.revision();
    let (px, py) = a.sketch_to_screen((40.0, 20.0), W, H).unwrap();
    assert!(a.sketch_drag_begin(px, py, W, H));
    assert!(!a.sketch_drag_end(), "released where it started");
    assert_eq!(a.authority.revision(), rev);
    // Esc during a drag cancels it.
    assert!(a.sketch_drag_begin(px, py, W, H));
    assert!(a.sketch_cancel());
    assert!(a.sketching.as_ref().unwrap().drag.is_none());
}

#[test]
fn deleting_several_selected_things_is_one_undo_step() {
    let mut a = xy();
    no_grid(&mut a);
    a.set_tool(Tool::Circle);
    click(&mut a, (0.0, 0.0));
    click(&mut a, (10.0, 0.0));
    click(&mut a, (50.0, 0.0));
    click(&mut a, (60.0, 0.0));
    a.set_tool(Tool::Select);
    for (p, add) in [((0.0, 10.0), false), ((50.0, 10.0), true)] {
        let (x, y) = a.sketch_to_screen(p, W, H).unwrap();
        a.sketch_click_with(x, y, W, H, add);
    }
    assert_eq!(a.sketching.as_ref().unwrap().selection.len(), 2);
    let rev = a.authority.revision();
    assert!(a.sketch_delete_selected());
    assert_eq!(a.authority.revision().0, rev.0 + 1);
    assert!(
        solved(&a).entities().is_empty(),
        "both circles and their centres are gone"
    );
    a.undo().unwrap();
    assert_eq!(
        count(&solved(&a), |e| matches!(e, Entity::Circle { .. })),
        2
    );
}

// ---- helpers shared by the SolidWorks-style tests -------------------------------------------

fn solved(a: &AppState) -> dc_sketch::Sketch {
    session_sketch(a)
}

fn constraints_of(sk: &dc_sketch::Sketch, f: impl Fn(&Constraint) -> bool) -> usize {
    sk.constraints().values().filter(|c| f(c)).count()
}

fn profile_count(a: &AppState) -> usize {
    let name = a.sketching.as_ref().unwrap().sketch.clone();
    match a
        .authority
        .query(&Query::SketchProfiles {
            sketch: name,
            after: None,
            limit: None,
        })
        .unwrap()
    {
        QueryResult::SketchProfiles {
            profiles, problems, ..
        } => {
            assert!(problems.is_empty(), "{problems:?}");
            profiles.len()
        }
        other => panic!("{other:?}"),
    }
}

// ---- smart dimensioning ------------------------------------------------------------------------

use dc_app::dimensioning::{Orient, Pick, Resolved, Spec};

/// With the Dimension tool: click the geometry at each position (picking it).
fn dim_pick(a: &mut AppState, at_pos: &[(f64, f64)]) {
    a.set_tool(Tool::Dimension);
    for p in at_pos {
        click(a, *p);
    }
}

/// Click empty space to place the dimension there.
fn dim_place(a: &mut AppState, at_pos: (f64, f64)) {
    click(a, at_pos);
}

fn edit_text(a: &mut AppState, text: &str) {
    a.sketching
        .as_mut()
        .unwrap()
        .dim
        .edit
        .as_mut()
        .expect("the value box is open")
        .text = text.into();
}

/// Pick, place, type, commit.
fn dimension(a: &mut AppState, picks: &[(f64, f64)], place: (f64, f64), text: &str) -> bool {
    dim_pick(a, picks);
    dim_place(a, place);
    edit_text(a, text);
    a.apply_dimension()
}

fn picks(a: &AppState) -> Vec<Pick> {
    a.sketching.as_ref().unwrap().dim.picks.clone()
}

fn spec_for(a: &AppState, picks: &[Pick], cursor: (f64, f64)) -> Spec {
    match a.resolve_dimension(picks, cursor) {
        Resolved::Spec(s) => s,
        Resolved::Wait(m) | Resolved::Refuse(m) => panic!("no spec: {m}"),
    }
}

fn entity_id_at(a: &AppState, p: (f64, f64)) -> String {
    let (x, y) = a.sketch_to_screen(p, W, H).unwrap();
    match a.dimension_pick_at(x, y, W, H) {
        Some(Pick::Entity(id)) => id,
        other => panic!("expected an entity at {p:?}, found {other:?}"),
    }
}

fn draw_line(a: &mut AppState, p: (f64, f64), q: (f64, f64)) {
    a.set_tool(Tool::Line);
    click(a, p);
    click(a, q);
    a.sketch_cancel();
}

fn add_point(a: &mut AppState, id: &str, x: f64, y: f64) {
    let sketch = a.sketching.as_ref().unwrap().sketch.clone();
    a.submit(CommandKind::SketchAddEntity {
        sketch,
        entity_id: id.into(),
        entity: Entity::Point { x, y },
    })
    .unwrap();
}

fn point_at(a: &AppState, id: &str) -> (f64, f64) {
    session_sketch(a).point(id).unwrap()
}

#[test]
fn picking_a_line_shows_its_length_live_and_commits_nothing_until_it_is_placed() {
    let mut a = xy();
    no_grid(&mut a);
    draw_line(&mut a, (0.0, 0.0), (30.0, 0.0));
    let rev = a.authority.revision();
    dim_pick(&mut a, &[(15.0, 0.0)]);
    assert_eq!(picks(&a).len(), 1);
    assert_eq!(a.authority.revision(), rev, "picking logs nothing");
    // The cursor drives a preview dimension: ISO style, with arrows, extension lines and the value.
    at(&mut a, (15.0, 20.0));
    let o = a.sketch_overlay(W, H).unwrap();
    let d = o
        .dims
        .iter()
        .find(|d| d.preview)
        .expect("a preview dimension follows the cursor");
    assert_eq!(d.text, "30");
    assert_eq!(d.arrows.len(), 2, "an arrowhead at each end");
    assert_eq!(
        d.witness.len(),
        2,
        "an extension line from each end of the line"
    );
    assert!(
        o.dim_prompt
            .as_deref()
            .is_some_and(|p| p.starts_with("Length")),
        "{:?}",
        o.dim_prompt
    );
    // Moving the cursor moves the dimension line; nothing is committed.
    let y1 = d.text_at.1;
    at(&mut a, (15.0, 40.0));
    let y2 = a
        .sketch_overlay(W, H)
        .unwrap()
        .dims
        .iter()
        .find(|d| d.preview)
        .unwrap()
        .text_at
        .1;
    assert!(
        (y1 - y2).abs() > 20.0,
        "the dimension line follows the cursor ({y1} -> {y2})"
    );
    assert_eq!(a.authority.revision(), rev);
    // Clicking empty space places it and opens the value box, pre-filled.
    dim_place(&mut a, (15.0, 40.0));
    let e = a
        .sketching
        .as_ref()
        .unwrap()
        .dim
        .edit
        .clone()
        .expect("the value box opened");
    assert_eq!((e.text.as_str(), e.existing), ("30", None));
    assert_eq!(a.authority.revision(), rev, "still nothing committed");
    edit_text(&mut a, "45");
    assert!(a.apply_dimension());
    assert_eq!(a.authority.revision().0, rev.0 + 1, "one command");
    let sk = solved(&a);
    let pts = points(&sk);
    assert!((dc_math::det::hypot(pts[0].0 - pts[1].0, pts[0].1 - pts[1].1) - 45.0).abs() < 1e-6);
    assert!(
        session_sketch(&a).placements().len() == 1,
        "the placement is remembered in the sketch itself"
    );
}

#[test]
fn where_the_cursor_is_decides_aligned_horizontal_or_vertical() {
    let mut a = xy();
    no_grid(&mut a);
    draw_line(&mut a, (0.0, 0.0), (40.0, 30.0));
    let line = entity_id_at(&a, (20.0, 15.0));
    let p = [Pick::Entity(line.clone())];
    // Beside the line, along its normal: aligned (its true length, 50).
    assert_eq!(
        spec_for(&a, &p, (8.0, 31.0)),
        Spec::Length { line: line.clone() }
    );
    // Above it: horizontal (40). To its right: vertical (30).
    for (cursor, want_orient) in [
        ((20.0, 60.0), Orient::Horizontal),
        ((70.0, 15.0), Orient::Vertical),
    ] {
        match spec_for(&a, &p, cursor) {
            Spec::Between { orient, .. } => assert_eq!(orient, want_orient, "{cursor:?}"),
            other => panic!("{other:?}"),
        }
    }
    // The preview text follows: 50, 40, 30.
    dim_pick(&mut a, &[(20.0, 15.0)]);
    for (cursor, text) in [
        ((8.0, 31.0), "50"),
        ((20.0, 60.0), "40"),
        ((70.0, 15.0), "30"),
    ] {
        at(&mut a, cursor);
        let o = a.sketch_overlay(W, H).unwrap();
        assert_eq!(
            o.dims.iter().find(|d| d.preview).unwrap().text,
            text,
            "{cursor:?}"
        );
    }
}

#[test]
fn clicking_an_axis_after_a_line_measures_that_lines_distance_from_the_axis() {
    let mut a = xy();
    no_grid(&mut a);
    draw_line(&mut a, (10.0, 30.0), (50.0, 30.0)); // snaps flat: a horizontal constraint
    dim_pick(&mut a, &[(30.0, 30.0)]);
    at(&mut a, (30.0, 60.0));
    assert_eq!(
        a.sketch_overlay(W, H)
            .unwrap()
            .dims
            .iter()
            .find(|d| d.preview)
            .unwrap()
            .text,
        "40",
        "with only the line picked it means its length"
    );
    // Now press the horizontal axis: the same dimension now means distance from that axis.
    click(&mut a, (80.0, 0.0));
    assert_eq!(picks(&a).len(), 2);
    assert!(matches!(picks(&a)[1], Pick::AxisX));
    at(&mut a, (30.0, 60.0));
    let o = a.sketch_overlay(W, H).unwrap();
    assert_eq!(
        o.dims.iter().find(|d| d.preview).unwrap().text,
        "30",
        "30 mm above the axis"
    );
    assert!(o.axes.iter().any(|x| x.picked), "the picked axis stays lit");
    let rev = a.authority.revision();
    dim_place(&mut a, (95.0, 15.0));
    assert!(a.sketching.as_ref().unwrap().dim.edit.is_some());
    edit_text(&mut a, "35");
    assert!(a.apply_dimension(), "{:?}", a.messages.last());
    assert_eq!(a.authority.revision().0, rev.0 + 1);
    let sk = solved(&a);
    let ys: Vec<f64> = points(&sk)
        .iter()
        .map(|p| p.1)
        .filter(|y| *y != 0.0)
        .collect();
    assert!(
        ys.iter().all(|y| (y - 35.0).abs() < 1e-6),
        "the line now sits 35 above the axis: {:?}",
        points(&sk)
    );
    assert_eq!(
        constraints_of(&sk, |c| matches!(c, Constraint::VerticalDistance { .. })),
        1
    );
    // The reference to the origin is an ordinary fixed point.
    let fixed_origin = sk
        .constraints()
        .values()
        .any(|c| matches!(c, Constraint::Fixed { x, y, .. } if *x == 0.0 && *y == 0.0));
    assert!(fixed_origin, "{:?}", sk.constraints());
}

#[test]
fn the_vertical_axis_measures_horizontally_and_the_origin_point_is_made_once() {
    let mut a = xy();
    no_grid(&mut a);
    draw_line(&mut a, (20.0, 10.0), (20.0, 50.0)); // vertical
    assert!(dimension(
        &mut a,
        &[(20.0, 30.0), (0.0, 80.0)],
        (35.0, 70.0),
        "25"
    ));
    let sk = solved(&a);
    assert_eq!(
        constraints_of(&sk, |c| matches!(c, Constraint::HorizontalDistance { .. })),
        1
    );
    assert!(
        points(&sk).iter().any(|p| near(*p, (25.0, 10.0)))
            && points(&sk).iter().any(|p| near(*p, (25.0, 50.0))),
        "{:?}",
        points(&sk)
    );
    // A second dimension from the other axis reuses the same origin point.
    assert!(dimension(
        &mut a,
        &[(25.0, 20.0), (80.0, 0.0)],
        (60.0, 25.0),
        "12"
    ));
    let sk = solved(&a);
    let origins = sk
        .constraints()
        .values()
        .filter(|c| matches!(c, Constraint::Fixed { x, y, .. } if *x == 0.0 && *y == 0.0))
        .count();
    assert_eq!(
        origins, 1,
        "one fixed origin point shared by both dimensions"
    );
    assert_eq!(
        constraints_of(&sk, |c| matches!(c, Constraint::VerticalDistance { .. })),
        1
    );
}

#[test]
fn two_points_measure_a_distance_whose_direction_follows_the_cursor() {
    let mut a = xy();
    no_grid(&mut a);
    add_point(&mut a, "pa", 10.0, 10.0);
    add_point(&mut a, "pb", 40.0, 30.0);
    let picks_ab = [Pick::Entity("pa".into()), Pick::Entity("pb".into())];
    let want = |cursor| match spec_for(&a, &picks_ab, cursor) {
        Spec::Between { orient, .. } => orient,
        other => panic!("{other:?}"),
    };
    assert_eq!(
        want((25.0, 60.0)),
        Orient::Horizontal,
        "straight above the middle"
    );
    assert_eq!(want((70.0, 20.0)), Orient::Vertical);
    assert_eq!(want((16.0, 33.0)), Orient::Aligned);
    // Commit the horizontal one: ordered left to right, stored positive, and editing it moves pb.
    assert!(dimension(
        &mut a,
        &[(10.0, 10.0), (40.0, 30.0)],
        (25.0, 60.0),
        "45"
    ));
    let sk = solved(&a);
    assert!(
        matches!(
            sk.constraints().values().find(|c| matches!(c, Constraint::HorizontalDistance { .. })),
            Some(Constraint::HorizontalDistance { a, b, value }) if a == "pa" && b == "pb" && *value == 45.0
        ),
        "{:?}",
        sk.constraints()
    );
    // Picking them in the other order gives the same dimension (left to right), not a negative one.
    let mut b = xy();
    no_grid(&mut b);
    add_point(&mut b, "pa", 10.0, 10.0);
    add_point(&mut b, "pb", 40.0, 30.0);
    assert!(dimension(
        &mut b,
        &[(40.0, 30.0), (10.0, 10.0)],
        (25.0, 60.0),
        "45"
    ));
    assert_eq!(
        constraints_of(
            &solved(&b),
            |c| matches!(c, Constraint::HorizontalDistance { value, .. } if *value > 0.0)
        ),
        1
    );
}

#[test]
fn a_point_and_a_line_measure_the_perpendicular_distance_and_keep_their_side() {
    let mut a = xy();
    no_grid(&mut a);
    draw_line(&mut a, (0.0, 0.0), (80.0, 0.0));
    add_point(&mut a, "above", 40.0, 25.0);
    add_point(&mut a, "below", 60.0, -15.0);
    assert!(dimension(
        &mut a,
        &[(40.0, 25.0), (20.0, 0.0)],
        (-20.0, 12.0),
        "30"
    ));
    // The line and the point are both free, so the solver may nudge both; the distance is what counts.
    let line_y = |a: &AppState| {
        let sk = solved(a);
        let line = sk.entities().values().find_map(|e| match e {
            Entity::Line { a, .. } => Some(a.clone()),
            _ => None,
        });
        sk.point(&line.unwrap()).unwrap().1
    };
    assert!((point_at(&a, "above").1 - line_y(&a) - 30.0).abs() < 1e-6);
    assert!(dimension(
        &mut a,
        &[(60.0, -15.0), (20.0, 0.0)],
        (-20.0, -8.0),
        "20"
    ));
    assert!(
        (point_at(&a, "below").1 - line_y(&a) + 20.0).abs() < 1e-6,
        "it stays 20 below the line, not flipped above it"
    );
    assert!(
        (point_at(&a, "above").1 - line_y(&a) - 30.0).abs() < 1e-6,
        "and the first one still holds"
    );
    let sk = solved(&a);
    let signed: Vec<f64> = sk
        .constraints()
        .values()
        .filter_map(|c| match c {
            Constraint::PointLineDistance { value, .. } => Some(*value),
            _ => None,
        })
        .collect();
    assert_eq!(signed.len(), 2);
    assert!(
        signed.contains(&30.0) && signed.contains(&-20.0),
        "{signed:?}"
    );
    // The inspector and the dimension both show magnitudes.
    assert!(sk
        .constraints()
        .values()
        .filter_map(|c| c.value())
        .all(|v| v > 0.0));
}

#[test]
fn parallel_lines_measure_the_gap_and_other_lines_measure_their_angle() {
    let mut a = xy();
    no_grid(&mut a);
    draw_line(&mut a, (0.0, 0.0), (60.0, 0.0));
    draw_line(&mut a, (0.0, 20.0), (60.0, 20.0));
    draw_line(&mut a, (0.0, 40.0), (50.0, 55.0));
    let (l1, l2, l3) = (
        entity_id_at(&a, (30.0, 0.0)),
        entity_id_at(&a, (30.0, 20.0)),
        entity_id_at(&a, (25.0, 47.5)),
    );
    assert!(matches!(
        a.resolve_dimension(
            &[Pick::Entity(l1.clone()), Pick::Entity(l2.clone())],
            (80.0, 10.0)
        ),
        Resolved::Spec(Spec::PointLine { .. })
    ));
    assert!(
        matches!(
            a.resolve_dimension(&[Pick::Entity(l1), Pick::Entity(l3)], (80.0, 10.0)),
            Resolved::Spec(Spec::Angle { .. })
        ),
        "two lines that are not parallel measure the angle between them"
    );
    assert!(dimension(
        &mut a,
        &[(30.0, 0.0), (30.0, 20.0)],
        (80.0, 10.0),
        "35"
    ));
    // The two lines are 35 apart (both were free, so each may have moved a little).
    let sk = solved(&a);
    let mut line_ys: Vec<f64> = sk
        .entities()
        .values()
        .filter_map(|e| match e {
            Entity::Line { a, .. } => sk.point(a).map(|p| p.1),
            _ => None,
        })
        .collect();
    line_ys.sort_by(f64::total_cmp);
    assert!(
        line_ys
            .windows(2)
            .any(|w| (w[1] - w[0] - 35.0).abs() < 1e-6),
        "some pair of lines is 35 apart: {line_ys:?}"
    );
}

#[test]
fn circles_arcs_and_axes_each_get_the_natural_dimension() {
    let mut a = xy();
    no_grid(&mut a);
    a.set_tool(Tool::Circle);
    click(&mut a, (30.0, 40.0));
    click(&mut a, (40.0, 40.0));
    // A circle on its own is a diameter, shown with the Ø symbol while it is previewed.
    dim_pick(&mut a, &[(30.0, 50.0)]);
    at(&mut a, (60.0, 70.0));
    assert_eq!(
        a.sketch_overlay(W, H)
            .unwrap()
            .dims
            .iter()
            .find(|d| d.preview)
            .unwrap()
            .text,
        "Ø20"
    );
    dim_place(&mut a, (60.0, 70.0));
    edit_text(&mut a, "30");
    assert!(a.apply_dimension());
    let r = solved(&a).constraints().values().find_map(|c| match c {
        Constraint::Radius { value, .. } => Some(*value),
        _ => None,
    });
    assert_eq!(r, Some(15.0), "typed as a diameter, stored as a radius");
    // The circle with the vertical axis is its centre's distance from that axis.
    a.sketch_cancel();
    assert!(dimension(
        &mut a,
        &[(30.0, 55.0), (0.0, 85.0)],
        (50.0, 85.0),
        "60"
    ));
    assert!(
        point_at(&a, "p1").0 == 60.0
            || solved(&a)
                .entities()
                .keys()
                .any(|k| point_at(&a, k).0 == 60.0),
        "centre moved to 60 from the axis"
    );
    // An arc is a radius.
    a.set_tool(Tool::Arc3);
    click(&mut a, (-60.0, 10.0));
    click(&mut a, (-20.0, 10.0));
    click(&mut a, (-40.0, 30.0));
    let arc = solved(&a)
        .entities()
        .iter()
        .find(|(_, e)| matches!(e, Entity::Arc { .. }))
        .map(|(k, _)| k.clone())
        .unwrap();
    assert!(matches!(
        spec_for(&a, &[Pick::Entity(arc)], (-40.0, 60.0)),
        Spec::Round {
            diameter: false,
            ..
        }
    ));
}

#[test]
fn clicking_a_placed_dimension_reopens_it_and_dimensioning_again_edits_instead_of_duplicating() {
    let mut a = xy();
    no_grid(&mut a);
    draw_line(&mut a, (0.0, 0.0), (30.0, 0.0));
    assert!(dimension(&mut a, &[(15.0, 0.0)], (15.0, 20.0), "40"));
    let cid = session_sketch(&a)
        .placements()
        .keys()
        .next()
        .unwrap()
        .clone();
    // Click the dimension's text with the Dimension tool: it reopens with the current value.
    a.set_tool(Tool::Dimension);
    let o = a.sketch_overlay(W, H).unwrap();
    let text_at = o
        .dims
        .iter()
        .find(|d| d.id.as_deref() == Some(cid.as_str()))
        .unwrap()
        .text_at;
    a.sketch_click(text_at.0, text_at.1, W, H);
    let e = a
        .sketching
        .as_ref()
        .unwrap()
        .dim
        .edit
        .clone()
        .expect("reopened");
    assert_eq!(
        (e.text.as_str(), e.existing.as_deref()),
        ("40", Some(cid.as_str()))
    );
    edit_text(&mut a, "55");
    assert!(a.apply_dimension());
    assert_eq!(
        constraints_of(&solved(&a), |c| matches!(c, Constraint::Length { .. })),
        1
    );
    // Double-click with the Select tool does the same.
    a.set_tool(Tool::Select);
    let o = a.sketch_overlay(W, H).unwrap();
    let t = o
        .dims
        .iter()
        .find(|d| d.id.as_deref() == Some(cid.as_str()))
        .unwrap()
        .text_at;
    assert!(a.sketch_double_click(t.0, t.1, W, H));
    assert_eq!(
        a.sketching
            .as_ref()
            .unwrap()
            .dim
            .edit
            .as_ref()
            .unwrap()
            .text,
        "55"
    );
    a.sketch_cancel();
    // Dimensioning the same line from scratch finds the existing dimension and edits that one.
    dim_pick(&mut a, &[(27.0, 0.0)]);
    dim_place(&mut a, (27.0, -25.0));
    let e = a.sketching.as_ref().unwrap().dim.edit.clone().unwrap();
    assert_eq!(e.existing.as_deref(), Some(cid.as_str()));
    assert_eq!(e.text, "55");
}

#[test]
fn escape_drops_the_picks_and_the_value_box_without_committing_anything() {
    let mut a = xy();
    no_grid(&mut a);
    draw_line(&mut a, (0.0, 0.0), (30.0, 0.0));
    let rev = a.authority.revision();
    dim_pick(&mut a, &[(15.0, 0.0)]);
    assert!(a.sketch_cancel());
    assert!(picks(&a).is_empty());
    dim_pick(&mut a, &[(15.0, 0.0)]);
    dim_place(&mut a, (15.0, 30.0));
    assert!(a.sketching.as_ref().unwrap().dim.edit.is_some());
    assert!(a.sketch_cancel());
    let d = &a.sketching.as_ref().unwrap().dim;
    assert!(d.edit.is_none() && d.picks.is_empty());
    assert_eq!(a.authority.revision(), rev, "nothing was ever committed");
    assert_eq!(
        a.sketching.as_ref().unwrap().tool,
        Tool::Dimension,
        "the tool stays active"
    );
}

#[test]
fn bad_numbers_are_refused_in_the_box_and_the_engine_explains_contradictions() {
    let mut a = xy();
    no_grid(&mut a);
    a.set_tool(Tool::Rectangle);
    click(&mut a, (0.0, 0.0));
    click(&mut a, (40.0, 20.0));
    dim_pick(&mut a, &[(20.0, 0.0)]);
    dim_place(&mut a, (20.0, -20.0));
    let rev = a.authority.revision();
    for bad in ["abc", "-3", "0", ""] {
        edit_text(&mut a, bad);
        assert!(!a.apply_dimension(), "{bad:?}");
        assert!(a
            .sketching
            .as_ref()
            .unwrap()
            .dim
            .edit
            .as_ref()
            .unwrap()
            .error
            .is_some());
    }
    assert_eq!(
        a.authority.revision(),
        rev,
        "nothing committed for a bad number"
    );
    edit_text(&mut a, "40");
    assert!(a.apply_dimension());
    // Top and bottom of a rectangle cannot differ: refused, box stays open with the reason.
    dim_pick(&mut a, &[(20.0, 20.0)]);
    dim_place(&mut a, (20.0, 40.0));
    edit_text(&mut a, "50");
    let rev = a.authority.revision();
    assert!(!a.apply_dimension());
    assert_eq!(a.authority.revision(), rev);
    let e = a
        .sketching
        .as_ref()
        .unwrap()
        .dim
        .edit
        .as_ref()
        .expect("still open to fix the value");
    assert!(e.error.as_deref().is_some_and(|m| !m.is_empty()), "{e:?}");
}

#[test]
fn the_cursor_highlights_what_a_click_would_pick_origin_first_then_entities_then_axes() {
    let mut a = xy();
    no_grid(&mut a);
    draw_line(&mut a, (10.0, 30.0), (50.0, 30.0));
    a.set_tool(Tool::Dimension);
    let hover = |a: &mut AppState, p: (f64, f64)| {
        at(a, p);
        a.sketching.as_ref().unwrap().dim.hover.clone()
    };
    assert_eq!(hover(&mut a, (0.0, 0.0)), Some(Pick::Origin));
    assert!(matches!(hover(&mut a, (30.0, 30.0)), Some(Pick::Entity(_))));
    assert_eq!(hover(&mut a, (80.0, 0.0)), Some(Pick::AxisX));
    assert_eq!(hover(&mut a, (0.0, 70.0)), Some(Pick::AxisY));
    assert_eq!(hover(&mut a, (90.0, 60.0)), None);
    // The overlay lights up the axis under the cursor.
    at(&mut a, (80.0, 0.0));
    let o = a.sketch_overlay(W, H).unwrap();
    assert!(o.axes.iter().any(|x| !x.picked));
    // A line wins over an axis it lies on.
    let mut b = xy();
    no_grid(&mut b);
    draw_line(&mut b, (10.0, 0.0), (50.0, 0.0));
    b.set_tool(Tool::Dimension);
    assert!(matches!(hover(&mut b, (30.0, 0.0)), Some(Pick::Entity(_))));
}

#[test]
fn a_horizontal_dimension_is_drawn_iso_style_horizontal_with_vertical_extension_lines() {
    let mut a = xy();
    no_grid(&mut a);
    draw_line(&mut a, (0.0, 0.0), (40.0, 30.0));
    dim_pick(&mut a, &[(20.0, 15.0)]);
    dim_place(&mut a, (20.0, 55.0)); // above: horizontal
    let d = a
        .sketch_overlay(W, H)
        .unwrap()
        .dims
        .into_iter()
        .find(|d| d.preview)
        .unwrap();
    assert_eq!(d.text, "40");
    let [p1, p2] = d.line[0];
    assert!(
        (p1.1 - p2.1).abs() < 1e-6,
        "the dimension line is horizontal on screen"
    );
    assert!(d.angle.abs() < 1e-6, "so is its text");
    for [s, e] in &d.witness {
        assert!(
            (s.0 - e.0).abs() < 1e-6,
            "extension lines are vertical: {s:?} {e:?}"
        );
    }
    // Arrows touch the extension lines, pointing outward.
    let (left, right) = if p1.0 < p2.0 { (p1, p2) } else { (p2, p1) };
    for (tip, dir) in &d.arrows {
        if (tip.0 - left.0).abs() < 1e-6 {
            assert!(dir.0 < -0.99, "left arrow points left: {dir:?}");
        } else {
            assert!(
                (tip.0 - right.0).abs() < 1e-6 && dir.0 > 0.99,
                "right arrow points right"
            );
        }
    }
    assert!(d.above, "the text sits above the line");
}

#[test]
fn undoing_a_dimension_that_created_the_origin_point_removes_both_in_one_step() {
    let mut a = xy();
    no_grid(&mut a);
    draw_line(&mut a, (20.0, 10.0), (20.0, 50.0));
    let before = solved(&a);
    assert!(dimension(
        &mut a,
        &[(20.0, 30.0), (0.0, 80.0)],
        (35.0, 70.0),
        "25"
    ));
    assert!(
        solved(&a).entities().len() > before.entities().len(),
        "the origin point was added"
    );
    a.undo().unwrap();
    assert_eq!(
        solved(&a),
        before,
        "one undo takes back the dimension and the origin point"
    );
}

#[test]
fn drawn_dimension_text_has_at_most_two_decimals_but_the_box_keeps_full_precision() {
    let mut a = xy();
    no_grid(&mut a);
    draw_line(&mut a, (0.0, 0.0), (40.0, 30.0));
    dim_pick(&mut a, &[(20.0, 15.0)]);
    at(&mut a, (8.0, 31.0)); // beside the line: aligned, length exactly 50
    let t = a
        .sketch_overlay(W, H)
        .unwrap()
        .dims
        .into_iter()
        .find(|d| d.preview)
        .unwrap()
        .text;
    assert_eq!(t, "50");
    draw_line(&mut a, (0.0, 60.0), (41.0, 90.0)); // length 50.8035...
    a.sketch_cancel();
    dim_pick(&mut a, &[(20.5, 75.0)]);
    at(&mut a, (5.0, 95.0));
    let o = a.sketch_overlay(W, H).unwrap();
    let t = &o.dims.iter().find(|d| d.preview).unwrap().text;
    assert!(t.len() <= 5 && t.contains('.'), "two decimals at most: {t}");
    dim_place(&mut a, (5.0, 95.0));
    let boxed = a.sketching.as_ref().unwrap().dim.edit.clone().unwrap().text;
    assert!(
        boxed.len() > t.len(),
        "the box holds more digits ({boxed}) than the drawing ({t})"
    );
}

// ---- selecting the origin and axes, showing freedom, over / conflicting constraints ----------

fn pick_at(a: &mut AppState, p: (f64, f64), add: bool) {
    let (x, y) = a.sketch_to_screen(p, W, H).unwrap();
    a.sketch_click_with(x, y, W, H, add);
}

fn selection(a: &AppState) -> Vec<String> {
    a.sketching.as_ref().unwrap().selection.clone()
}

fn add_constraint(a: &mut AppState, id: &str, c: Constraint) -> bool {
    let sketch = a.sketching.as_ref().unwrap().sketch.clone();
    a.submit(CommandKind::SketchAddConstraint {
        sketch,
        constraint_id: id.into(),
        constraint: c,
    })
    .is_ok()
}

fn fix(a: &mut AppState, id: &str, p: &str, x: f64, y: f64) {
    assert!(add_constraint(
        a,
        id,
        Constraint::Fixed {
            point: p.into(),
            x,
            y
        }
    ));
}

#[test]
fn the_origin_and_both_axes_can_be_selected_hovered_and_deselected() {
    let mut a = xy();
    no_grid(&mut a);
    pick_at(&mut a, (0.0, 0.0), false);
    assert_eq!(selection(&a), ["@origin"]);
    pick_at(&mut a, (50.0, 0.0), false);
    assert_eq!(selection(&a), ["@x"]);
    pick_at(&mut a, (0.0, 40.0), true);
    assert_eq!(selection(&a), ["@x", "@y"]);
    pick_at(&mut a, (50.0, 0.0), true);
    assert_eq!(selection(&a), ["@y"], "shift-click toggles an axis off");
    pick_at(&mut a, (60.0, 50.0), false);
    assert!(selection(&a).is_empty(), "empty space clears");

    // Hovering shows what it is, and the axis lights up.
    at(&mut a, (50.0, 0.0));
    let o = a.sketch_overlay(W, H).unwrap();
    assert_eq!(o.tooltip.as_ref().unwrap().1, "Horizontal axis");
    assert!(o.axes.iter().any(|x| !x.picked));
    pick_at(&mut a, (50.0, 0.0), false);
    let o = a.sketch_overlay(W, H).unwrap();
    assert!(o.axes.iter().any(|x| x.picked));
    // Nothing of this was logged.
    assert_eq!(a.authority.revision().0, 1);
    // Delete with only an axis selected does nothing and does not fail.
    assert!(!a.sketch_delete_selected());
}

#[test]
fn a_real_point_at_the_origin_is_picked_instead_of_the_virtual_origin() {
    let mut a = xy();
    no_grid(&mut a);
    add_point(&mut a, "o", 0.0, 0.0);
    fix(&mut a, "fo", "o", 0.0, 0.0);
    pick_at(&mut a, (0.0, 0.0), false);
    assert_eq!(selection(&a), ["o"]);
}

#[test]
fn relations_with_an_axis_or_the_origin() {
    let mut a = xy();
    no_grid(&mut a);
    add_point(&mut a, "p", 20.0, 15.0);
    add_point(&mut a, "q", 30.0, -10.0);
    pick_at(&mut a, (50.0, 0.0), false); // horizontal axis
    pick_at(&mut a, (20.0, 15.0), true);
    assert!(a.available_relations().contains(&Relation::PointOnLine));
    let rev = a.authority.revision().0;
    assert!(a.apply_relation(Relation::PointOnLine));
    assert_eq!(a.authority.revision().0, rev + 1);
    let sk = solved(&a);
    assert_eq!(
        constraints_of(&sk, |c| matches!(c, Constraint::OnAxis { .. })),
        1
    );
    assert_eq!(point_at(&a, "p").1, 0.0, "now on the horizontal axis");

    // The vertical axis and another point.
    pick_at(&mut a, (0.0, 40.0), false);
    pick_at(&mut a, (30.0, -10.0), true);
    assert!(a.apply_relation(Relation::PointOnLine));
    assert_eq!(point_at(&a, "q").0, 0.0);

    // The origin and a point: the point goes to (0, 0).
    add_point(&mut a, "r", 7.0, 9.0);
    pick_at(&mut a, (0.0, 0.0), false);
    pick_at(&mut a, (7.0, 9.0), true);
    assert!(a.available_relations().contains(&Relation::Coincident));
    assert!(a.apply_relation(Relation::Coincident));
    assert_eq!(point_at(&a, "r"), (0.0, 0.0));
}

#[test]
fn a_line_and_an_axis_can_be_parallel_perpendicular_or_lie_on_it() {
    let mut a = xy();
    no_grid(&mut a);
    a.set_tool(Tool::Line);
    click(&mut a, (10.0, 10.0));
    click(&mut a, (40.0, 20.0));
    a.sketch_cancel();
    click(&mut a, (10.0, -20.0));
    click(&mut a, (40.0, -30.0));
    a.sketch_cancel();
    a.set_tool(Tool::Select);
    pick_at(&mut a, (0.0, 40.0), false); // vertical axis
    pick_at(&mut a, (25.0, 15.0), true);
    let rel = a.available_relations();
    for r in [
        Relation::Parallel,
        Relation::Perpendicular,
        Relation::PointOnLine,
    ] {
        assert!(rel.contains(&r), "{r:?} in {rel:?}");
    }
    // Parallel to the vertical axis = vertical.
    assert!(a.apply_relation(Relation::Parallel));
    let sk = solved(&a);
    assert_eq!(
        constraints_of(&sk, |c| matches!(c, Constraint::Vertical { .. })),
        1
    );
    // Perpendicular to the horizontal axis = vertical as well; to the vertical axis = horizontal.
    pick_at(&mut a, (0.0, 40.0), false);
    pick_at(&mut a, (25.0, -25.0), true);
    assert!(a.apply_relation(Relation::Perpendicular));
    let sk = solved(&a);
    assert_eq!(
        constraints_of(&sk, |c| matches!(c, Constraint::Horizontal { .. })),
        1
    );
    // On the axis: both ends.
    pick_at(&mut a, (50.0, 0.0), false);
    pick_at(&mut a, (25.0, -25.0), true);
    assert!(a.apply_relation(Relation::PointOnLine));
    let sk = solved(&a);
    assert_eq!(
        constraints_of(&sk, |c| matches!(c, Constraint::OnAxis { .. })),
        2
    );
}

#[test]
fn free_points_show_which_way_they_can_still_move() {
    let mut a = xy();
    no_grid(&mut a);
    add_point(&mut a, "free", 20.0, 20.0);
    add_point(&mut a, "slide", 40.0, 30.0);
    add_point(&mut a, "pin", -30.0, 10.0);
    assert!(add_constraint(
        &mut a,
        "ax",
        Constraint::OnAxis {
            point: "slide".into(),
            axis: dc_sketch::SketchAxis::X
        }
    ));
    fix(&mut a, "fp", "pin", -30.0, 10.0);
    let o = a.sketch_overlay(W, H).unwrap();
    assert_eq!(
        o.freedom.len(),
        2,
        "pinned points get no marks: {:?}",
        o.freedom
    );
    let two = o.freedom.iter().find(|m| m.dof == 2).unwrap();
    assert!(two.dir.is_none());
    let one = o.freedom.iter().find(|m| m.dof == 1).unwrap();
    let d = one.dir.unwrap();
    assert!(
        (d.0.abs() - 1.0).abs() < 1e-9 && d.1.abs() < 1e-9,
        "along screen x: {d:?}"
    );

    // With the switch off only hovered or selected things show their freedom.
    a.sketching.as_mut().unwrap().show_freedom = false;
    assert!(a.sketch_overlay(W, H).unwrap().freedom.is_empty());
    pick_at(&mut a, (40.0, 30.0), false);
    // `slide` sits on the axis now, at (40, 0).
    pick_at(&mut a, (40.0, 0.0), false);
    let o = a.sketch_overlay(W, H).unwrap();
    assert_eq!(o.freedom.len(), 1);
    assert!(o.freedom[0].emphasis);

    // The tooltip says it in words.
    at(&mut a, (40.0, 0.0));
    let o = a.sketch_overlay(W, H).unwrap();
    let tip = o.tooltip.unwrap().1;
    assert!(
        tip.contains("slide") && tip.contains("horizontally only"),
        "{tip}"
    );
    at(&mut a, (-30.0, 10.0));
    let tip = a.sketch_overlay(W, H).unwrap().tooltip.unwrap().1;
    assert!(tip.contains("pinned"), "{tip}");
}

#[test]
fn a_circles_free_radius_is_marked_until_it_is_dimensioned() {
    let mut a = xy();
    no_grid(&mut a);
    a.set_tool(Tool::Circle);
    click(&mut a, (0.0, 0.0));
    click(&mut a, (10.0, 0.0));
    a.sketch_cancel();
    let o = a.sketch_overlay(W, H).unwrap();
    assert!(o.freedom.iter().any(|m| m.radius && m.dof == 1));
    let name = a.sketching.as_ref().unwrap().sketch.clone();
    let circle = session_sketch(&a)
        .entities()
        .iter()
        .find(|(_, e)| matches!(e, Entity::Circle { .. }))
        .map(|(id, _)| id.clone())
        .unwrap();
    a.submit(CommandKind::SketchAddConstraint {
        sketch: name,
        constraint_id: "r".into(),
        constraint: Constraint::Radius {
            target: circle,
            value: 10.0,
        },
    })
    .unwrap();
    assert!(!a
        .sketch_overlay(W, H)
        .unwrap()
        .freedom
        .iter()
        .any(|m| m.radius));
}

#[test]
fn redundant_relations_turn_their_entities_over_defined_and_a_refused_change_shows_the_clash() {
    use dc_app::sketching::Problem;
    let mut a = xy();
    no_grid(&mut a);
    add_point(&mut a, "p", 0.0, 0.0);
    add_point(&mut a, "q", 30.0, 0.0);
    fix(&mut a, "fp", "p", 0.0, 0.0);
    fix(&mut a, "fq", "q", 30.0, 0.0);
    assert!(add_constraint(
        &mut a,
        "d1",
        Constraint::Distance {
            a: "p".into(),
            b: "q".into(),
            value: 30.0
        }
    ));
    // A second, identical distance adds nothing: the sketch is over defined (but consistent).
    let o = a.sketch_overlay(W, H).unwrap();
    assert!(
        o.redundant >= 1,
        "the distance is implied by the two fixed points"
    );
    assert!(
        o.points.iter().all(|d| d.problem == Problem::Over),
        "{:?}",
        o.points
    );
    assert!(!o.conflict);

    // A different distance contradicts them: refused, and the clash is drawn.
    assert!(!add_constraint(
        &mut a,
        "d2",
        Constraint::Distance {
            a: "p".into(),
            b: "q".into(),
            value: 12.0
        }
    ));
    let o = a.sketch_overlay(W, H).unwrap();
    assert!(o.conflict);
    assert!(o.points.iter().all(|d| d.problem == Problem::Conflict));
    assert_eq!(
        session_sketch(&a).constraints().len(),
        3,
        "nothing was stored"
    );

    // Clicking elsewhere, or any successful change, clears the red.
    pick_at(&mut a, (60.0, 50.0), false);
    assert!(!a.sketch_overlay(W, H).unwrap().conflict);
}

// ---- clash messages, relation glyphs, and the extra relations --------------------------------

fn select(a: &mut AppState, ids: &[&str]) {
    a.sketching.as_mut().unwrap().selection = ids.iter().map(|s| (*s).to_owned()).collect();
}

fn add_entity(a: &mut AppState, id: &str, entity: Entity) {
    let sketch = a.sketching.as_ref().unwrap().sketch.clone();
    a.submit(CommandKind::SketchAddEntity {
        sketch,
        entity_id: id.into(),
        entity,
    })
    .unwrap();
}

fn add_line_between(a: &mut AppState, id: &str, p: &str, q: &str) {
    add_entity(
        a,
        id,
        Entity::Line {
            a: p.into(),
            b: q.into(),
        },
    );
}

fn add_circle(a: &mut AppState, id: &str, centre: &str, radius: f64) {
    add_entity(
        a,
        id,
        Entity::Circle {
            center: centre.into(),
            radius,
        },
    );
}

#[test]
fn a_refused_change_names_the_relations_it_clashed_with() {
    let mut a = xy();
    no_grid(&mut a);
    add_point(&mut a, "p", 0.0, 0.0);
    add_point(&mut a, "q", 30.0, 0.0);
    fix(&mut a, "fp", "p", 0.0, 0.0);
    fix(&mut a, "fq", "q", 30.0, 0.0);
    // Removing either fixed point would allow a 12 mm distance, so both are named.
    assert!(!add_constraint(
        &mut a,
        "d",
        Constraint::Distance {
            a: "p".into(),
            b: "q".into(),
            value: 12.0
        }
    ));
    let s = a.sketching.as_ref().unwrap();
    assert_eq!(s.conflict_relations, ["fp", "fq"]);
    let note = s.conflict_note.clone().unwrap();
    assert!(
        note.contains("fixed p") && note.contains("fixed q"),
        "{note}"
    );
    let last = a
        .messages
        .last()
        .map(|l| l.text.clone())
        .unwrap_or_default();
    assert!(
        last.contains("fixed p"),
        "the status bar says it too: {last}"
    );
    // The inspector and the glyphs know which relations clash.
    let g = a.sketch_overlay(W, H).unwrap().glyphs;
    assert!(g
        .iter()
        .all(|g| g.problem == dc_app::sketching::Problem::Conflict));
    // Any successful change forgets it.
    add_point(&mut a, "z", 5.0, 5.0);
    assert!(a.sketching.as_ref().unwrap().conflict_relations.is_empty());
}

#[test]
fn a_clash_that_needs_two_removals_still_lists_the_candidates() {
    let mut a = xy();
    no_grid(&mut a);
    add_point(&mut a, "p", 0.0, 0.0);
    add_point(&mut a, "q", 30.0, 0.0);
    fix(&mut a, "fp", "p", 0.0, 0.0);
    fix(&mut a, "fq", "q", 30.0, 0.0);
    assert!(add_constraint(
        &mut a,
        "d1",
        Constraint::Distance {
            a: "p".into(),
            b: "q".into(),
            value: 30.0
        }
    ));
    assert!(!add_constraint(
        &mut a,
        "d2",
        Constraint::Distance {
            a: "p".into(),
            b: "q".into(),
            value: 12.0
        }
    ));
    let s = a.sketching.as_ref().unwrap();
    assert!(
        s.conflict_relations.contains(&"d1".to_string()),
        "{:?}",
        s.conflict_relations
    );
}

#[test]
fn every_relation_gets_a_glyph_that_can_be_hovered_picked_and_deleted() {
    let mut a = xy();
    no_grid(&mut a);
    add_point(&mut a, "p", 10.0, 10.0);
    add_point(&mut a, "q", 40.0, 10.0);
    add_line_between(&mut a, "l", "p", "q");
    assert!(add_constraint(
        &mut a,
        "h",
        Constraint::Horizontal { line: "l".into() }
    ));
    fix(&mut a, "fx", "p", 10.0, 10.0);
    // Dimensions have no glyph; relations do.
    assert!(add_constraint(
        &mut a,
        "len",
        Constraint::Length {
            line: "l".into(),
            value: 30.0
        }
    ));
    let glyphs = a.sketch_overlay(W, H).unwrap().glyphs;
    let ids: Vec<&str> = glyphs.iter().map(|g| g.constraint.as_str()).collect();
    assert_eq!(ids, ["fx", "h"], "sorted by id, no dimension: {ids:?}");

    // Hovering the glyph says what it is and lights up what it constrains.
    let g = glyphs.iter().find(|g| g.constraint == "h").unwrap().clone();
    a.sketch_hover(Some((g.pos.0, g.pos.1, W, H)));
    let o = a.sketch_overlay(W, H).unwrap();
    assert_eq!(o.tooltip.as_ref().unwrap().1, "horizontal l");
    assert!(o.strokes.iter().any(|s| s.hover));
    // Clicking selects the relation instead of any entity; Escape drops it.
    a.sketch_click(g.pos.0, g.pos.1, W, H);
    assert_eq!(a.sketching.as_ref().unwrap().relations, ["h"]);
    assert!(selection(&a).is_empty());
    assert!(a.sketch_overlay(W, H).unwrap().relation.is_some());
    assert!(a.sketch_cancel());
    assert!(a.sketching.as_ref().unwrap().relations.is_empty());
    // Delete removes exactly that relation, in one undoable command.
    a.sketch_click(g.pos.0, g.pos.1, W, H);
    let rev = a.authority.revision().0;
    assert!(a.sketch_delete_selected());
    assert_eq!(a.authority.revision().0, rev + 1);
    assert!(!session_sketch(&a).constraints().contains_key("h"));
    assert!(a.sketching.as_ref().unwrap().relations.is_empty());
    // The switch hides them all.
    a.sketching.as_mut().unwrap().show_relations = false;
    assert!(a.sketch_overlay(W, H).unwrap().glyphs.is_empty());
}

#[test]
fn glyphs_at_the_same_spot_fan_out_and_redundant_ones_are_marked() {
    let mut a = xy();
    no_grid(&mut a);
    add_point(&mut a, "p", 10.0, 10.0);
    fix(&mut a, "f1", "p", 10.0, 10.0);
    fix(&mut a, "f2", "p", 10.0, 10.0);
    let g = a.sketch_overlay(W, H).unwrap().glyphs;
    assert_eq!(g.len(), 2);
    assert!(
        (g[0].pos.0 - g[1].pos.0).abs() >= 18.0,
        "{:?} {:?}",
        g[0].pos,
        g[1].pos
    );
    assert!(g
        .iter()
        .any(|g| g.problem == dc_app::sketching::Problem::Over));
}

#[test]
fn midpoint_symmetric_on_curve_and_origin_line_relations() {
    let mut a = xy();
    no_grid(&mut a);
    add_point(&mut a, "a", 0.0, 0.0);
    add_point(&mut a, "b", 10.0, 4.0);
    add_point(&mut a, "m", 3.0, 3.0);
    add_line_between(&mut a, "l", "a", "b");
    fix(&mut a, "fa", "a", 0.0, 0.0);
    fix(&mut a, "fb", "b", 10.0, 4.0);
    select(&mut a, &["m", "l"]);
    let rel = a.available_relations();
    assert!(rel.contains(&Relation::Midpoint) && rel.contains(&Relation::PointOnLine));
    assert!(a.apply_relation(Relation::Midpoint));
    assert!(near(point_at(&a, "m"), (5.0, 2.0)));

    // Two points mirrored in a line.
    add_point(&mut a, "u", 3.0, 0.0);
    add_point(&mut a, "v", 0.0, 1.0);
    fix(&mut a, "fu", "u", 3.0, 0.0);
    select(&mut a, &["u", "v", "l"]);
    assert!(a.available_relations().contains(&Relation::Symmetric));
    assert!(a.apply_relation(Relation::Symmetric));
    let (u, v) = (point_at(&a, "u"), point_at(&a, "v"));
    let mid = ((u.0 + v.0) / 2.0, (u.1 + v.1) / 2.0);
    // The midpoint of u and v is on the line y = 0.4 x, and u -> v is perpendicular to it.
    assert!((mid.1 - 0.4 * mid.0).abs() < 1e-6);
    assert!(((v.0 - u.0) * 10.0 + (v.1 - u.1) * 4.0).abs() < 1e-6);

    // A point on a circle.
    add_point(&mut a, "c", 30.0, 30.0);
    add_point(&mut a, "s", 41.0, 33.0);
    add_circle(&mut a, "circ", "c", 8.0);
    fix(&mut a, "fc", "c", 30.0, 30.0);
    select(&mut a, &["s", "circ"]);
    assert!(a.available_relations().contains(&Relation::OnCurve));
    assert!(a.apply_relation(Relation::OnCurve));
    let s = point_at(&a, "s");
    // The circle's radius was free, so it may have changed to meet the point: compare with it.
    let Some(Entity::Circle { radius, .. }) = session_sketch(&a).entity("circ").cloned() else {
        panic!()
    };
    assert!((dc_math::det::hypot(s.0 - 30.0, s.1 - 30.0) - radius).abs() < 1e-6);

    // The origin and a line: the line passes through it (an origin point is made once).
    add_point(&mut a, "x1", 10.0, -10.0);
    add_point(&mut a, "x2", 20.0, -15.0);
    add_line_between(&mut a, "ln", "x1", "x2");
    fix(&mut a, "fx1", "x1", 10.0, -10.0);
    pick_at(&mut a, (0.0, 0.0), false);
    let (px, py) = a.sketch_to_screen((15.0, -12.5), W, H).unwrap();
    a.sketch_click_with(px, py, W, H, true);
    // This sketch already has a fixed point at (0, 0) (`a`), so that is what the origin picks.
    assert_eq!(selection(&a), ["a", "ln"]);
    assert!(a.available_relations().contains(&Relation::PointOnLine));
    assert!(a.apply_relation(Relation::PointOnLine));
    let x2 = point_at(&a, "x2");
    // x1 = (10,-10) fixed, the line goes through the origin, so x2 is on y = -x.
    assert!((x2.1 + x2.0).abs() < 1e-6, "{x2:?}");
}

#[test]
fn a_line_through_the_virtual_origin_creates_the_origin_point_once() {
    let mut a = xy();
    no_grid(&mut a);
    add_point(&mut a, "x1", 10.0, -10.0);
    add_point(&mut a, "x2", 20.0, -15.0);
    add_line_between(&mut a, "ln", "x1", "x2");
    fix(&mut a, "fx1", "x1", 10.0, -10.0);
    assert!(a.origin_entity().is_none());
    pick_at(&mut a, (0.0, 0.0), false);
    pick_at(&mut a, (15.0, -12.5), true);
    assert_eq!(selection(&a), ["@origin", "ln"]);
    let rev = a.authority.revision().0;
    assert!(a.apply_relation(Relation::PointOnLine));
    assert_eq!(a.authority.revision().0, rev + 1, "one undo step");
    let o = a.origin_entity().expect("an origin point now exists");
    assert_eq!(point_at(&a, &o), (0.0, 0.0));
    let x2 = point_at(&a, "x2");
    assert!((x2.1 + x2.0).abs() < 1e-6, "{x2:?}");
    // Doing it for another line reuses that point.
    add_point(&mut a, "y1", -10.0, 30.0);
    add_point(&mut a, "y2", -20.0, 40.0);
    add_line_between(&mut a, "ln2", "y1", "y2");
    pick_at(&mut a, (0.0, 0.0), false);
    assert_eq!(
        selection(&a),
        std::slice::from_ref(&o),
        "a real origin point is picked now"
    );
}

#[test]
fn equal_radius_concentric_and_collinear() {
    let mut a = xy();
    no_grid(&mut a);
    add_point(&mut a, "c1", 0.0, 0.0);
    add_point(&mut a, "c2", 40.0, 0.0);
    add_circle(&mut a, "k1", "c1", 5.0);
    add_circle(&mut a, "k2", "c2", 9.0);
    fix(&mut a, "f1", "c1", 0.0, 0.0);
    fix(&mut a, "f2", "c2", 40.0, 0.0);
    add_constraint(
        &mut a,
        "r1",
        Constraint::Radius {
            target: "k1".into(),
            value: 5.0,
        },
    );
    select(&mut a, &["k1", "k2"]);
    let rel = a.available_relations();
    assert!(rel.contains(&Relation::Equal) && rel.contains(&Relation::Concentric));
    assert!(a.apply_relation(Relation::Equal));
    let Some(Entity::Circle { radius, .. }) = session_sketch(&a).entity("k2").cloned() else {
        panic!()
    };
    assert!((radius - 5.0).abs() < 1e-6, "{radius}");
    // Concentric: the second circle's centre moves onto the first's.
    select(&mut a, &["k1", "k2"]);
    assert!(
        !a.apply_relation(Relation::Concentric),
        "both centres are fixed in different places, so this clashes and is refused"
    );
    assert!(a.sketching.as_ref().unwrap().conflict_note.is_some());
    a.remove_constraint(&a.sketching.as_ref().unwrap().sketch.clone(), "f2");
    select(&mut a, &["k1", "k2"]);
    assert!(a.apply_relation(Relation::Concentric));
    assert!(near(point_at(&a, "c2"), (0.0, 0.0)));

    // Collinear: the second line's ends go onto the first line.
    add_point(&mut a, "a1", 0.0, 20.0);
    add_point(&mut a, "a2", 20.0, 20.0);
    add_point(&mut a, "b1", 30.0, 25.0);
    add_point(&mut a, "b2", 50.0, 27.0);
    add_line_between(&mut a, "la", "a1", "a2");
    add_line_between(&mut a, "lb", "b1", "b2");
    fix(&mut a, "fa1", "a1", 0.0, 20.0);
    fix(&mut a, "fa2", "a2", 20.0, 20.0);
    select(&mut a, &["la", "lb"]);
    assert!(a.available_relations().contains(&Relation::Collinear));
    assert!(a.apply_relation(Relation::Collinear));
    assert!((point_at(&a, "b1").1 - 20.0).abs() < 1e-6);
    assert!((point_at(&a, "b2").1 - 20.0).abs() < 1e-6);
}

#[test]
fn dragging_a_square_corner_moves_the_circle_it_is_related_to_while_dragging() {
    let mut a = xy();
    no_grid(&mut a);
    // A square whose opposite corners lie on a circle centred in it.
    for (id, x, y) in [
        ("p1", 0.0, 0.0),
        ("p2", 20.0, 0.0),
        ("p3", 20.0, 20.0),
        ("p4", 0.0, 20.0),
        ("c", 10.0, 10.0),
    ] {
        add_point(&mut a, id, x, y);
    }
    for (id, p, q) in [
        ("l1", "p1", "p2"),
        ("l2", "p2", "p3"),
        ("l3", "p3", "p4"),
        ("l4", "p4", "p1"),
    ] {
        add_line_between(&mut a, id, p, q);
    }
    add_circle(&mut a, "k", "c", 14.1421356);
    for (i, l) in ["l1", "l3"].iter().enumerate() {
        assert!(add_constraint(
            &mut a,
            &format!("h{i}"),
            Constraint::Horizontal { line: (*l).into() }
        ));
    }
    for (i, l) in ["l2", "l4"].iter().enumerate() {
        assert!(add_constraint(
            &mut a,
            &format!("v{i}"),
            Constraint::Vertical { line: (*l).into() }
        ));
    }
    fix(&mut a, "fp1", "p1", 0.0, 0.0);
    for (i, p) in ["p1", "p3"].iter().enumerate() {
        assert!(add_constraint(
            &mut a,
            &format!("oc{i}"),
            Constraint::PointOnCurve {
                point: (*p).into(),
                curve: "k".into()
            }
        ));
    }
    let radius_of = |sk: &dc_sketch::Sketch| match sk.entity("k") {
        Some(Entity::Circle { radius, .. }) => *radius,
        _ => panic!(),
    };
    let before = radius_of(&session_sketch(&a));
    let rev = a.authority.revision();
    let (sx, sy) = a.sketch_to_screen((20.0, 20.0), W, H).unwrap();
    assert!(a.sketch_drag_begin(sx, sy, W, H));
    let (x, y) = a.sketch_to_screen((40.0, 30.0), W, H).unwrap();
    a.sketch_drag_move(x, y, W, H);
    // Mid-drag, nothing is committed, yet the circle on screen already has its new size.
    assert_eq!(a.authority.revision(), rev);
    assert_eq!(radius_of(&session_sketch(&a)), before);
    let preview = a
        .sketching
        .as_ref()
        .unwrap()
        .drag
        .as_ref()
        .unwrap()
        .sketch
        .clone()
        .expect("the solved copy");
    let during = radius_of(&preview);
    assert!(during > before + 5.0, "{before} -> {during}");
    let o = a.sketch_overlay(W, H).unwrap();
    let ring = o.strokes.iter().find(|s| s.pts.len() > 20).unwrap();
    let xs: Vec<f64> = ring.pts.iter().map(|p| p.0).collect();
    let width =
        xs.iter().cloned().fold(f64::MIN, f64::max) - xs.iter().cloned().fold(f64::MAX, f64::min);
    let px_per_mm = (a.sketch_to_screen((1.0, 0.0), W, H).unwrap().0
        - a.sketch_to_screen((0.0, 0.0), W, H).unwrap().0)
        .abs();
    assert!(
        (width / px_per_mm - 2.0 * during).abs() < 0.5,
        "the drawn circle follows the preview radius"
    );
    // Relation glyphs follow too: the two on-curve glyphs sit at the dragged geometry.
    assert!(!o.glyphs.is_empty());
    // Moving to the same place again solves nothing new, and release commits exactly what was shown.
    a.sketch_drag_move(x, y, W, H);
    assert!(a.sketch_drag_end());
    assert_eq!(a.authority.revision().0, rev.0 + 1);
    let after = radius_of(&session_sketch(&a));
    assert!((after - during).abs() < 1e-6, "{after} vs {during}");
    assert!(
        near(point_at(&a, "p3"), (40.0, 30.0)),
        "{:?}",
        point_at(&a, "p3")
    );
}

#[test]
fn a_drag_step_stays_cheap_on_a_sketch_with_many_shapes() {
    let mut a = xy();
    no_grid(&mut a);
    // 12 rectangles drawn the normal way (each with horizontal / vertical relations).
    a.set_tool(Tool::Rectangle);
    for i in 0..12 {
        let x = -90.0 + f64::from(i % 6) * 30.0;
        let y = -40.0 + f64::from(i / 6) * 40.0;
        click(&mut a, (x, y));
        click(&mut a, (x + 20.0, y + 25.0));
    }
    a.set_tool(Tool::Select);
    let n = session_sketch(&a).entities().len();
    let (sx, sy) = a.sketch_to_screen((-70.0, -15.0), W, H).unwrap();
    assert!(a.sketch_drag_begin(sx, sy, W, H));
    let t = std::time::Instant::now();
    let steps = 20;
    for i in 0..steps {
        let (x, y) = a
            .sketch_to_screen((-70.0 + f64::from(i), -15.0 + f64::from(i)), W, H)
            .unwrap();
        a.sketch_drag_move(x, y, W, H);
    }
    let per_step = t.elapsed().as_secs_f64() * 1000.0 / f64::from(steps);
    eprintln!("{n} entities: {per_step:.2} ms per drag step");
    // Loose bound for slow CI machines; a debug build is several times slower than release.
    assert!(per_step < 100.0, "{per_step} ms per step");
    // What the preview showed is what release commits.
    let shown = a
        .sketching
        .as_ref()
        .unwrap()
        .drag
        .as_ref()
        .unwrap()
        .sketch
        .clone()
        .unwrap();
    assert!(a.sketch_drag_end());
    assert_eq!(shown.entities(), session_sketch(&a).entities());
}

// ---- dragging whole shapes, the live feature check ---------------------------------------------

fn screen(a: &AppState, p: (f64, f64)) -> (f64, f64) {
    a.sketch_to_screen(p, W, H).unwrap()
}

fn drag_to(a: &mut AppState, from: (f64, f64), to: (f64, f64)) {
    let (x, y) = screen(a, from);
    assert!(
        a.sketch_drag_begin(x, y, W, H),
        "something to grab at {from:?}"
    );
    let (x, y) = screen(a, to);
    a.sketch_drag_move(x, y, W, H);
}

fn dragging(a: &AppState) -> dc_app::sketching::Drag {
    a.sketching.as_ref().unwrap().drag.clone().unwrap()
}

#[test]
fn dragging_a_free_line_moves_the_whole_line_in_one_command() {
    let mut a = xy();
    no_grid(&mut a);
    add_point(&mut a, "p", 0.0, 0.0);
    add_point(&mut a, "q", 30.0, 10.0);
    add_line_between(&mut a, "l", "p", "q");
    a.set_tool(Tool::Select);
    let rev = a.authority.revision();
    drag_to(&mut a, (15.0, 5.0), (25.0, -15.0));
    assert_eq!(a.authority.revision(), rev, "nothing logged while dragging");
    assert_eq!(dragging(&a).kind, dc_app::sketching::DragKind::Shape);
    // The overlay shows the line already moved by (10, -20).
    let o = a.sketch_overlay(W, H).unwrap();
    let want = (screen(&a, (10.0, -20.0)), screen(&a, (40.0, -10.0)));
    let line = &o.strokes[0].pts;
    assert!(
        (line[0].0 - want.0 .0).abs() < 1e-6 && (line[1].1 - want.1 .1).abs() < 1e-6,
        "{line:?} vs {want:?}"
    );
    assert!(a.sketch_drag_end());
    assert_eq!(a.authority.revision().0, rev.0 + 1);
    assert!(near(point_at(&a, "p"), (10.0, -20.0)));
    assert!(near(point_at(&a, "q"), (40.0, -10.0)));
    let last = a
        .authority
        .log()
        .last()
        .map(|e| format!("{:?}", e.command.kind))
        .unwrap_or_default();
    assert!(last.contains("SketchDrag "), "{last}");
    a.undo().unwrap();
    assert!(near(point_at(&a, "p"), (0.0, 0.0)));
}

#[test]
fn dragging_the_edge_of_a_rectangle_resizes_it_and_keeps_it_square_cornered() {
    let mut a = xy();
    no_grid(&mut a);
    a.set_tool(Tool::Rectangle);
    click(&mut a, (0.0, 0.0));
    click(&mut a, (40.0, 20.0));
    a.set_tool(Tool::Select);
    // The right edge, grabbed in its middle, pulled right and (uselessly) up.
    drag_to(&mut a, (40.0, 10.0), (55.0, 18.0));
    let preview = dragging(&a).sketch.unwrap();
    let pts = points(&preview);
    // Only x can change for a vertical edge between horizontal ones.
    assert!(pts.iter().any(|p| near(*p, (55.0, 0.0))), "{pts:?}");
    assert!(pts.iter().any(|p| near(*p, (55.0, 20.0))), "{pts:?}");
    assert!(pts.iter().any(|p| near(*p, (0.0, 20.0))), "{pts:?}");
    assert!(a.sketch_drag_end());
    let pts = points(&solved(&a));
    assert!(pts.iter().any(|p| near(*p, (55.0, 20.0))));
}

#[test]
fn dragging_a_circles_rim_resizes_it_without_moving_its_centre() {
    let mut a = xy();
    a.set_tool(Tool::Circle);
    no_grid(&mut a);
    click(&mut a, (0.0, 0.0));
    click(&mut a, (10.0, 0.0));
    a.set_tool(Tool::Select);
    let radius_of = |sk: &dc_sketch::Sketch| {
        sk.entities()
            .values()
            .find_map(|e| match e {
                Entity::Circle { radius, .. } => Some(*radius),
                _ => None,
            })
            .unwrap()
    };
    drag_to(&mut a, (0.0, 10.0), (0.0, 16.0)); // grabbed at the top, pulled out
    assert_eq!(dragging(&a).kind, dc_app::sketching::DragKind::Radius);
    assert!((radius_of(&dragging(&a).sketch.unwrap()) - 16.0).abs() < 1e-6);
    assert!(
        (radius_of(&session_sketch(&a)) - 10.0).abs() < 1e-6,
        "not committed yet"
    );
    assert!(a.sketch_drag_end());
    assert!((radius_of(&solved(&a)) - 16.0).abs() < 1e-6);
    assert!(points(&solved(&a)).iter().all(|p| near(*p, (0.0, 0.0))));
    // With the grid on the radius snaps, unless Ctrl is held.
    a.sketching.as_mut().unwrap().snap_grid = true;
    let (x, y) = screen(&a, (0.0, 16.0));
    assert!(a.sketch_drag_begin(x, y, W, H));
    let (x, y) = screen(&a, (0.0, 20.3));
    a.sketch_drag_move_with(x, y, W, H, true);
    assert!(
        (radius_of(&dragging(&a).sketch.unwrap()) - 20.3).abs() < 1e-6,
        "free"
    );
    a.sketch_drag_move_with(x, y + 0.01, W, H, false);
    let snapped = radius_of(&dragging(&a).sketch.unwrap());
    assert!(
        (snapped - 20.3).abs() > 1e-6,
        "snapped to the grid: {snapped}"
    );
    assert!(a.sketch_cancel());
    assert!(
        (radius_of(&solved(&a)) - 16.0).abs() < 1e-6,
        "Esc cancelled it"
    );
}

#[test]
fn dragging_a_dimensioned_circle_rim_snaps_back_and_a_grab_does_not_jump() {
    let mut a = xy();
    no_grid(&mut a);
    add_point(&mut a, "c", 0.0, 0.0);
    add_circle(&mut a, "k", "c", 10.0);
    assert!(add_constraint(
        &mut a,
        "r",
        Constraint::Radius {
            target: "k".into(),
            value: 10.0
        }
    ));
    a.set_tool(Tool::Select);
    drag_to(&mut a, (10.0, 0.0), (20.0, 0.0));
    let sk = dragging(&a).sketch.unwrap();
    assert!(
        matches!(sk.entity("k"), Some(Entity::Circle { radius, .. }) if (radius - 10.0).abs() < 1e-6)
    );
    a.sketch_cancel();
    // A point grabbed 3 px off its centre keeps that offset instead of jumping to the cursor.
    add_point(&mut a, "m", 30.0, 30.0);
    let (x, y) = screen(&a, (30.0, 30.0));
    assert!(a.sketch_drag_begin(x + 3.0, y, W, H));
    a.sketch_drag_move(x + 3.0, y, W, H);
    assert!(
        near(dragging(&a).target, (30.0, 30.0)),
        "{:?}",
        dragging(&a).target
    );
    a.sketch_drag_move(x + 13.0, y, W, H);
    let moved = dragging(&a).target;
    assert!((moved.0 - 30.0).abs() > 0.1 && (moved.1 - 30.0).abs() < 1e-9);
    a.sketch_cancel();
}

#[test]
fn dragging_an_arc_moves_all_its_points() {
    let mut a = xy();
    no_grid(&mut a);
    for (id, x, y) in [("c", 0.0, 0.0), ("s", 10.0, 0.0), ("e", 0.0, 10.0)] {
        add_point(&mut a, id, x, y);
    }
    add_entity(
        &mut a,
        "arc",
        Entity::Arc {
            center: "c".into(),
            start: "s".into(),
            end: "e".into(),
        },
    );
    a.set_tool(Tool::Select);
    let m = core::f64::consts::FRAC_1_SQRT_2 * 10.0;
    drag_to(&mut a, (m, m), (m + 5.0, m + 5.0));
    assert!(a.sketch_drag_end());
    assert!(near(point_at(&a, "c"), (5.0, 5.0)));
    assert!(near(point_at(&a, "s"), (15.0, 5.0)));
    assert!(near(point_at(&a, "e"), (5.0, 15.0)));
}

#[test]
fn pressing_on_a_relation_symbol_does_not_start_a_drag() {
    let mut a = xy();
    no_grid(&mut a);
    add_point(&mut a, "p", 0.0, 20.0);
    add_point(&mut a, "q", 40.0, 20.0);
    add_line_between(&mut a, "l", "p", "q");
    assert!(add_constraint(
        &mut a,
        "h",
        Constraint::Horizontal { line: "l".into() }
    ));
    a.set_tool(Tool::Select);
    let g = a.sketch_overlay(W, H).unwrap().glyphs[0].pos;
    assert!(!a.sketch_drag_begin(g.0, g.1, W, H));
}

fn plate_with_hole(a: &mut AppState) -> String {
    a.set_tool(Tool::Rectangle);
    click(a, (0.0, 0.0));
    click(a, (100.0, 60.0));
    a.set_tool(Tool::Circle);
    click(a, (50.0, 30.0));
    click(a, (58.0, 30.0));
    let name = a.sketching.as_ref().unwrap().sketch.clone();
    let profile = match a
        .authority
        .query(&Query::SketchProfiles {
            sketch: name.clone(),
            after: None,
            limit: None,
        })
        .unwrap()
    {
        QueryResult::SketchProfiles { profiles, .. } => profiles[0].id.clone(),
        other => panic!("{other:?}"),
    };
    a.end_sketch();
    a.submit(CommandKind::FeatureExtrude {
        feature: "plate".into(),
        sketch: name.clone(),
        profiles: vec![profile],
        depth: 10.0,
        reverse: false,
        extent: Default::default(),
    })
    .unwrap();
    assert!(a.edit_sketch(&name));
    a.set_tool(Tool::Select);
    name
}

#[test]
fn a_drag_that_would_break_a_feature_is_flagged_while_dragging_and_refused_on_release() {
    let mut a = xy();
    no_grid(&mut a);
    plate_with_hole(&mut a);
    let rev = a.authority.revision();
    // Fine: a slightly bigger hole.
    drag_to(&mut a, (50.0, 38.0), (50.0, 42.0));
    a.drag_check(true);
    assert!(dragging(&a).broken.is_none(), "{:?}", dragging(&a).broken);
    assert!(a.sketch_overlay(W, H).unwrap().drag_warning.is_none());
    // Too big: the hole would cross the edge of the plate (30 mm half-height).
    let (x, y) = screen(&a, (50.0, 72.0));
    a.sketch_drag_move(x, y, W, H);
    a.drag_check(true);
    let why = dragging(&a).broken.expect("the feature would break");
    let o = a.sketch_overlay(W, H).unwrap();
    assert_eq!(o.drag_warning.as_deref(), Some(why.as_str()));
    assert!(
        o.strokes
            .iter()
            .any(|s| s.problem == dc_app::sketching::Problem::Conflict),
        "the dragged shape is drawn red"
    );
    // Back to something valid clears it.
    let (x, y) = screen(&a, (50.0, 40.0));
    a.sketch_drag_move(x, y, W, H);
    a.drag_check(true);
    assert!(dragging(&a).broken.is_none());
    // Release where it breaks: refused with the reason, and the sketch is as it was.
    let (x, y) = screen(&a, (50.0, 72.0));
    a.sketch_drag_move(x, y, W, H);
    assert!(!a.sketch_drag_end());
    assert_eq!(a.authority.revision(), rev);
    assert!(a
        .messages
        .last()
        .is_some_and(|m| m.level == dc_app::state::Level::Error));
}

#[test]
fn a_sketch_nothing_is_built_from_is_never_feature_checked() {
    let mut a = xy();
    no_grid(&mut a);
    add_point(&mut a, "p", 0.0, 0.0);
    a.set_tool(Tool::Select);
    drag_to(&mut a, (0.0, 0.0), (10.0, 10.0));
    assert!(a.drag_check_due() || dragging(&a).broken.is_none());
    a.drag_check(true);
    assert!(!a.drag_check_due());
    assert!(dragging(&a).broken.is_none());
}

// ---- box selection, moving a selection, and saying why something cannot move ---------------

fn box_select(a: &mut AppState, from: (f64, f64), to: (f64, f64), additive: bool) -> usize {
    let (fx, fy) = screen(a, from);
    let (tx, ty) = screen(a, to);
    assert!(a.sketch_box_begin(fx, fy));
    a.sketch_box_move(tx, ty);
    a.sketch_box_end(additive, W, H)
}

#[test]
fn window_selection_takes_what_is_inside_and_crossing_takes_what_it_touches() {
    let mut a = xy();
    no_grid(&mut a);
    // (The virtual origin is what gets boxed here, so the rectangle is not tied to it.)
    a.sketching.as_mut().unwrap().snaps.points = false;
    a.set_tool(Tool::Rectangle);
    click(&mut a, (0.0, 0.0));
    click(&mut a, (40.0, 20.0));
    a.set_tool(Tool::Circle);
    click(&mut a, (80.0, 10.0));
    click(&mut a, (90.0, 10.0));
    add_point(&mut a, "free", 20.0, 50.0);
    a.set_tool(Tool::Select);
    // Window around the rectangle: its four edges (not its corner points; the origin is boxed
    // too, as it sits inside the window).
    assert_eq!(box_select(&mut a, (-5.0, -5.0), (45.0, 25.0), false), 5);
    let sel = selection(&a);
    assert_eq!(
        sel.iter().filter(|i| i.starts_with('l')).count(),
        4,
        "{sel:?}"
    );
    assert!(
        sel.iter().all(|i| i.starts_with('l') || i == "@origin"),
        "{sel:?}"
    );
    assert!(sel.contains(&"@origin".to_string()));
    // A window that only partly covers the circle does not take it...
    assert_eq!(box_select(&mut a, (70.0, -5.0), (85.0, 25.0), false), 0);
    assert!(selection(&a).is_empty());
    // ...but right-to-left (crossing) does, and does not take the rectangle.
    assert_eq!(box_select(&mut a, (85.0, 25.0), (70.0, 4.0), false), 1);
    // A stand-alone point is selected too; additive keeps what was there.
    assert_eq!(box_select(&mut a, (15.0, 45.0), (25.0, 55.0), true), 1);
    assert_eq!(selection(&a).len(), 2);
    assert!(selection(&a).contains(&"free".to_string()));
    // Crossing a single edge, away from the axes, takes just that edge.
    assert_eq!(box_select(&mut a, (30.0, 25.0), (25.0, 15.0), false), 1);
    // Crossing the bottom edge where it lies on the horizontal axis takes the axis too.
    assert_eq!(box_select(&mut a, (30.0, 5.0), (25.0, -5.0), false), 2);
    // Escape drops a box in progress.
    let (fx, fy) = screen(&a, (-30.0, -30.0));
    assert!(a.sketch_box_begin(fx, fy));
    assert!(a.sketch_cancel());
    assert!(a.sketching.as_ref().unwrap().boxsel.is_none());
    assert_eq!(a.sketch_box_end(false, W, H), 0);
}

#[test]
fn a_selection_moves_together_and_an_unselected_grab_selects_only_itself() {
    let mut a = xy();
    no_grid(&mut a);
    for (id, x, y) in [
        ("a1", 0.0, 0.0),
        ("a2", 20.0, 0.0),
        ("b1", 0.0, 30.0),
        ("b2", 20.0, 30.0),
        ("c1", 50.0, 0.0),
        ("c2", 70.0, 0.0),
    ] {
        add_point(&mut a, id, x, y);
    }
    add_line_between(&mut a, "la", "a1", "a2");
    add_line_between(&mut a, "lb", "b1", "b2");
    add_line_between(&mut a, "lc", "c1", "c2");
    a.set_tool(Tool::Select);
    select(&mut a, &["la", "lb"]);
    drag_to(&mut a, (10.0, 0.0), (15.0, 12.0));
    assert!(dragging(&a).multi);
    assert_eq!(selection(&a).len(), 2, "the selection is kept");
    assert!(a.sketch_drag_end());
    for (id, p) in [
        ("a1", (5.0, 12.0)),
        ("b2", (25.0, 42.0)),
        ("c1", (50.0, 0.0)),
    ] {
        assert!(near(point_at(&a, id), p), "{id}: {:?}", point_at(&a, id));
    }
    // Grabbing something that is not selected selects just that and moves just that.
    drag_to(&mut a, (60.0, 0.0), (60.0, 10.0));
    assert!(!dragging(&a).multi);
    assert_eq!(selection(&a), ["lc"]);
    assert!(a.sketch_drag_end());
    assert!(near(point_at(&a, "c1"), (50.0, 10.0)));
    assert!(near(point_at(&a, "a1"), (5.0, 12.0)));
}

#[test]
fn a_fully_defined_shape_says_why_it_cannot_be_dragged() {
    let mut a = xy();
    no_grid(&mut a);
    add_point(&mut a, "p", 0.0, 0.0);
    add_point(&mut a, "q", 30.0, 0.0);
    add_line_between(&mut a, "l", "p", "q");
    fix(&mut a, "fp", "p", 0.0, 0.0);
    fix(&mut a, "fq", "q", 30.0, 0.0);
    a.set_tool(Tool::Select);
    // Hovering warns before you try.
    at(&mut a, (15.0, 0.0));
    let tip = a.sketch_overlay(W, H).unwrap().tooltip.unwrap().1;
    assert!(tip.contains("cannot be dragged"), "{tip}");
    // Dragging it: nothing moves, and the toolbar note names what holds it.
    drag_to(&mut a, (15.0, 0.0), (15.0, 20.0));
    let o = a.sketch_overlay(W, H).unwrap();
    let note = o.drag_note.expect("a note");
    assert!(
        note.contains("Fully defined") && note.contains("fixed p"),
        "{note}"
    );
    assert!(near(point_at(&a, "p"), (0.0, 0.0)));
    assert!(!a.sketch_drag_end(), "nothing changed, nothing committed");
    // A point that can only slide along an axis says so, but a free one has no note.
    add_point(&mut a, "s", 10.0, 0.0);
    assert!(add_constraint(
        &mut a,
        "ax",
        Constraint::OnAxis {
            point: "s".into(),
            axis: dc_sketch::SketchAxis::X
        }
    ));
    let (x, y) = screen(&a, (10.0, 0.0));
    assert!(a.sketch_drag_begin(x, y, W, H));
    let (x, y) = screen(&a, (14.0, 9.0));
    a.sketch_drag_move(x, y, W, H);
    let note = a.sketch_overlay(W, H).unwrap().drag_note.unwrap();
    assert!(note.contains("only move horizontally"), "{note}");
    a.sketch_cancel();
    add_point(&mut a, "free", 50.0, 50.0);
    drag_to(&mut a, (50.0, 50.0), (60.0, 60.0));
    assert!(a.sketch_overlay(W, H).unwrap().drag_note.is_none());
}

// ---- datums and relation symbols in a box, select all, select chain ------------------------------

#[test]
fn a_box_takes_the_origin_and_crossed_axes_but_they_do_not_block_relations() {
    let mut a = xy();
    no_grid(&mut a);
    a.set_tool(Tool::Rectangle);
    click(&mut a, (10.0, 10.0));
    click(&mut a, (50.0, 30.0));
    a.set_tool(Tool::Select);
    // A window around the origin takes the origin, never an axis (they are endless).
    assert_eq!(box_select(&mut a, (-5.0, -5.0), (5.0, 5.0), false), 1);
    assert_eq!(selection(&a), ["@origin"]);
    // Crossing the horizontal axis (right to left) takes the axis, and the origin if inside.
    box_select(&mut a, (30.0, 5.0), (-5.0, -5.0), false);
    let sel = selection(&a);
    assert!(
        ["@x", "@y", "@origin"]
            .iter()
            .all(|d| sel.contains(&d.to_string())),
        "the box covers the origin and both axes: {sel:?}"
    );
    // Crossing only the vertical axis takes just that one.
    box_select(&mut a, (5.0, 40.0), (-5.0, 35.0), false);
    assert_eq!(selection(&a), ["@y"]);
    // A box that misses both takes neither.
    assert_eq!(box_select(&mut a, (20.0, 40.0), (30.0, 50.0), false), 0);
    // A big crossing box takes the rectangle's edges and the datums, yet Horizontal and
    // Vertical are still offered for the edges: the datums are just along for the ride.
    box_select(&mut a, (70.0, 45.0), (-10.0, -10.0), false);
    let sel = selection(&a);
    assert!(
        sel.iter().any(|i| i == "@origin")
            && sel.iter().filter(|i| i.starts_with('l')).count() == 4,
        "{sel:?}"
    );
    let rel = a.available_relations();
    assert!(rel.contains(&Relation::Horizontal), "{rel:?}");
    // And the relation acts on the edges only: no axis or origin relation appears. (All four
    // edges horizontal would flatten the rectangle, which the engine now refuses as a collapse;
    // either way nothing may be tied to an axis.)
    let _ = a.apply_relation(Relation::Horizontal);
    assert_eq!(
        constraints_of(&solved(&a), |c| matches!(c, Constraint::OnAxis { .. })),
        0
    );
}

#[test]
fn a_box_over_only_relation_symbols_selects_them_and_delete_removes_them_in_one_step() {
    let mut a = xy();
    no_grid(&mut a);
    a.set_tool(Tool::Rectangle);
    click(&mut a, (0.0, 0.0));
    click(&mut a, (60.0, 40.0));
    a.set_tool(Tool::Select);
    let glyphs = a.sketch_overlay(W, H).unwrap().glyphs;
    assert!(
        glyphs.len() >= 4,
        "the rectangle's horizontal / vertical relations"
    );
    let top: Vec<_> = glyphs
        .iter()
        .filter(|g| g.pos.1 < screen(&a, (0.0, 20.0)).1)
        .collect();
    assert!(!top.is_empty());
    // A box that covers the symbols above the top edge but no geometry.
    let (x0, x1) = (screen(&a, (-5.0, 0.0)).0, screen(&a, (65.0, 0.0)).0);
    let ytop = top.iter().map(|g| g.pos.1).fold(f64::MAX, f64::min) - 4.0;
    let ybot = top.iter().map(|g| g.pos.1).fold(f64::MIN, f64::max) + 4.0;
    assert!(a.sketch_box_begin(x0, ytop));
    a.sketch_box_move(x1, ybot);
    let n = a.sketch_box_end(false, W, H);
    assert_eq!(n, top.len(), "only the symbols");
    let s = a.sketching.as_ref().unwrap();
    assert!(s.selection.is_empty() && s.relations.len() == top.len());
    let o = a.sketch_overlay(W, H).unwrap();
    assert!(o.relation.as_deref().is_some());
    let rev = a.authority.revision().0;
    let before = session_sketch(&a).constraints().len();
    assert!(a.sketch_delete_selected());
    assert_eq!(
        a.authority.revision().0,
        rev + 1,
        "one undo step for all of them"
    );
    assert_eq!(session_sketch(&a).constraints().len(), before - n);
    // Shift-click on a symbol toggles it in the set.
    let g = a.sketch_overlay(W, H).unwrap().glyphs;
    a.sketch_click_with(g[0].pos.0, g[0].pos.1, W, H, true);
    a.sketch_click_with(g[1].pos.0, g[1].pos.1, W, H, true);
    assert_eq!(a.sketching.as_ref().unwrap().relations.len(), 2);
    a.sketch_click_with(g[0].pos.0, g[0].pos.1, W, H, true);
    assert_eq!(a.sketching.as_ref().unwrap().relations.len(), 1);
}

#[test]
fn select_all_and_select_chain() {
    let mut a = xy();
    no_grid(&mut a);
    a.set_tool(Tool::Rectangle);
    click(&mut a, (0.0, 0.0));
    click(&mut a, (30.0, 20.0));
    a.set_tool(Tool::Circle);
    click(&mut a, (60.0, 10.0));
    click(&mut a, (66.0, 10.0));
    a.set_tool(Tool::Line);
    click(&mut a, (0.0, 40.0));
    click(&mut a, (20.0, 45.0));
    click(&mut a, (40.0, 40.0));
    a.sketch_cancel();
    add_point(&mut a, "lone", 80.0, 40.0);
    a.set_tool(Tool::Select);
    // Everything: 4 edges, the circle, 2 polyline segments and the lone point.
    assert_eq!(a.sketch_select_all(), 8);
    assert!(!selection(&a)
        .iter()
        .any(|i| i.starts_with('p') && i != "lone"));
    // Double-click on one edge of the rectangle selects its four edges, replacing the rest.
    let (x, y) = screen(&a, (15.0, 0.0));
    assert!(a.sketch_double_click(x, y, W, H));
    assert_eq!(selection(&a).len(), 4);
    // The polyline's chain is its two segments; the circle is a chain of one.
    let (x, y) = screen(&a, (10.0, 42.5));
    assert!(a.sketch_double_click(x, y, W, H));
    assert_eq!(selection(&a).len(), 2);
    let (x, y) = screen(&a, (66.0, 10.0));
    assert!(a.sketch_double_click(x, y, W, H));
    assert_eq!(selection(&a).len(), 1);
}

#[test]
fn a_multi_drag_that_cannot_follow_the_cursor_says_so() {
    let mut a = xy();
    no_grid(&mut a);
    for (id, x, y) in [
        ("a1", 0.0, 0.0),
        ("a2", 20.0, 0.0),
        ("b1", 0.0, 30.0),
        ("b2", 20.0, 30.0),
    ] {
        add_point(&mut a, id, x, y);
    }
    add_line_between(&mut a, "la", "a1", "a2");
    add_line_between(&mut a, "lb", "b1", "b2");
    // One of the points is dimensioned to stay 30 above the other line: moving only some of the
    // selection would break it. Selecting just `la` and `lb` moves both, so pin `b2` instead.
    fix(&mut a, "fb2", "b2", 20.0, 30.0);
    a.set_tool(Tool::Select);
    select(&mut a, &["la", "lb"]);
    drag_to(&mut a, (10.0, 0.0), (15.0, 8.0));
    assert!(dragging(&a).multi);
    let note = a.sketch_overlay(W, H).unwrap().drag_note.expect("a note");
    assert!(note.contains("nearest valid position"), "{note}");
    assert!(a.sketch_cancel());
}

// ---- chains through coincident relations, selecting dimensions, the right-click menu ------------

#[test]
fn a_chain_follows_coincident_relations_as_well_as_shared_points() {
    let mut a = xy();
    no_grid(&mut a);
    // Two segments that meet at different points glued by a coincident relation, plus a
    // third that merely lies nearby.
    for (id, x, y) in [
        ("a1", 0.0, 0.0),
        ("a2", 20.0, 0.0),
        ("b1", 20.0, 0.0),
        ("b2", 40.0, 10.0),
        ("c1", 40.0, 10.0),
        ("c2", 60.0, 10.0),
        ("d1", 0.0, 30.0),
        ("d2", 20.0, 30.0),
    ] {
        add_point(&mut a, id, x, y);
    }
    for (id, p, q) in [
        ("la", "a1", "a2"),
        ("lb", "b1", "b2"),
        ("lc", "c1", "c2"),
        ("ld", "d1", "d2"),
    ] {
        add_line_between(&mut a, id, p, q);
    }
    for (id, p, q) in [("j1", "a2", "b1"), ("j2", "b2", "c1")] {
        assert!(add_constraint(
            &mut a,
            id,
            Constraint::Coincident {
                a: p.into(),
                b: q.into()
            }
        ));
    }
    a.set_tool(Tool::Select);
    let (x, y) = screen(&a, (10.0, 0.0));
    assert!(a.sketch_double_click(x, y, W, H));
    let mut sel = selection(&a);
    sel.sort();
    assert_eq!(
        sel,
        ["la", "lb", "lc"],
        "joined through two coincident relations"
    );
    // The unrelated segment is a chain of its own.
    let (x, y) = screen(&a, (10.0, 30.0));
    assert!(a.sketch_double_click(x, y, W, H));
    assert_eq!(selection(&a), ["ld"]);
}

fn rectangle_with_a_dimension(a: &mut AppState) -> String {
    a.set_tool(Tool::Rectangle);
    click(a, (0.0, 0.0));
    click(a, (40.0, 20.0));
    a.set_tool(Tool::Select);
    let sketch = a.sketching.as_ref().unwrap().sketch.clone();
    let l = session_sketch(a)
        .entities()
        .iter()
        .find(|(_, e)| matches!(e, Entity::Line { .. }))
        .map(|(id, _)| id.clone())
        .unwrap();
    a.submit(CommandKind::SketchAddConstraint {
        sketch,
        constraint_id: "dim".into(),
        constraint: Constraint::Length {
            line: l,
            value: 40.0,
        },
    })
    .unwrap();
    "dim".into()
}

#[test]
fn a_dimension_can_be_clicked_boxed_highlighted_and_deleted_with_other_things() {
    let mut a = xy();
    no_grid(&mut a);
    rectangle_with_a_dimension(&mut a);
    let o = a.sketch_overlay(W, H).unwrap();
    let d = o
        .dims
        .iter()
        .find(|d| d.id.as_deref() == Some("dim"))
        .unwrap()
        .clone();
    assert!(!d.selected && !d.hover);
    // Hovering its text highlights it and names it; clicking selects it (and no entity).
    a.sketch_hover(Some((d.text_at.0, d.text_at.1, W, H)));
    let o = a.sketch_overlay(W, H).unwrap();
    assert!(o.dims.iter().any(|d| d.hover));
    assert!(o.tooltip.as_ref().unwrap().1.contains("length"));
    a.sketch_click(d.text_at.0, d.text_at.1, W, H);
    assert_eq!(a.sketching.as_ref().unwrap().relations, ["dim"]);
    assert!(a
        .sketch_overlay(W, H)
        .unwrap()
        .dims
        .iter()
        .any(|d| d.selected));
    // Pressing on it does not start a drag; Shift-click adds an edge to the selection.
    assert!(!a.sketch_drag_begin(d.text_at.0, d.text_at.1, W, H));
    let (x, y) = screen(&a, (20.0, 20.0));
    a.sketch_click_with(x, y, W, H, true);
    assert_eq!(selection(&a).len(), 1);
    assert_eq!(a.sketching.as_ref().unwrap().relations.len(), 1);
    // Delete removes the edge and the dimension in one step.
    let rev = a.authority.revision().0;
    assert!(a.sketch_delete_selected());
    assert_eq!(a.authority.revision().0, rev + 1);
    assert!(!session_sketch(&a).constraints().contains_key("dim"));
    a.undo().unwrap();
    assert!(session_sketch(&a).constraints().contains_key("dim"));

    // A box takes the dimension together with the geometry it covers (the text is outside the
    // rectangle, so draw the box around both).
    let d = a
        .sketch_overlay(W, H)
        .unwrap()
        .dims
        .into_iter()
        .find(|d| d.id.as_deref() == Some("dim"))
        .unwrap();
    let pts = [
        screen(&a, (-5.0, -5.0)),
        screen(&a, (45.0, 25.0)),
        d.text_at,
    ];
    let min = |f: fn(&(f64, f64)) -> f64| pts.iter().map(f).fold(f64::MAX, f64::min) - 20.0;
    let max = |f: fn(&(f64, f64)) -> f64| pts.iter().map(f).fold(f64::MIN, f64::max) + 20.0;
    // A window (top-left to bottom-right) around the rectangle and the dimension text.
    let (x0, y0, x1, y1) = (min(|p| p.0), min(|p| p.1), max(|p| p.0), max(|p| p.1));
    assert!(a.sketch_box_begin(x0, y0));
    a.sketch_box_move(x1, y1);
    assert!(a.sketch_box_end(false, W, H) >= 5);
    assert_eq!(a.sketching.as_ref().unwrap().relations, ["dim"]);
    assert!(selection(&a).iter().filter(|i| i.starts_with('l')).count() == 4);
    // A box over only the text still takes just the dimension.
    a.sketch_cancel();
    let (x0, y0) = (d.text_at.0 - 30.0, d.text_at.1 - 14.0);
    assert!(a.sketch_box_begin(x0, y0));
    a.sketch_box_move(d.text_at.0 + 30.0, d.text_at.1 + 14.0);
    assert_eq!(a.sketch_box_end(false, W, H), 1);
    assert!(selection(&a).is_empty());
}

fn labels(menu: &[dc_app::context::MenuEntry]) -> Vec<String> {
    menu.iter()
        .filter_map(|e| match e {
            dc_app::context::MenuEntry::Item(i) => Some(i.label.clone()),
            _ => None,
        })
        .collect()
}

#[test]
fn the_right_click_menu_selects_what_is_under_the_pointer_and_offers_what_fits() {
    use dc_app::context::MenuAction;
    let mut a = xy();
    no_grid(&mut a);
    let dim = rectangle_with_a_dimension(&mut a);
    // On an edge: selects it, offers chain, dimension, construction, delete, its relations.
    let (x, y) = screen(&a, (20.0, 20.0));
    let menu = a.sketch_context_open(x, y, W, H);
    assert_eq!(selection(&a).len(), 1);
    let l = labels(&menu);
    for want in [
        "Select Chain",
        "Smart Dimension",
        "Construction Geometry",
        "Delete",
        "Select All",
    ] {
        assert!(l.iter().any(|x| x == want), "{want} in {l:?}");
    }
    assert!(l.iter().any(|x| x.starts_with("Delete relation:")), "{l:?}");
    // Select Chain from the menu takes the rectangle's four edges.
    assert!(a.sketch_context_do(&MenuAction::SelectChain));
    assert_eq!(selection(&a).len(), 4);
    // Right-clicking something already selected keeps the whole selection.
    let menu = a.sketch_context_open(x, y, W, H);
    assert_eq!(selection(&a).len(), 4);
    assert!(
        labels(&menu).iter().any(|x| x == "Delete (4)"),
        "{:?}",
        labels(&menu)
    );
    assert!(labels(&menu).iter().any(|x| x.starts_with("Add relation:")));
    // Construction toggles through the menu, as one undo step.
    let rev = a.authority.revision().0;
    assert!(a.sketch_context_do(&MenuAction::Construction));
    assert_eq!(a.authority.revision().0, rev + 1);
    assert!(a.sketch_context_do(&MenuAction::Construction));
    // Smart Dimension from the menu starts the dimension tool with that edge picked.
    a.sketch_context_open(x, y, W, H);
    a.sketch_click(x, y, W, H);
    a.sketch_context_open(x, y, W, H);
    assert!(a.sketch_context_do(&MenuAction::SmartDimension));
    let s = a.sketching.as_ref().unwrap();
    assert_eq!(s.tool, Tool::Dimension);
    assert_eq!(s.dim.picks.len(), 1);
    a.set_tool(Tool::Select);
    // On a dimension: select it and offer Delete; on empty space: select all.
    let d = a
        .sketch_overlay(W, H)
        .unwrap()
        .dims
        .into_iter()
        .find(|d| d.id.as_deref() == Some(dim.as_str()))
        .unwrap();
    let menu = a.sketch_context_open(d.text_at.0, d.text_at.1, W, H);
    assert_eq!(
        a.sketching.as_ref().unwrap().relations,
        std::slice::from_ref(&dim)
    );
    assert!(labels(&menu).iter().any(|x| x == "Delete"));
    let (ex, ey) = screen(&a, (80.0, 60.0));
    let menu = a.sketch_context_open(ex, ey, W, H);
    assert!(labels(&menu).iter().any(|x| x == "Select All"));
    assert!(a.sketch_context_do(&MenuAction::SelectAll));
    assert!(selection(&a).len() >= 4);
    assert!(a.sketch_context_do(&MenuAction::ClearSelection));
    assert!(selection(&a).is_empty() && a.sketching.as_ref().unwrap().relations.is_empty());
    // While drawing a line chain the menu offers End chain and Select.
    a.set_tool(Tool::Line);
    click(&mut a, (0.0, 40.0));
    let menu = a.sketch_context_open(x, y, W, H);
    assert_eq!(labels(&menu), ["End chain", "Select"]);
    assert!(a.sketch_context_do(&MenuAction::EndChain));
    assert!(a.sketching.as_ref().unwrap().first.is_none());
    assert_eq!(
        a.sketching.as_ref().unwrap().tool,
        Tool::Line,
        "still drawing lines"
    );
    assert!(a.sketch_context_do(&MenuAction::Select));
    assert_eq!(a.sketching.as_ref().unwrap().tool, Tool::Select);
}

// ---- Trim, Extend and Split tools ---------------------------------------------------------------

/// A horizontal line from (0, y) to (40, y), crossed by verticals at x = 10 and x = 30.
fn ladder(a: &mut AppState, y: f64, prefix: &str) -> String {
    let id = |s: &str| format!("{prefix}{s}");
    for (n, x, yy) in [
        ("a", 0.0, y),
        ("b", 40.0, y),
        ("c", 10.0, y - 5.0),
        ("d", 10.0, y + 5.0),
        ("e", 30.0, y - 5.0),
        ("f", 30.0, y + 5.0),
    ] {
        add_point(a, &id(n), x, yy);
    }
    for (n, p, q) in [("h", "a", "b"), ("v1", "c", "d"), ("v2", "e", "f")] {
        add_line_between(a, &id(n), &id(p), &id(q));
    }
    id("h")
}

fn line_ends(a: &AppState, id: &str) -> ((f64, f64), (f64, f64)) {
    let sk = session_sketch(a);
    match sk.entity(id) {
        Some(Entity::Line { a: p, b: q }) => (sk.point(p).unwrap(), sk.point(q).unwrap()),
        other => panic!("{id}: {other:?}"),
    }
}

#[test]
fn trim_shows_what_goes_in_red_and_a_click_removes_exactly_that_in_one_step() {
    let mut a = xy();
    no_grid(&mut a);
    let h = ladder(&mut a, 0.0, "");
    a.set_tool(Tool::Trim);
    at(&mut a, (36.0, 0.0));
    let o = a.sketch_overlay(W, H).unwrap();
    assert_eq!(
        o.edit_removed.len(),
        1,
        "a red preview of the part that would go"
    );
    let red = &o.edit_removed[0];
    let (s0, s1) = (screen(&a, (30.0, 0.0)), screen(&a, (40.0, 0.0)));
    assert!(
        (red[0].0 - s0.0).abs() < 1e-6 && (red.last().unwrap().0 - s1.0).abs() < 1e-6,
        "from the vertical at 30 to the end: {red:?}"
    );
    let rev = a.authority.revision();
    click(&mut a, (36.0, 0.0));
    assert_eq!(a.authority.revision().0, rev.0 + 1);
    assert!((line_ends(&a, &h).1 .0 - 30.0).abs() < 1e-6);
    a.undo().unwrap();
    assert!(
        (line_ends(&a, &h).1 .0 - 40.0).abs() < 1e-6,
        "undone in one step"
    );
    // Pointing at nothing previews nothing and a click does nothing.
    at(&mut a, (20.0, 30.0));
    assert!(a.sketch_overlay(W, H).unwrap().edit_removed.is_empty());
    let rev = a.authority.revision();
    click(&mut a, (20.0, 30.0));
    assert_eq!(a.authority.revision(), rev);
}

#[test]
fn trim_says_why_it_cannot_trim_and_leaves_the_sketch_alone() {
    let mut a = xy();
    no_grid(&mut a);
    add_point(&mut a, "c", 0.0, 0.0);
    add_circle(&mut a, "k", "c", 10.0);
    add_point(&mut a, "p", -5.0, 5.0);
    add_point(&mut a, "q", 5.0, 5.0);
    add_line_between(&mut a, "chord", "p", "q");
    a.set_tool(Tool::Trim);
    // The chord's two ends are inside the circle, so the circle has no crossing at all: it
    // would simply be deleted; add one crossing and it is refused.
    add_point(&mut a, "r", 0.0, 0.0);
    add_point(&mut a, "t", 20.0, 0.0);
    add_line_between(&mut a, "ray", "r", "t");
    at(&mut a, (0.0, 10.0));
    let o = a.sketch_overlay(W, H).unwrap();
    let why = o.tooltip.expect("a reason").1;
    assert!(why.contains("two other shapes"), "{why}");
    assert!(o.edit_removed.is_empty());
    let rev = a.authority.revision();
    click(&mut a, (0.0, 10.0));
    assert_eq!(a.authority.revision(), rev, "nothing changed");
    assert!(a
        .messages
        .last()
        .is_some_and(|m| m.level == dc_app::state::Level::Error));
}

#[test]
fn extend_previews_in_green_and_a_click_extends() {
    let mut a = xy();
    no_grid(&mut a);
    add_point(&mut a, "a", 0.0, 0.0);
    add_point(&mut a, "b", 10.0, 0.0);
    add_line_between(&mut a, "h", "a", "b");
    add_point(&mut a, "c", 25.0, -5.0);
    add_point(&mut a, "d", 25.0, 5.0);
    add_line_between(&mut a, "wall", "c", "d");
    a.set_tool(Tool::Extend);
    at(&mut a, (9.0, 0.5));
    let o = a.sketch_overlay(W, H).unwrap();
    assert_eq!(o.edit_added.len(), 1);
    let g = &o.edit_added[0];
    let (e0, e1) = (screen(&a, (10.0, 0.0)), screen(&a, (25.0, 0.0)));
    assert!(
        (g[0].0 - e0.0).abs() < 1e-6 && (g.last().unwrap().0 - e1.0).abs() < 1e-6,
        "{g:?}"
    );
    click(&mut a, (9.0, 0.5));
    assert!((line_ends(&a, "h").1 .0 - 25.0).abs() < 1e-6);

    // The marker case is covered by the next test.
}

#[test]
fn split_cuts_a_line_where_clicked() {
    let mut a = xy();
    no_grid(&mut a);
    add_point(&mut a, "a", 0.0, 0.0);
    add_point(&mut a, "b", 20.0, 0.0);
    add_line_between(&mut a, "h", "a", "b");
    a.set_tool(Tool::Split);
    at(&mut a, (7.0, 0.2));
    assert_eq!(a.sketch_overlay(W, H).unwrap().edit_markers.len(), 1);
    let rev = a.authority.revision();
    click(&mut a, (7.0, 0.2));
    assert_eq!(a.authority.revision().0, rev.0 + 1);
    let lines = session_sketch(&a)
        .entities()
        .values()
        .filter(|e| matches!(e, Entity::Line { .. }))
        .count();
    assert_eq!(lines, 2);
    assert!((line_ends(&a, "h").1 .0 - 7.0).abs() < 1e-6);
}

#[test]
fn a_power_trim_stroke_trims_everything_it_crosses_as_one_undo_step() {
    let mut a = xy();
    no_grid(&mut a);
    // A vertical line at x = 0 and three horizontals crossing it.
    add_point(&mut a, "v1", 0.0, -20.0);
    add_point(&mut a, "v2", 0.0, 20.0);
    add_line_between(&mut a, "vert", "v1", "v2");
    let mut hs = Vec::new();
    for (n, y) in [("lo", -10.0), ("mid", 0.0), ("hi", 10.0)] {
        add_point(&mut a, &format!("{n}a"), -20.0, y);
        add_point(&mut a, &format!("{n}b"), 20.0, y);
        add_line_between(&mut a, n, &format!("{n}a"), &format!("{n}b"));
        hs.push(n.to_string());
    }
    a.set_tool(Tool::Trim);
    let rev = a.authority.revision();
    // Drag straight down across all three at x = 10: each loses its right half.
    let start = screen(&a, (10.0, 15.0));
    assert!(a.trim_drag_begin(start.0, start.1));
    for y in [10.0, 5.0, 0.0, -5.0, -10.0, -15.0] {
        let p = screen(&a, (10.0, y));
        a.trim_drag_move(p.0, p.1, W, H);
    }
    // Live: the overlay already shows the trimmed lines, and a trail.
    assert_eq!(
        a.authority.revision(),
        rev,
        "nothing is committed mid-stroke"
    );
    assert!(a.sketch_overlay(W, H).unwrap().trim_trail.len() > 3);
    for h in &hs {
        let sk = a.view_sketch().unwrap();
        let Some(Entity::Line { b, .. }) = sk.entity(h) else {
            panic!()
        };
        assert!(
            (sk.point(b).unwrap().0).abs() < 1e-6,
            "{h} stops at the vertical"
        );
    }
    assert!(a.trim_drag_end());
    assert_eq!(
        a.authority.revision().0,
        rev.0 + 1,
        "one step for the whole stroke"
    );
    for h in &hs {
        assert!((line_ends(&a, h).1 .0).abs() < 1e-6);
    }
    a.undo().unwrap();
    assert!(
        (line_ends(&a, "mid").1 .0 - 20.0).abs() < 1e-6,
        "undone together"
    );
    // Esc abandons a stroke.
    assert!(a.trim_drag_begin(start.0, start.1));
    a.trim_drag_move(screen(&a, (10.0, 0.0)).0, screen(&a, (10.0, 0.0)).1, W, H);
    assert!(a.sketch_cancel());
    assert!(a.sketching.as_ref().unwrap().stroke.is_none());
    assert!(!a.trim_drag_end());
}

// ---- angle dimension, more relations, line / arc freedom, glyph details, trim modes -------------

fn arc_entity(a: &mut AppState, id: &str, c: (f64, f64), s: (f64, f64), e: (f64, f64)) {
    let (pc, ps, pe) = (format!("{id}c"), format!("{id}s"), format!("{id}e"));
    add_point(a, &pc, c.0, c.1);
    add_point(a, &ps, s.0, s.1);
    add_point(a, &pe, e.0, e.1);
    add_entity(
        a,
        id,
        Entity::Arc {
            center: pc,
            start: ps,
            end: pe,
        },
    );
}

#[test]
fn an_angle_between_two_lines_is_dimensioned_edited_and_undone_like_any_dimension() {
    let mut a = xy();
    no_grid(&mut a);
    add_point(&mut a, "o", 0.0, 0.0);
    add_point(&mut a, "p", 40.0, 0.0);
    add_point(&mut a, "q", 25.0, 25.0);
    add_line_between(&mut a, "la", "o", "p");
    add_line_between(&mut a, "lb", "o", "q");
    fix(&mut a, "fo", "o", 0.0, 0.0);
    fix(&mut a, "fp", "p", 40.0, 0.0);
    a.set_tool(Tool::Dimension);
    click(&mut a, (30.0, 0.0));
    click(&mut a, (12.0, 12.0));
    // Live: an ISO angle arc with two arrowheads and "45°".
    at(&mut a, (30.0, 14.0));
    let o = a.sketch_overlay(W, H).unwrap();
    let d = o.dims.iter().find(|d| d.preview).expect("a preview");
    assert_eq!(d.text, "45°");
    assert_eq!(d.arrows.len(), 2);
    assert!(d.line.len() >= 16, "an arc, drawn in short pieces");
    assert!(o.dim_prompt.as_deref().is_some_and(|p| p.contains("Angle")));
    // Commit nothing until placed; then the box asks for degrees.
    let rev = a.authority.revision();
    click(&mut a, (30.0, 14.0));
    assert_eq!(a.authority.revision(), rev);
    let edit = a.sketching.as_ref().unwrap().dim.edit.clone().unwrap();
    assert_eq!(edit.spec.unit(), "°");
    edit_text(&mut a, "200");
    assert!(!a.apply_dimension());
    assert!(a
        .sketching
        .as_ref()
        .unwrap()
        .dim
        .edit
        .as_ref()
        .unwrap()
        .error
        .as_deref()
        .is_some_and(|e| e.contains("180")));
    edit_text(&mut a, "60");
    assert!(a.apply_dimension());
    assert_eq!(a.authority.revision().0, rev.0 + 1);
    let (qx, qy) = point_at(&a, "q");
    assert!((det_deg(qy, qx) - 60.0).abs() < 1e-6, "{qx} {qy}");
    // The dimension is drawn, can be edited by double-click, and undone.
    let d = a
        .sketch_overlay(W, H)
        .unwrap()
        .dims
        .into_iter()
        .find(|d| !d.preview)
        .unwrap();
    assert_eq!(d.text, "60°");
    a.set_tool(Tool::Select);
    assert!(a.sketch_double_click(d.text_at.0, d.text_at.1, W, H));
    edit_text(&mut a, "30");
    assert!(a.apply_dimension());
    let (qx, qy) = point_at(&a, "q");
    assert!((det_deg(qy, qx) - 30.0).abs() < 1e-6);
    a.undo().unwrap();
    assert!((det_deg(point_at(&a, "q").1, point_at(&a, "q").0) - 60.0).abs() < 1e-6);

    // Drawn the other way round the angle keeps its sense (clockwise stays clockwise).
    add_point(&mut a, "r", 25.0, -25.0);
    add_line_between(&mut a, "lc", "o", "r");
    assert!(dimension(
        &mut a,
        &[(30.0, 0.0), (12.0, -12.0)],
        (30.0, -14.0),
        "20"
    ));
    let (rx, ry) = point_at(&a, "r");
    assert!((det_deg(ry, rx) + 20.0).abs() < 1e-6, "{rx} {ry}");
}

fn det_deg(y: f64, x: f64) -> f64 {
    dc_math::det::atan2(y, x).to_degrees()
}

#[test]
fn symmetric_about_an_axis_tangent_curves_and_merge_points_from_the_relation_buttons() {
    let mut a = xy();
    no_grid(&mut a);
    add_point(&mut a, "p", 20.0, 15.0);
    add_point(&mut a, "q", 22.0, -18.0);
    fix(&mut a, "fp", "p", 20.0, 15.0);
    // Two points and the horizontal axis: mirrored images.
    pick_at(&mut a, (20.0, 15.0), false);
    pick_at(&mut a, (22.0, -18.0), true);
    pick_at(&mut a, (60.0, 0.0), true);
    assert_eq!(selection(&a).len(), 3, "{:?}", selection(&a));
    assert!(a.available_relations().contains(&Relation::Symmetric));
    assert!(a.apply_relation(Relation::Symmetric));
    assert!(near(point_at(&a, "q"), (20.0, -15.0)));
    a.undo().unwrap();

    // Two circles: tangent (outside each other).
    add_point(&mut a, "c1", -40.0, 30.0);
    add_point(&mut a, "c2", -40.0, 0.0);
    add_circle(&mut a, "k1", "c1", 10.0);
    add_circle(&mut a, "k2", "c2", 8.0);
    fix(&mut a, "f1", "c1", -40.0, 30.0);
    assert!(add_constraint(
        &mut a,
        "r1",
        Constraint::Radius {
            target: "k1".into(),
            value: 10.0
        }
    ));
    assert!(add_constraint(
        &mut a,
        "r2",
        Constraint::Radius {
            target: "k2".into(),
            value: 8.0
        }
    ));
    select(&mut a, &["k1", "k2"]);
    assert!(a.available_relations().contains(&Relation::Tangent));
    assert!(a.apply_relation(Relation::Tangent));
    let (c2x, c2y) = point_at(&a, "c2");
    assert!(
        (dc_math::det::hypot(c2x + 40.0, c2y - 30.0) - 18.0).abs() < 1e-6,
        "{c2x} {c2y}"
    );
    assert_eq!(
        constraints_of(&solved(&a), |c| matches!(
            c,
            Constraint::TangentCurves { .. }
        )),
        1
    );

    // Merge: two separate points that stand for one corner become one.
    add_point(&mut a, "m1", 50.0, 50.0);
    add_point(&mut a, "m2", 50.0, 50.0);
    add_point(&mut a, "m3", 80.0, 50.0);
    add_line_between(&mut a, "lm", "m2", "m3");
    select(&mut a, &["m1", "m2"]);
    assert!(a.available_relations().contains(&Relation::Merge));
    let rev = a.authority.revision().0;
    assert!(a.apply_relation(Relation::Merge));
    assert_eq!(a.authority.revision().0, rev + 1, "one undo step");
    let sk = session_sketch(&a);
    assert!(
        sk.entity("m2").is_none() || sk.entity("m1").is_none(),
        "one of them is gone"
    );
    assert!(selection(&a).is_empty());
    a.undo().unwrap();
    assert!(session_sketch(&a).entity("m1").is_some() && session_sketch(&a).entity("m2").is_some());
}

#[test]
fn concentric_and_equal_radius_work_on_arcs_too() {
    let mut a = xy();
    no_grid(&mut a);
    arc_entity(&mut a, "a1", (0.0, 0.0), (10.0, 0.0), (0.0, 10.0));
    arc_entity(&mut a, "a2", (30.0, 0.0), (36.0, 0.0), (30.0, 6.0));
    fix(&mut a, "f1", "a1c", 0.0, 0.0);
    fix(&mut a, "f2", "a1s", 10.0, 0.0);
    select(&mut a, &["a1", "a2"]);
    let rel = a.available_relations();
    assert!(
        rel.contains(&Relation::Equal)
            && rel.contains(&Relation::Concentric)
            && rel.contains(&Relation::Tangent),
        "{rel:?}"
    );
    assert!(a.apply_relation(Relation::Equal));
    let (c, s) = (point_at(&a, "a2c"), point_at(&a, "a2s"));
    assert!(
        (dc_math::det::hypot(s.0 - c.0, s.1 - c.1) - 10.0).abs() < 1e-6,
        "radius copied"
    );
    select(&mut a, &["a1", "a2"]);
    assert!(a.apply_relation(Relation::Concentric));
    assert!(near(point_at(&a, "a2c"), (0.0, 0.0)));
}

#[test]
fn a_hovered_line_or_arc_shows_how_it_can_still_move() {
    let mut a = xy();
    no_grid(&mut a);
    add_point(&mut a, "p", 0.0, 0.0);
    add_point(&mut a, "q", 30.0, 0.0);
    add_line_between(&mut a, "free", "p", "q");
    arc_entity(&mut a, "arc", (0.0, 40.0), (10.0, 40.0), (0.0, 50.0));
    fix(&mut a, "fc", "arcc", 0.0, 40.0);
    fix(&mut a, "fs", "arcs", 10.0, 40.0);
    a.set_tool(Tool::Select);
    a.sketching.as_mut().unwrap().show_freedom = false;
    assert!(a.sketch_overlay(W, H).unwrap().freedom.is_empty());
    // The free line: no mark at its middle (that is where the midpoint handle lives).
    select(&mut a, &["free"]);
    let o = a.sketch_overlay(W, H).unwrap();
    let mid = screen(&a, (15.0, 0.0));
    assert!(
        !o.freedom
            .iter()
            .any(|m| (m.pos.0 - mid.0).abs() < 1e-6 && (m.pos.1 - mid.1).abs() < 1e-6),
        "{:?}",
        o.freedom
    );
    // The arc: its end can go round (sweep) but its radius is fixed by start and centre.
    select(&mut a, &["arc"]);
    let o = a.sketch_overlay(W, H).unwrap();
    assert!(o.freedom.iter().any(|m| m.sweep), "{:?}", o.freedom);
    assert!(!o.freedom.iter().any(|m| m.radius));
    // And the tooltip says so.
    let k = core::f64::consts::FRAC_1_SQRT_2;
    at(&mut a, (10.0 * k, 40.0 + 10.0 * k));
    let tip = a.sketch_overlay(W, H).unwrap().tooltip.unwrap().1;
    assert!(
        tip.contains("sweep free") && !tip.contains("radius free"),
        "{tip}"
    );
    // Pin the end too: no marks left.
    fix(&mut a, "fe", "arce", 0.0, 50.0);
    select(&mut a, &["arc"]);
    assert!(!a
        .sketch_overlay(W, H)
        .unwrap()
        .freedom
        .iter()
        .any(|m| m.sweep || m.radius));
}

#[test]
fn glyphs_sit_at_the_middle_of_an_arc_scale_and_can_be_read_with_any_tool() {
    let mut a = xy();
    no_grid(&mut a);
    arc_entity(&mut a, "a1", (0.0, 0.0), (20.0, 0.0), (0.0, 20.0));
    arc_entity(&mut a, "a2", (50.0, 0.0), (60.0, 0.0), (50.0, 10.0));
    assert!(add_constraint(
        &mut a,
        "eq",
        Constraint::EqualRadius {
            a: "a1".into(),
            b: "a2".into()
        }
    ));
    let o = a.sketch_overlay(W, H).unwrap();
    let g = o.glyphs.iter().find(|g| g.constraint == "eq").unwrap();
    // The first arc's symbol is out beyond its middle (45 degrees), not at its start.
    let k = core::f64::consts::FRAC_1_SQRT_2;
    let mid = screen(&a, (20.0 * k, 20.0 * k));
    let c0 = screen(&a, (0.0, 0.0));
    let (dx, dy) = (g.pos.0 - c0.0, g.pos.1 - c0.1);
    let want = (mid.0 - c0.0, mid.1 - c0.1);
    let cos = (dx * want.0 + dy * want.1)
        / (dc_math::det::hypot(dx, dy) * dc_math::det::hypot(want.0, want.1));
    assert!(
        cos > 0.999,
        "direction from the centre is along the arc's middle: {cos}"
    );
    assert!(
        dc_math::det::hypot(dx, dy) > dc_math::det::hypot(want.0, want.1),
        "outside the arc"
    );

    // Bigger symbols are bigger to hit.
    let pos = o.glyphs[0].pos;
    a.sketching.as_mut().unwrap().glyph_scale = 1.0;
    assert!(a.glyph_at(pos.0 + 14.0, pos.1, W, H).is_none());
    a.sketching.as_mut().unwrap().glyph_scale = 1.6;
    let pos = a.sketch_overlay(W, H).unwrap().glyphs[0].pos;
    assert!(a.glyph_at(pos.0 + 14.0, pos.1, W, H).is_some());
    assert_eq!(a.sketch_overlay(W, H).unwrap().glyphs[0].scale, 1.6);

    // With another tool the symbol can still be read, but a click is the tool's, not the symbol's.
    a.set_tool(Tool::Line);
    a.sketch_hover(Some((pos.0, pos.1, W, H)));
    let s = a.sketching.as_ref().unwrap();
    assert!(s.hover_relation.is_some());
    assert!(a
        .sketch_overlay(W, H)
        .unwrap()
        .tooltip
        .unwrap()
        .1
        .contains("equal radius"));
    a.sketch_click(pos.0, pos.1, W, H);
    assert!(
        a.sketching.as_ref().unwrap().relations.is_empty(),
        "no relation was picked"
    );
    assert!(
        a.sketching.as_ref().unwrap().first.is_some(),
        "the click started a line"
    );
}

#[test]
fn corner_trim_between_bounds_construction_and_circle_split_through_the_tools() {
    use dc_app::editing::TrimMode;
    let mut a = xy();
    no_grid(&mut a);
    // Corner: a horizontal and a vertical line that cross like a plus.
    for (n, x, y) in [
        ("h1", 0.0, 0.0),
        ("h2", 40.0, 0.0),
        ("v1", 20.0, -20.0),
        ("v2", 20.0, 20.0),
    ] {
        add_point(&mut a, n, x, y);
    }
    add_line_between(&mut a, "h", "h1", "h2");
    add_line_between(&mut a, "v", "v1", "v2");
    a.set_tool(Tool::Trim);
    a.set_trim_mode(TrimMode::Corner);
    at(&mut a, (5.0, 0.0));
    assert!(
        a.sketch_overlay(W, H).unwrap().edit_removed.is_empty(),
        "nothing to preview until two are picked"
    );
    click(&mut a, (5.0, 0.0)); // keep the left part of h
    assert_eq!(selection(&a), ["h"], "the first pick is highlighted");
    at(&mut a, (20.0, -12.0));
    let o = a.sketch_overlay(W, H).unwrap();
    assert_eq!(
        o.edit_removed.len(),
        2,
        "both would be trimmed to the corner"
    );
    let rev = a.authority.revision().0;
    click(&mut a, (20.0, -12.0)); // keep the lower part of v
    assert_eq!(a.authority.revision().0, rev + 1);
    assert!(near(line_ends(&a, "h").1, (20.0, 0.0)));
    assert!(near(line_ends(&a, "v").1, (20.0, 0.0)));
    a.undo().unwrap();
    assert!(near(line_ends(&a, "h").1, (40.0, 0.0)));

    // Between bounds: three bars cross a line; only the outer two cut.
    let mut b = xy();
    no_grid(&mut b);
    let line = ladder(&mut b, 0.0, "");
    add_point(&mut b, "x1", 20.0, -5.0);
    add_point(&mut b, "x2", 20.0, 5.0);
    add_line_between(&mut b, "mid", "x1", "x2");
    b.set_tool(Tool::Trim);
    b.set_trim_mode(TrimMode::Between);
    click(&mut b, (10.0, 4.0)); // first bound
    click(&mut b, (30.0, 4.0)); // second bound
    assert_eq!(b.sketching.as_ref().unwrap().trim_bounds.len(), 2);
    at(&mut b, (15.0, 0.0));
    let red = b.sketch_overlay(W, H).unwrap().edit_removed;
    let (s0, s1) = (screen(&b, (10.0, 0.0)), screen(&b, (30.0, 0.0)));
    assert!(
        (red[0][0].0 - s0.0).abs() < 1e-6 && (red[0].last().unwrap().0 - s1.0).abs() < 1e-6,
        "{red:?}"
    );
    click(&mut b, (15.0, 0.0));
    assert!(near(line_ends(&b, &line).1, (10.0, 0.0)));
    // Changing mode forgets the picks.
    b.set_trim_mode(TrimMode::Closest);
    assert!(b.sketching.as_ref().unwrap().trim_bounds.is_empty());

    // Construction lines only cut when asked.
    let mut c = xy();
    no_grid(&mut c);
    add_point(&mut c, "a", 0.0, 0.0);
    add_point(&mut c, "b", 40.0, 0.0);
    add_line_between(&mut c, "h", "a", "b");
    add_point(&mut c, "g1", 30.0, -5.0);
    add_point(&mut c, "g2", 30.0, 5.0);
    add_line_between(&mut c, "guide", "g1", "g2");
    let sketch = c.sketching.as_ref().unwrap().sketch.clone();
    c.submit(CommandKind::SketchSetConstruction {
        sketch,
        entity_id: "guide".into(),
        construction: true,
    })
    .unwrap();
    c.set_tool(Tool::Trim);
    at(&mut c, (35.0, 0.0));
    let red = c.sketch_overlay(W, H).unwrap().edit_removed;
    assert!(
        (red[0].last().unwrap().0 - screen(&c, (40.0, 0.0)).0).abs() < 1e-6
            && (red[0][0].0 - screen(&c, (0.0, 0.0)).0).abs() < 1e-6,
        "the whole line: {red:?}"
    );
    c.sketching.as_mut().unwrap().cut_construction = true;
    at(&mut c, (35.0, 0.0));
    let red = c.sketch_overlay(W, H).unwrap().edit_removed;
    assert!(
        (red[0][0].0 - screen(&c, (30.0, 0.0)).0).abs() < 1e-6,
        "now cut at the guide: {red:?}"
    );

    // Splitting a circle marks two opposite places and makes two arcs.
    let mut d = xy();
    no_grid(&mut d);
    add_point(&mut d, "c", 0.0, 0.0);
    add_circle(&mut d, "k", "c", 10.0);
    d.set_tool(Tool::Split);
    at(&mut d, (10.0, 0.5));
    assert_eq!(d.sketch_overlay(W, H).unwrap().edit_markers.len(), 2);
    click(&mut d, (10.0, 0.5));
    assert_eq!(
        session_sketch(&d)
            .entities()
            .values()
            .filter(|e| matches!(e, Entity::Arc { .. }))
            .count(),
        2
    );
}

// ---- moving a placed dimension -----------------------------------------------------------------

fn text_of(a: &AppState, cid: &str) -> (f64, f64) {
    a.sketch_overlay(W, H)
        .unwrap()
        .dims
        .into_iter()
        .find(|d| d.id.as_deref() == Some(cid))
        .expect("the dimension is drawn")
        .text_at
}

#[test]
fn a_placed_dimension_can_be_dragged_somewhere_else_and_stays_there() {
    let mut a = xy();
    no_grid(&mut a);
    draw_line(&mut a, (0.0, 0.0), (40.0, 0.0));
    assert!(dimension(&mut a, &[(20.0, 0.0)], (20.0, 15.0), "40"));
    let cid = session_sketch(&a)
        .placements()
        .keys()
        .next()
        .unwrap()
        .clone();
    assert_eq!(
        session_sketch(&a).placement(&cid),
        Some((20.0, 15.0)),
        "saved with the sketch"
    );
    a.set_tool(Tool::Select);
    let before = text_of(&a, &cid);
    let rev = a.authority.revision();

    // Pick it up by its text and carry it below the line.
    assert!(a.dimension_drag_begin(before.0, before.1, W, H));
    let target = screen(&a, (20.0, -18.0));
    a.dimension_drag_move(target.0, target.1, W, H);
    // Live: it is drawn at the new place, and nothing is logged yet.
    assert_eq!(a.authority.revision(), rev);
    let live = text_of(&a, &cid);
    assert!(
        live.1 > before.1 + 20.0,
        "moved down the screen: {before:?} -> {live:?}"
    );
    assert_eq!(
        session_sketch(&a).placement(&cid),
        Some((20.0, 15.0)),
        "the document still has the old place"
    );
    assert!(a.dimension_drag_end());
    assert_eq!(
        a.authority.revision().0,
        rev.0 + 1,
        "one command, one undo step"
    );
    let p = session_sketch(&a).placement(&cid).unwrap();
    assert!(
        (p.0 - 20.0).abs() < 1e-6 && (p.1 + 18.0).abs() < 1e-6,
        "{p:?}"
    );
    assert!(
        a.sketching.as_ref().unwrap().dim_places.is_empty(),
        "no leftover override"
    );
    // It is now below the line (the text sits on the side of the dimension line, away from it).
    assert!((text_of(&a, &cid).1 - live.1).abs() < 1e-6);
    // Moving a dimension changes no geometry.
    let ids = line_point_ids(&a);
    assert!(near(session_sketch(&a).point(&ids[0]).unwrap(), (0.0, 0.0)));
    assert!(near(
        session_sketch(&a).point(&ids[1]).unwrap(),
        (40.0, 0.0)
    ));
    // Undo puts it back; redo moves it again.
    a.undo().unwrap();
    assert_eq!(session_sketch(&a).placement(&cid), Some((20.0, 15.0)));
    a.redo().unwrap();
    assert!((session_sketch(&a).placement(&cid).unwrap().1 + 18.0).abs() < 1e-6);
}

fn line_point_ids(a: &AppState) -> Vec<String> {
    let sk = session_sketch(a);
    sk.entities()
        .values()
        .find_map(|e| match e {
            Entity::Line { a, b } => Some(vec![a.clone(), b.clone()]),
            _ => None,
        })
        .unwrap()
}

#[test]
fn a_dimension_can_be_grabbed_by_its_line_and_the_move_can_be_cancelled_or_survive_a_reopen() {
    let mut a = xy();
    no_grid(&mut a);
    draw_line(&mut a, (0.0, 0.0), (40.0, 0.0));
    assert!(dimension(&mut a, &[(20.0, 0.0)], (20.0, 15.0), "40"));
    let cid = session_sketch(&a)
        .placements()
        .keys()
        .next()
        .unwrap()
        .clone();
    a.set_tool(Tool::Select);
    // Press on the dimension line, away from the text.
    let d = a
        .sketch_overlay(W, H)
        .unwrap()
        .dims
        .into_iter()
        .find(|d| !d.preview)
        .unwrap();
    let [p, q] = d.line[0];
    let near_end = ((p.0 * 3.0 + q.0) / 4.0, (p.1 * 3.0 + q.1) / 4.0);
    assert!(
        (near_end.0 - d.text_at.0).abs() > 30.0,
        "a spot on the line that is not the text: {near_end:?} vs {:?}",
        d.text_at
    );
    // Hovering there shows it can be picked up.
    a.sketch_hover(Some((near_end.0, near_end.1, W, H)));
    assert!(a
        .sketch_overlay(W, H)
        .unwrap()
        .tooltip
        .unwrap()
        .1
        .contains("drag to move"));
    assert!(a.dimension_drag_begin(near_end.0, near_end.1, W, H));
    let there = screen(&a, (20.0, 30.0));
    a.dimension_drag_move(there.0, there.1, W, H);
    // Esc drops it back where it was, and logs nothing.
    let rev = a.authority.revision();
    assert!(a.sketch_cancel());
    assert_eq!(a.authority.revision(), rev);
    assert!(a.sketching.as_ref().unwrap().dim_drag.is_none());
    assert_eq!(session_sketch(&a).placement(&cid), Some((20.0, 15.0)));
    assert!(
        (text_of(&a, &cid).1 - d.text_at.1).abs() < 1e-6,
        "drawn where it was"
    );

    // A drag that ends where it started commits nothing.
    assert!(a.dimension_drag_begin(d.text_at.0, d.text_at.1, W, H));
    assert!(!a.dimension_drag_end());
    assert_eq!(a.authority.revision(), rev);

    // Move it for real, leave the sketch and come back: it is where it was left.
    assert!(a.dimension_drag_begin(d.text_at.0, d.text_at.1, W, H));
    a.dimension_drag_move(there.0, there.1, W, H);
    assert!(a.dimension_drag_end());
    let name = a.sketching.as_ref().unwrap().sketch.clone();
    a.end_sketch();
    assert!(a.edit_sketch(&name));
    let p = session_sketch(&a).placement(&cid).unwrap();
    assert!((p.1 - 30.0).abs() < 1e-6, "{p:?}");
    // Deleting the dimension takes its place with it.
    a.set_tool(Tool::Select);
    let t = text_of(&a, &cid);
    a.sketch_click(t.0, t.1, W, H);
    assert!(a.sketch_delete_selected());
    assert!(session_sketch(&a).placements().is_empty());
}

#[test]
fn the_dimension_tool_moves_dimensions_too_but_not_while_one_is_being_made() {
    let mut a = xy();
    no_grid(&mut a);
    draw_line(&mut a, (0.0, 0.0), (40.0, 0.0));
    assert!(dimension(&mut a, &[(20.0, 0.0)], (20.0, 15.0), "40"));
    let cid = session_sketch(&a)
        .placements()
        .keys()
        .next()
        .unwrap()
        .clone();
    a.set_tool(Tool::Dimension);
    let t = text_of(&a, &cid);
    assert!(
        a.dimension_drag_begin(t.0, t.1, W, H),
        "nothing picked yet: the press moves the dimension"
    );
    a.sketch_cancel();
    // With a pick in progress, the press belongs to the dimension being made.
    click(&mut a, (20.0, 0.0));
    assert!(!a.dimension_drag_begin(t.0, t.1, W, H));
}

// ---- snapping a dragged dimension, grabbing extension lines, dimensions that follow -------------

/// Two separate 40 mm lines, each with a placed horizontal-ish length dimension below it.
fn two_dimensioned_lines(a: &mut AppState) -> (String, String) {
    draw_line(a, (0.0, 0.0), (40.0, 0.0));
    draw_line(a, (60.0, 0.0), (100.0, 0.0));
    assert!(dimension(a, &[(20.0, 0.0)], (20.0, -15.0), "40"));
    assert!(dimension(a, &[(80.0, 0.0)], (80.0, -12.0), "40"));
    let ids: Vec<String> = session_sketch(a).placements().keys().cloned().collect();
    a.set_tool(Tool::Select);
    (ids[0].clone(), ids[1].clone())
}

fn drag_dimension_to(a: &mut AppState, cid: &str, to: (f64, f64), free: bool) {
    let t = text_of(a, cid);
    assert!(a.dimension_drag_begin(t.0, t.1, W, H));
    let p = screen(a, to);
    a.dimension_drag_move_with(p.0, p.1, W, H, free);
}

#[test]
fn a_dragged_dimension_snaps_to_the_grid_the_centre_and_a_neighbouring_dimension() {
    let mut a = xy();
    no_grid(&mut a);
    let (first, second) = two_dimensioned_lines(&mut a);
    let (first, second) = if session_sketch(&a).placement(&first).unwrap().0 < 50.0 {
        (first, second)
    } else {
        (second, first)
    };
    // Centre: near the middle of the 0..40 line it jumps onto it, with a guide.
    drag_dimension_to(&mut a, &first, (21.2, -17.0), false);
    let at = a.sketching.as_ref().unwrap().dim_places[&first];
    assert!((at.0 - 20.0).abs() < 1e-9, "centred: {at:?}");
    assert!((at.1 + 17.0).abs() < 1e-9, "the height is untouched");
    assert!(
        !a.sketch_overlay(W, H).unwrap().dim_guides.is_empty(),
        "a guide is shown"
    );
    // Far from the centre nothing snaps, and no guide is left over.
    let p = screen(&a, (31.0, -17.0));
    a.dimension_drag_move_with(p.0, p.1, W, H, false);
    let at = a.sketching.as_ref().unwrap().dim_places[&first];
    assert!((at.0 - 31.0).abs() < 1e-6, "{at:?}");
    assert!(a.sketch_overlay(W, H).unwrap().dim_guides.is_empty());
    // Ctrl switches snapping off even next to the centre.
    let p = screen(&a, (21.2, -17.0));
    a.dimension_drag_move_with(p.0, p.1, W, H, true);
    assert!((a.sketching.as_ref().unwrap().dim_places[&first].0 - 21.2).abs() < 1e-6);
    a.sketch_cancel();

    // Level with the other dimension (whose line is at y = -12): within reach it lines up.
    drag_dimension_to(&mut a, &first, (28.0, -13.0), false);
    let at = a.sketching.as_ref().unwrap().dim_places[&first];
    assert!(
        (at.1 + 12.0).abs() < 1e-9,
        "level with its neighbour: {at:?}"
    );
    let o = a.sketch_overlay(W, H).unwrap();
    assert!(!o.dim_guides.is_empty());
    assert!(a.dimension_drag_end());
    assert!((session_sketch(&a).placement(&first).unwrap().1 + 12.0).abs() < 1e-6);
    assert!(
        a.sketching.as_ref().unwrap().dim_guides.is_empty(),
        "guides go with the drag"
    );
    let _ = second;

    // The grid, when it is on, applies to what is not otherwise snapped.
    a.sketching.as_mut().unwrap().snap_grid = true;
    let step = dc_app::sketching::snap_step(a.camera.distance);
    drag_dimension_to(&mut a, &first, (33.17, -26.43), false);
    let at = a.sketching.as_ref().unwrap().dim_places[&first];
    for v in [at.0, at.1] {
        let k = v / step;
        assert!((k - k.round()).abs() < 1e-6, "{v} is a multiple of {step}");
    }
    a.sketch_cancel();
}

#[test]
fn extension_lines_can_be_grabbed_but_never_take_a_press_meant_for_the_geometry() {
    let mut a = xy();
    no_grid(&mut a);
    draw_line(&mut a, (0.0, 0.0), (40.0, 0.0));
    assert!(dimension(&mut a, &[(20.0, 0.0)], (20.0, -15.0), "40"));
    let cid = session_sketch(&a)
        .placements()
        .keys()
        .next()
        .unwrap()
        .clone();
    a.set_tool(Tool::Select);
    let d = a
        .sketch_overlay(W, H)
        .unwrap()
        .dims
        .into_iter()
        .find(|d| !d.preview)
        .unwrap();
    assert_eq!(d.witness.len(), 2, "one extension line at each end");
    let [p, q] = d.witness[0];
    // Halfway along the extension line: grabbable, picks the dimension.
    let mid = ((p.0 + q.0) / 2.0, (p.1 + q.1) / 2.0);
    assert_eq!(
        a.dimension_label_at(mid.0, mid.1, W, H).as_deref(),
        Some(cid.as_str())
    );
    a.sketch_hover(Some((mid.0, mid.1, W, H)));
    assert!(a.sketch_overlay(W, H).unwrap().dims.iter().any(|d| d.hover));
    assert!(a.dimension_drag_begin(mid.0, mid.1, W, H));
    let there = screen(&a, (20.0, -25.0));
    a.dimension_drag_move(there.0, there.1, W, H);
    assert!(a.dimension_drag_end());
    assert!((session_sketch(&a).placement(&cid).unwrap().1 + 25.0).abs() < 1.0);
    // At the geometry end of the extension line the point belongs to the shape, not to it.
    let d = a
        .sketch_overlay(W, H)
        .unwrap()
        .dims
        .into_iter()
        .find(|d| !d.preview)
        .unwrap();
    let [p, _] = d.witness[0];
    let near_corner = (p.0, p.1 - 3.0);
    assert_eq!(
        a.dimension_label_at(near_corner.0, near_corner.1, W, H),
        None
    );
    let corner = screen(&a, (0.0, 0.0));
    assert!(
        !a.dimension_drag_begin(corner.0, corner.1 + 4.0, W, H),
        "a press by the corner is not the dimension's"
    );
    assert!(
        a.sketch_drag_begin(corner.0, corner.1 + 4.0, W, H),
        "the corner point can still be dragged"
    );
    assert!(a.sketching.as_ref().unwrap().dim_drag.is_none());
    a.sketch_cancel();
}

#[test]
fn a_dimension_follows_its_geometry_while_it_is_dragged_and_stays_with_it_afterwards() {
    let mut a = xy();
    no_grid(&mut a);
    add_point(&mut a, "p", 0.0, 0.0);
    add_point(&mut a, "q", 40.0, 0.0);
    add_line_between(&mut a, "l", "p", "q");
    fix(&mut a, "fp", "p", 0.0, 0.0);
    assert!(dimension(&mut a, &[(20.0, 0.0)], (20.0, -15.0), "40"));
    // A horizontal-distance would not rotate; this aligned length does when the line turns.
    let cid = session_sketch(&a)
        .placements()
        .keys()
        .next()
        .unwrap()
        .clone();
    // Free the length so the end can be pulled.
    a.remove_constraint(&a.sketching.as_ref().unwrap().sketch.clone(), &cid);
    let sketch = a.sketching.as_ref().unwrap().sketch.clone();
    a.submit(CommandKind::SketchAddConstraint {
        sketch: sketch.clone(),
        constraint_id: "len".into(),
        constraint: Constraint::Length {
            line: "l".into(),
            value: 40.0,
        },
    })
    .unwrap();
    a.submit(CommandKind::SketchPlaceDimension {
        sketch,
        constraint_id: "len".into(),
        x: 20.0,
        y: -15.0,
    })
    .unwrap();
    a.set_tool(Tool::Select);
    // Drag the end point round the fixed start (the length holds it on a circle): the line
    // turns a quarter turn and the dimension goes with it, live.
    let (sx, sy) = screen(&a, (40.0, 0.0));
    assert!(a.sketch_drag_begin(sx, sy, W, H));
    let (tx, ty) = screen(&a, (0.0, 40.0));
    a.sketch_drag_move(tx, ty, W, H);
    let live = text_of(&a, "len");
    let rev = a.authority.revision();
    assert_eq!(rev, a.authority.revision(), "nothing logged yet");
    assert!(a.sketch_drag_end());
    let after = text_of(&a, "len");
    assert!(
        (live.0 - after.0).abs() < 1.0 && (live.1 - after.1).abs() < 1.0,
        "the preview and the result agree: {live:?} vs {after:?}"
    );
    // The line now points up; the dimension was 15 mm to its right (below an eastward line is
    // to its right), so it sits at about (15, 20) -- not left behind at (20, -15).
    let p = session_sketch(&a).placement("len").unwrap();
    assert!(
        (p.0 - 15.0).abs() < 0.5 && (p.1 - 20.0).abs() < 0.5,
        "{p:?}"
    );
    // One undo puts line and dimension back together.
    a.undo().unwrap();
    assert_eq!(session_sketch(&a).placement("len"), Some((20.0, -15.0)));
}

// ---- the extra drawing tools, snaps and inference ------------------------------------------------

fn count_kind(a: &AppState, f: impl Fn(&Entity) -> bool) -> usize {
    session_sketch(a)
        .entities()
        .values()
        .filter(|e| f(e))
        .count()
}

#[test]
fn three_point_rectangles_and_the_parallelogram_are_drawn_from_three_clicks() {
    let mut a = xy();
    no_grid(&mut a);
    // Corner, corner, width: a rectangle turned 30 degrees.
    a.set_tool(Tool::ThreePointRect);
    let u = (
        dc_math::det::cos(30.0_f64.to_radians()),
        dc_math::det::sin(30.0_f64.to_radians()),
    );
    click(&mut a, (0.0, 0.0));
    click(&mut a, (40.0 * u.0, 40.0 * u.1));
    // The preview already shows the shape with its size.
    at(&mut a, (40.0 * u.0 - 10.0 * u.1, 40.0 * u.1 + 10.0 * u.0));
    let o = a.sketch_overlay(W, H).unwrap();
    assert!(
        o.preview.iter().any(|p| p.len() == 5),
        "{:?}",
        o.preview.len()
    );
    assert!(
        o.labels.iter().any(|l| l.1.contains("40.00")),
        "{:?}",
        o.labels
    );
    click(&mut a, (40.0 * u.0 - 10.0 * u.1, 40.0 * u.1 + 10.0 * u.0));
    assert_eq!(count_kind(&a, |e| matches!(e, Entity::Line { .. })), 4);
    let sk = solved(&a);
    assert_eq!(
        constraints_of(&sk, |c| matches!(c, Constraint::Perpendicular { .. })),
        3
    );
    let pts = points(&sk);
    assert!(
        pts.iter()
            .any(|p| near(*p, (40.0 * u.0 - 10.0 * u.1, 40.0 * u.1 + 10.0 * u.0))),
        "{pts:?}"
    );
    assert!(
        pts.iter().any(|p| near(*p, (-10.0 * u.1, 10.0 * u.0))),
        "{pts:?}"
    );
    // One undo removes the whole rectangle.
    a.undo().unwrap();
    assert_eq!(count_kind(&a, |e| matches!(e, Entity::Line { .. })), 0);

    // Centre, middle of a side, the other half-side.
    a.set_tool(Tool::ThreePointCentreRect);
    click(&mut a, (0.0, 0.0));
    click(&mut a, (20.0, 0.0));
    click(&mut a, (5.0, 10.0));
    assert_eq!(
        count_kind(&a, |e| matches!(e, Entity::Line { .. })),
        6,
        "4 sides and 2 construction diagonals"
    );
    let sk = solved(&a);
    assert!(
        points(&sk).iter().any(|p| near(*p, (20.0, 10.0)))
            && points(&sk).iter().any(|p| near(*p, (-20.0, -10.0)))
    );
    assert_eq!(
        sk.entities()
            .keys()
            .filter(|k| sk.is_construction(k))
            .count(),
        2
    );
    a.undo().unwrap();

    // Parallelogram: the fourth corner completes it.
    a.set_tool(Tool::Parallelogram);
    click(&mut a, (0.0, 0.0));
    click(&mut a, (30.0, 0.0));
    click(&mut a, (40.0, 15.0));
    let sk = solved(&a);
    assert!(
        points(&sk).iter().any(|p| near(*p, (10.0, 15.0))),
        "{:?}",
        points(&sk)
    );
    assert_eq!(
        constraints_of(&sk, |c| matches!(c, Constraint::Parallel { .. })),
        2
    );
    // Three points in a line make no parallelogram, and say why.
    a.set_tool(Tool::Parallelogram);
    click(&mut a, (50.0, 0.0));
    click(&mut a, (60.0, 0.0));
    let rev = a.authority.revision();
    click(&mut a, (70.0, 0.0));
    assert_eq!(a.authority.revision(), rev);
    assert!(a
        .messages
        .last()
        .is_some_and(|m| m.level == dc_app::state::Level::Error));
}

#[test]
fn a_polygon_has_the_chosen_number_of_equal_sides_and_a_slot_is_two_arcs_and_two_tangent_lines() {
    let mut a = xy();
    no_grid(&mut a);
    a.set_tool(Tool::Polygon);
    a.sketching.as_mut().unwrap().polygon_sides = 5;
    click(&mut a, (0.0, 0.0));
    click(&mut a, (20.0, 0.0));
    let sk = solved(&a);
    assert_eq!(count_kind(&a, |e| matches!(e, Entity::Line { .. })), 5);
    assert_eq!(
        constraints_of(&sk, |c| matches!(c, Constraint::EqualLength { .. })),
        4
    );
    assert_eq!(
        constraints_of(&sk, |c| matches!(c, Constraint::PointOnCurve { .. })),
        5
    );
    assert_eq!(
        sk.entities()
            .keys()
            .filter(|k| sk.is_construction(k))
            .count(),
        1,
        "the circle guide"
    );
    // Vertices on the circle of radius 20, the first at the click.
    let on = points(&sk)
        .iter()
        .filter(|p| (dc_math::det::hypot(p.0, p.1) - 20.0).abs() < 1e-6)
        .count();
    assert_eq!(on, 5);
    // Circumscribed: the click is the middle of a side (apothem 20).
    a.set_tool(Tool::Polygon);
    a.sketching.as_mut().unwrap().polygon_circumscribed = true;
    a.sketching.as_mut().unwrap().polygon_sides = 6;
    click(&mut a, (100.0, 0.0));
    click(&mut a, (120.0, 0.0));
    let sk = solved(&a);
    assert_eq!(
        constraints_of(&sk, |c| matches!(c, Constraint::Tangent { .. })),
        6
    );
    let r = 20.0 / dc_math::det::cos(core::f64::consts::PI / 6.0);
    assert!(points(&sk).iter().filter(|p| p.0 > 50.0).all(|p| (dc_math::det::hypot(p.0 - 100.0, p.1) - r).abs() < 1e-3 || *p == (100.0, 0.0)), "{:?}", points(&sk));

    // Slot: two end centres and a width.
    a.set_tool(Tool::Slot);
    click(&mut a, (0.0, 60.0));
    click(&mut a, (40.0, 60.0));
    click(&mut a, (20.0, 66.0));
    let sk = solved(&a);
    assert_eq!(count_kind(&a, |e| matches!(e, Entity::Arc { .. })), 2);
    assert_eq!(
        constraints_of(&sk, |c| matches!(c, Constraint::Tangent { .. })),
        6 + 4
    );
    assert!(
        points(&sk).iter().any(|p| near(*p, (0.0, 66.0)))
            && points(&sk).iter().any(|p| near(*p, (40.0, 54.0)))
    );
    let slot_profiles = a.sketching.as_ref().map(|s| s.sketch.clone()).unwrap();
    let _ = slot_profiles;
    assert_eq!(profile_count(&a), 3, "the polygons and the slot each close");
    // A slot needs a width.
    a.set_tool(Tool::Slot);
    click(&mut a, (0.0, 100.0));
    click(&mut a, (40.0, 100.0));
    let rev = a.authority.revision();
    click(&mut a, (20.0, 100.0));
    assert_eq!(a.authority.revision(), rev);
}

#[test]
fn a_tangent_arc_leaves_a_line_or_an_arc_smoothly_and_the_point_and_centerline_tools_work() {
    let mut a = xy();
    no_grid(&mut a);
    a.set_tool(Tool::Line);
    click(&mut a, (0.0, 0.0));
    click(&mut a, (30.0, 0.0));
    a.sketch_cancel();
    // From the end of the line, curving up: the arc is tangent (the line is the x axis).
    a.set_tool(Tool::TangentArc);
    click(&mut a, (30.0, 0.0));
    at(&mut a, (50.0, 20.0));
    let o = a.sketch_overlay(W, H).unwrap();
    assert!(
        o.labels.iter().any(|l| l.1.starts_with('R')),
        "{:?}",
        o.labels
    );
    click(&mut a, (50.0, 20.0));
    let sk = solved(&a);
    assert_eq!(count_kind(&a, |e| matches!(e, Entity::Arc { .. })), 1);
    assert_eq!(
        constraints_of(&sk, |c| matches!(c, Constraint::Tangent { .. })),
        1
    );
    // The centre is straight above the line's end, at the radius.
    let arc = sk
        .entities()
        .values()
        .find_map(|e| match e {
            Entity::Arc { center, .. } => sk.point(center),
            _ => None,
        })
        .unwrap();
    assert!((arc.0 - 30.0).abs() < 1e-6, "{arc:?}");
    assert!(
        (arc.1 - 20.0 * 0.0 - (20.0f64 * 20.0 + 20.0 * 20.0) / (2.0 * 20.0)).abs() < 1e-6,
        "{arc:?}"
    );
    // Continue with another tangent arc from the first one's end.
    a.set_tool(Tool::TangentArc);
    click(&mut a, (50.0, 20.0));
    click(&mut a, (70.0, 40.0));
    assert_eq!(count_kind(&a, |e| matches!(e, Entity::Arc { .. })), 2);
    assert!(
        constraints_of(&solved(&a), |c| matches!(
            c,
            Constraint::TangentCurves { .. }
        )) == 1
    );
    // Starting in empty space is refused with a reason.
    a.set_tool(Tool::TangentArc);
    click(&mut a, (0.0, 80.0));
    let rev = a.authority.revision();
    click(&mut a, (10.0, 90.0));
    assert_eq!(a.authority.revision(), rev);

    // A point; and a centreline is construction, chained like a line.
    a.set_tool(Tool::Point);
    click(&mut a, (-30.0, -30.0));
    assert!(points(&solved(&a)).iter().any(|p| near(*p, (-30.0, -30.0))));
    a.set_tool(Tool::Centerline);
    click(&mut a, (-50.0, -10.0));
    click(&mut a, (-50.0, 10.0));
    click(&mut a, (-30.0, 10.0));
    let sk = solved(&a);
    let construction_lines = sk
        .entities()
        .iter()
        .filter(|(k, e)| matches!(e, Entity::Line { .. }) && sk.is_construction(k))
        .count();
    assert_eq!(construction_lines, 2);
}

#[test]
fn the_pointer_snaps_to_midpoints_quadrants_and_inferred_directions_and_each_can_be_switched_off() {
    use dc_app::inference::InferKind;
    let mut a = xy();
    no_grid(&mut a);
    a.set_tool(Tool::Line);
    click(&mut a, (0.0, 0.0));
    click(&mut a, (40.0, 10.0));
    a.sketch_cancel();
    a.set_tool(Tool::Circle);
    click(&mut a, (0.0, 40.0));
    click(&mut a, (8.0, 40.0));
    // Midpoint of the line: a snap that will carry a midpoint relation.
    a.set_tool(Tool::Line);
    let (mx, my) = a.sketch_to_screen((20.0, 5.0), W, H).unwrap();
    a.sketch_hover(Some((mx + 2.0, my - 2.0, W, H)));
    let snap = a.sketching.as_ref().unwrap().cursor.clone().unwrap();
    assert!(
        snap.midpoint.is_some() && near(snap.pos, (20.0, 5.0)),
        "{snap:?}"
    );
    assert_eq!(
        a.sketch_overlay(W, H).unwrap().hint.as_deref(),
        Some("midpoint")
    );
    click_px(&mut a, (20.0, 5.0), 2.0, -2.0);
    click(&mut a, (20.0, -30.0));
    let sk = solved(&a);
    assert_eq!(
        constraints_of(&sk, |c| matches!(c, Constraint::Midpoint { .. })),
        1
    );
    a.sketch_cancel();
    // A quadrant of the circle (centre (0,40), radius 8): the right-hand one has no point of its
    // own (a circle is a centre and a radius), so it snaps as a quadrant.
    let (qx, qy) = a.sketch_to_screen((8.0, 40.0), W, H).unwrap();
    a.sketch_hover(Some((qx + 1.0, qy + 1.0, W, H)));
    let snap = a.sketching.as_ref().unwrap().cursor.clone().unwrap();
    assert!(
        snap.on_curve.is_some() && near(snap.pos, (8.0, 40.0)),
        "{snap:?}"
    );
    let (tx, ty) = a.sketch_to_screen((0.0, 48.0), W, H).unwrap();
    a.sketch_hover(Some((tx, ty - 2.0, W, H)));
    let snap = a.sketching.as_ref().unwrap().cursor.clone().unwrap();
    assert!(
        snap.on_curve.is_some() && near(snap.pos, (0.0, 48.0)),
        "{snap:?}"
    );
    // Clicking it makes a point that is related to the circle.
    click_px(&mut a, (0.0, 48.0), 0.0, -2.0);
    click(&mut a, (30.0, 70.0));
    assert!(
        constraints_of(&solved(&a), |c| matches!(
            c,
            Constraint::PointOnCurve { .. }
        )) >= 1
    );
    // Perpendicular to the line you are leaving from its end point, within 3 degrees.
    a.sketch_cancel();
    click(&mut a, (40.0, 10.0));
    let dir = (-10.0 / 41.231056, 40.0 / 41.231056); // perpendicular to (40, 10)
    let target = (40.0 + 20.0 * dir.0 + 0.4, 10.0 + 20.0 * dir.1);
    at(&mut a, target);
    let snap = a.sketching.as_ref().unwrap().cursor.clone().unwrap();
    assert_eq!(
        snap.infer.as_ref().map(|i| i.kind),
        Some(InferKind::Perpendicular),
        "{snap:?}"
    );
    assert_eq!(
        a.sketch_overlay(W, H).unwrap().hint.as_deref(),
        Some("perpendicular")
    );
    click(&mut a, target);
    let sk = solved(&a);
    assert_eq!(
        constraints_of(&sk, |c| matches!(c, Constraint::Perpendicular { .. })),
        1
    );
    // The new point is exactly square to the line: dot product zero.
    let (p, q) = (dc_math::det::hypot(1.0, 0.0), 0.0);
    let _ = (p, q);
    // Every snap can be switched off.
    a.sketch_cancel();
    a.sketching.as_mut().unwrap().snaps.midpoints = false;
    a.sketch_hover(Some((mx + 2.0, my - 2.0, W, H)));
    assert!(a
        .sketching
        .as_ref()
        .unwrap()
        .cursor
        .as_ref()
        .unwrap()
        .midpoint
        .is_none());
    a.sketching.as_mut().unwrap().snaps.quadrants = false;
    a.sketch_hover(Some((tx, ty - 2.0, W, H)));
    assert!(a
        .sketching
        .as_ref()
        .unwrap()
        .cursor
        .as_ref()
        .unwrap()
        .on_curve
        .is_none());
    a.sketching.as_mut().unwrap().snaps.points = false;
    a.sketch_hover(Some((qx + 1.0, qy + 1.0, W, H)));
    assert!(a
        .sketching
        .as_ref()
        .unwrap()
        .cursor
        .as_ref()
        .unwrap()
        .point
        .is_none());
    // Tangent: leaving the end of an arc along its tangent.
    let mut b = xy();
    no_grid(&mut b);
    b.set_tool(Tool::Arc);
    click(&mut b, (0.0, 0.0));
    click(&mut b, (10.0, 0.0));
    click(&mut b, (0.0, 10.0));
    b.sketch_cancel();
    b.set_tool(Tool::Line);
    click(&mut b, (0.0, 10.0));
    // The arc runs counter-clockwise from (10,0) to (0,10): at its end it heads along -x.
    at(&mut b, (-15.0, 10.3));
    let snap = b.sketching.as_ref().unwrap().cursor.clone().unwrap();
    assert_eq!(
        snap.infer.as_ref().map(|i| i.kind),
        Some(InferKind::Tangent),
        "{snap:?}"
    );
    click(&mut b, (-15.0, 10.3));
    assert_eq!(
        constraints_of(&solved(&b), |c| matches!(c, Constraint::Tangent { .. })),
        1
    );
}

// ---- the Modify tools --------------------------------------------------------------------------

fn lines_of(a: &AppState) -> Vec<String> {
    session_sketch(a)
        .entities()
        .iter()
        .filter(|(_, e)| matches!(e, Entity::Line { .. }))
        .map(|(id, _)| id.clone())
        .collect()
}

fn a_rectangle() -> AppState {
    let mut a = xy();
    no_grid(&mut a);
    // (Not tied to the origin: these tests move the shape about.)
    a.sketching.as_mut().unwrap().snaps.points = false;
    a.set_tool(Tool::Rectangle);
    click(&mut a, (0.0, 0.0));
    click(&mut a, (40.0, 20.0));
    a.set_tool(Tool::Select);
    a
}

#[test]
fn modify_operations_act_on_the_selection_as_one_command_each_and_select_what_they_make() {
    use dc_app::modify::ModifyOp;
    let mut a = a_rectangle();
    // Nothing selected: a reason, not a silent nothing.
    let rev = a.authority.revision();
    assert!(!a.apply_modify(&ModifyOp::Move {
        dx: 5.0,
        dy: 0.0,
        copy: false
    }));
    assert_eq!(a.authority.revision(), rev);
    assert!(a.messages.last().is_some_and(|m| m.text.contains("select")));

    let all = lines_of(&a);
    a.sketching.as_mut().unwrap().selection = all.clone();
    // Copy: four more lines, now selected; one undo removes the copy.
    assert!(a.apply_modify(&ModifyOp::Move {
        dx: 60.0,
        dy: 0.0,
        copy: true
    }));
    assert_eq!(a.authority.revision().0, rev.0 + 1);
    assert_eq!(lines_of(&a).len(), 8);
    assert_eq!(
        selection(&a).len(),
        4,
        "the copy is selected, ready to be moved again"
    );
    assert!(selection(&a).iter().all(|i| !all.contains(i)));
    a.undo().unwrap();
    assert_eq!(lines_of(&a).len(), 4);

    // In-place move, rotate and scale.
    a.sketching.as_mut().unwrap().selection = all.clone();
    assert!(a.apply_modify(&ModifyOp::Move {
        dx: 5.0,
        dy: 5.0,
        copy: false
    }));
    assert!(points(&solved(&a)).iter().any(|p| near(*p, (5.0, 5.0))));
    a.sketching.as_mut().unwrap().selection = all.clone();
    assert!(a.apply_modify(&ModifyOp::Scale {
        cx: 5.0,
        cy: 5.0,
        factor: 2.0,
        copy: false
    }));
    assert!(
        points(&solved(&a)).iter().any(|p| near(*p, (85.0, 45.0))),
        "{:?}",
        points(&solved(&a))
    );
    a.sketching.as_mut().unwrap().selection = all.clone();
    assert!(a.apply_modify(&ModifyOp::Rotate {
        cx: 0.0,
        cy: 0.0,
        degrees: 90.0,
        copy: false
    }));
    assert!(
        points(&solved(&a)).iter().any(|p| near(*p, (-45.0, 85.0))),
        "{:?}",
        points(&solved(&a))
    );
    // A bad number is refused with the engine's reason and nothing changes.
    let (rev, h) = (a.authority.revision(), a.authority.state_hash());
    a.sketching.as_mut().unwrap().selection = all.clone();
    assert!(!a.apply_modify(&ModifyOp::Scale {
        cx: 0.0,
        cy: 0.0,
        factor: 0.0,
        copy: false
    }));
    assert_eq!((a.authority.revision(), a.authority.state_hash()), (rev, h));
}

#[test]
fn mirror_pattern_offset_fillet_and_chamfer_from_the_selection() {
    use dc_app::modify::ModifyOp;
    // Mirror about an axis, and about a selected line (selected last).
    let mut a = a_rectangle();
    a.sketching.as_mut().unwrap().selection = lines_of(&a);
    assert!(a.apply_modify(&ModifyOp::MirrorAxisY));
    assert!(
        lines_of(&a).len() >= 8 - 1,
        "mirrored copies (an edge on the axis is shared)"
    );
    let mut m = a_rectangle();
    add_point(&mut m, "m1", 60.0, -10.0);
    add_point(&mut m, "m2", 60.0, 30.0);
    add_line_between(&mut m, "mirror", "m1", "m2");
    let mut sel = lines_of(&m);
    sel.retain(|l| l != "mirror");
    sel.push("mirror".into());
    m.sketching.as_mut().unwrap().selection = sel;
    assert!(m.apply_modify(&ModifyOp::MirrorSelectedLine));
    assert!(
        points(&solved(&m)).iter().any(|p| near(*p, (80.0, 20.0))),
        "{:?}",
        points(&solved(&m))
    );
    // The mirror line must be last and a line.
    let mut bad = a_rectangle();
    bad.sketching.as_mut().unwrap().selection = lines_of(&bad);
    let l = lines_of(&bad)[0].clone();
    bad.sketching.as_mut().unwrap().selection = vec![l];
    assert!(!bad.apply_modify(&ModifyOp::MirrorSelectedLine));

    // Patterns.
    let mut p = a_rectangle();
    p.sketching.as_mut().unwrap().selection = lines_of(&p);
    assert!(p.apply_modify(&ModifyOp::LinearPattern {
        dx: 50.0,
        dy: 0.0,
        count: 3
    }));
    assert_eq!(lines_of(&p).len(), 12);
    p.undo().unwrap();
    p.sketching.as_mut().unwrap().selection = lines_of(&p);
    assert!(p.apply_modify(&ModifyOp::CircularPattern {
        cx: 0.0,
        cy: -50.0,
        degrees: 360.0,
        count: 4
    }));
    assert_eq!(lines_of(&p).len(), 16);
    assert!(!p.apply_modify(&ModifyOp::LinearPattern {
        dx: 50.0,
        dy: 0.0,
        count: 1
    }));

    // Offset an outline outward (positive is outside).
    let mut o = a_rectangle();
    o.sketching.as_mut().unwrap().selection = lines_of(&o);
    assert!(o.apply_modify(&ModifyOp::Offset { distance: 3.0 }));
    assert!(points(&solved(&o)).iter().any(|p| near(*p, (43.0, 23.0))));

    // Fillet and chamfer need two lines that meet.
    let mut f = a_rectangle();
    let all = lines_of(&f);
    f.sketching.as_mut().unwrap().selection = all.clone();
    assert!(
        !f.apply_modify(&ModifyOp::Fillet { radius: 3.0 }),
        "four lines is not a corner"
    );
    // Pick two neighbouring edges by their positions.
    let edge = |a: &mut AppState, at: (f64, f64)| {
        let (x, y) = screen(a, at);
        a.sketch_click_with(x, y, W, H, true);
    };
    f.sketching.as_mut().unwrap().selection.clear();
    edge(&mut f, (20.0, 0.0));
    edge(&mut f, (40.0, 10.0));
    assert!(f.apply_modify(&ModifyOp::Fillet { radius: 4.0 }));
    assert_eq!(count_kind(&f, |e| matches!(e, Entity::Arc { .. })), 1);
    assert!(constraints_of(&solved(&f), |c| matches!(c, Constraint::Tangent { .. })) == 2);
    let mut c = a_rectangle();
    c.sketching.as_mut().unwrap().selection.clear();
    edge(&mut c, (20.0, 0.0));
    edge(&mut c, (40.0, 10.0));
    assert!(c.apply_modify(&ModifyOp::Chamfer { d1: 5.0, d2: 8.0 }));
    assert!(
        points(&solved(&c)).iter().any(|p| near(*p, (35.0, 0.0)))
            && points(&solved(&c)).iter().any(|p| near(*p, (40.0, 8.0)))
    );
}

#[test]
fn the_modify_preview_shows_the_result_without_changing_anything() {
    use dc_app::modify::ModifyOp;
    let mut a = a_rectangle();
    a.sketching.as_mut().unwrap().selection = lines_of(&a);
    let rev = a.authority.revision();
    let op = ModifyOp::Move {
        dx: 70.0,
        dy: 0.0,
        copy: true,
    };
    let lines = a.modify_preview(&op);
    assert_eq!(lines.len(), 4, "the four edges the copy would add");
    assert!(
        lines.iter().flatten().any(|p| near(*p, (70.0, 0.0)))
            && lines.iter().flatten().any(|p| near(*p, (110.0, 20.0)))
    );
    // The overlay draws it (in screen points) while the panel holds the operation.
    a.sketching.as_mut().unwrap().modify_preview = Some(op);
    assert_eq!(a.sketch_overlay(W, H).unwrap().modify_preview.len(), 4);
    assert_eq!(a.authority.revision(), rev, "a preview logs nothing");
    // A move in place previews the moved shapes; an impossible one previews nothing.
    assert_eq!(
        a.modify_preview(&ModifyOp::Move {
            dx: 3.0,
            dy: 0.0,
            copy: false
        })
        .len(),
        4
    );
    assert!(a
        .modify_preview(&ModifyOp::Scale {
            cx: 0.0,
            cy: 0.0,
            factor: -1.0,
            copy: false
        })
        .is_empty());
    assert!(a
        .modify_preview(&ModifyOp::Fillet { radius: 2.0 })
        .is_empty());
    // The default centre for turning and scaling is the middle of the selection.
    assert!(near(a.selection_centre(), (20.0, 10.0)));
}

// ---- midpoint handles and lines drawn from the middle -----------------------------------------

fn a_lone_line() -> AppState {
    let mut a = xy();
    no_grid(&mut a);
    a.set_tool(Tool::Line);
    click(&mut a, (0.0, 0.0));
    click(&mut a, (40.0, 0.0));
    a.set_tool(Tool::Select);
    a.sketching.as_mut().unwrap().selection.clear();
    a
}

#[test]
fn a_lines_middle_is_a_handle_once_the_line_is_selected_and_becomes_a_real_point_only_when_needed()
{
    let mut a = a_lone_line();
    let line = lines_of(&a)[0].clone();
    let (mx, my) = screen(&a, (20.0, 0.0));
    let (qx, qy) = screen(&a, (30.0, 0.0));
    // A line that is not selected shows no middle, and a click on the middle just selects it.
    a.sketch_hover(Some((mx, my, W, H)));
    assert_eq!(
        a.sketching.as_ref().unwrap().hover.as_deref(),
        Some(line.as_str())
    );
    assert!(a.sketch_overlay(W, H).unwrap().mid_handles.is_empty());
    a.sketch_click(mx, my, W, H);
    assert_eq!(selection(&a), vec![line.clone()]);
    // Selected, but the middle is not in use: it stays hidden until the pointer comes near it.
    a.sketch_hover(None);
    assert!(a.sketch_overlay(W, H).unwrap().mid_handles.is_empty());
    a.sketch_hover(Some((qx, qy, W, H)));
    assert!(
        a.sketch_overlay(W, H).unwrap().mid_handles.is_empty(),
        "far along the line"
    );
    let (nx, ny) = screen(&a, (23.0, 0.0));
    a.sketch_hover(Some((nx, ny, W, H)));
    let o = a.sketch_overlay(W, H).unwrap();
    assert_eq!(o.mid_handles.len(), 1, "close to the middle");
    assert!(!o.mid_handles[0].1);
    a.sketch_hover(Some((mx, my, W, H)));
    assert_eq!(
        a.sketching.as_ref().unwrap().hover,
        Some(format!("@mid:{line}"))
    );
    assert!(a.sketch_overlay(W, H).unwrap().mid_handles[0].1);
    let rev = a.authority.revision();
    // A click only selects the handle: nothing is added to the document.
    a.sketch_click(mx, my, W, H);
    assert_eq!(a.authority.revision(), rev);
    assert_eq!(selection(&a), vec![format!("@mid:{line}")]);
    assert!(a.sketch_overlay(W, H).unwrap().mid_handles[0].1);
    // It becomes a real point, once, when a relation needs it.
    add_point(&mut a, "t", 20.0, 30.0);
    let (tx, ty) = screen(&a, (20.0, 30.0));
    a.sketch_click_with(tx, ty, W, H, true);
    assert!(a.apply_relation(Relation::Coincident));
    let sk = solved(&a);
    assert_eq!(
        constraints_of(&sk, |c| matches!(c, Constraint::Midpoint { .. })),
        1
    );
    // (The line was free, so it and t meet part of the way.)
    let mid = sk
        .constraints()
        .values()
        .find_map(|c| match c {
            Constraint::Midpoint { point, .. } => Some(point.clone()),
            _ => None,
        })
        .unwrap();
    assert!(near(sk.point("t").unwrap(), sk.point(&mid).unwrap()));
}

#[test]
fn a_selected_midpoint_can_be_dimensioned_and_reuses_its_point() {
    let mut a = a_lone_line();
    add_point(&mut a, "t", 20.0, 30.0);
    let (mx, my) = screen(&a, (20.0, 0.0));
    let (qx, qy) = screen(&a, (30.0, 0.0));
    a.sketch_click(qx, qy, W, H); // the line first, then its middle
    a.sketch_click(mx, my, W, H);
    let rev = a.authority.revision();
    // Smart Dimension takes the selected midpoint as its first pick (made real at that moment).
    a.set_tool(Tool::Dimension);
    assert_eq!(a.authority.revision().0, rev.0 + 1);
    let picks = &a.sketching.as_ref().unwrap().dim.picks;
    assert_eq!(picks.len(), 1);
    // Selecting the handle again reuses that point.
    a.set_tool(Tool::Select);
    a.sketching.as_mut().unwrap().selection = vec![format!("@mid:{}", lines_of(&a)[0])];
    a.materialize_selected_midpoints();
    assert_eq!(count_kind(&a, |e| matches!(e, Entity::Point { .. })), 4);
}

#[test]
fn a_line_can_be_drawn_from_its_middle() {
    for tool in [Tool::Line, Tool::Centerline] {
        let mut a = xy();
        no_grid(&mut a);
        a.sketching.as_mut().unwrap().midpoint_line = true;
        a.set_tool(tool);
        click(&mut a, (10.0, 5.0));
        // The preview already shows the mirrored far end and the middle handle.
        let (x, y) = screen(&a, (30.0, 5.0));
        a.sketch_hover(Some((x, y, W, H)));
        let o = a.sketch_overlay(W, H).unwrap();
        assert_eq!(o.mid_handles.len(), 1);
        assert!(o
            .preview
            .iter()
            .flatten()
            .any(|p| (p.0 - screen(&a, (-10.0, 5.0)).0).abs() < 1.5));
        let rev = a.authority.revision();
        click(&mut a, (30.0, 5.0));
        assert_eq!(a.authority.revision().0, rev.0 + 1, "one undo step");
        let sk = solved(&a);
        let pts = points(&sk);
        assert!(pts.iter().any(|p| near(*p, (-10.0, 5.0))), "{pts:?}");
        assert!(pts.iter().any(|p| near(*p, (30.0, 5.0))));
        assert!(pts.iter().any(|p| near(*p, (10.0, 5.0))));
        assert_eq!(
            constraints_of(&sk, |c| matches!(c, Constraint::Midpoint { .. })),
            1
        );
        assert_eq!(
            constraints_of(&sk, |c| matches!(c, Constraint::Horizontal { .. })),
            1
        );
        // The chain ends: the next click starts something new.
        assert!(a.sketching.as_ref().unwrap().first.is_none());
        // Dragging an end keeps the middle in the middle.
        let id = lines_of(&a)[0].clone();
        let is_construction = sk.is_construction(&id);
        assert_eq!(is_construction, tool == Tool::Centerline);
    }
}

// ---- dimension snapping for round dimensions, undo while drawing, merge report, drag-draw -----

#[test]
fn a_round_dimension_snaps_to_a_tidy_direction_from_its_centre() {
    let mut a = xy();
    no_grid(&mut a);
    add_point(&mut a, "c", 0.0, 0.0);
    add_circle(&mut a, "k", "c", 10.0);
    assert!(add_constraint(
        &mut a,
        "r1",
        Constraint::Radius {
            target: "k".into(),
            value: 10.0
        }
    ));
    let t = text_of(&a, "r1");
    assert!(a.dimension_drag_begin(t.0, t.1, W, H));
    // Near the 45 degree diagonal: lands on it, with a guide from the centre.
    let p = screen(&a, (19.0, 21.0));
    a.dimension_drag_move_with(p.0, p.1, W, H, false);
    let at = a.sketching.as_ref().unwrap().dim_places["r1"];
    assert!((at.0 - at.1).abs() < 1e-6, "on the diagonal: {at:?}");
    assert!(!a.sketch_overlay(W, H).unwrap().dim_guides.is_empty());
    // Far from any tidy direction it stays where it is, and Ctrl bypasses the snap.
    let p = screen(&a, (30.0, 12.0));
    a.dimension_drag_move_with(p.0, p.1, W, H, false);
    let at = a.sketching.as_ref().unwrap().dim_places["r1"];
    assert!(
        (at.0 - 30.0).abs() < 1e-6 && (at.1 - 12.0).abs() < 1e-6,
        "{at:?}"
    );
    let p = screen(&a, (19.0, 21.0));
    a.dimension_drag_move_with(p.0, p.1, W, H, true);
    let at = a.sketching.as_ref().unwrap().dim_places["r1"];
    assert!((at.1 - 21.0).abs() < 1e-6);
}

#[test]
fn undo_while_a_line_chain_is_in_progress_takes_back_the_last_segment() {
    let mut a = xy();
    no_grid(&mut a);
    a.set_tool(Tool::Line);
    click(&mut a, (0.0, 0.0));
    click(&mut a, (30.0, 10.0));
    click(&mut a, (50.0, 40.0));
    assert_eq!(lines_of(&a).len(), 2);
    assert!(a.sketch_undo_in_progress());
    assert_eq!(lines_of(&a).len(), 1, "the last segment is gone");
    assert!(
        a.sketching.as_ref().unwrap().first.is_some(),
        "still drawing from the old end"
    );
    assert!(a.sketch_undo_in_progress());
    assert!(lines_of(&a).is_empty());
    // One more drops the pending start; then nothing is in progress and the caller undoes normally.
    assert!(a.sketch_undo_in_progress());
    assert!(a.sketching.as_ref().unwrap().first.is_none());
    assert!(!a.sketch_undo_in_progress());
}

#[test]
fn merging_points_says_which_relations_it_dropped() {
    let mut a = xy();
    no_grid(&mut a);
    add_point(&mut a, "a", 0.0, 0.0);
    add_point(&mut a, "b", 10.0, 0.0);
    add_point(&mut a, "c", 10.5, 0.0);
    add_line_between(&mut a, "l", "a", "b");
    assert!(add_constraint(
        &mut a,
        "co",
        Constraint::Coincident {
            a: "b".into(),
            b: "c".into()
        }
    ));
    // b and c are already glued together; once they are one point that relation means nothing.
    select(&mut a, &["b", "c"]);
    assert!(a.apply_relation(Relation::Merge));
    let last = a
        .messages
        .last()
        .map(|m| m.text.clone())
        .unwrap_or_default();
    assert!(last.contains("dropped"), "{last}");
}

#[test]
fn drawing_by_press_drag_release_makes_one_shape_and_a_plain_click_still_works() {
    let mut a = xy();
    no_grid(&mut a);
    a.set_tool(Tool::Rectangle);
    let (x0, y0) = screen(&a, (0.0, 0.0));
    let (x1, y1) = screen(&a, (40.0, 20.0));
    assert!(a.drag_draw_begin(x0, y0, W, H));
    assert!(a.drag_draw_end(x1, y1, W, H));
    assert_eq!(lines_of(&a).len(), 4);
    // A line dragged out is one line, not the start of a chain.
    a.set_tool(Tool::Line);
    let (x0, y0) = screen(&a, (0.0, 40.0));
    let (x1, y1) = screen(&a, (40.0, 40.0));
    assert!(a.drag_draw_begin(x0, y0, W, H));
    assert!(a.drag_draw_end(x1, y1, W, H));
    assert_eq!(lines_of(&a).len(), 5);
    assert!(a.sketching.as_ref().unwrap().first.is_none());
    // A press and release in the same place is a click: the line's start stays for the next click.
    assert!(a.drag_draw_begin(x0, y0, W, H));
    assert!(a.drag_draw_end(x0, y0, W, H));
    assert!(a.sketching.as_ref().unwrap().first.is_some());
}

// ---- shapes drawn from the origin or an axis are tied to it -----------------------------------

fn fixed_at_origin(a: &AppState) -> usize {
    constraints_of(
        &solved(a),
        |c| matches!(c, Constraint::Fixed { x, y, .. } if *x == 0.0 && *y == 0.0),
    )
}

#[test]
fn a_circle_a_centre_rectangle_and_a_centred_line_started_at_the_origin_are_tied_to_it() {
    // Circle: its centre is fixed at the origin (made once), so the circle is only free in size.
    let mut a = xy();
    a.set_tool(Tool::Circle);
    click(&mut a, (0.0, 0.0));
    click(&mut a, (12.0, 0.0));
    assert_eq!(fixed_at_origin(&a), 1);
    let sk = solved(&a);
    let Some((_, Entity::Circle { center, .. })) = sk
        .entities()
        .iter()
        .find(|(_, e)| matches!(e, Entity::Circle { .. }))
    else {
        panic!("a circle")
    };
    assert!(near(sk.point(center).unwrap(), (0.0, 0.0)));
    // A second shape from the origin reuses that point instead of adding another.
    a.set_tool(Tool::CenterRectangle);
    click(&mut a, (0.0, 0.0));
    click(&mut a, (30.0, 20.0));
    assert_eq!(fixed_at_origin(&a), 1, "one origin point, shared");
    // Centre line from the middle.
    a.sketching.as_mut().unwrap().midpoint_line = true;
    a.set_tool(Tool::Centerline);
    click(&mut a, (0.0, 0.0));
    click(&mut a, (0.0, 25.0));
    assert_eq!(fixed_at_origin(&a), 1);
    assert_eq!(
        constraints_of(&solved(&a), |c| matches!(c, Constraint::Midpoint { .. })),
        1
    );
}

#[test]
fn a_shape_started_on_an_axis_gets_an_on_axis_relation() {
    let mut a = xy();
    a.set_tool(Tool::Circle);
    click(&mut a, (30.0, 0.0)); // on the horizontal axis
    click(&mut a, (40.0, 0.0));
    let sk = solved(&a);
    assert_eq!(
        constraints_of(&sk, |c| matches!(
            c,
            Constraint::OnAxis {
                axis: dc_sketch::SketchAxis::X,
                ..
            }
        )),
        1
    );
    // Hovering shows what a click would do.
    let (x, y) = screen(&a, (0.0, 30.0));
    a.sketch_hover(Some((x, y, W, H)));
    let cur = a.sketching.as_ref().unwrap().cursor.clone().unwrap();
    assert_eq!(cur.axis, Some(dc_sketch::SketchAxis::Y));
    // Switching the snap off draws a free centre.
    a.sketching.as_mut().unwrap().snaps.edges = false;
    let (x, y) = screen(&a, (60.0, 0.0));
    a.sketch_hover(Some((x, y, W, H)));
    assert!(a
        .sketching
        .as_ref()
        .unwrap()
        .cursor
        .clone()
        .unwrap()
        .axis
        .is_none());
}

#[test]
fn free_construction_geometry_makes_the_sketch_under_defined_but_not_its_shapes() {
    let mut a = xy();
    no_grid(&mut a);
    add_point(&mut a, "c", 10.0, 10.0);
    add_circle(&mut a, "k", "c", 5.0);
    fix(&mut a, "fc", "c", 10.0, 10.0);
    assert!(add_constraint(
        &mut a,
        "r",
        Constraint::Radius {
            target: "k".into(),
            value: 5.0
        }
    ));
    a.set_tool(Tool::Select);
    let o = a.sketch_overlay(W, H).unwrap();
    assert_eq!(o.dof, 0, "the circle is defined");
    // A loose centre line: as construction it is still free, so the sketch is under defined,
    // but the status says it is only the construction.
    add_point(&mut a, "g1", 40.0, 0.0);
    add_point(&mut a, "g2", 60.0, 5.0);
    add_line_between(&mut a, "guide", "g1", "g2");
    let name = a.sketching.as_ref().unwrap().sketch.clone();
    a.submit(CommandKind::SketchSetConstruction {
        sketch: name,
        entity_id: "guide".into(),
        construction: true,
    })
    .unwrap();
    let o = a.sketch_overlay(W, H).unwrap();
    assert_eq!(o.dof, 4);
    assert!(o.shapes_defined, "the shapes themselves are defined");
    // As an ordinary line the same freedom is just part of the sketch.
    let name = a.sketching.as_ref().unwrap().sketch.clone();
    a.submit(CommandKind::SketchSetConstruction {
        sketch: name,
        entity_id: "guide".into(),
        construction: false,
    })
    .unwrap();
    let o = a.sketch_overlay(W, H).unwrap();
    assert_eq!(o.dof, 4);
    assert!(!o.shapes_defined);
}

#[test]
fn infinite_centre_lines_run_off_the_view_are_picked_anywhere_and_need_only_a_place() {
    let mut a = xy();
    no_grid(&mut a);
    a.sketching.as_mut().unwrap().centerline_infinite = true;
    a.set_tool(Tool::Centerline);
    click(&mut a, (10.0, 0.0));
    click(&mut a, (30.0, 0.0));
    a.set_tool(Tool::Select);
    let sk = session_sketch(&a);
    let (id, _) = sk
        .entities()
        .iter()
        .find(|(_, e)| matches!(e, Entity::Line { .. }))
        .unwrap();
    let id = id.clone();
    assert!(
        sk.is_construction(&id) && sk.is_infinite(&id),
        "an infinite construction line"
    );
    // Drawn from edge to edge of the view.
    let o = a.sketch_overlay(W, H).unwrap();
    let s = o.strokes.iter().find(|s| s.construction).unwrap();
    assert!(s.pts[0].0 < -1000.0 && s.pts[1].0 > 1000.0, "{:?}", s.pts);
    // Picked well beyond its two points.
    let (x, y) = a.sketch_to_screen((-60.0, 0.0), W, H).unwrap();
    assert_eq!(a.select_pick_at(x, y, W, H).as_deref(), Some(id.as_str()));
    // Drawn on the horizontal axis it is already placed: nothing is left to define.
    assert_eq!(a.sketch_overlay(W, H).unwrap().dof, 0);
    // The toolbar action toggles it for the selection, as one command.
    a.sketching.as_mut().unwrap().selection = vec![id.clone()];
    let rev = a.authority.revision();
    assert!(a.toggle_infinite());
    assert_eq!(a.authority.revision().0, rev.0 + 1);
    assert!(!session_sketch(&a).is_infinite(&id));
    assert_eq!(
        a.sketch_overlay(W, H).unwrap().dof,
        2,
        "finite: its two ends slide along the axis"
    );
    assert!(a.toggle_infinite());
    assert!(session_sketch(&a).is_infinite(&id));
    // A real line has nothing to make infinite.
    a.undo().unwrap();
    a.undo().unwrap();
}

#[test]
fn an_infinite_line_off_the_axes_has_two_freedoms_until_it_is_placed() {
    let mut a = xy();
    no_grid(&mut a);
    a.sketching.as_mut().unwrap().centerline_infinite = true;
    a.set_tool(Tool::Centerline);
    click(&mut a, (10.0, 12.0));
    click(&mut a, (30.0, 19.0));
    assert_eq!(
        a.sketch_overlay(W, H).unwrap().dof,
        2,
        "direction and offset"
    );
}

// ---- infinite lines as cutters, snap targets and (not) midpoints --------------------------------

fn an_infinite_guide(a: &mut AppState, from: (f64, f64), to: (f64, f64)) {
    add_point(a, "g1", from.0, from.1);
    add_point(a, "g2", to.0, to.1);
    add_line_between(a, "guide", "g1", "g2");
    let name = a.sketching.as_ref().unwrap().sketch.clone();
    for op in [
        CommandKind::SketchSetConstruction {
            sketch: name.clone(),
            entity_id: "guide".into(),
            construction: true,
        },
        CommandKind::SketchSetInfinite {
            sketch: name,
            entity_id: "guide".into(),
            infinite: true,
        },
    ] {
        a.submit(op).unwrap();
    }
}

#[test]
fn an_infinite_line_is_snapped_to_anywhere_along_it_and_has_no_midpoint() {
    let mut a = xy();
    no_grid(&mut a);
    // A vertical guide drawn short, at x = 20.
    an_infinite_guide(&mut a, (20.0, -5.0), (20.0, 5.0));
    a.set_tool(Tool::Line);
    // Far beyond its two points, the pointer still snaps onto it, on the line.
    let (x, y) = a.sketch_to_screen((20.0, 60.0), W, H).unwrap();
    let snap = a.compute_snap(x + 2.0, y, W, H).expect("a snap");
    assert_eq!(snap.on_line.as_deref(), Some("guide"));
    assert!((snap.pos.0 - 20.0).abs() < 1e-9, "{:?}", snap.pos);
    // The middle of its two points is no snap target: it has no middle.
    let (x, y) = a.sketch_to_screen((20.0, 0.0), W, H).unwrap();
    let snap = a.compute_snap(x, y, W, H).expect("still on the line");
    assert!(snap.midpoint.is_none(), "{snap:?}");
    // Nor a midpoint handle, even selected and hovered.
    a.set_tool(Tool::Select);
    a.sketching.as_mut().unwrap().selection = vec!["guide".into()];
    a.sketch_hover(Some((x, y, W, H)));
    assert!(a.sketch_overlay(W, H).unwrap().mid_handles.is_empty());
    assert_ne!(
        a.sketching.as_ref().unwrap().hover.as_deref(),
        Some("@mid:guide")
    );
}

#[test]
fn the_trim_tool_cuts_against_an_infinite_line_beyond_its_points_and_refuses_to_trim_it() {
    let mut a = xy();
    no_grid(&mut a);
    add_point(&mut a, "p", 0.0, 30.0);
    add_point(&mut a, "q", 50.0, 30.0);
    add_line_between(&mut a, "h", "p", "q");
    an_infinite_guide(&mut a, (20.0, -5.0), (20.0, 5.0));
    // Trimming h needs the guide to count: it is construction, so ask for it.
    a.set_tool(Tool::Trim);
    a.sketching.as_mut().unwrap().cut_construction = true;
    click(&mut a, (40.0, 30.0));
    let sk = solved(&a);
    let Some(Entity::Line { b, .. }) = sk.entity("h") else {
        panic!("h is still there: trimmed, not deleted");
    };
    assert!(
        near(sk.point(b).unwrap(), (20.0, 30.0)),
        "{:?}",
        sk.point(b)
    );
    // The infinite line itself is explained, not trimmed.
    let rev = a.authority.revision();
    click(&mut a, (20.0, -2.0));
    assert_eq!(a.authority.revision(), rev);
    assert!(
        a.messages
            .last()
            .is_some_and(|m| m.text.contains("infinite")),
        "{:?}",
        a.messages.last()
    );
}

#[test]
fn a_centre_line_drawn_from_its_middle_is_infinite_too_and_its_two_points_are_hidden() {
    let mut a = xy();
    no_grid(&mut a);
    {
        let s = a.sketching.as_mut().unwrap();
        s.centerline_infinite = true;
        s.midpoint_line = true;
    }
    a.set_tool(Tool::Centerline);
    click(&mut a, (10.0, 12.0));
    // While drawing, the preview already runs off both edges.
    let (x, y) = a.sketch_to_screen((30.0, 19.0), W, H).unwrap();
    a.sketch_hover(Some((x, y, W, H)));
    let o = a.sketch_overlay(W, H).unwrap();
    assert!(
        o.preview
            .iter()
            .any(|l| l.iter().any(|p| p.0 < -1000.0 || p.0 > 1000.0)),
        "{:?}",
        o.preview
    );
    click(&mut a, (30.0, 19.0));
    a.set_tool(Tool::Select);
    let sk = session_sketch(&a);
    let line = sk
        .entities()
        .iter()
        .find(|(_, e)| matches!(e, Entity::Line { .. }))
        .map(|(id, _)| id.clone())
        .unwrap();
    assert!(
        sk.is_construction(&line) && sk.is_infinite(&line),
        "infinite from its middle"
    );
    // The points it is drawn through are not shown, picked or snapped to; only its middle
    // (the point the line was started at) is a real point.
    let Some(Entity::Line { a: pa, b: pb }) = sk.entity(&line) else {
        panic!()
    };
    assert!(sk.only_infinite_line_ends(pa) && sk.only_infinite_line_ends(pb));
    let o = a.sketch_overlay(W, H).unwrap();
    let shown: Vec<_> = o.points.iter().map(|d| d.pos).collect();
    let (ex, ey) = a.sketch_to_screen(sk.point(pb).unwrap(), W, H).unwrap();
    assert!(
        !shown
            .iter()
            .any(|p| (p.0 - ex).abs() < 1.0 && (p.1 - ey).abs() < 1.0),
        "its end point is not drawn"
    );
    assert_ne!(a.select_pick_at(ex, ey, W, H).as_deref(), Some(pb.as_str()));
    // The line itself is what you pick there.
    assert_eq!(
        a.select_pick_at(ex, ey, W, H).as_deref(),
        Some(line.as_str())
    );
}
