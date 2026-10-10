//! Parametric features: operations that turn sketches into solids. Sits above `dc-kernel` and
//! `dc-sketch`; the kernel knows nothing about sketches.

pub mod extrude;
pub mod revolve;
pub mod shell;
pub mod solid_ops;
pub mod stepped;

pub use extrude::{extrude, ExtrudeError};
pub use revolve::{revolve, revolve_full, revolve_partial};
pub use shell::{shell_ends, shell_prism, ShellError};
pub use stepped::{build_solid, Level, RimSpec, Role, Round, StepError, Stepped};
