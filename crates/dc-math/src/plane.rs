//! Oriented planes with a 2D coordinate frame, used to place sketches in 3D.

use crate::{Tolerance, Vec2, Vec3};

/// A plane with an orthonormal in-plane frame. The normal is `x_axis × y_axis` (right-handed).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Plane {
    pub origin: Vec3,
    pub x_axis: Vec3,
    pub y_axis: Vec3,
}

impl Plane {
    /// Builds a plane from an origin and two in-plane directions. The directions must be
    /// non-degenerate and perpendicular within the angular tolerance; they are normalized.
    pub fn new(origin: Vec3, x_dir: Vec3, y_dir: Vec3, tol: Tolerance) -> Option<Plane> {
        let x = x_dir.normalized(tol)?;
        let y = y_dir.normalized(tol)?;
        if !tol.perpendicular(x, y) {
            return None;
        }
        Some(Plane {
            origin,
            x_axis: x,
            y_axis: y,
        })
    }

    pub fn normal(&self) -> Vec3 {
        self.x_axis.cross(self.y_axis)
    }

    /// Maps a point in the plane's 2D frame to 3D.
    pub fn to_3d(&self, p: Vec2) -> Vec3 {
        self.origin + self.x_axis * p.x + self.y_axis * p.y
    }

    /// Maps a 3D point to the plane's 2D frame (orthogonal projection).
    pub fn to_2d(&self, p: Vec3) -> Vec2 {
        let d = p - self.origin;
        Vec2::new(d.dot(self.x_axis), d.dot(self.y_axis))
    }

    /// Signed distance of a point from the plane, positive on the normal side.
    pub fn signed_distance(&self, p: Vec3) -> f64 {
        (p - self.origin).dot(self.normal())
    }

    /// The same plane moved along its normal.
    pub fn offset(&self, d: f64) -> Plane {
        Plane {
            origin: self.origin + self.normal() * d,
            ..*self
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const T: Tolerance = Tolerance::DEFAULT;

    #[test]
    fn frame_is_right_handed_and_round_trips() {
        let p = Plane::new(Vec3::new(1.0, 2.0, 3.0), Vec3::X, Vec3::Y, T).unwrap();
        assert_eq!(p.normal(), Vec3::Z);
        let q = Vec2::new(4.0, -5.0);
        assert_eq!(p.to_3d(q), Vec3::new(5.0, -3.0, 3.0));
        assert_eq!(p.to_2d(p.to_3d(q)), q);
        assert_eq!(p.signed_distance(Vec3::new(0.0, 0.0, 10.0)), 7.0);
    }

    #[test]
    fn rejects_degenerate_and_skewed_frames() {
        assert!(Plane::new(Vec3::ZERO, Vec3::ZERO, Vec3::Y, T).is_none());
        assert!(Plane::new(Vec3::ZERO, Vec3::X, Vec3::new(1.0, 1.0, 0.0), T).is_none());
        assert!(Plane::new(Vec3::ZERO, Vec3::X, Vec3::X, T).is_none());
    }

    #[test]
    fn offset_moves_along_the_normal() {
        let p = Plane::new(Vec3::ZERO, Vec3::Y, Vec3::Z, T).unwrap(); // normal = +X
        assert_eq!(p.normal(), Vec3::X);
        assert_eq!(p.offset(5.0).origin, Vec3::new(5.0, 0.0, 0.0));
    }
}
