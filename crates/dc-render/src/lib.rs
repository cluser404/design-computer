//! Rendering without a GPU: cameras, a deterministic software rasterizer, PNG output, picking.
//!
//! The same code draws the interactive viewport and the headless images agents ask for
//! (AGENTS.md §4.4: visual parity), and its output is plain data that tests can inspect pixel by
//! pixel. A GPU renderer can later replace the rasterizer behind the same [`Scene`] and
//! [`Camera`] types for large assemblies.

pub mod b64;
pub mod camera;
pub mod pick;
pub mod png;
pub mod raster;
pub mod scene;

pub use camera::{Camera, Projected, ProjectionKind, ViewPreset};
pub use pick::{box_hits, pick, pick_hit, Filter, Hit, Pick, PixelRect};
pub use raster::FaceRef;
pub use raster::{render, Image, LegendEntry, RenderMode, RenderOptions, RenderOutput};
pub use scene::{scene_of_authority, Body, Polyline, Scene};
