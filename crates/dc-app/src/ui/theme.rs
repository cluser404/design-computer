//! The look of the interface: colours, sizes and fonts, as tokens in one place.
//!
//! Light is the default (a light grey ribbon and panels around a white view, as in SolidWorks);
//! dark is a setting. Panels, menus and widgets get their colours from egui's visuals, set up
//! here; the ribbon, the tree and the view overlays read the tokens in [`Colors`].

use eframe::egui::{
    self, Color32, FontData, FontDefinitions, FontFamily, FontId, Stroke, TextStyle, Visuals,
};
use std::sync::Arc;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum ThemeMode {
    #[default]
    Light,
    Dark,
}

/// The font family holding only the icon glyphs.
pub fn icon_family() -> FontFamily {
    FontFamily::Name("icons".into())
}

/// The icon font at `size`; before the font is installed (the first frame) the normal font, so
/// drawing never panics on a family that is not bound yet.
pub fn icon_font(ctx: &egui::Context, size: f32) -> FontId {
    if ctx.fonts(|f| f.families().contains(&icon_family())) {
        FontId::new(size, icon_family())
    } else {
        FontId::proportional(size)
    }
}

/// Colour tokens. Everything the custom widgets paint takes its colour from here.
#[derive(Clone, Copy, Debug)]
pub struct Colors {
    /// The strip behind the ribbon's tabs and the title bar.
    pub chrome: Color32,
    /// The ribbon's command area and the panels.
    pub ribbon: Color32,
    /// A thin line between regions.
    pub line: Color32,
    pub text: Color32,
    pub text_weak: Color32,
    /// A button under the pointer.
    pub hover: Color32,
    pub hover_line: Color32,
    /// A button being pressed, or a toggled-on tool.
    pub active: Color32,
    pub active_line: Color32,
    /// The colour of selections, focus and the main action.
    pub accent: Color32,
    /// Drawing ink of the CAD icons: the lines and the main shapes.
    pub ink: Color32,
    pub ink_blue: Color32,
    pub ink_orange: Color32,
    pub ink_green: Color32,
    pub ink_red: Color32,
    pub ink_grey: Color32,
    /// The fill of shapes in icons (the "paper").
    pub paper: Color32,
}

impl Colors {
    pub fn of(mode: ThemeMode) -> Colors {
        match mode {
            ThemeMode::Light => Colors {
                chrome: Color32::from_rgb(222, 226, 232),
                ribbon: Color32::from_rgb(244, 246, 249),
                line: Color32::from_rgb(196, 202, 212),
                text: Color32::from_rgb(28, 33, 41),
                text_weak: Color32::from_rgb(108, 116, 128),
                hover: Color32::from_rgb(214, 232, 250),
                hover_line: Color32::from_rgb(120, 174, 226),
                active: Color32::from_rgb(190, 216, 244),
                active_line: Color32::from_rgb(52, 120, 200),
                accent: Color32::from_rgb(30, 110, 200),
                ink: Color32::from_rgb(52, 62, 78),
                ink_blue: Color32::from_rgb(30, 110, 200),
                ink_orange: Color32::from_rgb(238, 140, 24),
                ink_green: Color32::from_rgb(40, 158, 82),
                ink_red: Color32::from_rgb(212, 62, 60),
                ink_grey: Color32::from_rgb(150, 158, 172),
                paper: Color32::from_rgb(255, 255, 255),
            },
            ThemeMode::Dark => Colors {
                chrome: Color32::from_rgb(34, 37, 43),
                ribbon: Color32::from_rgb(44, 48, 56),
                line: Color32::from_rgb(70, 76, 88),
                text: Color32::from_rgb(226, 230, 237),
                text_weak: Color32::from_rgb(150, 158, 172),
                hover: Color32::from_rgb(58, 76, 102),
                hover_line: Color32::from_rgb(86, 130, 190),
                active: Color32::from_rgb(48, 86, 138),
                active_line: Color32::from_rgb(96, 156, 230),
                accent: Color32::from_rgb(96, 156, 230),
                ink: Color32::from_rgb(214, 220, 230),
                ink_blue: Color32::from_rgb(110, 170, 240),
                ink_orange: Color32::from_rgb(250, 164, 60),
                ink_green: Color32::from_rgb(84, 200, 124),
                ink_red: Color32::from_rgb(244, 100, 98),
                ink_grey: Color32::from_rgb(120, 128, 142),
                paper: Color32::from_rgb(64, 70, 82),
            },
        }
    }

    /// The tokens for the theme `ui` is drawn in.
    pub fn get(ctx: &egui::Context) -> Colors {
        Colors::of(if ctx.global_style().visuals.dark_mode {
            ThemeMode::Dark
        } else {
            ThemeMode::Light
        })
    }
}

static ICON_FONT: &[u8] = include_bytes!("../../assets/fonts/Phosphor.ttf");

/// Fonts: the egui defaults, plus the icon font, both as its own family and as a fallback of the
/// text families (so a glyph can be written into a label).
fn fonts() -> FontDefinitions {
    let mut f = FontDefinitions::default();
    f.font_data.insert(
        "phosphor".into(),
        Arc::new(FontData::from_static(ICON_FONT)),
    );
    f.families
        .insert(icon_family(), vec!["phosphor".to_owned()]);
    for fam in [FontFamily::Proportional, FontFamily::Monospace] {
        f.families
            .entry(fam)
            .or_default()
            .push("phosphor".to_owned());
    }
    f
}

/// Installs the default (light) theme the first time it is called for `ctx`, and does nothing
/// afterwards, so every window gets fonts and colours even when nobody asked.
pub fn ensure(ctx: &egui::Context) {
    let id = egui::Id::new("dc_theme_installed");
    if !ctx.data(|d| d.get_temp::<bool>(id)).unwrap_or(false) {
        ctx.data_mut(|d| d.insert_temp(id, true));
        install(ctx, ThemeMode::default());
    }
}

/// Sets fonts, text sizes, spacing and colours for `mode`. Call once at start and again when
/// the theme changes.
pub fn install(ctx: &egui::Context, mode: ThemeMode) {
    ctx.set_fonts(fonts());
    apply(ctx, mode);
}

/// Colours and sizes only (the fonts stay).
pub fn apply(ctx: &egui::Context, mode: ThemeMode) {
    let c = Colors::of(mode);
    let mut v = match mode {
        ThemeMode::Light => Visuals::light(),
        ThemeMode::Dark => Visuals::dark(),
    };
    v.panel_fill = c.ribbon;
    v.window_fill = match mode {
        ThemeMode::Light => Color32::WHITE,
        ThemeMode::Dark => Color32::from_rgb(50, 55, 64),
    };
    v.extreme_bg_color = c.paper;
    v.faint_bg_color = match mode {
        ThemeMode::Light => Color32::from_rgb(238, 241, 245),
        ThemeMode::Dark => Color32::from_rgb(52, 57, 66),
    };
    v.window_stroke = Stroke::new(1.0, c.line);
    v.selection.bg_fill = c.active;
    v.selection.stroke = Stroke::new(1.0, c.active_line);
    v.hyperlink_color = c.accent;
    v.override_text_color = Some(c.text);
    let round = egui::CornerRadius::same(3);
    let w = &mut v.widgets;
    w.noninteractive.bg_stroke = Stroke::new(1.0, c.line);
    w.noninteractive.fg_stroke = Stroke::new(1.0, c.text);
    w.inactive.corner_radius = round;
    w.hovered.corner_radius = round;
    w.active.corner_radius = round;
    w.open.corner_radius = round;
    w.inactive.weak_bg_fill = Color32::TRANSPARENT;
    w.inactive.bg_stroke = Stroke::new(1.0, Color32::TRANSPARENT);
    w.inactive.fg_stroke = Stroke::new(1.0, c.text);
    w.hovered.weak_bg_fill = c.hover;
    w.hovered.bg_fill = c.hover;
    w.hovered.bg_stroke = Stroke::new(1.0, c.hover_line);
    w.hovered.fg_stroke = Stroke::new(1.0, c.text);
    w.active.weak_bg_fill = c.active;
    w.active.bg_fill = c.active;
    w.active.bg_stroke = Stroke::new(1.0, c.active_line);
    w.active.fg_stroke = Stroke::new(1.0, c.text);
    w.open.weak_bg_fill = c.active;
    w.open.bg_fill = c.active;
    w.open.bg_stroke = Stroke::new(1.0, c.active_line);
    ctx.set_visuals_of(
        match mode {
            ThemeMode::Light => egui::Theme::Light,
            ThemeMode::Dark => egui::Theme::Dark,
        },
        v,
    );
    ctx.set_theme(match mode {
        ThemeMode::Light => egui::ThemePreference::Light,
        ThemeMode::Dark => egui::ThemePreference::Dark,
    });
    ctx.all_styles_mut(|style| {
        style.text_styles = [
            (TextStyle::Small, FontId::proportional(11.0)),
            (TextStyle::Body, FontId::proportional(13.0)),
            (TextStyle::Button, FontId::proportional(13.0)),
            (TextStyle::Heading, FontId::proportional(16.0)),
            (TextStyle::Monospace, FontId::monospace(12.5)),
        ]
        .into();
        style.spacing.item_spacing = egui::vec2(6.0, 4.0);
        style.spacing.button_padding = egui::vec2(8.0, 3.0);
        style.spacing.interact_size.y = 22.0;
        style.spacing.menu_margin = egui::Margin::same(4);
        style.interaction.tooltip_delay = 0.35;
    });
}
