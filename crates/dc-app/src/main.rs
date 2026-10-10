//! `designcomputer-app [FILE.dcad] [--demo] [--screenshot OUT.png [--size WxH]]`

use dc_app::state::AppState;
use dc_app::ui::{draw, UiState};
use eframe::egui;
use std::path::PathBuf;

struct Args {
    file: Option<PathBuf>,
    demo: bool,
    screenshot: Option<PathBuf>,
    size: (f32, f32),
    software: bool,
    select: Option<String>,
    sketch: Option<String>,
    then: Vec<String>,
    pick: Vec<String>,
    select_box: Option<[f64; 4]>,
    tool: Option<String>,
    hover: Option<[f64; 2]>,
    dim: Option<String>,
    material: Option<String>,
    view: Option<String>,
    feature: Option<String>,
    classic: bool,
    no_ao: bool,
    fxaa: bool,
    zoom: Option<f64>,
    no_reflection: bool,
    env: Option<String>,
    tilt: Option<f64>,
    bench: Option<u32>,
    tab: Option<String>,
    palette: bool,
    scale: Option<f32>,
    icon_sheet: bool,
    dark: bool,
    modify: Option<String>,
}

fn parse_args() -> Result<Args, String> {
    let mut a = Args {
        file: None,
        demo: false,
        screenshot: None,
        size: (1280.0, 800.0),
        software: false,
        select: None,
        sketch: None,
        then: Vec::new(),
        pick: Vec::new(),
        select_box: None,
        tool: None,
        hover: None,
        dim: None,
        material: None,
        view: None,
        feature: None,
        classic: false,
        no_ao: false,
        fxaa: false,
        zoom: None,
        no_reflection: false,
        env: None,
        tilt: None,
        bench: None,
        tab: None,
        palette: false,
        scale: None,
        icon_sheet: false,
        dark: false,
        modify: None,
    };
    let mut it = std::env::args().skip(1);
    while let Some(arg) = it.next() {
        match arg.as_str() {
            "--demo" => a.demo = true,
            "--software" => a.software = true,
            "--classic" => a.classic = true,
            "--no-ao" => a.no_ao = true,
            "--fxaa" => a.fxaa = true,
            "--no-reflection" => a.no_reflection = true,
            "--env" => a.env = it.next(),
            "--tilt" => a.tilt = it.next().and_then(|v| v.parse().ok()),
            "--bench" => a.bench = it.next().and_then(|v| v.parse().ok()),
            "--scale" => a.scale = it.next().and_then(|v| v.parse().ok()),
            "--modify" => a.modify = it.next(),
            "--dark" => a.dark = true,
            "--icon-sheet" => a.icon_sheet = true,
            "--tab" => a.tab = it.next(),
            "--palette" => a.palette = true,
            "--zoom" => a.zoom = it.next().and_then(|v| v.parse().ok()),
            "--sketch" => a.sketch = Some(it.next().ok_or("--sketch needs xy, xz or yz")?),
            "--dim" => a.dim = Some(it.next().ok_or("--dim needs PICK[,PICK]@X,Y")?),
            "--box" => {
                let v: Vec<f64> = it
                    .next()
                    .ok_or("--box needs X0,Y0,X1,Y1 (sketch coordinates)")?
                    .split(',')
                    .filter_map(|p| p.parse().ok())
                    .collect();
                a.select_box = Some(v.try_into().map_err(|_| "--box needs four numbers")?);
            }
            "--tool" => a.tool = Some(it.next().ok_or("--tool needs a tool name")?),
            "--hover" => {
                let v: Vec<f64> = it
                    .next()
                    .ok_or("--hover needs X,Y (sketch coordinates)")?
                    .split(',')
                    .filter_map(|p| p.parse().ok())
                    .collect();
                a.hover = Some(v.try_into().map_err(|_| "--hover needs two numbers")?);
            }
            "--pick" => a.pick.push(it.next().ok_or("--pick needs an entity id")?),
            "--then" => a.then.push(it.next().ok_or("--then needs a console line")?),
            "--feature" => a.feature = Some(it.next().ok_or("--feature needs extrude, cut or boss")?),
            "--material" => a.material = Some(it.next().ok_or("--material needs a name")?),
            "--screenshot" => a.screenshot = Some(PathBuf::from(it.next().ok_or("--screenshot needs a path")?)),
            "--select" => a.select = Some(it.next().ok_or("--select needs FEATURE, FEATURE:FACE or sketch/ID")?),
            "--view" => a.view = Some(it.next().ok_or("--view needs front|back|left|right|top|bottom|iso")?),
            "--size" => {
                let v = it.next().ok_or("--size needs WxH")?;
                let (w, h) = v.split_once('x').ok_or("--size looks like 1280x800")?;
                a.size = (w.parse().map_err(|_| "bad width")?, h.parse().map_err(|_| "bad height")?);
            }
            "-h" | "--help" => return Err("usage: designcomputer-app [FILE.dcad] [--demo] [--software] [--sketch PLANE] [--then LINE] [--select ITEM] [--view NAME] [--screenshot OUT.png] [--size WxH]".into()),
            s if s.starts_with('-') => return Err(format!("unknown option {s}")),
            s => a.file = Some(PathBuf::from(s)),
        }
    }
    Ok(a)
}

/// Captures the real window (UI included) once it has settled, saves it as a PNG and exits.
struct Screenshot {
    path: PathBuf,
    frames: u32,
    requested: bool,
    size: (f32, f32),
}

struct DesignApp {
    state: AppState,
    ui: UiState,
    shot: Option<Screenshot>,
    /// A selection rectangle to show (sketch coordinates), for screenshots.
    select_box: Option<[f64; 4]>,
    /// Where to put the pointer (sketch coordinates), for screenshots of hover previews.
    hover: Option<[f64; 2]>,
    /// `--bench N`: time N frames of a slowly turning view, print the result and exit.
    bench: Option<u32>,
    /// The settings as last written to disk.
    saved: dc_app::settings::Settings,
    /// Agent test hooks (inert unless the `devtools` feature attaches the runtime).
    dev: dc_app::devtools::Dev,
}

impl eframe::App for DesignApp {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        self.ui.hover_override = self.hover.map(|[x, y]| (x, y));
        self.dev.apply_fixtures(&mut self.state);
        {
            let ctx = ui.ctx().clone();
            let _frame_guard = eguidev::FrameGuard::new(&self.dev.handle, &ctx);
            self.dev.tag_epoch(ui);
            draw(&mut self.state, &mut self.ui, ui);
        }
        self.dev.publish(
            &self.state,
            serde_json::json!({
                "ribbon_tab": format!("{:?}", self.ui.ribbon_tab),
                "viewport": self.ui.viewport_rect.map(|r| [r.min.x, r.min.y, r.max.x, r.max.y]),
                "dark": self.ui.dark,
            }),
        );
        let now = dc_app::settings::Settings {
            dark: self.ui.dark,
            show_activity: self.ui.show_activity,
            nav: self.ui.nav,
        };
        if now != self.saved && self.bench.is_none() && !cfg!(feature = "devtools") {
            now.save();
            self.saved = now;
        }
        if let Some(n) = self.bench {
            if self.ui.bench_frames() == 0 {
                self.ui.start_bench();
            }
            if self.ui.bench_frames() >= n as usize + 5 {
                let tris: usize = self
                    .state
                    .scene
                    .bodies
                    .iter()
                    .map(|b| b.mesh.triangles.len())
                    .sum();
                eprintln!("{}\ntriangles: {tris}", self.ui.bench_report());
                std::process::exit(0);
            }
            // Turn the view a little each frame so nothing is cached, and keep drawing.
            self.state.camera.orbit(0.01, 0.0);
            ui.ctx().request_repaint();
        }
        // The rectangle needs the viewport size, which is only known once a frame has been drawn.
        if let (Some([x0, y0, x1, y1]), Some(r)) = (self.select_box, self.ui.viewport_rect) {
            let (w, h) = (f64::from(r.width()), f64::from(r.height()));
            let from = self.state.sketch_to_screen((x0, y0), w, h);
            let to = self.state.sketch_to_screen((x1, y1), w, h);
            if let (Some(from), Some(to), Some(s)) = (from, to, self.state.sketching.as_mut()) {
                s.boxsel = Some(dc_app::selecting::BoxSel { from, to });
            }
        }
        let Some(shot) = &mut self.shot else { return };
        shot.frames += 1;
        let ctx = ui.ctx().clone();
        if shot.frames == 2 {
            // No window manager (headless capture) does not negotiate a size on its own.
            ctx.send_viewport_cmd(egui::ViewportCommand::InnerSize(egui::vec2(
                shot.size.0,
                shot.size.1,
            )));
        }
        if shot.frames >= 25 && !shot.requested && !self.ui.environment_pending() {
            ctx.send_viewport_cmd(egui::ViewportCommand::Screenshot(egui::UserData::default()));
            shot.requested = true;
        }
        let image = ctx.input(|i| {
            i.events.iter().find_map(|e| match e {
                egui::Event::Screenshot { image, .. } => Some(image.clone()),
                _ => None,
            })
        });
        if let Some(img) = image {
            let rgb: Vec<u8> = img
                .pixels
                .iter()
                .flat_map(|p| [p.r(), p.g(), p.b()])
                .collect();
            let png = dc_render::png::encode_rgb(img.size[0] as u32, img.size[1] as u32, &rgb);
            match std::fs::write(&shot.path, png) {
                Ok(()) => eprintln!(
                    "screenshot: {} ({}x{})",
                    shot.path.display(),
                    img.size[0],
                    img.size[1]
                ),
                Err(e) => eprintln!("cannot write {}: {e}", shot.path.display()),
            }
            ctx.send_viewport_cmd(egui::ViewportCommand::Close);
        } else {
            ctx.request_repaint();
        }
    }
}

fn main() -> eframe::Result {
    let args = match parse_args() {
        Ok(a) => a,
        Err(e) => {
            eprintln!("{e}");
            std::process::exit(2);
        }
    };
    let mut state = AppState::new();
    if let Some(f) = &args.file {
        if let Err(e) = state.open(f) {
            eprintln!("{e}");
            std::process::exit(1);
        }
    } else if args.demo {
        state.load_showcase();
    }
    if let Some(p) = &args.sketch {
        let plane = match p.as_str() {
            "xz" => dc_sketch::PlaneSpec::Xz { offset: 0.0 },
            "yz" => dc_sketch::PlaneSpec::Yz { offset: 0.0 },
            _ => dc_sketch::PlaneSpec::Xy { offset: 0.0 },
        };
        state.begin_sketch(plane);
    }
    for line in &args.then {
        state.run_console(line);
    }
    if let (Some(spec), Some(_)) = (&args.dim, state.sketching.as_ref()) {
        // PICK[,PICK]@X,Y: dimension tool, those picks (entity ids, origin, axis-x, axis-y), cursor at X,Y.
        use dc_app::dimensioning::Pick;
        if let Some((picks, at)) = spec.split_once('@') {
            state.set_tool(dc_app::sketching::Tool::Dimension);
            let parsed: Vec<Pick> = picks
                .split(',')
                .map(|p| match p {
                    "origin" => Pick::Origin,
                    "axis-x" => Pick::AxisX,
                    "axis-y" => Pick::AxisY,
                    id => Pick::Entity(id.to_owned()),
                })
                .collect();
            let xy: Vec<f64> = at.split(',').filter_map(|v| v.parse().ok()).collect();
            if let (Some(s), [x, y]) = (state.sketching.as_mut(), xy.as_slice()) {
                s.dim.picks = parsed;
                s.dim.cursor = Some((*x, *y));
            }
        }
    }
    if let Some(t) = &args.tool {
        use dc_app::sketching::Tool;
        if let Some(tool) = Tool::ALL
            .into_iter()
            .find(|x| x.name().eq_ignore_ascii_case(t))
        {
            state.set_tool(tool);
        }
    }
    if let (Some(s), false) = (state.sketching.as_mut(), args.pick.is_empty()) {
        s.selection = args.pick.clone();
    }
    if args.classic {
        state.realview = false;
    }
    if let Some(e) = &args.env {
        use dc_app::environment::Environment;
        state.environment = match e.as_str() {
            "room" => Environment::Room,
            "generated" => Environment::Studio,
            "brown" => Environment::BrownStudio,
            "simple" => Environment::Simple,
            _ => Environment::PhotoStudio,
        };
    }
    if args.no_reflection {
        state.floor_reflection = false;
    }
    if args.fxaa {
        state.fxaa = true;
    }
    if args.no_ao {
        state.ambient_occlusion = false;
    }
    if let Some(k) = &args.feature {
        use dc_app::featuring::FeatureKind;
        if let Some(kind) = FeatureKind::ALL
            .into_iter()
            .find(|x| x.name().eq_ignore_ascii_case(k))
        {
            state.begin_feature(kind);
        }
    }
    if let Some(m) = &args.material {
        use dc_app::state::Material;
        state.material = match m.as_str() {
            "aluminum" | "aluminium" => Material::Aluminum,
            "brass" => Material::Brass,
            "copper" => Material::Copper,
            "plastic" => Material::Plastic,
            "paint" => Material::Paint,
            _ => Material::Steel,
        };
    }
    if let Some(v) = &args.view {
        let preset = match v.as_str() {
            "front" => dc_render::ViewPreset::Front,
            "back" => dc_render::ViewPreset::Back,
            "left" => dc_render::ViewPreset::Left,
            "right" => dc_render::ViewPreset::Right,
            "top" => dc_render::ViewPreset::Top,
            "bottom" => dc_render::ViewPreset::Bottom,
            _ => dc_render::ViewPreset::Iso,
        };
        state.set_view(preset);
    }
    if let Some(deg) = args.tilt {
        // Elevation above the ground in degrees (for screenshots of low angles).
        state.camera.pitch = deg.to_radians();
    }
    if let Some(z) = args.zoom {
        // Multiplies the fitted distance: above 1 zooms out.
        state.camera.distance *= z;
    }
    if let Some(sel) = &args.select {
        use dc_app::state::Selected;
        state.select(match sel.split_once(':') {
            Some((f, face)) => Selected::Face {
                feature: f.into(),
                face: face.into(),
            },
            None => match sel.strip_prefix("sketch/") {
                Some(sk) => Selected::Sketch(sk.into()),
                None => Selected::Feature(sel.clone()),
            },
        });
    }
    let no_gl = args.software;
    let options = eframe::NativeOptions {
        depth_buffer: 24,
        multisampling: 4,
        viewport: egui::ViewportBuilder::default()
            .with_inner_size([args.size.0, args.size.1])
            .with_title("Design Computer")
            .with_app_id("designcomputer"),
        ..Default::default()
    };
    let shot = args.screenshot.map(|path| Screenshot {
        path,
        frames: 0,
        requested: false,
        size: args.size,
    });
    eframe::run_native(
        "Design Computer",
        options,
        Box::new(move |cc| {
            // Fonts, colours and sizes of the interface (light, as SolidWorks).
            dc_app::ui::theme::install(&cc.egui_ctx, dc_app::ui::ThemeMode::Light);
            // A real GL context means the viewport can draw on the GPU.
            if let Some(z) = args.scale {
                // Zoom the interface out (or in): a wider layout in a window of the same pixel size.
                cc.egui_ctx.set_zoom_factor(z);
            }
            let mut ui = if cc.gl.is_some() && !no_gl {
                UiState::with_gl()
            } else {
                UiState::default()
            };
            if let Some(t) = &args.tab {
                use dc_app::ui::ribbon::Tab;
                ui.ribbon_tab = match t.as_str() {
                    "sketch" => Tab::Sketch,
                    "view" => Tab::View,
                    "evaluate" => Tab::Evaluate,
                    _ => Tab::Features,
                };
            }
            ui.icon_sheet = args.icon_sheet;
            // Under test (`devtools`) the user's saved settings are neither read nor written.
            let settings = if cfg!(feature = "devtools") {
                dc_app::settings::Settings::default()
            } else {
                dc_app::settings::Settings::load()
            };
            ui.dark = args.dark || settings.dark;
            ui.show_activity = settings.show_activity;
            ui.nav = settings.nav;
            if let Some(k) = &args.modify {
                use dc_app::ui::{ModifyKind, ModifyPanel};
                let kind = match k.as_str() {
                    "rotate" => ModifyKind::Rotate,
                    "scale" => ModifyKind::Scale,
                    "mirror" => ModifyKind::Mirror,
                    "offset" => ModifyKind::Offset,
                    "fillet" => ModifyKind::Fillet,
                    "chamfer" => ModifyKind::Chamfer,
                    _ => ModifyKind::Move,
                };
                ui.modify = Some(ModifyPanel::new(kind, (0.0, 0.0)));
            }
            if args.palette {
                ui.open_palette();
            }
            let saved = dc_app::settings::Settings {
                dark: ui.dark,
                show_activity: ui.show_activity,
                nav: ui.nav,
            };
            Ok(Box::new(DesignApp {
                dev: dc_app::devtools::Dev::new().attach(),
                saved,
                state,
                ui,
                shot,
                select_box: args.select_box,
                hover: args.hover,
                bench: args.bench,
            }))
        }),
    )
}
