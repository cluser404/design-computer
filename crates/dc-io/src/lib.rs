//! File formats. Today: the native `.dcad` container (see [`dcad`]).

pub mod dcad;
pub mod exr;
mod zip;

pub use dcad::{read, write, Dcad, DcadError, Manifest, FORMAT_VERSION};
