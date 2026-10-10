//! The ribbon (SolidWorks' "CommandManager"): tabs, and under them groups of command buttons.
//!
//! Buttons are large (a 30 px icon with its name under it), small (a 16 px icon with its name
//! beside it, stacked up to three high), or split (a large button that runs its main command and
//! has an arrow with the variants). Every button is a command of the registry, so its name,
//! icon, tooltip, shortcut and "why is it greyed out" come from one place.
//!
//! Where the window is narrow the groups at the right fold into one button each (with the
//! group's name) that opens the group as a menu, so nothing is ever cut off.

use super::icons::{self, CadIcon, Icon};
use super::theme::Colors;
use crate::commands::{self, Host};
use crate::sketching::{Relation, Tool};
use crate::state::AppState;
use crate::ui::icon_font as g;
use eframe::egui::{
    self, Align2, Color32, FontId, Pos2, Rect, RichText, Sense, Stroke, StrokeKind, Ui, Vec2,
};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum Tab {
    #[default]
    Features,
    Sketch,
    Evaluate,
    View,
}

impl Tab {
    pub const ALL: [Tab; 4] = [Tab::Features, Tab::Sketch, Tab::Evaluate, Tab::View];

    pub fn label(self) -> &'static str {
        match self {
            Tab::Features => "Features",
            Tab::Sketch => "Sketch",
            Tab::Evaluate => "Evaluate",
            Tab::View => "View",
        }
    }
}

/// One thing in a group.
#[derive(Clone, Debug)]
pub enum Item {
    /// A large button.
    Big(&'static str),
    /// One to three small buttons, one above the other.
    Stack(Vec<&'static str>),
    /// A large button that runs `main` and has an arrow listing `variants`.
    Split {
        main: &'static str,
        variants: Vec<&'static str>,
    },
    /// A large button that only opens a menu.
    Menu {
        label: &'static str,
        icon: Icon,
        entries: Vec<&'static str>,
    },
    /// The snap settings of the sketch (a menu of switches).
    Snaps,
    /// What the sketch shows besides the geometry (a menu of switches).
    SketchDisplay,
}

#[derive(Clone, Debug)]
pub struct Group {
    pub title: &'static str,
    pub icon: Icon,
    pub items: Vec<Item>,
}

/// The id as the registry spells it (`'static`), panicking on a typo, which a test catches.
fn cid(id: &str) -> &'static str {
    commands::find(id)
        .map(|c| c.id.as_str())
        .unwrap_or_else(|| panic!("the ribbon names a command that does not exist: {id}"))
}

fn tools(list: &[Tool]) -> Vec<&'static str> {
    list.iter().map(|t| cid(&commands::tool_id(*t))).collect()
}

/// What each tab holds. The Sketch tab's first group is "Sketch" until a sketch is open and
/// "Exit Sketch" while one is.
pub fn groups(tab: Tab, app: &AppState) -> Vec<Group> {
    match tab {
        Tab::Features => vec![
            Group {
                title: "Features",
                icon: Icon::Cad(CadIcon::ExtrudedBoss),
                items: vec![
                    Item::Big(cid("feature.extrude")),
                    Item::Big(cid("feature.cut")),
                    Item::Big(cid("feature.boss")),
                    Item::Big(cid("feature.revolve")),
                    Item::Big(cid("feature.loft")),
                    Item::Big(cid("feature.sweep")),
                    Item::Big(cid("feature.fillet")),
                    Item::Big(cid("feature.chamfer")),
                    Item::Big(cid("feature.shell")),
                ],
            },
            Group {
                title: "Sketch",
                icon: Icon::Cad(CadIcon::Sketch),
                items: vec![sketch_entry(app)],
            },
        ],
        Tab::Sketch => vec![
            Group {
                title: "Sketch",
                icon: Icon::Cad(CadIcon::Sketch),
                items: vec![sketch_entry(app), Item::Big(cid("sketch.tool.dimension"))],
            },
            Group {
                title: "Draw",
                icon: Icon::Cad(CadIcon::Line),
                items: vec![
                    Item::Big(cid("sketch.tool.select")),
                    Item::Split {
                        main: cid("sketch.tool.line"),
                        variants: tools(&[Tool::Line, Tool::Centerline]),
                    },
                    Item::Split {
                        main: cid("sketch.tool.rectangle"),
                        variants: tools(&[
                            Tool::Rectangle,
                            Tool::CenterRectangle,
                            Tool::ThreePointRect,
                            Tool::ThreePointCentreRect,
                            Tool::Parallelogram,
                        ]),
                    },
                    Item::Big(cid("sketch.tool.circle")),
                    Item::Split {
                        main: cid("sketch.tool.arc"),
                        variants: tools(&[Tool::Arc, Tool::Arc3, Tool::TangentArc]),
                    },
                    Item::Big(cid("sketch.tool.polygon")),
                    Item::Big(cid("sketch.tool.slot")),
                    Item::Big(cid("sketch.tool.point")),
                ],
            },
            Group {
                title: "Modify",
                icon: Icon::Cad(CadIcon::Trim),
                items: vec![
                    Item::Big(cid("sketch.tool.trim")),
                    Item::Stack(vec![
                        cid("sketch.tool.extend"),
                        cid("sketch.tool.split"),
                        cid("sketch.modify.offset"),
                    ]),
                    Item::Stack(vec![
                        cid("sketch.modify.move"),
                        cid("sketch.modify.rotate"),
                        cid("sketch.modify.scale"),
                    ]),
                    Item::Stack(vec![
                        cid("sketch.modify.mirror"),
                        cid("sketch.modify.linear_pattern"),
                        cid("sketch.modify.circular_pattern"),
                    ]),
                    Item::Stack(vec![
                        cid("sketch.modify.fillet"),
                        cid("sketch.modify.chamfer"),
                    ]),
                ],
            },
            Group {
                title: "Relations",
                icon: Icon::Cad(CadIcon::Relation),
                items: vec![
                    Item::Menu {
                        label: "Add Relation",
                        icon: Icon::Cad(CadIcon::Relation),
                        entries: Relation::ALL
                            .iter()
                            .map(|r| cid(&commands::relation_id(*r)))
                            .collect(),
                    },
                    Item::Big(cid("sketch.fully_define")),
                    Item::Stack(vec![
                        cid("sketch.check"),
                        cid("sketch.repair"),
                        cid("sketch.construction"),
                    ]),
                ],
            },
            Group {
                title: "Aids",
                icon: Icon::Cad(CadIcon::Snap),
                items: vec![
                    Item::Snaps,
                    Item::SketchDisplay,
                    Item::Big(cid("sketch.view_normal")),
                    Item::Stack(vec![
                        cid("sketch.view_rotate_left"),
                        cid("sketch.view_rotate_right"),
                        cid("sketch.view_flip"),
                    ]),
                ],
            },
        ],
        Tab::Evaluate => vec![Group {
            title: "Measure",
            icon: Icon::Cad(CadIcon::CheckSketch),
            items: vec![Item::Big(cid("evaluate.mass"))],
        }],
        Tab::View => vec![
            Group {
                title: "Orientation",
                icon: Icon::Cad(CadIcon::Fit),
                items: vec![
                    Item::Big(cid("view.fit")),
                    Item::Split {
                        main: cid("view.iso"),
                        variants: vec![
                            cid("view.iso"),
                            cid("view.front"),
                            cid("view.back"),
                            cid("view.left"),
                            cid("view.right"),
                            cid("view.top"),
                            cid("view.bottom"),
                        ],
                    },
                    Item::Stack(vec![cid("view.front"), cid("view.top"), cid("view.right")]),
                    Item::Stack(vec![cid("view.back"), cid("view.bottom"), cid("view.left")]),
                    Item::Big(cid("view.perspective")),
                ],
            },
            Group {
                title: "Display",
                icon: Icon::Glyph(g::SUN),
                items: vec![
                    Item::Big(cid("view.realview")),
                    Item::Menu {
                        label: "Environment",
                        icon: Icon::Glyph(g::SUN),
                        entries: crate::environment::Environment::ALL
                            .iter()
                            .map(|e| cid(&commands::environment_id(*e)))
                            .collect(),
                    },
                    Item::Menu {
                        label: "Material",
                        icon: Icon::Glyph(g::PAINT_BUCKET),
                        entries: crate::state::Material::ALL
                            .iter()
                            .map(|m| cid(&commands::material_id(*m)))
                            .collect(),
                    },
                    Item::Stack(vec![
                        cid("view.origins"),
                        cid("view.edges"),
                        cid("view.sketches"),
                    ]),
                    Item::Stack(vec![
                        cid("view.sketch_grid"),
                        cid("view.ambient_occlusion"),
                        cid("view.floor_reflection"),
                    ]),
                    Item::Stack(vec![cid("view.fxaa"), cid("view.dark_theme")]),
                ],
            },
            Group {
                title: "Select",
                icon: Icon::Cad(CadIcon::Select),
                items: vec![
                    Item::Stack(vec![
                        cid("select.faces"),
                        cid("select.edges"),
                        cid("select.sketch_lines"),
                    ]),
                    Item::Stack(vec![cid("select.feature_first")]),
                ],
            },
        ],
    }
}

/// The "Sketch" / "Exit Sketch" entry.
fn sketch_entry(app: &AppState) -> Item {
    if app.sketching.is_some() {
        Item::Big(cid("sketch.finish"))
    } else {
        Item::Menu {
            label: "Sketch",
            icon: Icon::Cad(CadIcon::Sketch),
            entries: vec![
                cid("sketch.new_xy"),
                cid("sketch.new_xz"),
                cid("sketch.new_yz"),
                cid("sketch.on_face"),
                cid("sketch.edit"),
            ],
        }
    }
}

// ---- drawing ---------------------------------------------------------------------------------

const BIG_H: f32 = 70.0;
const SMALL_H: f32 = 22.0;
const BIG_ICON: f32 = 30.0;
const SMALL_ICON: f32 = 16.0;
const COLLAPSED_W: f32 = 64.0;
const PAD: f32 = 5.0;

/// A big button's label on one or two lines.
fn two_lines(label: &str) -> (String, Option<String>) {
    if label.chars().count() <= 11 {
        return (label.to_owned(), None);
    }
    let words: Vec<&str> = label.split(' ').collect();
    if words.len() < 2 {
        return (label.to_owned(), None);
    }
    // The split that makes the two lines most even.
    let mut best = (usize::MAX, 1);
    for k in 1..words.len() {
        let (a, b) = (words[..k].join(" ").len(), words[k..].join(" ").len());
        let diff = a.max(b);
        if diff < best.0 {
            best = (diff, k);
        }
    }
    (words[..best.1].join(" "), Some(words[best.1..].join(" ")))
}

fn text_width(ui: &Ui, text: &str, size: f32) -> f32 {
    ui.painter()
        .layout_no_wrap(text.to_owned(), FontId::proportional(size), Color32::BLACK)
        .size()
        .x
}

fn big_width(ui: &Ui, label: &str, extra: f32) -> f32 {
    let (a, b) = two_lines(label);
    let w = text_width(ui, &a, 11.5).max(b.map_or(0.0, |b| text_width(ui, &b, 11.5)));
    (w + 14.0).max(52.0) + extra
}

fn small_width(ui: &Ui, label: &str) -> f32 {
    SMALL_ICON + 8.0 + text_width(ui, label, 12.0) + 12.0
}

fn label_of(id: &str) -> String {
    commands::find(id).map_or(id.to_owned(), |c| {
        // Menu entries carry a long label ("Add relation: Horizontal"); buttons use the name.
        c.label.trim_end_matches('…').to_owned()
    })
}

fn item_width(ui: &Ui, item: &Item) -> f32 {
    match item {
        Item::Big(id) => big_width(ui, &label_of(id), 0.0),
        Item::Stack(ids) => ids
            .iter()
            .map(|id| small_width(ui, &label_of(id)))
            .fold(0.0, f32::max),
        Item::Split { main, .. } => big_width(ui, &label_of(main), 0.0),
        Item::Menu { label, .. } => big_width(ui, label, 0.0),
        Item::Snaps => big_width(ui, "Snaps", 0.0),
        Item::SketchDisplay => big_width(ui, "Sketch Display", 0.0),
    }
}

fn group_width(ui: &Ui, group: &Group) -> f32 {
    group.items.iter().map(|i| item_width(ui, i)).sum::<f32>()
        + PAD * (group.items.len() as f32 + 1.0)
}

/// What a button needs to be drawn and run, gathered once per frame.
pub struct Cx<'a> {
    pub app: &'a mut AppState,
    pub host: &'a mut dyn Host,
    pub colors: Colors,
}

impl Cx<'_> {
    fn enabled(&self, id: &str) -> Result<(), &'static str> {
        commands::enabled(id, self.app, self.host)
    }

    fn run(&mut self, id: &str) {
        if let Err(e) = commands::run(id, self.app, self.host) {
            self.app.error(e, None);
        }
    }
}

/// The tooltip of a command: its name and shortcut, what it does, why it is off if it is, and
/// the command an agent would send for the same thing.
fn tooltip(ui: &mut Ui, id: &str, off: Option<&str>, colors: &Colors) {
    let Some(c) = commands::find(id) else { return };
    ui.set_max_width(300.0);
    ui.horizontal(|ui| {
        ui.label(RichText::new(c.label.trim_end_matches('…')).strong());
        if !c.shortcut.is_empty() {
            ui.label(RichText::new(format!("({})", c.shortcut)).color(colors.text_weak));
        }
    });
    ui.label(&c.tip);
    if let Some(reason) = off {
        ui.label(RichText::new(reason).color(colors.ink_orange));
    }
    if !c.agent.is_empty() {
        ui.add_space(2.0);
        ui.label(
            RichText::new(format!("agent command: {}", c.agent))
                .small()
                .monospace()
                .color(colors.text_weak),
        );
    }
}

fn button_frame(
    ui: &Ui,
    rect: Rect,
    hovered: bool,
    pressed: bool,
    on: bool,
    enabled: bool,
    colors: &Colors,
) {
    let (fill, line) = if on || pressed {
        (colors.active, colors.active_line)
    } else if hovered && enabled {
        (colors.hover, colors.hover_line)
    } else {
        return;
    };
    ui.painter()
        .rect(rect, 3.0, fill, Stroke::new(1.0, line), StrokeKind::Inside);
}

fn draw_label(ui: &Ui, rect: Rect, label: &str, colors: &Colors, enabled: bool) {
    let (a, b) = two_lines(label);
    let col = if enabled {
        colors.text
    } else {
        colors.text.gamma_multiply(0.45)
    };
    let font = FontId::proportional(11.5);
    let top = rect.top() + 4.0 + BIG_ICON + 3.0;
    let paint = |y: f32, t: &str| {
        ui.painter().text(
            Pos2::new(rect.center().x, y),
            Align2::CENTER_TOP,
            t,
            font.clone(),
            col,
        );
    };
    paint(top, &a);
    if let Some(b) = b {
        paint(top + 12.5, &b);
    }
}

fn tip_id() -> egui::Id {
    egui::Id::new("ribbon_tip_rects")
}

/// Remembers where a runnable command's button is this frame, for the Alt key tips.
fn note_tip(ui: &Ui, id: &'static str, rect: Rect, enabled: bool) {
    if enabled {
        ui.ctx().data_mut(|d| {
            d.get_temp_mut_or_default::<Vec<(&'static str, Rect)>>(tip_id())
                .push((id, rect));
        });
    }
}

/// Forgets last frame's button places (call before drawing the command row).
pub fn begin_tips(ctx: &egui::Context) {
    ctx.data_mut(|d| d.remove_temp::<Vec<(&'static str, Rect)>>(tip_id()));
}

/// The commands drawn this frame in order, with where they are.
pub fn tip_targets(ctx: &egui::Context) -> Vec<(&'static str, Rect)> {
    ctx.data(|d| d.get_temp::<Vec<(&'static str, Rect)>>(tip_id()))
        .unwrap_or_default()
}

/// A large command button. Returns whether it was clicked.
fn big_button(ui: &mut Ui, cx: &mut Cx, id: &'static str, width: f32) -> bool {
    let off = cx.enabled(id).err();
    let (rect, resp) = ui.allocate_exact_size(Vec2::new(width, BIG_H), Sense::click());
    note_tip(ui, id, rect, off.is_none());
    let on = commands::checked(id, cx.app, cx.host).unwrap_or(false);
    crate::devtools::tag(
        &format!("ribbon.{id}"),
        eguidev::WidgetRole::Button,
        &label_of(id),
        Some(on),
        &resp,
    );
    button_frame(
        ui,
        rect,
        resp.hovered(),
        resp.is_pointer_button_down_on(),
        on,
        off.is_none(),
        &cx.colors,
    );
    if let Some(c) = commands::find(id) {
        let icon = Rect::from_center_size(
            Pos2::new(rect.center().x, rect.top() + 4.0 + BIG_ICON / 2.0),
            Vec2::splat(BIG_ICON),
        );
        icons::paint(ui.painter(), icon, c.icon, &cx.colors, off.is_none());
        draw_label(ui, rect, &label_of(id), &cx.colors, off.is_none());
    }
    let colors = cx.colors;
    resp.clone().on_hover_ui(|ui| tooltip(ui, id, off, &colors));
    resp.clicked() && off.is_none()
}

/// A small command button (icon and name on one line).
fn small_button(ui: &mut Ui, cx: &mut Cx, id: &'static str, width: f32) -> bool {
    let off = cx.enabled(id).err();
    let (rect, resp) = ui.allocate_exact_size(Vec2::new(width, SMALL_H), Sense::click());
    note_tip(ui, id, rect, off.is_none());
    let on = commands::checked(id, cx.app, cx.host).unwrap_or(false);
    crate::devtools::tag(
        &format!("ribbon.{id}"),
        eguidev::WidgetRole::Button,
        &label_of(id),
        Some(on),
        &resp,
    );
    button_frame(
        ui,
        rect,
        resp.hovered(),
        resp.is_pointer_button_down_on(),
        on,
        off.is_none(),
        &cx.colors,
    );
    if let Some(c) = commands::find(id) {
        let icon = Rect::from_center_size(
            Pos2::new(rect.left() + 4.0 + SMALL_ICON / 2.0, rect.center().y),
            Vec2::splat(SMALL_ICON),
        );
        icons::paint(ui.painter(), icon, c.icon, &cx.colors, off.is_none());
        ui.painter().text(
            Pos2::new(rect.left() + 8.0 + SMALL_ICON, rect.center().y),
            Align2::LEFT_CENTER,
            label_of(id),
            FontId::proportional(12.0),
            if off.is_none() {
                cx.colors.text
            } else {
                cx.colors.text.gamma_multiply(0.45)
            },
        );
    }
    let colors = cx.colors;
    resp.clone().on_hover_ui(|ui| tooltip(ui, id, off, &colors));
    resp.clicked() && off.is_none()
}

/// One row of a drop-down: the command's icon, name and shortcut, with a tick if it is switched on.
pub fn menu_row(ui: &mut Ui, cx: &mut Cx, id: &'static str) -> bool {
    let off = cx.enabled(id).err();
    let w = ui.available_width().max(190.0);
    let (rect, resp) = ui.allocate_exact_size(Vec2::new(w, 24.0), Sense::click());
    crate::devtools::tag(
        &format!("menu.{id}"),
        eguidev::WidgetRole::Button,
        &label_of(id),
        None,
        &resp,
    );
    let on = commands::checked(id, cx.app, cx.host);
    button_frame(
        ui,
        rect,
        resp.hovered(),
        false,
        false,
        off.is_none(),
        &cx.colors,
    );
    if let Some(c) = commands::find(id) {
        if on == Some(true) {
            ui.painter().text(
                Pos2::new(rect.left() + 8.0, rect.center().y),
                Align2::CENTER_CENTER,
                g::CHECK.to_string(),
                super::theme::icon_font(ui.ctx(), 13.0),
                cx.colors.accent,
            );
        }
        let icon = Rect::from_center_size(
            Pos2::new(rect.left() + 28.0, rect.center().y),
            Vec2::splat(16.0),
        );
        icons::paint(ui.painter(), icon, c.icon, &cx.colors, off.is_none());
        let col = if off.is_none() {
            cx.colors.text
        } else {
            cx.colors.text.gamma_multiply(0.45)
        };
        ui.painter().text(
            Pos2::new(rect.left() + 44.0, rect.center().y),
            Align2::LEFT_CENTER,
            c.label
                .trim_start_matches("Add relation: ")
                .trim_start_matches("Environment: ")
                .trim_start_matches("Material: "),
            FontId::proportional(12.5),
            col,
        );
        if !c.shortcut.is_empty() {
            ui.painter().text(
                Pos2::new(rect.right() - 8.0, rect.center().y),
                Align2::RIGHT_CENTER,
                &c.shortcut,
                FontId::proportional(11.0),
                cx.colors.text_weak,
            );
        }
    }
    let colors = cx.colors;
    resp.clone().on_hover_ui(|ui| tooltip(ui, id, off, &colors));
    resp.clicked() && off.is_none()
}

/// A large button that opens `content` as a menu.
fn menu_button(
    ui: &mut Ui,
    cx: &mut Cx,
    key: &str,
    label: &str,
    icon: Icon,
    width: f32,
    content: impl FnOnce(&mut Ui, &mut Cx),
) {
    let (rect, resp) = ui.allocate_exact_size(Vec2::new(width, BIG_H), Sense::click());
    crate::devtools::tag(
        &format!("ribbon.menu.{key}"),
        eguidev::WidgetRole::MenuButton,
        label,
        None,
        &resp,
    );
    let open = egui::Popup::is_id_open(ui.ctx(), egui::Popup::default_response_id(&resp));
    button_frame(
        ui,
        rect,
        resp.hovered(),
        resp.is_pointer_button_down_on(),
        open,
        true,
        &cx.colors,
    );
    let icon_rect = Rect::from_center_size(
        Pos2::new(rect.center().x, rect.top() + 4.0 + BIG_ICON / 2.0),
        Vec2::splat(BIG_ICON),
    );
    icons::paint(ui.painter(), icon_rect, icon, &cx.colors, true);
    draw_label(ui, rect, label, &cx.colors, true);
    caret(ui, rect, &cx.colors);
    let _ = key;
    egui::Popup::menu(&resp).show(|ui| {
        ui.set_min_width(210.0);
        content(ui, cx);
    });
}

/// The small drop-down arrow at the foot of a large button.
fn caret(ui: &Ui, rect: Rect, colors: &Colors) {
    ui.painter().text(
        Pos2::new(rect.center().x, rect.bottom() - 2.0),
        Align2::CENTER_BOTTOM,
        g::CARET_DOWN.to_string(),
        super::theme::icon_font(ui.ctx(), 9.0),
        colors.text_weak,
    );
}

/// A large button that runs `main`, with an arrow that lists the `variants`.
fn split_button(
    ui: &mut Ui,
    cx: &mut Cx,
    main: &'static str,
    variants: &[&'static str],
    width: f32,
) {
    let off = cx.enabled(main).err();
    let (rect, resp) = ui.allocate_exact_size(Vec2::new(width, BIG_H), Sense::hover());
    let arrow_rect = Rect::from_min_max(
        Pos2::new(rect.left(), rect.bottom() - 14.0),
        rect.right_bottom(),
    );
    let main_rect = Rect::from_min_max(
        rect.left_top(),
        Pos2::new(rect.right(), rect.bottom() - 14.0),
    );
    let main_resp = ui.interact(
        main_rect,
        ui.id().with(("split_main", main)),
        Sense::click(),
    );
    let arrow_resp = ui.interact(
        arrow_rect,
        ui.id().with(("split_arrow", main)),
        Sense::click(),
    );
    crate::devtools::tag(
        &format!("ribbon.{main}"),
        eguidev::WidgetRole::Button,
        &label_of(main),
        None,
        &main_resp,
    );
    crate::devtools::tag(
        &format!("ribbon.{main}.more"),
        eguidev::WidgetRole::MenuButton,
        "more",
        None,
        &arrow_resp,
    );
    let on = commands::checked(main, cx.app, cx.host).unwrap_or(false)
        || variants
            .iter()
            .any(|v| commands::checked(v, cx.app, cx.host).unwrap_or(false));
    let open = egui::Popup::is_id_open(ui.ctx(), egui::Popup::default_response_id(&arrow_resp));
    button_frame(
        ui,
        rect,
        resp.hovered() || main_resp.hovered() || arrow_resp.hovered(),
        main_resp.is_pointer_button_down_on(),
        on || open,
        off.is_none(),
        &cx.colors,
    );
    if let Some(c) = commands::find(main) {
        let icon_rect = Rect::from_center_size(
            Pos2::new(rect.center().x, rect.top() + 4.0 + BIG_ICON / 2.0),
            Vec2::splat(BIG_ICON),
        );
        // The icon shown is the variant last chosen if it is on, so the button says what it will do.
        let shown = variants
            .iter()
            .find(|v| commands::checked(v, cx.app, cx.host).unwrap_or(false))
            .and_then(|v| commands::find(v))
            .unwrap_or(c);
        icons::paint(
            ui.painter(),
            icon_rect,
            shown.icon,
            &cx.colors,
            off.is_none(),
        );
        draw_label(ui, rect, &label_of(&shown.id), &cx.colors, off.is_none());
    }
    caret(ui, rect, &cx.colors);
    let colors = cx.colors;
    main_resp
        .clone()
        .on_hover_ui(|ui| tooltip(ui, main, off, &colors));
    if main_resp.clicked() && off.is_none() {
        cx.run(main);
    }
    let variants: Vec<&'static str> = variants.to_vec();
    egui::Popup::menu(&arrow_resp).show(|ui| {
        ui.set_min_width(210.0);
        for v in &variants {
            if menu_row(ui, cx, v) {
                cx.run(v);
                ui.close();
            }
        }
    });
}

fn snaps_menu(ui: &mut Ui, cx: &mut Cx) {
    let Some(s) = &mut cx.app.sketching else {
        ui.weak("Start a sketch to set its snaps.");
        return;
    };
    ui.checkbox(&mut s.snap_grid, "Snap to grid");
    ui.checkbox(&mut s.snaps.points, "Points and origin");
    ui.checkbox(&mut s.snaps.edges, "Edges");
    ui.checkbox(&mut s.snaps.guides, "Level / upright guides");
    ui.checkbox(&mut s.snaps.midpoints, "Midpoints");
    ui.checkbox(&mut s.snaps.quadrants, "Circle quadrants");
    ui.checkbox(
        &mut s.snaps.inference,
        "Perpendicular / collinear / tangent",
    );
    ui.separator();
    ui.weak("Hold Ctrl while dragging to move freely");
}

fn sketch_display_menu(ui: &mut Ui, cx: &mut Cx) {
    let Some(s) = &mut cx.app.sketching else {
        ui.weak("Start a sketch to set what it shows.");
        return;
    };
    ui.checkbox(&mut s.show_relations, "Relation symbols");
    if s.show_relations {
        ui.add(
            egui::Slider::new(&mut s.glyph_scale, 0.7..=1.8)
                .show_value(false)
                .text("size"),
        );
    }
    ui.checkbox(&mut s.show_freedom, "Freedom arrows");
    let mut grid = cx.app.show_sketch_grid;
    if ui.checkbox(&mut grid, "Grid").changed() {
        cx.app.show_sketch_grid = grid;
        cx.app.view_version += 1;
    }
    let mut hide = cx.app.hide_bodies;
    if ui
        .checkbox(&mut hide, "Hide the part while sketching")
        .changed()
    {
        cx.app.set_hide_bodies(hide);
    }
}

fn draw_item(ui: &mut Ui, cx: &mut Cx, item: &Item) {
    let w = item_width(ui, item);
    match item {
        Item::Big(id) => {
            if big_button(ui, cx, id, w) {
                cx.run(id);
            }
        }
        Item::Stack(ids) => {
            ui.vertical(|ui| {
                ui.spacing_mut().item_spacing.y = 1.0;
                ui.add_space(2.0);
                for id in ids {
                    if small_button(ui, cx, id, w) {
                        cx.run(id);
                    }
                }
            });
        }
        Item::Split { main, variants } => split_button(ui, cx, main, variants, w),
        Item::Menu {
            label,
            icon,
            entries,
        } => {
            menu_button(ui, cx, label, label, *icon, w, |ui, cx| {
                for id in entries {
                    if menu_row(ui, cx, id) {
                        cx.run(id);
                        ui.close();
                    }
                }
            });
        }
        Item::Snaps => menu_button(
            ui,
            cx,
            "snaps",
            "Snaps",
            Icon::Cad(CadIcon::Snap),
            w,
            |ui, cx| {
                snaps_menu(ui, cx);
            },
        ),
        Item::SketchDisplay => menu_button(
            ui,
            cx,
            "sketch_display",
            "Sketch Display",
            Icon::Glyph(g::EYE),
            w,
            sketch_display_menu,
        ),
    }
}

/// A group folded into one button: its icon and name, opening the whole group as a menu.
fn draw_collapsed(ui: &mut Ui, cx: &mut Cx, group: &Group) {
    menu_button(
        ui,
        cx,
        group.title,
        group.title,
        group.icon,
        COLLAPSED_W,
        |ui, cx| {
            for item in &group.items {
                match item {
                    Item::Big(id) => entry(ui, cx, id),
                    Item::Stack(ids) => ids.iter().for_each(|id| entry(ui, cx, id)),
                    Item::Split { variants, .. } => {
                        variants.iter().for_each(|id| entry(ui, cx, id))
                    }
                    Item::Menu { entries, .. } => entries.iter().for_each(|id| entry(ui, cx, id)),
                    Item::Snaps => {
                        ui.separator();
                        snaps_menu(ui, cx);
                    }
                    Item::SketchDisplay => {
                        ui.separator();
                        sketch_display_menu(ui, cx);
                    }
                }
            }
        },
    );
}

fn entry(ui: &mut Ui, cx: &mut Cx, id: &'static str) {
    if menu_row(ui, cx, id) {
        cx.run(id);
        ui.close();
    }
}

/// Which groups fold, given the width there is: from the right, until the rest fit.
pub fn fold(widths: &[f32], available: f32) -> Vec<bool> {
    let mut folded = vec![false; widths.len()];
    let mut total: f32 = widths.iter().sum();
    for i in (0..widths.len()).rev() {
        if total <= available {
            break;
        }
        if widths[i] > COLLAPSED_W {
            total -= widths[i] - COLLAPSED_W;
            folded[i] = true;
        }
    }
    folded
}

/// Draws the command row of `tab`.
pub fn show_commands(ui: &mut Ui, cx: &mut Cx, tab: Tab) {
    let groups = groups(tab, cx.app);
    let widths: Vec<f32> = groups.iter().map(|g| group_width(ui, g) + 1.0).collect();
    let folded = fold(&widths, ui.available_width() - 8.0);
    ui.horizontal_top(|ui| {
        ui.spacing_mut().item_spacing.x = PAD;
        ui.add_space(PAD);
        for (i, group) in groups.iter().enumerate() {
            if folded[i] {
                draw_collapsed(ui, cx, group);
            } else {
                for item in &group.items {
                    draw_item(ui, cx, item);
                }
            }
            // A thin line between groups.
            let r = ui
                .allocate_exact_size(Vec2::new(1.0, BIG_H - 6.0), Sense::hover())
                .0;
            ui.painter().line_segment(
                [
                    r.center_top() + Vec2::new(0.0, 3.0),
                    r.center_bottom() - Vec2::new(0.0, 3.0),
                ],
                Stroke::new(1.0, cx.colors.line),
            );
        }
    });
}

/// The height of the command row.
pub const COMMAND_ROW_HEIGHT: f32 = BIG_H + 8.0;

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;

    fn ids_of(g: &Group) -> Vec<&'static str> {
        let mut v = Vec::new();
        for i in &g.items {
            match i {
                Item::Big(id) => v.push(*id),
                Item::Stack(ids) => v.extend(ids),
                Item::Split { main, variants } => {
                    v.push(*main);
                    v.extend(variants);
                }
                Item::Menu { entries, .. } => v.extend(entries),
                Item::Snaps | Item::SketchDisplay => {}
            }
        }
        v
    }

    #[test]
    fn every_tab_refers_only_to_real_commands_and_a_stack_has_at_most_three() {
        let mut app = AppState::new();
        for sketching in [false, true] {
            if sketching {
                app.load_demo();
                app.begin_sketch(dc_sketch::PlaneSpec::Xy { offset: 0.0 });
            }
            for tab in Tab::ALL {
                let groups = groups(tab, &app);
                assert!(!groups.is_empty(), "{tab:?}");
                for g in &groups {
                    assert!(!g.items.is_empty(), "{tab:?}/{}", g.title);
                    for id in ids_of(g) {
                        assert!(commands::find(id).is_some(), "{id}");
                    }
                    for i in &g.items {
                        if let Item::Stack(ids) = i {
                            assert!((1..=3).contains(&ids.len()), "{tab:?}/{}", g.title);
                        }
                    }
                }
            }
        }
    }

    #[test]
    fn the_sketch_group_offers_exit_only_while_sketching() {
        let mut app = AppState::new();
        let has = |app: &AppState, id: &str| {
            groups(Tab::Sketch, app)
                .iter()
                .flat_map(ids_of)
                .any(|x| x == id)
        };
        assert!(has(&app, "sketch.new_xy") && !has(&app, "sketch.finish"));
        app.begin_sketch(dc_sketch::PlaneSpec::Xy { offset: 0.0 });
        assert!(has(&app, "sketch.finish") && !has(&app, "sketch.new_xy"));
    }

    #[test]
    fn every_drawing_tool_and_modify_operation_is_on_the_sketch_tab() {
        let app = AppState::new();
        let on_tab: HashSet<&str> = groups(Tab::Sketch, &app).iter().flat_map(ids_of).collect();
        for t in Tool::ALL {
            let id = commands::tool_id(t);
            assert!(on_tab.contains(id.as_str()), "{t:?} has no button");
        }
        for k in crate::ui::ModifyKind::ALL {
            let id = commands::modify_id(k);
            assert!(on_tab.contains(id.as_str()), "{k:?} has no button");
        }
        for r in Relation::ALL {
            let id = commands::relation_id(r);
            assert!(on_tab.contains(id.as_str()), "{r:?} has no button");
        }
    }

    #[test]
    fn folding_takes_groups_from_the_right_until_the_rest_fit() {
        let w = [200.0, 300.0, 250.0, 180.0];
        // Room for everything: nothing folds.
        assert_eq!(fold(&w, 1000.0), vec![false; 4]);
        // A little short: only the last folds.
        assert_eq!(fold(&w, 880.0), vec![false, false, false, true]);
        // Much shorter: from the right, in order.
        let f = fold(&w, 500.0);
        assert!(!f[0] && f[3] && f[2], "{f:?}");
        // Never folds a group that is already no wider than a folded one.
        assert_eq!(fold(&[40.0, 40.0], 10.0), vec![false, false]);
        // Whatever the width, the folded total is as small as folding can make it.
        let total: f32 = fold(&w, 0.0)
            .iter()
            .zip(&w)
            .map(|(f, w)| if *f { COLLAPSED_W } else { *w })
            .sum();
        assert!(total <= 4.0 * COLLAPSED_W + 1.0);
    }

    #[test]
    fn labels_split_evenly_over_two_lines_and_short_ones_stay_whole() {
        assert_eq!(two_lines("Trim"), ("Trim".into(), None));
        assert_eq!(
            two_lines("Smart Dimension"),
            ("Smart".into(), Some("Dimension".into()))
        );
        let (a, b) = two_lines("Fully define sketch");
        assert_eq!((a.as_str(), b.as_deref()), ("Fully define", Some("sketch")));
        assert_eq!(two_lines("Isometric"), ("Isometric".into(), None));
    }
}
