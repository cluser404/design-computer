//! The egui drawing and input handling. All document logic lives in [`AppState`]; this file only
//! turns widgets and gestures into calls on it.

use crate::context::{MenuAction, MenuEntry};
use crate::dimensioning::DimDraw;
use crate::editing::TrimMode;
use crate::featuring::FeatureKind;
use crate::gl::{Frame, Geometry, GlShared, ViewState};
use crate::modify::ModifyOp;
use crate::relations::{Glyph, GlyphKind};
use crate::sketching::{is_datum, AxisMark, FreedomMark, Marker, Problem, Relation, Tool};
use crate::state::{AppState, Level, Selected};
use dc_doc::{Query, QueryResult};
use dc_render::ViewPreset;
use dc_sketch::PlaneSpec;
use eframe::egui;
use eframe::egui::{
    Color32, DragValue, Key, Modifiers, Pos2, Rect, RichText, Sense, TextureHandle, TextureOptions,
};
use std::sync::Arc;
use std::time::Duration;

pub mod icon_font;
pub mod icons;
pub mod manager;
pub mod ribbon;
pub mod shell;
pub mod theme;

pub use icons::{CadIcon, Icon};
pub use theme::{Colors, ThemeMode};

#[derive(Clone, Copy, PartialEq, Eq)]
enum DialogKind {
    Open,
    SaveAs,
}

struct Dialog {
    kind: DialogKind,
    text: String,
}

/// UI-only state: nothing here is part of the document.
#[derive(Default)]
pub struct UiState {
    texture: Option<TextureHandle>,
    /// What the texture currently shows: (view version, pixel size, supersample).
    drawn: Option<(u64, [u32; 2], u32)>,
    interacting_until: f64,
    pub viewport_rect: Option<Rect>,
    dialog: Option<Dialog>,
    console: String,
    focus_console: bool,
    /// The dock at the bottom shows the activity (who did what) instead of the console.
    pub show_activity: bool,
    /// Which mouse buttons rotate, pan and zoom the view.
    pub nav: crate::navigation::NavPreset,
    /// Alt key tips: which level they are at.
    pub tips: crate::keytips::Tips,
    /// A key pressed while tips are showing, waiting for the ribbon to use it.
    pub tip_key: Option<String>,
    alt_down: bool,
    alt_used: bool,
    /// Last window title sent.
    title: String,
    /// The ribbon tab shown, and whether the ribbon is folded away.
    pub ribbon_tab: ribbon::Tab,
    pub ribbon_hidden: bool,
    /// Was a sketch open last frame (to switch tabs when one opens or closes)?
    last_sketching: bool,
    /// The command palette, while it is open.
    pub palette: Option<shell::Palette>,
    /// Keep the classic menu bar under the title bar.
    pub menu_pinned: bool,
    /// A command asked to close the window.
    want_quit: bool,
    /// Which tab of the left panel is in front.
    pub left_tab: manager::LeftTab,
    /// The text of the tree's filter box.
    pub tree_filter: String,
    /// The plane or origin row last clicked in the tree (they are not document selections).
    tree_pick: Option<String>,
    /// A tree row being renamed: (is feature, id, text so far).
    renaming: Option<(bool, String, String)>,
    /// Was a page (a command being set up) open last frame?
    page_was_active: bool,
    /// The dark theme is chosen (the light one is the default).
    pub dark: bool,
    /// The theme last applied to the window.
    theme_applied: Option<bool>,
    /// Show the sheet of all icons instead of the window (`--icon-sheet`).
    pub icon_sheet: bool,
    /// Present when the window has a real OpenGL context; the viewport is then drawn on the GPU.
    gl: Option<Arc<GlShared>>,
    geom: Option<Arc<Geometry>>,
    /// Which dimension target the number box last grabbed keyboard focus for.
    dim_focus: Option<String>,
    /// What the sketch's right-click menu offers (worked out when the button went down).
    pub context_menu: Vec<MenuEntry>,
    /// Pretend the pointer is here (sketch coordinates), for screenshots of hover previews.
    pub hover_override: Option<(f64, f64)>,
    /// The Modify panel, when it is open.
    pub modify: Option<ModifyPanel>,
    /// Where a Ctrl-drag selection box started (screen points), while one is being drawn.
    box_start: Option<Pos2>,
    /// The modifier keys that came with the last pointer button event (a press or a release):
    /// a drag or a click is judged by what was held when the button went down, even when the
    /// platform's own modifier state is not reported (scripted input).
    pointer_mods: egui::Modifiers,
}

impl crate::commands::Host for UiState {
    fn has_gpu(&self) -> bool {
        self.uses_gl()
    }

    fn ask_file(&mut self, save: bool, current: &str) {
        self.dialog = Some(Dialog {
            kind: if save {
                DialogKind::SaveAs
            } else {
                DialogKind::Open
            },
            text: current.to_owned(),
        });
    }

    fn next_nav(&mut self) -> &'static str {
        self.nav = self.nav.next();
        self.nav.name()
    }

    fn open_modify(&mut self, kind: ModifyKind, centre: (f64, f64)) {
        self.modify = Some(ModifyPanel::new(kind, centre));
    }

    fn open_palette(&mut self) {
        self.palette = Some(shell::Palette {
            fresh: true,
            ..Default::default()
        });
    }

    fn quit(&mut self) {
        self.want_quit = true;
    }

    fn dark_theme(&self) -> bool {
        self.dark
    }

    fn set_dark_theme(&mut self, dark: bool) {
        self.dark = dark;
    }
}

impl UiState {
    /// Opens the command palette (Ctrl+K).
    pub fn open_palette(&mut self) {
        crate::commands::Host::open_palette(self);
    }

    /// Starts timing GPU frames (`--bench`). `false` if the viewport is not on the GPU.
    pub fn start_bench(&self) -> bool {
        self.gl.as_ref().map(|g| g.start_bench()).is_some()
    }

    pub fn bench_frames(&self) -> usize {
        self.gl.as_ref().map_or(0, |g| g.bench_frames())
    }

    pub fn bench_report(&self) -> String {
        self.gl.as_ref().map_or_else(
            || "the viewport is not on the GPU".into(),
            |g| g.bench_report(),
        )
    }

    /// Is the viewport still preparing an environment in the background?
    pub fn environment_pending(&self) -> bool {
        self.gl.as_ref().is_some_and(|g| g.environment_pending())
    }

    /// A UI that draws the viewport with OpenGL (for a real window).
    pub fn with_gl() -> Self {
        UiState {
            gl: Some(Arc::new(GlShared::default())),
            ..Default::default()
        }
    }

    pub fn uses_gl(&self) -> bool {
        self.gl.as_ref().is_some_and(|g| !g.failed())
    }

    pub fn has_texture(&self) -> bool {
        self.texture.is_some()
    }
}

fn short(h: dc_doc::Hash128) -> String {
    h.to_hex()[..8].to_owned()
}

/// Below this window width (in points) the left panel is hidden so the viewport keeps
/// room; widen the window or tile it larger to get them back.
const SHOW_TREE_FROM: f32 = 560.0;

/// Draws the whole window into `ui` (the root of the viewport) for one frame.
pub fn draw(app: &mut AppState, st: &mut UiState, ui: &mut egui::Ui) {
    let ctx = ui.ctx().clone();
    theme::ensure(&ctx);
    if st.theme_applied != Some(st.dark) {
        theme::apply(
            &ctx,
            if st.dark {
                ThemeMode::Dark
            } else {
                ThemeMode::Light
            },
        );
        st.theme_applied = Some(st.dark);
    }
    shortcuts(app, st, &ctx);
    app.refresh_scene();

    let title = format!(
        "{}{} — Design Computer",
        app.path
            .as_ref()
            .and_then(|p| p.file_name())
            .map_or("untitled".into(), |n| n.to_string_lossy().into_owned()),
        if app.dirty() { " •" } else { "" }
    );
    if title != st.title {
        ctx.send_viewport_cmd(egui::ViewportCommand::Title(title.clone()));
        st.title = title;
    }

    // The ribbon follows what you are doing: a sketch opens the Sketch tab, closing it goes back.
    let sketching_now = app.sketching.is_some();
    if sketching_now != st.last_sketching {
        st.last_sketching = sketching_now;
        st.ribbon_tab = if sketching_now {
            ribbon::Tab::Sketch
        } else {
            ribbon::Tab::Features
        };
        st.ribbon_hidden = false;
    }
    let colors = Colors::get(&ctx);
    egui::Panel::top("titlebar")
        .exact_size(34.0)
        .frame(
            egui::Frame::new()
                .fill(colors.chrome)
                .inner_margin(egui::Margin::symmetric(0, 4)),
        )
        .show(ui, |ui| shell::titlebar(app, st, ui));
    if st.menu_pinned {
        egui::Panel::top("menu").show(ui, |ui| menu(app, st, ui));
    }
    egui::Panel::top("tabs")
        .exact_size(26.0)
        .frame(egui::Frame::new().fill(colors.chrome))
        .show(ui, |ui| shell::tab_strip(app, st, ui));
    if !st.ribbon_hidden {
        egui::Panel::top("ribbon")
            .exact_size(ribbon::COMMAND_ROW_HEIGHT + 6.0)
            .frame(
                egui::Frame::new()
                    .fill(colors.ribbon)
                    .inner_margin(egui::Margin::symmetric(0, 3)),
            )
            .show(ui, |ui| shell::ribbon_row(app, st, ui));
    }
    egui::Panel::bottom("status")
        .exact_size(26.0)
        .frame(
            egui::Frame::new()
                .fill(colors.chrome)
                .inner_margin(egui::Margin::symmetric(0, 3)),
        )
        .show(ui, |ui| shell::status_bar(app, ui));
    // A fixed height: the viewport above it must not move when messages arrive.
    egui::Panel::bottom("console")
        .exact_size(132.0)
        .show(ui, |ui| console(app, st, ui));
    // The window may be any size (a tiling window manager hands out whatever tile fits), so the
    // left panel scales with it and steps aside when the viewport would get cramped (unless a
    // command is being set up: its page must be visible).
    let w = ui.available_width();
    let page_open = manager::page_active(app, st);
    if w >= SHOW_TREE_FROM || page_open {
        egui::Panel::left("manager")
            .resizable(true)
            .default_size((w * 0.22).clamp(250.0, 340.0))
            .size_range(200.0..=(w * if page_open { 0.6 } else { 0.4 }).max(200.0))
            .frame(
                egui::Frame::new()
                    .fill(colors.ribbon)
                    .inner_margin(egui::Margin::same(6)),
            )
            .show(ui, |ui| manager::left_panel(app, st, ui));
    }
    if st.icon_sheet {
        egui::CentralPanel::default().show(ui, shell::icon_sheet);
        return;
    }
    egui::CentralPanel::default()
        .frame(egui::Frame::NONE)
        .show(ui, |ui| viewport(app, st, ui));
    dialogs(app, st, &ctx);
    shell::palette(app, st, &ctx);
    if std::mem::take(&mut st.want_quit) {
        ctx.send_viewport_cmd(egui::ViewportCommand::Close);
    }
}

/// Alt key tips: a tap of Alt (pressed and released with nothing else in between) shows them,
/// Escape or Alt again hides them, and while they show the keys belong to them.
fn key_tips(st: &mut UiState, ctx: &egui::Context) -> bool {
    use crate::keytips::Tips;
    let (alt, other_key, esc, typed) = ctx.input(|i| {
        let other = i.events.iter().any(
            |e| matches!(e, egui::Event::Key { pressed: true, key, .. } if *key != Key::Escape),
        );
        let esc = i.key_pressed(Key::Escape);
        let typed: Option<String> = i.events.iter().find_map(|e| match e {
            egui::Event::Key {
                pressed: true,
                key,
                modifiers,
                ..
            } if !modifiers.alt && !modifiers.command => Some(key.name().to_uppercase()),
            _ => None,
        });
        (i.modifiers.alt, other, esc, typed)
    });
    if alt && !st.alt_down {
        st.alt_used = false;
    }
    if alt && other_key {
        st.alt_used = true;
    }
    if !alt && st.alt_down && !st.alt_used && !ctx.egui_wants_keyboard_input() {
        st.tips = if st.tips == Tips::Off {
            Tips::Tabs
        } else {
            Tips::Off
        };
        st.tip_key = None;
    }
    st.alt_down = alt;
    if st.tips == Tips::Off {
        return false;
    }
    if esc {
        st.tips = Tips::Off;
        return true;
    }
    if !alt {
        if let Some(k) = typed {
            st.tip_key = Some(k);
        }
    }
    true
}

fn shortcuts(app: &mut AppState, st: &mut UiState, ctx: &egui::Context) {
    if key_tips(st, ctx) {
        return;
    }
    if ctx.egui_wants_keyboard_input() {
        return;
    }
    // An open Modify page takes Escape first (the sketch would otherwise use it to cancel a tool).
    if st.modify.is_some() && ctx.input(|i| i.key_pressed(Key::Escape)) {
        st.modify = None;
        if let Some(s) = &mut app.sketching {
            s.modify_preview = None;
        }
        return;
    }
    // The file and search shortcuts are commands of the registry (their names, tooltips and
    // keys are listed there), so a menu, a button and a key all do the same thing.
    let file_keys = ctx.input_mut(|i| {
        let shift = Modifiers::COMMAND | Modifiers::SHIFT;
        if i.consume_key(Modifiers::COMMAND, Key::K) {
            Some("app.palette")
        } else if i.consume_key(shift, Key::S) {
            Some("file.save_as")
        } else if i.consume_key(Modifiers::COMMAND, Key::S) {
            Some("file.save")
        } else if i.consume_key(Modifiers::COMMAND, Key::O) {
            Some("file.open")
        } else if i.consume_key(Modifiers::COMMAND, Key::N) {
            Some("file.new")
        } else {
            None
        }
    });
    let view_key = ctx.input_mut(|i| {
        [
            (Key::Num1, "view.front"),
            (Key::Num2, "view.back"),
            (Key::Num3, "view.left"),
            (Key::Num4, "view.right"),
            (Key::Num5, "view.top"),
            (Key::Num6, "view.bottom"),
            (Key::Num7, "view.iso"),
        ]
        .into_iter()
        .find(|(k, _)| i.consume_key(Modifiers::COMMAND, *k))
        .map(|(_, id)| id)
    });
    if let Some(id) = view_key {
        let _ = crate::commands::run(id, app, st);
    }
    if let Some(id) = file_keys {
        // Save with no file yet means Save as.
        let id = if id == "file.save" && app.path.is_none() {
            "file.save_as"
        } else {
            id
        };
        if let Err(e) = crate::commands::run(id, app, st) {
            app.error(e, None);
        }
    }
    let trim_keys = app.sketching.as_ref().is_some_and(|s| s.tool == Tool::Trim);
    let (undo, redo, fit, del, views) = ctx.input_mut(|i| {
        let redo = i.consume_key(Modifiers::COMMAND | Modifiers::SHIFT, Key::Z)
            || i.consume_key(Modifiers::COMMAND, Key::Y);
        let undo = i.consume_key(Modifiers::COMMAND, Key::Z);
        let fit = i.consume_key(Modifiers::NONE, Key::F);
        let del = i.consume_key(Modifiers::NONE, Key::Delete);
        let mut views = None;
        for (k, v) in [
            (Key::Num1, ViewPreset::Front),
            (Key::Num2, ViewPreset::Back),
            (Key::Num3, ViewPreset::Left),
            (Key::Num4, ViewPreset::Right),
            (Key::Num5, ViewPreset::Top),
            (Key::Num6, ViewPreset::Bottom),
            (Key::Num7, ViewPreset::Iso),
        ] {
            if !trim_keys && i.consume_key(Modifiers::NONE, k) {
                views = Some(v);
            }
        }
        (undo, redo, fit, del, views)
    });
    if undo {
        // Half way through a shape, Ctrl+Z takes back that, not the last finished thing.
        if !app.sketch_undo_in_progress() {
            let _ = app.undo();
        }
    }
    if redo {
        let _ = app.redo();
    }
    if app.sketching.is_some() {
        // In sketch mode the view stays square-on to the plane; keys pick tools instead.
        if del {
            app.sketch_delete_selected();
        }
        let tool = ctx.input_mut(|i| {
            [
                (Key::S, Tool::Select),
                (Key::L, Tool::Line),
                (Key::R, Tool::Rectangle),
                (Key::E, Tool::CenterRectangle),
                (Key::C, Tool::Circle),
                (Key::A, Tool::Arc),
                (Key::T, Tool::Arc3),
                (Key::D, Tool::Dimension),
                (Key::W, Tool::Trim),
                (Key::N, Tool::Extend),
                (Key::P, Tool::Split),
            ]
            .into_iter()
            .find(|(k, _)| i.consume_key(Modifiers::NONE, *k))
            .map(|(_, t)| t)
        });
        if let Some(t) = tool {
            app.set_tool(t);
        }
        // In the Trim tool the number keys pick its mode (the view cannot change while sketching).
        if app.sketching.as_ref().is_some_and(|s| s.tool == Tool::Trim) {
            for (k, m) in [Key::Num1, Key::Num2, Key::Num3]
                .into_iter()
                .zip(TrimMode::ALL)
            {
                if ctx.input_mut(|i| i.consume_key(Modifiers::NONE, k)) {
                    app.set_trim_mode(m);
                }
            }
        }
        if ctx.input_mut(|i| i.consume_key(Modifiers::NONE, Key::X)) {
            app.toggle_construction();
        }
        if ctx.input_mut(|i| i.consume_key(Modifiers::COMMAND, Key::A)) {
            app.set_tool(Tool::Select);
            app.sketch_select_all();
        }
        if ctx.input_mut(|i| i.consume_key(Modifiers::NONE, Key::Escape)) {
            app.sketch_cancel();
        }
        return;
    }
    if fit {
        app.fit();
    }
    if del {
        let _ = app.delete_selected();
    }
    if let Some(v) = views {
        app.set_view(v);
    }
    if ctx.input(|i| i.key_pressed(Key::Slash)) {
        st.focus_console = true;
    }
}

/// The classic menu bar (kept for people who like it, pinned from the title bar's menu).
fn menu(app: &mut AppState, st: &mut UiState, ui: &mut egui::Ui) {
    egui::MenuBar::new().ui(ui, |ui| {
        file_menu(app, st, ui);
        edit_menu(app, ui);
        ui.menu_button("Sketch", |ui| sketch_menu(app, ui));
        feature_menu(app, ui);
        view_menu(app, st, ui);
    });
}

fn file_menu(app: &mut AppState, st: &mut UiState, ui: &mut egui::Ui) {
    ui.menu_button("File", |ui| {
        if ui.button("New").clicked() {
            *app = AppState::new();
            ui.close();
        }
        if ui.button("Open…").clicked() {
            st.dialog = Some(Dialog {
                kind: DialogKind::Open,
                text: app
                    .path
                    .as_ref()
                    .map(|p| p.display().to_string())
                    .unwrap_or_default(),
            });
            ui.close();
        }
        let can_save = app.path.is_some();
        if ui
            .add_enabled(can_save, egui::Button::new("Save"))
            .clicked()
        {
            if let Some(p) = app.path.clone() {
                if let Err(e) = app.save(&p) {
                    app.error(e, None);
                }
            }
            ui.close();
        }
        if ui.button("Save as…").clicked() {
            st.dialog = Some(Dialog {
                kind: DialogKind::SaveAs,
                text: app
                    .path
                    .as_ref()
                    .map(|p| p.display().to_string())
                    .unwrap_or_else(|| "part.dcad".into()),
            });
            ui.close();
        }
        ui.separator();
        if ui
            .button("Load showcase part")
            .on_hover_text("A part made for judging the viewport: curved and flat faces, steps, a thin rib, deep holes and pockets")
            .clicked()
        {
            *app = AppState::new();
            app.load_showcase();
            ui.close();
        }
        if ui.button("Load small demo part").clicked() {
            *app = AppState::new();
            app.load_demo();
            ui.close();
        }
        ui.separator();
        if ui.button("Quit").clicked() {
            ui.ctx().send_viewport_cmd(egui::ViewportCommand::Close);
        }
    });
}

fn edit_menu(app: &mut AppState, ui: &mut egui::Ui) {
    ui.menu_button("Edit", |ui| {
        if ui
            .add(egui::Button::new("Undo").shortcut_text("Ctrl+Z"))
            .clicked()
        {
            let _ = app.undo();
            ui.close();
        }
        if ui
            .add(egui::Button::new("Redo").shortcut_text("Ctrl+Y"))
            .clicked()
        {
            let _ = app.redo();
            ui.close();
        }
        ui.separator();
        if ui
            .add(egui::Button::new("Delete selection").shortcut_text("Del"))
            .clicked()
        {
            let _ = app.delete_selected();
            ui.close();
        }
    });
}

fn feature_menu(app: &mut AppState, ui: &mut egui::Ui) {
    ui.menu_button("Feature", |ui| {
        for k in FeatureKind::ALL {
            if ui
                .button(format!("{}…", k.name()))
                .on_hover_text(k.hint())
                .clicked()
            {
                app.begin_feature(k);
                ui.close();
            }
        }
    });
}

fn view_menu(app: &mut AppState, st: &mut UiState, ui: &mut egui::Ui) {
    ui.menu_button("View", |ui| {
        if ui
            .add(egui::Button::new("Fit to part").shortcut_text("F"))
            .clicked()
        {
            app.fit();
            ui.close();
        }
        for (label, v, key) in [
            ("Front", ViewPreset::Front, "1"),
            ("Back", ViewPreset::Back, "2"),
            ("Left", ViewPreset::Left, "3"),
            ("Right", ViewPreset::Right, "4"),
            ("Top", ViewPreset::Top, "5"),
            ("Bottom", ViewPreset::Bottom, "6"),
            ("Isometric", ViewPreset::Iso, "7"),
        ] {
            if ui
                .add(egui::Button::new(label).shortcut_text(key))
                .clicked()
            {
                app.set_view(v);
                ui.close();
            }
        }
        ui.separator();
        let perspective = app.camera.projection == dc_render::ProjectionKind::Perspective;
        if ui
            .selectable_label(perspective, "Perspective")
            .on_hover_text("Off (the default) draws without foreshortening, as SolidWorks does")
            .clicked()
        {
            app.toggle_projection();
        }
        let mut changed = false;
        // As in SolidWorks: the model origin is shown (View > Origins); the grid belongs to
        // the plane being sketched on and is off until switched on.
        changed |= ui.checkbox(&mut app.show_origins, "Origins").changed();
        changed |= ui
            .checkbox(&mut app.show_sketch_grid, "Sketch grid")
            .changed();
        changed |= ui
            .checkbox(&mut app.ambient_occlusion, "Ambient occlusion")
            .on_hover_text("Darkens corners and pockets, as SolidWorks does (GPU view only)")
            .changed();
        changed |= ui
            .checkbox(&mut app.fxaa, "Extra smoothing (FXAA)")
            .on_hover_text("A final smoothing pass; softens the image a little (GPU view only)")
            .changed();
        ui.menu_button("RealView environment", |ui| {
            for e in crate::environment::Environment::ALL {
                if ui
                    .selectable_label(app.environment == e, e.name())
                    .clicked()
                {
                    app.environment = e;
                    changed = true;
                    ui.close();
                }
            }
        });
        changed |= ui
            .checkbox(&mut app.floor_reflection, "Floor reflection")
            .on_hover_text("A faint reflection of the part in the ground (RealView, GPU view only)")
            .changed();
        changed |= ui.checkbox(&mut app.show_edges, "Edges").changed();
        changed |= ui.checkbox(&mut app.show_sketches, "Sketches").changed();
        ui.separator();
        let gpu = st.uses_gl();
        let r = ui
            .add_enabled(
                gpu,
                egui::Checkbox::new(&mut app.realview, "RealView (reflections, shadows)"),
            )
            .on_disabled_hover_text("needs the OpenGL viewport");
        changed |= r.changed();
        ui.add_enabled_ui(gpu && app.realview, |ui| {
            ui.menu_button("Material", |ui| {
                for m in crate::state::Material::ALL {
                    if ui.radio_value(&mut app.material, m, m.name()).clicked() {
                        changed = true;
                        ui.close();
                    }
                }
            });
        });
        if changed {
            app.view_version += 1;
        }
    });
}

fn sketch_menu(app: &mut AppState, ui: &mut egui::Ui) {
    if app.sketching.is_some() {
        if ui.button("Finish sketch").clicked() {
            app.end_sketch();
            ui.close();
        }
        if ui
            .add(egui::Button::new("Select all").shortcut_text("Ctrl+A"))
            .clicked()
        {
            app.set_tool(Tool::Select);
            app.sketch_select_all();
            ui.close();
        }
        ui.separator();
    }
    for (label, plane) in [
        (
            "New sketch on XY plane (top)",
            PlaneSpec::Xy { offset: 0.0 },
        ),
        (
            "New sketch on XZ plane (front)",
            PlaneSpec::Xz { offset: 0.0 },
        ),
        (
            "New sketch on YZ plane (right)",
            PlaneSpec::Yz { offset: 0.0 },
        ),
    ] {
        if ui.button(label).clicked() {
            app.begin_sketch(plane);
            ui.close();
        }
    }
    let face = match &app.selected {
        Selected::Face { feature, face } => Some((feature.clone(), face.clone())),
        _ => None,
    };
    if ui
        .add_enabled(
            face.is_some(),
            egui::Button::new("New sketch on selected face"),
        )
        .on_disabled_hover_text("click a flat face in the view first")
        .clicked()
    {
        if let Some((feature, face)) = face {
            app.begin_sketch(PlaneSpec::Face {
                feature,
                face,
                offset: 0.0,
            });
        }
        ui.close();
    }
    if let Selected::Sketch(id) = app.selected.clone() {
        if app.sketching.as_ref().is_none_or(|s| s.sketch != id)
            && ui.button(format!("Edit sketch '{id}'")).clicked()
        {
            app.edit_sketch(&id);
            ui.close();
        }
    }
}

fn console(app: &mut AppState, st: &mut UiState, ui: &mut egui::Ui) {
    ui.horizontal(|ui| {
        let a = ui.selectable_value(&mut st.show_activity, false, "Console");
        crate::devtools::tag(
            "dock.console",
            eguidev::WidgetRole::Selectable,
            "Console",
            None,
            &a,
        );
        let b = ui
            .selectable_value(&mut st.show_activity, true, "Activity")
            .on_hover_text("Every committed change, with who made it (you, a script or an agent)");
        crate::devtools::tag(
            "dock.activity",
            eguidev::WidgetRole::Selectable,
            "Activity",
            None,
            &b,
        );
    });
    if st.show_activity {
        egui::ScrollArea::vertical()
            .id_salt("activity")
            .max_height(ui.available_height())
            .auto_shrink([false, false])
            .show(ui, |ui| {
                for (rev, who, what) in app.activity(200) {
                    ui.horizontal(|ui| {
                        ui.weak(format!("#{rev}"));
                        ui.label(RichText::new(who).strong());
                        ui.label(what);
                    });
                }
            });
        return;
    }
    egui::ScrollArea::vertical()
        .stick_to_bottom(true)
        .max_height(ui.available_height() - 34.0)
        .auto_shrink([false, false])
        .show(ui, |ui| {
            for m in app.messages.iter().rev().take(40).rev() {
                match m.level {
                    Level::Info => {
                        ui.weak(&m.text);
                    }
                    Level::Error => {
                        ui.colored_label(Color32::from_rgb(210, 70, 60), &m.text);
                        if let Some(h) = &m.hint {
                            ui.label(RichText::new(format!("   → {h}")).weak());
                        }
                    }
                }
            }
        });
    ui.horizontal(|ui| {
        ui.label("›");
        let r = ui.add(
            egui::TextEdit::singleline(&mut st.console)
                .desired_width(f32::INFINITY)
                .hint_text("command (JSON), undo, redo, fit    — the same commands an agent sends"),
        );
        if st.focus_console {
            r.request_focus();
            st.focus_console = false;
        }
        if r.lost_focus() && ui.input(|i| i.key_pressed(Key::Enter)) {
            let line = std::mem::take(&mut st.console);
            app.run_console(&line);
            r.request_focus();
        }
    });
}

fn plane_text(p: &PlaneSpec) -> String {
    match p {
        PlaneSpec::Xy { offset } => format!("XY plane{}", off(*offset)),
        PlaneSpec::Xz { offset } => format!("XZ plane{}", off(*offset)),
        PlaneSpec::Yz { offset } => format!("YZ plane{}", off(*offset)),
        PlaneSpec::Face {
            feature,
            face,
            offset,
        } => format!("face {face} of {feature}{}", off(*offset)),
    }
}

fn off(o: f64) -> String {
    if o == 0.0 {
        String::new()
    } else {
        format!(" + {o} mm")
    }
}

fn feature_panel(app: &mut AppState, ui: &mut egui::Ui, id: &str) {
    let Some(row) = app.feature_rows().into_iter().find(|r| r.id == id) else {
        return;
    };
    ui.label(RichText::new(&row.id).strong().size(16.0));
    let rounds = row.kind == "fillet" || row.kind == "chamfer";
    if row.sketch.is_empty() {
        ui.label(row.kind.clone());
    } else {
        ui.label(format!("{} from sketch '{}'", row.kind, row.sketch));
    }
    if let Some(t) = &row.target {
        ui.label(format!("built on '{t}'"));
    }
    ui.separator();
    if let Some(d) = row.depth {
        let mut v = d;
        ui.horizontal(|ui| {
            ui.label(match row.kind.as_str() {
                "fillet" => "Radius",
                "chamfer" => "Distance",
                _ => "Depth",
            });
            let r = ui.add(
                DragValue::new(&mut v)
                    .speed(0.2)
                    .range(0.001..=100_000.0)
                    .suffix(" mm"),
            );
            if (r.drag_stopped() || r.lost_focus()) && (v - d).abs() > 1e-12 {
                let _ = app.set_depth(id, v);
            }
        });
    } else if !rounds {
        ui.label("Depth: through all");
    }
    if let Ok(QueryResult::FeatureSummary { info }) = app
        .authority
        .query(&Query::FeatureSummary { feature: id.into() })
    {
        let s = &info.summary;
        ui.separator();
        egui::Grid::new("measures").num_columns(2).show(ui, |ui| {
            ui.label("Volume");
            ui.label(format!("{:.3} mm³", s.volume));
            ui.end_row();
            ui.label("Surface area");
            ui.label(format!("{:.3} mm²", s.area));
            ui.end_row();
            ui.label("Size");
            ui.label(format!(
                "{:.2} × {:.2} × {:.2} mm",
                s.bounds_max[0] - s.bounds_min[0],
                s.bounds_max[1] - s.bounds_min[1],
                s.bounds_max[2] - s.bounds_min[2]
            ));
            ui.end_row();
            ui.label("Faces / edges");
            ui.label(format!("{} / {}", s.faces, s.edges));
            ui.end_row();
        });
    }
    ui.separator();
    if ui.button("Delete feature").clicked() {
        let _ = app.delete_selected();
    }
}

fn sketch_panel(app: &mut AppState, ui: &mut egui::Ui, id: &str) {
    let Some(row) = app.sketch_rows().into_iter().find(|r| r.id == id) else {
        return;
    };
    ui.label(RichText::new(&row.id).strong().size(16.0));
    ui.label(format!("on {}", plane_text(&row.info.plane)));
    ui.horizontal_wrapped(|ui| {
        for k in FeatureKind::ALL {
            if ui
                .button(format!("{}…", k.name()))
                .on_hover_text(k.hint())
                .clicked()
            {
                app.selected = Selected::Sketch(id.to_owned());
                app.begin_feature(k);
            }
        }
    });
    if app.sketching.as_ref().is_none_or(|s| s.sketch != id) && ui.button("Edit sketch").clicked() {
        app.edit_sketch(id);
    }
    ui.label(format!(
        "{} entities, {} constraints",
        row.info.entities, row.info.constraints
    ));
    if row.info.entities == 0 {
        ui.label(RichText::new("empty: nothing drawn yet").weak());
    } else if row.info.fully_constrained {
        ui.colored_label(Color32::from_rgb(40, 150, 70), "fully constrained");
    } else {
        ui.colored_label(
            Color32::from_rgb(200, 120, 0),
            format!("{} degrees of freedom left", row.info.dof),
        );
    }
    let redundant: Vec<String> = app
        .authority
        .state()
        .sketch(id)
        .map(|s| s.report.redundant.clone())
        .unwrap_or_default();
    let clashing: Vec<String> = app
        .sketching
        .as_ref()
        .filter(|s| s.sketch == id)
        .map(|s| s.conflict_relations.clone())
        .unwrap_or_default();
    ui.separator();
    ui.label("Dimensions");
    for (cid, c) in app.constraints_of(id) {
        if let Some(v0) = c.value() {
            let mut v = v0;
            ui.horizontal(|ui| {
                ui.label(&cid);
                if clashing.contains(&cid) {
                    ui.label(RichText::new("clashes").color(SKETCH_CONFLICT))
                        .on_hover_text("The last change was refused because it contradicts this dimension.");
                }
                if redundant.contains(&cid) {
                    ui.label(RichText::new("redundant").color(SKETCH_OVER))
                        .on_hover_text("Other relations already fix this, so it adds nothing (the sketch is over defined). Remove it or one of the others.");
                }
                let is_angle = matches!(c, dc_sketch::Constraint::Angle { .. });
                let r = ui.add(
                    DragValue::new(&mut v)
                        .speed(if is_angle { 0.5 } else { 0.2 })
                        .range(if is_angle {
                            0.001..=179.999
                        } else {
                            0.001..=100_000.0
                        })
                        .suffix(if is_angle { " °" } else { " mm" }),
                );
                if (r.drag_stopped() || r.lost_focus()) && (v - v0).abs() > 1e-12 {
                    let _ = app.set_dimension(id, &cid, v);
                }
            });
        }
    }
    let relations: Vec<(String, dc_sketch::Constraint)> = app
        .constraints_of(id)
        .into_iter()
        .filter(|(_, c)| c.value().is_none())
        .collect();
    if !relations.is_empty() {
        ui.separator();
        ui.label(format!("Relations ({})", relations.len()));
        let mut remove = None;
        for (cid, c) in &relations {
            ui.horizontal(|ui| {
                ui.label(RichText::new(crate::relations::relation_label(c)).small());
                if clashing.contains(cid) {
                    ui.label(RichText::new("clashes").small().color(SKETCH_CONFLICT))
                        .on_hover_text("The last change was refused because it contradicts this relation.");
                }
                if redundant.contains(cid) {
                    ui.label(RichText::new("redundant").small().color(SKETCH_OVER))
                        .on_hover_text("Other relations already fix this, so it adds nothing (the sketch is over defined). Remove it or one of the others.");
                }
                if ui
                    .small_button("remove")
                    .on_hover_text(format!("remove relation {cid}"))
                    .clicked()
                {
                    remove = Some(cid.clone());
                }
            });
        }
        if let Some(cid) = remove {
            app.remove_constraint(id, &cid);
        }
    }
    ui.separator();
    if ui.button("Delete sketch").clicked() {
        let _ = app.delete_selected();
    }
}

fn edge_panel(app: &mut AppState, ui: &mut egui::Ui, feature: &str, edge: &str) {
    ui.label(RichText::new(edge).strong().size(16.0));
    ui.label(format!("edge of '{feature}', where its two faces meet"));
    ui.separator();
    for (kind, text) in [
        (FeatureKind::Fillet, "Fillet this edge"),
        (FeatureKind::Chamfer, "Chamfer this edge"),
    ] {
        if ui.button(text).clicked() {
            app.begin_rounding_with_edge(kind, feature, edge);
        }
    }
}

fn sketch_entity_panel(app: &mut AppState, ui: &mut egui::Ui, sketch: &str, entity: &str) {
    ui.label(RichText::new(entity).strong().size(16.0));
    ui.label(format!(
        "{} of sketch '{sketch}'",
        app.entity_kind(sketch, entity)
    ));
    ui.separator();
    if ui.button(format!("Edit sketch '{sketch}'")).clicked() {
        app.edit_sketch(sketch);
    }
}

fn face_panel(app: &mut AppState, ui: &mut egui::Ui, feature: &str, face: &str) {
    ui.label(RichText::new(face).strong().size(16.0));
    ui.label(format!("face of '{feature}'"));
    if let Some(f) = app.faces_of(feature).into_iter().find(|f| f.name == face) {
        ui.separator();
        egui::Grid::new("face").num_columns(2).show(ui, |ui| {
            ui.label("Surface");
            ui.label(&f.surface);
            ui.end_row();
            ui.label("Area");
            ui.label(format!("{:.3} mm²", f.area));
            ui.end_row();
            if let Some(n) = f.normal {
                ui.label("Normal");
                ui.label(format!("({:.2}, {:.2}, {:.2})", n[0], n[1], n[2]));
                ui.end_row();
            }
            if let Some(r) = f.radius {
                ui.label("Radius");
                ui.label(format!("{r:.3} mm"));
                ui.end_row();
            }
        });
        if f.surface == "plane" {
            ui.separator();
            if ui.button("New sketch on this face").clicked() {
                app.begin_sketch(PlaneSpec::Face {
                    feature: feature.into(),
                    face: face.into(),
                    offset: 0.0,
                });
            }
        }
    }
}

fn viewport(app: &mut AppState, st: &mut UiState, ui: &mut egui::Ui) {
    let ctx = ui.ctx().clone();
    let avail = ui.available_size();
    let (rect, resp) = ui.allocate_exact_size(avail, Sense::click_and_drag());
    st.viewport_rect = Some(rect);
    crate::devtools::tag(
        "viewport",
        eguidev::WidgetRole::Unknown,
        "3D view",
        None,
        &resp,
    );
    app.set_viewport_aspect(f64::from(rect.width()) / f64::from(rect.height().max(1.0)));
    let ppp = ctx.pixels_per_point();
    let (pw, ph) = (
        ((rect.width() * ppp).round() as u32).clamp(16, 2400),
        ((rect.height() * ppp).round() as u32).clamp(16, 1800),
    );
    let now = ctx.input(|i| i.time);

    let sketching = app.sketching.is_some();
    ctx.input(|i| {
        for e in &i.events {
            if let egui::Event::PointerButton { modifiers, .. } = e {
                st.pointer_mods = *modifiers;
            }
        }
    });
    let shift = ctx.input(|i| i.modifiers.shift) || st.pointer_mods.shift;
    let drag = resp.drag_delta();
    // Left drag orbits (not while sketching: the view stays square-on to the plane), right or
    // middle drag pans, scroll zooms.
    // SolidWorks' controls: middle button rotates, Ctrl + middle pans, Shift + middle zooms. A
    // sketch cannot rotate (it stays square-on), so there the middle button pans.
    let middle = resp.dragged_by(egui::PointerButton::Middle);
    let ctrl = ctx.input(|i| i.modifiers.ctrl) || st.pointer_mods.ctrl;
    use crate::navigation::NavAction;
    let mid = middle.then(|| st.nav.middle_drag(ctrl, shift));
    // A sketch cannot rotate (it stays square-on), so there rotating pans instead.
    let mid = match mid {
        Some(NavAction::Rotate) if sketching => Some(NavAction::Pan),
        m => m,
    };
    if mid == Some(NavAction::Zoom) {
        // Dragging up zooms in, like the wheel rolled forwards.
        app.zoom_scroll(f64::from(-drag.y * ppp));
        st.interacting_until = now + 0.25;
    } else if resp.dragged_by(egui::PointerButton::Primary) && ctrl && !sketching {
        // Ctrl + drag draws a selection box (handled below), it does not turn the view.
    } else if resp.dragged_by(egui::PointerButton::Primary) && !shift && !sketching
        || mid == Some(NavAction::Rotate)
    {
        app.orbit_px(f64::from(drag.x * ppp), f64::from(drag.y * ppp));
        st.interacting_until = now + 0.25;
    } else if resp.dragged_by(egui::PointerButton::Secondary)
        || mid == Some(NavAction::Pan)
        || (resp.dragged_by(egui::PointerButton::Primary) && shift)
    {
        app.pan_px(
            f64::from(drag.x * ppp),
            f64::from(drag.y * ppp),
            f64::from(ph),
        );
        st.interacting_until = now + 0.25;
    }
    if resp.hovered() {
        let scroll = ctx.input(|i| i.smooth_scroll_delta.y);
        if scroll != 0.0 {
            app.zoom_scroll(f64::from(scroll));
            st.interacting_until = now + 0.25;
        }
    }
    let to_px = |p: Pos2| {
        (
            (f64::from(p.x - rect.min.x)) * f64::from(ppp),
            (f64::from(p.y - rect.min.y)) * f64::from(ppp),
        )
    };
    if sketching {
        // Sketch coordinates are worked out in points (what the overlay is drawn in).
        let (w, h) = (f64::from(rect.width()), f64::from(rect.height()));
        let local = |p: Pos2| (f64::from(p.x - rect.min.x), f64::from(p.y - rect.min.y));
        app.hover_at(None);
        let additive = ctx.input(|i| i.modifiers.shift || i.modifiers.command);
        if resp.drag_started_by(egui::PointerButton::Primary) {
            if let Some(p) = ctx.input(|i| i.pointer.press_origin()) {
                let (x, y) = local(p);
                // Pressing on something grabs it; pressing on empty space starts a selection box.
                if !app.sketch_drag_begin(x, y, w, h)
                    && !app.dimension_drag_begin(x, y, w, h)
                    && !app.trim_drag_begin(x, y)
                    && !app.drag_draw_begin(x, y, w, h)
                {
                    app.sketch_box_begin(x, y);
                }
            }
        } else if resp.dragged_by(egui::PointerButton::Primary) {
            if let Some(p) = resp.interact_pointer_pos() {
                let (x, y) = local(p);
                // Holding Ctrl moves freely, without snapping to the grid.
                let free = ctx.input(|i| i.modifiers.ctrl);
                app.sketch_drag_move_with(x, y, w, h, free);
                app.sketch_box_move(x, y);
                app.trim_drag_move(x, y, w, h);
                app.dimension_drag_move(x, y, w, h);
            }
            // The feature check is rate limited; keep redrawing until it has caught up even if
            // the pointer holds still.
            if app.drag_check_due() {
                ctx.request_repaint_after(std::time::Duration::from_millis(60));
            }
            if app
                .sketching
                .as_ref()
                .is_some_and(|s| s.drag.is_some() || s.dim_drag.is_some())
            {
                ctx.set_cursor_icon(egui::CursorIcon::Grabbing);
            }
        }
        if resp.drag_stopped() {
            app.sketch_drag_end();
            app.dimension_drag_end();
            if let Some(p) = resp.interact_pointer_pos().or_else(|| resp.hover_pos()) {
                let (x, y) = local(p);
                app.drag_draw_end(x, y, w, h);
            }
            app.sketch_box_end(additive, w, h);
            app.trim_drag_end();
        }
        if resp.double_clicked() {
            if let Some(p) = resp.interact_pointer_pos() {
                let (x, y) = local(p);
                app.sketch_double_click(x, y, w, h);
            }
        }
        if resp.secondary_clicked() {
            // Right-click opens SolidWorks' context menu (it no longer cancels: Esc does that).
            if let Some(p) = resp.interact_pointer_pos() {
                let (x, y) = local(p);
                st.context_menu = app.sketch_context_open(x, y, w, h);
            }
        } else if resp.clicked() {
            if let Some(p) = resp.interact_pointer_pos() {
                let (x, y) = local(p);
                app.sketch_click_with(x, y, w, h, additive);
            }
        }
        let mut chosen: Option<MenuAction> = None;
        resp.context_menu(|ui| {
            for entry in &st.context_menu {
                match entry {
                    MenuEntry::Separator => {
                        ui.separator();
                    }
                    MenuEntry::Item(it) => {
                        let mut text = it.label.clone();
                        if it.checked == Some(true) {
                            text = format!("✓ {text}");
                        }
                        let mut b = egui::Button::new(text);
                        if !it.shortcut.is_empty() {
                            b = b.shortcut_text(it.shortcut);
                        }
                        if ui.add(b).clicked() {
                            chosen = Some(it.action.clone());
                            ui.close();
                        }
                    }
                }
            }
        });
        if let Some(a) = chosen {
            app.sketch_context_do(&a);
        }
        // While a shape is being drawn by dragging, the pointer still steers its preview.
        let steering = app.sketching.as_ref().is_some_and(|s| s.drag_drawing);
        let at = (!resp.dragged() || steering)
            .then(|| resp.hover_pos().or_else(|| resp.interact_pointer_pos()))
            .flatten()
            .map(|p| {
                let (x, y) = local(p);
                (x, y, w, h)
            });
        let at = match st.hover_override {
            Some(p) => app
                .sketch_to_screen(p, w, h)
                .map(|(x, y)| (x, y, w, h))
                .or(at),
            None => at,
        };
        app.sketch_hover(at);
        // Say what a press here would do: grab something, or pick a relation.
        if let Some(s) = app.sketching.as_ref().filter(|s| s.drag.is_none()) {
            let grabbable = s.hover.as_deref().is_some_and(|h| !is_datum(h));
            if matches!(s.tool, Tool::Trim | Tool::Extend | Tool::Split) {
                ctx.set_cursor_icon(egui::CursorIcon::Crosshair);
            } else if grabbable {
                ctx.set_cursor_icon(egui::CursorIcon::Grab);
            } else if let Some(r) = &s.hover_relation {
                // A dimension can be picked up and moved; a relation symbol only clicked.
                let is_dimension = app
                    .sketch_data()
                    .and_then(|sk| sk.constraints().get(r))
                    .is_some_and(|c| c.value().is_some());
                ctx.set_cursor_icon(if is_dimension {
                    egui::CursorIcon::Grab
                } else {
                    egui::CursorIcon::PointingHand
                });
            }
        }
    } else {
        if resp.double_clicked() {
            app.fit();
        } else if resp.clicked() {
            if let Some(p) = resp.interact_pointer_pos() {
                let (x, y) = to_px(p);
                app.click_view(x, y, f64::from(pw), f64::from(ph), ctrl);
            }
        }
        // Ctrl + drag: a selection box. To the right it takes what is wholly inside, to the
        // left what it touches; Shift as well adds to the selection.
        if resp.drag_started_by(egui::PointerButton::Primary) && ctrl {
            st.box_start = ctx.input(|i| i.pointer.press_origin());
        }
        if let Some(start) = st.box_start {
            let end = resp
                .interact_pointer_pos()
                .or_else(|| ctx.input(|i| i.pointer.latest_pos()));
            if let Some(end) = end {
                let crossing = end.x < start.x;
                if resp.dragged() {
                    let band = Rect::from_two_pos(start, end);
                    let col = if crossing {
                        Color32::from_rgb(40, 170, 90)
                    } else {
                        Color32::from_rgb(40, 120, 230)
                    };
                    let painter = ui.painter().with_clip_rect(rect);
                    painter.rect_filled(band, 0.0, col.gamma_multiply(0.12));
                    painter.rect_stroke(
                        band,
                        0.0,
                        egui::Stroke::new(1.5, col),
                        egui::StrokeKind::Middle,
                    );
                }
                if resp.drag_stopped() {
                    let (x0, y0) = to_px(start);
                    let (x1, y1) = to_px(end);
                    if (x1 - x0).abs() > 3.0 || (y1 - y0).abs() > 3.0 {
                        app.box_select(
                            (x0, y0, x1, y1),
                            crossing,
                            shift,
                            f64::from(pw),
                            f64::from(ph),
                        );
                    }
                    st.box_start = None;
                }
            }
            if !resp.dragged() && !resp.drag_stopped() {
                st.box_start = None;
            }
        }
        if resp.dragged() {
            app.hover_at(None);
        } else {
            let at = resp.hover_pos().map(|p| {
                let (x, y) = to_px(p);
                (x, y, f64::from(pw), f64::from(ph))
            });
            app.hover_at(at);
        }
    }

    if let Some(shared) = st.gl.clone() {
        if let Some(e) = shared.take_error() {
            app.error(
                format!("GL viewport unavailable ({e}); using the software renderer"),
                None,
            );
        }
        if !shared.failed() {
            let geom = match &st.geom {
                Some(g) if g.version == app.scene_version => g.clone(),
                _ => {
                    let g = Arc::new(Geometry::from_scene(&app.scene, app.scene_version));
                    st.geom = Some(g.clone());
                    g
                }
            };
            let extent = app
                .scene
                .bounds()
                .map_or(0.0, |(lo, hi)| (hi - lo).length());
            {
                let wake = ctx.clone();
                shared.set_repaint(move || wake.request_repaint());
            }
            let view = ViewState {
                aspect: f64::from(rect.width()) / f64::from(rect.height().max(1.0)),
                extent,
                grid: false,
                origins: app.show_origins,
                pixels_per_point: ctx.pixels_per_point(),
                viewport_px_height: rect.height() * ctx.pixels_per_point(),
                ambient_occlusion: app.ambient_occlusion,
                fxaa: app.fxaa,
                floor_reflection: app.floor_reflection,
                environment: app.environment,
                local_grid: app.local_grid(),
                edges: app.show_edges,
                sketches: app.show_sketches,
                selected: app.selected_face_ref(),
                hovered: app.hovered.clone(),
                realview: app.realview.then_some(app.material),
                bounds: app.scene.body_bounds(),
            };
            let frame = Arc::new(Frame::new(geom, &app.camera, &view));
            // The fallback behind the GL image (and the whole background without GL): the same
            // backdrop as a grid of coloured vertices.
            let mut mesh = egui::Mesh::default();
            let (nx, ny) = (32usize, 24usize);
            for j in 0..=ny {
                for i in 0..=nx {
                    let (fu, fv) = (i as f32 / nx as f32, j as f32 / ny as f32);
                    let [r, g, b] =
                        crate::backdrop::background_at(fu, fv).map(|c| (c * 255.0).round() as u8);
                    mesh.colored_vertex(
                        egui::pos2(
                            rect.left() + rect.width() * fu,
                            rect.top() + rect.height() * fv,
                        ),
                        Color32::from_rgb(r, g, b),
                    );
                }
            }
            for j in 0..ny {
                for i in 0..nx {
                    let a = (j * (nx + 1) + i) as u32;
                    let b = a + 1;
                    let c = a + (nx + 1) as u32;
                    mesh.add_triangle(a, b, c + 1);
                    mesh.add_triangle(a, c + 1, c);
                }
            }
            ui.painter().add(egui::Shape::mesh(mesh));
            let cb = eframe::egui_glow::CallbackFn::new(move |info, painter| {
                let v = info.viewport_in_pixels();
                let vp = [v.left_px, v.from_bottom_px, v.width_px, v.height_px];
                shared.paint(painter.gl(), vp, painter.intermediate_fbo(), &frame);
            });
            ui.painter().add(egui::PaintCallback {
                rect,
                callback: Arc::new(cb),
            });
        }
    }
    if !st.uses_gl() {
        software_viewport(app, st, ui, rect, (pw, ph), resp.dragged(), now);
    }

    if sketching {
        draw_sketch_overlay(app, ui, rect);
        sketch_toolbar(app, &ctx, rect);
        dimension_box(app, st, &ctx, rect);
    }

    draw_highlights(app, ui, rect);
    context_toolbar(app, st, ui, rect);
    // The heads-up view toolbar and the confirmation corner float over the view.
    shell::hud(app, st, &ctx, rect);
    shell::confirmation_corner(app, &ctx, rect);
    egui::Area::new(egui::Id::new("view_hint"))
        .fixed_pos(rect.left_top() + egui::vec2(10.0, 8.0))
        .show(&ctx, |ui| {
            if let Some(h) = &app.hovered {
                ui.label(
                    RichText::new(format!("{} › {}", h.body, h.face))
                        .background_color(Color32::from_rgba_unmultiplied(255, 255, 255, 200))
                        .color(Color32::from_gray(40)),
                );
            }
        });
}

/// Edges and sketch lines that are hovered, selected or chosen, drawn over the view.
fn draw_highlights(app: &AppState, ui: &egui::Ui, rect: Rect) {
    use crate::highlight::{highlights, HighlightKind};
    let (w, h) = (f64::from(rect.width()), f64::from(rect.height()));
    let painter = ui.painter().with_clip_rect(rect);
    for hl in highlights(app) {
        let (color, width) = match hl.kind {
            HighlightKind::Hover => (Color32::from_rgb(40, 150, 255), 3.0),
            HighlightKind::Selected => (Color32::from_rgb(255, 140, 0), 3.5),
            HighlightKind::Chosen => (Color32::from_rgb(255, 140, 0), 4.0),
            HighlightKind::Axis => (Color32::from_rgb(200, 40, 200), 3.5),
        };
        for [a, b] in &hl.segments {
            let (Some(pa), Some(pb)) = (app.camera.project(*a, w, h), app.camera.project(*b, w, h))
            else {
                continue;
            };
            painter.line_segment(
                [
                    rect.min + egui::vec2(pa.x as f32, pa.y as f32),
                    rect.min + egui::vec2(pb.x as f32, pb.y as f32),
                ],
                egui::Stroke::new(width, color),
            );
        }
    }
}

/// The small toolbar at the bottom of the view with what can be done to the selection.
fn context_toolbar(app: &mut AppState, st: &mut UiState, ui: &mut egui::Ui, rect: Rect) {
    use crate::contextbar::{actions, ContextAction};
    let list = actions(app);
    if list.is_empty() {
        return;
    }
    let colors = theme::Colors::get(ui.ctx());
    // Laid out by hand (not an egui Area) so it never needs a sizing pass: appearing must not
    // cost a frame, or a double click on the view that selects something would be missed.
    let font = egui::FontId::proportional(13.0);
    let pad = 10.0;
    let cells: Vec<_> = list
        .iter()
        .map(|a| {
            ui.painter()
                .layout_no_wrap(a.label().to_owned(), font.clone(), colors.text)
        })
        .collect();
    let total: f32 = cells.iter().map(|g| g.size().x + 2.0 * pad).sum();
    let bar = Rect::from_center_size(
        Pos2::new(rect.center().x, rect.bottom() - 30.0),
        egui::vec2(total + 8.0, 30.0),
    );
    ui.painter().rect(
        bar,
        6.0,
        colors.chrome,
        egui::Stroke::new(1.0, colors.hover_line),
        egui::StrokeKind::Inside,
    );
    let mut chosen = None;
    let mut x = bar.left() + 4.0;
    for (i, (a, g)) in list.iter().zip(cells).enumerate() {
        let w = g.size().x + 2.0 * pad;
        let cell = Rect::from_min_size(Pos2::new(x, bar.top() + 3.0), egui::vec2(w, 24.0));
        let resp = ui.interact(cell, egui::Id::new(("context_action", i)), Sense::click());
        crate::devtools::tag(
            &format!("context.{}", a.label().to_lowercase().replace(' ', "_")),
            eguidev::WidgetRole::Button,
            a.label(),
            None,
            &resp,
        );
        if resp.hovered() {
            ui.painter().rect_filled(cell, 4.0, colors.hover);
        }
        ui.painter().galley(
            Pos2::new(cell.left() + pad, cell.center().y - g.size().y / 2.0),
            g,
            colors.text,
        );
        if resp.clicked() {
            chosen = Some(a.clone());
        }
        x += w;
    }
    match chosen {
        Some(ContextAction::SketchOnFace) => {
            let _ = crate::commands::run("sketch.on_face", app, st).map_err(|e| app.error(e, None));
        }
        Some(ContextAction::EditFeature(f)) => {
            app.begin_edit_feature(&f);
        }
        Some(ContextAction::Suppress(f)) => {
            app.set_suppressed(&f, true);
        }
        Some(ContextAction::Unsuppress(f)) => {
            app.set_suppressed(&f, false);
        }
        Some(ContextAction::EditSketch(s)) => {
            app.edit_sketch(&s);
        }
        Some(ContextAction::Rename(is_feature, id)) => {
            let kind = if is_feature { "feature" } else { "sketch" };
            let l = app.authority.state().label(kind, &id).to_owned();
            st.renaming = Some((is_feature, id, l));
        }
        Some(ContextAction::Delete) => {
            let _ = app.delete_selected();
        }
        None => {}
    }
}

fn dialogs(app: &mut AppState, st: &mut UiState, ctx: &egui::Context) {
    let Some(mut d) = st.dialog.take() else {
        return;
    };
    let mut keep = true;
    let mut go = false;
    egui::Window::new(match d.kind {
        DialogKind::Open => "Open document",
        DialogKind::SaveAs => "Save document as",
    })
    .collapsible(false)
    .resizable(false)
    .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
    .show(ctx, |ui| {
        ui.label("Path to a .dcad file:");
        let r = ui.add(egui::TextEdit::singleline(&mut d.text).desired_width(420.0));
        if r.lost_focus() && ui.input(|i| i.key_pressed(Key::Enter)) {
            go = true;
        }
        let dir = crate::filebrowser::folder_of(&d.text);
        ui.horizontal(|ui| {
            if ui.button("Up").on_hover_text("The parent folder").clicked() {
                d.text = crate::filebrowser::up(&dir);
            }
            ui.weak(dir.display().to_string());
        });
        egui::ScrollArea::vertical()
            .id_salt("file_list")
            .max_height(180.0)
            .show(ui, |ui| {
                ui.set_min_width(420.0);
                for e in crate::filebrowser::list(&dir) {
                    let label = if e.is_dir {
                        format!("{}/", e.name)
                    } else {
                        e.name.clone()
                    };
                    let resp = ui.selectable_label(false, label);
                    if resp.clicked() || resp.double_clicked() {
                        d.text = crate::filebrowser::pick(&dir, &e);
                        if resp.double_clicked() && !e.is_dir {
                            go = true;
                        }
                    }
                }
            });
        ui.horizontal(|ui| {
            if ui.button("OK").clicked() {
                go = true;
            }
            if ui.button("Cancel").clicked() {
                keep = false;
            }
        });
    });
    if go {
        let path = std::path::PathBuf::from(d.text.trim());
        let res = match d.kind {
            DialogKind::Open => app.open(&path),
            DialogKind::SaveAs => app.save(&path),
        };
        if let Err(e) = res {
            app.error(e, None);
        }
        keep = false;
    }
    if keep {
        st.dialog = Some(d);
    }
}

/// The CPU path: render with the shared software rasterizer into a texture. Fast while the view
/// is moving (no supersampling), crisp once it settles.
fn software_viewport(
    app: &mut AppState,
    st: &mut UiState,
    ui: &mut egui::Ui,
    rect: Rect,
    (pw, ph): (u32, u32),
    dragging: bool,
    now: f64,
) {
    let ctx = ui.ctx().clone();
    let interacting = now < st.interacting_until || dragging;
    let ss = if interacting { 1 } else { 2 };
    let want = (app.view_version, [pw, ph], ss);
    if st.drawn != Some(want) {
        let img = app.render(pw, ph, ss);
        let ci = egui::ColorImage::from_rgb([pw as usize, ph as usize], &img.rgb);
        match &mut st.texture {
            Some(t) => t.set(ci, TextureOptions::LINEAR),
            None => st.texture = Some(ctx.load_texture("viewport", ci, TextureOptions::LINEAR)),
        }
        st.drawn = Some(want);
    }
    if interacting {
        ctx.request_repaint_after(Duration::from_millis(280));
    }
    if let Some(t) = &st.texture {
        ui.painter().image(
            t.id(),
            rect,
            Rect::from_min_max(Pos2::ZERO, Pos2::new(1.0, 1.0)),
            Color32::WHITE,
        );
    }
}

fn label_pill(painter: &egui::Painter, at: Pos2, text: String, fg: Color32) {
    let galley = painter.layout_no_wrap(text, egui::FontId::proportional(13.0), fg);
    let r = Rect::from_center_size(at, galley.size() + egui::vec2(10.0, 4.0));
    painter.rect_filled(r, 4.0, Color32::from_rgba_unmultiplied(255, 255, 255, 225));
    painter.galley(r.min + egui::vec2(5.0, 2.0), galley, fg);
}

/// Dark when the constraints fully determine it, blue while it can still move: the SolidWorks
/// convention for how defined a sketch is.
const SKETCH_FREE: Color32 = Color32::from_rgb(30, 100, 220);
const SKETCH_DEFINED: Color32 = Color32::from_rgb(28, 30, 36);
const SKETCH_OVER: Color32 = Color32::from_rgb(225, 175, 0);
const SKETCH_CONFLICT: Color32 = Color32::from_rgb(220, 40, 40);
const SKETCH_PICKED: Color32 = Color32::from_rgb(255, 140, 0);
const SKETCH_PREVIEW: Color32 = Color32::from_rgb(235, 120, 0);

/// The sketch itself, the rubber band and the snap marker, drawn over the 3D view.
fn draw_sketch_overlay(app: &AppState, ui: &mut egui::Ui, rect: Rect) {
    let Some(o) = app.sketch_overlay(f64::from(rect.width()), f64::from(rect.height())) else {
        return;
    };
    let painter = ui.painter_at(rect);
    let pos = |p: (f64, f64)| rect.min + egui::vec2(p.0 as f32, p.1 as f32);
    // Axes and the origin light up while dimensioning, so it is clear what a click would pick.
    for a in &o.axes {
        let c = pos(a.a);
        let col = if a.picked {
            SKETCH_PICKED
        } else {
            Color32::from_rgb(255, 190, 90)
        };
        if a.mark == AxisMark::Origin {
            painter.circle_stroke(c, 9.0, egui::Stroke::new(2.5, col));
        } else {
            painter.line_segment([c, pos(a.b)], egui::Stroke::new(3.0, col));
        }
    }
    for s in &o.strokes {
        let pts: Vec<Pos2> = s.pts.iter().map(|p| pos(*p)).collect();
        if let Some(c) = problem_colour(s.problem) {
            painter.add(egui::Shape::line(
                pts.clone(),
                egui::Stroke::new(8.0, c.gamma_multiply(0.35)),
            ));
        }
        let colour = if s.selected {
            SKETCH_PICKED
        } else if s.hover {
            Color32::from_rgb(255, 170, 60)
        } else if let Some(c) = problem_colour(s.problem) {
            c
        } else if s.defined {
            SKETCH_DEFINED
        } else {
            SKETCH_FREE
        };
        let stroke = egui::Stroke::new(if s.selected || s.hover { 3.0 } else { 2.0 }, colour);
        if s.construction {
            painter.extend(egui::Shape::dashed_line(
                &pts,
                egui::Stroke::new(1.5, colour),
                9.0,
                5.0,
            ));
        } else {
            painter.add(egui::Shape::line(pts, stroke));
        }
    }
    for pv in &o.preview {
        let pts: Vec<Pos2> = pv.iter().map(|p| pos(*p)).collect();
        painter.add(egui::Shape::line(
            pts,
            egui::Stroke::new(1.5, SKETCH_PREVIEW),
        ));
    }
    // Trim shows what goes (red), Extend what is added (green), Split where it cuts.
    for (lines, col, width) in [
        (&o.edit_removed, Color32::from_rgb(230, 40, 40), 4.5),
        (&o.edit_added, Color32::from_rgb(30, 170, 80), 3.5),
    ] {
        for l in lines {
            let pts: Vec<Pos2> = l.iter().map(|p| pos(*p)).collect();
            painter.add(egui::Shape::line(pts, egui::Stroke::new(width, col)));
        }
    }
    for m in &o.edit_markers {
        painter.circle(
            pos(*m),
            5.0,
            Color32::from_rgb(255, 210, 60),
            egui::Stroke::new(1.8, SKETCH_PICKED),
        );
    }
    if o.trim_trail.len() > 1 {
        let pts: Vec<Pos2> = o.trim_trail.iter().map(|p| pos(*p)).collect();
        painter.add(egui::Shape::line(
            pts,
            egui::Stroke::new(1.6, Color32::from_rgb(230, 40, 40)),
        ));
    }
    // What the Modify panel would make, faintly.
    for l in &o.modify_preview {
        let pts: Vec<Pos2> = l.iter().map(|p| pos(*p)).collect();
        painter.add(egui::Shape::line(
            pts,
            egui::Stroke::new(1.8, SKETCH_PREVIEW.gamma_multiply(0.8)),
        ));
    }
    // Dashed guides while a dimension snaps to something.
    for [a, b] in &o.dim_guides {
        painter.extend(egui::Shape::dashed_line(
            &[pos(*a), pos(*b)],
            egui::Stroke::new(1.2, Color32::from_rgb(200, 60, 170)),
            5.0,
            4.0,
        ));
    }
    for m in &o.freedom {
        paint_freedom(&painter, m, rect.min);
    }
    // The selection rectangle: solid blue for "inside", dashed green for "touching".
    if let Some((a, b, crossing)) = o.select_box {
        let r = Rect::from_two_pos(pos(a), pos(b));
        let col = if crossing {
            Color32::from_rgb(40, 160, 80)
        } else {
            Color32::from_rgb(40, 110, 230)
        };
        painter.rect_filled(r, 0.0, col.gamma_multiply(0.12));
        let outline = [
            r.left_top(),
            r.right_top(),
            r.right_bottom(),
            r.left_bottom(),
            r.left_top(),
        ];
        if crossing {
            painter.extend(egui::Shape::dashed_line(
                &outline,
                egui::Stroke::new(1.4, col),
                6.0,
                4.0,
            ));
        } else {
            painter.add(egui::Shape::line(
                outline.to_vec(),
                egui::Stroke::new(1.4, col),
            ));
        }
    }
    for g in &o.glyphs {
        paint_glyph(&painter, g, rect.min);
    }
    for d in &o.points {
        if let Some(c) = problem_colour(d.problem) {
            painter.circle_filled(pos(d.pos), 8.0, c.gamma_multiply(0.35));
        }
        let (fill, edge) = match (d.selected || d.hover, d.defined) {
            (true, _) => (SKETCH_PICKED, SKETCH_PICKED),
            _ if d.problem != Problem::None => {
                let c = problem_colour(d.problem).unwrap_or(SKETCH_DEFINED);
                (c, c)
            }
            (false, true) => (SKETCH_DEFINED, SKETCH_DEFINED),
            (false, false) => (Color32::WHITE, SKETCH_FREE),
        };
        painter.circle(pos(d.pos), 3.5, fill, egui::Stroke::new(1.5, edge));
    }
    for (p, text) in &o.labels {
        label_pill(&painter, pos(*p), text.clone(), Color32::from_gray(30));
    }
    if let Some((p, text)) = &o.tooltip {
        label_pill(
            &painter,
            pos(*p) + egui::vec2(16.0, 18.0),
            text.clone(),
            Color32::from_gray(30),
        );
    }
    for d in &o.dims {
        paint_dimension(&painter, d, rect.min);
    }
    // Midpoint handles: shown on hovered / selected lines, filled when the pointer is on one.
    for (p, hot) in &o.mid_handles {
        let c = pos(*p);
        let t = [
            c + egui::vec2(0.0, -5.5),
            c + egui::vec2(5.5, 4.0),
            c + egui::vec2(-5.5, 4.0),
        ];
        let green = Color32::from_rgb(40, 150, 70);
        painter.add(egui::Shape::convex_polygon(
            t.to_vec(),
            if *hot {
                green
            } else {
                Color32::from_white_alpha(200)
            },
            egui::Stroke::new(1.6, green),
        ));
    }
    if let Some((p, kind)) = o.marker {
        let c = pos(p);
        match kind {
            Marker::Free | Marker::Horizontal | Marker::Vertical => {
                painter.circle_stroke(c, 4.0, egui::Stroke::new(1.5, SKETCH_PREVIEW));
            }
            Marker::Point => {
                painter.rect_stroke(
                    Rect::from_center_size(c, egui::vec2(12.0, 12.0)),
                    2.0,
                    egui::Stroke::new(2.0, Color32::from_rgb(220, 160, 0)),
                    egui::StrokeKind::Middle,
                );
            }
            Marker::OnEdge => {
                painter.rect_stroke(
                    Rect::from_center_size(c, egui::vec2(9.0, 9.0)),
                    4.5,
                    egui::Stroke::new(2.0, Color32::from_rgb(40, 150, 70)),
                    egui::StrokeKind::Middle,
                );
            }
            Marker::Origin => {
                painter.circle_stroke(
                    c,
                    7.0,
                    egui::Stroke::new(2.0, Color32::from_rgb(210, 60, 60)),
                );
            }
            Marker::Midpoint => {
                // A small triangle: the middle of a line.
                let t = [
                    c + egui::vec2(0.0, -6.0),
                    c + egui::vec2(6.0, 5.0),
                    c + egui::vec2(-6.0, 5.0),
                ];
                painter.add(egui::Shape::closed_line(
                    t.to_vec(),
                    egui::Stroke::new(2.0, Color32::from_rgb(40, 150, 70)),
                ));
            }
            Marker::Quadrant => {
                // A diamond: a quadrant point of a circle.
                let d = [
                    c + egui::vec2(0.0, -6.0),
                    c + egui::vec2(6.0, 0.0),
                    c + egui::vec2(0.0, 6.0),
                    c + egui::vec2(-6.0, 0.0),
                ];
                painter.add(egui::Shape::closed_line(
                    d.to_vec(),
                    egui::Stroke::new(2.0, Color32::from_rgb(40, 150, 70)),
                ));
            }
            Marker::Inferred => {
                painter.circle_stroke(
                    c,
                    5.5,
                    egui::Stroke::new(2.0, Color32::from_rgb(200, 60, 170)),
                );
            }
        }
        if let Some(h) = &o.hint {
            label_pill(
                &painter,
                c + egui::vec2(36.0, -16.0),
                h.clone(),
                SKETCH_PREVIEW,
            );
        }
    }
}

/// What the Modify panel is set up to do, and its numbers.
#[derive(Clone, Debug)]
pub struct ModifyPanel {
    pub kind: ModifyKind,
    pub dx: f64,
    pub dy: f64,
    pub copy: bool,
    pub cx: f64,
    pub cy: f64,
    pub degrees: f64,
    pub factor: f64,
    pub count: u32,
    pub distance: f64,
    pub radius: f64,
    pub d1: f64,
    pub d2: f64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ModifyKind {
    Move,
    Rotate,
    Scale,
    Mirror,
    LinearPattern,
    CircularPattern,
    Offset,
    Fillet,
    Chamfer,
}

impl ModifyKind {
    pub const ALL: [ModifyKind; 9] = [
        ModifyKind::Move,
        ModifyKind::Rotate,
        ModifyKind::Scale,
        ModifyKind::Mirror,
        ModifyKind::LinearPattern,
        ModifyKind::CircularPattern,
        ModifyKind::Offset,
        ModifyKind::Fillet,
        ModifyKind::Chamfer,
    ];

    pub fn name(self) -> &'static str {
        match self {
            ModifyKind::Move => "Move / Copy",
            ModifyKind::Rotate => "Rotate",
            ModifyKind::Scale => "Scale",
            ModifyKind::Mirror => "Mirror",
            ModifyKind::LinearPattern => "Linear pattern",
            ModifyKind::CircularPattern => "Circular pattern",
            ModifyKind::Offset => "Offset",
            ModifyKind::Fillet => "Fillet",
            ModifyKind::Chamfer => "Chamfer",
        }
    }
}

impl ModifyPanel {
    pub fn new(kind: ModifyKind, centre: (f64, f64)) -> Self {
        ModifyPanel {
            kind,
            dx: 10.0,
            dy: 0.0,
            copy: false,
            cx: centre.0,
            cy: centre.1,
            degrees: 90.0,
            factor: 2.0,
            count: 3,
            distance: 5.0,
            radius: 3.0,
            d1: 3.0,
            d2: 3.0,
        }
    }

    /// The operation these numbers describe (mirror has none: it is chosen by its buttons).
    fn op(&self) -> Option<ModifyOp> {
        Some(match self.kind {
            ModifyKind::Move => ModifyOp::Move {
                dx: self.dx,
                dy: self.dy,
                copy: self.copy,
            },
            ModifyKind::Rotate => ModifyOp::Rotate {
                cx: self.cx,
                cy: self.cy,
                degrees: self.degrees,
                copy: self.copy,
            },
            ModifyKind::Scale => ModifyOp::Scale {
                cx: self.cx,
                cy: self.cy,
                factor: self.factor,
                copy: self.copy,
            },
            ModifyKind::Mirror => return None,
            ModifyKind::LinearPattern => ModifyOp::LinearPattern {
                dx: self.dx,
                dy: self.dy,
                count: self.count,
            },
            ModifyKind::CircularPattern => ModifyOp::CircularPattern {
                cx: self.cx,
                cy: self.cy,
                degrees: if self.degrees.abs() < 1e-9 {
                    360.0
                } else {
                    self.degrees
                },
                count: self.count,
            },
            ModifyKind::Offset => ModifyOp::Offset {
                distance: self.distance,
            },
            ModifyKind::Fillet => ModifyOp::Fillet {
                radius: self.radius,
            },
            ModifyKind::Chamfer => ModifyOp::Chamfer {
                d1: self.d1,
                d2: self.d2,
            },
        })
    }
}

/// The Modify panel: pick an operation, set its numbers, see what it would make, apply it.
fn sketch_toolbar(app: &mut AppState, ctx: &egui::Context, rect: Rect) {
    let Some(s) = &app.sketching else { return };
    let (tool, name, construction) = (s.tool, s.sketch.clone(), s.construction);
    let mut cut_construction = s.cut_construction;
    let (mut sides, mut circumscribed) = (s.polygon_sides, s.polygon_circumscribed);
    let mut midpoint_line = s.midpoint_line;
    let mut centerline_infinite = s.centerline_infinite;
    let trim_mode = s.trim_mode;
    let bounds_ready = s.trim_bounds.len() == 2;
    let mut next_mode: Option<TrimMode> = None;
    let picked_relation = !s.relations.is_empty();
    let selection = s.selection.len();
    let summary = app.sketch_overlay(1.0, 1.0);
    let dof = summary.as_ref().map_or(0, |o| o.dof);
    let entity_count = app.sketch_data().map_or(0, |sk| sk.entities().len());
    let shapes_defined = summary.as_ref().is_some_and(|o| o.shapes_defined);
    let redundant = summary.as_ref().map_or(0, |o| o.redundant);
    let conflict = summary.as_ref().is_some_and(|o| o.conflict);
    let relation_text = summary.as_ref().and_then(|o| o.relation.clone());
    let drag_warning = summary.as_ref().and_then(|o| o.drag_warning.clone());
    let drag_note = summary.as_ref().and_then(|o| o.drag_note.clone());
    let conflict_note = app.sketching.as_ref().and_then(|s| s.conflict_note.clone());
    let problems = app
        .sketching
        .as_ref()
        .and_then(|s| s.check.clone())
        .unwrap_or_default();
    let mut remove_relation = false;
    let prompt = summary
        .and_then(|o| o.dim_prompt)
        .filter(|_| tool == Tool::Dimension);
    let relations = app.available_relations();
    let mut toggle_construction = false;
    let mut toggle_infinite = false;
    let has_construction_line = app.sketch_data().is_some_and(|sk| {
        app.sketching.as_ref().is_some_and(|s| {
            s.selection.iter().any(|id| {
                matches!(sk.entity(id), Some(dc_sketch::Entity::Line { .. }))
                    && sk.is_construction(id)
            })
        })
    });
    let mut relation = None;
    let mut delete = false;
    let mut action: Option<ToolbarAction> = None;
    // Beside the view toolbar if there is room for a readable bar, else below it, so the two
    // never overlap in a narrow window.
    let beside = rect.width() - shell::hud_reserved_width(rect.width());
    let (bar_top, bar_width) = if beside >= 240.0 {
        (8.0, beside)
    } else {
        (52.0, (rect.width() - 20.0).max(160.0))
    };
    egui::Area::new(egui::Id::new("sketch_bar"))
        .fixed_pos(rect.left_top() + egui::vec2(10.0, bar_top))
        .show(ctx, |ui| {
            egui::Frame::popup(ui.style()).show(ui, |ui| {
                // Wrap onto more rows in a narrow window (a small tile).
                ui.set_max_width(bar_width);
                ui.horizontal_wrapped(|ui| {
                    ui.label(RichText::new(format!("Sketch '{name}'")).strong());
                    ui.separator();
                    ui.separator();
                    if conflict {
                        ui.colored_label(SKETCH_CONFLICT, "conflict");
                    } else if redundant > 0 {
                        ui.colored_label(SKETCH_OVER, format!("over defined ({redundant})"));
                    } else if entity_count == 0 {
                        ui.label(RichText::new("empty").weak());
                    } else if dof == 0 {
                        ui.colored_label(Color32::from_rgb(30, 150, 70), "fully defined");
                    } else {
                        ui.colored_label(
                            SKETCH_FREE,
                            if shapes_defined {
                                format!("shapes defined · construction {dof} DOF free")
                            } else {
                                format!("{dof} DOF free")
                            },
                        );
                    }
                });
                ui.horizontal_wrapped(|ui| {
                    if ui
                        .selectable_label(construction, "Construction (X)")
                        .on_hover_text("New lines, circles and arcs become dashed guides that are not part of the profile. With something selected, X converts the selection instead.")
                        .clicked()
                    {
                        toggle_construction = true;
                    }
                    if tool == Tool::Centerline {
                        ui.separator();
                        ui.checkbox(&mut centerline_infinite, "Infinite length")
                            .on_hover_text("A centre line that runs on for ever both ways: it has no ends, so only where it is placed has to be defined");
                    }
                    if matches!(tool, Tool::Line | Tool::Centerline) {
                        ui.separator();
                        ui.checkbox(&mut midpoint_line, "From the middle")
                            .on_hover_text("First click is the middle of the line, second click one end; the other end mirrors it and the middle is kept in the middle");
                    }
                    if tool == Tool::Polygon {
                        ui.separator();
                        ui.label("sides");
                        ui.add(DragValue::new(&mut sides).range(3..=64));
                        ui.checkbox(&mut circumscribed, "around the circle")
                            .on_hover_text("Off: the click is a corner. On: the click is the middle of a side");
                    }
                });
                if let Some(p) = &prompt {
                    ui.separator();
                    ui.label(RichText::new(p).color(SKETCH_PREVIEW));
                }
                if matches!(tool, Tool::Trim | Tool::Extend) {
                    ui.separator();
                    ui.horizontal_wrapped(|ui| {
                        if tool == Tool::Trim {
                            for (i, m) in TrimMode::ALL.into_iter().enumerate() {
                                if ui
                                    .selectable_label(trim_mode == m, format!("{} ({})", m.name(), i + 1))
                                    .on_hover_text(m.hint())
                                    .clicked()
                                {
                                    next_mode = Some(m);
                                }
                            }
                            if trim_mode == TrimMode::Between && bounds_ready {
                                ui.separator();
                                if ui
                                    .button("Remove inside")
                                    .on_hover_text("Everything crossing both bounds loses the part between them")
                                    .clicked()
                                {
                                    action = Some(ToolbarAction::TrimBetween(true));
                                }
                                if ui
                                    .button("Remove outside")
                                    .on_hover_text("Everything crossing both bounds loses the parts beyond them")
                                    .clicked()
                                {
                                    action = Some(ToolbarAction::TrimBetween(false));
                                }
                            }
                            ui.separator();
                        }
                        ui.checkbox(&mut cut_construction, "Construction lines cut")
                            .on_hover_text(
                                "Off: centre lines and other construction geometry are ignored when working out what cuts a shape",
                            );
                    });
                }
                if let Some(n) = &drag_note {
                    ui.separator();
                    ui.colored_label(SKETCH_OVER, n);
                }
                if let Some(w) = &drag_warning {
                    ui.separator();
                    ui.colored_label(
                        SKETCH_CONFLICT,
                        format!("Releasing here will be refused: {w}"),
                    );
                }
                if let Some(n) = conflict_note.as_ref().filter(|_| conflict) {
                    ui.separator();
                    ui.colored_label(SKETCH_CONFLICT, n);
                }
                for p in &problems {
                    ui.colored_label(SKETCH_CONFLICT, format!("! {}", p.message));
                }
                if let (true, Some(text)) = (picked_relation, &relation_text) {
                    ui.separator();
                    ui.horizontal_wrapped(|ui| {
                        ui.label(format!("selected: {text}"));
                        if ui.button("Remove").clicked() {
                            remove_relation = true;
                        }
                    });
                }
                if selection > 0 && tool == Tool::Select {
                    ui.separator();
                    ui.horizontal_wrapped(|ui| {
                        ui.label(format!("{selection} selected:"));
                        for r in Relation::ALL {
                            if relations.contains(&r) && ui.button(r.name()).clicked() {
                                relation = Some(r);
                            }
                        }
                        if ui.button("Construction").clicked() {
                            toggle_construction = true;
                        }
                        if has_construction_line
                            && ui
                                .button("Infinite")
                                .on_hover_text("Make the selected construction lines infinite, or finite again")
                                .clicked()
                        {
                            toggle_infinite = true;
                        }
                        if ui.button("Delete").clicked() {
                            delete = true;
                        }
                    });
                }
            });
        });
    if let Some(s) = &mut app.sketching {
        s.cut_construction = cut_construction;
        s.polygon_sides = sides;
        s.midpoint_line = midpoint_line;
        s.centerline_infinite = centerline_infinite;
        s.polygon_circumscribed = circumscribed;
    }
    if let Some(m) = next_mode {
        app.set_trim_mode(m);
    }
    if remove_relation {
        app.delete_selected_relation();
    }
    if toggle_construction {
        app.toggle_construction();
    }
    if toggle_infinite {
        app.toggle_infinite();
    }
    if let Some(r) = relation {
        app.apply_relation(r);
    }
    if delete {
        app.sketch_delete_selected();
    }
    match action {
        Some(ToolbarAction::TrimBetween(inside)) => {
            app.sketch_trim_between(inside);
        }
        None => {}
    }
}

/// Something the toolbar asked for.
enum ToolbarAction {
    TrimBetween(bool),
}

/// A relation's symbol in a small rounded box: lines for horizontal / vertical / parallel /
/// perpendicular / equal, and a few pictograms for the rest.
fn paint_glyph(painter: &egui::Painter, g: &Glyph, origin: Pos2) {
    let k = g.scale as f32;
    let c = origin + egui::vec2(g.pos.0 as f32, g.pos.1 as f32);
    let ink = if g.selected {
        SKETCH_PICKED
    } else if let Some(p) = problem_colour(g.problem) {
        p
    } else if g.hover {
        Color32::from_rgb(255, 150, 20)
    } else {
        Color32::from_rgb(60, 70, 90)
    };
    let boxr = Rect::from_center_size(c, egui::vec2(16.0 * k, 16.0 * k));
    painter.rect(
        boxr,
        3.0,
        Color32::from_rgba_unmultiplied(
            255,
            255,
            255,
            if g.hover || g.selected { 245 } else { 215 },
        ),
        egui::Stroke::new(if g.selected || g.hover { 1.8 } else { 1.0 }, ink),
        egui::StrokeKind::Inside,
    );
    let st = egui::Stroke::new(1.5, ink);
    let p = |x: f32, y: f32| c + egui::vec2(x * k, y * k);
    let seg = |a: Pos2, b: Pos2| {
        painter.line_segment([a, b], st);
    };
    match g.kind {
        GlyphKind::Horizontal => seg(p(-5.0, 0.0), p(5.0, 0.0)),
        GlyphKind::Vertical => seg(p(0.0, -5.0), p(0.0, 5.0)),
        GlyphKind::Parallel => {
            seg(p(-3.5, 4.5), p(0.5, -4.5));
            seg(p(0.0, 4.5), p(4.0, -4.5));
        }
        GlyphKind::Perpendicular => {
            seg(p(0.0, -5.0), p(0.0, 4.0));
            seg(p(-5.0, 4.0), p(5.0, 4.0));
        }
        GlyphKind::Equal => {
            seg(p(-4.5, -2.0), p(4.5, -2.0));
            seg(p(-4.5, 2.0), p(4.5, 2.0));
        }
        GlyphKind::Coincident => {
            painter.circle_stroke(c, 3.5 * k, st);
            painter.circle_filled(c, 1.3 * k, ink);
        }
        GlyphKind::OnLine | GlyphKind::OnAxis => {
            seg(p(-5.5, 4.0), p(5.5, -4.0));
            painter.circle_filled(c, 2.2 * k, ink);
        }
        GlyphKind::Fixed => {
            painter.rect_filled(Rect::from_min_max(p(-3.5, -0.5), p(3.5, 5.0)), 1.0, ink);
            painter.add(egui::Shape::line(
                vec![p(-2.2, -0.5), p(-2.2, -3.0), p(2.2, -3.0), p(2.2, -0.5)],
                st,
            ));
        }
        GlyphKind::Tangent => {
            painter.circle_stroke(p(0.0, 1.5), 3.5 * k, st);
            seg(p(-5.5, -2.0), p(5.5, -2.0));
        }
        GlyphKind::Midpoint => {
            seg(p(-5.5, 0.0), p(5.5, 0.0));
            seg(p(-5.5, -2.5), p(-5.5, 2.5));
            seg(p(5.5, -2.5), p(5.5, 2.5));
            painter.circle_filled(c, 2.2 * k, ink);
        }
        GlyphKind::Symmetric => {
            seg(p(0.0, -5.5), p(0.0, 5.5));
            painter.circle_filled(p(-4.0, 0.0), 1.8 * k, ink);
            painter.circle_filled(p(4.0, 0.0), 1.8 * k, ink);
        }
    }
}

fn problem_colour(p: Problem) -> Option<Color32> {
    match p {
        Problem::None => None,
        Problem::Over => Some(SKETCH_OVER),
        Problem::Conflict => Some(SKETCH_CONFLICT),
    }
}

/// Small arrows showing which way a point (or a circle's radius) can still move: a double arrow
/// along the one direction it is free in, or a cross when it can go anywhere.
fn paint_freedom(painter: &egui::Painter, m: &FreedomMark, origin: Pos2) {
    let c = origin + egui::vec2(m.pos.0 as f32, m.pos.1 as f32);
    let col = SKETCH_FREE.gamma_multiply(if m.emphasis { 1.0 } else { 0.75 });
    let width = if m.emphasis { 2.0 } else { 1.4 };
    let arrow = |dir: egui::Vec2, from: f32, to: f32| {
        let tail = c + dir * from;
        let tip = c + dir * to;
        painter.line_segment([tail, tip], egui::Stroke::new(width, col));
        let side = egui::vec2(-dir.y, dir.x) * 2.6;
        let base = tip - dir * 5.0;
        painter.add(egui::Shape::convex_polygon(
            vec![tip, base + side, base - side],
            col,
            egui::Stroke::NONE,
        ));
    };
    match (m.dof, m.dir) {
        (1, Some(d)) => {
            let d = egui::vec2(d.0 as f32, d.1 as f32);
            let (from, to) = if m.radius || m.sweep {
                (0.0, 14.0)
            } else {
                (6.0, 20.0)
            };
            arrow(d, from, to);
            arrow(-d, from, to);
        }
        (2, _) if m.emphasis => {
            for d in [
                egui::vec2(1.0, 0.0),
                egui::vec2(-1.0, 0.0),
                egui::vec2(0.0, 1.0),
                egui::vec2(0.0, -1.0),
            ] {
                arrow(d, 6.0, 16.0);
            }
        }
        _ => {}
    }
}

/// One ISO-style dimension: extension lines, a dimension line with filled arrowheads, and the
/// text above the line, turned to follow it.
fn paint_dimension(painter: &egui::Painter, d: &DimDraw, origin: Pos2) {
    let pos = |p: (f64, f64)| origin + egui::vec2(p.0 as f32, p.1 as f32);
    let ink = if d.preview {
        SKETCH_PREVIEW
    } else if d.driven && !d.selected && !d.hover {
        Color32::from_rgb(120, 126, 140)
    } else if d.selected {
        SKETCH_PICKED
    } else if d.hover {
        Color32::from_rgb(255, 150, 20)
    } else {
        Color32::from_rgb(40, 44, 52)
    };
    for [a, b] in &d.witness {
        painter.line_segment(
            [pos(*a), pos(*b)],
            egui::Stroke::new(1.0, ink.gamma_multiply(0.75)),
        );
    }
    for [a, b] in &d.line {
        painter.line_segment([pos(*a), pos(*b)], egui::Stroke::new(1.3, ink));
    }
    for (tip, dir) in &d.arrows {
        let (tip, dir) = (pos(*tip), egui::vec2(dir.0 as f32, dir.1 as f32));
        let side = egui::vec2(-dir.y, dir.x) * 3.2;
        let base = tip - dir * 11.0;
        painter.add(egui::Shape::convex_polygon(
            vec![tip, base + side, base - side],
            ink,
            egui::Stroke::NONE,
        ));
    }
    let galley = painter.layout_no_wrap(d.text.clone(), egui::FontId::proportional(14.0), ink);
    let rot = egui::emath::Rot2::from_angle(d.angle as f32);
    let lift = if d.above {
        egui::vec2(0.0, -11.0)
    } else {
        egui::Vec2::ZERO
    };
    let centre = pos(d.text_at) + rot * lift;
    let top_left = centre - rot * (galley.size() / 2.0);
    painter.add(egui::epaint::TextShape::new(top_left, galley, ink).with_angle(d.angle as f32));
}

/// The box where a placed dimension's value is typed, next to the dimension itself.
fn dimension_box(app: &mut AppState, st: &mut UiState, ctx: &egui::Context, rect: Rect) {
    let Some(mut edit) = app.sketching.as_ref().and_then(|s| s.dim.edit.clone()) else {
        st.dim_focus = None;
        return;
    };
    let near = app
        .sketch_overlay(f64::from(rect.width()), f64::from(rect.height()))
        .and_then(|o| o.edit_at)
        .map_or(rect.center_bottom() - egui::vec2(0.0, 120.0), |p| {
            rect.min + egui::vec2(p.0 as f32, p.1 as f32)
        });
    let at = Pos2::new(
        (near.x - 70.0).clamp(rect.min.x + 4.0, (rect.max.x - 190.0).max(rect.min.x + 4.0)),
        (near.y + 22.0).clamp(rect.min.y + 4.0, (rect.max.y - 110.0).max(rect.min.y + 4.0)),
    );
    // A fresh box (a different dimension) grabs the keyboard and selects its number.
    let key = format!("{:?}{:?}", edit.spec, edit.existing);
    let mut apply = false;
    let mut cancel = false;
    egui::Window::new(edit.spec.title())
        .id(egui::Id::new("dimension_box"))
        .collapsible(false)
        .resizable(false)
        .fixed_pos(at)
        .show(ctx, |ui| {
            ui.horizontal(|ui| {
                let out = egui::TextEdit::singleline(&mut edit.text)
                    .desired_width(90.0)
                    .show(ui);
                ui.label(edit.spec.unit());
                if st.dim_focus.as_deref() != Some(key.as_str()) {
                    out.response.request_focus();
                    let mut state = out.state.clone();
                    state
                        .cursor
                        .set_char_range(Some(egui::text::CCursorRange::two(
                            egui::text::CCursor::new(0),
                            egui::text::CCursor::new(edit.text.chars().count()),
                        )));
                    state.store(ctx, out.response.id);
                    st.dim_focus = Some(key.clone());
                }
                if out.response.lost_focus() && ui.input(|i| i.key_pressed(Key::Enter)) {
                    apply = true;
                }
            });
            if ui.input(|i| i.key_pressed(Key::Escape)) {
                cancel = true;
            }
            ui.checkbox(&mut edit.driven, "Reference (measure only)")
                .on_hover_text(
                    "A reference dimension shows what the geometry measures, in brackets, and does not constrain it",
                );
            egui::CollapsingHeader::new("Precision and tolerance")
                .default_open(edit.style != dc_sketch::DimStyle::default())
                .show(ui, |ui| {
                    ui.horizontal(|ui| {
                        ui.label("decimals");
                        let label = edit.style.decimals.map_or("auto".to_owned(), |d| d.to_string());
                        egui::ComboBox::from_id_salt("dim_decimals")
                            .selected_text(label)
                            .show_ui(ui, |ui| {
                                ui.selectable_value(&mut edit.style.decimals, None, "auto");
                                for d in 0..=4_u8 {
                                    ui.selectable_value(&mut edit.style.decimals, Some(d), d.to_string());
                                }
                            });
                    });
                    ui.horizontal(|ui| {
                        ui.label("tolerance +");
                        ui.add(DragValue::new(&mut edit.style.tol_plus).speed(0.01).range(0.0..=1000.0));
                        ui.label("−");
                        ui.add(DragValue::new(&mut edit.style.tol_minus).speed(0.01).range(0.0..=1000.0));
                    });
                });
            if let Some(e) = &edit.error {
                ui.colored_label(Color32::from_rgb(210, 70, 60), e);
            }
            ui.horizontal(|ui| {
                if ui.button("OK").clicked() {
                    apply = true;
                }
                if ui.button("Cancel").clicked() {
                    cancel = true;
                }
            });
        });
    if let Some(s) = &mut app.sketching {
        if s.dim.edit.is_some() {
            s.dim.edit = Some(edit);
        }
    }
    if cancel {
        app.sketch_cancel();
    } else if apply {
        app.apply_dimension();
    }
}
