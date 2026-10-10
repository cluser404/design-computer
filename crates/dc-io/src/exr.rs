//! OpenEXR images (the format HDR environment maps come in), read with the `exr` crate
//! (BSD-3-Clause): it handles every compression OpenEXR has and decodes in parallel, about ten
//! times faster than the PIZ-only reader this replaced (30 ms against 300 ms for a 2K
//! panorama, identical pixels). Kept as a thin wrapper so the rest of the code does not
//! depend on the crate's types.

use std::fmt;

use exr::prelude::{ReadChannels, ReadLayers};

/// A decoded image: linear RGB, row 0 at the top.
#[derive(Clone, Debug)]
pub struct Image {
    pub width: usize,
    pub height: usize,
    /// `width * height * 3` floats.
    pub rgb: Vec<f32>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ExrError(pub String);

impl fmt::Display for ExrError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "OpenEXR: {}", self.0)
    }
}

impl std::error::Error for ExrError {}

/// Decodes the first layer of an OpenEXR file at its largest resolution.
pub fn read(data: &[u8]) -> Result<Image, ExrError> {
    let img = exr::prelude::read()
        .no_deep_data()
        .largest_resolution_level()
        .rgba_channels(
            |res, _| {
                (
                    res.width(),
                    res.height(),
                    vec![0f32; res.width() * res.height() * 3],
                )
            },
            |(w, h, rgb), pos, (r, g, b, _a): (f32, f32, f32, f32)| {
                if pos.x() < *w && pos.y() < *h {
                    let i = (pos.y() * *w + pos.x()) * 3;
                    rgb[i] = r;
                    rgb[i + 1] = g;
                    rgb[i + 2] = b;
                }
            },
        )
        .first_valid_layer()
        .all_attributes()
        .from_buffered(std::io::Cursor::new(data))
        .map_err(|e| ExrError(e.to_string()))?;
    let (width, height, rgb) = img.layer_data.channel_data.pixels;
    Ok(Image { width, height, rgb })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn garbage_is_refused_not_trusted() {
        assert!(read(b"not an exr").is_err());
        assert!(read(&[0x76, 0x2f, 0x31, 0x01, 2, 0, 0, 0]).is_err());
        let h = [
            0x76, 0x2f, 0x31, 0x01, 2, 0, 0, 0, b'c', b'h', 0, b'x', 0, 9, 0, 0, 0,
        ];
        for n in 0..h.len() {
            assert!(read(&h[..n]).is_err());
        }
    }
}
