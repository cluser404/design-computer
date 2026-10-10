//! Cuts and bosses on a finished solid through the general boolean, for directions and parts the
//! layered (stepped) model cannot hold: a sketch on a side face, a slanted plane, a part that
//! already has features across its axis. The tool is the extruded profile; the result is
//! validated by the kernel or refused with the reason.

use crate::extrude::{extrude, ExtrudeError};
use dc_kernel::boolean::{boolean, BoolError, BoolOp, BoolOptions};
use dc_kernel::Solid;
use dc_math::{Plane, Vec3};
use dc_sketch::Profile;
use std::fmt;
use std::sync::Arc;

#[derive(Debug, Clone, PartialEq)]
pub enum ToolError {
    /// The tool body could not be built from the profile.
    Tool(String),
    /// The boolean refused or failed.
    Bool(BoolError),
    /// The operation changes nothing (the cut misses the part).
    NoEffect(String),
    /// A boss that does not touch the part.
    Floating(String),
}

impl fmt::Display for ToolError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ToolError::Tool(s) | ToolError::NoEffect(s) | ToolError::Floating(s) => {
                write!(f, "{s}")
            }
            ToolError::Bool(e) => write!(f, "{e}"),
        }
    }
}

impl std::error::Error for ToolError {}

impl From<ExtrudeError> for ToolError {
    fn from(e: ExtrudeError) -> Self {
        ToolError::Tool(e.to_string())
    }
}

/// How far (along the plane's normal) the solid reaches on either side of the plane, plus a
/// margin: the length of a "through all" tool.
pub fn reach(target: &Solid, plane: &Plane) -> f64 {
    let n = plane.normal();
    let mut far: f64 = 0.0;
    if let Some(b) = target.bounds() {
        for x in [b.min.x, b.max.x] {
            for y in [b.min.y, b.max.y] {
                for z in [b.min.z, b.max.z] {
                    far = far.max(((Vec3::new(x, y, z) - plane.origin).dot(n)).abs());
                }
            }
        }
    }
    far + 1.0
}

/// The tool body: `profile` on `plane`, from offset `lo` to `hi` along the plane's normal.
pub fn tool_solid(plane: &Plane, profile: &Profile, lo: f64, hi: f64) -> Result<Solid, ToolError> {
    Ok(extrude(&plane.offset(lo), profile, hi - lo, false)?)
}

fn options(owner: &str, kind: &str) -> BoolOptions {
    let owner = owner.to_owned();
    let kind = kind.to_owned();
    BoolOptions {
        b_prefix: String::new(),
        b_namer: Some(Arc::new(move |name: &str| {
            // Faces of the tool: `cap_start`, `cap_end`, `side:<entity>`.
            match name {
                "cap_start" | "cap_end" => format!("{name}:{owner}"),
                other => match other.strip_prefix("side:") {
                    Some(e) => format!("side:{kind}:{owner}:{e}"),
                    None => format!("{kind}:{owner}:{other}"),
                },
            }
        })),
    }
}

/// `target` with `tool` removed. `owner` names the new faces.
pub fn cut_with_tool(target: &Solid, tool: &Solid, owner: &str) -> Result<Solid, ToolError> {
    let before = target.volume();
    match boolean(target, tool, BoolOp::Subtract, &options(owner, "cut")) {
        Ok(r) => {
            if before - r.solid.volume() <= 1e-9 * before.abs().max(1.0) {
                return Err(ToolError::NoEffect(
                    "the cut does not remove any material (it misses the part or only touches it)"
                        .into(),
                ));
            }
            Ok(r.solid)
        }
        Err(BoolError::Empty) => Err(ToolError::Bool(BoolError::Empty)),
        Err(e) => Err(ToolError::Bool(e)),
    }
}

/// `target` with `tool` added. The tool must touch the part: a boss that floats is refused.
pub fn boss_with_tool(target: &Solid, tool: &Solid, owner: &str) -> Result<Solid, ToolError> {
    match boolean(target, tool, BoolOp::Union, &options(owner, "boss")) {
        Ok(r) => {
            if r.solid.shell_count() > target.shell_count() {
                return Err(ToolError::Floating(
                    "the boss does not touch the part (it would float free)".into(),
                ));
            }
            if r.solid.volume() - target.volume() <= 1e-9 * target.volume().abs().max(1.0) {
                return Err(ToolError::NoEffect(
                    "the boss adds no material (it lies inside the part)".into(),
                ));
            }
            Ok(r.solid)
        }
        Err(e) => Err(ToolError::Bool(e)),
    }
}
