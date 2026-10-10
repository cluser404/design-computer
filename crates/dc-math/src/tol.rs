//! The single tolerance policy.
//!
//! All "are these equal / coincident / parallel" decisions in the kernel go through here, so
//! tolerances are never scattered as magic numbers. Units: millimetres and radians.
//! Values are `[OPEN]` until the kernel has real workloads to tune against.

use crate::vec::Vec3;

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Tolerance {
    /// Distances below this are considered zero (mm).
    pub linear: f64,
    /// Angles below this are considered zero (rad).
    pub angular: f64,
}

impl Tolerance {
    /// 1e-6 mm (one nanometre) and 1e-9 rad.
    pub const DEFAULT: Tolerance = Tolerance {
        linear: 1e-6,
        angular: 1e-9,
    };

    #[inline]
    pub fn is_zero(self, d: f64) -> bool {
        d.abs() <= self.linear
    }

    #[inline]
    pub fn lin_eq(self, a: f64, b: f64) -> bool {
        (a - b).abs() <= self.linear
    }

    #[inline]
    pub fn ang_eq(self, a: f64, b: f64) -> bool {
        (a - b).abs() <= self.angular
    }

    #[inline]
    pub fn coincident(self, a: Vec3, b: Vec3) -> bool {
        (a - b).length() <= self.linear
    }

    /// True if the two directions are parallel or anti-parallel (within angular tolerance).
    /// Returns false if either vector is (near) zero length.
    pub fn parallel(self, a: Vec3, b: Vec3) -> bool {
        match (a.normalized(self), b.normalized(self)) {
            (Some(a), Some(b)) => a.cross(b).length() <= self.angular,
            _ => false,
        }
    }

    /// True if the two directions are perpendicular (within angular tolerance).
    pub fn perpendicular(self, a: Vec3, b: Vec3) -> bool {
        match (a.normalized(self), b.normalized(self)) {
            (Some(a), Some(b)) => a.dot(b).abs() <= self.angular,
            _ => false,
        }
    }
}

impl Default for Tolerance {
    fn default() -> Self {
        Self::DEFAULT
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn coincidence_and_parallel() {
        let t = Tolerance::DEFAULT;
        assert!(t.coincident(Vec3::new(1.0, 2.0, 3.0), Vec3::new(1.0, 2.0, 3.0 + 5e-7)));
        assert!(!t.coincident(Vec3::new(0.0, 0.0, 0.0), Vec3::new(0.0, 0.0, 1e-3)));
        assert!(t.parallel(Vec3::X, Vec3::new(-2.0, 0.0, 0.0)));
        assert!(!t.parallel(Vec3::X, Vec3::Y));
        assert!(t.perpendicular(Vec3::X, Vec3::Z));
        assert!(!t.parallel(Vec3::ZERO, Vec3::X));
    }
}
