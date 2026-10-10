//! Boundary-representation (B-rep) kernel. No UI, no GPU, no sketches: just solids.
//!
//! Index-based topology (`u32` IDs into flat arrays) with exact analytic geometry (planes,
//! cylinders; lines and circular arcs). Every solid can be checked with [`Solid::validate`], and
//! area, volume and bounding box are computed **exactly** from the geometry, not from a mesh.

pub mod blend;
pub mod boolean;
pub mod builder;
pub mod mesh;
pub mod naming;
pub mod round;
pub mod solid;
pub mod validate;

pub use blend::{blend_edges, BlendError, BlendKind};
pub use mesh::{Mesh, MeshError, TessParams};
pub use round::{RimError, RimRound};
pub use solid::{Bounds, CoEdge, Curve, Edge, Face, FaceFrameError, Id, Solid, Surface};
pub use validate::{Issue, IssueCode};
