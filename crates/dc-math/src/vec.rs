//! Small `f64` vector types. Only operations that are exactly specified by IEEE 754 (plus
//! `sqrt`) are used, so results are identical on every platform.

use crate::{det, Tolerance};
use core::ops::{Add, AddAssign, Div, Mul, Neg, Sub, SubAssign};

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Vec2 {
    pub x: f64,
    pub y: f64,
}

impl Vec2 {
    pub const ZERO: Vec2 = Vec2 { x: 0.0, y: 0.0 };
    pub const X: Vec2 = Vec2 { x: 1.0, y: 0.0 };
    pub const Y: Vec2 = Vec2 { x: 0.0, y: 1.0 };

    #[inline]
    pub const fn new(x: f64, y: f64) -> Self {
        Self { x, y }
    }
    #[inline]
    pub fn dot(self, o: Self) -> f64 {
        self.x * o.x + self.y * o.y
    }
    /// 2D cross product (z component of the 3D cross product).
    #[inline]
    pub fn cross(self, o: Self) -> f64 {
        self.x * o.y - self.y * o.x
    }
    #[inline]
    pub fn length_squared(self) -> f64 {
        self.dot(self)
    }
    #[inline]
    pub fn length(self) -> f64 {
        det::sqrt(self.length_squared())
    }
    /// Unit vector, or `None` if shorter than the linear tolerance.
    pub fn normalized(self, tol: Tolerance) -> Option<Self> {
        let l = self.length();
        if l <= tol.linear {
            None
        } else {
            Some(Self::new(self.x / l, self.y / l))
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Vec3 {
    pub x: f64,
    pub y: f64,
    pub z: f64,
}

impl Vec3 {
    pub const ZERO: Vec3 = Vec3 {
        x: 0.0,
        y: 0.0,
        z: 0.0,
    };
    pub const X: Vec3 = Vec3 {
        x: 1.0,
        y: 0.0,
        z: 0.0,
    };
    pub const Y: Vec3 = Vec3 {
        x: 0.0,
        y: 1.0,
        z: 0.0,
    };
    pub const Z: Vec3 = Vec3 {
        x: 0.0,
        y: 0.0,
        z: 1.0,
    };

    #[inline]
    pub const fn new(x: f64, y: f64, z: f64) -> Self {
        Self { x, y, z }
    }
    #[inline]
    pub fn dot(self, o: Self) -> f64 {
        self.x * o.x + self.y * o.y + self.z * o.z
    }
    #[inline]
    pub fn cross(self, o: Self) -> Self {
        Self::new(
            self.y * o.z - self.z * o.y,
            self.z * o.x - self.x * o.z,
            self.x * o.y - self.y * o.x,
        )
    }
    #[inline]
    pub fn length_squared(self) -> f64 {
        self.dot(self)
    }
    #[inline]
    pub fn length(self) -> f64 {
        det::sqrt(self.length_squared())
    }
    /// Unit vector, or `None` if shorter than the linear tolerance.
    pub fn normalized(self, tol: Tolerance) -> Option<Self> {
        let l = self.length();
        if l <= tol.linear {
            None
        } else {
            Some(self / l)
        }
    }
}

macro_rules! impl_ops {
    ($t:ident { $($f:ident),+ }) => {
        impl Add for $t { type Output = $t; #[inline] fn add(self, o: $t) -> $t { $t { $($f: self.$f + o.$f),+ } } }
        impl Sub for $t { type Output = $t; #[inline] fn sub(self, o: $t) -> $t { $t { $($f: self.$f - o.$f),+ } } }
        impl Neg for $t { type Output = $t; #[inline] fn neg(self) -> $t { $t { $($f: -self.$f),+ } } }
        impl Mul<f64> for $t { type Output = $t; #[inline] fn mul(self, s: f64) -> $t { $t { $($f: self.$f * s),+ } } }
        impl Mul<$t> for f64 { type Output = $t; #[inline] fn mul(self, v: $t) -> $t { v * self } }
        impl Div<f64> for $t { type Output = $t; #[inline] fn div(self, s: f64) -> $t { $t { $($f: self.$f / s),+ } } }
        impl AddAssign for $t { #[inline] fn add_assign(&mut self, o: $t) { *self = *self + o; } }
        impl SubAssign for $t { #[inline] fn sub_assign(&mut self, o: $t) { *self = *self - o; } }
    };
}
impl_ops!(Vec2 { x, y });
impl_ops!(Vec3 { x, y, z });

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn algebra() {
        let a = Vec3::new(1.0, 2.0, 3.0);
        let b = Vec3::new(-2.0, 0.5, 4.0);
        assert_eq!(a + b, Vec3::new(-1.0, 2.5, 7.0));
        assert_eq!(Vec3::X.cross(Vec3::Y), Vec3::Z);
        assert_eq!(a.cross(b).dot(a), 0.0);
        assert_eq!(Vec3::new(3.0, 4.0, 0.0).length(), 5.0);
        assert_eq!(Vec2::new(3.0, 4.0).length(), 5.0);
        assert_eq!(Vec2::X.cross(Vec2::Y), 1.0);
    }

    #[test]
    fn normalize_rejects_tiny() {
        let t = Tolerance::DEFAULT;
        assert!(Vec3::new(1e-9, 0.0, 0.0).normalized(t).is_none());
        let n = Vec3::new(0.0, 0.0, 2.0).normalized(t).unwrap();
        assert_eq!(n, Vec3::Z);
    }
}
