//! What the pointer snaps to while drawing, beyond the grid and existing points: the middle of a
//! line, the quadrant points of a circle, and directions that follow from what is already there
//! (perpendicular or collinear to the line you are continuing, tangent to an arc you are leaving).
//! Each snap says what it implies, so the shape drawn from it gets the relation too. Logic only.

use crate::sketching::{angle_of, arc_sweep, dist, Anchor, P2};
use crate::state::AppState;
use dc_math::det;
use dc_sketch::Entity;

/// Which snaps are on (the grid is `SketchSession::snap_grid`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SnapOptions {
    /// Onto existing points and the origin.
    pub points: bool,
    /// Onto the edge of a line (a point-on-line relation).
    pub edges: bool,
    /// Level / upright with the previous point.
    pub guides: bool,
    /// The middle of a line (a midpoint relation).
    pub midpoints: bool,
    /// The four quadrant points of a circle or arc (a point-on-curve relation).
    pub quadrants: bool,
    /// Perpendicular / collinear / tangent to what the new line starts from.
    pub inference: bool,
}

impl Default for SnapOptions {
    fn default() -> Self {
        SnapOptions {
            points: true,
            edges: true,
            guides: true,
            midpoints: true,
            quadrants: true,
            inference: true,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum InferKind {
    Perpendicular,
    Collinear,
    Tangent,
}

impl InferKind {
    pub fn name(self) -> &'static str {
        match self {
            InferKind::Perpendicular => "perpendicular",
            InferKind::Collinear => "collinear",
            InferKind::Tangent => "tangent",
        }
    }
}

/// A direction the new line was pulled onto, and what it is relative to.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Infer {
    pub kind: InferKind,
    /// The line or arc it is perpendicular / collinear / tangent to.
    pub to: String,
}

/// How close (radians) the pointer's direction must be to snap onto an inferred one.
const INFER_RAD: f64 = 0.052; // about 3 degrees

fn unit(v: P2) -> P2 {
    let l = det::hypot(v.0, v.1).max(1e-12);
    (v.0 / l, v.1 / l)
}

impl AppState {
    /// If the new line from `first` toward `raw` is nearly perpendicular or collinear to a line
    /// that ends at `first`, or tangent to an arc that does, the position pulled onto it.
    pub(crate) fn inferred(&self, first: &Anchor, raw: P2) -> Option<(P2, Infer)> {
        let pid = first.point.as_ref()?;
        let sk = self.sketch_data()?;
        let at = sk.point(pid)?;
        let u = unit((raw.0 - at.0, raw.1 - at.1));
        if dist(raw, at) < 1e-9 {
            return None;
        }
        let mut candidates: Vec<(InferKind, String, P2)> = Vec::new();
        for (id, e) in sk.entities() {
            match e {
                Entity::Line { a, b } if a == pid || b == pid => {
                    let other = if a == pid { b } else { a };
                    let Some(o) = sk.point(other) else { continue };
                    // Away from the other end: continuing the line, or square to it.
                    let away = unit((at.0 - o.0, at.1 - o.1));
                    candidates.push((InferKind::Collinear, id.clone(), away));
                    for sign in [1.0, -1.0] {
                        candidates.push((
                            InferKind::Perpendicular,
                            id.clone(),
                            (-away.1 * sign, away.0 * sign),
                        ));
                    }
                }
                Entity::Arc { center, start, end } if start == pid || end == pid => {
                    let Some(c) = sk.point(center) else { continue };
                    let radial = unit((at.0 - c.0, at.1 - c.1));
                    // Counter-clockwise travel at the end of an arc, the reverse at its start.
                    let ccw = (-radial.1, radial.0);
                    let t = if end == pid { ccw } else { (-ccw.0, -ccw.1) };
                    candidates.push((InferKind::Tangent, id.clone(), t));
                }
                _ => {}
            }
        }
        candidates
            .into_iter()
            // Level and upright directions are the guides' job (they say "horizontal" /
            // "vertical", which is what people mean); inference is for the other angles.
            .filter(|(kind, _, e)| {
                *kind == InferKind::Tangent || (e.0.abs() > 1e-6 && e.1.abs() > 1e-6)
            })
            .filter_map(|(kind, to, e)| {
                let cross = u.0 * e.1 - u.1 * e.0;
                let dot = u.0 * e.0 + u.1 * e.1;
                let angle = det::atan2(cross.abs(), dot);
                (dot > 0.0 && angle < INFER_RAD).then_some((angle, kind, to, e))
            })
            .min_by(|a, b| a.0.total_cmp(&b.0))
            .map(|(_, kind, to, e)| {
                let t = (raw.0 - at.0) * e.0 + (raw.1 - at.1) * e.1;
                ((at.0 + e.0 * t, at.1 + e.1 * t), Infer { kind, to })
            })
    }

    /// The middle of a line the pointer is near, if any: (line, its middle).
    pub(crate) fn midpoint_near(
        &self,
        near: &dyn Fn(P2) -> Option<f64>,
        reach: f64,
    ) -> Option<(String, P2)> {
        let sk = self.sketch_data()?;
        sk.entities()
            .iter()
            .filter_map(|(id, e)| match e {
                Entity::Line { .. } if sk.is_infinite(id) => None,
                Entity::Line { a, b } => {
                    let (pa, pb) = (sk.point(a)?, sk.point(b)?);
                    let mid = ((pa.0 + pb.0) / 2.0, (pa.1 + pb.1) / 2.0);
                    near(mid)
                        .filter(|d| *d <= reach)
                        .map(|d| (d, id.clone(), mid))
                }
                _ => None,
            })
            .min_by(|a, b| a.0.total_cmp(&b.0))
            .map(|(_, id, p)| (id, p))
    }

    /// A quadrant point (right, top, left, bottom) of a circle or arc the pointer is near.
    pub(crate) fn quadrant_near(
        &self,
        near: &dyn Fn(P2) -> Option<f64>,
        reach: f64,
    ) -> Option<(String, P2)> {
        let sk = self.sketch_data()?;
        let tau = core::f64::consts::TAU;
        let mut best: Option<(f64, String, P2)> = None;
        for (id, e) in sk.entities() {
            let (c, r, span) = match e {
                Entity::Circle { center, radius } => (sk.point(center), *radius, None),
                Entity::Arc { center, start, end } => {
                    let (Some(c), Some(s), Some(en)) =
                        (sk.point(center), sk.point(start), sk.point(end))
                    else {
                        continue;
                    };
                    (
                        Some(c),
                        dist(c, s),
                        Some((angle_of(c, s), arc_sweep(c, s, en))),
                    )
                }
                _ => continue,
            };
            let Some(c) = c else { continue };
            for k in 0..4 {
                let a = f64::from(k) * tau / 4.0;
                if let Some((a0, sweep)) = span {
                    let d = (a - a0) % tau;
                    let d = if d < 0.0 { d + tau } else { d };
                    if d > sweep {
                        continue;
                    }
                }
                let p = (c.0 + r * det::cos(a), c.1 + r * det::sin(a));
                if let Some(d) = near(p).filter(|d| *d <= reach) {
                    if best.as_ref().is_none_or(|b| d < b.0) {
                        best = Some((d, id.clone(), p));
                    }
                }
            }
        }
        best.map(|(_, id, p)| (id, p))
    }
}
