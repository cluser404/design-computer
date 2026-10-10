//! Editing an existing feature, the Revolve / Fillet / Chamfer pages, "reverse" as a one-click
//! fix, and the design tree's organisation (names, suppress, rollback bar, folders).

use dc_app::featuring::{DraftFix, EndCondition, FeatureKind};
use dc_app::sketching::Tool;
use dc_app::state::{AppState, Selected};
use dc_app::tree::{self, NodeKind};
use dc_sketch::PlaneSpec;

const W: f64 = 800.0;
const H: f64 = 600.0;

fn click(a: &mut AppState, p: (f64, f64)) {
    let (x, y) = a.sketch_to_screen(p, W, H).expect("on screen");
    a.sketch_hover(Some((x, y, W, H)));
    a.sketch_click(x, y, W, H);
}

fn rectangle(a: &mut AppState, plane: PlaneSpec, from: (f64, f64), to: (f64, f64)) {
    a.begin_sketch(plane).unwrap();
    a.sketching.as_mut().unwrap().snap_grid = false;
    a.set_tool(Tool::Rectangle);
    click(a, from);
    click(a, to);
    a.end_sketch();
}

fn volume(a: &AppState, feature: &str) -> f64 {
    a.feature_rows()
        .into_iter()
        .find(|r| r.id == feature)
        .map(|r| r.volume)
        .unwrap_or(f64::NAN)
}

fn near(a: f64, b: f64) -> bool {
    (a - b).abs() < 1e-6 * b.abs().max(1.0)
}

fn plate() -> AppState {
    let mut a = AppState::new();
    rectangle(
        &mut a,
        PlaneSpec::Xy { offset: 0.0 },
        (0.0, 0.0),
        (40.0, 20.0),
    );
    assert!(a.begin_feature(FeatureKind::Extrude));
    a.feature_draft.as_mut().unwrap().depth = 10.0;
    assert!(a.commit_feature_draft());
    a
}

#[test]
fn editing_a_feature_opens_its_recipe_and_accepting_is_one_update_command() {
    let mut a = plate();
    assert!(near(volume(&a, "extrude1"), 8000.0));
    assert!(a.begin_edit_feature("extrude1"));
    let d = a.feature_draft.clone().unwrap();
    assert!(d.editing && d.name == "extrude1" && near(d.depth, 10.0));
    let rev = a.authority.revision();
    a.feature_draft.as_mut().unwrap().depth = 15.0;
    a.update_feature_preview();
    assert!(a.draft_problem().is_none());
    assert_eq!(a.authority.revision(), rev, "editing a draft logs nothing");
    assert!(near(volume(&a, "extrude1"), 8000.0), "not yet changed");
    assert!(a.commit_feature_draft());
    assert_eq!(a.authority.revision().0, rev.0 + 1);
    assert!(near(volume(&a, "extrude1"), 12000.0));
    assert_eq!(a.feature_rows().len(), 1, "no new feature appeared");
    // One undo puts the old recipe back.
    a.undo().unwrap();
    assert!(near(volume(&a, "extrude1"), 8000.0));
    // A feature that is off cannot be edited, with the reason.
    assert!(a.set_suppressed("extrude1", true));
    assert!(!a.begin_edit_feature("extrude1"));
}

#[test]
fn a_cut_that_misses_offers_to_go_the_other_way_and_doing_so_builds() {
    let mut a = AppState::new();
    rectangle(
        &mut a,
        PlaneSpec::Xy { offset: 0.0 },
        (0.0, 0.0),
        (40.0, 20.0),
    );
    assert!(a.begin_feature(FeatureKind::Extrude));
    {
        let d = a.feature_draft.as_mut().unwrap();
        d.depth = 10.0;
        d.reverse = true; // the part is below the sketch plane
    }
    assert!(a.commit_feature_draft());
    rectangle(
        &mut a,
        PlaneSpec::Xy { offset: 0.0 },
        (10.0, 5.0),
        (20.0, 15.0),
    );
    assert!(a.begin_feature(FeatureKind::Cut));
    {
        let d = a.feature_draft.as_mut().unwrap();
        d.end = EndCondition::Blind;
        d.depth = 3.0;
    }
    // A new cut already starts the way that reaches the part.
    assert!(a.feature_draft.as_ref().unwrap().reverse);
    assert!(a.draft_problem().is_none(), "{:?}", a.draft_problem());
    // Going the wrong way is still refused, with the fix on offer.
    a.feature_draft.as_mut().unwrap().reverse = false;
    a.update_feature_preview();
    assert!(a.draft_problem().is_some(), "cuts upward into nothing");
    assert_eq!(a.draft_fix(), Some(DraftFix::ReverseDirection));
    a.apply_draft_fix(DraftFix::ReverseDirection);
    assert!(a.draft_problem().is_none(), "{:?}", a.draft_problem());
    assert!(a.draft_fix().is_none());
    assert!(a.commit_feature_draft());
    assert!(near(volume(&a, "cut1"), 8000.0 - 10.0 * 10.0 * 3.0));
}

#[test]
fn a_revolve_page_turns_a_profile_about_the_chosen_axis() {
    let mut a = AppState::new();
    // A 5..10 by 0..20 rectangle turned about the sketch's y axis is a ring: pi (100 - 25) 20.
    rectangle(
        &mut a,
        PlaneSpec::Xy { offset: 0.0 },
        (5.0, 0.0),
        (10.0, 20.0),
    );
    assert!(a.begin_feature(FeatureKind::Revolve));
    let d = a.feature_draft.clone().unwrap();
    assert_eq!(d.axis, "y");
    assert!(a.draft_problem().is_none(), "{:?}", a.draft_problem());
    assert!(a.commit_feature_draft());
    let want = std::f64::consts::PI * 75.0 * 20.0;
    assert!(
        (volume(&a, "revolve1") - want).abs() < 0.5,
        "{}",
        volume(&a, "revolve1")
    );
    // And its recipe can be reopened.
    assert!(a.begin_edit_feature("revolve1"));
    assert_eq!(a.feature_draft.as_ref().unwrap().kind, FeatureKind::Revolve);
}

#[test]
fn fillet_and_chamfer_take_edges_as_pairs_of_faces_and_say_what_is_missing() {
    for (kind, name) in [
        (FeatureKind::Fillet, "fillet1"),
        (FeatureKind::Chamfer, "chamfer1"),
    ] {
        let mut a = plate();
        assert!(a.begin_feature(kind));
        assert!(
            a.draft_problem().unwrap().contains("click the edges"),
            "{:?}",
            a.draft_problem()
        );
        let sides: Vec<String> = a
            .faces_of("extrude1")
            .into_iter()
            .map(|f| f.name)
            .filter(|n| n.starts_with("side:"))
            .collect();
        assert_eq!(sides.len(), 4);
        // Some pair of side faces meets at an edge: take the first pair that builds.
        let mut found = false;
        'pairs: for i in 0..4 {
            for j in 0..4 {
                if i == j {
                    continue;
                }
                a.feature_draft.as_mut().unwrap().edges =
                    vec![[sides[i].clone(), sides[j].clone()]];
                a.update_feature_preview();
                if a.draft_problem().is_none() {
                    found = true;
                    break 'pairs;
                }
            }
        }
        assert!(found, "no adjacent pair builds");
        a.feature_draft.as_mut().unwrap().size = 2.0;
        a.update_feature_preview();
        assert!(a.commit_feature_draft());
        assert!(
            volume(&a, name) < 8000.0,
            "material was taken off: {}",
            volume(&a, name)
        );
        // Reopen: the edges are in the page.
        assert!(a.begin_edit_feature(name));
        assert_eq!(a.feature_draft.as_ref().unwrap().edges.len(), 1);
    }
}

#[test]
fn the_tree_shows_names_off_features_the_rollback_bar_and_folders() {
    let mut a = AppState::new();
    a.load_demo();
    let ids: Vec<String> = a.authority.state().ordered_features();
    assert!(ids.len() >= 2);
    let first = ids[0].clone();
    let last = ids.last().unwrap().clone();
    // A name shown instead of the id.
    assert!(a.rename(true, &first, "Base plate"));
    let t = tree::build(&a, "");
    assert!(t.children.iter().any(|n| n.label == "Base plate"));
    // Filtering finds it by the new name.
    assert!(tree::build(&a, "base plate")
        .walk()
        .iter()
        .any(|n| n.label == "Base plate"));
    // A suppressed feature is marked, with the reason.
    assert!(a.set_suppressed(&last, true));
    let t = tree::build(&a, "");
    let n = t
        .walk()
        .into_iter()
        .find(|n| n.kind == NodeKind::Feature(last.clone()))
        .unwrap();
    assert_eq!(n.off, Some("suppressed"));
    assert!(a.set_suppressed(&last, false));
    // The rollback bar: the node it follows carries it, later ones are off, and creation refuses.
    assert!(a.set_rollback(Some(&first)));
    let t = tree::build(&a, "");
    let bar: Vec<_> = t.walk().into_iter().filter(|n| n.bar_after).collect();
    assert_eq!(bar.len(), 1);
    assert_eq!(bar[0].kind, NodeKind::Feature(first.clone()));
    let n = t
        .walk()
        .into_iter()
        .find(|n| n.kind == NodeKind::Feature(last.clone()))
        .unwrap();
    assert_eq!(n.off, Some("after the rollback bar"));
    assert!(
        !a.begin_feature(FeatureKind::Extrude),
        "nothing can be added while rolled back"
    );
    assert!(a.set_rollback(None));
    // Folders gather features under one node.
    assert!(a.move_to_folder(&first, Some("Base")));
    let t = tree::build(&a, "");
    let folder = t
        .children
        .iter()
        .find(|n| n.kind == NodeKind::Folder("Base".into()))
        .expect("folder");
    assert_eq!(folder.children.len(), 1);
    assert_eq!(folder.children[0].kind, NodeKind::Feature(first.clone()));
    assert!(a.move_to_folder(&first, None));
    assert!(tree::build(&a, "")
        .children
        .iter()
        .all(|n| !matches!(n.kind, NodeKind::Folder(_))));
    // Selection still works on renamed things, and every step is undoable.
    a.select(Selected::Feature(first.clone()));
    // rename, suppress, unsuppress, bar, bar removed, into folder, out of it: seven steps.
    for _ in 0..7 {
        a.undo().unwrap();
    }
    assert_eq!(a.authority.state().label("feature", &first), first);
}

#[test]
fn mass_properties_are_exact_and_activity_names_who_did_what() {
    let a = AppState::new();
    assert!(a.mass_properties().is_none(), "no part, no answer");
    assert!(a.activity(10).is_empty());
    let mut a = plate();
    let t = a.mass_properties().expect("a part");
    assert!(t.contains("volume 8000.000 mm3"), "{t}");
    assert!(t.contains("surface area 2800.000 mm2"), "{t}");
    assert!(t.contains("40.000 x 20.000 x 10.000"), "{t}");
    assert!(a.rename(true, "extrude1", "Base"));
    assert!(a.mass_properties().unwrap().contains("'Base'"));
    let act = a.activity(3);
    assert_eq!(act[0].2, "feature.set_label");
    assert_eq!(act[0].1, "human:local");
    assert!(act[0].0 > act[1].0, "newest first");
}

#[test]
fn a_refused_draft_draws_what_it_is_about_in_red() {
    let mut a = AppState::new();
    rectangle(
        &mut a,
        PlaneSpec::Xy { offset: 0.0 },
        (0.0, 0.0),
        (40.0, 20.0),
    );
    assert!(a.begin_feature(FeatureKind::Extrude));
    {
        let d = a.feature_draft.as_mut().unwrap();
        d.depth = 10.0;
        d.reverse = true;
    }
    assert!(a.commit_feature_draft());
    rectangle(
        &mut a,
        PlaneSpec::Xy { offset: 0.0 },
        (10.0, 5.0),
        (20.0, 15.0),
    );
    assert!(a.begin_feature(FeatureKind::Cut));
    {
        let d = a.feature_draft.as_mut().unwrap();
        d.depth = 3.0;
        d.reverse = false; // wrong way on purpose: a new cut picks the right way itself
    }
    a.update_feature_preview();
    let p = a.feature_preview.clone().unwrap();
    assert!(p.result.is_err());
    assert!(!p.entities.is_empty(), "the refusal names what it is about");
    let red = a
        .scene
        .lines
        .iter()
        .filter(|l| l.color == [220, 40, 40])
        .count();
    let named_sketch = p.entities.iter().any(|e| e.starts_with("sketch/"));
    assert_eq!(red > 0, named_sketch, "{:?}", p.entities);
    // Once it builds, nothing is red.
    a.apply_draft_fix(DraftFix::ReverseDirection);
    assert!(a.scene.lines.iter().all(|l| l.color != [220, 40, 40]));
}

const VW: f64 = 800.0;
const VH: f64 = 600.0;

/// A pixel that picks `edge` of `feature` in the current view (searching the drawn edges).
fn pixel_on_edge(a: &AppState, feature: &str, edge: &str) -> (f64, f64) {
    let body = a.scene.bodies.iter().find(|b| b.id == feature).unwrap();
    for seg in body.edge_segments(edge) {
        let mid = (seg[0] + seg[1]) * 0.5;
        if let Some(p) = a.camera.project(mid, VW, VH) {
            if matches!(
                a.pick_hit_at(p.x, p.y, VW, VH),
                Some(dc_render::Hit::Edge { name, .. }) if name == edge
            ) {
                return (p.x, p.y);
            }
        }
    }
    panic!("no visible spot on {edge}");
}

#[test]
fn edges_are_hovered_selected_and_chosen_for_rounding_by_clicking_them() {
    let mut a = plate();
    a.fit();
    // Without a command: hovering an edge highlights it, clicking selects it, Delete refuses.
    let (x, y) = pixel_on_edge(&a, "extrude1", "cap_end|side:l1");
    a.hover_at(Some((x, y, VW, VH)));
    assert!(a.hover_other.is_some() && a.hovered.is_none());
    a.click_at(x, y, VW, VH);
    assert_eq!(
        a.selected,
        Selected::Edge {
            feature: "extrude1".into(),
            edge: "cap_end|side:l1".into()
        }
    );
    let e = a.delete_selected().unwrap_err();
    assert!(e.message.contains("cannot be deleted"), "{}", e.message);
    // The highlight layers: the selection is drawn.
    assert!(dc_app::highlight::highlights(&a)
        .iter()
        .any(|h| h.kind == dc_app::highlight::HighlightKind::Selected));
    // With the selection filter off for edges a click on the same spot picks the face.
    a.filter.edges = false;
    a.click_at(x, y, VW, VH);
    assert!(
        matches!(a.selected, Selected::Face { .. }),
        "{:?}",
        a.selected
    );
    a.filter.edges = true;

    // Fillet: only edges are offered, clicking adds and clicking again removes.
    assert!(a.begin_feature(FeatureKind::Fillet));
    let (x, y) = pixel_on_edge(&a, "extrude1", "side:l1|side:l2");
    a.click_at(x, y, VW, VH);
    assert_eq!(
        a.feature_draft.as_ref().unwrap().edges,
        [["side:l1".to_owned(), "side:l2".to_owned()]]
    );
    assert!(a.feature_draft.as_ref().is_some_and(|d| d.edges.len() == 1));
    assert!(a.draft_problem().is_none(), "{:?}", a.draft_problem());
    a.click_at(x, y, VW, VH);
    assert!(a.feature_draft.as_ref().unwrap().edges.is_empty());
    // A click on a face while rounding changes nothing.
    let sel = a.selected.clone();
    a.click_at(VW / 2.0, VH / 2.0, VW, VH);
    assert_eq!(a.selected, sel);
    // From an edge selection the rounding opens with that edge in.
    a.cancel_feature_draft();
    a.select(Selected::Edge {
        feature: "extrude1".into(),
        edge: "side:l2|side:l3".into(),
    });
    a.begin_rounding_with_edge(FeatureKind::Chamfer, "extrude1", "side:l2|side:l3");
    let d = a.feature_draft.clone().unwrap();
    assert_eq!(d.kind, FeatureKind::Chamfer);
    assert_eq!(d.edges, [["side:l2".to_owned(), "side:l3".to_owned()]]);
}

#[test]
fn a_sketch_line_in_the_view_is_the_revolve_axis_and_can_be_selected() {
    let mut a = AppState::new();
    rectangle(
        &mut a,
        PlaneSpec::Xy { offset: 0.0 },
        (5.0, 0.0),
        (10.0, 20.0),
    );
    // A free line along the y axis, 3 mm to the left of the rectangle: not part of any region.
    a.edit_sketch("sketch1");
    a.sketching.as_mut().unwrap().snap_grid = false;
    a.set_tool(Tool::Line);
    let click = |a: &mut AppState, p: (f64, f64)| {
        let (x, y) = a.sketch_to_screen(p, VW, VH).unwrap();
        a.sketch_hover(Some((x, y, VW, VH)));
        a.sketch_click(x, y, VW, VH);
    };
    click(&mut a, (-3.0, 0.0));
    click(&mut a, (-3.0, 30.0));
    a.end_sketch();
    // The free line is the last one drawn: l5, from the origin to (-3, 30).
    let line = "l5".to_owned();
    assert_eq!(a.entity_kind("sketch1", &line), "line");
    assert!(a.begin_feature(FeatureKind::Revolve));
    assert_eq!(a.feature_draft.as_ref().unwrap().axis, "y");
    a.camera = dc_render::Camera::preset(dc_render::ViewPreset::Top);
    a.fit();
    let p = a
        .camera
        .project(dc_math::Vec3::new(-1.5, 15.0, 0.0), VW, VH)
        .unwrap();
    a.click_at(p.x, p.y, VW, VH);
    assert_eq!(a.feature_draft.as_ref().unwrap().axis, line);
    // The axis is drawn in its own colour.
    assert!(dc_app::highlight::highlights(&a)
        .iter()
        .any(|h| h.kind == dc_app::highlight::HighlightKind::Axis));
    // Outside a command, the same line is selectable.
    a.cancel_feature_draft();
    a.click_at(p.x, p.y, VW, VH);
    assert_eq!(
        a.selected,
        Selected::SketchEntity {
            sketch: "sketch1".into(),
            entity: line
        }
    );
}

#[test]
fn the_rim_of_a_cylinder_is_picked_in_the_view_and_chamfered_then_filleted() {
    let mut a = AppState::new();
    a.begin_sketch(PlaneSpec::Xy { offset: 0.0 }).unwrap();
    a.sketching.as_mut().unwrap().snap_grid = false;
    a.set_tool(Tool::Circle);
    let click = |a: &mut AppState, p: (f64, f64)| {
        let (x, y) = a.sketch_to_screen(p, VW, VH).unwrap();
        a.sketch_hover(Some((x, y, VW, VH)));
        a.sketch_click(x, y, VW, VH);
    };
    click(&mut a, (0.0, 0.0));
    click(&mut a, (10.0, 0.0));
    a.end_sketch();
    assert!(a.begin_feature(FeatureKind::Extrude));
    a.feature_draft.as_mut().unwrap().depth = 20.0;
    assert!(a.commit_feature_draft());
    a.camera = dc_render::Camera::preset(dc_render::ViewPreset::Iso);
    a.fit();
    let base = volume(&a, "extrude1");
    for (kind, name) in [
        (FeatureKind::Chamfer, "chamfer1"),
        (FeatureKind::Fillet, "fillet1"),
    ] {
        let target = if name == "chamfer1" {
            "extrude1"
        } else {
            "chamfer1"
        };
        let rim = if name == "chamfer1" {
            "cap_end|side:c1"
        } else {
            "cap_start|side:c1"
        };
        assert!(a.begin_feature(kind));
        assert_eq!(
            a.feature_draft.as_ref().unwrap().target.as_deref(),
            Some(target)
        );
        a.camera = dc_render::Camera::preset(dc_render::ViewPreset::Iso);
        a.fit();
        // The rim is a circle: some visible spot on it picks it.
        let (x, y) = pixel_on_edge(&a, target, rim);
        a.click_at(x, y, VW, VH);
        assert_eq!(a.feature_draft.as_ref().unwrap().edges.len(), 1, "{kind:?}");
        a.feature_draft.as_mut().unwrap().size = 2.0;
        a.update_feature_preview();
        assert!(a.draft_problem().is_none(), "{:?}", a.draft_problem());
        assert!(a.commit_feature_draft(), "{kind:?}");
        assert!(volume(&a, name) < base, "{name}: {}", volume(&a, name));
    }
    // The faces of the result include a cone and a torus.
    let kinds: Vec<String> = a
        .faces_of("fillet1")
        .into_iter()
        .map(|f| f.surface)
        .collect();
    assert!(
        kinds.contains(&"cone".to_owned()) && kinds.contains(&"torus".to_owned()),
        "{kinds:?}"
    );
}

#[test]
fn a_cut_can_be_made_from_the_top_of_a_filleted_cylinder() {
    let mut a = AppState::new();
    a.begin_sketch(PlaneSpec::Xy { offset: 0.0 }).unwrap();
    a.sketching.as_mut().unwrap().snap_grid = false;
    a.set_tool(Tool::Circle);
    let click = |a: &mut AppState, p: (f64, f64)| {
        let (x, y) = a.sketch_to_screen(p, VW, VH).unwrap();
        a.sketch_hover(Some((x, y, VW, VH)));
        a.sketch_click(x, y, VW, VH);
    };
    click(&mut a, (0.0, 0.0));
    click(&mut a, (10.0, 0.0));
    a.end_sketch();
    assert!(a.begin_feature(FeatureKind::Extrude));
    a.feature_draft.as_mut().unwrap().depth = 20.0;
    assert!(a.commit_feature_draft());
    // Fillet the top rim.
    a.camera = dc_render::Camera::preset(dc_render::ViewPreset::Iso);
    a.fit();
    assert!(a.begin_feature(FeatureKind::Fillet));
    let (x, y) = pixel_on_edge(&a, "extrude1", "cap_end|side:c1");
    a.click_at(x, y, VW, VH);
    a.feature_draft.as_mut().unwrap().size = 2.0;
    a.update_feature_preview();
    assert!(a.commit_feature_draft());
    let rounded = volume(&a, "fillet1");
    // Sketch on the flat top and cut a hole into the part.
    a.select(Selected::Face {
        feature: "fillet1".into(),
        face: "cap_end".into(),
    });
    a.begin_sketch(PlaneSpec::Face {
        feature: "fillet1".into(),
        face: "cap_end".into(),
        offset: 0.0,
    })
    .unwrap();
    a.sketching.as_mut().unwrap().snap_grid = false;
    a.set_tool(Tool::Circle);
    click(&mut a, (0.0, 0.0));
    click(&mut a, (6.0, 0.0));
    a.end_sketch();
    assert!(a.begin_feature(FeatureKind::Cut));
    {
        let d = a.feature_draft.as_mut().unwrap();
        d.end = dc_app::featuring::EndCondition::ThroughAll;
        d.reverse = true;
    }
    a.update_feature_preview();
    assert!(a.draft_problem().is_none(), "{:?}", a.draft_problem());
    assert!(a.commit_feature_draft());
    let cut = volume(&a, "cut1");
    assert!(
        (rounded - cut - std::f64::consts::PI * 36.0 * 20.0).abs() < 1e-6 * rounded,
        "{rounded} {cut}"
    );
    // The result is drawn: its mesh exists and contains the rounded surface.
    assert!(a.scene.bodies.iter().any(|b| b.id == "cut1"));
}
