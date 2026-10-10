//! `render.view`: what the viewport shows, as an image an agent can look at, plus the data to
//! interpret it (a legend mapping colours to faces, and the exact camera used).

use dc_doc::{Authority, DiagCode, Diagnostic};
use dc_math::Vec3;
use dc_render::{
    b64, render, scene_of_authority, Camera, FaceRef, ProjectionKind, RenderMode, RenderOptions,
    ViewPreset,
};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Default, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum View {
    Front,
    Back,
    Left,
    Right,
    Top,
    Bottom,
    #[default]
    Iso,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Default, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum Mode {
    /// Lit surfaces, edges, sketches and a grid: what a person sees.
    #[default]
    Shaded,
    /// Every face a distinct flat colour; the `legend` says which is which. Use this to find out
    /// which face is at a pixel, or to learn face names without guessing.
    FaceIds,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Default, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum Proj {
    #[default]
    Perspective,
    Orthographic,
}

/// An explicit camera, instead of a standard view fitted to the part.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct CameraSpec {
    pub target: [f64; 3],
    pub distance: f64,
    /// Rotation about +Z in degrees. Eye = target + distance * (cos p cos y, cos p sin y, sin p).
    pub yaw_deg: f64,
    /// Elevation above the XY plane in degrees (-90..90).
    pub pitch_deg: f64,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct RenderRequest {
    /// Image width in pixels (default 640, at most 1600).
    pub width: Option<u32>,
    /// Image height in pixels (default 480, at most 1200).
    pub height: Option<u32>,
    /// A standard view, fitted to the part (default iso). Ignored if `camera` is given.
    #[serde(default)]
    pub view: View,
    #[serde(default)]
    pub projection: Proj,
    pub camera: Option<CameraSpec>,
    #[serde(default)]
    pub mode: Mode,
    /// Draw edges (default true; shaded mode only).
    pub edges: Option<bool>,
    pub grid: Option<bool>,
    /// Darken corners and pockets, as the app's ambient occlusion does (default false).
    pub ambient_occlusion: Option<bool>,
    /// Draw sketches as coloured outlines on their planes (default true).
    pub sketches: Option<bool>,
    /// Highlight one face in orange. `feature` is the body's feature ID.
    pub highlight: Option<HighlightSpec>,
    /// Also write the PNG into the host's output folder under this plain name ending `.png`.
    pub save_as: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct HighlightSpec {
    pub feature: String,
    pub face: String,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct LegendItem {
    /// `#rrggbb`
    pub color: String,
    pub feature: String,
    pub face: String,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct CameraReport {
    pub target: [f64; 3],
    pub distance: f64,
    pub yaw_deg: f64,
    pub pitch_deg: f64,
    pub projection: String,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct RenderResult {
    pub width: u32,
    pub height: u32,
    /// The PNG, base64 encoded.
    pub png_base64: String,
    pub bytes: usize,
    pub camera: CameraReport,
    /// Face-id mode only: colour to face. Empty otherwise.
    pub legend: Vec<LegendItem>,
    /// The features drawn (final bodies only; intermediate stages are not shown twice).
    pub features: Vec<String>,
    pub saved_to: Option<String>,
}

fn hex(c: [u8; 3]) -> String {
    format!("#{:02x}{:02x}{:02x}", c[0], c[1], c[2])
}

pub(crate) fn render_view(
    authority: &Authority,
    out_dir: Option<&std::path::Path>,
    req: &RenderRequest,
) -> Result<RenderResult, Diagnostic> {
    let (w, h) = (req.width.unwrap_or(640), req.height.unwrap_or(480));
    if !(16..=1600).contains(&w) || !(16..=1200).contains(&h) {
        return Err(Diagnostic::new(
            DiagCode::InvalidValue,
            format!("image size {w}x{h} is out of range (16..1600 by 16..1200)"),
        ));
    }
    let scene = scene_of_authority(authority);
    let projection = match req.projection {
        Proj::Perspective => ProjectionKind::Perspective,
        Proj::Orthographic => ProjectionKind::Orthographic,
    };
    let mut cam = match &req.camera {
        Some(c) => {
            if !c.distance.is_finite()
                || c.distance <= 0.0
                || c.target.iter().any(|v| !v.is_finite())
            {
                return Err(Diagnostic::new(
                    DiagCode::InvalidValue,
                    "camera target and distance must be finite, and distance positive",
                ));
            }
            let mut cam = Camera::preset(ViewPreset::Iso);
            cam.target = Vec3::new(c.target[0], c.target[1], c.target[2]);
            cam.distance = c.distance;
            cam.yaw = c.yaw_deg.to_radians();
            cam.pitch = c
                .pitch_deg
                .to_radians()
                .clamp(-core::f64::consts::FRAC_PI_2, core::f64::consts::FRAC_PI_2);
            cam
        }
        None => {
            let mut cam = Camera::preset(match req.view {
                View::Front => ViewPreset::Front,
                View::Back => ViewPreset::Back,
                View::Left => ViewPreset::Left,
                View::Right => ViewPreset::Right,
                View::Top => ViewPreset::Top,
                View::Bottom => ViewPreset::Bottom,
                View::Iso => ViewPreset::Iso,
            });
            if let Some((lo, hi)) = scene.body_bounds().or_else(|| scene.bounds()) {
                cam.fit_for(lo, hi, f64::from(w) / f64::from(h));
            }
            cam
        }
    };
    cam.projection = projection;

    let mut o = RenderOptions::new(w, h, cam);
    o.mode = match req.mode {
        Mode::Shaded => RenderMode::Shaded,
        Mode::FaceIds => RenderMode::FaceIds,
    };
    o.edges = req.edges.unwrap_or(true);
    o.grid = req.grid.unwrap_or(true);
    o.ambient_occlusion = req.ambient_occlusion.unwrap_or(false);
    o.sketches = req.sketches.unwrap_or(true);
    o.supersample = 2;
    o.selected = req.highlight.as_ref().map(|h| FaceRef {
        body: h.feature.clone(),
        face: h.face.clone(),
    });
    let out = render(&scene, &o);
    let png = out.image.to_png();

    let saved_to = match &req.save_as {
        None => None,
        Some(name) => {
            let Some(dir) = out_dir else {
                return Err(Diagnostic::new(DiagCode::PermissionDenied, "saving images is disabled: the host did not configure an output directory")
                    .suggest("ask the person running the app to start it with --export-dir DIR, or omit save_as"));
            };
            crate::export::validate_file_name(name, ".png")?;
            let target = dir.join(name);
            std::fs::write(&target, &png).map_err(|e| {
                Diagnostic::new(
                    DiagCode::FeatureFailed,
                    format!("could not write '{}': {e}", target.display()),
                )
            })?;
            Some(target.display().to_string())
        }
    };
    Ok(RenderResult {
        width: w,
        height: h,
        bytes: png.len(),
        png_base64: b64::encode(&png),
        camera: CameraReport {
            target: [cam.target.x, cam.target.y, cam.target.z],
            distance: cam.distance,
            yaw_deg: cam.yaw.to_degrees(),
            pitch_deg: cam.pitch.to_degrees(),
            projection: match projection {
                ProjectionKind::Perspective => "perspective".into(),
                ProjectionKind::Orthographic => "orthographic".into(),
            },
        },
        legend: out
            .legend
            .into_iter()
            .map(|l| LegendItem {
                color: hex(l.color),
                feature: l.body,
                face: l.face,
            })
            .collect(),
        features: scene.bodies.iter().map(|b| b.id.clone()).collect(),
        saved_to,
    })
}
