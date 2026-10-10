//! Keeping dimensions where they were put relative to what they measure.
//!
//! A dimension's place is stored as absolute sketch coordinates (so "put it there" is a plain
//! command), but when the geometry it measures moves, turns or is resized, the place follows:
//! it keeps its offset in a frame attached to the measured feature, the part along the feature
//! in proportion to its size and the part across it unchanged. Every commit does this, so the
//! GUI, scripts and agents all get the same behaviour, and a dimension never gets left behind
//! when a drag or an edit moves its shape.

use crate::model::{Constraint, Sketch};
use dc_math::det;

type P = (f64, f64);

/// A frame attached to what a dimension measures.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Frame {
    pub anchor: P,
    /// Unit direction along the measured feature. Dimensions whose drawing does not turn with
    /// the geometry (horizontal and vertical distances, radii, angles) keep a fixed direction
    /// (`rotates` is false).
    pub dir: P,
    pub rotates: bool,
    /// How big the measured feature is (a length or radius), so a dimension placed some way
    /// along it stays proportionally that far when it is resized.
    pub scale: f64,
}

fn mid(a: P, b: P) -> P {
    ((a.0 + b.0) / 2.0, (a.1 + b.1) / 2.0)
}

fn len(a: P, b: P) -> f64 {
    det::hypot(b.0 - a.0, b.1 - a.1)
}

fn unit(a: P, b: P) -> P {
    let (dx, dy) = (b.0 - a.0, b.1 - a.1);
    let l = det::hypot(dx, dy);
    if l < 1e-12 {
        (1.0, 0.0)
    } else {
        (dx / l, dy / l)
    }
}

impl Sketch {
    fn line_points(&self, line: &str) -> Option<(P, P)> {
        match self.entity(line)? {
            crate::Entity::Line { a, b } => Some((self.point(a)?, self.point(b)?)),
            _ => None,
        }
    }

    /// The frame a dimension is placed in: attached to the middle of what it measures.
    pub fn dimension_frame(&self, id: &str) -> Option<Frame> {
        let fixed = |anchor: P, dir: P, scale: f64| Frame {
            anchor,
            dir,
            rotates: false,
            scale,
        };
        let turning = |anchor: P, a: P, b: P| Frame {
            anchor,
            dir: unit(a, b),
            rotates: true,
            scale: len(a, b),
        };
        Some(match self.constraints().get(id)? {
            Constraint::Length { line, .. } => {
                let (a, b) = self.line_points(line)?;
                turning(mid(a, b), a, b)
            }
            Constraint::Distance { a, b, .. } => {
                let (pa, pb) = (self.point(a)?, self.point(b)?);
                turning(mid(pa, pb), pa, pb)
            }
            Constraint::HorizontalDistance { a, b, .. } => {
                let (pa, pb) = (self.point(a)?, self.point(b)?);
                fixed(mid(pa, pb), (1.0, 0.0), (pb.0 - pa.0).abs())
            }
            Constraint::VerticalDistance { a, b, .. } => {
                let (pa, pb) = (self.point(a)?, self.point(b)?);
                fixed(mid(pa, pb), (0.0, 1.0), (pb.1 - pa.1).abs())
            }
            Constraint::PointLineDistance { point, line, .. } => {
                let (a, b) = self.line_points(line)?;
                turning(self.point(point)?, a, b)
            }
            Constraint::Radius { target, .. } => match self.entity(target)? {
                crate::Entity::Circle { center, radius } => {
                    fixed(self.point(center)?, (1.0, 0.0), *radius)
                }
                crate::Entity::Arc { center, start, .. } => {
                    let c = self.point(center)?;
                    fixed(c, (1.0, 0.0), len(c, self.point(start)?))
                }
                _ => return None,
            },
            Constraint::Angle { a, b, .. } => {
                let ((a1, a2), (b1, b2)) = (self.line_points(a)?, self.line_points(b)?);
                // Where the two lines meet; the middle of the four ends if they never do.
                let (u, v) = ((a2.0 - a1.0, a2.1 - a1.1), (b2.0 - b1.0, b2.1 - b1.1));
                let cross = u.0 * v.1 - u.1 * v.0;
                let anchor = if cross.abs() > 1e-12 {
                    let d = (b1.0 - a1.0, b1.1 - a1.1);
                    let t = (d.0 * v.1 - d.1 * v.0) / cross;
                    (a1.0 + u.0 * t, a1.1 + u.1 * t)
                } else {
                    mid(mid(a1, a2), mid(b1, b2))
                };
                fixed(anchor, (1.0, 0.0), 1.0)
            }
            _ => return None,
        })
    }

    /// Moves every dimension's place along with the geometry it measures, comparing with the
    /// sketch as it was `before`. A place that was itself changed meanwhile is left alone.
    pub fn follow_placements(&mut self, before: &Sketch) {
        let ids: Vec<String> = self.placements.keys().cloned().collect();
        for id in ids {
            let (Some(old), Some(new)) = (before.placement(&id), self.placement(&id)) else {
                continue;
            };
            if old != new {
                continue; // this very edit put it somewhere
            }
            let (Some(f0), Some(f1)) = (before.dimension_frame(&id), self.dimension_frame(&id))
            else {
                continue;
            };
            if f0 == f1 {
                continue;
            }
            // The offset in the old frame (along the feature, across it), rebuilt in the new one:
            // the part along it grows or shrinks with the feature, the part across does not.
            let rel = (new.0 - f0.anchor.0, new.1 - f0.anchor.1);
            let along = rel.0 * f0.dir.0 + rel.1 * f0.dir.1;
            let across = -rel.0 * f0.dir.1 + rel.1 * f0.dir.0;
            let k = if f0.scale > 1e-9 && f1.scale > 1e-9 {
                f1.scale / f0.scale
            } else {
                1.0
            };
            let dir = if f1.rotates { f1.dir } else { f0.dir };
            let moved = (
                f1.anchor.0 + dir.0 * along * k - dir.1 * across,
                f1.anchor.1 + dir.1 * along * k + dir.0 * across,
            );
            let round = |v: f64| {
                let r = (v * 1e6).round() / 1e6;
                if r == 0.0 {
                    0.0
                } else {
                    r
                }
            };
            self.placements.insert(id, (round(moved.0), round(moved.1)));
        }
    }
}

#[cfg(test)]
mod tests {
    use crate::{Constraint, Entity, Sketch};

    fn rect_with_dims() -> Sketch {
        let mut s = Sketch::new();
        for (id, x, y) in [("a", 0.0, 0.0), ("b", 40.0, 0.0), ("c", 40.0, 20.0)] {
            s.add_entity(id, Entity::Point { x, y }).unwrap();
        }
        s.add_entity(
            "l",
            Entity::Line {
                a: "a".into(),
                b: "b".into(),
            },
        )
        .unwrap();
        s.add_constraint(
            "len",
            Constraint::Length {
                line: "l".into(),
                value: 40.0,
            },
        )
        .unwrap();
        s.add_constraint(
            "hd",
            Constraint::HorizontalDistance {
                a: "a".into(),
                b: "b".into(),
                value: 40.0,
            },
        )
        .unwrap();
        s.add_constraint(
            "rad",
            Constraint::Distance {
                a: "b".into(),
                b: "c".into(),
                value: 20.0,
            },
        )
        .unwrap();
        s.set_placement("len", 20.0, 10.0).unwrap();
        s.set_placement("hd", 20.0, -10.0).unwrap();
        s.set_placement("rad", 55.0, 10.0).unwrap();
        s
    }

    fn near(a: (f64, f64), b: (f64, f64)) -> bool {
        (a.0 - b.0).abs() < 1e-6 && (a.1 - b.1).abs() < 1e-6
    }

    #[test]
    fn a_dimension_keeps_its_offset_when_what_it_measures_moves() {
        let before = rect_with_dims();
        let mut s = before.clone();
        // Slide the whole thing 10 right and 5 up.
        for id in ["a", "b", "c"] {
            let (x, y) = s.point(id).unwrap();
            s.set_point(id, x + 10.0, y + 5.0).unwrap();
        }
        s.follow_placements(&before);
        assert!(near(s.placement("len").unwrap(), (30.0, 15.0)));
        assert!(near(s.placement("hd").unwrap(), (30.0, -5.0)));
        assert!(near(s.placement("rad").unwrap(), (65.0, 15.0)));
        // Nothing moved: nothing changes (and no rounding noise is introduced).
        let mut same = before.clone();
        same.follow_placements(&before);
        assert_eq!(same, before);
    }

    #[test]
    fn resizing_keeps_a_dimension_centred_and_turning_a_line_turns_its_dimension() {
        let before = rect_with_dims();
        // Make the line twice as long: the middle moves right by 20, so the dimension does too.
        let mut s = before.clone();
        s.set_point("b", 80.0, 0.0).unwrap();
        s.follow_placements(&before);
        assert!(
            near(s.placement("len").unwrap(), (40.0, 10.0)),
            "{:?}",
            s.placement("len")
        );
        // The horizontal distance's middle moved the same way and its direction is fixed.
        assert!(near(s.placement("hd").unwrap(), (40.0, -10.0)));
        // Turn the line 90 degrees about its start: the aligned dimension (10 mm to its left
        // side, as seen along the line) goes round with it; the horizontal one does not turn.
        let mut t = before.clone();
        t.set_point("b", 0.0, 40.0).unwrap();
        t.follow_placements(&before);
        // The line's middle was (20, 0) with the dimension at (20, 10), 10 to its left; now the
        // middle is (0, 20) and "left of an upward line" is -x.
        let p = t.placement("len").unwrap();
        assert!(near(p, (-10.0, 20.0)), "{p:?}");
        assert!(
            near(t.placement("hd").unwrap(), (0.0, 10.0)),
            "{:?}",
            t.placement("hd")
        );
    }

    #[test]
    fn a_place_changed_by_the_same_edit_is_not_moved_again() {
        let before = rect_with_dims();
        let mut s = before.clone();
        s.set_point("b", 80.0, 0.0).unwrap();
        s.set_placement("len", 100.0, 100.0).unwrap();
        s.follow_placements(&before);
        assert_eq!(
            s.placement("len"),
            Some((100.0, 100.0)),
            "the new place wins"
        );
        assert!(
            near(s.placement("hd").unwrap(), (40.0, -10.0)),
            "the others still follow"
        );
        // A dimension that did not exist before has nothing to follow.
        let mut u = before.clone();
        u.add_constraint(
            "new",
            Constraint::Length {
                line: "l".into(),
                value: 40.0,
            },
        )
        .unwrap();
        u.set_placement("new", 1.0, 2.0).unwrap();
        let w = u.clone();
        u.set_point("b", 80.0, 0.0).unwrap();
        u.follow_placements(&w);
        assert!(u.placement("new").is_some());
    }
    #[test]
    fn a_dimension_placed_along_a_feature_keeps_its_proportion_when_it_is_resized() {
        let mut b = rect_with_dims();
        // Off-centre: 10 mm right of the middle of the 40 mm line.
        b.set_placement("len", 30.0, 10.0).unwrap();
        let before = b.clone();
        let mut s = b;
        s.set_point("b", 80.0, 0.0).unwrap(); // the line doubles
        s.follow_placements(&before);
        // Middle moved to x = 40; 10 mm along became 20 mm along; 10 mm across stays.
        assert!(
            near(s.placement("len").unwrap(), (60.0, 10.0)),
            "{:?}",
            s.placement("len")
        );
        // Turning and resizing at once: 90 degrees and double length.
        let mut t = before.clone();
        t.set_point("b", 0.0, 80.0).unwrap();
        t.follow_placements(&before);
        // Middle (0, 40); along = up, 20 mm; across is left: 10 mm -> (-10, 60).
        assert!(
            near(t.placement("len").unwrap(), (-10.0, 60.0)),
            "{:?}",
            t.placement("len")
        );
        // Turned by more than a half turn the dimension stays on the same side of the line.
        for deg in [120.0_f64, 200.0, 300.0] {
            let mut u = before.clone();
            let r = deg.to_radians();
            u.set_point(
                "b",
                40.0 * dc_math::det::cos(r),
                40.0 * dc_math::det::sin(r),
            )
            .unwrap();
            u.follow_placements(&before);
            let p = u.placement("len").unwrap();
            let mid = (20.0 * dc_math::det::cos(r), 20.0 * dc_math::det::sin(r));
            let (along, across) = (
                (p.0 - mid.0) * dc_math::det::cos(r) + (p.1 - mid.1) * dc_math::det::sin(r),
                -(p.0 - mid.0) * dc_math::det::sin(r) + (p.1 - mid.1) * dc_math::det::cos(r),
            );
            assert!(
                (along - 10.0).abs() < 1e-5 && (across - 10.0).abs() < 1e-5,
                "{deg}: {along} {across}"
            );
        }
    }
}
