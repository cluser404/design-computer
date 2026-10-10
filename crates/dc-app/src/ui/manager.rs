//! The left panel, as in SolidWorks: tabs for the FeatureManager (the design tree) and the
//! PropertyManager (the page of the command being set up), and under the tree the properties of
//! what is selected. A page (a feature being made, a sketch modify) takes over the panel while
//! it is open and has OK and Cancel at its top, always in the same place.

use super::icons::{self, CadIcon, Icon};
use super::theme::Colors;
use super::{ModifyKind, UiState};
use crate::commands;
use crate::featuring::{EndCondition, FeatureKind};
use crate::modify::ModifyOp;
use crate::state::{AppState, Selected};
use crate::tree::{self, Node, NodeKind};
use crate::ui::icon_font as g;
use dc_doc::FeaturePattern;
use eframe::egui::{
    self, collapsing_header::CollapsingState, Align, Align2, DragValue, FontId, Layout, Pos2, Rect,
    RichText, Sense, Stroke, StrokeKind, Ui, Vec2,
};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum LeftTab {
    #[default]
    Features,
    Properties,
}

/// Is a page (a command being set up) open?
pub fn page_active(app: &AppState, st: &UiState) -> bool {
    app.feature_draft.is_some() || st.modify.is_some()
}

/// Draws the whole left panel.
pub fn left_panel(app: &mut AppState, st: &mut UiState, ui: &mut Ui) {
    let colors = Colors::get(ui.ctx());
    // A page opening brings the PropertyManager forward; closing it brings the tree back.
    let active = page_active(app, st);
    if active != st.page_was_active {
        st.page_was_active = active;
        st.left_tab = if active {
            LeftTab::Properties
        } else {
            LeftTab::Features
        };
    }
    if !active {
        st.left_tab = LeftTab::Features;
    }
    tab_strip(st, ui, active, &colors);
    ui.add_space(2.0);
    match st.left_tab {
        LeftTab::Features => features_tab(app, st, ui, &colors),
        LeftTab::Properties => page(app, st, ui, &colors),
    }
}

fn tab_strip(st: &mut UiState, ui: &mut Ui, page: bool, colors: &Colors) {
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 2.0;
        for (tab, glyph, tip, enabled) in [
            (
                LeftTab::Features,
                g::TREE_STRUCTURE,
                "FeatureManager: the design tree",
                true,
            ),
            (
                LeftTab::Properties,
                g::SLIDERS_HORIZONTAL,
                "PropertyManager: the settings of the command in progress",
                page,
            ),
        ] {
            let (rect, resp) = ui.allocate_exact_size(Vec2::new(34.0, 26.0), Sense::click());
            let on = st.left_tab == tab;
            if on {
                ui.painter().rect(
                    rect,
                    3.0,
                    colors.active,
                    Stroke::new(1.0, colors.active_line),
                    StrokeKind::Inside,
                );
            } else if resp.hovered() && enabled {
                ui.painter().rect(
                    rect,
                    3.0,
                    colors.hover,
                    Stroke::new(1.0, colors.hover_line),
                    StrokeKind::Inside,
                );
            }
            icons::paint(
                ui.painter(),
                Rect::from_center_size(rect.center(), Vec2::splat(18.0)),
                Icon::Glyph(glyph),
                colors,
                enabled,
            );
            crate::devtools::tag(
                &format!("left.tab.{tab:?}").to_lowercase(),
                eguidev::WidgetRole::Selectable,
                tip,
                Some(on),
                &resp,
            );
            if resp.on_hover_text(tip).clicked() && enabled {
                st.left_tab = tab;
            }
        }
    });
}

// ---- FeatureManager ---------------------------------------------------------------------------

fn features_tab(app: &mut AppState, st: &mut UiState, ui: &mut Ui, colors: &Colors) {
    // Filter box.
    ui.horizontal(|ui| {
        ui.label(RichText::new(g::FUNNEL.to_string()).color(colors.text_weak));
        ui.add(
            egui::TextEdit::singleline(&mut st.tree_filter)
                .hint_text("Filter the tree")
                .desired_width(f32::INFINITY),
        );
    });
    ui.add_space(2.0);
    if st.renaming.is_none()
        && ui.ctx().input(|i| i.key_pressed(egui::Key::F2))
        && !ui.ctx().egui_wants_keyboard_input()
    {
        match &app.selected {
            Selected::Feature(f) => {
                let l = app.authority.state().label("feature", f);
                st.renaming = Some((true, f.clone(), l.to_owned()));
            }
            Selected::Sketch(s) => {
                let l = app.authority.state().label("sketch", s);
                st.renaming = Some((false, s.clone(), l.to_owned()));
            }
            _ => {}
        }
    }
    // The tree takes the upper part, the properties of the selection the rest.
    let avail = ui.available_height();
    let tree_h = (avail * 0.58).max(120.0);
    let root = tree::build(app, &st.tree_filter);
    egui::ScrollArea::vertical()
        .id_salt("tree_scroll")
        .max_height(tree_h)
        .auto_shrink([false, true])
        .show(ui, |ui| {
            show_node(app, st, ui, &root, colors);
        });
    ui.separator();
    ui.label(RichText::new("Selection").small().color(colors.text_weak));
    egui::ScrollArea::vertical()
        .id_salt("selection_scroll")
        .auto_shrink([false, false])
        .show(ui, |ui| match app.selected.clone() {
            Selected::None => {
                ui.label(format!(
                    "{} feature(s), {} sketch(es)",
                    app.feature_rows().len(),
                    app.sketch_rows().len()
                ));
                ui.weak("Click a face in the view, or an item in the tree.");
                ui.weak("Drag to orbit · middle drag to pan · scroll to zoom");
            }
            Selected::Feature(f) => super::feature_panel(app, ui, &f),
            Selected::Sketch(s) => super::sketch_panel(app, ui, &s),
            Selected::Face { feature, face } => super::face_panel(app, ui, &feature, &face),
            Selected::Edge { feature, edge } => super::edge_panel(app, ui, &feature, &edge),
            Selected::SketchEntity { sketch, entity } => {
                super::sketch_entity_panel(app, ui, &sketch, &entity)
            }
        });
}

/// What a click on a tree row asks for, collected while drawing and done afterwards.
enum RowAction {
    Select(Selected),
    Clear,
    Edit(String),
    NewSketch(&'static str),
    Delete,
    EditFeature(String),
    Rename(bool, String),
    Suppress(String, bool),
    Rollback(Option<String>),
    ToFolder(String, Option<String>),
}

fn show_node(app: &mut AppState, st: &mut UiState, ui: &mut Ui, node: &Node, colors: &Colors) {
    let selected = node.is_selected(&app.selected)
        || (matches!(node.kind, NodeKind::Plane(_) | NodeKind::Origin)
            && st.tree_pick.as_deref() == Some(node.key().as_str()));
    let mut actions: Vec<RowAction> = Vec::new();
    if node.children.is_empty() {
        row(app, st, ui, node, selected, colors, &mut actions);
    } else {
        let id = ui.make_persistent_id(node.key());
        let open_by_default = matches!(node.kind, NodeKind::Root)
            || node.is_selected(&app.selected)
            || !st.tree_filter.is_empty();
        let state = CollapsingState::load_with_default_open(ui.ctx(), id, open_by_default);
        state
            .show_header(ui, |ui| {
                row(app, st, ui, node, selected, colors, &mut actions)
            })
            .body(|ui| {
                for c in &node.children {
                    show_node(app, st, ui, c, colors);
                }
            });
    }
    for a in actions {
        match a {
            RowAction::Select(s) => {
                st.tree_pick = None;
                app.select(s);
            }
            RowAction::Clear => {
                st.tree_pick = None;
                app.select(Selected::None);
            }
            RowAction::Edit(sk) => {
                app.edit_sketch(&sk);
            }
            RowAction::NewSketch(id) => {
                let _ = commands::run(id, app, st).map_err(|e| app.error(e, None));
            }
            RowAction::Delete => {
                let _ = app.delete_selected();
            }
            RowAction::EditFeature(f) => {
                app.begin_edit_feature(&f);
            }
            RowAction::Rename(is_feature, id) => {
                let current = app
                    .authority
                    .state()
                    .label(if is_feature { "feature" } else { "sketch" }, &id);
                st.renaming = Some((is_feature, id.clone(), current.to_owned()));
            }
            RowAction::Suppress(f, on) => {
                app.set_suppressed(&f, on);
            }
            RowAction::Rollback(to) => {
                app.set_rollback(to.as_deref());
            }
            RowAction::ToFolder(f, folder) => {
                app.move_to_folder(&f, folder.as_deref());
            }
        }
    }
}

fn row(
    app: &mut AppState,
    st: &mut UiState,
    ui: &mut Ui,
    node: &Node,
    selected: bool,
    colors: &Colors,
    actions: &mut Vec<RowAction>,
) {
    let h = 22.0;
    let (rect, resp) =
        ui.allocate_exact_size(Vec2::new(ui.available_width(), h), Sense::click_and_drag());
    // Features can be dragged onto a folder (or the document row, to take them out of one).
    if let NodeKind::Feature(f) = &node.kind {
        if resp.drag_started() {
            resp.dnd_set_drag_payload(f.clone());
        }
    }
    if let Some(target) = node.drop_folder() {
        if resp.dnd_hover_payload::<String>().is_some() {
            ui.painter().rect_stroke(
                rect,
                3.0,
                Stroke::new(1.5, colors.accent),
                StrokeKind::Inside,
            );
        }
        if let Some(f) = resp.dnd_release_payload::<String>() {
            actions.push(RowAction::ToFolder((*f).clone(), target));
        }
    }
    if selected {
        ui.painter().rect(
            rect,
            3.0,
            colors.active,
            Stroke::new(1.0, colors.active_line),
            StrokeKind::Inside,
        );
    } else if resp.hovered() {
        ui.painter().rect_filled(rect, 3.0, colors.hover);
    }
    let icon = Rect::from_center_size(
        Pos2::new(rect.left() + 10.0, rect.center().y),
        Vec2::splat(16.0),
    );
    icons::paint(ui.painter(), icon, node.icon, colors, true);
    let text_color = if node.off.is_some() {
        colors.text_weak
    } else {
        colors.text
    };
    crate::devtools::tag(
        &format!("tree.{}", node.key()),
        eguidev::WidgetRole::Selectable,
        &node.label,
        Some(selected),
        &resp,
    );
    let rename_key = match &node.kind {
        NodeKind::Feature(f) => Some((true, f.clone())),
        NodeKind::Sketch(s) => Some((false, s.clone())),
        _ => None,
    };
    let renaming_here = st
        .renaming
        .as_ref()
        .is_some_and(|(k, id, _)| rename_key.as_ref() == Some(&(*k, id.clone())));
    let mut label_w = 0.0;
    if renaming_here {
        let mut done = None;
        if let Some((_, _, text)) = &mut st.renaming {
            let edit_rect = Rect::from_min_size(
                Pos2::new(rect.left() + 22.0, rect.top() + 1.0),
                Vec2::new((rect.width() - 26.0).max(60.0), h - 2.0),
            );
            let r = ui.put(edit_rect, egui::TextEdit::singleline(text));
            r.request_focus();
            if ui.input(|i| i.key_pressed(egui::Key::Escape)) {
                done = Some(false);
            } else if r.lost_focus() || ui.input(|i| i.key_pressed(egui::Key::Enter)) {
                done = Some(true);
            }
        }
        if let Some(commit) = done {
            if let Some((is_feature, id, text)) = st.renaming.take() {
                if commit {
                    app.rename(is_feature, &id, &text);
                }
            }
        }
    } else {
        let label =
            ui.painter()
                .layout_no_wrap(node.label.clone(), FontId::proportional(13.0), text_color);
        label_w = label.size().x;
        ui.painter().galley(
            Pos2::new(rect.left() + 24.0, rect.center().y - label.size().y / 2.0),
            label,
            text_color,
        );
    }
    if node.bar_after {
        ui.painter().line_segment(
            [
                Pos2::new(rect.left(), rect.bottom()),
                Pos2::new(rect.right(), rect.bottom()),
            ],
            Stroke::new(3.0, colors.ink_orange),
        );
    }
    if let (Some(why), false) = (node.off, renaming_here) {
        ui.painter().text(
            Pos2::new(rect.left() + 24.0 + label_w + 8.0, rect.center().y),
            Align2::LEFT_CENTER,
            why,
            FontId::proportional(11.0),
            colors.ink_orange,
        );
    } else if let (Some(n), false) = (&node.note, renaming_here) {
        ui.painter().text(
            Pos2::new(rect.left() + 24.0 + label_w + 8.0, rect.center().y),
            Align2::LEFT_CENTER,
            n,
            FontId::proportional(11.0),
            if node.warn {
                colors.ink_orange
            } else {
                colors.text_weak
            },
        );
    }
    // Click selects, double click opens or starts, right click offers the usual things.
    if resp.double_clicked() {
        match &node.kind {
            NodeKind::Feature(f) => actions.push(RowAction::EditFeature(f.clone())),
            NodeKind::Sketch(s) => actions.push(RowAction::Edit(s.clone())),
            NodeKind::Plane(p) => actions.push(RowAction::NewSketch(plane_command(p))),
            _ => {}
        }
    } else if resp.clicked() {
        match &node.kind {
            NodeKind::Feature(f) => actions.push(RowAction::Select(Selected::Feature(f.clone()))),
            NodeKind::Sketch(s) => actions.push(RowAction::Select(Selected::Sketch(s.clone()))),
            NodeKind::Face { feature, face } => actions.push(RowAction::Select(Selected::Face {
                feature: feature.clone(),
                face: face.clone(),
            })),
            NodeKind::Plane(_) | NodeKind::Origin => st.tree_pick = Some(node.key()),
            NodeKind::Root => actions.push(RowAction::Clear),
            NodeKind::Folder(_) => {}
        }
    }
    let kind = node.kind.clone();
    let off = node.off;
    let is_suppressed = match &kind {
        NodeKind::Feature(f) => app.authority.state().suppressed().contains(f),
        _ => false,
    };
    let has_bar = app.authority.state().rollback().is_some();
    let folder_names: Vec<String> = app
        .authority
        .state()
        .folders()
        .into_iter()
        .map(|(n, _)| n)
        .collect();
    let new_folder = app.free_folder_name();
    resp.context_menu(|ui| {
        let mut close = false;
        match &kind {
            NodeKind::Feature(f) => {
                if off.is_none() && ui.button("Edit feature").clicked() {
                    actions.push(RowAction::EditFeature(f.clone()));
                    close = true;
                }
                if ui.button("Rename  (F2)").clicked() {
                    actions.push(RowAction::Rename(true, f.clone()));
                    close = true;
                }
                if is_suppressed {
                    if ui.button("Unsuppress").clicked() {
                        actions.push(RowAction::Suppress(f.clone(), false));
                        close = true;
                    }
                } else if ui.button("Suppress").clicked() {
                    actions.push(RowAction::Suppress(f.clone(), true));
                    close = true;
                }
                if ui.button("Roll back to here").clicked() {
                    actions.push(RowAction::Rollback(Some(f.clone())));
                    close = true;
                }
                if has_bar && ui.button("Roll forward to the end").clicked() {
                    actions.push(RowAction::Rollback(None));
                    close = true;
                }
                ui.menu_button("Move to folder", |ui| {
                    for name in &folder_names {
                        if ui.button(name).clicked() {
                            actions.push(RowAction::ToFolder(f.clone(), Some(name.clone())));
                            close = true;
                        }
                    }
                    if ui.button("New folder").clicked() {
                        actions.push(RowAction::ToFolder(f.clone(), Some(new_folder.clone())));
                        close = true;
                    }
                    if ui.button("Out of its folder").clicked() {
                        actions.push(RowAction::ToFolder(f.clone(), None));
                        close = true;
                    }
                });
                ui.separator();
                if ui.button("Delete").clicked() {
                    actions.push(RowAction::Select(Selected::Feature(f.clone())));
                    actions.push(RowAction::Delete);
                    close = true;
                }
            }
            NodeKind::Folder(_) => {
                ui.weak("Use \"Move to folder\" on a feature");
            }
            NodeKind::Sketch(s) => {
                if ui.button("Edit sketch").clicked() {
                    actions.push(RowAction::Edit(s.clone()));
                    close = true;
                }
                if ui.button("Rename  (F2)").clicked() {
                    actions.push(RowAction::Rename(false, s.clone()));
                    close = true;
                }
                if ui.button("Delete").clicked() {
                    actions.push(RowAction::Select(Selected::Sketch(s.clone())));
                    actions.push(RowAction::Delete);
                    close = true;
                }
            }
            NodeKind::Plane(p) => {
                if ui.button("Sketch on this plane").clicked() {
                    actions.push(RowAction::NewSketch(plane_command(p)));
                    close = true;
                }
            }
            NodeKind::Face { feature, face } => {
                if ui.button("Sketch on this face").clicked() {
                    actions.push(RowAction::Select(Selected::Face {
                        feature: feature.clone(),
                        face: face.clone(),
                    }));
                    actions.push(RowAction::NewSketch("sketch.on_face"));
                    close = true;
                }
            }
            NodeKind::Origin | NodeKind::Root => {
                ui.weak("nothing to do here");
            }
        }
        if close {
            ui.close();
        }
    });
    let _ = app;
}

fn plane_command(p: &dc_sketch::PlaneSpec) -> &'static str {
    match p {
        dc_sketch::PlaneSpec::Xz { .. } => "sketch.new_xz",
        dc_sketch::PlaneSpec::Yz { .. } => "sketch.new_yz",
        _ => "sketch.new_xy",
    }
}

// ---- PropertyManager pages --------------------------------------------------------------------

/// The page header: icon and title, with the accept and cancel buttons at the right.
fn header(
    ui: &mut Ui,
    colors: &Colors,
    icon: Icon,
    title: &str,
    ok_enabled: bool,
    ok_tip: &str,
) -> (bool, bool) {
    let (mut ok, mut cancel) = (false, false);
    ui.horizontal(|ui| {
        let (r, _) = ui.allocate_exact_size(Vec2::splat(22.0), Sense::hover());
        icons::paint(ui.painter(), r, icon, colors, true);
        ui.label(RichText::new(title).strong().size(15.0));
        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            let (r, resp) = ui.allocate_exact_size(Vec2::splat(28.0), Sense::click());
            if resp.hovered() {
                ui.painter().rect(
                    r,
                    3.0,
                    colors.hover,
                    Stroke::new(1.0, colors.hover_line),
                    StrokeKind::Inside,
                );
            }
            ui.painter().text(
                r.center(),
                Align2::CENTER_CENTER,
                g::X.to_string(),
                super::theme::icon_font(ui.ctx(), 18.0),
                colors.ink_red,
            );
            crate::devtools::tag(
                "page.cancel",
                eguidev::WidgetRole::Button,
                "Cancel",
                None,
                &resp,
            );
            if resp.on_hover_text("Cancel (Esc)").clicked() {
                cancel = true;
            }
            let (r, resp) = ui.allocate_exact_size(Vec2::splat(28.0), Sense::click());
            if resp.hovered() && ok_enabled {
                ui.painter().rect(
                    r,
                    3.0,
                    colors.hover,
                    Stroke::new(1.0, colors.hover_line),
                    StrokeKind::Inside,
                );
            }
            ui.painter().text(
                r.center(),
                Align2::CENTER_CENTER,
                g::CHECK.to_string(),
                super::theme::icon_font(ui.ctx(), 20.0),
                if ok_enabled {
                    colors.ink_green
                } else {
                    colors.ink_grey
                },
            );
            crate::devtools::tag(
                "page.ok",
                eguidev::WidgetRole::Button,
                "OK",
                Some(ok_enabled),
                &resp,
            );
            if resp.on_hover_text(ok_tip).clicked() && ok_enabled {
                ok = true;
            }
        });
    });
    ui.separator();
    (ok, cancel)
}

/// A titled group of a page, which can be folded.
fn group<R>(ui: &mut Ui, title: &str, open: bool, add: impl FnOnce(&mut Ui) -> R) {
    egui::CollapsingHeader::new(RichText::new(title).strong())
        .default_open(open)
        .show(ui, |ui| {
            add(ui);
        });
}

fn page(app: &mut AppState, st: &mut UiState, ui: &mut Ui, colors: &Colors) {
    if app.feature_draft.is_some() {
        feature_page(app, ui, colors);
    } else if st.modify.is_some() {
        modify_page(app, st, ui, colors);
    } else {
        ui.weak("Nothing is being set up. Start a command from the ribbon.");
    }
}

fn feature_page(app: &mut AppState, ui: &mut Ui, colors: &Colors) {
    let Some(mut d) = app.feature_draft.clone() else {
        return;
    };
    let before = d.clone();
    let sketches = app.extrudable_sketches();
    let profiles = app.profiles_of(&d.sketch);
    let targets = app.feature_targets();
    let problem = app.draft_problem();
    let fix = app.draft_fix();
    let axes = app.axes_of(&d.sketch);
    let second_sketches = if matches!(d.kind, FeatureKind::Loft | FeatureKind::Sweep) {
        app.second_sketch_choices(d.kind, &d.sketch)
    } else {
        Vec::new()
    };
    let profiles2 = if d.kind == FeatureKind::Loft {
        app.profiles_of(&d.sketch2)
    } else {
        Vec::new()
    };
    let (ok, cancel) = header(
        ui,
        colors,
        Icon::Cad(match d.kind {
            FeatureKind::Extrude => CadIcon::ExtrudedBoss,
            FeatureKind::Cut => CadIcon::ExtrudedCut,
            FeatureKind::Boss => CadIcon::Boss,
            FeatureKind::Revolve => CadIcon::Rotate,
            FeatureKind::Fillet => CadIcon::Fillet,
            FeatureKind::Chamfer => CadIcon::Chamfer,
            FeatureKind::Shell => CadIcon::Shell,
            FeatureKind::Loft => CadIcon::Loft,
            FeatureKind::Sweep => CadIcon::Sweep,
        }),
        &if d.editing {
            format!("Edit {}", d.kind.name())
        } else {
            d.kind.name().to_owned()
        },
        problem.is_none(),
        "OK (Enter)",
    );
    let mut new_sketch = None;
    let mut apply_fix = None;
    let from_sketch = d.kind.needs_sketch();
    let has_depth = matches!(
        d.kind,
        FeatureKind::Extrude | FeatureKind::Cut | FeatureKind::Boss
    );
    egui::ScrollArea::vertical()
        .auto_shrink([false, false])
        .show(ui, |ui| {
            ui.label(RichText::new(d.kind.hint()).small().color(colors.text_weak));
            ui.add_space(4.0);
            group(ui, "Name", true, |ui| {
                ui.add_enabled(
                    !d.editing,
                    egui::TextEdit::singleline(&mut d.name).desired_width(f32::INFINITY),
                );
            });
            group(ui, "From", true, |ui| {
                egui::Grid::new("feature_from")
                    .num_columns(2)
                    .spacing([8.0, 6.0])
                    .show(ui, |ui| {
                        if from_sketch {
                            ui.label("Sketch");
                            egui::ComboBox::from_id_salt("feature_sketch")
                                .selected_text(&d.sketch)
                                .show_ui(ui, |ui| {
                                    for s in &sketches {
                                        if ui.selectable_label(d.sketch == *s, s).clicked() {
                                            new_sketch = Some(s.clone());
                                        }
                                    }
                                });
                            ui.end_row();
                        }
                        if matches!(d.kind, FeatureKind::Loft | FeatureKind::Sweep) {
                            ui.label(if d.kind == FeatureKind::Loft {
                                "To sketch"
                            } else {
                                "Path sketch"
                            });
                            let r = egui::ComboBox::from_id_salt("feature_sketch2")
                                .selected_text(&d.sketch2)
                                .show_ui(ui, |ui| {
                                    for s in &second_sketches {
                                        if ui.selectable_label(d.sketch2 == *s, s).clicked() {
                                            d.sketch2 = s.clone();
                                            d.profiles2.clear();
                                            d.path = app
                                                .path_lines(s)
                                                .first()
                                                .cloned()
                                                .unwrap_or_default();
                                        }
                                    }
                                });
                            crate::devtools::tag(
                                "page.sketch2",
                                eguidev::WidgetRole::ComboBox,
                                "Second sketch",
                                None,
                                &r.response,
                            );
                            ui.end_row();
                        }
                        if d.kind == FeatureKind::Sweep {
                            ui.label("Path line");
                            let lines = app.path_lines(&d.sketch2);
                            egui::ComboBox::from_id_salt("feature_path")
                                .selected_text(&d.path)
                                .show_ui(ui, |ui| {
                                    for l in &lines {
                                        ui.selectable_value(&mut d.path, l.clone(), l);
                                    }
                                });
                            ui.end_row();
                        }
                        if !matches!(
                            d.kind,
                            FeatureKind::Extrude
                                | FeatureKind::Revolve
                                | FeatureKind::Loft
                                | FeatureKind::Sweep
                        ) {
                            ui.label("Part");
                            egui::ComboBox::from_id_salt("feature_target")
                                .selected_text(d.target.clone().unwrap_or_else(|| "none".into()))
                                .show_ui(ui, |ui| {
                                    for t in &targets {
                                        ui.selectable_value(&mut d.target, Some(t.clone()), t);
                                    }
                                });
                            ui.end_row();
                        }
                    });
            });
            if d.kind == FeatureKind::Shell {
                group(ui, "Faces to leave open", true, |ui| {
                    let ends = d
                        .target
                        .as_deref()
                        .and_then(|t| app.shell_end_faces(t));
                    match ends {
                        Some((a, b)) => {
                            for name in [a, b] {
                                let mut on = d.faces.contains(&name);
                                let r = ui.checkbox(&mut on, &name);
                                crate::devtools::tag(
                                    &format!("page.face.{name}"),
                                    eguidev::WidgetRole::Checkbox,
                                    &name,
                                    Some(on),
                                    &r,
                                );
                                if r.changed() {
                                    if on {
                                        d.faces.push(name.clone());
                                    } else {
                                        d.faces.retain(|f| *f != name);
                                    }
                                }
                            }
                        }
                        None => {
                            ui.label(
                                RichText::new(
                                    "This part is not a prism (two flat ends with only square faces between), so it cannot be shelled yet.",
                                )
                                .small()
                                .color(colors.ink_red),
                            );
                        }
                    }
                    ui.horizontal(|ui| {
                        ui.label("Thickness");
                        if let Some(v) = crate::devtools::number_override(ui, "page.thickness") {
                            d.size = v;
                        }
                        let r = ui.add(
                            DragValue::new(&mut d.size)
                                .speed(0.1)
                                .range(0.001..=100_000.0)
                                .suffix(" mm"),
                        );
                        crate::devtools::tag_number("page.thickness", "Thickness", d.size, &r);
                    });
                });
            }
            if d.kind.rounds_edges() {
                group(ui, "Edges", true, |ui| {
                    ui.label(
                        RichText::new(
                            "Click edges of the part in the view; click an edge again to take it out.",
                        )
                        .small()
                        .color(colors.text_weak),
                    );
                    let mut remove = None;
                    for (i, [a, b]) in d.edges.iter().enumerate() {
                        ui.horizontal(|ui| {
                            ui.label(format!("{a}  |  {b}"));
                            if ui.small_button("x").on_hover_text("Remove this edge").clicked() {
                                remove = Some(i);
                            }
                        });
                    }
                    if let Some(i) = remove {
                        d.edges.remove(i);
                    }
                    ui.horizontal(|ui| {
                        ui.label(if d.kind == FeatureKind::Fillet {
                            "Radius"
                        } else {
                            "Distance"
                        });
                        ui.add(
                            DragValue::new(&mut d.size)
                                .speed(0.1)
                                .range(0.001..=100_000.0)
                                .suffix(" mm"),
                        );
                    });
                });
            }
            if d.kind == FeatureKind::Loft {
                group(ui, "Second outline", true, |ui| {
                    let mut all = d.profiles2.is_empty();
                    if ui
                        .checkbox(&mut all, format!("Every closed region ({})", profiles2.len()))
                        .changed()
                        && all
                    {
                        d.profiles2.clear();
                    }
                    for p in &profiles2 {
                        let mut on = d.profiles2.contains(&p.id);
                        if ui.checkbox(&mut on, &p.id).changed() {
                            if on {
                                d.profiles2.push(p.id.clone());
                            } else {
                                d.profiles2.retain(|x| *x != p.id);
                            }
                        }
                    }
                });
            }
            if d.kind == FeatureKind::Revolve {
                group(ui, "Axis of revolution", true, |ui| {
                    egui::ComboBox::from_id_salt("feature_axis")
                        .selected_text(
                            axes.iter()
                                .find(|(id, _)| *id == d.axis)
                                .map_or(d.axis.clone(), |(_, n)| n.clone()),
                        )
                        .width(ui.available_width() - 8.0)
                        .show_ui(ui, |ui| {
                            for (id, name) in &axes {
                                ui.selectable_value(&mut d.axis, id.clone(), name);
                            }
                        });
                });
                group(ui, "Angle", true, |ui| {
                    if let Some(v) = crate::devtools::number_override(ui, "page.angle") {
                        d.angle = v;
                    }
                    let r = ui.add(
                        DragValue::new(&mut d.angle)
                            .speed(1.0)
                            .range(1.0..=360.0)
                            .suffix(" °"),
                    );
                    crate::devtools::tag_number("page.angle", "Angle", d.angle, &r);
                    ui.label(
                        RichText::new("360° is a full turn; less starts in the sketch plane")
                            .small()
                            .color(colors.text_weak),
                    );
                });
            }
            if from_sketch {
            group(ui, "Selected contours", true, |ui| {
                let label_of = |p: &dc_doc::query::ProfileInfo| {
                    format!(
                        "{}  ({:.1} mm², {} hole{})",
                        p.id,
                        p.area,
                        p.holes.len(),
                        if p.holes.len() == 1 { "" } else { "s" }
                    )
                };
                let one = d.kind == FeatureKind::Revolve;
                if !one {
                    let mut all = d.profiles.is_empty();
                    if ui
                        .checkbox(&mut all, format!("Every closed region ({})", profiles.len()))
                        .on_hover_text("Use all the closed shapes of the sketch")
                        .changed()
                        && all
                    {
                        d.profiles.clear();
                    }
                }
                ui.label(
                    RichText::new(if one {
                        "Click a region of the sketch in the view, or choose one:"
                    } else {
                        "Click regions of the sketch in the view to add or remove them:"
                    })
                    .small()
                    .color(colors.text_weak),
                );
                egui::ScrollArea::vertical()
                    .id_salt("contours")
                    .max_height(150.0)
                    .show(ui, |ui| {
                        for p in &profiles {
                            let on = d.profiles.is_empty() && !one || d.profiles.contains(&p.id);
                            let text = label_of(p);
                            if one {
                                let r = ui.radio(on, text);
                                crate::devtools::tag(
                                    &format!("page.profile.{}", p.id),
                                    eguidev::WidgetRole::Radio,
                                    &p.id,
                                    Some(on),
                                    &r,
                                );
                                if r.clicked() {
                                    d.profiles = vec![p.id.clone()];
                                }
                            } else {
                                let mut v = on;
                                let r = ui.checkbox(&mut v, text);
                                crate::devtools::tag(
                                    &format!("page.profile.{}", p.id),
                                    eguidev::WidgetRole::Checkbox,
                                    &p.id,
                                    Some(on),
                                    &r,
                                );
                                if r.changed() {
                                    if d.profiles.is_empty() {
                                        // Narrowing "every region": start from all but this one.
                                        d.profiles = profiles
                                            .iter()
                                            .filter(|q| q.id != p.id)
                                            .map(|q| q.id.clone())
                                            .collect();
                                    } else if v {
                                        d.profiles.push(p.id.clone());
                                    } else if d.profiles.len() > 1 {
                                        // At least one region stays: none would read as "all".
                                        d.profiles.retain(|x| *x != p.id);
                                    }
                                    if d.profiles.len() == profiles.len() {
                                        d.profiles.clear();
                                    }
                                }
                            }
                        }
                    });
            });
            }
            if has_depth {
            group(ui, "Direction 1", true, |ui| {
                egui::Grid::new("feature_dir1")
                    .num_columns(2)
                    .spacing([8.0, 6.0])
                    .show(ui, |ui| {
                        ui.label("End condition");
                        egui::ComboBox::from_id_salt("feature_end")
                            .selected_text(d.end.name())
                            .show_ui(ui, |ui| {
                                for e in EndCondition::for_kind(d.kind) {
                                    ui.selectable_value(&mut d.end, *e, e.name());
                                }
                            });
                        ui.end_row();
                        if d.end.has_depth() {
                            ui.label(if d.end == EndCondition::MidPlane {
                                "Total depth"
                            } else {
                                "Depth"
                            });
                            ui.add(
                                DragValue::new(&mut d.depth)
                                    .speed(0.2)
                                    .range(0.001..=100_000.0)
                                    .suffix(" mm"),
                            );
                            ui.end_row();
                        }
                        if d.end != EndCondition::ThroughBoth && d.end != EndCondition::MidPlane {
                            ui.label("");
                            let r = ui
                                .checkbox(&mut d.reverse, "Reverse direction")
                                .on_hover_text("Go the other way from the sketch plane");
                            crate::devtools::tag(
                                "page.reverse",
                                eguidev::WidgetRole::Checkbox,
                                "Reverse direction",
                                Some(d.reverse),
                                &r,
                            );
                            ui.end_row();
                        }
                    });
            });
            if d.end != EndCondition::ThroughBoth && d.end != EndCondition::MidPlane {
                group(ui, "Direction 2", d.second, |ui| {
                    let r = ui
                        .checkbox(&mut d.second, "Also the other way")
                        .on_hover_text("A second extent against the first, with its own depth");
                    crate::devtools::tag(
                        "page.second",
                        eguidev::WidgetRole::Checkbox,
                        "Also the other way",
                        Some(d.second),
                        &r,
                    );
                    if d.second {
                        ui.horizontal(|ui| {
                            ui.label("Depth");
                            ui.add(
                                DragValue::new(&mut d.depth2)
                                    .speed(0.2)
                                    .range(0.001..=100_000.0)
                                    .suffix(" mm"),
                            );
                        });
                    }
                });
            }
            }
            if matches!(d.kind, FeatureKind::Cut | FeatureKind::Boss) {
                group(ui, "Pattern", d.pattern.is_some(), |ui| {
                    pattern_editor(ui, &mut d.pattern);
                });
            }
            if d.kind == FeatureKind::Cut {
                group(ui, "Options", true, |ui| {
                    let r = ui.checkbox(&mut d.outside, "Flip side to cut")
                        .on_hover_text(
                            "Remove what is outside the profile and keep only what is inside it",
                        );
                    crate::devtools::tag(
                        "page.outside",
                        eguidev::WidgetRole::Checkbox,
                        "Flip side to cut",
                        Some(d.outside),
                        &r,
                    );
                });
            }
            if let Some(p) = &problem {
                ui.add_space(6.0);
                ui.colored_label(colors.ink_red, p);
                if let Some(f) = fix {
                    let r = ui.button(f.label());
                    crate::devtools::tag(
                        "page.fix",
                        eguidev::WidgetRole::Button,
                        f.label(),
                        None,
                        &r,
                    );
                    if r.clicked() {
                        apply_fix = Some(f);
                    }
                }
            }
        });
    if let Some(f) = apply_fix {
        app.feature_draft = Some(d);
        app.apply_draft_fix(f);
        return;
    }
    if let Some(s) = new_sketch {
        app.feature_draft = Some(d);
        app.draft_set_sketch(&s);
    } else if d != before {
        app.feature_draft = Some(d);
    }
    let enter = ui.ctx().input(|i| i.key_pressed(egui::Key::Enter))
        && !ui.ctx().egui_wants_keyboard_input();
    if cancel || ui.ctx().input(|i| i.key_pressed(egui::Key::Escape)) {
        app.cancel_feature_draft();
    } else if ok || (enter && problem.is_none()) {
        app.commit_feature_draft();
    } else {
        app.update_feature_preview();
    }
}

fn modify_page(app: &mut AppState, st: &mut UiState, ui: &mut Ui, colors: &Colors) {
    let Some(mut panel) = st.modify.clone() else {
        return;
    };
    let selected = app.selected_entities().len();
    let mut apply: Option<ModifyOp> = None;
    let ready = panel.op().is_some();
    let (ok, close) = header(
        ui,
        colors,
        Icon::Cad(CadIcon::Move),
        "Modify",
        ready,
        "Apply to the selection",
    );
    egui::ScrollArea::vertical()
        .auto_shrink([false, false])
        .show(ui, |ui| {
            group(ui, "Operation", true, |ui| {
                egui::ComboBox::from_id_salt("modify_kind")
                    .selected_text(panel.kind.name())
                    .width(ui.available_width() - 8.0)
                    .show_ui(ui, |ui| {
                        for k in ModifyKind::ALL {
                            if ui.selectable_label(panel.kind == k, k.name()).clicked() {
                                let c = app.selection_centre();
                                panel.kind = k;
                                panel.cx = c.0;
                                panel.cy = c.1;
                            }
                        }
                    });
                ui.label(RichText::new(format!("{selected} selected")).color(colors.text_weak));
            });
            group(ui, "Parameters", true, |ui| {
                let num = |ui: &mut Ui, label: &str, v: &mut f64| {
                    ui.horizontal(|ui| {
                        ui.label(label);
                        ui.add(DragValue::new(v).speed(0.5));
                    });
                };
                match panel.kind {
                    ModifyKind::Move => {
                        num(ui, "dx", &mut panel.dx);
                        num(ui, "dy", &mut panel.dy);
                        ui.checkbox(&mut panel.copy, "Copy (leave the original)");
                    }
                    ModifyKind::Rotate => {
                        num(ui, "centre x", &mut panel.cx);
                        num(ui, "centre y", &mut panel.cy);
                        num(ui, "degrees", &mut panel.degrees);
                        ui.checkbox(&mut panel.copy, "Copy");
                    }
                    ModifyKind::Scale => {
                        num(ui, "centre x", &mut panel.cx);
                        num(ui, "centre y", &mut panel.cy);
                        num(ui, "factor", &mut panel.factor);
                        ui.checkbox(&mut panel.copy, "Copy");
                    }
                    ModifyKind::Mirror => {
                        ui.label("Mirrored copies, kept symmetric to the originals.");
                        if ui.button("About the horizontal axis").clicked() {
                            apply = Some(ModifyOp::MirrorAxisX);
                        }
                        if ui.button("About the vertical axis").clicked() {
                            apply = Some(ModifyOp::MirrorAxisY);
                        }
                        if ui
                            .button("About the line selected last")
                            .on_hover_text("Select what to mirror, then the mirror line")
                            .clicked()
                        {
                            apply = Some(ModifyOp::MirrorSelectedLine);
                        }
                    }
                    ModifyKind::LinearPattern => {
                        num(ui, "dx", &mut panel.dx);
                        num(ui, "dy", &mut panel.dy);
                        ui.horizontal(|ui| {
                            ui.label("instances");
                            ui.add(DragValue::new(&mut panel.count).range(2..=200));
                        });
                    }
                    ModifyKind::CircularPattern => {
                        num(ui, "centre x", &mut panel.cx);
                        num(ui, "centre y", &mut panel.cy);
                        num(ui, "degrees (360 = full turn)", &mut panel.degrees);
                        ui.horizontal(|ui| {
                            ui.label("instances");
                            ui.add(DragValue::new(&mut panel.count).range(2..=200));
                        });
                    }
                    ModifyKind::Offset => {
                        num(
                            ui,
                            "distance (negative = the other side)",
                            &mut panel.distance,
                        );
                        ui.label("One chain of lines, or circles / arcs. Outside is positive.");
                    }
                    ModifyKind::Fillet => {
                        num(ui, "radius", &mut panel.radius);
                        ui.label("Select the two lines that meet at the corner.");
                    }
                    ModifyKind::Chamfer => {
                        num(ui, "distance 1", &mut panel.d1);
                        num(ui, "distance 2", &mut panel.d2);
                        ui.label("Select the two lines that meet at the corner.");
                    }
                }
            });
        });
    if ok {
        apply = panel.op();
    }
    // What the numbers would make is drawn while the page is open.
    if let Some(s) = &mut app.sketching {
        s.modify_preview = panel.op();
    }
    if let Some(op) = apply {
        app.apply_modify(&op);
    }
    if close || ui.ctx().input(|i| i.key_pressed(egui::Key::Escape)) {
        st.modify = None;
        if let Some(s) = &mut app.sketching {
            s.modify_preview = None;
        }
    } else {
        st.modify = Some(panel);
    }
}

/// Edits the copies of a cut or boss: none, a row or grid, a ring, or a mirror image.
fn mm(v: &mut f64) -> DragValue<'_> {
    DragValue::new(v).speed(0.2).suffix(" mm")
}

fn pattern_editor(ui: &mut Ui, pattern: &mut Option<FeaturePattern>) {
    let name = match pattern {
        None => "None",
        Some(FeaturePattern::Linear { .. }) => "Linear",
        Some(FeaturePattern::Circular { .. }) => "Circular",
        Some(FeaturePattern::Mirror { .. }) => "Mirror",
    };
    egui::ComboBox::from_id_salt("feature_pattern")
        .selected_text(name)
        .show_ui(ui, |ui| {
            if ui.selectable_label(pattern.is_none(), "None").clicked() {
                *pattern = None;
            }
            if ui.selectable_label(name == "Linear", "Linear").clicked() && name != "Linear" {
                *pattern = Some(FeaturePattern::Linear {
                    step: [10.0, 0.0],
                    count: 2,
                    step2: None,
                    count2: 1,
                });
            }
            if ui
                .selectable_label(name == "Circular", "Circular")
                .clicked()
                && name != "Circular"
            {
                *pattern = Some(FeaturePattern::Circular {
                    center: [0.0, 0.0],
                    count: 4,
                    step_degrees: 90.0,
                });
            }
            if ui.selectable_label(name == "Mirror", "Mirror").clicked() && name != "Mirror" {
                *pattern = Some(FeaturePattern::Mirror {
                    point: [0.0, 0.0],
                    direction_degrees: 90.0,
                });
            }
        });
    match pattern {
        None => {}
        Some(FeaturePattern::Linear {
            step,
            count,
            step2,
            count2,
        }) => {
            ui.horizontal(|ui| {
                ui.label("Copies");
                ui.add(DragValue::new(count).range(1..=200));
                ui.label("step x,y");
                ui.add(mm(&mut step[0]));
                ui.add(mm(&mut step[1]));
            });
            let mut grid = step2.is_some();
            if ui.checkbox(&mut grid, "Second direction (grid)").changed() {
                *step2 = grid.then_some([0.0, 10.0]);
                *count2 = if grid { 2 } else { 1 };
            }
            if let Some(s2) = step2 {
                ui.horizontal(|ui| {
                    ui.label("Rows");
                    ui.add(DragValue::new(count2).range(1..=200));
                    ui.label("step x,y");
                    ui.add(mm(&mut s2[0]));
                    ui.add(mm(&mut s2[1]));
                });
            }
        }
        Some(FeaturePattern::Circular {
            center,
            count,
            step_degrees,
        }) => {
            ui.horizontal(|ui| {
                ui.label("Copies");
                if ui.add(DragValue::new(count).range(1..=360)).changed() && *count > 0 {
                    *step_degrees = 360.0 / f64::from(*count);
                }
                ui.label("angle");
                ui.add(DragValue::new(step_degrees).speed(0.5).suffix(" deg"));
            });
            ui.horizontal(|ui| {
                ui.label("Centre x,y");
                ui.add(mm(&mut center[0]));
                ui.add(mm(&mut center[1]));
            });
        }
        Some(FeaturePattern::Mirror {
            point,
            direction_degrees,
        }) => {
            ui.horizontal(|ui| {
                ui.label("Line through x,y");
                ui.add(mm(&mut point[0]));
                ui.add(mm(&mut point[1]));
            });
            ui.horizontal(|ui| {
                ui.label("Direction");
                ui.add(DragValue::new(direction_degrees).speed(0.5).suffix(" deg"));
            });
        }
    }
}
