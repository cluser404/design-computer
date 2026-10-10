# Bundled environments

`brown_photostudio_02_2k.exr` — "Brown Photostudio 02", 2K, from Poly Haven (polyhaven.com),
published under CC0 (public domain dedication, no attribution required). Downloaded by the
project owner on 2026-10-10; please re-check the licence on the asset's page before
redistributing. Used by RealView as its default reflection environment (`environment.rs`),
decoded by `dc-io::exr` (the `exr` crate; PIZ-compressed, float RGB; pixels pinned against OpenCV in `crates/dc-io/tests/exr_file.rs`).

`photo_studio_01_2k.exr` — "Photo Studio 01", 2K, from Poly Haven, CC0 as well (same caveat:
re-check the licence on the asset's page). The default environment. Copied from the project
owner's Downloads folder on 2026-10-10.
