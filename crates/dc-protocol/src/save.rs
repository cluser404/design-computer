//! Saving the document as a native `.dcad` file, under the same host-controlled rules as export.

use crate::export::validate_file_name;
use dc_doc::{Authority, DiagCode, Diagnostic, Hash128, Revision};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::fs;
use std::io::Write;
use std::path::Path;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct SaveRequest {
    /// A plain file name ending in `.dcad` (letters, digits, `_ - .`); no directories.
    pub file_name: String,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct SaveResult {
    pub path: String,
    pub bytes: usize,
    /// Hash of the exact bytes written.
    pub digest: Hash128,
    /// The revision and state hash the file records.
    pub revision: Revision,
    pub state_hash: Hash128,
}

pub(crate) fn save_dcad(
    authority: &Authority,
    extras: &BTreeMap<String, Vec<u8>>,
    dir: Option<&Path>,
    req: &SaveRequest,
) -> Result<SaveResult, Diagnostic> {
    let Some(dir) = dir else {
        return Err(Diagnostic::new(
            DiagCode::PermissionDenied,
            "saving is disabled: the host did not configure an output directory",
        )
        .suggest("ask the person running the app to start it with --export-dir DIR"));
    };
    validate_file_name(&req.file_name, ".dcad")?;
    let bytes = dc_io::write(authority, extras);
    let target = dir.join(&req.file_name);
    let tmp = dir.join(format!(".{}.tmp", req.file_name));
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
    Ok(SaveResult {
        path: target.display().to_string(),
        bytes: bytes.len(),
        digest: Hash128::of(&[&bytes]),
        revision: authority.revision(),
        state_hash: authority.state_hash(),
    })
}
