//! STL export. Writes only into a directory the *host* approved at launch (`--export-dir`), under a
//! strictly validated file name, so a client (an agent) can never choose where on disk to write.

use dc_doc::{Authority, DiagCode, Diagnostic, Hash128};
use dc_kernel::TessParams;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Default, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum StlFormat {
    #[default]
    Binary,
    Ascii,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct ExportStlRequest {
    /// The feature whose solid to export.
    pub feature: String,
    /// A plain file name ending in `.stl` (letters, digits, `_ - .`); no directories.
    pub file_name: String,
    #[serde(default)]
    pub format: StlFormat,
    /// Maximum distance between true curves and the mesh, in mm (0.0001 to 1). Default 0.01.
    pub chord_tol: Option<f64>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct ExportStlResult {
    pub path: String,
    pub triangles: usize,
    pub bytes: usize,
    /// Hash of the exact bytes written.
    pub digest: Hash128,
    /// Volume of the exported mesh and of the exact B-rep, so the approximation error is visible.
    pub mesh_volume: f64,
    pub exact_volume: f64,
}

/// Why an export did not happen (all are reported to the client as structured diagnostics).
pub(crate) fn export_stl(
    authority: &Authority,
    dir: Option<&Path>,
    req: &ExportStlRequest,
) -> Result<ExportStlResult, Diagnostic> {
    let Some(dir) = dir else {
        return Err(Diagnostic::new(
            DiagCode::PermissionDenied,
            "export is disabled: the host did not configure an export directory",
        )
        .suggest("ask the person running the app to start it with --export-dir DIR"));
    };
    validate_file_name(&req.file_name, ".stl")?;
    let tol = req.chord_tol.unwrap_or(0.01);
    if !(0.0001..=1.0).contains(&tol) {
        return Err(Diagnostic::new(
            DiagCode::InvalidValue,
            format!("chord_tol must be between 0.0001 and 1 mm, got {tol}"),
        ));
    }
    let solid = authority.solid(&req.feature).ok_or_else(|| {
        Diagnostic::new(
            DiagCode::NotFound,
            format!("feature '{}' does not exist", req.feature),
        )
        .entity(format!("feature/{}", req.feature))
    })?;
    let mesh = solid
        .tessellate(TessParams {
            chord_tol: tol,
            ..TessParams::default()
        })
        .map_err(|e| Diagnostic::new(DiagCode::FeatureFailed, e.to_string()))?;
    let problems = mesh.check();
    if !problems.is_empty() {
        return Err(Diagnostic::new(
            DiagCode::FeatureFailed,
            format!(
                "refusing to export a mesh that is not watertight: {}",
                problems[0]
            ),
        )
        .suggest("this is a kernel bug; please report the model that triggered it"));
    }
    let bytes = match req.format {
        StlFormat::Binary => mesh.to_binary_stl(),
        StlFormat::Ascii => mesh
            .to_ascii_stl(req.file_name.trim_end_matches(".stl"))
            .into_bytes(),
    };
    let target = dir.join(&req.file_name);
    // Write to a temporary file, then rename: readers never see a half-written STL.
    let tmp: PathBuf = dir.join(format!(".{}.tmp", req.file_name));
    let io_err = |what: &str, e: std::io::Error| {
        Diagnostic::new(
            DiagCode::FeatureFailed,
            format!("could not {what} '{}': {e}", target.display()),
        )
    };
    let mut f = fs::File::create(&tmp).map_err(|e| io_err("write", e))?;
    f.write_all(&bytes)
        .and_then(|()| f.sync_all())
        .map_err(|e| io_err("write", e))?;
    fs::rename(&tmp, &target).map_err(|e| io_err("finalize", e))?;
    Ok(ExportStlResult {
        path: target.display().to_string(),
        triangles: mesh.triangles.len(),
        bytes: bytes.len(),
        digest: Hash128::of(&[&bytes]),
        mesh_volume: mesh.volume(),
        exact_volume: solid.volume(),
    })
}

pub(crate) fn validate_file_name(name: &str, ext: &str) -> Result<(), Diagnostic> {
    let ok = name.len() <= 100
        && name.len() > ext.len()
        && name.ends_with(ext)
        && !name.starts_with('.')
        && !name.contains("..")
        && name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '-' | '.'));
    if ok {
        Ok(())
    } else {
        Err(Diagnostic::new(
            DiagCode::InvalidName,
            format!("invalid file name {name:?}"),
        )
        .suggest(format!(
            "use a plain name ending in {ext}, like part-1{ext}: letters, digits, '_', '-', '.'; no folders"
        )))
    }
}
