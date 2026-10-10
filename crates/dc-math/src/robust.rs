//! Exact geometric predicates.
//!
//! `orient2d` says on which side of the line through `a` and `b` the point `c` lies, and gets it
//! right however close to the line it is: plain floating-point arithmetic rounds the two
//! products it subtracts, so for nearly collinear points it can report the wrong side (or zero),
//! and algorithms built on it (triangulation, point location, region booleans) can then loop,
//! drop material or cross their own boundaries. Here a fast evaluation with a proven error bound
//! settles almost every call, and the rest are evaluated exactly with floating-point expansions
//! (Shewchuk, "Adaptive Precision Floating-Point Arithmetic and Fast Robust Geometric
//! Predicates", 1997): every product and sum is kept as a sum of non-overlapping doubles with no
//! rounding at all. Only IEEE additions, subtractions and multiplications are used, in a fixed
//! order, so the result is identical on every platform (no fused multiply-add, no libm).
//!
//! Limits: exact for coordinates whose products and sums neither overflow nor underflow
//! (absolute values between about 1e-150 and 1e150); outside that the fast result is returned.

use crate::Vec2;

const EPS: f64 = 1.110_223_024_625_156_5e-16; // 2^-53
/// Error bound factor of the fast evaluation (Shewchuk's `ccwerrboundA`).
const CCW_ERR_BOUND_A: f64 = (3.0 + 16.0 * EPS) * EPS;
const SPLITTER: f64 = 134_217_729.0; // 2^27 + 1

/// `a + b` as an exact pair `(sum, error)`.
fn two_sum(a: f64, b: f64) -> (f64, f64) {
    let x = a + b;
    let bv = x - a;
    let av = x - bv;
    (x, (a - av) + (b - bv))
}

/// `a * b` as an exact pair `(product, error)`, by Dekker's splitting (no fused multiply-add).
fn two_product(a: f64, b: f64) -> (f64, f64) {
    let split = |v: f64| {
        let c = SPLITTER * v;
        let big = c - v;
        let hi = c - big;
        (hi, v - hi)
    };
    let x = a * b;
    let ((ahi, alo), (bhi, blo)) = (split(a), split(b));
    let e1 = x - ahi * bhi;
    let e2 = e1 - alo * bhi;
    let e3 = e2 - ahi * blo;
    (x, alo * blo - e3)
}

/// Adds the double `b` to the expansion `e` (increasing magnitude, non-overlapping), exactly.
fn grow(e: &[f64], b: f64) -> Vec<f64> {
    let mut out = Vec::with_capacity(e.len() + 1);
    let mut q = b;
    for &x in e {
        let (s, h) = two_sum(q, x);
        q = s;
        if h != 0.0 {
            out.push(h);
        }
    }
    if q != 0.0 || out.is_empty() {
        out.push(q);
    }
    out
}

/// The sign of the determinant, computed exactly:
/// `ax*by - ax*cy + bx*cy - bx*ay + cx*ay - cx*by`.
fn orient2d_exact(a: Vec2, b: Vec2, c: Vec2) -> f64 {
    let mut e: Vec<f64> = vec![0.0];
    for (p, q, sign) in [
        (a.x, b.y, 1.0),
        (a.x, c.y, -1.0),
        (b.x, c.y, 1.0),
        (b.x, a.y, -1.0),
        (c.x, a.y, 1.0),
        (c.x, b.y, -1.0),
    ] {
        let (x, y) = two_product(p, q);
        e = grow(&e, sign * y);
        e = grow(&e, sign * x);
    }
    // The most significant component of a non-overlapping expansion carries its sign.
    *e.last().unwrap_or(&0.0)
}

/// Positive if `a`, `b`, `c` turn counter-clockwise, negative if clockwise, zero if collinear.
/// The sign is exact; the magnitude is twice the signed area of the triangle (approximately,
/// when the points are very nearly collinear).
pub fn orient2d(a: Vec2, b: Vec2, c: Vec2) -> f64 {
    let left = (a.x - c.x) * (b.y - c.y);
    let right = (a.y - c.y) * (b.x - c.x);
    let det = left - right;
    let bound = CCW_ERR_BOUND_A * (left.abs() + right.abs());
    if !det.is_finite() || det.abs() > bound {
        return det;
    }
    let big = [a.x, a.y, b.x, b.y, c.x, c.y]
        .iter()
        .any(|v| v.abs() > 1e150 || (*v != 0.0 && v.abs() < 1e-150));
    if big {
        return det;
    }
    orient2d_exact(a, b, c)
}

/// [`orient2d`] reduced to -1, 0 or 1.
pub fn orient2d_sign(a: Vec2, b: Vec2, c: Vec2) -> i8 {
    let d = orient2d(a, b, c);
    if d > 0.0 {
        1
    } else if d < 0.0 {
        -1
    } else {
        0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn v(x: f64, y: f64) -> Vec2 {
        Vec2::new(x, y)
    }

    #[test]
    fn products_and_sums_are_exact() {
        // (a + b) as a pair adds back to the same real number.
        let (s, e) = two_sum(1.0, 1e-30);
        assert_eq!(s, 1.0);
        assert_eq!(e, 1e-30);
        // 0.1 * 0.1 is not representable; the pair holds the rounding error exactly.
        let (p, e) = two_product(0.1, 0.1);
        assert_eq!(p, 0.1 * 0.1);
        assert!(e != 0.0 && e.abs() < p * 1.2e-16);
        // A product that is exact has no error.
        assert_eq!(two_product(3.0, 5.0), (15.0, 0.0));
        // Powers of two scale exactly.
        // 2^60 * 2^-30 = 2^30, written as exact binary constants.
        let (p60, m30, p30) = (
            1_152_921_504_606_846_976.0,
            9.313_225_746_154_785e-10,
            1_073_741_824.0,
        );
        assert_eq!(two_product(p60, m30), (p30, 0.0));
    }

    #[test]
    fn easy_cases() {
        assert!(orient2d(v(0.0, 0.0), v(1.0, 0.0), v(0.0, 1.0)) > 0.0);
        assert!(orient2d(v(0.0, 0.0), v(0.0, 1.0), v(1.0, 0.0)) < 0.0);
        assert_eq!(orient2d(v(0.0, 0.0), v(1.0, 1.0), v(2.0, 2.0)), 0.0);
        assert_eq!(orient2d_sign(v(0.0, 0.0), v(1.0, 0.0), v(0.0, 1.0)), 1);
        // The magnitude is twice the area.
        assert_eq!(orient2d(v(0.0, 0.0), v(4.0, 0.0), v(0.0, 3.0)), 12.0);
        // Swapping two points flips the sign; a cyclic shift keeps it.
        let (a, b, c) = (v(0.3, 0.7), v(5.1, -2.0), v(-1.5, 4.25));
        assert_eq!(orient2d_sign(a, b, c), -orient2d_sign(b, a, c));
        assert_eq!(orient2d_sign(a, b, c), orient2d_sign(b, c, a));
    }

    #[test]
    fn nearly_collinear_points_get_the_right_side() {
        // Each row: a, b, c and the exact sign, computed independently with rational
        // arithmetic (Python `fractions`) for cases where plain evaluation gets it wrong.
        type Case = ([f64; 2], [f64; 2], [f64; 2], i8);
        let cases: [Case; 16] = [
            (
                [
                    f64::from_bits(0x3fe0000000000000),
                    f64::from_bits(0x3fe0000000000007),
                ],
                [
                    f64::from_bits(0x4028000000000000),
                    f64::from_bits(0x4028000000000000),
                ],
                [
                    f64::from_bits(0x4038000000000000),
                    f64::from_bits(0x4038000000000000),
                ],
                1,
            ),
            (
                [
                    f64::from_bits(0x3fe0000000000000),
                    f64::from_bits(0x3fe000000000000e),
                ],
                [
                    f64::from_bits(0x4028000000000000),
                    f64::from_bits(0x4028000000000000),
                ],
                [
                    f64::from_bits(0x4038000000000000),
                    f64::from_bits(0x4038000000000000),
                ],
                1,
            ),
            (
                [
                    f64::from_bits(0x3fe0000000000003),
                    f64::from_bits(0x3fe0000000000000),
                ],
                [
                    f64::from_bits(0x4028000000000000),
                    f64::from_bits(0x4028000000000000),
                ],
                [
                    f64::from_bits(0x4038000000000000),
                    f64::from_bits(0x4038000000000000),
                ],
                -1,
            ),
            (
                [
                    f64::from_bits(0x3fe0000000000003),
                    f64::from_bits(0x3fe0000000000007),
                ],
                [
                    f64::from_bits(0x4028000000000000),
                    f64::from_bits(0x4028000000000000),
                ],
                [
                    f64::from_bits(0x4038000000000000),
                    f64::from_bits(0x4038000000000000),
                ],
                1,
            ),
            (
                [
                    f64::from_bits(0x3fe0000000000003),
                    f64::from_bits(0x3fe000000000000e),
                ],
                [
                    f64::from_bits(0x4028000000000000),
                    f64::from_bits(0x4028000000000000),
                ],
                [
                    f64::from_bits(0x4038000000000000),
                    f64::from_bits(0x4038000000000000),
                ],
                1,
            ),
            (
                [
                    f64::from_bits(0x3fe0000000000006),
                    f64::from_bits(0x3fe0000000000000),
                ],
                [
                    f64::from_bits(0x4028000000000000),
                    f64::from_bits(0x4028000000000000),
                ],
                [
                    f64::from_bits(0x4038000000000000),
                    f64::from_bits(0x4038000000000000),
                ],
                -1,
            ),
            (
                [
                    f64::from_bits(0xc0419de197a6d4b4),
                    f64::from_bits(0xc05175216d48484c),
                ],
                [
                    f64::from_bits(0xc0416ff3c6f7c523),
                    f64::from_bits(0xc051cb16f469a8cc),
                ],
                [
                    f64::from_bits(0xc04213b4e21199fd),
                    f64::from_bits(0xc050989cd5ee57ac),
                ],
                1,
            ),
            (
                [
                    f64::from_bits(0x4056d3649a26d976),
                    f64::from_bits(0xc0251be2c3925bd0),
                ],
                [
                    f64::from_bits(0x4057f697e7d4cf87),
                    f64::from_bits(0xc027d0f3375ed072),
                ],
                [
                    f64::from_bits(0x405a281821228b96),
                    f64::from_bits(0xc02d0956133bf9ed),
                ],
                -1,
            ),
            (
                [
                    f64::from_bits(0xc0493c02f44280b8),
                    f64::from_bits(0x401291563fa74ee0),
                ],
                [
                    f64::from_bits(0xc0471ed905e8fb09),
                    f64::from_bits(0x40212392e1af584f),
                ],
                [
                    f64::from_bits(0xc043ceec0ff41018),
                    f64::from_bits(0x402d725f5507c494),
                ],
                -1,
            ),
            (
                [
                    f64::from_bits(0xc03e46098dfa158c),
                    f64::from_bits(0xc04a60f2226628e8),
                ],
                [
                    f64::from_bits(0xc039b9516810e3cc),
                    f64::from_bits(0xc047e7710015dee9),
                ],
                [
                    f64::from_bits(0xc0426e3348e6d2d1),
                    f64::from_bits(0xc04df62db426e8a6),
                ],
                -1,
            ),
            (
                [
                    f64::from_bits(0xc036e16a039a5da8),
                    f64::from_bits(0xc0335c5c73b7cbec),
                ],
                [
                    f64::from_bits(0xc0339f4d29b63529),
                    f64::from_bits(0xc034d92adb069144),
                ],
                [
                    f64::from_bits(0xc03e3ebad426e4da),
                    f64::from_bits(0xc02fff46b4bf3f22),
                ],
                1,
            ),
            (
                [
                    f64::from_bits(0x4050e41ad8723a7e),
                    f64::from_bits(0xc048a51f1881fc26),
                ],
                [
                    f64::from_bits(0x4051a5e314247c42),
                    f64::from_bits(0xc0468a246abed3d5),
                ],
                [
                    f64::from_bits(0x404e36e1ca60d5aa),
                    f64::from_bits(0xc04d9b45642db861),
                ],
                -1,
            ),
            (
                [
                    f64::from_bits(0x4050cf723ae6c958),
                    f64::from_bits(0x4031a9c00de294cc),
                ],
                [
                    f64::from_bits(0x405176c528b77112),
                    f64::from_bits(0x402eb35117a45f7e),
                ],
                [
                    f64::from_bits(0x4052c2acebe38468),
                    f64::from_bits(0x4025865becce66e3),
                ],
                1,
            ),
            (
                [
                    f64::from_bits(0x40396caf6c8222d4),
                    f64::from_bits(0xc04b977b129dbd21),
                ],
                [
                    f64::from_bits(0x4038c332fa4f8579),
                    f64::from_bits(0xc04cef5041850cd3),
                ],
                [
                    f64::from_bits(0x403a5e629c660bfb),
                    f64::from_bits(0xc049ad263ef7c649),
                ],
                1,
            ),
            (
                [
                    f64::from_bits(0xc04906d6a07ae32a),
                    f64::from_bits(0xc04205ef27772012),
                ],
                [
                    f64::from_bits(0xc04a77644d9e3624),
                    f64::from_bits(0xc0444335e329de35),
                ],
                [
                    f64::from_bits(0xc04cf9086567e7ae),
                    f64::from_bits(0xc0482944a6efa3a6),
                ],
                1,
            ),
            (
                [
                    f64::from_bits(0xc04beddcd9e6c13e),
                    f64::from_bits(0xc05524a605ee5bfd),
                ],
                [
                    f64::from_bits(0xc04dfce7027f0cde),
                    f64::from_bits(0xc05482876ffd1bc2),
                ],
                [
                    f64::from_bits(0xc047025d1a6f604b),
                    f64::from_bits(0xc056a8134b75e34e),
                ],
                -1,
            ),
        ];
        for (a, b, c, want) in cases {
            let got = orient2d_sign(v(a[0], a[1]), v(b[0], b[1]), v(c[0], c[1]));
            assert_eq!(got, want, "{a:?} {b:?} {c:?}");
            // The plain evaluation really is wrong on these (otherwise the test proves nothing).
            let naive = (a[0] - c[0]) * (b[1] - c[1]) - (a[1] - c[1]) * (b[0] - c[0]);
            let naive_sign = if naive > 0.0 {
                1
            } else if naive < 0.0 {
                -1
            } else {
                0
            };
            assert_ne!(naive_sign, want, "{a:?} {b:?} {c:?}: not a hard case");
            // And every permutation agrees with the exact answer.
            let (pa, pb, pc) = (v(a[0], a[1]), v(b[0], b[1]), v(c[0], c[1]));
            assert_eq!(orient2d_sign(pb, pc, pa), want);
            assert_eq!(orient2d_sign(pc, pa, pb), want);
            assert_eq!(orient2d_sign(pb, pa, pc), -want);
        }
    }

    #[test]
    fn exact_collinear_points_are_exactly_zero_even_when_floats_cannot_say() {
        // Points on y = x scaled by awkward values: collinear in exact arithmetic.
        for k in [0.1, 0.3, 1.0 / 3.0, 7.7, 1e-8, 123456.789] {
            let p = |t: f64| v(t * k, t * k);
            assert_eq!(orient2d_sign(p(1.0), p(2.0), p(4.0)), 0);
            assert_eq!(orient2d_sign(p(0.0), p(5.0), p(-3.0)), 0);
        }
    }

    #[test]
    fn it_is_deterministic_and_never_panics_on_odd_input() {
        for (x, y) in [
            (f64::NAN, 0.0),
            (f64::INFINITY, 1.0),
            (1e300, 1e300),
            (1e-300, 3.0),
        ] {
            let _ = orient2d(v(x, y), v(1.0, 2.0), v(3.0, 5.0));
        }
        let a = orient2d(v(0.1, 0.2), v(0.3, 0.4), v(0.5, 0.6000000000000001));
        let b = orient2d(v(0.1, 0.2), v(0.3, 0.4), v(0.5, 0.6000000000000001));
        assert_eq!(a.to_bits(), b.to_bits());
    }
}
