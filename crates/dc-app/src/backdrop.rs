//! The viewport background: white with a soft lavender band across the middle, as in
//! SolidWorks' "three point faded" scene. It is not a plain top-to-bottom gradient: measured
//! on a screenshot of that program the band is centred a little above half height and rises
//! towards the right, fades gently upwards (much further on the right, where it reaches the
//! top edge), and falls off quickly below, to white. The numbers are a least-squares fit of
//! that model to the screenshot (rms error about 1 of 255 levels, worst sample 6).
//!
//! The same formula is in the GL shader (`gl.rs`, `BG_FS`); `background_at` is the reference
//! the fallback mesh uses and the test pins.

/// Colour of the empty background at `(u, v)`: u from the left, v from the top, both 0..1.
pub fn background_at(u: f32, v: f32) -> [f32; 3] {
    const WHITE: [f32; 3] = [254.0, 254.0, 254.0];
    const PEAK: [f32; 3] = [218.0, 222.0, 233.0];
    // Centre line of the band: v = C0 + (C1 - C0) * u.
    const C0: f32 = 0.5298;
    const C1: f32 = 0.4889;
    // How far below the centre line it takes to reach white.
    const DOWN: f32 = 0.2028;
    // How far above it fades: E0 on the left, E0 + E1 on the right (between M0 and M1).
    const E0: f32 = 0.246;
    const E1: f32 = 1.0032;
    const M0: f32 = 0.3656;
    const M1: f32 = 1.3;
    // Strength on the left (it is 1 on the right) and the shape of the upper fade.
    const A0: f32 = 0.6665;
    const SHAPE: f32 = 0.8653;
    let smooth = |a: f32, b: f32, x: f32| {
        let t = ((x - a) / (b - a)).clamp(0.0, 1.0);
        t * t * (3.0 - 2.0 * t)
    };
    let d = v - (C0 + (C1 - C0) * u);
    let intensity = if d > 0.0 {
        1.0 - smooth(0.0, DOWN, d)
    } else {
        let e = E0 + E1 * smooth(M0, M1, u);
        let t = (-d / e).clamp(0.0, 1.0);
        (1.0 - smooth(0.0, 1.0, t)).powf(SHAPE)
    } * (A0 + (1.0 - A0) * u);
    [0, 1, 2].map(|i| (WHITE[i] + (PEAK[i] - WHITE[i]) * intensity) / 255.0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn it_matches_the_pixels_measured_on_the_solidworks_screenshot() {
        // (x, y, colour) on a 1251 x 859 screenshot.
        let samples: [(f32, f32, [f32; 3]); 12] = [
            (60.0, 420.0, [232.0, 234.0, 241.0]),
            (60.0, 180.0, [254.0, 254.0, 254.0]),
            (300.0, 300.0, [247.0, 248.0, 250.0]),
            (300.0, 420.0, [228.0, 231.0, 239.0]),
            (600.0, 360.0, [228.0, 231.0, 239.0]),
            (600.0, 420.0, [224.0, 228.0, 236.0]),
            (900.0, 60.0, [250.0, 251.0, 252.0]),
            (900.0, 240.0, [232.0, 234.0, 241.0]),
            (900.0, 540.0, [244.0, 245.0, 248.0]),
            (1180.0, 60.0, [229.0, 231.0, 239.0]),
            (1180.0, 360.0, [218.0, 222.0, 233.0]),
            (600.0, 720.0, [254.0, 254.0, 254.0]),
        ];
        for (x, y, want) in samples {
            let got = background_at(x / 1251.0, y / 859.0).map(|c| c * 255.0);
            for c in 0..3 {
                assert!(
                    (got[c] - want[c]).abs() <= 7.0,
                    "at ({x}, {y}) channel {c}: {got:?} against {want:?}"
                );
            }
        }
    }

    #[test]
    fn it_is_white_at_the_edges_and_never_darker_than_the_peak() {
        for i in 0..=40 {
            for j in 0..=40 {
                let c = background_at(i as f32 / 40.0, j as f32 / 40.0);
                for v in c {
                    assert!(
                        (217.0 / 255.0..=254.0 / 255.0 + 1e-4).contains(&v),
                        "{i},{j}: {c:?}"
                    );
                }
            }
        }
        assert_eq!(background_at(0.5, 1.0), [254.0 / 255.0; 3]);
        assert_eq!(background_at(0.05, 0.1), [254.0 / 255.0; 3]);
    }
}
