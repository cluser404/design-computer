//! Prefiltering an environment panorama for glossy reflections.
//!
//! A shiny surface does not mirror one direction of the environment, it averages a lobe of
//! directions whose width depends on the roughness (the GGX distribution). Averaging that
//! lobe per pixel is far too slow in a shader, so it is done once, here: level `k` of the
//! result holds, for every direction, the environment averaged over the lobe of roughness
//! `k / (levels - 1)`, and the shader just picks the level (blending between two) from the
//! surface's roughness. Level 0 is the image itself (a perfect mirror); each next level is
//! half the size, since a blurrier image needs fewer pixels.
//!
//! Method: filtered importance sampling (Colbert and Krivanek, "GPU-Based Importance
//! Sampling", GPU Gems 3, ch. 20), with the usual simplification that the view direction
//! equals the normal. Samples read a box-filtered mip of the source chosen from the size of
//! the sample's solid angle, so a few dozen samples are enough and there is no noise.

use core::f64::consts::PI;
use dc_math::det;

use crate::environment::Panorama;

const SAMPLES: usize = 64;

/// One source image with its box-filtered half-size copies.
struct Chain {
    /// (width, height, rgb) per level, level 0 the full image.
    levels: Vec<(usize, usize, Vec<f32>)>,
}

impl Chain {
    fn new(p: &Panorama) -> Chain {
        let mut rgb = Vec::with_capacity(p.width * p.height * 3);
        for px in p.rgba.chunks_exact(4) {
            rgb.extend_from_slice(&px[..3]);
        }
        let mut levels = vec![(p.width, p.height, rgb)];
        loop {
            let (w, h, src) = levels.last().unwrap();
            if *w < 4 || *h < 2 {
                break;
            }
            let (nw, nh) = (w / 2, h / 2);
            let mut out = vec![0f32; nw * nh * 3];
            for y in 0..nh {
                for x in 0..nw {
                    for c in 0..3 {
                        let at = |xx: usize, yy: usize| src[(yy * w + xx) * 3 + c];
                        out[(y * nw + x) * 3 + c] = 0.25
                            * (at(2 * x, 2 * y)
                                + at(2 * x + 1, 2 * y)
                                + at(2 * x, 2 * y + 1)
                                + at(2 * x + 1, 2 * y + 1));
                    }
                }
            }
            levels.push((nw, nh, out));
        }
        Chain { levels }
    }

    /// Bilinear read of one level: u wraps round, v is clamped.
    fn bilinear(&self, level: usize, u: f32, v: f32) -> [f32; 3] {
        let (w, h, d) = &self.levels[level];
        let x = u * *w as f32 - 0.5;
        let y = (v * *h as f32 - 0.5).clamp(0.0, (*h - 1) as f32);
        let (x0, y0) = (x.floor(), y.floor());
        let (fx, fy) = (x - x0, y - y0);
        let xi = |k: f32| (k as i64).rem_euclid(*w as i64) as usize;
        let (xa, xb) = (xi(x0), xi(x0 + 1.0));
        let (ya, yb) = (y0 as usize, (y0 as usize + 1).min(*h - 1));
        let mut out = [0f32; 3];
        for (c, o) in out.iter_mut().enumerate() {
            let t = |xx: usize, yy: usize| d[(yy * *w + xx) * 3 + c];
            let top = t(xa, ya) * (1.0 - fx) + t(xb, ya) * fx;
            let bot = t(xa, yb) * (1.0 - fx) + t(xb, yb) * fx;
            *o = top * (1.0 - fy) + bot * fy;
        }
        out
    }

    /// Trilinear read at a fractional level.
    fn sample(&self, lod: f32, dir: [f32; 3]) -> [f32; 3] {
        let u = fast_atan2(dir[1], dir[0]) * (0.5 / core::f32::consts::PI) + 0.5;
        let v = fast_acos(dir[2].clamp(-1.0, 1.0)) * (1.0 / core::f32::consts::PI);
        let top = (self.levels.len() - 1) as f32;
        let lod = lod.clamp(0.0, top);
        let l0 = lod.floor() as usize;
        let l1 = (l0 + 1).min(self.levels.len() - 1);
        let f = lod - l0 as f32;
        let a = self.bilinear(l0, u, v);
        if f < 1e-3 || l0 == l1 {
            return a;
        }
        let b = self.bilinear(l1, u, v);
        [
            a[0] + (b[0] - a[0]) * f,
            a[1] + (b[1] - a[1]) * f,
            a[2] + (b[2] - a[2]) * f,
        ]
    }
}

/// Polynomial arctangent, error below 1e-5 rad: good enough for picking a texel.
fn fast_atan(x: f32) -> f32 {
    let (a, inv) = if x.abs() > 1.0 {
        (1.0 / x, true)
    } else {
        (x, false)
    };
    let s = a * a;
    let r =
        a * (0.999_866 + s * (-0.330_299_5 + s * (0.180_141 + s * (-0.085_133 + s * 0.020_835_1))));
    if inv {
        core::f32::consts::FRAC_PI_2 * x.signum() - r
    } else {
        r
    }
}

fn fast_atan2(y: f32, x: f32) -> f32 {
    if x == 0.0 && y == 0.0 {
        return 0.0;
    }
    if x.abs() >= y.abs() {
        let a = fast_atan(y / x);
        if x > 0.0 {
            a
        } else if y >= 0.0 {
            a + core::f32::consts::PI
        } else {
            a - core::f32::consts::PI
        }
    } else {
        let a = fast_atan(x / y);
        if y > 0.0 {
            core::f32::consts::FRAC_PI_2 - a
        } else {
            -core::f32::consts::FRAC_PI_2 - a
        }
    }
}

/// Abramowitz and Stegun 4.4.45, error below 7e-5 rad.
fn fast_acos(x: f32) -> f32 {
    let a = x.abs();
    let r = (1.0 - a).max(0.0).sqrt()
        * (1.570_728_8 + a * (-0.212_114_4 + a * (0.074_261 - a * 0.018_729_3)));
    if x >= 0.0 {
        r
    } else {
        core::f32::consts::PI - r
    }
}

/// The GGX sample set for one roughness, in the tangent frame of the normal (+z): the
/// direction of each sample, its weight (cos of the angle to the normal) and the mip level
/// to read it from.
fn lobe(rough: f64, src_w: usize, src_h: usize) -> Vec<([f32; 3], f32, f32)> {
    let alpha = (rough * rough).max(1e-4);
    let a2 = alpha * alpha;
    let texel_solid_angle = 4.0 * PI / (src_w * src_h) as f64;
    let mut out = Vec::with_capacity(SAMPLES);
    for i in 0..SAMPLES {
        // Hammersley points: i/N and the radical inverse of i.
        let u1 = (i as f64 + 0.5) / SAMPLES as f64;
        let u2 = radical_inverse(i as u32);
        let phi = 2.0 * PI * u2;
        let cos_t = det::sqrt((1.0 - u1) / (1.0 + (a2 - 1.0) * u1));
        let sin_t = det::sqrt((1.0 - cos_t * cos_t).max(0.0));
        let h = [sin_t * det::cos(phi), sin_t * det::sin(phi), cos_t];
        // View = normal = +z, so the light direction is the view reflected about h.
        let l = [
            2.0 * cos_t * h[0],
            2.0 * cos_t * h[1],
            2.0 * cos_t * h[2] - 1.0,
        ];
        if l[2] <= 0.0 {
            continue;
        }
        let q = cos_t * cos_t * (a2 - 1.0) + 1.0;
        let d = a2 / (PI * q * q);
        // pdf of the light direction is D * (n.h) / (4 v.h) = D / 4 here.
        let pdf = d / 4.0;
        let sample_solid_angle = 1.0 / (SAMPLES as f64 * pdf);
        let lod = (0.5 * (sample_solid_angle / texel_solid_angle).log2() + 1.0).max(0.0);
        out.push((
            [l[0] as f32, l[1] as f32, l[2] as f32],
            l[2] as f32,
            lod as f32,
        ));
    }
    out
}

fn radical_inverse(mut bits: u32) -> f64 {
    bits = bits.rotate_right(16);
    bits = ((bits & 0x5555_5555) << 1) | ((bits & 0xAAAA_AAAA) >> 1);
    bits = ((bits & 0x3333_3333) << 2) | ((bits & 0xCCCC_CCCC) >> 2);
    bits = ((bits & 0x0F0F_0F0F) << 4) | ((bits & 0xF0F0_F0F0) >> 4);
    bits = ((bits & 0x00FF_00FF) << 8) | ((bits & 0xFF00_FF00) >> 8);
    bits as f64 * 2.328_306_436_538_696_3e-10
}

/// The prefiltered levels of `base`: level 0 is `base` itself, level `k` has width
/// `base.width >> k` and holds roughness `k / (levels - 1)`.
pub fn prefilter(base: &Panorama, levels: usize) -> Vec<Panorama> {
    let chain = Chain::new(base);
    let mut out = vec![Panorama {
        width: base.width,
        height: base.height,
        rgba: base.rgba.clone(),
    }];
    for k in 1..levels {
        let (w, h) = ((base.width >> k).max(2), (base.height >> k).max(1));
        let rough = k as f64 / (levels - 1) as f64;
        let lobe = lobe(rough, base.width, base.height);
        let wsum: f32 = lobe.iter().map(|s| s.1).sum();
        let mut rgba = vec![0f32; w * h * 4];
        let threads = std::thread::available_parallelism()
            .map_or(1, |n| n.get())
            .min(h);
        let rows_per = h.div_ceil(threads);
        std::thread::scope(|scope| {
            for (t, chunk) in rgba.chunks_mut(rows_per * w * 4).enumerate() {
                let (chain, lobe) = (&chain, &lobe);
                scope.spawn(move || {
                    for (r, row) in chunk.chunks_mut(w * 4).enumerate() {
                        let j = t * rows_per + r;
                        let el = PI / 2.0 - PI * (j as f64 + 0.5) / h as f64;
                        for (i, px) in row.chunks_mut(4).enumerate() {
                            let az = 2.0 * PI * (i as f64 + 0.5) / w as f64 - PI;
                            let n = [
                                (det::cos(el) * det::cos(az)) as f32,
                                (det::cos(el) * det::sin(az)) as f32,
                                det::sin(el) as f32,
                            ];
                            let up = if n[2].abs() < 0.999 {
                                [0.0, 0.0, 1.0]
                            } else {
                                [1.0, 0.0, 0.0]
                            };
                            let tx = normalize(cross(up, n));
                            let ty = cross(n, tx);
                            let mut acc = [0f32; 3];
                            for (l, weight, lod) in lobe {
                                let d = [
                                    tx[0] * l[0] + ty[0] * l[1] + n[0] * l[2],
                                    tx[1] * l[0] + ty[1] * l[1] + n[1] * l[2],
                                    tx[2] * l[0] + ty[2] * l[1] + n[2] * l[2],
                                ];
                                let c = chain.sample(*lod, d);
                                for q in 0..3 {
                                    acc[q] += c[q] * weight;
                                }
                            }
                            px[0] = acc[0] / wsum;
                            px[1] = acc[1] / wsum;
                            px[2] = acc[2] / wsum;
                            px[3] = 1.0;
                        }
                    }
                });
            }
        });
        out.push(Panorama {
            width: w,
            height: h,
            rgba,
        });
    }
    out
}

fn cross(a: [f32; 3], b: [f32; 3]) -> [f32; 3] {
    [
        a[1] * b[2] - a[2] * b[1],
        a[2] * b[0] - a[0] * b[2],
        a[0] * b[1] - a[1] * b[0],
    ]
}

fn normalize(a: [f32; 3]) -> [f32; 3] {
    let n = (a[0] * a[0] + a[1] * a[1] + a[2] * a[2]).sqrt().max(1e-12);
    [a[0] / n, a[1] / n, a[2] / n]
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pano(w: usize, h: usize, f: impl Fn(usize, usize) -> f32) -> Panorama {
        let mut rgba = Vec::new();
        for y in 0..h {
            for x in 0..w {
                let v = f(x, y);
                rgba.extend_from_slice(&[v, v, v, 1.0]);
            }
        }
        Panorama {
            width: w,
            height: h,
            rgba,
        }
    }

    #[test]
    fn the_fast_trig_is_accurate_enough_to_pick_texels() {
        for i in -200..=200 {
            let x = i as f32 / 200.0;
            let exact = det::acos(x as f64) as f32;
            assert!((fast_acos(x) - exact).abs() < 2e-4, "acos {x}");
        }
        for i in 0..720 {
            let a = (i as f64 / 720.0 - 0.5) * 2.0 * PI;
            let got = fast_atan2(det::sin(a) as f32, det::cos(a) as f32) as f64;
            // Angles are equal modulo a full turn (pi and -pi are the same direction).
            let diff = (got - a + PI).rem_euclid(2.0 * PI) - PI;
            assert!(diff.abs() < 1e-4, "atan2 {a}: {got}");
        }
    }

    #[test]
    fn level_zero_is_the_image_and_a_uniform_image_stays_uniform() {
        let p = pano(64, 32, |_, _| 0.7);
        let levels = prefilter(&p, 5);
        assert_eq!(levels.len(), 5);
        assert_eq!(levels[0].rgba, p.rgba);
        for (k, l) in levels.iter().enumerate() {
            assert_eq!(
                (l.width, l.height),
                ((64usize >> k).max(2), (32usize >> k).max(1))
            );
            assert!(
                l.rgba.chunks(4).all(|c| (c[0] - 0.7).abs() < 2e-3),
                "level {k}"
            );
        }
    }

    #[test]
    fn a_bright_spot_spreads_wider_as_roughness_grows_and_keeps_its_energy() {
        // A bright square on the horizon, in a dark image.
        let p = pano(256, 128, |x, y| {
            if (120..136).contains(&x) && (56..72).contains(&y) {
                50.0
            } else {
                0.0
            }
        });
        let levels = prefilter(&p, 4);
        // Peak at the spot's direction relative to level 0, and the fraction of pixels lit.
        let mut last_peak = f32::MAX;
        let mut last_lit = 0.0;
        for l in &levels[1..] {
            let peak = l.rgba.chunks(4).map(|c| c[0]).fold(0.0, f32::max);
            let lit = l.rgba.chunks(4).filter(|c| c[0] > 0.02).count() as f32
                / (l.width * l.height) as f32;
            assert!(
                peak < last_peak,
                "the peak falls as the lobe widens ({peak} vs {last_peak})"
            );
            assert!(lit > last_lit, "the lit area grows ({lit} vs {last_lit})");
            last_peak = peak;
            last_lit = lit;
        }
        // Energy: the sphere-weighted mean brightness is about the same at every level.
        let mean = |l: &Panorama| {
            let (mut s, mut ws) = (0.0f64, 0.0f64);
            for j in 0..l.height {
                let wgt = det::cos(PI / 2.0 - PI * (j as f64 + 0.5) / l.height as f64).abs();
                for i in 0..l.width {
                    s += l.rgba[(j * l.width + i) * 4] as f64 * wgt;
                    ws += wgt;
                }
            }
            s / ws
        };
        let m0 = mean(&levels[0]);
        for (k, l) in levels.iter().enumerate().skip(1) {
            let m = mean(l);
            assert!((m - m0).abs() / m0 < 0.25, "level {k}: mean {m} vs {m0}");
        }
    }
}
