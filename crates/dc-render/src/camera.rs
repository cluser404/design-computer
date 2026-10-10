//! A CAD camera: Z is up, the view orbits a target point, and the same maths gives projection for
//! drawing and rays for picking.

use core::f64::consts::FRAC_PI_2;
use dc_math::{det, Vec3};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ProjectionKind {
    Perspective,
    Orthographic,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ViewPreset {
    Front,
    Back,
    Left,
    Right,
    Top,
    Bottom,
    Iso,
}

/// A point on screen: pixels (y down), a depth key that is larger for closer points and varies
/// linearly across the screen (so it can be interpolated per pixel), and the distance along the
/// view direction.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Projected {
    pub x: f64,
    pub y: f64,
    pub key: f64,
    pub depth: f64,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Camera {
    pub target: Vec3,
    /// Distance from the eye to the target.
    pub distance: f64,
    /// Rotation about +Z, radians. The eye sits at `target + distance * (cos p cos y, cos p sin y, sin p)`.
    pub yaw: f64,
    /// Elevation above the XY plane, radians, in `[-π/2, π/2]`.
    pub pitch: f64,
    pub fov_y: f64,
    pub projection: ProjectionKind,
}

impl Default for Camera {
    fn default() -> Self {
        Camera::preset(ViewPreset::Iso)
    }
}

impl Camera {
    pub fn preset(p: ViewPreset) -> Camera {
        let (yaw, pitch) = match p {
            ViewPreset::Front => (-FRAC_PI_2, 0.0),
            ViewPreset::Back => (FRAC_PI_2, 0.0),
            ViewPreset::Left => (core::f64::consts::PI, 0.0),
            ViewPreset::Right => (0.0, 0.0),
            ViewPreset::Top => (-FRAC_PI_2, FRAC_PI_2),
            ViewPreset::Bottom => (-FRAC_PI_2, -FRAC_PI_2),
            ViewPreset::Iso => (-core::f64::consts::FRAC_PI_4, 0.615_479_708_670_387_3),
        };
        Camera {
            target: Vec3::ZERO,
            distance: 200.0,
            yaw,
            pitch,
            fov_y: 35.0_f64.to_radians(),
            projection: ProjectionKind::Perspective,
        }
    }

    /// Unit vector from the target toward the eye.
    fn toward_eye(&self) -> Vec3 {
        let (cp, sp) = (det::cos(self.pitch), det::sin(self.pitch));
        Vec3::new(cp * det::cos(self.yaw), cp * det::sin(self.yaw), sp)
    }

    pub fn eye(&self) -> Vec3 {
        self.target + self.toward_eye() * self.distance
    }

    /// `(right, up, forward)`: an orthonormal frame, right-handed, with no singularity at the poles.
    pub fn basis(&self) -> (Vec3, Vec3, Vec3) {
        let forward = -self.toward_eye();
        let right = Vec3::new(-det::sin(self.yaw), det::cos(self.yaw), 0.0);
        let up = right.cross(forward);
        (right, up, forward)
    }

    pub fn tan_half(&self) -> f64 {
        det::tan(self.fov_y * 0.5)
    }

    /// Half the visible height at the target's distance, in world units.
    pub fn half_height(&self) -> f64 {
        self.distance * self.tan_half()
    }

    pub fn near(&self) -> f64 {
        (self.distance * 0.01).max(1e-6)
    }

    pub fn orbit(&mut self, d_yaw: f64, d_pitch: f64) {
        self.yaw += d_yaw;
        self.pitch = (self.pitch + d_pitch).clamp(-FRAC_PI_2, FRAC_PI_2);
    }

    /// Drags the scene by a pixel delta (it follows the cursor).
    pub fn pan(&mut self, dx_px: f64, dy_px: f64, viewport_h_px: f64) {
        let (right, up, _) = self.basis();
        let per_px = 2.0 * self.half_height() / viewport_h_px.max(1.0);
        self.target = self.target - right * (dx_px * per_px) + up * (dy_px * per_px);
    }

    /// Multiplies the distance (`< 1` zooms in).
    pub fn zoom(&mut self, factor: f64) {
        self.distance = (self.distance * factor).clamp(1e-6, 1e12);
    }

    /// Centres on a bounding box and backs off until all of it is in view, in a square viewport.
    pub fn fit(&mut self, min: Vec3, max: Vec3) {
        self.fit_for(min, max, 1.0);
    }

    /// Like [`fit`](Self::fit) for a viewport `aspect` (width / height) wide: a narrow viewport
    /// has a narrower horizontal field of view, so the part has to be farther away to fit.
    pub fn fit_for(&mut self, min: Vec3, max: Vec3, aspect: f64) {
        self.target = (min + max) * 0.5;
        let radius = ((max - min).length() * 0.5).max(1e-6);
        // Half-tangent of the tighter of the two fields of view.
        let t = self.tan_half() * aspect.clamp(0.05, 1.0);
        self.distance = radius * det::sqrt(1.0 + t * t) / t * 1.15;
    }

    /// Camera-space coordinates: `x` right, `y` up, `z` forward distance.
    pub fn camera_space(&self, p: Vec3) -> (f64, f64, f64) {
        let (right, up, forward) = self.basis();
        let d = p - self.eye();
        (d.dot(right), d.dot(up), d.dot(forward))
    }

    /// Projects a point already in camera space. `None` if it is behind the near plane
    /// (perspective only).
    pub fn project_camera(&self, c: (f64, f64, f64), w: f64, h: f64) -> Option<Projected> {
        let (x, y, z) = c;
        let aspect = w / h;
        let (nx, ny, key) = match self.projection {
            ProjectionKind::Perspective => {
                if z < self.near() {
                    return None;
                }
                let t = self.tan_half();
                (x / (z * t * aspect), y / (z * t), 1.0 / z)
            }
            ProjectionKind::Orthographic => {
                let hh = self.half_height();
                (x / (hh * aspect), y / hh, -z)
            }
        };
        Some(Projected {
            x: (nx + 1.0) * 0.5 * w,
            y: (1.0 - ny) * 0.5 * h,
            key,
            depth: z,
        })
    }

    pub fn project(&self, p: Vec3, w: f64, h: f64) -> Option<Projected> {
        self.project_camera(self.camera_space(p), w, h)
    }

    /// The ray through pixel `(px, py)` (y down): an origin and a unit direction.
    pub fn ray(&self, px: f64, py: f64, w: f64, h: f64) -> (Vec3, Vec3) {
        let (right, up, forward) = self.basis();
        let aspect = w / h;
        let nx = 2.0 * px / w - 1.0;
        let ny = 1.0 - 2.0 * py / h;
        match self.projection {
            ProjectionKind::Perspective => {
                let t = self.tan_half();
                let d = forward + right * (nx * t * aspect) + up * (ny * t);
                (self.eye(), d / d.length())
            }
            ProjectionKind::Orthographic => {
                let hh = self.half_height();
                // Start well behind the target so geometry on either side of it is hit.
                let o = self.eye() - forward * (self.distance * 4.0)
                    + right * (nx * hh * aspect)
                    + up * (ny * hh);
                (o, forward)
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn close(a: Vec3, b: Vec3) -> bool {
        (a - b).length() < 1e-9
    }

    #[test]
    fn standard_views_have_the_conventional_axes() {
        let b = |p| Camera::preset(p).basis();
        let (r, u, f) = b(ViewPreset::Front);
        assert!(
            close(r, Vec3::X) && close(u, Vec3::Z) && close(f, Vec3::Y),
            "front: x right, z up, looking +y"
        );
        let (r, u, f) = b(ViewPreset::Top);
        assert!(
            close(r, Vec3::X) && close(u, Vec3::Y) && close(f, -Vec3::Z),
            "top: x right, y up"
        );
        let (r, u, _) = b(ViewPreset::Right);
        assert!(
            close(r, Vec3::Y) && close(u, Vec3::Z),
            "right: y to the right"
        );
        let (r, u, _) = b(ViewPreset::Left);
        assert!(close(r, -Vec3::Y) && close(u, Vec3::Z));
        let (r, u, f) = b(ViewPreset::Bottom);
        assert!(close(r, Vec3::X) && close(u, -Vec3::Y) && close(f, Vec3::Z));
        let (r, u, f) = b(ViewPreset::Back);
        assert!(close(r, -Vec3::X) && close(u, Vec3::Z) && close(f, -Vec3::Y));
        // Every basis is orthonormal and right-handed (right x up = -forward ... i.e. up = right x forward).
        for p in [
            ViewPreset::Front,
            ViewPreset::Back,
            ViewPreset::Left,
            ViewPreset::Right,
            ViewPreset::Top,
            ViewPreset::Bottom,
            ViewPreset::Iso,
        ] {
            let (r, u, f) = b(p);
            assert!(
                (r.length() - 1.0).abs() < 1e-12
                    && (u.length() - 1.0).abs() < 1e-12
                    && (f.length() - 1.0).abs() < 1e-12
            );
            assert!(r.dot(u).abs() < 1e-12 && r.dot(f).abs() < 1e-12 && u.dot(f).abs() < 1e-12);
        }
    }

    #[test]
    fn the_target_projects_to_the_centre_and_rays_invert_projection() {
        for kind in [ProjectionKind::Perspective, ProjectionKind::Orthographic] {
            let c = Camera {
                projection: kind,
                target: Vec3::new(10.0, -5.0, 3.0),
                ..Camera::default()
            };
            let p = c.project(c.target, 800.0, 600.0).unwrap();
            assert!((p.x - 400.0).abs() < 1e-9 && (p.y - 300.0).abs() < 1e-9);
            // Project a world point, shoot a ray through that pixel: it passes through the point.
            for w in [Vec3::new(14.0, 2.0, 9.0), Vec3::new(-30.0, 12.0, -4.0)] {
                let s = c.project(w, 800.0, 600.0).unwrap();
                let (o, d) = c.ray(s.x, s.y, 800.0, 600.0);
                let t = (w - o).dot(d);
                assert!((o + d * t - w).length() < 1e-7, "{kind:?}");
            }
        }
    }

    #[test]
    fn orbit_pan_zoom_and_fit_behave() {
        let mut c = Camera::preset(ViewPreset::Front);
        let eye0 = c.eye();
        assert!(close(eye0, Vec3::new(0.0, -200.0, 0.0)));
        c.orbit(0.0, 10.0); // pitch is clamped at the pole, no flip
        assert_eq!(c.pitch, FRAC_PI_2);
        let (r, u, _) = c.basis();
        assert!(
            r.length() > 0.99 && u.length() > 0.99,
            "no singularity at the pole"
        );
        // Pan moves the target by the right amount: one viewport height of drag = the visible height.
        let mut c = Camera::preset(ViewPreset::Front);
        c.pan(0.0, 600.0, 600.0);
        assert!(
            (c.target.z - 2.0 * c.half_height()).abs() < 1e-9,
            "dragging down moves the scene down"
        );
        let d = c.distance;
        c.zoom(0.5);
        assert_eq!(c.distance, d * 0.5);
        // Fit puts all eight corners of a box inside the viewport.
        let (lo, hi) = (Vec3::new(-50.0, -20.0, 0.0), Vec3::new(150.0, 80.0, 30.0));
        let mut c = Camera::default();
        c.fit(lo, hi);
        for &x in &[lo.x, hi.x] {
            for &y in &[lo.y, hi.y] {
                for &z in &[lo.z, hi.z] {
                    let p = c
                        .project(Vec3::new(x, y, z), 800.0, 600.0)
                        .expect("in front of the camera");
                    assert!(
                        (0.0..=800.0).contains(&p.x) && (0.0..=600.0).contains(&p.y),
                        "{p:?}"
                    );
                }
            }
        }
    }

    #[test]
    fn fitting_accounts_for_narrow_and_wide_viewports() {
        // A box much wider than tall, in a tall narrow viewport and a wide one.
        let (lo, hi) = (
            Vec3::new(-100.0, -10.0, -10.0),
            Vec3::new(100.0, 10.0, 10.0),
        );
        for (w, h) in [(300.0, 900.0), (900.0, 300.0), (800.0, 600.0)] {
            for kind in [ProjectionKind::Perspective, ProjectionKind::Orthographic] {
                let mut c = Camera::preset(ViewPreset::Front);
                c.projection = kind;
                c.fit_for(lo, hi, w / h);
                for &x in &[lo.x, hi.x] {
                    for &y in &[lo.y, hi.y] {
                        for &z in &[lo.z, hi.z] {
                            let p = c.project(Vec3::new(x, y, z), w, h).expect("visible");
                            assert!(
                                (0.0..=w).contains(&p.x) && (0.0..=h).contains(&p.y),
                                "{w}x{h} {kind:?}: {p:?}"
                            );
                        }
                    }
                }
            }
        }
        // The narrow viewport needs the camera farther back than the wide one.
        let (mut narrow, mut wide) = (Camera::default(), Camera::default());
        narrow.fit_for(lo, hi, 0.33);
        wide.fit_for(lo, hi, 3.0);
        assert!(narrow.distance > wide.distance);
    }

    #[test]
    fn closer_points_have_a_larger_depth_key_in_both_projections() {
        for kind in [ProjectionKind::Perspective, ProjectionKind::Orthographic] {
            let mut c = Camera::preset(ViewPreset::Front);
            c.projection = kind;
            let near = c.project(Vec3::new(0.0, -50.0, 0.0), 100.0, 100.0).unwrap();
            let far = c.project(Vec3::new(0.0, 50.0, 0.0), 100.0, 100.0).unwrap();
            assert!(near.key > far.key && near.depth < far.depth, "{kind:?}");
        }
        // Behind the near plane (perspective): not projectable.
        let c = Camera::preset(ViewPreset::Front);
        assert!(c
            .project(Vec3::new(0.0, -300.0, 0.0), 100.0, 100.0)
            .is_none());
    }
}
