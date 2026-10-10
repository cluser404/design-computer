//! The Loft and Sweep pages: two sketches in, one feature out, through the same commands an
//! agent sends.

use dc_app::featuring::FeatureKind;
use dc_app::sketching::Tool;
use dc_app::state::AppState;
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

fn volume(a: &AppState, id: &str) -> f64 {
    a.feature_rows()
        .into_iter()
        .find(|r| r.id == id)
        .unwrap()
        .volume
}

#[test]
fn a_loft_page_joins_two_sketches_and_editing_it_changes_the_result() {
    let mut a = AppState::new();
    rectangle(
        &mut a,
        PlaneSpec::Xy { offset: 0.0 },
        (0.0, 0.0),
        (40.0, 20.0),
    );
    rectangle(
        &mut a,
        PlaneSpec::Xy { offset: 30.0 },
        (0.0, 0.0),
        (20.0, 10.0),
    );
    assert!(a.begin_feature(FeatureKind::Loft));
    let d = a.feature_draft.clone().unwrap();
    assert_ne!(
        d.sketch, d.sketch2,
        "the page starts with two different sketches"
    );
    assert!(a.draft_problem().is_none(), "{:?}", a.draft_problem());
    assert!(a.commit_feature_draft());
    // A frustum of a pyramid with rectangular ends: V = h/3 (A1 + A2 + sqrt(A1 A2)).
    let (a1, a2) = (800.0f64, 200.0f64);
    let exact = 30.0 / 3.0 * (a1 + a2 + (a1 * a2).sqrt());
    let v = volume(&a, "loft1");
    assert!((v - exact).abs() < 1e-6 * exact, "{v} vs {exact}");
    // The page opens again from the feature.
    assert!(a.begin_edit_feature("loft1"));
    let d = a.feature_draft.clone().unwrap();
    assert_eq!(d.kind, FeatureKind::Loft);
    let mut both = [d.sketch.as_str(), d.sketch2.as_str()];
    both.sort();
    assert_eq!(both, ["sketch1", "sketch2"]);
}

#[test]
fn a_sweep_page_follows_a_line_of_another_sketch() {
    let mut a = AppState::new();
    rectangle(
        &mut a,
        PlaneSpec::Xy { offset: 0.0 },
        (0.0, 0.0),
        (10.0, 6.0),
    );
    // The path: a line in the front plane (x along world x, y along world z).
    a.begin_sketch(PlaneSpec::Xz { offset: 0.0 }).unwrap();
    a.sketching.as_mut().unwrap().snap_grid = false;
    a.set_tool(Tool::Line);
    click(&mut a, (20.0, 0.0));
    click(&mut a, (20.0, 25.0));
    a.end_sketch();
    assert!(a.begin_feature(FeatureKind::Sweep));
    let d = a.feature_draft.clone().unwrap();
    assert_eq!(d.sketch, "sketch1");
    assert_eq!(d.sketch2, "sketch2");
    assert!(!d.path.is_empty(), "the one line is the default path");
    assert!(a.draft_problem().is_none(), "{:?}", a.draft_problem());
    assert!(a.commit_feature_draft());
    let v = volume(&a, "sweep1");
    assert!((v - 60.0 * 25.0).abs() < 1e-6 * 1500.0, "{v}");
    assert!(a.begin_edit_feature("sweep1"));
    assert_eq!(a.feature_draft.as_ref().unwrap().kind, FeatureKind::Sweep);
}

#[test]
fn without_a_second_sketch_the_pages_say_what_is_missing() {
    let mut a = AppState::new();
    rectangle(
        &mut a,
        PlaneSpec::Xy { offset: 0.0 },
        (0.0, 0.0),
        (10.0, 6.0),
    );
    assert!(!a.begin_feature(FeatureKind::Loft));
    assert!(!a.begin_feature(FeatureKind::Sweep));
}
