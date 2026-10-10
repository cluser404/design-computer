//! Every action of the application, as data: one registry that the ribbon, the menus, the
//! command palette, the tooltips and the shortcuts all read, so a command is described once and
//! behaves the same wherever it appears. (AGENTS.md §12.3: every action is a bindable command.)
//!
//! A command has a stable `id` (`sketch.tool.line`), a label, a tooltip, the shortcut to show,
//! an icon, and the protocol command an agent would use to do the same (empty for pure view
//! changes). [`run`] performs it through [`AppState`], which sends ordinary commands to the
//! authority; nothing here edits the document directly. Logic only, no egui widgets.

use crate::environment::Environment;
use crate::featuring::FeatureKind;
use crate::sketching::{Relation, Tool};
use crate::state::{AppState, Material, Selected};
use crate::ui::icon_font as g;
use crate::ui::{CadIcon, Icon, ModifyKind};
use dc_render::ViewPreset;
use dc_sketch::PlaneSpec;
use std::sync::OnceLock;

/// One action.
#[derive(Clone, Debug)]
pub struct Command {
    pub id: String,
    pub label: String,
    pub tip: String,
    /// The shortcut as shown ("Ctrl+Z"), empty if none.
    pub shortcut: String,
    pub icon: Icon,
    /// The group it is listed under in the palette.
    pub category: &'static str,
    /// The protocol command an agent sends for the same thing (empty for view-only actions).
    pub agent: &'static str,
}

/// What a command needs from the window it runs in.
pub trait Host {
    /// Is the viewport drawn on the GPU (RealView and friends need it)?
    fn has_gpu(&self) -> bool;
    /// Ask for a file name to open or save to.
    fn ask_file(&mut self, save: bool, current: &str);
    /// Open the panel of a sketch modify operation.
    fn open_modify(&mut self, kind: ModifyKind, centre: (f64, f64));
    fn open_palette(&mut self);
    fn quit(&mut self);
    /// Is the interface in its dark theme?
    fn dark_theme(&self) -> bool;
    fn set_dark_theme(&mut self, dark: bool);
    /// Switches to the next mouse navigation preset and returns its name.
    fn next_nav(&mut self) -> &'static str {
        crate::navigation::NavPreset::default().name()
    }
}

fn cmd(
    id: impl Into<String>,
    label: impl Into<String>,
    tip: impl Into<String>,
    shortcut: impl Into<String>,
    icon: Icon,
    category: &'static str,
    agent: &'static str,
) -> Command {
    Command {
        id: id.into(),
        label: label.into(),
        tip: tip.into(),
        shortcut: shortcut.into(),
        icon,
        category,
        agent,
    }
}

fn tool_icon(t: Tool) -> Icon {
    Icon::Cad(match t {
        Tool::Select => CadIcon::Select,
        Tool::Line => CadIcon::Line,
        Tool::Centerline => CadIcon::Centerline,
        Tool::Rectangle => CadIcon::Rectangle,
        Tool::CenterRectangle => CadIcon::CenterRectangle,
        Tool::ThreePointRect => CadIcon::ThreePointRect,
        Tool::ThreePointCentreRect => CadIcon::ThreePointCentreRect,
        Tool::Parallelogram => CadIcon::Parallelogram,
        Tool::Polygon => CadIcon::Polygon,
        Tool::Slot => CadIcon::Slot,
        Tool::Circle => CadIcon::Circle,
        Tool::Arc => CadIcon::Arc,
        Tool::Arc3 => CadIcon::Arc3,
        Tool::TangentArc => CadIcon::TangentArc,
        Tool::Point => CadIcon::Point,
        Tool::Dimension => CadIcon::Dimension,
        Tool::Trim => CadIcon::Trim,
        Tool::Extend => CadIcon::Extend,
        Tool::Split => CadIcon::Split,
    })
}

/// The id of the command that selects `t`.
pub fn tool_id(t: Tool) -> String {
    let n = match t {
        Tool::Select => "select",
        Tool::Line => "line",
        Tool::Centerline => "centerline",
        Tool::Rectangle => "rectangle",
        Tool::CenterRectangle => "centre_rectangle",
        Tool::ThreePointRect => "three_point_rectangle",
        Tool::ThreePointCentreRect => "three_point_centre_rectangle",
        Tool::Parallelogram => "parallelogram",
        Tool::Polygon => "polygon",
        Tool::Slot => "slot",
        Tool::Circle => "circle",
        Tool::Arc => "arc",
        Tool::Arc3 => "arc3",
        Tool::TangentArc => "tangent_arc",
        Tool::Point => "point",
        Tool::Dimension => "dimension",
        Tool::Trim => "trim",
        Tool::Extend => "extend",
        Tool::Split => "split",
    };
    format!("sketch.tool.{n}")
}

pub fn modify_id(k: ModifyKind) -> String {
    let n = match k {
        ModifyKind::Move => "move",
        ModifyKind::Rotate => "rotate",
        ModifyKind::Scale => "scale",
        ModifyKind::Mirror => "mirror",
        ModifyKind::LinearPattern => "linear_pattern",
        ModifyKind::CircularPattern => "circular_pattern",
        ModifyKind::Offset => "offset",
        ModifyKind::Fillet => "fillet",
        ModifyKind::Chamfer => "chamfer",
    };
    format!("sketch.modify.{n}")
}

fn modify_icon(k: ModifyKind) -> Icon {
    Icon::Cad(match k {
        ModifyKind::Move => CadIcon::Move,
        ModifyKind::Rotate => CadIcon::Rotate,
        ModifyKind::Scale => CadIcon::Scale,
        ModifyKind::Mirror => CadIcon::Mirror,
        ModifyKind::LinearPattern => CadIcon::LinearPattern,
        ModifyKind::CircularPattern => CadIcon::CircularPattern,
        ModifyKind::Offset => CadIcon::Offset,
        ModifyKind::Fillet => CadIcon::Fillet,
        ModifyKind::Chamfer => CadIcon::Chamfer,
    })
}

pub fn relation_id(r: Relation) -> String {
    format!(
        "sketch.relation.{}",
        r.name().to_lowercase().replace([' ', '-'], "_")
    )
}

fn slug(name: &str) -> String {
    let mut out = String::new();
    for c in name.chars() {
        if c.is_ascii_alphanumeric() {
            out.push(c.to_ascii_lowercase());
        } else if !out.ends_with('_') {
            out.push('_');
        }
    }
    out.trim_matches('_').to_owned()
}

/// The id of the command that chooses the RealView environment `e`.
pub fn environment_id(e: Environment) -> String {
    format!("view.environment.{}", slug(e.name()))
}

/// The id of the command that chooses the RealView material `m`.
pub fn material_id(m: Material) -> String {
    format!("view.material.{}", slug(m.name()))
}

#[allow(clippy::vec_init_then_push)] // a long literal list, one command per statement
fn build() -> Vec<Command> {
    let mut v = Vec::new();
    // ---- file and edit ---------------------------------------------------------------------
    v.push(cmd(
        "file.new",
        "New",
        "Start an empty document",
        "Ctrl+N",
        Icon::Glyph(g::FILE_PLUS),
        "File",
        "",
    ));
    v.push(cmd(
        "file.open",
        "Open…",
        "Open a .dcad file",
        "Ctrl+O",
        Icon::Glyph(g::FOLDER_OPEN),
        "File",
        "",
    ));
    v.push(cmd(
        "file.save",
        "Save",
        "Save to the current file",
        "Ctrl+S",
        Icon::Glyph(g::FLOPPY_DISK),
        "File",
        "document.save",
    ));
    v.push(cmd(
        "file.save_as",
        "Save as…",
        "Save under a new name",
        "Ctrl+Shift+S",
        Icon::Glyph(g::DOWNLOAD_SIMPLE),
        "File",
        "document.save",
    ));
    v.push(cmd("file.showcase", "Load showcase part", "A part made for judging the viewport: curved and flat faces, steps, a thin rib, deep holes and pockets", "", Icon::Glyph(g::CUBE), "File", ""));
    v.push(cmd(
        "file.demo",
        "Load small demo part",
        "A small plate with a boss, a hole and a pocket",
        "",
        Icon::Glyph(g::CUBE),
        "File",
        "",
    ));
    v.push(cmd(
        "file.quit",
        "Quit",
        "Close the application",
        "Ctrl+Q",
        Icon::Glyph(g::X),
        "File",
        "",
    ));
    v.push(cmd(
        "edit.undo",
        "Undo",
        "Undo your last command",
        "Ctrl+Z",
        Icon::Glyph(g::ARROW_U_UP_LEFT),
        "Edit",
        "history.undo",
    ));
    v.push(cmd(
        "edit.redo",
        "Redo",
        "Redo what you undid",
        "Ctrl+Y",
        Icon::Glyph(g::ARROW_U_UP_RIGHT),
        "Edit",
        "history.redo",
    ));
    v.push(cmd(
        "edit.delete",
        "Delete",
        "Delete the selection",
        "Del",
        Icon::Glyph(g::TRASH),
        "Edit",
        "feature.delete",
    ));
    v.push(cmd(
        "app.palette",
        "Search commands",
        "Find any command by typing part of its name",
        "Ctrl+K",
        Icon::Glyph(g::MAGNIFYING_GLASS),
        "Application",
        "",
    ));

    // ---- features --------------------------------------------------------------------------
    for k in FeatureKind::ALL {
        let (id, icon, agent, label) = match k {
            FeatureKind::Extrude => (
                "feature.extrude",
                CadIcon::ExtrudedBoss,
                "feature.extrude",
                "Extruded Boss/Base",
            ),
            FeatureKind::Cut => (
                "feature.cut",
                CadIcon::ExtrudedCut,
                "feature.cut",
                "Extruded Cut",
            ),
            FeatureKind::Boss => (
                "feature.boss",
                CadIcon::Boss,
                "feature.boss",
                "Boss on part",
            ),
            FeatureKind::Revolve => (
                "feature.revolve",
                CadIcon::Rotate,
                "feature.revolve",
                "Revolved Boss/Base",
            ),
            FeatureKind::Fillet => (
                "feature.fillet",
                CadIcon::Fillet,
                "feature.fillet",
                "Fillet",
            ),
            FeatureKind::Chamfer => (
                "feature.chamfer",
                CadIcon::Chamfer,
                "feature.chamfer",
                "Chamfer",
            ),
            FeatureKind::Shell => ("feature.shell", CadIcon::Shell, "feature.shell", "Shell"),
            FeatureKind::Loft => (
                "feature.loft",
                CadIcon::Loft,
                "feature.loft",
                "Lofted Boss/Base",
            ),
            FeatureKind::Sweep => (
                "feature.sweep",
                CadIcon::Sweep,
                "feature.sweep",
                "Swept Boss/Base",
            ),
        };
        v.push(cmd(
            id,
            label,
            k.hint(),
            "",
            Icon::Cad(icon),
            "Features",
            agent,
        ));
    }

    // ---- sketch ----------------------------------------------------------------------------
    v.push(cmd(
        "sketch.new_xy",
        "Sketch on Top plane",
        "Start a sketch on the XY plane",
        "",
        Icon::Cad(CadIcon::Sketch),
        "Sketch",
        "sketch.create",
    ));
    v.push(cmd(
        "sketch.new_xz",
        "Sketch on Front plane",
        "Start a sketch on the XZ plane",
        "",
        Icon::Cad(CadIcon::Sketch),
        "Sketch",
        "sketch.create",
    ));
    v.push(cmd(
        "sketch.new_yz",
        "Sketch on Right plane",
        "Start a sketch on the YZ plane",
        "",
        Icon::Cad(CadIcon::Sketch),
        "Sketch",
        "sketch.create",
    ));
    v.push(cmd(
        "sketch.on_face",
        "Sketch on selected face",
        "Start a sketch on the flat face you selected",
        "",
        Icon::Cad(CadIcon::Sketch),
        "Sketch",
        "sketch.create",
    ));
    v.push(cmd(
        "sketch.edit",
        "Edit sketch",
        "Reopen the selected sketch",
        "",
        Icon::Glyph(g::PENCIL_SIMPLE),
        "Sketch",
        "",
    ));
    v.push(cmd(
        "sketch.finish",
        "Exit sketch",
        "Finish the sketch",
        "",
        Icon::Cad(CadIcon::ExitSketch),
        "Sketch",
        "",
    ));
    v.push(cmd(
        "sketch.select_all",
        "Select all",
        "Select everything in the sketch",
        "Ctrl+A",
        Icon::Glyph(g::SELECTION),
        "Sketch",
        "",
    ));
    for t in Tool::ALL {
        let mut tip = t.hint().to_owned();
        if !t.key().is_empty() {
            tip.push_str(&format!(" (key {})", t.key()));
        }
        v.push(cmd(
            tool_id(t),
            t.name(),
            tip,
            t.key(),
            tool_icon(t),
            "Sketch",
            "sketch.add_entity",
        ));
    }
    for k in ModifyKind::ALL {
        v.push(cmd(
            modify_id(k),
            format!("{}…", k.name()),
            format!("{} the selected sketch geometry", k.name()),
            "",
            modify_icon(k),
            "Sketch",
            "sketch.transform",
        ));
    }
    for r in Relation::ALL {
        v.push(cmd(
            relation_id(r),
            format!("Add relation: {}", r.name()),
            format!("Make the selected geometry {}", r.name().to_lowercase()),
            "",
            Icon::Cad(CadIcon::Relation),
            "Sketch",
            "sketch.add_constraint",
        ));
    }
    v.push(cmd(
        "sketch.fully_define",
        "Fully define sketch",
        "Adds the relations and dimensions still missing",
        "",
        Icon::Cad(CadIcon::FullyDefine),
        "Sketch",
        "sketch.fully_define",
    ));
    v.push(cmd(
        "sketch.check",
        "Check sketch",
        "Finds open contours and other problems",
        "",
        Icon::Cad(CadIcon::CheckSketch),
        "Sketch",
        "sketch.profiles",
    ));
    v.push(cmd(
        "view.nav_next",
        "Next mouse navigation preset",
        "Switch between SolidWorks, Fusion / Onshape and Blender mouse controls",
        "",
        Icon::Cad(CadIcon::Fit),
        "View",
        "",
    ));
    v.push(cmd(
        "evaluate.mass",
        "Mass Properties",
        "Exact volume, surface area and size of the part",
        "",
        Icon::Cad(CadIcon::CheckSketch),
        "Evaluate",
        "feature.summary",
    ));
    v.push(cmd(
        "sketch.repair",
        "Repair sketch",
        "Merges points in the same place, removes empty and repeated things",
        "",
        Icon::Cad(CadIcon::Repair),
        "Sketch",
        "sketch.repair",
    ));
    v.push(cmd("sketch.construction", "Construction geometry", "Turns the selection (or what you draw next) into dashed guides that are not part of the profile", "X", Icon::Cad(CadIcon::Construction), "Sketch", "sketch.set_construction"));
    v.push(cmd(
        "sketch.view_normal",
        "Normal to",
        "Turn the view square on to the sketch",
        "",
        Icon::Cad(CadIcon::NormalTo),
        "Sketch",
        "",
    ));
    v.push(cmd(
        "sketch.view_rotate_left",
        "Rotate view 90° left",
        "Turn the sketch view a quarter turn",
        "",
        Icon::Glyph(g::ARROW_COUNTER_CLOCKWISE),
        "Sketch",
        "",
    ));
    v.push(cmd(
        "sketch.view_rotate_right",
        "Rotate view 90° right",
        "Turn the sketch view a quarter turn",
        "",
        Icon::Glyph(g::ARROW_CLOCKWISE),
        "Sketch",
        "",
    ));
    v.push(cmd(
        "sketch.view_flip",
        "Flip view",
        "Look at the sketch from behind",
        "",
        Icon::Glyph(g::ARROWS_CLOCKWISE),
        "Sketch",
        "",
    ));
    v.push(cmd(
        "sketch.hide_bodies",
        "Hide the part",
        "Hide the solid while sketching",
        "",
        Icon::Glyph(g::EYE_SLASH),
        "Sketch",
        "",
    ));

    // ---- view ------------------------------------------------------------------------------
    v.push(cmd(
        "view.fit",
        "Zoom to fit",
        "Fit the whole part in the window",
        "F",
        Icon::Cad(CadIcon::Fit),
        "View",
        "",
    ));
    for (id, label, key, icon) in [
        ("view.front", "Front", "1", CadIcon::ViewFront),
        ("view.back", "Back", "2", CadIcon::ViewBack),
        ("view.left", "Left", "3", CadIcon::ViewLeft),
        ("view.right", "Right", "4", CadIcon::ViewRight),
        ("view.top", "Top", "5", CadIcon::ViewTop),
        ("view.bottom", "Bottom", "6", CadIcon::ViewBottom),
        ("view.iso", "Isometric", "7", CadIcon::ViewIso),
    ] {
        v.push(cmd(
            id,
            label,
            format!("Look at the part from the {}", label.to_lowercase()),
            key,
            Icon::Cad(icon),
            "View",
            "",
        ));
    }
    v.push(cmd(
        "view.perspective",
        "Perspective",
        "Off (the default) draws without foreshortening, as SolidWorks does",
        "",
        Icon::Glyph(g::PERSPECTIVE),
        "View",
        "",
    ));
    v.push(cmd(
        "view.realview",
        "RealView",
        "Reflections, shadows and ambient occlusion (needs the OpenGL view)",
        "",
        Icon::Glyph(g::SUN),
        "View",
        "",
    ));
    v.push(cmd(
        "view.origins",
        "Origins",
        "Show the model origin",
        "",
        Icon::Cad(CadIcon::Origin),
        "View",
        "",
    ));
    v.push(cmd(
        "view.edges",
        "Edges",
        "Draw the edges of the model",
        "",
        Icon::Glyph(g::BOUNDING_BOX),
        "View",
        "",
    ));
    v.push(cmd(
        "view.sketches",
        "Sketches",
        "Show the sketches of the model",
        "",
        Icon::Glyph(g::PENCIL_SIMPLE),
        "View",
        "",
    ));
    v.push(cmd(
        "view.sketch_grid",
        "Sketch grid",
        "Show the grid of the plane being sketched on",
        "",
        Icon::Glyph(g::GRID_FOUR),
        "View",
        "",
    ));
    v.push(cmd(
        "view.ambient_occlusion",
        "Ambient occlusion",
        "Darkens corners and pockets, as SolidWorks does (GPU view only)",
        "",
        Icon::Glyph(g::CIRCLE_HALF),
        "View",
        "",
    ));
    for (id, label, tip) in [
        (
            "select.faces",
            "Select faces",
            "Clicking the part can pick faces (switch off to click through to edges)",
        ),
        (
            "select.edges",
            "Select edges",
            "Clicking near an edge of the part picks the edge",
        ),
        (
            "select.sketch_lines",
            "Select sketch lines",
            "Clicking a sketch line drawn in the view picks it",
        ),
        (
            "select.feature_first",
            "Select features first",
            "A click picks the feature; a second click on it picks the face or edge (Ctrl+click adds, Ctrl+drag draws a selection box)",
        ),
    ] {
        v.push(cmd(id, label, tip, "", Icon::Glyph(g::CIRCLE), "View", ""));
    }
    v.push(cmd(
        "view.fxaa",
        "Extra smoothing (FXAA)",
        "A final smoothing pass; softens the image a little (GPU view only)",
        "",
        Icon::Glyph(g::CIRCLE),
        "View",
        "",
    ));
    for e in Environment::ALL {
        v.push(cmd(
            environment_id(e),
            format!("Environment: {}", e.name()),
            "What RealView's shiny surfaces reflect",
            "",
            Icon::Glyph(g::SUN),
            "View",
            "",
        ));
    }
    for m in Material::ALL {
        v.push(cmd(
            material_id(m),
            format!("Material: {}", m.name()),
            "The material RealView shows the part in",
            "",
            Icon::Glyph(g::PAINT_BUCKET),
            "View",
            "",
        ));
    }
    v.push(cmd(
        "view.dark_theme",
        "Dark theme",
        "Dark panels and ribbon instead of the light ones",
        "",
        Icon::Glyph(g::CIRCLE_HALF),
        "View",
        "",
    ));
    v.push(cmd(
        "view.floor_reflection",
        "Floor reflection",
        "A faint reflection of the part in the ground (RealView, GPU view only)",
        "",
        Icon::Glyph(g::STACK),
        "View",
        "",
    ));
    v
}

static REGISTRY: OnceLock<Vec<Command>> = OnceLock::new();

/// All commands, in a fixed order.
pub fn all() -> &'static [Command] {
    REGISTRY.get_or_init(build)
}

pub fn find(id: &str) -> Option<&'static Command> {
    all().iter().find(|c| c.id == id)
}

/// Can `id` be run now? `Err` carries the reason, shown in the tooltip of the greyed-out button.
pub fn enabled(id: &str, app: &AppState, host: &dyn Host) -> Result<(), &'static str> {
    let sketching = app.sketching.is_some();
    let need_sketch = || {
        if sketching {
            Ok(())
        } else {
            Err("start or open a sketch first")
        }
    };
    match id {
        "file.save" => app
            .path
            .as_ref()
            .map(|_| ())
            .ok_or("this document has no file yet: use Save as"),
        _ if id.starts_with("view.environment.") || id.starts_with("view.material.") => {
            if host.has_gpu() && app.realview {
                Ok(())
            } else if host.has_gpu() {
                Err("switch RealView on first")
            } else {
                Err("needs the OpenGL viewport")
            }
        }
        "view.realview" | "view.ambient_occlusion" | "view.fxaa" | "view.floor_reflection" => {
            if host.has_gpu() {
                Ok(())
            } else {
                Err("needs the OpenGL viewport")
            }
        }
        "evaluate.mass" => {
            if app.feature_targets().is_empty() {
                Err("make a part first")
            } else {
                Ok(())
            }
        }
        "feature.fillet" | "feature.chamfer" | "feature.shell" => {
            if app.feature_targets().is_empty() {
                Err("make a part first")
            } else {
                Ok(())
            }
        }
        "feature.extrude" | "feature.cut" | "feature.boss" | "feature.revolve" => {
            if sketching {
                Err("finish the sketch first")
            } else if app.extrudable_sketches().is_empty() {
                Err("draw a sketch with a closed profile first")
            } else {
                Ok(())
            }
        }
        "sketch.on_face" => match app.selected {
            Selected::Face { .. } => Ok(()),
            _ => Err("click a flat face in the view first"),
        },
        "sketch.edit" => match &app.selected {
            Selected::Sketch(id) if app.sketching.as_ref().is_none_or(|s| &s.sketch != id) => {
                Ok(())
            }
            Selected::Sketch(_) => Err("this sketch is already open"),
            _ => Err("select a sketch in the tree first"),
        },
        "sketch.finish"
        | "sketch.select_all"
        | "sketch.fully_define"
        | "sketch.check"
        | "sketch.repair"
        | "sketch.construction"
        | "sketch.view_normal"
        | "sketch.view_rotate_left"
        | "sketch.view_rotate_right"
        | "sketch.view_flip"
        | "sketch.hide_bodies" => need_sketch(),
        "edit.delete" => {
            if matches!(app.selected, Selected::None) && !sketching {
                Err("nothing is selected")
            } else {
                Ok(())
            }
        }
        _ if id.starts_with("sketch.tool.") => need_sketch(),
        _ if id.starts_with("sketch.modify.") => {
            need_sketch()?;
            if app
                .sketching
                .as_ref()
                .is_some_and(|s| s.selection.is_empty())
            {
                Err("select some sketch geometry first")
            } else {
                Ok(())
            }
        }
        _ if id.starts_with("sketch.relation.") => {
            need_sketch()?;
            let ok = Relation::ALL
                .iter()
                .find(|r| relation_id(**r) == id)
                .is_some_and(|r| app.available_relations().contains(r));
            if ok {
                Ok(())
            } else {
                Err("select the geometry this relation needs first")
            }
        }
        _ => Ok(()),
    }
}

/// Is the toggle or tool behind `id` switched on? `None` for plain actions.
pub fn checked(id: &str, app: &AppState, host: &dyn Host) -> Option<bool> {
    Some(match id {
        "view.dark_theme" => host.dark_theme(),
        "view.perspective" => app.camera.projection == dc_render::ProjectionKind::Perspective,
        "view.realview" => app.realview,
        "view.origins" => app.show_origins,
        "view.edges" => app.show_edges,
        "view.sketches" => app.show_sketches,
        "view.sketch_grid" => app.show_sketch_grid,
        "view.ambient_occlusion" => app.ambient_occlusion,
        "view.fxaa" => app.fxaa,
        "select.faces" => app.filter.faces,
        "select.edges" => app.filter.edges,
        "select.sketch_lines" => app.filter.sketch_lines,
        "select.feature_first" => app.select_mode == crate::state::SelectMode::FeatureFirst,
        "view.floor_reflection" => app.floor_reflection,
        "sketch.hide_bodies" => app.hide_bodies,
        _ if id.starts_with("view.environment.") => Environment::ALL
            .iter()
            .any(|e| environment_id(*e) == id && *e == app.environment),
        _ if id.starts_with("view.material.") => Material::ALL
            .iter()
            .any(|m| material_id(*m) == id && *m == app.material),
        "sketch.construction" => app.sketching.as_ref().is_some_and(|s| s.construction),
        _ if id.starts_with("sketch.tool.") => {
            let current = app.sketching.as_ref()?.tool;
            Tool::ALL.iter().any(|t| tool_id(*t) == id && *t == current)
        }
        _ => return None,
    })
}

/// Runs `id`. `Err` is a message for the status bar.
pub fn run(id: &str, app: &mut AppState, host: &mut dyn Host) -> Result<(), String> {
    if find(id).is_none() {
        return Err(format!("there is no command '{id}'"));
    }
    enabled(id, app, host).map_err(str::to_owned)?;
    let view = |app: &mut AppState, p: ViewPreset| app.set_view(p);
    match id {
        "file.new" => *app = AppState::new(),
        "file.open" => {
            let cur = app
                .path
                .as_ref()
                .map(|p| p.display().to_string())
                .unwrap_or_default();
            host.ask_file(false, &cur);
        }
        "file.save" => {
            if let Some(p) = app.path.clone() {
                app.save(&p)?;
            }
        }
        "file.save_as" => {
            let cur = app
                .path
                .as_ref()
                .map_or_else(|| "part.dcad".into(), |p| p.display().to_string());
            host.ask_file(true, &cur);
        }
        "file.showcase" => {
            *app = AppState::new();
            app.load_showcase();
        }
        "file.demo" => {
            *app = AppState::new();
            app.load_demo();
        }
        "file.quit" => host.quit(),
        "edit.undo" => {
            app.undo().map_err(|d| d.message)?;
        }
        "edit.redo" => {
            app.redo().map_err(|d| d.message)?;
        }
        "edit.delete" => {
            if app.sketching.is_some() {
                app.sketch_delete_selected();
            } else {
                app.delete_selected().map_err(|d| d.message)?;
            }
        }
        "app.palette" => host.open_palette(),
        "feature.extrude" => {
            app.begin_feature(FeatureKind::Extrude);
        }
        "feature.cut" => {
            app.begin_feature(FeatureKind::Cut);
        }
        "feature.boss" => {
            app.begin_feature(FeatureKind::Boss);
        }
        "feature.revolve" => {
            app.begin_feature(FeatureKind::Revolve);
        }
        "feature.fillet" => {
            app.begin_feature(FeatureKind::Fillet);
        }
        "feature.chamfer" => {
            app.begin_feature(FeatureKind::Chamfer);
        }
        "feature.shell" => {
            app.begin_feature(FeatureKind::Shell);
        }
        "feature.loft" => {
            app.begin_feature(FeatureKind::Loft);
        }
        "feature.sweep" => {
            app.begin_feature(FeatureKind::Sweep);
        }
        "sketch.new_xy" => {
            app.begin_sketch(PlaneSpec::Xy { offset: 0.0 });
        }
        "sketch.new_xz" => {
            app.begin_sketch(PlaneSpec::Xz { offset: 0.0 });
        }
        "sketch.new_yz" => {
            app.begin_sketch(PlaneSpec::Yz { offset: 0.0 });
        }
        "sketch.on_face" => {
            if let Selected::Face { feature, face } = app.selected.clone() {
                app.begin_sketch(PlaneSpec::Face {
                    feature,
                    face,
                    offset: 0.0,
                });
            }
        }
        "sketch.edit" => {
            if let Selected::Sketch(s) = app.selected.clone() {
                app.edit_sketch(&s);
            }
        }
        "sketch.finish" => app.end_sketch(),
        "sketch.select_all" => {
            app.set_tool(Tool::Select);
            app.sketch_select_all();
        }
        "sketch.fully_define" => {
            app.sketch_fully_define();
        }
        "sketch.check" => {
            app.sketch_check();
        }
        "view.nav_next" => {
            let n = host.next_nav();
            app.info(format!("Mouse navigation: {n}"));
        }
        "evaluate.mass" => match app.mass_properties() {
            Some(t) => app.info(t),
            None => return Err("make a part first".into()),
        },
        "sketch.repair" => {
            app.sketch_repair();
        }
        "sketch.construction" => {
            app.toggle_construction();
        }
        "sketch.view_normal" => app.sketch_view_normal(),
        "sketch.view_rotate_left" => app.sketch_view_rotate(-1),
        "sketch.view_rotate_right" => app.sketch_view_rotate(1),
        "sketch.view_flip" => app.sketch_view_flip(),
        "sketch.hide_bodies" => {
            let hide = !app.hide_bodies;
            app.set_hide_bodies(hide);
        }
        "view.dark_theme" => {
            let dark = !host.dark_theme();
            host.set_dark_theme(dark);
        }
        "view.fit" => app.fit(),
        "view.front" => view(app, ViewPreset::Front),
        "view.back" => view(app, ViewPreset::Back),
        "view.left" => view(app, ViewPreset::Left),
        "view.right" => view(app, ViewPreset::Right),
        "view.top" => view(app, ViewPreset::Top),
        "view.bottom" => view(app, ViewPreset::Bottom),
        "view.iso" => view(app, ViewPreset::Iso),
        "view.perspective" => app.toggle_projection(),
        "view.realview" => flip(app, |a| &mut a.realview),
        "view.origins" => flip(app, |a| &mut a.show_origins),
        "view.edges" => flip(app, |a| &mut a.show_edges),
        "view.sketches" => flip(app, |a| &mut a.show_sketches),
        "view.sketch_grid" => flip(app, |a| &mut a.show_sketch_grid),
        "view.ambient_occlusion" => flip(app, |a| &mut a.ambient_occlusion),
        "view.fxaa" => flip(app, |a| &mut a.fxaa),
        "select.faces" => flip(app, |a| &mut a.filter.faces),
        "select.edges" => flip(app, |a| &mut a.filter.edges),
        "select.sketch_lines" => flip(app, |a| &mut a.filter.sketch_lines),
        "select.feature_first" => {
            app.select_mode = match app.select_mode {
                crate::state::SelectMode::Direct => crate::state::SelectMode::FeatureFirst,
                crate::state::SelectMode::FeatureFirst => crate::state::SelectMode::Direct,
            };
            app.view_version += 1;
        }
        "view.floor_reflection" => flip(app, |a| &mut a.floor_reflection),
        _ if id.starts_with("sketch.tool.") => {
            if let Some(t) = Tool::ALL.iter().find(|t| tool_id(**t) == id) {
                app.set_tool(*t);
            }
        }
        _ if id.starts_with("view.environment.") => {
            if let Some(e) = Environment::ALL.iter().find(|e| environment_id(**e) == id) {
                app.environment = *e;
                app.view_version += 1;
            }
        }
        _ if id.starts_with("view.material.") => {
            if let Some(m) = Material::ALL.iter().find(|m| material_id(**m) == id) {
                app.material = *m;
                app.view_version += 1;
            }
        }
        _ if id.starts_with("sketch.modify.") => {
            if let Some(k) = ModifyKind::ALL.iter().find(|k| modify_id(**k) == id) {
                let c = app.selection_centre();
                host.open_modify(*k, c);
            }
        }
        _ if id.starts_with("sketch.relation.") => {
            if let Some(r) = Relation::ALL.iter().find(|r| relation_id(**r) == id) {
                app.apply_relation(*r);
            }
        }
        _ => return Err(format!("the command '{id}' is listed but not wired up")),
    }
    Ok(())
}

fn flip(app: &mut AppState, field: impl FnOnce(&mut AppState) -> &mut bool) {
    let f = field(app);
    *f = !*f;
    app.view_version += 1;
}

/// Commands matching `query`, best first: a prefix of the label, then the start of a word, then
/// the letters of the query in order anywhere (a subsequence), each case-blind. The label, the
/// id and the category are searched. An empty query lists everything in registry order.
pub fn search(query: &str) -> Vec<&'static Command> {
    let q: Vec<char> = query
        .to_lowercase()
        .chars()
        .filter(|c| !c.is_whitespace())
        .collect();
    if q.is_empty() {
        return all().iter().collect();
    }
    let mut scored: Vec<(u32, usize, &Command)> = Vec::new();
    for (i, c) in all().iter().enumerate() {
        let best = [c.label.as_str(), c.id.as_str(), c.category]
            .into_iter()
            .enumerate()
            .filter_map(|(field, text)| score(&q, text).map(|s| s + field as u32 * 10))
            .min();
        if let Some(s) = best {
            scored.push((s, i, c));
        }
    }
    scored.sort_by_key(|(s, i, _)| (*s, *i));
    scored.into_iter().map(|(_, _, c)| c).collect()
}

/// Lower is better; `None` if the letters do not occur in order.
fn score(q: &[char], text: &str) -> Option<u32> {
    let t: Vec<char> = text.to_lowercase().chars().collect();
    if t.starts_with(q) {
        return Some(0);
    }
    // The start of a later word.
    let ts: String = t.iter().collect();
    for (i, _) in ts
        .char_indices()
        .filter(|(_, c)| *c == ' ' || *c == '.' || *c == '_')
    {
        if ts[i + 1..].starts_with(&q.iter().collect::<String>()) {
            return Some(1);
        }
    }
    // Letters in order; a tighter span is better.
    let mut at = 0;
    let (mut first, mut last) = (None, 0);
    for &c in q {
        let pos = t[at..].iter().position(|x| *x == c)? + at;
        first.get_or_insert(pos);
        last = pos;
        at = pos + 1;
    }
    Some(2 + (last - first.unwrap_or(0)) as u32)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;

    /// A host for tests: records what the commands asked for.
    #[derive(Default)]
    struct Spy {
        gpu: bool,
        asked: Vec<(bool, String)>,
        modify: Vec<ModifyKind>,
        palette: bool,
        quit: bool,
        dark: bool,
    }
    impl Host for Spy {
        fn has_gpu(&self) -> bool {
            self.gpu
        }
        fn ask_file(&mut self, save: bool, current: &str) {
            self.asked.push((save, current.to_owned()));
        }
        fn open_modify(&mut self, kind: ModifyKind, _centre: (f64, f64)) {
            self.modify.push(kind);
        }
        fn open_palette(&mut self) {
            self.palette = true;
        }
        fn quit(&mut self) {
            self.quit = true;
        }
        fn dark_theme(&self) -> bool {
            self.dark
        }
        fn set_dark_theme(&mut self, dark: bool) {
            self.dark = dark;
        }
    }

    #[test]
    fn ids_are_unique_and_every_command_is_described() {
        let mut seen = HashSet::new();
        for c in all() {
            assert!(seen.insert(c.id.clone()), "{} listed twice", c.id);
            assert!(
                !c.label.is_empty() && !c.tip.is_empty(),
                "{} lacks text",
                c.id
            );
            assert!(c.id.contains('.') && !c.id.contains(' '), "odd id {}", c.id);
        }
        // Tools and modify operations all have a command.
        for t in Tool::ALL {
            assert!(find(&tool_id(t)).is_some(), "{t:?}");
        }
        for k in ModifyKind::ALL {
            assert!(find(&modify_id(k)).is_some(), "{k:?}");
        }
        for r in Relation::ALL {
            assert!(find(&relation_id(r)).is_some(), "{r:?}");
        }
    }

    #[test]
    fn shortcuts_shown_do_not_clash_and_tool_keys_match_the_tools() {
        let mut keys = HashSet::new();
        for c in all().iter().filter(|c| !c.shortcut.is_empty()) {
            assert!(
                keys.insert(c.shortcut.clone()),
                "{} reuses {}",
                c.id,
                c.shortcut
            );
        }
        for t in Tool::ALL {
            let c = find(&tool_id(t)).unwrap();
            assert_eq!(c.shortcut, t.key(), "{t:?}");
        }
    }

    #[test]
    fn every_command_runs_on_a_fresh_part_without_panicking_and_refusals_say_why() {
        let mut ran = 0;
        for c in all() {
            let mut app = AppState::new();
            app.load_demo();
            let mut host = Spy::default();
            match run(&c.id, &mut app, &mut host) {
                Ok(()) => ran += 1,
                Err(reason) => assert!(!reason.is_empty(), "{} refused without a reason", c.id),
            }
        }
        assert!(ran > 20, "only {ran} commands ran");
    }

    #[test]
    fn commands_do_what_they_say() {
        let mut app = AppState::new();
        app.load_demo();
        let mut host = Spy::default();
        // Sketch tools need a sketch.
        assert!(run("sketch.tool.line", &mut app, &mut host).is_err());
        run("sketch.new_xy", &mut app, &mut host).unwrap();
        assert!(app.sketching.is_some());
        run("sketch.tool.circle", &mut app, &mut host).unwrap();
        assert_eq!(checked("sketch.tool.circle", &app, &host), Some(true));
        assert_eq!(checked("sketch.tool.line", &app, &host), Some(false));
        // Features are refused while sketching, with the reason.
        assert_eq!(
            enabled("feature.extrude", &app, &host),
            Err("finish the sketch first")
        );
        run("sketch.finish", &mut app, &mut host).unwrap();
        assert!(app.sketching.is_none());
        // Toggles flip and report their state; the view version changes so the viewport redraws.
        let v = app.view_version;
        let before = checked("view.edges", &app, &host).unwrap();
        run("view.edges", &mut app, &mut host).unwrap();
        assert_eq!(checked("view.edges", &app, &host), Some(!before));
        assert_ne!(app.view_version, v);
        // RealView needs the GPU.
        assert!(run("view.realview", &mut app, &mut host).is_err());
        host.gpu = true;
        run("view.realview", &mut app, &mut host).unwrap();
        // Undo and redo go through the authority.
        let revision = app.authority.revision();
        run("edit.undo", &mut app, &mut host).unwrap();
        assert!(
            app.authority.revision() > revision,
            "undo is a logged command"
        );
        // Host requests.
        run("file.open", &mut app, &mut host).unwrap();
        run("app.palette", &mut app, &mut host).unwrap();
        run("file.quit", &mut app, &mut host).unwrap();
        assert!(host.palette && host.quit && host.asked.len() == 1 && !host.asked[0].0);
        // Unknown ids are refused by name.
        assert!(run("no.such", &mut app, &mut host)
            .unwrap_err()
            .contains("no.such"));
    }

    #[test]
    fn search_finds_what_a_person_would_type() {
        let top = |q: &str| search(q).first().map(|c| c.id.clone());
        assert_eq!(top("extruded boss").as_deref(), Some("feature.extrude"));
        assert_eq!(top("undo").as_deref(), Some("edit.undo"));
        assert_eq!(top("circ").as_deref(), Some("sketch.tool.circle"));
        // Letters in order across words.
        assert!(search("fdsk").iter().any(|c| c.id == "sketch.fully_define"));
        // The id and the category are searched too.
        assert!(search("sketch.tool.trim")
            .iter()
            .any(|c| c.id == "sketch.tool.trim"));
        assert_eq!(search("").len(), all().len());
        assert!(search("zzzzqq").is_empty());
        // A prefix beats a scattered match.
        let r = search("line");
        assert_eq!(r[0].id, "sketch.tool.line");
    }
}
