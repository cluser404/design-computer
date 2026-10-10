// The reference values are exact half-precision numbers, written out in full on purpose.
#![allow(clippy::excessive_precision)]

//! The bundled HDR environment decodes to exactly the pixels an independent reader (OpenCV)
//! gave when the decoder was written; the expected values below were taken from it.

use dc_io::exr;

const FILE: &[u8] = include_bytes!("../../dc-app/assets/environments/brown_photostudio_02_2k.exr");

#[test]
fn the_bundled_environment_decodes_to_the_reference_pixels() {
    let img = exr::read(FILE).expect("decodes");
    assert_eq!((img.width, img.height), (2048, 1024));
    assert_eq!(img.rgb.len(), 2048 * 1024 * 3);
    assert!(img.rgb.iter().all(|v| v.is_finite() && *v >= 0.0));
    // (row, column, rgb), including the brightest light (values far above 1).
    let reference: [(usize, usize, [f32; 3]); 9] = [
        (0, 0, [0.296875, 0.283203125, 0.259765625]),
        (1023, 2047, [0.55078125, 0.55078125, 0.55078125]),
        (512, 1024, [0.06884765625, 0.0791015625, 0.029296875]),
        (300, 700, [0.2080078125, 0.1865234375, 0.17578125]),
        (100, 1500, [0.267578125, 0.2578125, 0.2578125]),
        (700, 200, [0.35546875, 0.3359375, 0.3046875]),
        (1023, 0, [0.55078125, 0.55078125, 0.55078125]),
        (250, 1111, [0.025146484375, 0.029052734375, 0.031982421875]),
        (263, 1267, [107.5, 111.5, 110.5]),
    ];
    for (y, x, want) in reference {
        let i = (y * 2048 + x) * 3;
        assert_eq!(img.rgb[i..i + 3], want, "pixel at row {y}, column {x}");
    }
}

#[test]
fn a_damaged_file_is_refused_cleanly() {
    // Cutting the file anywhere (or flipping bytes) must give an error or an image, never a panic.
    for cut in [100, 400, 700, 5000, FILE.len() / 2, FILE.len() - 1] {
        let _ = exr::read(&FILE[..cut]);
    }
    let mut bad = FILE.to_vec();
    for i in (800..bad.len()).step_by(9973) {
        bad[i] ^= 0x5a;
    }
    let _ = exr::read(&bad);
}
