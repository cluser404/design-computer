//! Deterministic math foundation for Design Computer.
//!
//! Everything in the kernel, solver and features builds on this crate. It exists to uphold the
//! determinism contract (AGENTS.md §4.5): the same inputs give bit-identical outputs on every
//! supported OS/CPU. Do not call `f64::sin` & co. directly; use [`det`].

pub mod det;
pub mod plane;
pub mod robust;
pub mod tol;
pub mod vec;

pub use plane::Plane;
pub use tol::Tolerance;
pub use vec::{Vec2, Vec3};
