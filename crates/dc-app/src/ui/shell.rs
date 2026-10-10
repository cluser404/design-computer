//! The frame around the viewport: the title bar (menu, quick access, command search), the ribbon
//! tabs, the command palette, the heads-up view toolbar, the confirmation corner and the status
//! bar. They take their commands from the registry, like the ribbon does.

use super::icons::{self, CadIcon, Icon};
use super::ribbon::{self, Cx, Tab};
use super::theme::Colors;
use super::{icon_font as g, UiState};
use crate::commands::{self};
use crate::sketching::Tool;
use crate::state::{AppState, Level};
use eframe::egui::{
    self, Align, Align2, FontId, Key, Layout, Pos2, Rect, RichText, Sense, Stroke, StrokeKind, Ui,
    Vec2,
};

/// State of the open command palette.
#[derive(Default, Clone)]
pub struct Palette {
    pub query: String,
    pub selected: usize,
    /// Set on the frame the palette opens, to put the cursor in its field.
    pub fresh: bool,
}

/// A small icon-only button for the quick access row and the view toolbar. Returns clicked.
fn icon_button(ui: &mut Ui, cx: &mut Cx, id: &'static str, size: f32) -> bool {
    let off = commands::enabled(id, cx.app, cx.host).err();
    let (rect, resp) = ui.allocate_exact_size(Vec2::splat(size), Sense::click());
    let on = commands::checked(id, cx.app, cx.host).unwrap_or(false);
    if on || resp.hovered() && off.is_none() || resp.is_pointer_button_down_on() {
        let (fill, line) = if on || resp.is_pointer_button_down_on() {
            (cx.colors.active, cx.colors.active_line)
        } else {
            (cx.colors.hover, cx.colors.hover_line)
        };
        ui.painter()
            .rect(rect, 3.0, fill, Stroke::new(1.0, line), StrokeKind::Inside);
    }
    if let Some(c) = commands::find(id) {
        let inner = Rect::from_center_size(rect.center(), Vec2::splat(size - 8.0));
        icons::paint(ui.painter(), inner, c.icon, &cx.colors, off.is_none());
    }
    let colors = cx.colors;
    resp.clone().on_hover_ui(|ui| {
        if let Some(c) = commands::find(id) {
            ui.horizontal(|ui| {
                ui.label(RichText::new(c.label.trim_end_matches('…')).strong());
                if !c.shortcut.is_empty() {
                    ui.label(RichText::new(format!("({})", c.shortcut)).color(colors.text_weak));
                }
            });
            ui.label(&c.tip);
            if let Some(r) = off {
                ui.label(RichText::new(r).color(colors.ink_orange));
            }
        }
    });
    resp.clicked() && off.is_none()
}

/// The title bar: the menu, the quick access buttons, the document name, and command search.
pub fn titlebar(app: &mut AppState, st: &mut UiState, ui: &mut Ui) {
    let colors = Colors::get(ui.ctx());
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 3.0;
        ui.add_space(4.0);
        // The logo opens the classic menus (File, Edit, ...), as SolidWorks' logo does.
        let logo = ui.add(
            egui::Button::new(
                RichText::new(format!("DC {}", g::CARET_DOWN))
                    .strong()
                    .color(colors.accent),
            )
            .min_size(Vec2::new(46.0, 24.0)),
        );
        egui::Popup::menu(&logo).show(|ui| {
            ui.set_min_width(170.0);
            super::file_menu(app, st, ui);
            super::edit_menu(app, ui);
            ui.menu_button("Sketch", |ui| super::sketch_menu(app, ui));
            ui.menu_button("Feature", |ui| super::feature_menu(app, ui));
            ui.menu_button("View", |ui| super::view_menu(app, st, ui));
            ui.separator();
            ui.checkbox(&mut st.menu_pinned, "Keep the menu bar visible");
        });
        ui.add_space(4.0);
        // Quick access: the commands used all the time.
        {
            let mut cx = Cx {
                app,
                host: st,
                colors,
            };
            let mut todo: Vec<&'static str> = Vec::new();
            for id in [
                "file.new",
                "file.open",
                "file.save",
                "edit.undo",
                "edit.redo",
            ] {
                let id: &'static str = commands::find(id).map_or("", |c| c.id.as_str());
                if icon_button(ui, &mut cx, id, 26.0) {
                    todo.push(id);
                }
            }
            for id in todo {
                if let Err(e) = commands::run(id, cx.app, cx.host) {
                    cx.app.error(e, None);
                }
            }
        }
        // Right to left: command search, then the document name in the middle.
        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            ui.add_space(6.0);
            let (rect, resp) = ui.allocate_exact_size(Vec2::new(230.0, 24.0), Sense::click());
            ui.painter().rect(
                rect,
                12.0,
                colors.ribbon,
                Stroke::new(
                    1.0,
                    if resp.hovered() {
                        colors.hover_line
                    } else {
                        colors.line
                    },
                ),
                StrokeKind::Inside,
            );
            ui.painter().text(
                Pos2::new(rect.left() + 12.0, rect.center().y),
                Align2::LEFT_CENTER,
                g::MAGNIFYING_GLASS.to_string(),
                super::theme::icon_font(ui.ctx(), 13.0),
                colors.text_weak,
            );
            ui.painter().text(
                Pos2::new(rect.left() + 32.0, rect.center().y),
                Align2::LEFT_CENTER,
                "Search commands",
                FontId::proportional(12.5),
                colors.text_weak,
            );
            ui.painter().text(
                Pos2::new(rect.right() - 12.0, rect.center().y),
                Align2::RIGHT_CENTER,
                "Ctrl+K",
                FontId::proportional(11.0),
                colors.text_weak,
            );
            if resp
                .on_hover_text("Find any command by typing part of its name")
                .clicked()
            {
                st.open_palette();
            }
            let name = app
                .path
                .as_ref()
                .and_then(|p| p.file_name())
                .map_or("untitled".into(), |n| n.to_string_lossy().into_owned());
            ui.with_layout(Layout::left_to_right(Align::Center), |ui| {
                ui.add_space(14.0);
                ui.label(RichText::new(name).strong());
                if app.dirty() {
                    ui.label(RichText::new("●").small().color(colors.ink_orange))
                        .on_hover_text("unsaved changes");
                }
            });
        });
    });
}

/// The tab strip. Returns the tab clicked, if any.
pub fn tab_strip(app: &AppState, st: &mut UiState, ui: &mut Ui) {
    let colors = Colors::get(ui.ctx());
    let mut tab_rects: Vec<(Tab, Rect)> = Vec::new();
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 2.0;
        ui.add_space(8.0);
        for tab in Tab::ALL {
            let font = FontId::proportional(13.0);
            let w = ui
                .painter()
                .layout_no_wrap(tab.label().to_owned(), font.clone(), colors.text)
                .size()
                .x
                + 22.0;
            let (rect, resp) = ui.allocate_exact_size(Vec2::new(w, 26.0), Sense::click());
            let selected = st.ribbon_tab == tab && !st.ribbon_hidden;
            if selected {
                ui.painter().rect_filled(rect, 0.0, colors.ribbon);
                ui.painter().line_segment(
                    [rect.left_top(), rect.right_top()],
                    Stroke::new(2.0, colors.accent),
                );
            } else if resp.hovered() {
                ui.painter()
                    .rect_filled(rect, 0.0, colors.hover.gamma_multiply(0.7));
            }
            ui.painter().text(
                rect.center() + Vec2::new(0.0, 1.0),
                Align2::CENTER_CENTER,
                tab.label(),
                font,
                if selected {
                    colors.text
                } else {
                    colors.text_weak
                },
            );
            if resp.clicked() {
                if st.ribbon_tab == tab {
                    st.ribbon_hidden = !st.ribbon_hidden;
                } else {
                    st.ribbon_tab = tab;
                    st.ribbon_hidden = false;
                }
            }
            crate::devtools::tag(
                &format!("tab.{}", tab.label().to_lowercase()),
                eguidev::WidgetRole::Selectable,
                tab.label(),
                Some(selected),
                &resp,
            );
            tab_rects.push((tab, rect));
            resp.on_hover_text("Click the open tab again to fold the ribbon away");
        }
        let _ = app;
    });
    if st.tips == crate::keytips::Tips::Tabs {
        let labels: Vec<&str> = Tab::ALL.iter().map(|t| t.label()).collect();
        let letters = crate::keytips::letters(&labels);
        let painter = ui.ctx().layer_painter(egui::LayerId::new(
            egui::Order::Tooltip,
            egui::Id::new("key_tips"),
        ));
        for ((_, rect), l) in tab_rects.iter().zip(&letters) {
            badge(
                &painter,
                rect.center_bottom() - Vec2::new(0.0, 4.0),
                &l.to_string(),
                &colors,
            );
        }
        if let Some(k) = st.tip_key.take() {
            if let Some(i) = letters.iter().position(|l| l.to_string() == k) {
                st.ribbon_tab = tab_rects[i].0;
                st.ribbon_hidden = false;
                st.tips = crate::keytips::Tips::Commands;
            }
        }
    }
}

/// Draws the command row for the current tab.
pub fn ribbon_row(app: &mut AppState, st: &mut UiState, ui: &mut Ui) {
    let colors = Colors::get(ui.ctx());
    let tab = st.ribbon_tab;
    ribbon::begin_tips(ui.ctx());
    let mut cx = Cx {
        app,
        host: st,
        colors,
    };
    ribbon::show_commands(ui, &mut cx, tab);
    key_tip_commands(app, st, ui);
}

/// The Alt key tips for the commands of the open tab: a badge on each runnable button, and
/// the key pressed runs its command.
fn key_tip_commands(app: &mut AppState, st: &mut UiState, ui: &Ui) {
    use crate::keytips::{command_keys, Tips};
    if st.tips != Tips::Commands {
        return;
    }
    let targets = ribbon::tip_targets(ui.ctx());
    let keys = command_keys(targets.len());
    let colors = Colors::get(ui.ctx());
    let painter = ui.ctx().layer_painter(egui::LayerId::new(
        egui::Order::Tooltip,
        egui::Id::new("key_tips"),
    ));
    for ((_, rect), k) in targets.iter().zip(&keys) {
        badge(
            &painter,
            rect.center_bottom() - Vec2::new(0.0, 8.0),
            k,
            &colors,
        );
    }
    if let Some(pressed) = st.tip_key.take() {
        if let Some(i) = keys.iter().position(|k| *k == pressed) {
            st.tips = Tips::Off;
            if let Err(e) = commands::run(targets[i].0, app, st) {
                app.error(e, None);
            }
        }
    }
}

fn badge(p: &egui::Painter, at: Pos2, text: &str, colors: &Colors) {
    let r = Rect::from_center_size(at, Vec2::new(18.0, 16.0));
    p.rect(
        r,
        3.0,
        colors.text,
        Stroke::new(1.0, colors.paper),
        StrokeKind::Inside,
    );
    p.text(
        r.center(),
        Align2::CENTER_CENTER,
        text,
        FontId::proportional(11.0),
        colors.paper,
    );
}

/// Opens the command palette on Ctrl+K and runs its keyboard handling and list.
pub fn palette(app: &mut AppState, st: &mut UiState, ctx: &egui::Context) {
    let Some(mut p) = st.palette.take() else {
        return;
    };
    let colors = Colors::get(ctx);
    let results = commands::search(&p.query);
    let shown = results.len().min(11);
    let (mut run_id, mut close) = (None, false);
    ctx.input_mut(|i| {
        if i.consume_key(egui::Modifiers::NONE, Key::ArrowDown) && shown > 0 {
            p.selected = (p.selected + 1).min(shown - 1);
        }
        if i.consume_key(egui::Modifiers::NONE, Key::ArrowUp) {
            p.selected = p.selected.saturating_sub(1);
        }
        if i.consume_key(egui::Modifiers::NONE, Key::Escape) {
            close = true;
        }
        if i.consume_key(egui::Modifiers::NONE, Key::Enter) {
            run_id = results.get(p.selected).map(|c| c.id.clone());
        }
    });
    egui::Area::new(egui::Id::new("palette"))
        .anchor(Align2::CENTER_TOP, [0.0, 56.0])
        .order(egui::Order::Foreground)
        .show(ctx, |ui| {
            egui::Frame::popup(ui.style())
                .inner_margin(egui::Margin::same(8))
                .show(ui, |ui| {
                    ui.set_width(460.0);
                    let edit = ui.add(
                        egui::TextEdit::singleline(&mut p.query)
                            .desired_width(f32::INFINITY)
                            .hint_text("Type a command: undo, circle, extrude, view…")
                            .font(FontId::proportional(15.0)),
                    );
                    if p.fresh {
                        edit.request_focus();
                        p.fresh = false;
                    }
                    if edit.changed() {
                        p.selected = 0;
                    }
                    ui.add_space(4.0);
                    if results.is_empty() {
                        ui.label(RichText::new("no command matches").color(colors.text_weak));
                    }
                    for (n, c) in results.iter().take(shown).enumerate() {
                        let off = commands::enabled(&c.id, app, st).err();
                        let (rect, resp) = ui.allocate_exact_size(
                            Vec2::new(ui.available_width(), 26.0),
                            Sense::click(),
                        );
                        if n == p.selected || resp.hovered() {
                            ui.painter().rect(
                                rect,
                                3.0,
                                colors.hover,
                                Stroke::new(1.0, colors.hover_line),
                                StrokeKind::Inside,
                            );
                        }
                        let icon = Rect::from_center_size(
                            Pos2::new(rect.left() + 16.0, rect.center().y),
                            Vec2::splat(16.0),
                        );
                        icons::paint(ui.painter(), icon, c.icon, &colors, off.is_none());
                        let col = if off.is_none() {
                            colors.text
                        } else {
                            colors.text.gamma_multiply(0.5)
                        };
                        ui.painter().text(
                            Pos2::new(rect.left() + 34.0, rect.center().y),
                            Align2::LEFT_CENTER,
                            &c.label,
                            FontId::proportional(13.0),
                            col,
                        );
                        let right = if c.shortcut.is_empty() {
                            c.category.to_owned()
                        } else {
                            format!("{}   {}", c.category, c.shortcut)
                        };
                        ui.painter().text(
                            Pos2::new(rect.right() - 8.0, rect.center().y),
                            Align2::RIGHT_CENTER,
                            right,
                            FontId::proportional(11.0),
                            colors.text_weak,
                        );
                        if let Some(r) = off {
                            resp.clone().on_hover_text(r);
                        }
                        if resp.clicked() {
                            run_id = Some(c.id.clone());
                        }
                    }
                });
        });
    if let Some(id) = run_id {
        // A command that cannot run now says why, and the palette stays open.
        match commands::enabled(&id, app, st) {
            Ok(()) => {
                if let Err(e) = commands::run(&id, app, st) {
                    app.error(e, None);
                }
                close = true;
            }
            Err(reason) => app.info(format!(
                "{}: {reason}",
                commands::find(&id).map_or("", |c| c.label.as_str())
            )),
        }
    }
    if !close {
        st.palette = Some(p);
    }
}

/// The view toolbar is compact (three buttons) when the viewport is narrower than this.
const HUD_COMPACT_BELOW: f32 = 760.0;

/// The width the heads-up toolbar and the confirmation corner take at the right of the viewport,
/// so what sits at the left (the sketch strip) can leave room for them.
pub fn hud_reserved_width(viewport_width: f32) -> f32 {
    let hud = if viewport_width < HUD_COMPACT_BELOW {
        120.0
    } else {
        262.0
    };
    hud + 96.0
}

/// The heads-up view toolbar at the top-right of the viewport.
pub fn hud(app: &mut AppState, st: &mut UiState, ctx: &egui::Context, rect: Rect) {
    let colors = Colors::get(ctx);
    let compact = rect.width() < HUD_COMPACT_BELOW;
    // Make room for the confirmation corner when it is showing.
    let corner = app.sketching.is_some() || app.feature_draft.is_some();
    let right = rect.right() - 8.0 - if corner { 88.0 } else { 0.0 };
    egui::Area::new(egui::Id::new("hud"))
        .pivot(Align2::RIGHT_TOP)
        .fixed_pos(Pos2::new(right, rect.top() + 8.0))
        .order(egui::Order::Foreground)
        .show(ctx, |ui| {
            egui::Frame::NONE
                .fill(colors.ribbon.gamma_multiply(0.92))
                .stroke(Stroke::new(1.0, colors.line))
                .corner_radius(4.0)
                .inner_margin(egui::Margin::symmetric(4, 2))
                .show(ui, |ui| {
                    ui.horizontal(|ui| {
                        ui.spacing_mut().item_spacing.x = 2.0;
                        let mut cx = Cx {
                            app,
                            host: st,
                            colors,
                        };
                        let mut todo: Vec<&'static str> = Vec::new();
                        let direct: &[&str] = if compact {
                            &["view.fit"]
                        } else {
                            &[
                                "view.fit",
                                "view.iso",
                                "view.front",
                                "view.top",
                                "view.right",
                                "view.perspective",
                                "view.realview",
                            ]
                        };
                        for id in direct {
                            let id: &'static str = commands::find(id).map_or("", |c| c.id.as_str());
                            if icon_button(ui, &mut cx, id, 26.0) {
                                todo.push(id);
                            }
                        }
                        if compact {
                            // All the views in one drop-down.
                            hud_menu(
                                ui,
                                &mut cx,
                                "orientation",
                                Icon::Cad(CadIcon::ViewIso),
                                "View orientation",
                                &[
                                    "view.iso",
                                    "view.front",
                                    "view.back",
                                    "view.left",
                                    "view.right",
                                    "view.top",
                                    "view.bottom",
                                    "view.perspective",
                                    "view.realview",
                                ],
                            );
                        }
                        ui.separator();
                        hud_menu(
                            ui,
                            &mut cx,
                            "display",
                            Icon::Glyph(g::EYE),
                            "What the view shows",
                            &[
                                "view.origins",
                                "view.edges",
                                "view.sketches",
                                "view.sketch_grid",
                                "view.ambient_occlusion",
                                "view.floor_reflection",
                                "view.fxaa",
                            ],
                        );
                        for id in todo {
                            if let Err(e) = commands::run(id, cx.app, cx.host) {
                                cx.app.error(e, None);
                            }
                        }
                    });
                });
        });
}

/// An icon with a drop-arrow that opens a list of commands.
fn hud_menu(ui: &mut Ui, cx: &mut Cx, key: &str, icon: Icon, tip: &str, ids: &[&str]) {
    let (r, resp) = ui.allocate_exact_size(Vec2::new(34.0, 26.0), Sense::click());
    let open = egui::Popup::is_id_open(ui.ctx(), egui::Popup::default_response_id(&resp));
    if resp.hovered() || open {
        ui.painter().rect(
            r,
            3.0,
            if open {
                cx.colors.active
            } else {
                cx.colors.hover
            },
            Stroke::new(
                1.0,
                if open {
                    cx.colors.active_line
                } else {
                    cx.colors.hover_line
                },
            ),
            StrokeKind::Inside,
        );
    }
    icons::paint(
        ui.painter(),
        Rect::from_center_size(r.center() - Vec2::new(5.0, 0.0), Vec2::splat(16.0)),
        icon,
        &cx.colors,
        true,
    );
    ui.painter().text(
        r.center() + Vec2::new(9.0, 2.0),
        Align2::CENTER_CENTER,
        g::CARET_DOWN.to_string(),
        super::theme::icon_font(ui.ctx(), 9.0),
        cx.colors.text_weak,
    );
    let _ = key;
    resp.clone().on_hover_text(tip);
    let ids: Vec<&'static str> = ids
        .iter()
        .filter_map(|id| commands::find(id).map(|c| c.id.as_str()))
        .collect();
    egui::Popup::menu(&resp).show(|ui| {
        ui.set_min_width(200.0);
        for id in ids {
            if ribbon::menu_row(ui, cx, id) {
                if let Err(e) = commands::run(id, cx.app, cx.host) {
                    cx.app.error(e, None);
                }
            }
        }
    });
}

/// The confirmation corner: the way out of a sketch or a feature dialog, always in the same place.
pub fn confirmation_corner(app: &mut AppState, ctx: &egui::Context, rect: Rect) {
    let colors = Colors::get(ctx);
    let sketching = app.sketching.is_some();
    let feature = app.feature_draft.is_some();
    if !sketching && !feature {
        return;
    }
    let (mut ok, mut cancel) = (false, false);
    // A feature that cannot be built cannot be accepted.
    let problem = if feature { app.draft_problem() } else { None };
    let ok_enabled = problem.is_none();
    egui::Area::new(egui::Id::new("confirmation"))
        .fixed_pos(Pos2::new(rect.right() - 88.0, rect.top() + 8.0))
        .order(egui::Order::Foreground)
        .show(ctx, |ui| {
            egui::Frame::NONE
                .fill(colors.ribbon.gamma_multiply(0.95))
                .stroke(Stroke::new(1.0, colors.line))
                .corner_radius(4.0)
                .inner_margin(egui::Margin::same(3))
                .show(ui, |ui| {
                    ui.horizontal(|ui| {
                        ui.spacing_mut().item_spacing.x = 4.0;
                        let (r, resp) = ui.allocate_exact_size(Vec2::splat(30.0), Sense::click());
                        if resp.hovered() && ok_enabled {
                            ui.painter().rect_filled(r, 3.0, colors.hover);
                        }
                        if sketching {
                            icons::paint(
                                ui.painter(),
                                r.shrink(3.0),
                                Icon::Cad(CadIcon::ExitSketch),
                                &colors,
                                true,
                            );
                        } else {
                            ui.painter().text(
                                r.center(),
                                Align2::CENTER_CENTER,
                                g::CHECK.to_string(),
                                super::theme::icon_font(ui.ctx(), 22.0),
                                if ok_enabled {
                                    colors.ink_green
                                } else {
                                    colors.ink_grey
                                },
                            );
                        }
                        let tip = if sketching {
                            "Exit sketch".to_owned()
                        } else if let Some(p) = &problem {
                            format!("Cannot accept yet: {p}")
                        } else {
                            "OK (Enter)".to_owned()
                        };
                        if resp.on_hover_text(tip).clicked() && ok_enabled {
                            ok = true;
                        }
                        if feature {
                            let (r, resp) =
                                ui.allocate_exact_size(Vec2::splat(30.0), Sense::click());
                            if resp.hovered() {
                                ui.painter().rect_filled(r, 3.0, colors.hover);
                            }
                            ui.painter().text(
                                r.center(),
                                Align2::CENTER_CENTER,
                                g::X.to_string(),
                                super::theme::icon_font(ui.ctx(), 20.0),
                                colors.ink_red,
                            );
                            if resp.on_hover_text("Cancel (Esc)").clicked() {
                                cancel = true;
                            }
                        }
                    });
                });
        });
    if feature {
        if cancel {
            app.cancel_feature_draft();
        } else if ok {
            app.commit_feature_draft();
        }
    } else if ok {
        app.end_sketch();
    }
}

/// The status bar: the hint for what you are doing, the sketch state, and the document state.
pub fn status_bar(app: &AppState, ui: &mut Ui) {
    let colors = Colors::get(ui.ctx());
    ui.horizontal(|ui| {
        ui.add_space(6.0);
        // The right-hand items first; the message then takes what width is left, cut off with
        // an ellipsis rather than running into them.
        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            ui.add_space(6.0);
            ui.label(RichText::new("MMGS").color(colors.text_weak))
                .on_hover_text("millimetre, gram, second");
            ui.separator();
            ui.label(RichText::new(format!("{}", app.actor)).color(colors.text_weak));
            ui.separator();
            ui.label(
                RichText::new(format!(
                    "rev {}  ·  {}",
                    app.authority.revision().0,
                    super::short(app.authority.state_hash())
                ))
                .color(colors.text_weak),
            )
            .on_hover_text("the revision and the hash of the whole document state");
            if let Some(s) = &app.sketching {
                ui.separator();
                if let Some(c) = &s.cursor {
                    ui.label(format!("x {:.2}  y {:.2} mm", c.pos.0, c.pos.1));
                }
                let t: Tool = s.tool;
                ui.label(
                    RichText::new(format!("Editing sketch '{}' · {}", s.sketch, t.name()))
                        .color(colors.accent),
                );
                ui.separator();
            }
            ui.with_layout(Layout::left_to_right(Align::Center), |ui| {
                let hint: Option<String> = app
                    .sketching
                    .as_ref()
                    .map(|s| s.tool.hint().to_owned())
                    .or_else(|| {
                        app.hovered
                            .as_ref()
                            .map(|h| format!("{} › {}", h.body, h.face))
                    });
                let text = if let Some(m) = app.messages.last() {
                    let col = match m.level {
                        Level::Info => colors.text_weak,
                        Level::Error => colors.ink_red,
                    };
                    Some(RichText::new(&m.text).color(col))
                } else {
                    hint.map(|h| RichText::new(h).color(colors.text_weak))
                };
                if let Some(t) = text {
                    ui.add(egui::Label::new(t).truncate());
                }
            });
        });
    });
}

/// A sheet of every CAD icon, large, with its name (`--icon-sheet`), for reviewing the drawings.
pub fn icon_sheet(ui: &mut Ui) {
    let colors = Colors::get(ui.ctx());
    let per_row = ((ui.available_width() / 96.0) as usize).max(1);
    ui.add_space(8.0);
    egui::ScrollArea::vertical().show(ui, |ui| {
        for row in CadIcon::ALL.chunks(per_row) {
            ui.horizontal_top(|ui| {
                for icon in row {
                    ui.allocate_ui(Vec2::new(90.0, 118.0), |ui| {
                        ui.vertical(|ui| {
                            for size in [56.0, 24.0] {
                                let (rect, _) =
                                    ui.allocate_exact_size(Vec2::splat(size + 8.0), Sense::hover());
                                ui.painter().rect_filled(rect, 4.0, colors.paper);
                                ui.painter().rect_stroke(
                                    rect,
                                    4.0,
                                    Stroke::new(1.0, colors.line),
                                    StrokeKind::Inside,
                                );
                                icons::paint(
                                    ui.painter(),
                                    rect.shrink(4.0),
                                    Icon::Cad(*icon),
                                    &colors,
                                    true,
                                );
                            }
                            ui.label(RichText::new(format!("{icon:?}")).small());
                        });
                    });
                }
            });
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_palette_state_starts_fresh_with_nothing_selected() {
        let p = Palette::default();
        assert!(p.query.is_empty() && p.selected == 0);
    }
}
