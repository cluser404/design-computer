//! Selecting in the 3D view: Ctrl-click adds and removes, a rubber band takes what is inside or
//! what it touches, feature-first mode drills down, and selected edges and faces carry into the
//! commands that use them.

use dc_app::featuring::FeatureKind;
use dc_app::sketching::Tool;
use dc_app::state::{AppState, SelectMode, Selected};
use dc_sketch::PlaneSpec;

const W: f64 = 800.0;
const H: f64 = 600.0;

fn click(a: &mut AppState, p: (f64, f64)) {
    let (x, y) = a.sketch_to_screen(p, W, H).expect("on screen");
    a.sketch_hover(Some((x, y, W, H)));
    a.sketch_click(x, y, W, H);
}

fn plate() -> AppState {
    let mut a = AppState::new();
    a.begin_sketch(PlaneSpec::Xy { offset: 0.0 }).unwrap();
    a.sketching.as_mut().unwrap().snap_grid = false;
    a.set_tool(Tool::Rectangle);
    click(&mut a, (0.0, 0.0));
    click(&mut a, (40.0, 20.0));
    a.end_sketch();
    assert!(a.begin_feature(FeatureKind::Extrude));
    a.feature_draft.as_mut().unwrap().depth = 10.0;
    assert!(a.commit_feature_draft());
    a.fit();
    a
}

fn pixel_on_edge(a: &AppState, edge: &str) -> (f64, f64) {
    let body = a.scene.bodies.iter().find(|b| b.id == "extrude1").unwrap();
    for seg in body.edge_segments(edge) {
        let mid = (seg[0] + seg[1]) * 0.5;
        if let Some(p) = a.camera.project(mid, W, H) {
            if matches!(a.pick_hit_at(p.x, p.y, W, H), Some(dc_render::Hit::Edge { name, .. }) if name == edge)
            {
                return (p.x, p.y);
            }
        }
    }
    panic!("no visible spot on {edge}");
}

fn face_names(a: &AppState) -> Vec<String> {
    a.selection()
        .into_iter()
        .filter_map(|s| match s {
            Selected::Face { face, .. } => Some(face),
            _ => None,
        })
        .collect()
}

#[test]
fn ctrl_click_adds_and_takes_out_and_a_plain_click_replaces() {
    let mut a = plate();
    let (e1, e2) = (
        pixel_on_edge(&a, "cap_end|side:l1"),
        pixel_on_edge(&a, "side:l1|side:l2"),
    );
    a.click_view(e1.0, e1.1, W, H, false);
    assert_eq!(a.selection().len(), 1);
    a.click_view(e2.0, e2.1, W, H, true);
    let sel = a.selection();
    assert_eq!(sel.len(), 2, "{sel:?}");
    assert_eq!(
        a.selected,
        *sel.last().unwrap(),
        "the last one clicked is the primary"
    );
    // Ctrl-click the first again: out. The primary stays.
    a.click_view(e1.0, e1.1, W, H, true);
    assert_eq!(a.selection().len(), 1);
    assert!(matches!(a.selected, Selected::Edge { ref edge, .. } if edge == "side:l1|side:l2"));
    // Add it back, then a plain click replaces everything.
    a.click_view(e1.0, e1.1, W, H, true);
    assert_eq!(a.selection().len(), 2);
    a.click_view(e1.0, e1.1, W, H, false);
    assert_eq!(a.selection().len(), 1);
    // Ctrl-click on nothing changes nothing; a plain click on nothing clears.
    a.click_view(e2.0, e2.1, W, H, true);
    assert_eq!(a.selection().len(), 2);
    a.click_view(1.0, 1.0, W, H, true);
    assert_eq!(a.selection().len(), 2);
    a.click_view(1.0, 1.0, W, H, false);
    assert!(a.selection().is_empty() && a.selected == Selected::None);
    // Setting the primary directly (as the tree does) drops the rest.
    a.set_selection(vec![
        Selected::Feature("extrude1".into()),
        Selected::Sketch("sketch1".into()),
    ]);
    assert_eq!(a.selection().len(), 2);
    a.selected = Selected::Feature("extrude1".into());
    assert_eq!(a.selection(), vec![Selected::Feature("extrude1".into())]);
}

#[test]
fn a_rubber_band_to_the_right_takes_what_is_inside_and_to_the_left_what_it_touches() {
    let mut a = plate();
    // Everything: three faces and nine edges are visible in the isometric view.
    a.box_select((0.0, 0.0, W, H), false, false, W, H);
    let n = a.selection();
    assert_eq!(
        n.iter()
            .filter(|s| matches!(s, Selected::Face { .. }))
            .count(),
        3,
        "{n:?}"
    );
    assert_eq!(
        n.iter()
            .filter(|s| matches!(s, Selected::Edge { .. }))
            .count(),
        9,
        "{n:?}"
    );
    // The filter limits it.
    a.filter.faces = false;
    a.filter.sketch_lines = false;
    a.box_select((0.0, 0.0, W, H), false, false, W, H);
    assert!(!a.selection().is_empty());
    assert!(a
        .selection()
        .iter()
        .all(|s| matches!(s, Selected::Edge { .. })));
    a.filter.faces = true;
    a.filter.sketch_lines = true;
    // A small box on the top face: touching takes the face, enclosing takes nothing.
    let c = a
        .camera
        .project(dc_math::Vec3::new(20.0, 10.0, 10.0), W, H)
        .unwrap();
    let small = (c.x - 6.0, c.y - 6.0, c.x + 6.0, c.y + 6.0);
    a.box_select(small, false, false, W, H);
    assert!(a.selection().is_empty());
    a.box_select(small, true, false, W, H);
    assert_eq!(face_names(&a), vec!["cap_end".to_owned()]);
    // With `add` it joins what is selected; corners in any order give the same box.
    let e = pixel_on_edge(&a, "side:l1|side:l2");
    a.box_select(
        (e.0 + 6.0, e.1 + 6.0, e.0 - 6.0, e.1 - 6.0),
        true,
        true,
        W,
        H,
    );
    let sel = a.selection();
    assert!(
        sel.contains(&Selected::Face {
            feature: "extrude1".into(),
            face: "cap_end".into()
        }),
        "{sel:?}"
    );
    assert!(
        sel.iter()
            .any(|s| matches!(s, Selected::Edge { edge, .. } if edge == "side:l1|side:l2")),
        "{sel:?}"
    );
    // Selected faces other than the primary are outlined in the view.
    assert!(dc_app::highlight::highlights(&a)
        .iter()
        .any(|h| h.kind == dc_app::highlight::HighlightKind::Selected));
}

#[test]
fn feature_first_mode_picks_the_feature_and_the_next_click_drills_down() {
    let mut a = plate();
    a.select_mode = SelectMode::FeatureFirst;
    let c = a
        .camera
        .project(dc_math::Vec3::new(20.0, 10.0, 10.0), W, H)
        .unwrap();
    // Making the part selected it already, so a click drills straight in; clear it to start over.
    a.click_view(c.x, c.y, W, H, false);
    assert!(matches!(a.selected, Selected::Face { .. }));
    a.set_selection(Vec::new());
    a.click_view(c.x, c.y, W, H, false);
    assert_eq!(a.selected, Selected::Feature("extrude1".into()));
    a.click_view(c.x, c.y, W, H, false);
    assert!(
        matches!(a.selected, Selected::Face { ref face, .. } if face == "cap_end"),
        "{:?}",
        a.selected
    );
    // From inside the feature a click on an edge drills to the edge too.
    let e = pixel_on_edge(&a, "side:l1|side:l2");
    a.click_view(e.0, e.1, W, H, false);
    assert!(
        matches!(a.selected, Selected::Edge { .. }),
        "{:?}",
        a.selected
    );
    // Clearing the selection starts over at the feature; the default mode picks directly.
    a.click_view(1.0, 1.0, W, H, false);
    assert!(a.selection().is_empty(), "{:?}", a.selection());
    a.click_view(c.x, c.y, W, H, false);
    assert_eq!(
        a.selected,
        Selected::Feature("extrude1".into()),
        "{:?}",
        a.selection()
    );
    a.select_mode = SelectMode::Direct;
    a.click_view(1.0, 1.0, W, H, false);
    a.click_view(c.x, c.y, W, H, false);
    assert!(matches!(a.selected, Selected::Face { .. }));
}

#[test]
fn selected_edges_become_the_edges_of_a_fillet_and_selected_ends_the_openings_of_a_shell() {
    let mut a = plate();
    let (e1, e2) = (
        pixel_on_edge(&a, "cap_end|side:l1"),
        pixel_on_edge(&a, "side:l1|side:l2"),
    );
    a.click_view(e1.0, e1.1, W, H, false);
    a.click_view(e2.0, e2.1, W, H, true);
    // Two edges that meet at a corner: refused by the fillet, but both are in the draft.
    assert!(a.begin_feature(FeatureKind::Fillet));
    let d = a.feature_draft.clone().unwrap();
    assert_eq!(d.edges.len(), 2, "{:?}", d.edges);
    assert_eq!(d.target.as_deref(), Some("extrude1"));
    assert!(
        a.draft_problem().is_some(),
        "two edges at one corner need a corner patch"
    );
    a.cancel_feature_draft();
    // The top face selected: the shell leaves it open.
    let c = a
        .camera
        .project(dc_math::Vec3::new(20.0, 10.0, 10.0), W, H)
        .unwrap();
    a.click_view(c.x, c.y, W, H, false);
    assert!(a.begin_feature(FeatureKind::Shell));
    let d = a.feature_draft.clone().unwrap();
    assert_eq!(d.faces, vec!["cap_end".to_owned()]);
    assert!(a.draft_problem().is_none(), "{:?}", a.draft_problem());
    assert!(a.commit_feature_draft());
    let vol = a
        .feature_rows()
        .into_iter()
        .find(|r| r.id == "shell1")
        .unwrap()
        .volume;
    assert!((vol - (8000.0 - 36.0 * 16.0 * 8.0)).abs() < 1e-6, "{vol}");
}

#[test]
fn several_features_selected_together_are_deleted_in_one_step_later_ones_first() {
    let mut a = plate();
    assert!(a.begin_feature(FeatureKind::Shell));
    a.feature_draft.as_mut().unwrap().faces = vec!["cap_end".into()];
    assert!(a.commit_feature_draft());
    assert_eq!(a.feature_rows().len(), 2);
    let rev = a.authority.revision().0;
    a.set_selection(vec![
        Selected::Feature("extrude1".into()),
        Selected::Feature("shell1".into()),
    ]);
    a.delete_selected().unwrap();
    assert!(a.feature_rows().is_empty());
    assert_eq!(a.authority.revision().0, rev + 1, "one step");
    a.undo().unwrap();
    assert_eq!(a.feature_rows().len(), 2, "one undo brings both back");
}

#[test]
fn a_hidden_edge_behind_a_crossing_edge_is_not_picked_in_a_symmetric_view() {
    // In this window shape the hidden bottom edge crosses the front top edge exactly at its
    // middle on screen; a picker whose triangles leave a crack on shared edges saw through it
    // and offered the hidden edge. (Found by a smoke test in a window of this size.)
    let (w, h) = (674.0, 746.0);
    let mut a = plate();
    let (lo, hi) = a.scene.body_bounds().unwrap();
    a.camera.fit_for(lo, hi, w / h);
    a.box_select((0.05 * w, 0.05 * h, 0.95 * w, 0.95 * h), false, false, w, h);
    let sel = a.selection();
    let edges = sel
        .iter()
        .filter(|s| matches!(s, Selected::Edge { .. }))
        .count();
    assert_eq!(edges, 9, "{sel:?}");
}
