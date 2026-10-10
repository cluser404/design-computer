//! The extra drawing tools: 3-point rectangles, parallelogram, polygon, slot, tangent arc and
//! point. Each is one `Transaction` of ordinary sketch commands (so an agent can do the same),
//! built from the clicks. The geometry is plain functions so previews and results agree. No egui.

use crate::sketching::{angle_of, Anchor, Tool, P2};
use crate::state::AppState;
use dc_math::det;
use dc_sketch::{Constraint, Entity};

fn sub(a: P2, b: P2) -> P2 {
    (a.0 - b.0, a.1 - b.1)
}
fn add(a: P2, b: P2) -> P2 {
    (a.0 + b.0, a.1 + b.1)
}
fn mul(a: P2, k: f64) -> P2 {
    (a.0 * k, a.1 * k)
}
fn dot(a: P2, b: P2) -> f64 {
    a.0 * b.0 + a.1 * b.1
}
fn len(a: P2) -> f64 {
    det::hypot(a.0, a.1)
}
fn unit(a: P2) -> P2 {
    let l = len(a).max(1e-12);
    (a.0 / l, a.1 / l)
}
fn perp(a: P2) -> P2 {
    (-a.1, a.0)
}

/// Corners of the rectangle with one side from `a` to `b` and `c` giving the width.
pub fn rect3_corners(a: P2, b: P2, c: P2) -> [P2; 4] {
    let u = unit(sub(b, a));
    let n = perp(u);
    let width = dot(sub(c, b), n);
    [a, b, add(b, mul(n, width)), add(a, mul(n, width))]
}

/// Corners of the rectangle centred on `c`, with `m` the middle of one side and `k` giving the
/// distance to the sides along the other direction.
pub fn rect3_centre_corners(c: P2, m: P2, k: P2) -> [P2; 4] {
    let (e1, half_w) = (unit(sub(m, c)), len(sub(m, c)));
    let v = perp(e1);
    let half_h = dot(sub(k, c), v).abs();
    let at = |sw: f64, sh: f64| add(c, add(mul(e1, sw * half_w), mul(v, sh * half_h)));
    [at(1.0, 1.0), at(-1.0, 1.0), at(-1.0, -1.0), at(1.0, -1.0)]
}

/// The vertices of a regular polygon about `c`: the first at (or, circumscribed, beside) `edge`.
/// Returns the vertices and the circle radius they lie on.
pub fn polygon_vertices(c: P2, edge: P2, sides: u32, circumscribed: bool) -> (Vec<P2>, f64) {
    let n = f64::from(sides.max(3));
    let radius = len(sub(edge, c));
    let theta = angle_of(c, edge);
    let (rc, phase) = if circumscribed {
        // `edge` is the middle of a side; the circle through the corners is larger.
        (
            radius / det::cos(core::f64::consts::PI / n),
            core::f64::consts::PI / n,
        )
    } else {
        (radius, 0.0)
    };
    let pts = (0..sides.max(3))
        .map(|k| {
            let a = theta + phase + core::f64::consts::TAU * f64::from(k) / n;
            (c.0 + rc * det::cos(a), c.1 + rc * det::sin(a))
        })
        .collect();
    (pts, rc)
}

/// A straight slot's pieces: the two lines (as point pairs), the two arcs as
/// (centre, start, end) counter-clockwise, and the radius.
pub struct SlotShape {
    pub lines: [(P2, P2); 2],
    pub arcs: [(P2, P2, P2); 2],
    pub radius: f64,
}

pub fn slot_shape(c1: P2, c2: P2, click: P2) -> Option<SlotShape> {
    let u = unit(sub(c2, c1));
    if len(sub(c2, c1)) < 1e-9 {
        return None;
    }
    let n = perp(u);
    let r = dot(sub(click, c1), n).abs();
    if r < 1e-6 {
        return None;
    }
    let off = |p: P2, s: f64| add(p, mul(n, s * r));
    Some(SlotShape {
        lines: [(off(c1, 1.0), off(c2, 1.0)), (off(c1, -1.0), off(c2, -1.0))],
        // Round the first end counter-clockwise from +n to -n (through -u), the second from -n to +n.
        arcs: [
            (c1, off(c1, 1.0), off(c1, -1.0)),
            (c2, off(c2, -1.0), off(c2, 1.0)),
        ],
        radius: r,
    })
}

/// What a tangent arc continues from: the direction of travel at its start point, and what it
/// continues (a line, or an arc).
#[derive(Clone, Debug)]
pub struct TangentContext {
    pub dir: P2,
    pub from_line: Option<String>,
    pub from_arc: Option<String>,
}

/// The arc from `p` (leaving along `dir`) to `e`: centre, then start and end counter-clockwise.
pub fn tangent_arc_geometry(p: P2, dir: P2, e: P2) -> Option<(P2, P2, P2)> {
    let n = perp(dir);
    let d = sub(e, p);
    let k = dot(n, d);
    if k.abs() < 1e-9 {
        return None; // straight ahead or behind: that is a line, not an arc
    }
    let s = dot(d, d) / (2.0 * k);
    let c = add(p, mul(n, s));
    let cross = (p.0 - c.0) * (e.1 - c.1) - (p.1 - c.1) * (e.0 - c.0);
    // Travelling along `dir` is counter-clockwise exactly when the centre is on the left (s > 0).
    let ccw = s > 0.0;
    let _ = cross;
    Some(if ccw { (c, p, e) } else { (c, e, p) })
}

impl AppState {
    /// The tangent direction at a point where a line or an arc ends (so an arc can leave it
    /// smoothly), if the point is such an end.
    pub fn tangent_context(&self, point: &str) -> Option<TangentContext> {
        let sk = self.sketch_data()?;
        let at = sk.point(point)?;
        for (id, e) in sk.entities() {
            match e {
                Entity::Line { a, b } if a == point || b == point => {
                    let other = sk.point(if a == point { b } else { a })?;
                    return Some(TangentContext {
                        dir: unit(sub(at, other)),
                        from_line: Some(id.clone()),
                        from_arc: None,
                    });
                }
                Entity::Arc { center, start, end } if start == point || end == point => {
                    let c = sk.point(center)?;
                    let radial = unit(sub(at, c));
                    let ccw = perp(radial);
                    return Some(TangentContext {
                        dir: if end == point { ccw } else { mul(ccw, -1.0) },
                        from_line: None,
                        from_arc: Some(id.clone()),
                    });
                }
                _ => {}
            }
        }
        None
    }

    fn polygon_settings(&self) -> (u32, bool) {
        self.sketching
            .as_ref()
            .map_or((6, false), |s| (s.polygon_sides, s.polygon_circumscribed))
    }

    /// Lines for a shape in progress: what it would be if the next click were here.
    pub(crate) fn new_shape_preview(
        &self,
        tool: Tool,
        first: &Anchor,
        second: Option<&Anchor>,
        cur: P2,
    ) -> (Vec<Vec<P2>>, Option<String>) {
        let closed = |pts: &[P2]| {
            let mut v = pts.to_vec();
            v.push(pts[0]);
            v
        };
        match (tool, second) {
            (Tool::ThreePointRect, None)
            | (Tool::ThreePointCentreRect, None)
            | (Tool::Parallelogram, None)
            | (Tool::Slot, None) => (vec![vec![first.pos, cur]], None),
            (Tool::ThreePointRect, Some(b)) => {
                let c = rect3_corners(first.pos, b.pos, cur);
                let w = dot(sub(cur, b.pos), perp(unit(sub(b.pos, first.pos)))).abs();
                (
                    vec![closed(&c)],
                    Some(format!("{:.2} × {:.2}", len(sub(b.pos, first.pos)), w)),
                )
            }
            (Tool::ThreePointCentreRect, Some(m)) => {
                let c = rect3_centre_corners(first.pos, m.pos, cur);
                (
                    vec![closed(&c), vec![c[0], c[2]], vec![c[1], c[3]]],
                    Some(format!(
                        "{:.2} × {:.2}",
                        2.0 * len(sub(c[0], c[1])) / 2.0,
                        len(sub(c[1], c[2]))
                    )),
                )
            }
            (Tool::Parallelogram, Some(b)) => {
                let p4 = add(first.pos, sub(cur, b.pos));
                (vec![closed(&[first.pos, b.pos, cur, p4])], None)
            }
            (Tool::Polygon, _) => {
                let (n, circ) = self.polygon_settings();
                let (pts, rc) = polygon_vertices(first.pos, cur, n, circ);
                let circle: Vec<P2> = (0..=48)
                    .map(|i| {
                        let a = core::f64::consts::TAU * f64::from(i) / 48.0;
                        (
                            first.pos.0 + rc * det::cos(a),
                            first.pos.1 + rc * det::sin(a),
                        )
                    })
                    .collect();
                (vec![closed(&pts), circle], Some(format!("{n} sides")))
            }
            (Tool::Slot, Some(b)) => match slot_shape(first.pos, b.pos, cur) {
                Some(s) => {
                    let mut out: Vec<Vec<P2>> = s.lines.iter().map(|(a, b)| vec![*a, *b]).collect();
                    for (c, st, en) in s.arcs {
                        let a0 = angle_of(c, st);
                        out.push(
                            (0..=24)
                                .map(|i| {
                                    let t = a0 + core::f64::consts::PI * f64::from(i) / 24.0;
                                    (c.0 + s.radius * det::cos(t), c.1 + s.radius * det::sin(t))
                                })
                                .collect(),
                        );
                        let _ = en;
                    }
                    (out, Some(format!("width {:.2}", 2.0 * s.radius)))
                }
                None => (vec![vec![first.pos, b.pos]], None),
            },
            (Tool::TangentArc, _) => {
                let ctx = first.point.as_ref().and_then(|p| self.tangent_context(p));
                match ctx.and_then(|c| tangent_arc_geometry(first.pos, c.dir, cur)) {
                    Some((c, st, en)) => {
                        let r = len(sub(st, c));
                        let a0 = angle_of(c, st);
                        let sweep = {
                            let t = core::f64::consts::TAU;
                            let d = (angle_of(c, en) - a0) % t;
                            if d < 0.0 {
                                d + t
                            } else {
                                d
                            }
                        };
                        (
                            vec![(0..=32)
                                .map(|i| {
                                    let t = a0 + sweep * f64::from(i) / 32.0;
                                    (c.0 + r * det::cos(t), c.1 + r * det::sin(t))
                                })
                                .collect()],
                            Some(format!("R{r:.2}")),
                        )
                    }
                    None => (vec![vec![first.pos, cur]], None),
                }
            }
            _ => (Vec::new(), None),
        }
    }

    // ---- building -----------------------------------------------------------------------------

    fn perpendicular_corners(bt: &mut crate::sketching::Batch, p: [&String; 4]) {
        let l: Vec<String> = (0..4).map(|i| bt.line(p[i], p[(i + 1) % 4])).collect();
        for i in 0..3 {
            bt.constraint(
                "pe",
                Constraint::Perpendicular {
                    a: l[i].clone(),
                    b: l[i + 1].clone(),
                },
            );
        }
    }

    pub(crate) fn add_rect3(&mut self, a: &Anchor, b: &Anchor, c: P2) -> Result<(), ()> {
        let corners = rect3_corners(a.pos, b.pos, c);
        if len(sub(corners[1], corners[0])) < 1e-6 || len(sub(corners[2], corners[1])) < 1e-6 {
            self.error("a rectangle needs a length and a width", None);
            return Err(());
        }
        let mut bt = self.batch().ok_or(())?;
        let p1 = bt.point(a);
        let p2 = bt.point(b);
        let p3 = bt.corner(corners[2]);
        let p4 = bt.corner(corners[3]);
        Self::perpendicular_corners(&mut bt, [&p1, &p2, &p3, &p4]);
        self.commit("Add 3-point rectangle", bt)
    }

    pub(crate) fn add_rect3_centre(&mut self, c: &Anchor, m: &Anchor, k: P2) -> Result<(), ()> {
        let corners = rect3_centre_corners(c.pos, m.pos, k);
        if len(sub(corners[1], corners[0])) < 1e-6 || len(sub(corners[2], corners[1])) < 1e-6 {
            self.error("a rectangle needs a length and a width", None);
            return Err(());
        }
        let mut bt = self.batch().ok_or(())?;
        let centre = bt.point(c);
        let p: Vec<String> = corners.iter().map(|q| bt.corner(*q)).collect();
        Self::perpendicular_corners(&mut bt, [&p[0], &p[1], &p[2], &p[3]]);
        // Construction diagonals through the centre, as the centre rectangle has.
        for (x, y) in [(0, 2), (1, 3)] {
            let d = bt.push_entity(
                "l",
                Entity::Line {
                    a: p[x].clone(),
                    b: p[y].clone(),
                },
            );
            bt.mark_construction(&d);
            bt.constraint(
                "ol",
                Constraint::PointOnLine {
                    point: centre.clone(),
                    line: d,
                },
            );
        }
        self.commit("Add 3-point centre rectangle", bt)
    }

    pub(crate) fn add_parallelogram(
        &mut self,
        a: &Anchor,
        b: &Anchor,
        c: &Anchor,
    ) -> Result<(), ()> {
        let p4 = add(a.pos, sub(c.pos, b.pos));
        let area =
            (b.pos.0 - a.pos.0) * (c.pos.1 - b.pos.1) - (b.pos.1 - a.pos.1) * (c.pos.0 - b.pos.0);
        if area.abs() < 1e-9 {
            self.error(
                "those three points are in a line, so there is no parallelogram",
                None,
            );
            return Err(());
        }
        let mut bt = self.batch().ok_or(())?;
        let (p1, p2, p3) = (bt.point(a), bt.point(b), bt.point(c));
        let p4 = bt.corner(p4);
        let l = [
            bt.line(&p1, &p2),
            bt.line(&p2, &p3),
            bt.line(&p3, &p4),
            bt.line(&p4, &p1),
        ];
        for (x, y) in [(0, 2), (1, 3)] {
            bt.constraint(
                "pa",
                Constraint::Parallel {
                    a: l[x].clone(),
                    b: l[y].clone(),
                },
            );
        }
        self.commit("Add parallelogram", bt)
    }

    pub(crate) fn add_polygon(&mut self, c: &Anchor, edge: &Anchor) -> Result<(), ()> {
        let (n, circumscribed) = self.polygon_settings();
        if len(sub(edge.pos, c.pos)) < 1e-6 {
            self.error("a polygon needs a size: click away from the centre", None);
            return Err(());
        }
        let (pts, rc) = polygon_vertices(c.pos, edge.pos, n, circumscribed);
        let mut bt = self.batch().ok_or(())?;
        let centre = bt.point(c);
        // Corner polygons stand on the circle through their corners; side polygons wrap round
        // the circle their sides touch (the one the click sized).
        let guide_radius = if circumscribed {
            len(sub(edge.pos, c.pos))
        } else {
            rc
        };
        let circle = bt.push_entity(
            "k",
            Entity::Circle {
                center: centre,
                radius: guide_radius,
            },
        );
        bt.mark_construction(&circle);
        let ids: Vec<String> = pts.iter().map(|p| bt.corner(*p)).collect();
        let lines: Vec<String> = (0..ids.len())
            .map(|i| bt.line(&ids[i], &ids[(i + 1) % ids.len()]))
            .collect();
        for i in 0..lines.len() {
            if circumscribed {
                bt.constraint(
                    "t",
                    Constraint::Tangent {
                        line: lines[i].clone(),
                        curve: circle.clone(),
                    },
                );
            } else {
                bt.constraint(
                    "oc",
                    Constraint::PointOnCurve {
                        point: ids[i].clone(),
                        curve: circle.clone(),
                    },
                );
            }
            if i + 1 < lines.len() {
                bt.constraint(
                    "eq",
                    Constraint::EqualLength {
                        a: lines[i].clone(),
                        b: lines[i + 1].clone(),
                    },
                );
            }
        }
        self.commit(&format!("Add {n}-sided polygon"), bt)
    }

    pub(crate) fn add_slot(&mut self, c1: &Anchor, c2: &Anchor, click: P2) -> Result<(), ()> {
        let Some(shape) = slot_shape(c1.pos, c2.pos, click) else {
            self.error("a slot needs two different centres and a width", None);
            return Err(());
        };
        let mut bt = self.batch().ok_or(())?;
        let (m1, m2) = (bt.point(c1), bt.point(c2));
        let mut line_ids = Vec::new();
        for (a, b) in shape.lines {
            let (pa, pb) = (bt.corner(a), bt.corner(b));
            line_ids.push((pa.clone(), pb.clone(), bt.line(&pa, &pb)));
        }
        let (first, second) = (shape.arcs[0], shape.arcs[1]);
        // Arc ends are the line ends: the arc at c1 runs from the +n line's start to the -n line's start.
        let arc1 = bt.shape(
            "a",
            Entity::Arc {
                center: m1.clone(),
                start: line_ids[0].0.clone(),
                end: line_ids[1].0.clone(),
            },
        );
        let arc2 = bt.shape(
            "a",
            Entity::Arc {
                center: m2.clone(),
                start: line_ids[1].1.clone(),
                end: line_ids[0].1.clone(),
            },
        );
        let _ = (first, second);
        for l in [&line_ids[0].2, &line_ids[1].2] {
            for a in [&arc1, &arc2] {
                bt.constraint(
                    "t",
                    Constraint::Tangent {
                        line: l.clone(),
                        curve: a.clone(),
                    },
                );
            }
        }
        bt.constraint("eq", Constraint::EqualRadius { a: arc1, b: arc2 });
        // The centre line, as construction geometry.
        let axis = bt.push_entity("l", Entity::Line { a: m1, b: m2 });
        bt.mark_construction(&axis);
        self.commit("Add slot", bt)
    }

    pub(crate) fn add_tangent_arc(&mut self, start: &Anchor, end: &Anchor) -> Result<(), ()> {
        let Some(p) = start.point.clone() else {
            self.error(
                "a tangent arc starts from the end of a line or arc",
                Some("click that end point first".into()),
            );
            return Err(());
        };
        let Some(ctx) = self.tangent_context(&p) else {
            self.error(
                "that point is not the end of a line or arc to continue from",
                None,
            );
            return Err(());
        };
        let Some((c, s, e)) = tangent_arc_geometry(start.pos, ctx.dir, end.pos) else {
            self.error(
                "that end point is straight ahead of the line: it needs to be off to one side",
                None,
            );
            return Err(());
        };
        let mut bt = self.batch().ok_or(())?;
        let centre = bt.corner(c);
        // The start is the existing point; the end is the click (or a point it snapped to).
        let (st, en) = if (s.0 - start.pos.0).abs() < 1e-9 && (s.1 - start.pos.1).abs() < 1e-9 {
            (p.clone(), bt.point(end))
        } else {
            (bt.point(end), p.clone())
        };
        let _ = (e, s);
        let arc = bt.shape(
            "a",
            Entity::Arc {
                center: centre,
                start: st,
                end: en,
            },
        );
        if let Some(line) = ctx.from_line {
            bt.constraint("t", Constraint::Tangent { line, curve: arc });
        } else if let Some(other) = ctx.from_arc {
            bt.constraint("t", Constraint::TangentCurves { a: other, b: arc });
        }
        self.commit("Add tangent arc", bt)
    }

    pub(crate) fn add_point_at(&mut self, at: &Anchor) -> Result<(), ()> {
        if at.point.is_some() {
            self.error("there is already a point there", None);
            return Err(());
        }
        let mut bt = self.batch().ok_or(())?;
        bt.point(at);
        self.commit("Add point", bt)
    }
}
