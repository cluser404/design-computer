//! Deterministic scalar functions.
//!
//! `f64::sin` and friends call the platform's libm, whose results can differ in the last bit
//! between operating systems and CPUs. These wrappers use the pure-Rust `libm` crate instead.
//! `sqrt`, `abs`, `floor`, `ceil` and basic arithmetic are exactly specified by IEEE 754 and are
//! safe to use directly.

#![allow(clippy::disallowed_methods)]

/// Correctly rounded square root (IEEE 754 exact, identical everywhere).
#[inline]
pub fn sqrt(x: f64) -> f64 {
    x.sqrt()
}

#[inline]
pub fn sin(x: f64) -> f64 {
    libm::sin(x)
}
#[inline]
pub fn cos(x: f64) -> f64 {
    libm::cos(x)
}
#[inline]
pub fn tan(x: f64) -> f64 {
    libm::tan(x)
}
#[inline]
pub fn asin(x: f64) -> f64 {
    libm::asin(x)
}
#[inline]
pub fn acos(x: f64) -> f64 {
    libm::acos(x)
}
#[inline]
pub fn atan(x: f64) -> f64 {
    libm::atan(x)
}
#[inline]
pub fn atan2(y: f64, x: f64) -> f64 {
    libm::atan2(y, x)
}
#[inline]
pub fn exp(x: f64) -> f64 {
    libm::exp(x)
}
#[inline]
pub fn ln(x: f64) -> f64 {
    libm::log(x)
}
#[inline]
pub fn log10(x: f64) -> f64 {
    libm::log10(x)
}
#[inline]
pub fn powf(x: f64, y: f64) -> f64 {
    libm::pow(x, y)
}
#[inline]
pub fn hypot(x: f64, y: f64) -> f64 {
    libm::hypot(x, y)
}
#[inline]
pub fn cbrt(x: f64) -> f64 {
    libm::cbrt(x)
}

/// Computes `(sin x, cos x)`.
#[inline]
pub fn sin_cos(x: f64) -> (f64, f64) {
    (sin(x), cos(x))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// FNV-1a over the bit patterns of a fixed sweep of function outputs. If this value changes,
    /// numeric behavior changed; CI runs this on every OS/CPU target and all must agree.
    #[test]
    fn golden_bits() {
        let mut h: u64 = 0xcbf2_9ce4_8422_2325;
        let mut feed = |v: f64| {
            for b in v.to_bits().to_le_bytes() {
                h ^= u64::from(b);
                h = h.wrapping_mul(0x0100_0000_01b3);
            }
        };
        for i in -200..=200 {
            let x = f64::from(i) * 0.173;
            feed(sin(x));
            feed(cos(x));
            feed(tan(x));
            feed(atan(x));
            feed(exp(x * 0.1));
            feed(atan2(x, 1.0 + x * x));
            feed(hypot(x, 2.0));
            feed(cbrt(x));
            feed(sqrt(x.abs()));
            if x > 0.0 {
                feed(ln(x));
                feed(log10(x));
                feed(powf(x, 1.7));
            }
        }
        for i in -100..=100 {
            let x = f64::from(i) / 100.0;
            feed(asin(x));
            feed(acos(x));
        }
        assert_eq!(h, GOLDEN, "numeric golden hash changed: {h:#018x}");
    }

    const GOLDEN: u64 = 0xe7f8_c18c_703c_e825;

    #[test]
    fn basic_accuracy() {
        assert!((sin(core::f64::consts::FRAC_PI_2) - 1.0).abs() < 1e-15);
        assert!((cos(0.0) - 1.0).abs() < 1e-15);
        assert!((atan2(1.0, 1.0) - core::f64::consts::FRAC_PI_4).abs() < 1e-15);
        assert_eq!(sqrt(4.0), 2.0);
    }
}
