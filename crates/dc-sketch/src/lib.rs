//! 2D parametric sketches: entities, constraints and (see [`solver`]) a constraint solver.
//!
//! Pure data and math; no UI, no document, no I/O. Everything is deterministic: ordered maps,
//! fixed iteration order, and only `dc_math::det` functions.

pub mod edit;
pub mod model;
pub mod placement;
pub mod profile;
pub mod region;
pub mod solver;
mod sparse;
pub mod transform;

pub use edit::EditReport;
pub use model::{
    Constraint, DimStyle, Entity, Kind, PlaneSpec, Sketch, SketchAxis, SketchError, SketchErrorCode,
};
pub use profile::{
    Curve2, Loop, LoopEdge, Profile, ProfileProblem, ProfileProblemCode, ProfileReport,
};
pub use region::{boolean, overlaps, refine, subtract, union, Op, RegionError, Subtracted};
pub use solver::{DragRadius, DragTarget, PointMotion, SolveReport, SolveStatus};
pub use transform::{MirrorAbout, Pattern, Transform};
