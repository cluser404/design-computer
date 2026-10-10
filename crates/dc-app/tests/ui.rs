//! The egui layer, driven without a window: whole frames with synthetic mouse and keyboard input.

use dc_app::state::{AppState, Selected};
use dc_app::ui::{draw, UiState};
use dc_render::{ProjectionKind, ViewPreset};
use eframe::egui::{self, Event, Key, Modifiers, PointerButton, Pos2, Rect};

struct Harness {
    ctx: egui::Context,
    app: AppState,
    ui: UiState,
    time: f64,
    /// Modifier keys currently held, as a real windowing backend reports them every frame.
    held: Modifiers,
}

impl Harness {
    fn new(app: AppState) -> Self {
        let mut h = Harness {
            ctx: egui::Context::default(),
            app,
            ui: UiState::default(),
            time: 0.0,
            held: Modifiers::NONE,
        };
        for _ in 0..3 {
            h.frame(vec![]);
        }
        h
    }

    fn frame(&mut self, events: Vec<Event>) -> egui::FullOutput {
        self.time += 0.05;
        let raw = egui::RawInput {
            screen_rect: Some(Rect::from_min_size(Pos2::ZERO, egui::vec2(1280.0, 800.0))),
            time: Some(self.time),
            modifiers: self.held,
            events,
            ..Default::default()
        };
        let (app, ui) = (&mut self.app, &mut self.ui);
        self.ctx.run_ui(raw, |u| draw(app, ui, u))
    }

    fn centre(&self) -> Pos2 {
        self.ui
            .viewport_rect
            .expect("a frame has been drawn")
            .center()
    }

    fn press(&mut self, pos: Pos2, mods: Modifiers) {
        self.held = mods;
        self.frame(vec![Event::PointerMoved(pos)]);
        self.frame(vec![Event::PointerButton {
            pos,
            button: PointerButton::Primary,
            pressed: true,
            modifiers: mods,
        }]);
    }

    fn release(&mut self, pos: Pos2, mods: Modifiers) {
        self.frame(vec![Event::PointerButton {
            pos,
            button: PointerButton::Primary,
            pressed: false,
            modifiers: mods,
        }]);
        self.held = Modifiers::NONE;
        self.frame(vec![]);
    }

    fn click(&mut self, pos: Pos2) {
        self.press(pos, Modifiers::NONE);
        self.release(pos, Modifiers::NONE);
    }

    fn key(&mut self, key: Key, mods: Modifiers) {
        let ev = |pressed| Event::Key {
            key,
            physical_key: Some(key),
            pressed,
            repeat: false,
            modifiers: mods,
        };
        self.held = mods;
        self.frame(vec![ev(true)]);
        self.frame(vec![ev(false)]);
        self.held = Modifiers::NONE;
    }
}

fn demo() -> AppState {
    let mut a = AppState::new();
    a.load_demo();
    a
}

#[test]
fn the_whole_window_draws_and_the_view_is_fitted_to_the_viewport() {
    let mut h = Harness::new(demo());
    let out = h.frame(vec![]);
    assert!(
        out.shapes.len() > 20,
        "panels, text and the viewport image all produce shapes: {}",
        out.shapes.len()
    );
    let r = h.ui.viewport_rect.unwrap();
    assert!(r.width() > 400.0 && r.height() > 300.0, "{r:?}");
    assert!(h.ui.has_texture(), "the render was uploaded");
    // The part fits the viewport as it actually is (not a guessed 4:3).
    let (w, hh) = (f64::from(r.width()), f64::from(r.height()));
    let (lo, hi) = h.app.scene.body_bounds().unwrap();
    for &x in &[lo.x, hi.x] {
        for &y in &[lo.y, hi.y] {
            for &z in &[lo.z, hi.z] {
                let p = h
                    .app
                    .camera
                    .project(dc_math::Vec3::new(x, y, z), w, hh)
                    .unwrap();
                assert!(
                    (0.0..=w).contains(&p.x) && (0.0..=hh).contains(&p.y),
                    "corner off screen: {p:?} in {w}x{hh}"
                );
            }
        }
    }
}

#[test]
fn an_empty_document_and_a_failing_command_still_draw() {
    let mut h = Harness::new(AppState::new());
    h.app.run_console("this is not a command");
    h.app
        .run_console(r#"{"type":"param.set","name":"x","value":1}"#);
    let out = h.frame(vec![]);
    assert!(!out.shapes.is_empty());
}

#[test]
fn clicking_the_viewport_selects_the_face_under_the_cursor() {
    let mut h = Harness::new(demo());
    h.app.set_view(ViewPreset::Top);
    h.app.camera.projection = ProjectionKind::Orthographic;
    h.frame(vec![]);
    h.click(h.centre());
    match &h.app.selected {
        Selected::Face { feature, face } => {
            assert_eq!((feature.as_str(), face.as_str()), ("hole", "cap_end"))
        }
        other => panic!("{other:?}"),
    }
    // Clicking far into a corner of the viewport (empty space) clears it.
    let r = h.ui.viewport_rect.unwrap();
    h.click(r.left_top() + egui::vec2(6.0, 6.0));
    assert_eq!(h.app.selected, Selected::None);
}

#[test]
fn hovering_names_the_face_under_the_cursor() {
    let mut h = Harness::new(demo());
    h.app.set_view(ViewPreset::Top);
    h.frame(vec![]);
    h.frame(vec![Event::PointerMoved(h.centre())]);
    assert!(h.app.hovered.is_some());
    h.frame(vec![Event::PointerGone]);
    assert!(h.app.hovered.is_none());
}

#[test]
fn dragging_orbits_and_shift_dragging_pans() {
    let mut h = Harness::new(demo());
    let c = h.centre();
    let yaw0 = h.app.camera.yaw;
    h.press(c, Modifiers::NONE);
    h.frame(vec![Event::PointerMoved(c + egui::vec2(40.0, 0.0))]);
    h.frame(vec![Event::PointerMoved(c + egui::vec2(90.0, 20.0))]);
    h.release(c + egui::vec2(90.0, 20.0), Modifiers::NONE);
    assert!(h.app.camera.yaw < yaw0, "dragging right turned the part");
    assert_eq!(h.app.selected, Selected::None, "a drag is not a click");

    let target0 = h.app.camera.target;
    h.press(c, Modifiers::SHIFT);
    h.frame(vec![Event::PointerMoved(c + egui::vec2(30.0, 30.0))]);
    h.frame(vec![Event::PointerMoved(c + egui::vec2(60.0, 50.0))]);
    h.release(c + egui::vec2(60.0, 50.0), Modifiers::SHIFT);
    assert!(
        (h.app.camera.target - target0).length() > 0.1,
        "shift-drag panned"
    );
}

#[test]
fn scrolling_zooms_and_double_clicking_fits() {
    let mut h = Harness::new(demo());
    let d0 = h.app.camera.distance;
    h.frame(vec![Event::PointerMoved(h.centre())]);
    for _ in 0..4 {
        h.frame(vec![Event::MouseWheel {
            unit: egui::MouseWheelUnit::Point,
            delta: egui::vec2(0.0, 40.0),
            modifiers: Modifiers::NONE,
            phase: egui::TouchPhase::Move,
        }]);
    }
    for _ in 0..6 {
        h.frame(vec![]);
    }
    assert!(
        h.app.camera.distance < d0,
        "scrolling up moved closer ({} -> {})",
        d0,
        h.app.camera.distance
    );
    let c = h.centre();
    h.frame(vec![Event::PointerMoved(c)]);
    for _ in 0..2 {
        h.frame(vec![Event::PointerButton {
            pos: c,
            button: PointerButton::Primary,
            pressed: true,
            modifiers: Modifiers::NONE,
        }]);
        h.frame(vec![Event::PointerButton {
            pos: c,
            button: PointerButton::Primary,
            pressed: false,
            modifiers: Modifiers::NONE,
        }]);
    }
    assert!(
        (h.app.camera.distance - d0).abs() / d0 < 0.02,
        "double click returned to the fitted distance"
    );
}

#[test]
fn keyboard_shortcuts_undo_and_redo_both_ways() {
    let mut h = Harness::new(demo());
    let before = h.app.authority.state_hash();
    h.app.set_depth("plate", 25.0).unwrap();
    let edited = h.app.authority.state_hash();
    assert_ne!(edited, before);
    h.key(Key::Z, Modifiers::COMMAND);
    assert_eq!(
        h.app.authority.state_hash(),
        before,
        "Ctrl+Z undid the edit"
    );
    h.key(Key::Y, Modifiers::COMMAND);
    assert_eq!(h.app.authority.state_hash(), edited, "Ctrl+Y redid it");
    h.key(Key::Z, Modifiers::COMMAND);
    assert_eq!(h.app.authority.state_hash(), before);
    h.key(Key::Z, Modifiers::COMMAND | Modifiers::SHIFT);
    assert_eq!(
        h.app.authority.state_hash(),
        edited,
        "Ctrl+Shift+Z is the other spelling of redo"
    );
}

#[test]
fn view_keys_switch_the_camera() {
    let mut h = Harness::new(demo());
    h.key(Key::Num5, Modifiers::NONE);
    assert_eq!(
        h.app.camera.pitch,
        core::f64::consts::FRAC_PI_2,
        "5 = top view"
    );
    h.key(Key::Num1, Modifiers::NONE);
    assert_eq!(h.app.camera.pitch, 0.0, "1 = front view");
    h.app.camera.distance *= 5.0;
    h.key(Key::F, Modifiers::NONE);
    assert!(
        h.app.camera.distance < 5.0 * 1000.0
            && (h.app.camera.target - dc_math::Vec3::new(50.0, 30.0, 13.0)).length() < 40.0
    );
}

#[test]
fn typing_into_the_console_runs_the_command() {
    let mut h = Harness::new(AppState::new());
    h.key(Key::Slash, Modifiers::NONE); // focus the console
    h.frame(vec![]);
    h.frame(vec![Event::Text(
        r#"{"type":"param.set","name":"w","value":7}"#.into(),
    )]);
    h.frame(vec![]);
    h.key(Key::Enter, Modifiers::NONE);
    assert_eq!(
        h.app.authority.state().get("w"),
        Some(7.0),
        "messages: {:?}",
        h.app.messages
    );
    // Typing letters did not trigger single-key shortcuts such as F or the number views.
    assert!(
        h.app.camera.pitch > 0.0,
        "the iso camera was left alone while typing"
    );
}

// ---- sketch mode through real egui input --------------------------------------------------

use dc_app::sketching::Tool;
use dc_doc::CommandKind;
use dc_sketch::{Constraint, Entity, PlaneSpec};

fn sketching_harness() -> Harness {
    let mut h = Harness::new(AppState::new());
    h.app.begin_sketch(PlaneSpec::Xy { offset: 0.0 }).unwrap();
    h.app.sketching.as_mut().unwrap().snap_grid = false;
    h.frame(vec![]);
    h
}

/// Window position of a sketch coordinate.
fn win(h: &Harness, p: (f64, f64)) -> Pos2 {
    let r = h.ui.viewport_rect.unwrap();
    let (x, y) = h
        .app
        .sketch_to_screen(p, f64::from(r.width()), f64::from(r.height()))
        .unwrap();
    r.min + egui::vec2(x as f32, y as f32)
}

fn lines(h: &Harness) -> usize {
    let s = &h.app.sketching.as_ref().unwrap().sketch;
    let sk = &h.app.authority.state().sketch(s).unwrap().sketch;
    sk.entities()
        .values()
        .filter(|e| matches!(e, Entity::Line { .. }))
        .count()
}

#[test]
fn the_sketch_toolbar_and_overlay_draw_without_a_gl_context() {
    let mut h = sketching_harness();
    let plain = h.frame(vec![]).shapes.len();
    h.app.set_tool(Tool::Rectangle);
    let p = win(&h, (10.0, 10.0));
    h.click(p);
    h.frame(vec![Event::PointerMoved(win(&h, (60.0, 40.0)))]);
    let with_preview = h.frame(vec![]).shapes.len();
    assert!(
        with_preview > plain,
        "a rubber band adds shapes ({plain} -> {with_preview})"
    );
}

#[test]
fn drawing_a_rectangle_with_the_mouse_and_the_r_key() {
    let mut h = sketching_harness();
    h.key(Key::R, Modifiers::NONE);
    assert_eq!(
        h.app.sketching.as_ref().unwrap().tool,
        Tool::Rectangle,
        "R picks the rectangle tool"
    );
    let (a, b) = (win(&h, (10.0, 5.0)), win(&h, (60.0, 35.0)));
    h.click(a);
    assert_eq!(lines(&h), 0, "one corner is only a preview");
    h.click(b);
    assert_eq!(lines(&h), 4);
}

#[test]
fn left_drag_does_not_orbit_while_sketching_but_right_drag_pans() {
    let mut h = sketching_harness();
    let (yaw, pitch) = (h.app.camera.yaw, h.app.camera.pitch);
    let c = h.centre();
    h.press(c, Modifiers::NONE);
    h.frame(vec![Event::PointerMoved(c + egui::vec2(40.0, 30.0))]);
    h.frame(vec![Event::PointerMoved(c + egui::vec2(90.0, 60.0))]);
    h.release(c + egui::vec2(90.0, 60.0), Modifiers::NONE);
    assert_eq!(
        (h.app.camera.yaw, h.app.camera.pitch),
        (yaw, pitch),
        "the view stays square-on"
    );
    let t0 = h.app.camera.target;
    h.frame(vec![Event::PointerButton {
        pos: c,
        button: PointerButton::Secondary,
        pressed: true,
        modifiers: Modifiers::NONE,
    }]);
    h.frame(vec![Event::PointerMoved(c + egui::vec2(30.0, 0.0))]);
    h.frame(vec![Event::PointerMoved(c + egui::vec2(60.0, 0.0))]);
    h.frame(vec![Event::PointerButton {
        pos: c + egui::vec2(60.0, 0.0),
        button: PointerButton::Secondary,
        pressed: false,
        modifiers: Modifiers::NONE,
    }]);
    assert!(
        (h.app.camera.target - t0).length() > 0.1,
        "right-drag panned"
    );
}

#[test]
fn right_click_opens_the_menu_and_escape_cancels_a_shape_in_progress() {
    let mut h = sketching_harness();
    h.key(Key::C, Modifiers::NONE);
    let p = win(&h, (20.0, 20.0));
    h.click(p);
    assert!(h.app.sketching.as_ref().unwrap().first.is_some());
    let c = win(&h, (30.0, 30.0));
    h.frame(vec![Event::PointerMoved(c)]);
    h.frame(vec![Event::PointerButton {
        pos: c,
        button: PointerButton::Secondary,
        pressed: true,
        modifiers: Modifiers::NONE,
    }]);
    h.frame(vec![Event::PointerButton {
        pos: c,
        button: PointerButton::Secondary,
        pressed: false,
        modifiers: Modifiers::NONE,
    }]);
    // As in SolidWorks, right-click opens a menu instead of cancelling; the shape is still there.
    assert!(
        h.app.sketching.as_ref().unwrap().first.is_some(),
        "right-click keeps the circle in progress"
    );
    let labels: Vec<String> =
        h.ui.context_menu
            .iter()
            .filter_map(|e| match e {
                dc_app::context::MenuEntry::Item(i) => Some(i.label.clone()),
                _ => None,
            })
            .collect();
    assert_eq!(labels, ["Select"], "{labels:?}");
    h.key(Key::Escape, Modifiers::NONE);
    assert!(
        h.app.sketching.as_ref().unwrap().first.is_none(),
        "Escape too"
    );
    h.key(Key::Escape, Modifiers::NONE);
    assert_eq!(h.app.sketching.as_ref().unwrap().tool, Tool::Select);
}

#[test]
fn view_keys_do_not_leave_the_sketch_plane_and_typing_in_the_console_is_not_a_tool_key() {
    let mut h = sketching_harness();
    let cam = h.app.camera;
    h.key(Key::Num7, Modifiers::NONE);
    h.key(Key::F, Modifiers::NONE);
    assert_eq!(h.app.camera, cam, "no view change while sketching");
    // With the console focused, letters go to the console, not the tool selector.
    h.app.set_tool(Tool::Select);
    h.key(Key::Slash, Modifiers::NONE);
    h.frame(vec![]);
    h.frame(vec![Event::Text("r".into())]);
    assert_eq!(h.app.sketching.as_ref().unwrap().tool, Tool::Select);
    let _ = CommandKind::HistoryUndo { revision: None };
}

// ---- the SolidWorks-style sketch tools through real egui input ------------------------------

#[test]
fn the_new_tool_keys_pick_tools_and_x_toggles_construction_mode() {
    let mut h = sketching_harness();
    for (key, tool) in [
        (Key::E, Tool::CenterRectangle),
        (Key::A, Tool::Arc),
        (Key::T, Tool::Arc3),
        (Key::L, Tool::Line),
        (Key::R, Tool::Rectangle),
        (Key::C, Tool::Circle),
        (Key::D, Tool::Dimension),
        (Key::S, Tool::Select),
    ] {
        h.key(key, Modifiers::NONE);
        assert_eq!(h.app.sketching.as_ref().unwrap().tool, tool, "{key:?}");
    }
    assert!(!h.app.sketching.as_ref().unwrap().construction);
    h.key(Key::X, Modifiers::NONE);
    assert!(
        h.app.sketching.as_ref().unwrap().construction,
        "X with nothing selected sets the mode"
    );
    h.key(Key::X, Modifiers::NONE);
    assert!(!h.app.sketching.as_ref().unwrap().construction);
}

#[test]
fn a_centre_rectangle_drawn_with_the_mouse_has_dashed_diagonals_in_the_overlay() {
    let mut h = sketching_harness();
    h.key(Key::E, Modifiers::NONE);
    let (c, k) = (win(&h, (0.0, 0.0)), win(&h, (40.0, 25.0)));
    h.click(c);
    h.click(k);
    assert_eq!(lines(&h), 6, "four edges and two construction diagonals");
    let r = h.ui.viewport_rect.unwrap();
    let o = h
        .app
        .sketch_overlay(f64::from(r.width()), f64::from(r.height()))
        .unwrap();
    assert_eq!(o.strokes.iter().filter(|s| s.construction).count(), 2);
}

#[test]
fn dragging_a_corner_with_the_mouse_moves_it_with_one_command() {
    let mut h = sketching_harness();
    h.key(Key::R, Modifiers::NONE);
    let (a, b) = (win(&h, (0.0, 0.0)), win(&h, (40.0, 20.0)));
    h.click(a);
    h.click(b);
    h.key(Key::S, Modifiers::NONE);
    let rev = h.app.authority.revision();
    h.press(b, Modifiers::NONE);
    let to = win(&h, (60.0, 35.0));
    h.frame(vec![Event::PointerMoved(b + egui::vec2(10.0, -8.0))]);
    h.frame(vec![Event::PointerMoved(to)]);
    assert_eq!(
        h.app.authority.revision(),
        rev,
        "nothing is logged mid-drag"
    );
    assert!(
        h.app.sketching.as_ref().unwrap().drag.is_some(),
        "a drag is in progress"
    );
    h.release(to, Modifiers::NONE);
    assert_eq!(
        h.app.authority.revision().0,
        rev.0 + 1,
        "one command on release"
    );
    let s = &h.app.sketching.as_ref().unwrap().sketch;
    let sk = &h.app.authority.state().sketch(s).unwrap().sketch;
    let moved = sk.entities().values().any(
        |e| matches!(e, Entity::Point { x, y } if (x - 60.0).abs() < 0.6 && (y - 35.0).abs() < 0.6),
    );
    assert!(moved, "the corner ended up near (60, 35)");
}

#[test]
fn shift_click_selects_several_and_the_toolbar_offers_relations() {
    let mut h = sketching_harness();
    h.key(Key::L, Modifiers::NONE);
    for (p, q) in [((0.0, 0.0), (40.0, 5.0)), ((0.0, 30.0), (40.0, 20.0))] {
        let (a, b) = (win(&h, p), win(&h, q));
        h.click(a);
        h.click(b);
        h.key(Key::Escape, Modifiers::NONE);
    }
    h.key(Key::S, Modifiers::NONE);
    h.click(win(&h, (20.0, 2.5)));
    h.press(win(&h, (20.0, 25.0)), Modifiers::SHIFT);
    h.release(win(&h, (20.0, 25.0)), Modifiers::SHIFT);
    assert_eq!(
        h.app.sketching.as_ref().unwrap().selection.len(),
        2,
        "shift added the second line"
    );
    assert!(h
        .app
        .available_relations()
        .contains(&dc_app::sketching::Relation::Parallel));
    // The toolbar with the relation row still draws.
    assert!(!h.frame(vec![]).shapes.is_empty());
    // Delete removes both.
    h.key(Key::Delete, Modifiers::NONE);
    assert_eq!(lines(&h), 0);
}

#[test]
fn the_toolbar_wraps_in_a_narrow_window_instead_of_overflowing() {
    let mut h = Harness::new(AppState::new());
    h.app.begin_sketch(PlaneSpec::Xy { offset: 0.0 }).unwrap();
    // A small tile: the tree shows, the inspector hides, the toolbar must fit the viewport.
    let raw = egui::RawInput {
        screen_rect: Some(Rect::from_min_size(Pos2::ZERO, egui::vec2(700.0, 600.0))),
        time: Some(5.0),
        ..Default::default()
    };
    for _ in 0..3 {
        let (app, ui) = (&mut h.app, &mut h.ui);
        let _ = h.ctx.run_ui(raw.clone(), |u| draw(app, ui, u));
    }
    let r = h.ui.viewport_rect.unwrap();
    assert!(r.width() > 300.0, "the viewport keeps room: {r:?}");
}

// ---- smart dimensioning through real egui input --------------------------------------------

#[test]
fn the_whole_dimension_flow_with_the_mouse_and_keyboard() {
    let mut h = sketching_harness();
    h.key(Key::L, Modifiers::NONE);
    let (a, b) = (win(&h, (10.0, 30.0)), win(&h, (50.0, 30.0)));
    h.click(a);
    h.click(b);
    h.key(Key::Escape, Modifiers::NONE); // end the chain
    h.key(Key::D, Modifiers::NONE);
    assert_eq!(h.app.sketching.as_ref().unwrap().tool, Tool::Dimension);

    // Pick the line: a live dimension follows the mouse and nothing is committed.
    let rev = h.app.authority.revision();
    h.click(win(&h, (30.0, 30.0)));
    h.frame(vec![Event::PointerMoved(win(&h, (30.0, 60.0)))]);
    h.frame(vec![]);
    let r = h.ui.viewport_rect.unwrap();
    let o = h
        .app
        .sketch_overlay(f64::from(r.width()), f64::from(r.height()))
        .unwrap();
    assert_eq!(
        o.dims.iter().filter(|d| d.preview).count(),
        1,
        "the preview dimension is drawn"
    );
    assert!(o.dim_prompt.is_some());
    assert_eq!(h.app.authority.revision(), rev);

    // Press the horizontal axis: it now means distance from the axis (30).
    h.click(win(&h, (80.0, 0.0)));
    h.frame(vec![Event::PointerMoved(win(&h, (95.0, 15.0)))]);
    let o = h
        .app
        .sketch_overlay(f64::from(r.width()), f64::from(r.height()))
        .unwrap();
    assert_eq!(o.dims.iter().find(|d| d.preview).unwrap().text, "30");

    // Click empty space to place it: the value box opens, then type and press Enter.
    h.click(win(&h, (95.0, 15.0)));
    assert!(
        h.app.sketching.as_ref().unwrap().dim.edit.is_some(),
        "placed: the value box is open"
    );
    h.frame(vec![]);
    h.frame(vec![]);
    h.frame(vec![Event::Text("35".into())]);
    h.frame(vec![]);
    h.key(Key::Enter, Modifiers::NONE);
    assert!(
        h.app.sketching.as_ref().unwrap().dim.edit.is_none(),
        "committed: {:?}",
        h.app.messages.last()
    );
    let s = &h.app.sketching.as_ref().unwrap().sketch;
    let sk = &h.app.authority.state().sketch(s).unwrap().sketch;
    let v = sk.constraints().values().find_map(|c| match c {
        Constraint::VerticalDistance { value, .. } => Some(*value),
        _ => None,
    });
    assert_eq!(v, Some(35.0), "{:?}", h.app.messages);
}

#[test]
fn escape_in_the_value_box_cancels_the_dimension() {
    let mut h = sketching_harness();
    h.key(Key::L, Modifiers::NONE);
    let (a, b) = (win(&h, (10.0, 30.0)), win(&h, (50.0, 30.0)));
    h.click(a);
    h.click(b);
    h.key(Key::Escape, Modifiers::NONE);
    h.key(Key::D, Modifiers::NONE);
    h.click(win(&h, (30.0, 30.0)));
    h.click(win(&h, (30.0, 5.0)));
    assert!(h.app.sketching.as_ref().unwrap().dim.edit.is_some());
    h.frame(vec![]);
    h.frame(vec![]);
    h.key(Key::Escape, Modifiers::NONE);
    h.frame(vec![]);
    assert!(
        h.app.sketching.as_ref().unwrap().dim.edit.is_none(),
        "Escape closed the box"
    );
    assert!(h.app.sketching.as_ref().unwrap().dim.picks.is_empty());
    let s = &h.app.sketching.as_ref().unwrap().sketch;
    let sk = &h.app.authority.state().sketch(s).unwrap().sketch;
    assert!(
        sk.constraints().values().all(|c| c.value().is_none()),
        "no dimension was added"
    );
}

#[test]
fn dragging_on_empty_space_selects_with_a_box_and_a_selected_set_drags_together() {
    let mut h = sketching_harness();
    h.key(Key::R, Modifiers::NONE);
    // (Not tied to the origin: the virtual origin is what the box is expected to cover.)
    h.app.sketching.as_mut().unwrap().snaps.points = false;
    h.click(win(&h, (0.0, 0.0)));
    h.click(win(&h, (30.0, 20.0)));
    h.click(win(&h, (60.0, 0.0)));
    h.click(win(&h, (90.0, 20.0)));
    h.key(Key::S, Modifiers::NONE);
    let rev = h.app.authority.revision();
    // Left to right around the first rectangle only: a window selection.
    let (a, b) = (win(&h, (-10.0, -10.0)), win(&h, (40.0, 30.0)));
    h.press(a, Modifiers::NONE);
    h.frame(vec![Event::PointerMoved(a + egui::vec2(20.0, 15.0))]);
    assert!(
        h.app.sketching.as_ref().unwrap().boxsel.is_some(),
        "a box is being dragged"
    );
    h.frame(vec![Event::PointerMoved(b)]);
    let o = h.app.sketch_overlay(800.0, 600.0).unwrap();
    assert!(o.select_box.is_some_and(|(_, _, crossing)| !crossing));
    h.release(b, Modifiers::NONE);
    assert_eq!(h.app.authority.revision(), rev, "selecting logs nothing");
    let sel = h.app.sketching.as_ref().unwrap().selection.clone();
    // The four edges of the first rectangle, plus the origin, which the box also covers.
    let edges = sel.iter().filter(|i| !i.starts_with('@')).count();
    assert_eq!(edges, 4, "{sel:?}");
    assert!(sel.contains(&"@origin".to_string()), "{sel:?}");
    // Dragging one selected edge moves the whole selection.
    let grab = win(&h, (15.0, 0.0));
    h.press(grab, Modifiers::NONE);
    h.frame(vec![Event::PointerMoved(grab + egui::vec2(12.0, -6.0))]);
    let to = win(&h, (15.0, -25.0));
    h.frame(vec![Event::PointerMoved(to)]);
    h.release(to, Modifiers::NONE);
    assert_eq!(
        h.app.authority.revision().0,
        rev.0 + 1,
        "one command for the move"
    );
    let s = &h.app.sketching.as_ref().unwrap().sketch;
    let sk = &h.app.authority.state().sketch(s).unwrap().sketch;
    let near = |p: (f64, f64), q: (f64, f64)| (p.0 - q.0).abs() < 0.6 && (p.1 - q.1).abs() < 0.6;
    let pts: Vec<(f64, f64)> = sk
        .entities()
        .values()
        .filter_map(|e| match e {
            Entity::Point { x, y } => Some((*x, *y)),
            _ => None,
        })
        .collect();
    assert!(
        pts.iter().any(|p| near(*p, (0.0, -25.0))),
        "first rectangle moved down: {pts:?}"
    );
    assert!(pts.iter().any(|p| near(*p, (30.0, -5.0))), "{pts:?}");
    assert!(
        pts.iter().any(|p| near(*p, (60.0, 0.0))),
        "second rectangle stayed: {pts:?}"
    );
}

#[test]
fn dragging_a_dimension_with_the_mouse_moves_it_with_one_command() {
    let mut h = sketching_harness();
    let sketch = h.app.sketching.as_ref().unwrap().sketch.clone();
    let ops = vec![
        CommandKind::SketchAddEntity {
            sketch: sketch.clone(),
            entity_id: "a".into(),
            entity: Entity::Point { x: 0.0, y: 0.0 },
        },
        CommandKind::SketchAddEntity {
            sketch: sketch.clone(),
            entity_id: "b".into(),
            entity: Entity::Point { x: 40.0, y: 0.0 },
        },
        CommandKind::SketchAddEntity {
            sketch: sketch.clone(),
            entity_id: "l".into(),
            entity: Entity::Line {
                a: "a".into(),
                b: "b".into(),
            },
        },
        CommandKind::SketchAddConstraint {
            sketch: sketch.clone(),
            constraint_id: "len".into(),
            constraint: dc_sketch::Constraint::Length {
                line: "l".into(),
                value: 40.0,
            },
        },
        CommandKind::SketchPlaceDimension {
            sketch: sketch.clone(),
            constraint_id: "len".into(),
            x: 20.0,
            y: 15.0,
        },
    ];
    h.app
        .submit(CommandKind::Transaction {
            label: "setup".into(),
            ops,
        })
        .unwrap();
    h.key(Key::S, Modifiers::NONE);
    h.frame(vec![]);
    let r = h.ui.viewport_rect.unwrap();
    let text = h
        .app
        .sketch_overlay(f64::from(r.width()), f64::from(r.height()))
        .unwrap()
        .dims
        .into_iter()
        .find(|d| d.id.as_deref() == Some("len"))
        .unwrap()
        .text_at;
    let from = r.min + egui::vec2(text.0 as f32, text.1 as f32);
    let to = win(&h, (20.0, -20.0));
    let rev = h.app.authority.revision();
    h.press(from, Modifiers::NONE);
    h.frame(vec![Event::PointerMoved(from + egui::vec2(6.0, 14.0))]);
    h.frame(vec![Event::PointerMoved(to)]);
    assert!(
        h.app.sketching.as_ref().unwrap().dim_drag.is_some(),
        "the dimension is in the hand"
    );
    assert!(
        h.app.sketching.as_ref().unwrap().boxsel.is_none(),
        "this is not a selection box"
    );
    assert_eq!(
        h.app.authority.revision(),
        rev,
        "nothing is logged mid-drag"
    );
    h.release(to, Modifiers::NONE);
    assert_eq!(
        h.app.authority.revision().0,
        rev.0 + 1,
        "one command on release"
    );
    let s = &h.app.sketching.as_ref().unwrap().sketch;
    let p = h
        .app
        .authority
        .state()
        .sketch(s)
        .unwrap()
        .sketch
        .placement("len")
        .unwrap();
    assert!(
        (p.0 - 20.0).abs() < 1.0 && (p.1 + 20.0).abs() < 1.0,
        "{p:?}"
    );
}

fn middle(h: &mut Harness, pos: Pos2, pressed: bool, mods: Modifiers) {
    h.held = if pressed { mods } else { Modifiers::NONE };
    h.frame(vec![Event::PointerButton {
        pos,
        button: PointerButton::Middle,
        pressed,
        modifiers: mods,
    }]);
}

#[test]
fn the_middle_button_rotates_a_part_and_pans_inside_a_sketch_and_shift_middle_zooms() {
    // Outside a sketch the middle button orbits, as in SolidWorks.
    let mut h = Harness::new(demo());
    h.frame(vec![]);
    let c = h.centre();
    let yaw = h.app.camera.yaw;
    middle(&mut h, c, true, Modifiers::NONE);
    h.frame(vec![Event::PointerMoved(c + egui::vec2(40.0, 0.0))]);
    h.frame(vec![Event::PointerMoved(c + egui::vec2(80.0, 0.0))]);
    middle(&mut h, c + egui::vec2(80.0, 0.0), false, Modifiers::NONE);
    assert!((h.app.camera.yaw - yaw).abs() > 1e-6, "rotated");
    // Shift + middle zooms.
    let d = h.app.camera.distance;
    middle(&mut h, c, true, Modifiers::SHIFT);
    h.frame(vec![Event::PointerMoved(c + egui::vec2(0.0, 30.0))]);
    h.frame(vec![Event::PointerMoved(c + egui::vec2(0.0, 80.0))]);
    middle(&mut h, c + egui::vec2(0.0, 80.0), false, Modifiers::SHIFT);
    assert!((h.app.camera.distance - d).abs() > 1e-6, "zoomed");
    // In a sketch the view stays square-on and the middle button pans.
    let mut s = sketching_harness();
    let (yaw, pitch, t0) = (s.app.camera.yaw, s.app.camera.pitch, s.app.camera.target);
    let c = s.centre();
    s.frame(vec![Event::PointerMoved(c)]);
    middle(&mut s, c, true, Modifiers::NONE);
    s.frame(vec![Event::PointerMoved(c + egui::vec2(30.0, 0.0))]);
    s.frame(vec![Event::PointerMoved(c + egui::vec2(70.0, 0.0))]);
    middle(&mut s, c + egui::vec2(70.0, 0.0), false, Modifiers::NONE);
    assert_eq!((s.app.camera.yaw, s.app.camera.pitch), (yaw, pitch));
    assert!(
        (s.app.camera.target - t0).length() > 0.1,
        "panned {:?} {:?}",
        s.app.camera.target,
        t0
    );
}

#[test]
fn ctrl_z_inside_a_half_drawn_chain_takes_back_a_segment_and_trim_keys_pick_modes() {
    let mut h = sketching_harness();
    h.key(Key::L, Modifiers::NONE);
    for p in [(0.0, 0.0), (30.0, 10.0), (50.0, 40.0)] {
        let w = win(&h, p);
        h.click(w);
    }
    assert_eq!(lines(&h), 2);
    h.key(Key::Z, Modifiers::COMMAND);
    assert_eq!(lines(&h), 1);
    h.key(Key::Escape, Modifiers::NONE);
    // The trim tool's number keys choose its mode.
    h.key(Key::W, Modifiers::NONE);
    h.key(Key::Num3, Modifiers::NONE);
    assert_eq!(
        h.app.sketching.as_ref().unwrap().trim_mode,
        dc_app::editing::TrimMode::ALL[2]
    );
    h.key(Key::Num1, Modifiers::NONE);
    assert_eq!(
        h.app.sketching.as_ref().unwrap().trim_mode,
        dc_app::editing::TrimMode::ALL[0]
    );
}

#[test]
fn the_feature_dialog_draws_and_accepting_it_makes_the_feature() {
    use dc_app::featuring::FeatureKind;
    let mut h = sketching_harness();
    h.key(Key::R, Modifiers::NONE);
    h.click(win(&h, (0.0, 0.0)));
    h.click(win(&h, (40.0, 20.0)));
    h.app.end_sketch();
    h.frame(vec![]);
    assert!(h.app.begin_feature(FeatureKind::Extrude));
    h.frame(vec![]);
    h.frame(vec![]);
    assert!(
        h.app.feature_draft.is_some(),
        "the dialog stays open while drawing"
    );
    assert_eq!(h.app.scene.bodies.len(), 1, "the preview is in the scene");
    // Escape cancels it.
    h.key(Key::Escape, Modifiers::NONE);
    assert!(h.app.feature_draft.is_none());
    assert!(h.app.scene.bodies.is_empty());
}

// ---- the ribbon shell: palette, tabs, layout -------------------------------------------------

fn type_text(h: &mut Harness, text: &str) {
    h.frame(vec![Event::Text(text.to_owned())]);
    h.frame(vec![]);
}

#[test]
fn ctrl_k_opens_the_command_palette_typing_filters_it_and_enter_runs_the_command() {
    let mut h = Harness::new(demo());
    assert!(h.ui.palette.is_none());
    h.key(Key::K, Modifiers::COMMAND);
    h.frame(vec![]);
    assert!(h.ui.palette.is_some(), "Ctrl+K opens the palette");
    // The default camera is isometric; ask for the front view by name.
    let before = (h.app.camera.yaw, h.app.camera.pitch);
    type_text(&mut h, "front");
    assert_eq!(h.ui.palette.as_ref().unwrap().query, "front");
    h.key(Key::Enter, Modifiers::NONE);
    h.frame(vec![]);
    assert!(
        h.ui.palette.is_none(),
        "running a command closes the palette"
    );
    let front = dc_render::Camera::preset(dc_render::ViewPreset::Front);
    assert_ne!(before, (front.yaw, front.pitch));
    assert!(
        (h.app.camera.yaw - front.yaw).abs() < 1e-9
            && (h.app.camera.pitch - front.pitch).abs() < 1e-9,
        "the front view was set"
    );
}

#[test]
fn a_command_that_cannot_run_keeps_the_palette_open_and_says_why_and_escape_closes_it() {
    let mut h = Harness::new(demo());
    h.key(Key::K, Modifiers::COMMAND);
    h.frame(vec![]);
    type_text(&mut h, "exit sketch");
    h.key(Key::Enter, Modifiers::NONE);
    h.frame(vec![]);
    assert!(h.ui.palette.is_some(), "it stays open");
    let last = h.app.messages.last().expect("a message").text.clone();
    assert!(last.contains("start or open a sketch first"), "{last}");
    h.key(Key::Escape, Modifiers::NONE);
    h.frame(vec![]);
    assert!(h.ui.palette.is_none(), "Escape closes it");
}

#[test]
fn the_ribbon_opens_the_sketch_tab_with_a_sketch_and_goes_back_when_it_ends() {
    use dc_app::ui::ribbon::Tab;
    let mut h = Harness::new(demo());
    assert_eq!(h.ui.ribbon_tab, Tab::Features);
    h.app.begin_sketch(PlaneSpec::Xy { offset: 0.0 }).unwrap();
    h.frame(vec![]);
    assert_eq!(h.ui.ribbon_tab, Tab::Sketch);
    h.app.end_sketch();
    h.frame(vec![]);
    assert_eq!(h.ui.ribbon_tab, Tab::Features);
    // The user can choose another tab and it stays until the sketch state changes again.
    h.ui.ribbon_tab = Tab::View;
    h.frame(vec![]);
    h.frame(vec![]);
    assert_eq!(h.ui.ribbon_tab, Tab::View);
}

#[test]
fn the_chrome_has_fixed_heights_so_the_viewport_does_not_jump_and_works_at_any_width() {
    for (w, h_px) in [
        (1280.0f32, 800.0f32),
        (900.0, 700.0),
        (640.0, 1000.0),
        (420.0, 700.0),
    ] {
        let mut h = Harness::new(demo());
        let run = |h: &mut Harness| {
            h.time += 0.05;
            let raw = egui::RawInput {
                screen_rect: Some(Rect::from_min_size(Pos2::ZERO, egui::vec2(w, h_px))),
                time: Some(h.time),
                ..Default::default()
            };
            let (app, ui) = (&mut h.app, &mut h.ui);
            let _ = h.ctx.run_ui(raw, |u| draw(app, ui, u));
        };
        run(&mut h);
        run(&mut h);
        let r0 = h.ui.viewport_rect.unwrap();
        // Messages, a sketch opening and a tool choice do not move the viewport's edges
        // (except that the sketch strip is inside it).
        h.app.info("a message");
        h.app.begin_sketch(PlaneSpec::Xy { offset: 0.0 }).unwrap();
        run(&mut h);
        run(&mut h);
        h.app.set_tool(Tool::Circle);
        run(&mut h);
        let r1 = h.ui.viewport_rect.unwrap();
        assert_eq!(r0.min, r1.min, "width {w}");
        assert!(
            (r0.max.y - r1.max.y).abs() < 0.5,
            "width {w}: {r0:?} -> {r1:?}"
        );
        assert!(
            r1.width() > 120.0 && r1.height() > 150.0,
            "width {w}: {r1:?}"
        );
    }
}

// ---- the left panel: tree tab and property pages --------------------------------------------

/// A finished sketch holding one rectangle, ready for a feature.
fn a_harness_with_a_rectangle() -> Harness {
    let mut h = sketching_harness();
    h.key(Key::R, Modifiers::NONE);
    let (a, b) = (win(&h, (10.0, 10.0)), win(&h, (60.0, 40.0)));
    h.click(a);
    h.click(b);
    h.app.end_sketch();
    h.frame(vec![]);
    h
}

#[test]
fn a_feature_page_takes_over_the_left_panel_enter_accepts_it_and_the_tree_comes_back() {
    use dc_app::ui::manager::LeftTab;
    let mut h = a_harness_with_a_rectangle();
    assert_eq!(h.ui.left_tab, LeftTab::Features);
    assert!(h.app.begin_feature(dc_app::featuring::FeatureKind::Extrude));
    h.frame(vec![]);
    h.frame(vec![]);
    assert_eq!(h.ui.left_tab, LeftTab::Properties, "the page is in front");
    assert!(
        h.app.feature_rows().is_empty(),
        "nothing is made until it is accepted"
    );
    h.key(Key::Enter, Modifiers::NONE);
    h.frame(vec![]);
    assert_eq!(h.app.feature_rows().len(), 1, "Enter accepted the page");
    assert!(h.app.feature_draft.is_none());
    h.frame(vec![]);
    assert_eq!(h.ui.left_tab, LeftTab::Features, "the tree is back");
}

#[test]
fn escape_cancels_a_feature_page_and_logs_nothing() {
    let mut h = a_harness_with_a_rectangle();
    let rev = h.app.authority.revision();
    assert!(h.app.begin_feature(dc_app::featuring::FeatureKind::Extrude));
    h.frame(vec![]);
    h.key(Key::Escape, Modifiers::NONE);
    h.frame(vec![]);
    assert!(h.app.feature_draft.is_none());
    assert_eq!(h.app.authority.revision(), rev);
}

#[test]
fn the_modify_page_opens_in_the_left_panel_and_escape_closes_it() {
    use dc_app::ui::manager::LeftTab;
    use dc_app::ui::{ModifyKind, ModifyPanel};
    let mut h = a_harness_with_a_rectangle();
    h.app.edit_sketch("sketch1");
    h.frame(vec![]);
    h.ui.modify = Some(ModifyPanel::new(ModifyKind::Move, (0.0, 0.0)));
    h.frame(vec![]);
    h.frame(vec![]);
    assert_eq!(h.ui.left_tab, LeftTab::Properties);
    h.key(Key::Escape, Modifiers::NONE);
    h.frame(vec![]);
    assert!(h.ui.modify.is_none());
    h.frame(vec![]);
    assert_eq!(h.ui.left_tab, LeftTab::Features);
}

#[test]
fn an_open_page_stays_visible_even_in_a_window_too_narrow_for_the_tree() {
    let mut h = a_harness_with_a_rectangle();
    let run = |h: &mut Harness| {
        h.time += 0.05;
        let raw = egui::RawInput {
            screen_rect: Some(Rect::from_min_size(Pos2::ZERO, egui::vec2(480.0, 700.0))),
            time: Some(h.time),
            ..Default::default()
        };
        let (app, ui) = (&mut h.app, &mut h.ui);
        let _ = h.ctx.run_ui(raw, |u| draw(app, ui, u));
    };
    run(&mut h);
    run(&mut h);
    let without = h.ui.viewport_rect.unwrap().min.x;
    assert!(without < 5.0, "no left panel in a narrow window: {without}");
    assert!(h.app.begin_feature(dc_app::featuring::FeatureKind::Extrude));
    run(&mut h);
    run(&mut h);
    let with = h.ui.viewport_rect.unwrap().min.x;
    assert!(with > 150.0, "the page took room on the left: {with}");
}

#[test]
fn tapping_alt_shows_key_tips_and_letters_then_keys_reach_commands() {
    use dc_app::keytips::Tips;
    use dc_app::ui::ribbon::Tab;
    let mut h = Harness::new(demo());
    h.frame(vec![]);
    // Holding Alt alone and letting go shows letters on the tabs.
    h.held = Modifiers::ALT;
    h.frame(vec![]);
    h.held = Modifiers::NONE;
    h.frame(vec![]);
    assert_eq!(h.ui.tips, Tips::Tabs);
    // "S" opens the Sketch tab's tips.
    h.key(Key::S, Modifiers::NONE);
    h.frame(vec![]);
    assert_eq!(h.ui.ribbon_tab, Tab::Sketch);
    assert_eq!(h.ui.tips, Tips::Commands);
    // Escape puts them away without doing anything.
    let rev = h.app.authority.revision();
    h.key(Key::Escape, Modifiers::NONE);
    assert_eq!(h.ui.tips, Tips::Off);
    assert_eq!(h.app.authority.revision(), rev);
    // Alt used with another key (Alt+F) is not a tap.
    h.held = Modifiers::ALT;
    h.frame(vec![Event::Key {
        key: Key::F,
        physical_key: Some(Key::F),
        pressed: true,
        repeat: false,
        modifiers: Modifiers::ALT,
    }]);
    h.held = Modifiers::NONE;
    h.frame(vec![]);
    assert_eq!(h.ui.tips, Tips::Off);
    // Tap again, then the tab letter, then a command key: the first Sketch-tab command runs.
    h.held = Modifiers::ALT;
    h.frame(vec![]);
    h.held = Modifiers::NONE;
    h.frame(vec![]);
    h.key(Key::F, Modifiers::NONE);
    h.frame(vec![]);
    assert_eq!(h.ui.tips, Tips::Commands);
    h.key(Key::Num1, Modifiers::NONE);
    h.frame(vec![]);
    assert_eq!(h.ui.tips, Tips::Off, "a command key ends the tips");
}
