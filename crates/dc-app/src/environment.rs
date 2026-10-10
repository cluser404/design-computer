//! The surroundings that RealView's shiny surfaces reflect: a panoramic image of a room, made
//! here with plain formulas (no image files), so a polished surface shows windows, lamps, a
//! horizon and a floor instead of abstract bands. The direction convention is the model's:
//! z up, azimuth measured from +x towards +y.

use core::f64::consts::{FRAC_PI_2, PI};
use dc_math::det;

/// Which surroundings RealView reflects.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Environment {
    /// A photographed studio (CC0, Poly Haven "Photo Studio 01"), bundled with the app.
    #[default]
    PhotoStudio,
    /// Another photographed studio (CC0, Poly Haven "Brown Photostudio 02"), bundled too.
    BrownStudio,
    /// The plain studio made of a few mathematical light shapes (the earlier look).
    Simple,
    /// A made-up studio: big softboxes with dividers, ceiling panels, a bright floor.
    Studio,
    /// A room with a large window onto a bright sky, a small window and a lamp.
    Room,
}

impl Environment {
    pub const ALL: [Environment; 5] = [
        Environment::PhotoStudio,
        Environment::BrownStudio,
        Environment::Studio,
        Environment::Room,
        Environment::Simple,
    ];

    pub fn name(self) -> &'static str {
        match self {
            Environment::Simple => "Simple studio",
            Environment::PhotoStudio => "Photo studio (HDRI)",
            Environment::BrownStudio => "Brown photo studio (HDRI)",
            Environment::Studio => "Studio (generated)",
            Environment::Room => "Room with window",
        }
    }

    /// Does this environment use the panorama (the simple one is worked out in the shader)?
    pub fn has_image(self) -> bool {
        self != Environment::Simple
    }
}

/// Panorama size: twice as wide as high, as the equirectangular layout wants.
pub const WIDTH: usize = 512;
pub const HEIGHT: usize = 256;

fn smooth(a: f64, b: f64, x: f64) -> f64 {
    let t = ((x - a) / (b - a)).clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

/// A soft-edged rectangle in (azimuth, elevation), degrees: 1 inside, fading over `soft`.
fn panel(az: f64, el: f64, az0: f64, az1: f64, el0: f64, el1: f64, soft: f64) -> f64 {
    // Azimuth wraps: measure the distance from the middle of the span.
    let mid = (az0 + az1) * 0.5;
    let half = (az1 - az0) * 0.5;
    let mut d = (az - mid) % 360.0;
    if d > 180.0 {
        d -= 360.0;
    }
    if d < -180.0 {
        d += 360.0;
    }
    let a = 1.0 - smooth(half - soft, half + soft, d.abs());
    let e = smooth(el0 - soft, el0 + soft, el) * (1.0 - smooth(el1 - soft, el1 + soft, el));
    a * e
}

/// The radiance seen in direction `d` (any length; z up).
pub fn radiance(env: Environment, d: [f64; 3]) -> [f32; 3] {
    let n = (d[0] * d[0] + d[1] * d[1] + d[2] * d[2]).sqrt().max(1e-12);
    let (x, y, z) = (d[0] / n, d[1] / n, d[2] / n);
    let az = det::atan2(y, x) * 180.0 / PI;
    let el = det::asin(z.clamp(-1.0, 1.0)) * 180.0 / PI;
    let c = match env {
        Environment::Simple | Environment::PhotoStudio | Environment::BrownStudio => {
            [0.2, 0.2, 0.2]
        }
        Environment::Studio => studio(az, el),
        Environment::Room => room(az, el),
    };
    [c[0] as f32, c[1] as f32, c[2] as f32]
}

fn add(c: &mut [f64; 3], tint: [f64; 3], k: f64) {
    for i in 0..3 {
        c[i] += tint[i] * k;
    }
}

fn studio(az: f64, el: f64) -> [f64; 3] {
    let s = det::sin(el.to_radians());
    // Walls and ceiling: a dim, slightly warm room; the floor is a lighter mid grey.
    let (hor, zen, flo) = ([0.30, 0.31, 0.33], [0.10, 0.11, 0.14], [0.17, 0.17, 0.18]);
    let mut c = if s >= 0.0 {
        let t = det::sqrt(s);
        [0, 1, 2].map(|i| hor[i] + (zen[i] - hor[i]) * t)
    } else {
        let t = det::sqrt(-s);
        [0, 1, 2].map(|i| hor[i] + (flo[i] - hor[i]) * t)
    };
    // The key softbox, where the shadow-casting light is (azimuth -47, elevation 31): two panes
    // with a divider to one side of the light itself, so a reflection of it shows a frame.
    let key = panel(az, el, -73.0, -21.0, 6.0, 58.0, 1.2);
    let divider = panel(az, el, -34.5, -31.5, 4.0, 60.0, 0.6);
    add(
        &mut c,
        [1.0, 0.97, 0.93],
        7.0 * key * (1.0 - 0.85 * divider),
    );
    // A cool fill softbox opposite, a tall strip behind, and three ceiling panels.
    add(
        &mut c,
        [0.86, 0.92, 1.0],
        2.8 * panel(az, el, 115.0, 155.0, 2.0, 48.0, 1.5),
    );
    add(
        &mut c,
        [1.0, 0.99, 0.97],
        2.0 * panel(az, el, -170.0, -152.0, 4.0, 62.0, 1.0),
    );
    add(
        &mut c,
        [1.0, 0.99, 0.97],
        1.4 * panel(az, el, 8.0, 22.0, -2.0, 40.0, 1.0),
    );
    for centre in [0.0, 120.0, 240.0] {
        add(
            &mut c,
            [1.0, 1.0, 0.98],
            1.5 * panel(az, el, centre - 26.0, centre + 26.0, 64.0, 78.0, 2.0),
        );
    }
    // The floor lit under the softboxes, and a thin bright line at the horizon.
    add(
        &mut c,
        [0.9, 0.88, 0.85],
        0.55 * panel(az, el, -180.0, 180.0, -50.0, -8.0, 14.0),
    );
    add(
        &mut c,
        [1.0, 1.0, 1.0],
        0.5 * det::exp(-((el / 1.4) * (el / 1.4))),
    );
    c
}

fn room(az: f64, el: f64) -> [f64; 3] {
    let s = det::sin(el.to_radians());
    // Warm walls, a lighter ceiling, a dark wood floor.
    let (wall, ceil, flo) = ([0.55, 0.52, 0.48], [0.85, 0.84, 0.82], [0.22, 0.17, 0.13]);
    let mut c = if s >= 0.0 {
        let t = smooth(0.35, 0.8, s);
        [0, 1, 2].map(|i| wall[i] + (ceil[i] - wall[i]) * t)
    } else {
        let t = smooth(0.0, 0.35, -s);
        [0, 1, 2].map(|i| wall[i] + (flo[i] - wall[i]) * t)
    };
    // The big window at the key direction: bright sky behind a frame with mullions.
    let window = panel(az, el, -75.0, -19.0, 5.0, 54.0, 1.0);
    let sky_t = smooth(5.0, 54.0, el);
    let sky = [
        2.6 + (1.1 - 2.6) * sky_t,
        2.7 + (1.5 - 2.7) * sky_t,
        2.8 + (2.4 - 2.8) * sky_t,
    ];
    let mullion = panel(az, el, -34.8, -31.2, 5.0, 54.0, 0.5)
        .max(panel(az, el, -75.0, -19.0, 28.5, 31.5, 0.5));
    let frame = 1.0 - panel(az, el, -72.5, -21.5, 7.5, 51.5, 0.8) * (1.0 - 0.0);
    let dark = (mullion + frame).clamp(0.0, 1.0);
    for i in 0..3 {
        let lit = sky[i] * 2.4 * (1.0 - dark) + 0.04 * dark;
        c[i] = c[i] * (1.0 - window) + lit * window;
    }
    // The sun, high in the sky behind the window.
    let (sa, se) = ((-42.0f64).to_radians(), 38.0f64.to_radians());
    let (azr, elr) = (az.to_radians(), el.to_radians());
    let cosang = det::sin(elr) * det::sin(se) + det::cos(elr) * det::cos(se) * det::cos(azr - sa);
    add(
        &mut c,
        [1.0, 0.95, 0.85],
        40.0 * smooth(0.9985, 0.9995, cosang) * window,
    );
    // A small window on the far wall, a picture, a ceiling lamp.
    let small = panel(az, el, 130.0, 152.0, 14.0, 46.0, 0.8);
    let small_bar = panel(az, el, 140.0, 142.0, 14.0, 46.0, 0.4);
    add(&mut c, [0.9, 0.95, 1.0], 1.4 * small * (1.0 - small_bar));
    add(
        &mut c,
        [0.5, 0.45, 0.4],
        0.5 * panel(az, el, 205.0, 235.0, 6.0, 30.0, 0.8),
    );
    add(&mut c, [1.0, 0.9, 0.75], 4.0 * smooth(72.0, 84.0, el));
    // Light from the window on the floor.
    add(
        &mut c,
        [1.0, 0.95, 0.85],
        0.7 * panel(az, el, -110.0, -10.0, -48.0, -10.0, 10.0),
    );
    c
}

/// The bundled photographed studio (see `assets/environments/README.md` for its licence).
const PHOTO_STUDIO: &[u8] = include_bytes!("../assets/environments/photo_studio_01_2k.exr");
const BROWN_STUDIO: &[u8] = include_bytes!("../assets/environments/brown_photostudio_02_2k.exr");

/// A panorama: linear RGBA floats, row 0 straight up, azimuth 0 in the middle column.
pub struct Panorama {
    pub width: usize,
    pub height: usize,
    pub rgba: Vec<f32>,
}

/// The azimuth (degrees) the shadow-casting light shines from; the photographed studio is
/// turned so its main light is there, so shadows and reflections agree.
pub const KEY_AZIMUTH: f64 = -47.5;

fn generated(env: Environment) -> Panorama {
    let mut rgba = Vec::with_capacity(WIDTH * HEIGHT * 4);
    for j in 0..HEIGHT {
        // Row 0 is straight up.
        let el = FRAC_PI_2 - PI * (j as f64 + 0.5) / HEIGHT as f64;
        for i in 0..WIDTH {
            let az = 2.0 * PI * (i as f64 + 0.5) / WIDTH as f64 - PI;
            let d = [
                det::cos(el) * det::cos(az),
                det::cos(el) * det::sin(az),
                det::sin(el),
            ];
            let c = radiance(env, d);
            rgba.extend_from_slice(&[c[0], c[1], c[2], 1.0]);
        }
    }
    Panorama {
        width: WIDTH,
        height: HEIGHT,
        rgba,
    }
}

/// Near the top and bottom of an equirectangular image every row is the whole circle of
/// directions squeezed into one ring, so a row of `w` texels covers only `w * cos(elevation)`
/// texels' worth of real directions. Left alone, a reflection looking straight up or down
/// shows each column as a streak fanning out from the pole. Each row is blurred horizontally
/// by its stretch (`1 / cos(elevation)` texels, a box filter, wrapping round), which is exactly
/// the averaging a correctly sampled image would have, so the pole becomes one smooth value.
fn blur_poles(rgba: &mut [f32], w: usize, h: usize) {
    let mut padded = vec![0f64; w * 3 + 1];
    for j in 0..h {
        let el = FRAC_PI_2 - PI * (j as f64 + 0.5) / h as f64;
        let stretch = 1.0 / det::cos(el).max(1e-6);
        if stretch < 1.15 {
            continue;
        }
        // The stretch is the least that removes the distortion; detail on the floor and ceiling
        // far from the horizon is also much finer than anything a reflection can show, so the
        // blur grows with the square of the stretch. Two passes of a box this wide make a tent,
        // which leaves no ripples behind.
        let width = ((stretch * stretch).round() as usize).clamp(1, w);
        let half = width / 2;
        for _ in 0..2 {
            for c in 0..3 {
                // Running sums over the row repeated three times, so a window up to a whole
                // row wide can wrap on either side.
                padded[0] = 0.0;
                for k in 0..3 * w {
                    padded[k + 1] = padded[k] + rgba[(j * w + k % w) * 4 + c] as f64;
                }
                for x in 0..w {
                    let a = x + w - half;
                    rgba[(j * w + x) * 4 + c] =
                        ((padded[a + width] - padded[a]) / width as f64) as f32;
                }
            }
        }
    }
}

/// The photographed studio, with its columns rolled so the brightest light above the horizon
/// is at [`KEY_AZIMUTH`].
fn photographed(file: &[u8]) -> Option<Panorama> {
    let img = dc_io::exr::read(file).ok()?;
    let (w, h) = (img.width, img.height);
    // Where is the light? Sum brightness per column over the upper half, smoothed over a
    // window of about 20 degrees so one hot pixel does not decide.
    let mut col = vec![0f64; w];
    for y in 0..h / 2 {
        for (x, c) in col.iter_mut().enumerate() {
            let i = (y * w + x) * 3;
            // Clamp so a tiny sun does not outweigh a big softbox.
            *c += (img.rgb[i] + img.rgb[i + 1] + img.rgb[i + 2]).min(30.0) as f64;
        }
    }
    let half = w / 36;
    let mut best = (0usize, -1.0f64);
    for x in 0..w {
        let sum: f64 = (0..=2 * half).map(|k| col[(x + w - half + k) % w]).sum();
        if sum > best.1 {
            best = (x, sum);
        }
    }
    // Column x shows azimuth (x + 0.5) / w * 360 - 180; move the brightest to KEY_AZIMUTH.
    let want = ((KEY_AZIMUTH + 180.0) / 360.0 * w as f64).round() as usize % w;
    let shift = (want + w - best.0) % w;
    let mut rgba = Vec::with_capacity(w * h * 4);
    for y in 0..h {
        for x in 0..w {
            let sx = (x + w - shift) % w;
            let i = (y * w + sx) * 3;
            rgba.extend_from_slice(&[img.rgb[i], img.rgb[i + 1], img.rgb[i + 2], 1.0]);
        }
    }
    blur_poles(&mut rgba, w, h);
    Some(Panorama {
        width: w,
        height: h,
        rgba,
    })
}

/// The panorama for `env` (the photographed one falls back to the generated studio if it
/// cannot be decoded, which would be a build problem, not a user one).
pub fn panorama(env: Environment) -> Panorama {
    match env {
        Environment::PhotoStudio => {
            photographed(PHOTO_STUDIO).unwrap_or_else(|| generated(Environment::Studio))
        }
        Environment::BrownStudio => {
            photographed(BROWN_STUDIO).unwrap_or_else(|| generated(Environment::Studio))
        }
        e => generated(e),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn lum(c: [f32; 3]) -> f32 {
        0.2126 * c[0] + 0.7152 * c[1] + 0.0722 * c[2]
    }

    #[test]
    fn the_key_light_is_where_the_shadow_comes_from_and_is_the_brightest_thing() {
        // The shadow-casting light points along (0.55, -0.60, 0.52).
        for env in [Environment::Studio, Environment::Room] {
            let key = lum(radiance(env, [0.55, -0.60, 0.52]));
            let behind = lum(radiance(env, [-0.55, 0.60, 0.52]));
            let floor = lum(radiance(env, [0.0, 0.0, -1.0]));
            assert!(key > 2.0, "{env:?}: the key window is bright ({key})");
            assert!(
                key > 2.0 * behind.max(0.2),
                "{env:?}: and brighter than the far side: {key} against {behind}"
            );
            assert!(
                floor > 0.05 && floor < 1.5,
                "{env:?}: the floor is neither black nor a lamp"
            );
        }
    }

    #[test]
    fn a_panorama_has_the_right_size_is_finite_and_is_deterministic() {
        for env in Environment::ALL.into_iter().filter(|e| e.has_image()) {
            let p = panorama(env);
            assert_eq!(p.rgba.len(), p.width * p.height * 4);
            assert_eq!(p.width, 2 * p.height, "equirectangular");
            assert!(p.rgba.iter().all(|v| v.is_finite() && *v >= 0.0));
            assert_eq!(p.rgba, panorama(env).rgba);
        }
        // Straight up and down are single points: every azimuth gives the same colour there.
        let up = radiance(Environment::Studio, [0.0, 0.0, 1.0]);
        let up2 = radiance(Environment::Studio, [1e-9, 1e-9, 1.0]);
        assert_eq!(up, up2);
    }

    #[test]
    fn the_room_has_a_window_frame_and_a_sun() {
        // A mullion between the panes is dark, the glass beside it bright, the sun very bright.
        let glass = lum(radiance(Environment::Room, dir(-60.0, 20.0)));
        let bar = lum(radiance(Environment::Room, dir(-33.0, 20.0)));
        let sun = lum(radiance(Environment::Room, dir(-42.0, 38.0)));
        assert!(glass > 1.5 && bar < 0.5 && sun > 8.0, "{glass} {bar} {sun}");
    }

    fn dir(az: f64, el: f64) -> [f64; 3] {
        let (a, e) = (az.to_radians(), el.to_radians());
        [
            det::cos(e) * det::cos(a),
            det::cos(e) * det::sin(a),
            det::sin(e),
        ]
    }

    #[test]
    fn the_poles_of_a_photographed_studio_are_smooth_not_streaks() {
        // Spread of the bottom row (looking straight down) against the same for a row at the
        // horizon: the pole must be nearly uniform, the horizon must keep its detail.
        let spread = |p: &Panorama, j: usize| {
            let vals: Vec<f32> = (0..p.width)
                .map(|x| p.rgba[(j * p.width + x) * 4 + 1])
                .collect();
            let mean = vals.iter().sum::<f32>() / vals.len() as f32;
            let var = vals.iter().map(|v| (v - mean) * (v - mean)).sum::<f32>() / vals.len() as f32;
            var.sqrt() / mean.max(1e-6)
        };
        for env in [Environment::PhotoStudio, Environment::BrownStudio] {
            let p = panorama(env);
            let (bottom, top, horizon) = (
                spread(&p, p.height - 1),
                spread(&p, 0),
                spread(&p, p.height / 2),
            );
            assert!(bottom < 0.05 && top < 0.05, "{env:?}: poles {bottom} {top}");
            assert!(
                horizon > 0.2,
                "{env:?}: the horizon keeps its detail ({horizon})"
            );
        }
    }

    #[test]
    fn blurring_the_poles_keeps_the_average_of_each_row() {
        let (w, h) = (64usize, 32usize);
        let mut a: Vec<f32> = (0..w * h * 4).map(|i| ((i * 7919) % 13) as f32).collect();
        let mean =
            |a: &[f32], j: usize| (0..w).map(|x| a[(j * w + x) * 4] as f64).sum::<f64>() / w as f64;
        let before: Vec<f64> = (0..h).map(|j| mean(&a, j)).collect();
        blur_poles(&mut a, w, h);
        for (j, b) in before.iter().enumerate() {
            assert!((mean(&a, j) - b).abs() < 1e-4, "row {j}");
        }
    }

    #[test]
    fn the_photographed_studios_are_loaded_and_their_light_is_where_the_shadow_comes_from() {
        for env in [Environment::PhotoStudio, Environment::BrownStudio] {
            check_photographed(env);
        }
    }

    fn check_photographed(env: Environment) {
        let p = panorama(env);
        assert_eq!(
            (p.width, p.height),
            (2048, 1024),
            "the real image, not the fallback"
        );
        // The brightest column over a 20 degree window should now be at the key azimuth.
        let (w, h) = (p.width, p.height);
        let mut best = (0usize, -1.0f64);
        for x in 0..w {
            let mut sum = 0.0f64;
            for k in 0..=(w / 18) {
                let xx = (x + w - w / 36 + k) % w;
                for y in 0..h / 2 {
                    let i = (y * w + xx) * 4;
                    sum += (p.rgba[i] + p.rgba[i + 1] + p.rgba[i + 2]).min(30.0) as f64;
                }
            }
            if sum > best.1 {
                best = (x, sum);
            }
        }
        let az = (best.0 as f64 + 0.5) / w as f64 * 360.0 - 180.0;
        assert!(
            (az - KEY_AZIMUTH).abs() < 4.0,
            "{env:?}: brightest light at azimuth {az}"
        );
    }
}
