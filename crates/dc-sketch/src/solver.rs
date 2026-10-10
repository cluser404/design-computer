//! Geometric constraint solver.
//!
//! * Each constraint becomes one or more scalar residual equations over the sketch's variables
//!   (point coordinates, circle radii).
//! * Jacobians come from **forward-mode automatic differentiation** ([`Dual`]), so they are exact
//!   and there is no hand-derived math to get wrong.
//! * The system is solved with Levenberg–Marquardt (Marquardt scaling, dense Cholesky).
//! * Residuals are scaled to length-like (or unitless) quantities when the system is built, so a
//!   single absolute tolerance is meaningful from micrometres to metres.
//! * After solving, the Jacobian is analysed: Gauss–Jordan elimination gives the degrees of
//!   freedom and which entities are still free; Gram–Schmidt over the constraint rows (in ID
//!   order) finds redundant and conflicting constraints.
//!
//! A failed solve never changes the sketch: the previous geometry is kept (`applied == false`).
//! Everything is deterministic: ordered maps, fixed loop order, no platform math.

// Dense numeric kernels read more clearly with index loops.
#![allow(clippy::needless_range_loop)]

use crate::model::{Constraint, Entity, Sketch, SketchError};
use crate::sparse::Profile;
use dc_math::det;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};
use std::ops::{Add, Div, Mul, Neg, Sub};

/// Max |residual| for the system to count as solved. Residuals are length-like or unitless.
const SOLVED_TOL: f64 = 1e-9;
/// Once solved, keep refining (Newton converges quadratically, so this is cheap) until residuals
/// are this small, so results are essentially exact (40 mm comes out as 40, not 40.0000000001).
const POLISH_TOL: f64 = 1e-13;
/// Upper bound on refinement iterations after reaching `SOLVED_TOL`.
const MAX_POLISH: u32 = 4;
/// A constraint with a dependent row and a residual above this is *conflicting*, not redundant.
const CONFLICT_TOL: f64 = 1e-6;
const MAX_ITERATIONS: u32 = 200;
/// A line shorter or a circle smaller than this (mm, one micrometre) has collapsed: not a valid solution.
const COLLAPSED: f64 = 1e-3;
/// Maximum number of local scalars any single equation touches (two lines = 8 coordinates).
const MAXV: usize = 8;
/// Where a dragged point should end up.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct DragTarget {
    pub point: String,
    pub x: f64,
    pub y: f64,
}

/// The radius a dragged circle should end up with.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct DragRadius {
    pub circle: String,
    pub value: f64,
}

/// Something held fixed while the rest of the sketch adapts (a drag).
#[derive(Clone, Copy, Debug)]
enum Hold<'a> {
    Point(&'a str, f64, f64),
    Radius(&'a str, f64),
}

/// Owner name of the temporary equation that holds a dragged point at the cursor.
const DRAG_OWNER: &str = "__drag";

// ---------------------------------------------------------------------------------------------
// Scalars: plain f64 and dual numbers (value + gradient over the equation's local variables)
// ---------------------------------------------------------------------------------------------

trait Num:
    Copy
    + Add<Output = Self>
    + Sub<Output = Self>
    + Mul<Output = Self>
    + Div<Output = Self>
    + Neg<Output = Self>
{
    fn c(v: f64) -> Self;
    fn sqrt(self) -> Self;
    /// The angle of the point `(x, y)` = `(other, self)`, in (-pi, pi].
    fn atan2(self, x: Self) -> Self;
}

impl Num for f64 {
    fn c(v: f64) -> f64 {
        v
    }
    fn sqrt(self) -> f64 {
        det::sqrt(self)
    }
    fn atan2(self, x: f64) -> f64 {
        det::atan2(self, x)
    }
}

#[derive(Clone, Copy)]
struct Dual {
    v: f64,
    g: [f64; MAXV],
}

impl Dual {
    fn var(v: f64, i: usize) -> Self {
        let mut g = [0.0; MAXV];
        g[i] = 1.0;
        Dual { v, g }
    }
}

impl Num for Dual {
    fn c(v: f64) -> Dual {
        Dual { v, g: [0.0; MAXV] }
    }
    fn sqrt(self) -> Dual {
        let r = det::sqrt(self.v);
        let k = 0.5 / r.max(1e-300);
        let mut g = self.g;
        for gi in &mut g {
            *gi *= k;
        }
        Dual { v: r, g }
    }
    fn atan2(self, x: Dual) -> Dual {
        let (y, den) = (self, (self.v * self.v + x.v * x.v).max(1e-300));
        let mut g = [0.0; MAXV];
        for (i, gi) in g.iter_mut().enumerate() {
            *gi = (x.v * y.g[i] - y.v * x.g[i]) / den;
        }
        Dual {
            v: det::atan2(y.v, x.v),
            g,
        }
    }
}

impl Add for Dual {
    type Output = Dual;
    fn add(self, o: Dual) -> Dual {
        let mut g = self.g;
        for i in 0..MAXV {
            g[i] += o.g[i];
        }
        Dual { v: self.v + o.v, g }
    }
}
impl Sub for Dual {
    type Output = Dual;
    fn sub(self, o: Dual) -> Dual {
        let mut g = self.g;
        for i in 0..MAXV {
            g[i] -= o.g[i];
        }
        Dual { v: self.v - o.v, g }
    }
}
impl Mul for Dual {
    type Output = Dual;
    fn mul(self, o: Dual) -> Dual {
        let mut g = [0.0; MAXV];
        for i in 0..MAXV {
            g[i] = self.g[i] * o.v + o.g[i] * self.v;
        }
        Dual { v: self.v * o.v, g }
    }
}
impl Div for Dual {
    type Output = Dual;
    fn div(self, o: Dual) -> Dual {
        let mut g = [0.0; MAXV];
        let d2 = o.v * o.v;
        for i in 0..MAXV {
            g[i] = (self.g[i] * o.v - self.v * o.g[i]) / d2;
        }
        Dual { v: self.v / o.v, g }
    }
}
impl Neg for Dual {
    type Output = Dual;
    fn neg(self) -> Dual {
        let mut g = self.g;
        for gi in &mut g {
            *gi = -*gi;
        }
        Dual { v: -self.v, g }
    }
}

// ---------------------------------------------------------------------------------------------
// Equations
// ---------------------------------------------------------------------------------------------

/// Residual formulas. `x` layouts are documented per variant; `scale` constants are chosen when
/// the system is built so residuals are length-like or unitless.
#[derive(Clone, Copy, Debug)]
enum Kernel {
    /// x = [ax, ay, bx, by]
    Coincident,
    /// x = [ay, by]
    Horizontal,
    /// x = [ax, bx]
    Vertical,
    /// x = [p1x,p1y,p2x,p2y, p3x,p3y,p4x,p4y]; cross(p2-p1, p4-p3)
    Parallel { scale: f64 },
    /// same layout; dot(p2-p1, p4-p3)
    Perpendicular { scale: f64 },
    /// same layout; |p2-p1|² - |p4-p3|²
    EqualLength { scale: f64 },
    /// x = [ax, ay, bx, by]; |b-a| = value (also arc radius: center to start)
    Distance { value: f64 },
    /// x = [ax, ay, bx, by]; bx - ax = value
    HDist { value: f64 },
    /// x = [ax, ay, bx, by]; by - ay = value
    VDist { value: f64 },
    /// x = [px,py, ax,ay, bx,by]; signed distance from p to the line ab (left is positive) = value
    PointLine { value: f64 },
    /// x = [r]
    CircleRadius { value: f64 },
    /// x = [px, py]
    Fixed { x: f64, y: f64 },
    /// x = [px,py, ax,ay, bx,by]; p is the middle of ab
    Midpoint,
    /// x = [ax,ay, bx,by, lax,lay, lbx,lby]; a and b mirror each other across the line l
    Symmetric { scale: f64 },
    /// x = [ra, rb]
    EqRadiusCC,
    /// x = [r, cx,cy, sx,sy]; circle radius equals the arc's
    EqRadiusCA,
    /// x = [c1x,c1y, s1x,s1y, c2x,c2y, s2x,s2y]; two arcs' radii agree
    EqRadiusAA,
    /// x = [px,py, cx,cy, r]; p is on the circle
    OnCircle,
    /// x = [px,py, cx,cy, sx,sy]; p is on the circle of the arc
    OnArc,
    /// x = [u1x,u1y, u2x,u2y, v1x,v1y, v2x,v2y]; the angle from line u to line v is `theta`
    /// (satisfied when sin(angle - theta) = 0, the branch nearest the current shape)
    Angle { cos: f64, sin: f64, scale: f64 },
    /// x = [ax,ay, bx,by]; a and b mirror each other across the horizontal axis
    SymXAxis,
    /// x = [ax,ay, bx,by]; across the vertical axis
    SymYAxis,
    /// x = [c1x,c1y, c2x,c2y, r1, r2]; two circles touch: |c1 c2| = k1 r1 + k2 r2
    TanCC { k1: f64, k2: f64 },
    /// x = [c1x,c1y, c2x,c2y, r1, sx,sy]; a circle and an arc (start point s)
    TanCA { k1: f64, k2: f64 },
    /// x = [c1x,c1y, s1x,s1y, c2x,c2y, s2x,s2y]; two arcs
    TanAA { k1: f64, k2: f64 },
    /// x = [cx,cy, sx,sy, ex,ey]; arc length (radius * sweep) = value. The sweep is measured
    /// counter-clockwise and unwrapped around `shift` (the middle of where it is and where it is
    /// going), so the equation is smooth all the way between them.
    ArcLen { value: f64, shift: f64 },
    /// x = [px, py]; py = 0 (on the horizontal axis)
    OnXAxis,
    /// x = [px, py]; px = 0 (on the vertical axis)
    OnYAxis,
    /// x = [px,py, ax,ay, bx,by]; distance from p to the infinite line ab
    PointOnLine { scale: f64 },
    /// x = [ax,ay, bx,by, cx,cy, r]; dist(c, line ab)² = r²
    TangentCircle { scale: f64 },
    /// x = [ax,ay, bx,by, cx,cy, sx,sy]; r² = |c-s|²
    TangentArc { scale: f64 },
    /// x = [cx,cy, sx,sy, ex,ey]; |c-s|² = |c-e|² (an arc's two radii agree)
    ArcConsistency { scale: f64 },
}

impl Kernel {
    fn arity(self) -> usize {
        match self {
            Kernel::Coincident
            | Kernel::Fixed { .. }
            | Kernel::Midpoint
            | Kernel::SymXAxis
            | Kernel::SymYAxis
            | Kernel::Symmetric { .. } => 2,
            _ => 1,
        }
    }
}

fn eval<T: Num>(k: Kernel, x: &[T], out: &mut Vec<T>) {
    match k {
        Kernel::Coincident => {
            out.push(x[0] - x[2]);
            out.push(x[1] - x[3]);
        }
        Kernel::Horizontal | Kernel::Vertical => out.push(x[0] - x[1]),
        Kernel::Parallel { scale } => {
            let (ux, uy) = (x[2] - x[0], x[3] - x[1]);
            let (vx, vy) = (x[6] - x[4], x[7] - x[5]);
            out.push((ux * vy - uy * vx) * T::c(scale));
        }
        Kernel::Perpendicular { scale } => {
            let (ux, uy) = (x[2] - x[0], x[3] - x[1]);
            let (vx, vy) = (x[6] - x[4], x[7] - x[5]);
            out.push((ux * vx + uy * vy) * T::c(scale));
        }
        Kernel::EqualLength { scale } => {
            let (ux, uy) = (x[2] - x[0], x[3] - x[1]);
            let (vx, vy) = (x[6] - x[4], x[7] - x[5]);
            out.push((ux * ux + uy * uy - vx * vx - vy * vy) * T::c(scale));
        }
        Kernel::Distance { value } => {
            let (dx, dy) = (x[2] - x[0], x[3] - x[1]);
            let w = 1.0 / (2.0 * value.max(1e-3));
            out.push((dx * dx + dy * dy - T::c(value * value)) * T::c(w));
        }
        Kernel::HDist { value } => out.push(x[2] - x[0] - T::c(value)),
        Kernel::VDist { value } => out.push(x[3] - x[1] - T::c(value)),
        Kernel::PointLine { value } => {
            let (ux, uy) = (x[4] - x[2], x[5] - x[3]);
            let (wx, wy) = (x[0] - x[2], x[1] - x[3]);
            let len = (ux * ux + uy * uy + T::c(1e-18)).sqrt();
            out.push((ux * wy - uy * wx) / len - T::c(value));
        }
        Kernel::CircleRadius { value } => out.push(x[0] - T::c(value)),
        Kernel::Fixed { x: fx, y: fy } => {
            out.push(x[0] - T::c(fx));
            out.push(x[1] - T::c(fy));
        }
        Kernel::Midpoint => {
            let half = T::c(0.5);
            out.push(x[0] - (x[2] + x[4]) * half);
            out.push(x[1] - (x[3] + x[5]) * half);
        }
        Kernel::Symmetric { scale } => {
            let half = T::c(0.5);
            let (ux, uy) = (x[6] - x[4], x[7] - x[5]);
            let (mx, my) = ((x[0] + x[2]) * half - x[4], (x[1] + x[3]) * half - x[5]);
            out.push((ux * my - uy * mx) * T::c(scale));
            out.push(((x[2] - x[0]) * ux + (x[3] - x[1]) * uy) * T::c(scale));
        }
        Kernel::EqRadiusCC => out.push(x[0] - x[1]),
        Kernel::EqRadiusCA => {
            let (dx, dy) = (x[3] - x[1], x[4] - x[2]);
            out.push(x[0] - (dx * dx + dy * dy + T::c(1e-18)).sqrt());
        }
        Kernel::EqRadiusAA => {
            let (ax, ay) = (x[2] - x[0], x[3] - x[1]);
            let (bx, by) = (x[6] - x[4], x[7] - x[5]);
            out.push(
                (ax * ax + ay * ay + T::c(1e-18)).sqrt() - (bx * bx + by * by + T::c(1e-18)).sqrt(),
            );
        }
        Kernel::OnCircle => {
            let (dx, dy) = (x[0] - x[2], x[1] - x[3]);
            out.push((dx * dx + dy * dy + T::c(1e-18)).sqrt() - x[4]);
        }
        Kernel::OnArc => {
            let (dx, dy) = (x[0] - x[2], x[1] - x[3]);
            let (sx, sy) = (x[4] - x[2], x[5] - x[3]);
            out.push(
                (dx * dx + dy * dy + T::c(1e-18)).sqrt() - (sx * sx + sy * sy + T::c(1e-18)).sqrt(),
            );
        }
        Kernel::Angle { cos, sin, scale } => {
            let (ux, uy) = (x[2] - x[0], x[3] - x[1]);
            let (vx, vy) = (x[6] - x[4], x[7] - x[5]);
            let cross = ux * vy - uy * vx;
            let dot = ux * vx + uy * vy;
            out.push((cross * T::c(cos) - dot * T::c(sin)) * T::c(scale));
        }
        Kernel::SymXAxis => {
            out.push(x[0] - x[2]);
            out.push(x[1] + x[3]);
        }
        Kernel::SymYAxis => {
            out.push(x[1] - x[3]);
            out.push(x[0] + x[2]);
        }
        Kernel::TanCC { k1, k2 } => {
            let (dx, dy) = (x[2] - x[0], x[3] - x[1]);
            let d = (dx * dx + dy * dy + T::c(1e-18)).sqrt();
            out.push(d - (x[4] * T::c(k1) + x[5] * T::c(k2)));
        }
        Kernel::TanCA { k1, k2 } => {
            let (dx, dy) = (x[2] - x[0], x[3] - x[1]);
            let d = (dx * dx + dy * dy + T::c(1e-18)).sqrt();
            let (sx, sy) = (x[5] - x[2], x[6] - x[3]);
            let r2 = (sx * sx + sy * sy + T::c(1e-18)).sqrt();
            out.push(d - (x[4] * T::c(k1) + r2 * T::c(k2)));
        }
        Kernel::TanAA { k1, k2 } => {
            let (dx, dy) = (x[4] - x[0], x[5] - x[1]);
            let d = (dx * dx + dy * dy + T::c(1e-18)).sqrt();
            let (ax, ay) = (x[2] - x[0], x[3] - x[1]);
            let (bx, by) = (x[6] - x[4], x[7] - x[5]);
            let r1 = (ax * ax + ay * ay + T::c(1e-18)).sqrt();
            let r2 = (bx * bx + by * by + T::c(1e-18)).sqrt();
            out.push(d - (r1 * T::c(k1) + r2 * T::c(k2)));
        }
        Kernel::ArcLen { value, shift } => {
            let (sx, sy) = (x[2] - x[0], x[3] - x[1]);
            let (ex, ey) = (x[4] - x[0], x[5] - x[1]);
            let r = (sx * sx + sy * sy + T::c(1e-18)).sqrt();
            let (cross, dot) = (sx * ey - sy * ex, sx * ex + sy * ey);
            let (cs, sn) = (T::c(det::cos(shift)), T::c(det::sin(shift)));
            // The angle from start to end, minus `shift`, wrapped into (-pi, pi], plus `shift` back.
            let psi = (cross * cs - dot * sn).atan2(dot * cs + cross * sn);
            out.push(r * (psi + T::c(shift)) - T::c(value));
        }
        Kernel::OnXAxis => out.push(x[1]),
        Kernel::OnYAxis => out.push(x[0]),
        Kernel::PointOnLine { scale } => {
            let (ux, uy) = (x[4] - x[2], x[5] - x[3]);
            let (wx, wy) = (x[0] - x[2], x[1] - x[3]);
            out.push((ux * wy - uy * wx) * T::c(scale));
        }
        Kernel::TangentCircle { scale } => {
            let (ux, uy) = (x[2] - x[0], x[3] - x[1]);
            let (wx, wy) = (x[4] - x[0], x[5] - x[1]);
            let cross = ux * wy - uy * wx;
            let l2 = ux * ux + uy * uy + T::c(1e-18);
            out.push((cross * cross / l2 - x[6] * x[6]) * T::c(scale));
        }
        Kernel::TangentArc { scale } => {
            let (ux, uy) = (x[2] - x[0], x[3] - x[1]);
            let (wx, wy) = (x[4] - x[0], x[5] - x[1]);
            let cross = ux * wy - uy * wx;
            let l2 = ux * ux + uy * uy + T::c(1e-18);
            let (rx, ry) = (x[6] - x[4], x[7] - x[5]);
            out.push((cross * cross / l2 - (rx * rx + ry * ry)) * T::c(scale));
        }
        Kernel::ArcConsistency { scale } => {
            let (sx, sy) = (x[2] - x[0], x[3] - x[1]);
            let (ex, ey) = (x[4] - x[0], x[5] - x[1]);
            out.push((sx * sx + sy * sy - ex * ex - ey * ey) * T::c(scale));
        }
    }
}

struct Equation {
    /// Constraint ID, or the arc's ID for its built-in consistency equation.
    owner: String,
    vars: Vec<usize>,
    kernel: Kernel,
}

// ---------------------------------------------------------------------------------------------
// Building the system from a sketch
// ---------------------------------------------------------------------------------------------

struct System {
    x: Vec<f64>,
    eqs: Vec<Equation>,
    /// Point ID -> index of its x variable (y is next).
    point_var: BTreeMap<String, usize>,
    /// Circle ID -> index of its radius variable.
    radius_var: BTreeMap<String, usize>,
}

impl System {
    fn build(s: &Sketch, drag: &[Hold], blame_last: Option<&str>) -> System {
        let mut x = Vec::new();
        let mut point_var = BTreeMap::new();
        let mut radius_var = BTreeMap::new();
        for (id, e) in &s.entities {
            match e {
                Entity::Point { x: px, y: py } => {
                    point_var.insert(id.clone(), x.len());
                    x.push(*px);
                    x.push(*py);
                }
                Entity::Circle { radius, .. } => {
                    radius_var.insert(id.clone(), x.len());
                    x.push(*radius);
                }
                _ => {}
            }
        }
        let mut sys = System {
            x,
            eqs: Vec::new(),
            point_var,
            radius_var,
        };

        // Structural equations first, so user constraints (not these) are blamed for dependence.
        for (id, e) in &s.entities {
            if let Entity::Arc { center, start, end } = e {
                let r = sys.dist(center, start);
                let vars = sys.pts(&[center, start, end]);
                let scale = 1.0 / (2.0 * r.max(1e-3));
                sys.push(id, vars, Kernel::ArcConsistency { scale });
            }
        }
        // Blame goes to whatever is analysed last, so the caller can put the constraint it just
        // added or edited at the end.
        // Reference (driven) dimensions measure; they do not constrain.
        for (id, c) in s
            .constraints
            .iter()
            .filter(|(id, _)| Some(id.as_str()) != blame_last && !s.driven.contains(*id))
        {
            sys.add_constraint(s, id, c);
        }
        if let Some((id, c)) = blame_last
            .filter(|b| !s.driven.contains(*b))
            .and_then(|b| s.constraints.get_key_value(b))
        {
            sys.add_constraint(s, id, c);
        }
        // The held point goes last so user constraints are never blamed for it.
        for hold in drag {
            match *hold {
                Hold::Point(id, x, y) => {
                    let i = sys.point_var[id];
                    sys.push(DRAG_OWNER, vec![i, i + 1], Kernel::Fixed { x, y });
                }
                Hold::Radius(id, value) => {
                    let v = vec![sys.radius_var[id]];
                    sys.push(DRAG_OWNER, v, Kernel::CircleRadius { value });
                }
            }
        }
        sys
    }

    fn pos(&self, id: &str) -> (f64, f64) {
        let i = self.point_var[id];
        (self.x[i], self.x[i + 1])
    }

    fn dist(&self, a: &str, b: &str) -> f64 {
        let (ax, ay) = self.pos(a);
        let (bx, by) = self.pos(b);
        det::sqrt((bx - ax) * (bx - ax) + (by - ay) * (by - ay))
    }

    fn pts(&self, ids: &[&String]) -> Vec<usize> {
        ids.iter()
            .flat_map(|id| {
                let i = self.point_var[id.as_str()];
                [i, i + 1]
            })
            .collect()
    }

    fn push(&mut self, owner: &str, vars: Vec<usize>, kernel: Kernel) {
        self.eqs.push(Equation {
            owner: owner.to_owned(),
            vars,
            kernel,
        });
    }

    fn curve_centre(&self, s: &Sketch, id: &str) -> String {
        match &s.entities[id] {
            Entity::Circle { center, .. } | Entity::Arc { center, .. } => center.clone(),
            _ => unreachable!("validated when the constraint was added"),
        }
    }

    fn curve_radius(&self, s: &Sketch, id: &str) -> f64 {
        match &s.entities[id] {
            Entity::Circle { .. } => self.x[self.radius_var[id]],
            Entity::Arc { center, start, .. } => self.dist(center, start),
            _ => unreachable!("validated when the constraint was added"),
        }
    }

    fn line_ends<'a>(s: &'a Sketch, id: &str) -> (&'a String, &'a String) {
        match &s.entities[id] {
            Entity::Line { a, b } => (a, b),
            _ => unreachable!("validated when the constraint was added"),
        }
    }

    fn add_constraint(&mut self, s: &Sketch, id: &str, c: &Constraint) {
        match c {
            Constraint::Coincident { a, b } => {
                let v = self.pts(&[a, b]);
                self.push(id, v, Kernel::Coincident);
            }
            Constraint::Horizontal { line } => {
                let (a, b) = Self::line_ends(s, line);
                let v = vec![
                    self.point_var[a.as_str()] + 1,
                    self.point_var[b.as_str()] + 1,
                ];
                self.push(id, v, Kernel::Horizontal);
            }
            Constraint::Vertical { line } => {
                let (a, b) = Self::line_ends(s, line);
                let v = vec![self.point_var[a.as_str()], self.point_var[b.as_str()]];
                self.push(id, v, Kernel::Vertical);
            }
            Constraint::Parallel { a, b }
            | Constraint::Perpendicular { a, b }
            | Constraint::EqualLength { a, b } => {
                let (p1, p2) = Self::line_ends(s, a);
                let (p3, p4) = Self::line_ends(s, b);
                let (l1, l2) = (self.dist(p1, p2), self.dist(p3, p4));
                let v = self.pts(&[p1, p2, p3, p4]);
                let kernel = match c {
                    Constraint::Parallel { .. } => Kernel::Parallel {
                        scale: 1.0 / (l1 * l2).max(1e-6),
                    },
                    Constraint::Perpendicular { .. } => Kernel::Perpendicular {
                        scale: 1.0 / (l1 * l2).max(1e-6),
                    },
                    _ => Kernel::EqualLength {
                        scale: 1.0 / (l1 + l2).max(1e-3),
                    },
                };
                self.push(id, v, kernel);
            }
            Constraint::Length { line, value } => {
                let (a, b) = Self::line_ends(s, line);
                let v = self.pts(&[a, b]);
                self.push(id, v, Kernel::Distance { value: *value });
            }
            Constraint::Distance { a, b, value } => {
                let v = self.pts(&[a, b]);
                self.push(id, v, Kernel::Distance { value: *value });
            }
            Constraint::HorizontalDistance { a, b, value } => {
                let v = self.pts(&[a, b]);
                self.push(id, v, Kernel::HDist { value: *value });
            }
            Constraint::VerticalDistance { a, b, value } => {
                let v = self.pts(&[a, b]);
                self.push(id, v, Kernel::VDist { value: *value });
            }
            Constraint::PointLineDistance { point, line, value } => {
                let (a, b) = Self::line_ends(s, line);
                let v = self.pts(&[point, a, b]);
                self.push(id, v, Kernel::PointLine { value: *value });
            }
            Constraint::Radius { target, value } => match &s.entities[target.as_str()] {
                Entity::Circle { .. } => {
                    let v = vec![self.radius_var[target.as_str()]];
                    self.push(id, v, Kernel::CircleRadius { value: *value });
                }
                Entity::Arc { center, start, .. } => {
                    let v = self.pts(&[center, start]);
                    self.push(id, v, Kernel::Distance { value: *value });
                }
                _ => unreachable!("validated when the constraint was added"),
            },
            Constraint::Fixed { point, x, y } => {
                let v = self.pts(&[point]);
                self.push(id, v, Kernel::Fixed { x: *x, y: *y });
            }
            Constraint::OnAxis { point, axis } => {
                let v = self.pts(&[point]);
                let k = match axis {
                    crate::SketchAxis::X => Kernel::OnXAxis,
                    crate::SketchAxis::Y => Kernel::OnYAxis,
                };
                self.push(id, v, k);
            }
            Constraint::PointOnLine { point, line } => {
                let (a, b) = Self::line_ends(s, line);
                let scale = 1.0 / self.dist(a, b).max(1e-3);
                let v = self.pts(&[point, a, b]);
                self.push(id, v, Kernel::PointOnLine { scale });
            }
            Constraint::ArcLength { arc, value } => {
                let Entity::Arc { center, start, end } = &s.entities[arc.as_str()] else {
                    unreachable!("validated when the constraint was added")
                };
                // Which way round the sweep goes now: the angle from start to end, made
                // counter-clockwise by a full turn when it came out negative.
                let (c, st, en) = (self.pos(center), self.pos(start), self.pos(end));
                let (sx, sy) = (st.0 - c.0, st.1 - c.1);
                let (ex, ey) = (en.0 - c.0, en.1 - c.1);
                let tau = core::f64::consts::TAU;
                let now = {
                    let a = det::atan2(sx * ey - sy * ex, sx * ex + sy * ey);
                    if a < 0.0 {
                        a + tau
                    } else {
                        a
                    }
                };
                let radius = det::hypot(sx, sy).max(1e-9);
                let aim = (*value / radius).clamp(1e-6, tau - 1e-6);
                let v = self.pts(&[center, start, end]);
                self.push(
                    id,
                    v,
                    Kernel::ArcLen {
                        value: *value,
                        shift: (now + aim) / 2.0,
                    },
                );
            }
            Constraint::Angle { a, b, value } => {
                let (a1, a2) = Self::line_ends(s, a);
                let (b1, b2) = Self::line_ends(s, b);
                let (lu, lv) = (self.dist(a1, a2).max(1e-3), self.dist(b1, b2).max(1e-3));
                let theta = value * core::f64::consts::PI / 180.0;
                let v = self.pts(&[a1, a2, b1, b2]);
                self.push(
                    id,
                    v,
                    Kernel::Angle {
                        cos: det::cos(theta),
                        sin: det::sin(theta),
                        scale: (lu + lv) / (2.0 * lu * lv),
                    },
                );
            }
            Constraint::SymmetricAxis { a, b, axis } => {
                let v = self.pts(&[a, b]);
                let k = match axis {
                    crate::SketchAxis::X => Kernel::SymXAxis,
                    crate::SketchAxis::Y => Kernel::SymYAxis,
                };
                self.push(id, v, k);
            }
            Constraint::TangentCurves { a, b } => {
                // Outside each other (centres a radius sum apart) or one inside the other (a radius
                // difference apart): whichever the shapes are closest to now.
                let (ca, ra, cb, rb) = (
                    self.curve_centre(s, a),
                    self.curve_radius(s, a),
                    self.curve_centre(s, b),
                    self.curve_radius(s, b),
                );
                let d = self.dist(&ca, &cb);
                let (k1, k2) = if (d - (ra + rb)).abs() <= (d - (ra - rb).abs()).abs() {
                    (1.0, 1.0)
                } else if ra >= rb {
                    (1.0, -1.0)
                } else {
                    (-1.0, 1.0)
                };
                let mut v: Vec<usize> = Vec::new();
                let kernel = match (&s.entities[a.as_str()], &s.entities[b.as_str()]) {
                    (Entity::Circle { .. }, Entity::Circle { .. }) => {
                        v.extend(self.pts(&[&ca, &cb]));
                        v.push(self.radius_var[a.as_str()]);
                        v.push(self.radius_var[b.as_str()]);
                        Kernel::TanCC { k1, k2 }
                    }
                    (Entity::Circle { .. }, Entity::Arc { start, .. }) => {
                        v.extend(self.pts(&[&ca, &cb]));
                        v.push(self.radius_var[a.as_str()]);
                        v.extend(self.pts(&[start]));
                        Kernel::TanCA { k1, k2 }
                    }
                    (Entity::Arc { .. }, Entity::Circle { .. }) => {
                        // Put the circle first.
                        v.extend(self.pts(&[&cb, &ca]));
                        v.push(self.radius_var[b.as_str()]);
                        let Entity::Arc { start, .. } = &s.entities[a.as_str()] else {
                            unreachable!()
                        };
                        v.extend(self.pts(&[start]));
                        Kernel::TanCA { k1: k2, k2: k1 }
                    }
                    (Entity::Arc { start: sa, .. }, Entity::Arc { start: sb, .. }) => {
                        v.extend(self.pts(&[&ca, sa, &cb, sb]));
                        Kernel::TanAA { k1, k2 }
                    }
                    _ => unreachable!("validated when the constraint was added"),
                };
                self.push(id, v, kernel);
            }
            Constraint::Midpoint { point, line } => {
                let (a, b) = Self::line_ends(s, line);
                let v = self.pts(&[point, a, b]);
                self.push(id, v, Kernel::Midpoint);
            }
            Constraint::Symmetric { a, b, line } => {
                let (la, lb) = Self::line_ends(s, line);
                let scale = 1.0 / self.dist(la, lb).max(1e-3);
                let v = self.pts(&[a, b, la, lb]);
                self.push(id, v, Kernel::Symmetric { scale });
            }
            Constraint::EqualRadius { a, b } => {
                let (ea, eb) = (&s.entities[a.as_str()], &s.entities[b.as_str()]);
                match (ea, eb) {
                    (Entity::Circle { .. }, Entity::Circle { .. }) => {
                        let v = vec![self.radius_var[a.as_str()], self.radius_var[b.as_str()]];
                        self.push(id, v, Kernel::EqRadiusCC);
                    }
                    (Entity::Circle { .. }, Entity::Arc { center, start, .. })
                    | (Entity::Arc { center, start, .. }, Entity::Circle { .. }) => {
                        let circle = if matches!(ea, Entity::Circle { .. }) {
                            a
                        } else {
                            b
                        };
                        let mut v = vec![self.radius_var[circle.as_str()]];
                        v.extend(self.pts(&[center, start]));
                        self.push(id, v, Kernel::EqRadiusCA);
                    }
                    (
                        Entity::Arc {
                            center: c1,
                            start: s1,
                            ..
                        },
                        Entity::Arc {
                            center: c2,
                            start: s2,
                            ..
                        },
                    ) => {
                        let v = self.pts(&[c1, s1, c2, s2]);
                        self.push(id, v, Kernel::EqRadiusAA);
                    }
                    _ => unreachable!("validated when the constraint was added"),
                }
            }
            Constraint::PointOnCurve { point, curve } => match &s.entities[curve.as_str()] {
                Entity::Circle { center, .. } => {
                    let mut v = self.pts(&[point, center]);
                    v.push(self.radius_var[curve.as_str()]);
                    self.push(id, v, Kernel::OnCircle);
                }
                Entity::Arc { center, start, .. } => {
                    let v = self.pts(&[point, center, start]);
                    self.push(id, v, Kernel::OnArc);
                }
                _ => unreachable!("validated when the constraint was added"),
            },
            Constraint::Tangent { line, curve } => {
                let (a, b) = Self::line_ends(s, line);
                match &s.entities[curve.as_str()] {
                    Entity::Circle { center, .. } => {
                        let r = self.x[self.radius_var[curve.as_str()]];
                        let mut v = self.pts(&[a, b, center]);
                        v.push(self.radius_var[curve.as_str()]);
                        self.push(
                            id,
                            v,
                            Kernel::TangentCircle {
                                scale: 1.0 / (2.0 * r.max(1e-3)),
                            },
                        );
                    }
                    Entity::Arc { center, start, .. } => {
                        let r = self.dist(center, start);
                        let v = self.pts(&[a, b, center, start]);
                        self.push(
                            id,
                            v,
                            Kernel::TangentArc {
                                scale: 1.0 / (2.0 * r.max(1e-3)),
                            },
                        );
                    }
                    _ => unreachable!("validated when the constraint was added"),
                }
            }
        }
    }

    fn rows(&self) -> usize {
        self.eqs.iter().map(|e| e.kernel.arity()).sum()
    }

    fn residuals(&self, x: &[f64]) -> Vec<f64> {
        let mut r = Vec::with_capacity(self.rows());
        let mut local = Vec::with_capacity(MAXV);
        for e in &self.eqs {
            local.clear();
            local.extend(e.vars.iter().map(|&i| x[i]));
            eval(e.kernel, &local, &mut r);
        }
        r
    }

    /// Residuals and dense row-major Jacobian (rows x n).
    fn residuals_and_jacobian(&self, x: &[f64]) -> (Vec<f64>, Vec<f64>) {
        let n = x.len();
        let m = self.rows();
        let mut r = Vec::with_capacity(m);
        let mut j = vec![0.0; m * n];
        let mut local: Vec<Dual> = Vec::with_capacity(MAXV);
        let mut out: Vec<Dual> = Vec::with_capacity(2);
        let mut row = 0;
        for e in &self.eqs {
            local.clear();
            local.extend(e.vars.iter().enumerate().map(|(k, &i)| Dual::var(x[i], k)));
            out.clear();
            eval(e.kernel, &local, &mut out);
            for d in &out {
                r.push(d.v);
                for (k, &i) in e.vars.iter().enumerate() {
                    // A variable can appear twice in one equation; contributions add.
                    j[row * n + i] += d.g[k];
                }
                row += 1;
            }
        }
        (r, j)
    }
}

// ---------------------------------------------------------------------------------------------
// Linear algebra (dense, small, deterministic)
// ---------------------------------------------------------------------------------------------

fn max_abs(v: &[f64]) -> f64 {
    v.iter()
        .fold(0.0, |m, x| if x.abs() > m { x.abs() } else { m })
}

/// Solves A x = b for symmetric positive-definite A (row-major n x n). Overwrites `a` and `b`
/// (result in `b`). Returns false if A is not positive definite.
fn cholesky_solve(a: &mut [f64], n: usize, b: &mut [f64]) -> bool {
    for i in 0..n {
        for j in 0..=i {
            let mut sum = a[i * n + j];
            for k in 0..j {
                sum -= a[i * n + k] * a[j * n + k];
            }
            if i == j {
                if sum <= 0.0 || !sum.is_finite() {
                    return false;
                }
                a[i * n + i] = det::sqrt(sum);
            } else {
                a[i * n + j] = sum / a[j * n + j];
            }
        }
    }
    for i in 0..n {
        let mut sum = b[i];
        for k in 0..i {
            sum -= a[i * n + k] * b[k];
        }
        b[i] = sum / a[i * n + i];
    }
    for i in (0..n).rev() {
        let mut sum = b[i];
        for k in i + 1..n {
            sum -= a[k * n + i] * b[k];
        }
        b[i] = sum / a[i * n + i];
    }
    true
}

struct Rref {
    n: usize,
    mat: Vec<f64>,
    /// (row, column) of each pivot.
    pivots: Vec<(usize, usize)>,
}

/// Gauss–Jordan elimination with partial pivoting and a scale-relative tolerance.
fn rref(mut a: Vec<f64>, m: usize, n: usize) -> Rref {
    let tol = 1e-9 * max_abs(&a).max(1.0);
    let mut pivots = Vec::new();
    let mut row = 0;
    for col in 0..n {
        if row >= m {
            break;
        }
        let mut p = row;
        for r in row + 1..m {
            if a[r * n + col].abs() > a[p * n + col].abs() {
                p = r;
            }
        }
        if a[p * n + col].abs() <= tol {
            continue;
        }
        if p != row {
            for c in 0..n {
                a.swap(row * n + c, p * n + c);
            }
        }
        let pv = a[row * n + col];
        for c in 0..n {
            a[row * n + c] /= pv;
        }
        for r in 0..m {
            if r != row {
                let f = a[r * n + col];
                if f != 0.0 {
                    for c in 0..n {
                        a[r * n + c] -= f * a[row * n + c];
                    }
                }
            }
        }
        pivots.push((row, col));
        row += 1;
    }
    Rref { n, mat: a, pivots }
}

impl Rref {
    /// The directions the variables can still move in (the null space of the constraint
    /// Jacobian): an orthonormal basis of the coupled ones, in a fixed order, plus the variables
    /// no constraint touches at all, which are free on their own and need no vector (this keeps
    /// a sketch of many unconstrained points cheap).
    fn null_basis(&self) -> (Vec<Vec<f64>>, Vec<bool>) {
        let n = self.n;
        let mut is_pivot = vec![false; n];
        for &(_, c) in &self.pivots {
            is_pivot[c] = true;
        }
        let mut alone = vec![false; n];
        let mut basis: Vec<Vec<f64>> = Vec::new();
        for f in (0..n).filter(|f| !is_pivot[*f]) {
            let coupled = self
                .pivots
                .iter()
                .any(|&(r, _)| self.mat[r * n + f].abs() > 1e-12);
            if !coupled {
                alone[f] = true;
                continue;
            }
            let mut v = vec![0.0; n];
            v[f] = 1.0;
            for &(r, c) in &self.pivots {
                v[c] = -self.mat[r * n + f];
            }
            for _pass in 0..2 {
                for b in &basis {
                    let dot: f64 = v.iter().zip(b).map(|(a, c)| a * c).sum();
                    for (a, c) in v.iter_mut().zip(b) {
                        *a -= dot * c;
                    }
                }
            }
            let norm = det::sqrt(v.iter().map(|a| a * a).sum::<f64>());
            if norm > 1e-9 {
                for a in &mut v {
                    *a /= norm;
                }
                basis.push(v);
            }
        }
        (basis, alone)
    }

    /// For each variable: does the solution set still allow it to move?
    fn free_vars(&self) -> Vec<bool> {
        let mut is_pivot = vec![false; self.n];
        for &(_, c) in &self.pivots {
            is_pivot[c] = true;
        }
        let mut free = vec![false; self.n];
        for c in 0..self.n {
            if !is_pivot[c] {
                free[c] = true;
            }
        }
        for &(r, c) in &self.pivots {
            for f in 0..self.n {
                if !is_pivot[f] && self.mat[r * self.n + f].abs() > 1e-7 {
                    free[c] = true;
                    break;
                }
            }
        }
        free
    }
}

// ---------------------------------------------------------------------------------------------
// Levenberg–Marquardt
// ---------------------------------------------------------------------------------------------

struct LmResult {
    iterations: u32,
    status: SolveStatus,
}

/// From this many variables the Newton step is solved with the sparse factorisation. Below it
/// the dense one is as fast and, importantly, unchanged: the same arithmetic as before, so the
/// coordinates of documents saved earlier (and the state hashes they carry) stay bit-identical.
const SPARSE_FROM: usize = 160;

/// Systems with more equations than this may skip the dependence check when the row reduction
/// already shows every equation independent (see `solve_cluster`).
const GS_SKIP_FROM: usize = 100;

fn levenberg_marquardt(sys: &System, x: &mut [f64]) -> LmResult {
    let sparse = x.len() >= SPARSE_FROM;
    lm(sys, x, sparse)
}

/// The sparsity of the system: the layout for the factorisation, and for each Jacobian row the
/// variables it can be non-zero in.
fn sparse_layout(sys: &System, n: usize) -> (Profile, Vec<Vec<usize>>) {
    let mut links = Vec::new();
    let mut row_vars: Vec<Vec<usize>> = Vec::new();
    for e in &sys.eqs {
        let mut vars = e.vars.clone();
        vars.sort_unstable();
        vars.dedup();
        for (k, &a) in vars.iter().enumerate() {
            for &b in &vars[..k] {
                links.push((a, b));
            }
        }
        for _ in 0..e.kernel.arity() {
            row_vars.push(vars.clone());
        }
    }
    (Profile::new(n, &links), row_vars)
}

fn lm(sys: &System, x: &mut [f64], sparse: bool) -> LmResult {
    let n = x.len();
    let (mut r, mut j) = sys.residuals_and_jacobian(x);
    let m = r.len();
    if m == 0 || n == 0 {
        return LmResult {
            iterations: 0,
            status: SolveStatus::Solved,
        };
    }
    let mut cost = 0.5 * r.iter().map(|v| v * v).sum::<f64>();
    let mut lambda = 1e-6;
    let layout = sparse.then(|| sparse_layout(sys, n));

    let mut polish = 0;
    for iter in 0..MAX_ITERATIONS {
        let res = max_abs(&r);
        if res <= POLISH_TOL {
            return LmResult {
                iterations: iter,
                status: SolveStatus::Solved,
            };
        }
        if res <= SOLVED_TOL {
            polish += 1;
            if polish > MAX_POLISH {
                return LmResult {
                    iterations: iter,
                    status: SolveStatus::Solved,
                };
            }
        }
        let mut g = vec![0.0; n];
        for row in 0..m {
            for c in 0..n {
                g[c] += j[row * n + c] * r[row];
            }
        }
        if max_abs(&g) <= 1e-13 {
            // Stationary point of the least-squares cost. If residuals are negligible we are done
            // (nothing left to polish); otherwise the constraints contradict each other.
            return LmResult {
                iterations: iter,
                status: if res <= SOLVED_TOL {
                    SolveStatus::Solved
                } else {
                    SolveStatus::Inconsistent
                },
            };
        }
        // JᵀJ: dense for small systems, in profile storage for large ones.
        let mut jtj = Vec::new();
        let mut jtj_sparse = Vec::new();
        if let Some((prof, row_vars)) = &layout {
            jtj_sparse = prof.zeros();
            for (row, cols) in row_vars.iter().enumerate() {
                for (k, &a) in cols.iter().enumerate() {
                    let ja = j[row * n + a];
                    if ja != 0.0 {
                        for &b in &cols[..=k] {
                            prof.add(&mut jtj_sparse, a, b, ja * j[row * n + b]);
                        }
                    }
                }
            }
        } else {
            jtj = vec![0.0; n * n];
            for row in 0..m {
                for a in 0..n {
                    let ja = j[row * n + a];
                    if ja != 0.0 {
                        for b in 0..=a {
                            jtj[a * n + b] += ja * j[row * n + b];
                        }
                    }
                }
            }
            for a in 0..n {
                for b in a + 1..n {
                    jtj[a * n + b] = jtj[b * n + a];
                }
            }
        }

        loop {
            // Ridge (identity) damping: as lambda -> 0 this is the minimum-norm Newton step, which
            // satisfies the linearised constraints while moving the geometry as little as possible.
            let mut step: Vec<f64> = g.iter().map(|v| -v).collect();
            let factored = if let Some((prof, _)) = &layout {
                let mut vals = jtj_sparse.clone();
                prof.add_diagonal(&mut vals, lambda);
                prof.solve(&mut vals, &mut step)
            } else {
                let mut a = jtj.clone();
                for i in 0..n {
                    a[i * n + i] += lambda;
                }
                cholesky_solve(&mut a, n, &mut step)
            };
            if factored {
                let x_new: Vec<f64> = x.iter().zip(&step).map(|(a, b)| a + b).collect();
                let r_new = sys.residuals(&x_new);
                let cost_new = 0.5 * r_new.iter().map(|v| v * v).sum::<f64>();
                if cost_new.is_finite() && cost_new < cost {
                    x.copy_from_slice(&x_new);
                    let (rr, jj) = sys.residuals_and_jacobian(x);
                    r = rr;
                    j = jj;
                    cost = cost_new;
                    lambda = (lambda / 3.0).max(1e-15);
                    break;
                }
            }
            lambda *= 4.0;
            if lambda > 1e15 {
                let status = if max_abs(&r) <= SOLVED_TOL {
                    SolveStatus::Solved
                } else {
                    SolveStatus::DidNotConverge
                };
                return LmResult {
                    iterations: iter,
                    status,
                };
            }
        }
    }
    let status = if max_abs(&r) <= SOLVED_TOL {
        SolveStatus::Solved
    } else {
        SolveStatus::DidNotConverge
    };
    LmResult {
        iterations: MAX_ITERATIONS,
        status,
    }
}

// ---------------------------------------------------------------------------------------------
// Public API
// ---------------------------------------------------------------------------------------------

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum SolveStatus {
    /// All constraints satisfied.
    Solved,
    /// The constraints contradict each other (see `conflicting`).
    Inconsistent,
    /// The iteration limit was hit without a solution (e.g. a very distant starting layout).
    DidNotConverge,
}

/// How a point can still move: 0 = pinned, 1 = along `direction`, 2 = anywhere in the plane.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct PointMotion {
    pub dof: u8,
    /// Unit vector of the allowed motion when `dof` is 1 (sign normalised: x > 0, or x = 0 and y > 0).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub direction: Option<[f64; 2]>,
}

/// Result of a solve. Identical for GUI and agents (AGENTS.md §4.4): everything a human would
/// read off the DOF display is here as data.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct SolveReport {
    pub status: SolveStatus,
    pub iterations: u32,
    /// Largest remaining |residual| (length-like units).
    pub max_residual: f64,
    pub variables: usize,
    /// Rank of the constraint system.
    pub independent_equations: usize,
    /// Degrees of freedom left (includes rigid motion if nothing anchors the sketch).
    pub dof: usize,
    /// The part of `dof` that belongs to real shapes, when some of it is only construction
    /// geometry that is still free (then this is smaller than `dof`). Absent when equal.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub shape_dof: Option<usize>,
    /// Entities that can still move, sorted. Empty when fully constrained.
    pub under_constrained: Vec<String>,
    /// For every point that can still move, in which directions. Pinned points are absent.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub motion: BTreeMap<String, PointMotion>,
    /// For every line that can still move as a whole, how its middle point can move.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub shape_motion: BTreeMap<String, PointMotion>,
    /// Circles and arcs whose radius is still free.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub radius_free: Vec<String>,
    /// Arcs whose sweep angle (how far round they go) is still free.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub sweep_free: Vec<String>,
    /// Constraints that add nothing (implied by others) and are satisfied.
    pub redundant: Vec<String>,
    /// Constraints that cannot be satisfied together with earlier ones.
    pub conflicting: Vec<String>,
    /// True if the sketch geometry was updated. A failed solve leaves the sketch untouched.
    pub applied: bool,
}

impl SolveReport {
    pub fn is_fully_constrained(&self) -> bool {
        self.status == SolveStatus::Solved && self.dof == 0
    }
}

/// Writes solved variables back into the shapes.
fn store(entities: &mut BTreeMap<String, Entity>, sys: &System, x: &[f64]) {
    for (id, e) in entities {
        match e {
            Entity::Point { x: px, y: py } => {
                let i = sys.point_var[id.as_str()];
                *px = canon(x[i]);
                *py = canon(x[i + 1]);
            }
            Entity::Circle { radius, .. } => *radius = canon(x[sys.radius_var[id.as_str()]]),
            _ => {}
        }
    }
}

/// Quantum of stored coordinates, in mm (0.1 nm).
const SNAP: f64 = 1e10;

fn snap(v: f64) -> f64 {
    canon((v * SNAP).round() / SNAP)
}

fn canon(v: f64) -> f64 {
    if v == 0.0 {
        0.0
    } else {
        v
    }
}

impl Sketch {
    /// Solves the constraints starting from the current geometry (so dragging stays stable) and,
    /// if successful, updates the geometry. Always returns a full analysis.
    pub fn solve(&mut self) -> SolveReport {
        self.solve_inner(&[], None)
    }

    /// Like [`solve`](Self::solve), but if the constraints contradict each other the blame falls
    /// on `constraint` (the one just added or edited) rather than on whichever sorts last.
    pub fn solve_blaming(&mut self, constraint: &str) -> SolveReport {
        self.solve_inner(&[], Some(constraint))
    }

    /// Drags a point to `(x, y)` the way a user would with the mouse.
    ///
    /// First the point is held exactly at the cursor while everything else adapts; if that is
    /// feasible the point lands precisely there. If not (e.g. a fully dimensioned sketch), the
    /// sketch settles at the nearest valid configuration to the cursor. If even that fails, the
    /// sketch is left exactly as it was. The report describes the resulting sketch.
    pub fn drag_point(&mut self, id: &str, x: f64, y: f64) -> Result<SolveReport, SketchError> {
        let target = DragTarget {
            point: id.to_owned(),
            x,
            y,
        };
        self.drag(std::slice::from_ref(&target), None)
    }

    /// Drags several points (a line's two ends, an arc's three points) and/or a circle's radius
    /// at once, the same way: held exactly if feasible, else the nearest valid shape, else
    /// untouched. Moving all of a shape's points by the same amount moves the shape.
    pub fn drag(
        &mut self,
        points: &[DragTarget],
        radius: Option<&DragRadius>,
    ) -> Result<SolveReport, SketchError> {
        let saved = self.clone();
        for t in points {
            self.set_point(&t.point, t.x, t.y)?;
        }
        if let Some(r) = radius {
            self.set_circle_radius(&r.circle, r.value)?;
        }
        let holds: Vec<Hold> = points
            .iter()
            .map(|t| Hold::Point(t.point.as_str(), t.x, t.y))
            .chain(radius.map(|r| Hold::Radius(r.circle.as_str(), r.value)))
            .collect();
        let mut held = self.clone();
        if held.solve_inner(&holds, None).status == SolveStatus::Solved {
            // Re-analyse without the temporary hold so DOF numbers describe the real sketch.
            let report = held.solve();
            *self = held;
            return Ok(report);
        }
        let report = self.solve();
        if !report.applied {
            *self = saved;
        }
        Ok(report)
    }

    /// Like [`drag_point`](Self::drag_point), but solves only the part of the sketch that is
    /// connected to the point (through shared points, shapes and constraints), because nothing
    /// else can move. The work is proportional to that cluster, not to the whole sketch, which is
    /// what a live drag preview needs. The result for the cluster is what `drag_point` gives;
    /// everything outside it is untouched. Returns whether the sketch was updated; no analysis
    /// report is computed.
    pub fn drag_point_local(&mut self, id: &str, x: f64, y: f64) -> Result<bool, SketchError> {
        let target = DragTarget {
            point: id.to_owned(),
            x,
            y,
        };
        self.drag_local(std::slice::from_ref(&target), None)
    }

    /// [`drag`](Self::drag) restricted to the cluster the dragged things belong to.
    pub fn drag_local(
        &mut self,
        points: &[DragTarget],
        radius: Option<&DragRadius>,
    ) -> Result<bool, SketchError> {
        for t in points {
            self.expect_kind(&t.point, &[crate::model::Kind::Point])?;
        }
        if let Some(r) = radius {
            self.expect_kind(&r.circle, &[crate::model::Kind::Circle])?;
        }
        let mut cluster = BTreeSet::new();
        for id in points
            .iter()
            .map(|t| t.point.as_str())
            .chain(radius.map(|r| r.circle.as_str()))
        {
            cluster.extend(self.cluster_of(id));
        }
        let mut sub = self.clone();
        sub.entities.retain(|k, _| cluster.contains(k));
        sub.constraints
            .retain(|_, c| c.refs().iter().all(|r| cluster.contains(*r)));
        sub.construction.retain(|k| cluster.contains(k));
        let report = sub.drag(points, radius)?;
        if !report.applied {
            return Ok(false);
        }
        for (k, e) in sub.entities {
            self.entities.insert(k, e);
        }
        Ok(true)
    }

    /// The entities connected to `id` through shapes and constraints (including itself).
    fn cluster_of(&self, id: &str) -> BTreeSet<String> {
        fn link<'a>(links: &mut BTreeMap<&'a str, Vec<&'a str>>, group: Vec<&'a str>) {
            for a in &group {
                links.entry(*a).or_default().extend(group.iter().copied());
            }
        }
        let mut links: BTreeMap<&str, Vec<&str>> = BTreeMap::new();
        for (k, e) in &self.entities {
            let mut g = e.refs();
            g.push(k.as_str());
            link(&mut links, g);
        }
        for c in self.constraints.values() {
            link(&mut links, c.refs());
        }
        let mut seen: BTreeSet<String> = BTreeSet::new();
        let mut todo = vec![id];
        while let Some(n) = todo.pop() {
            if !seen.insert(n.to_owned()) {
                continue;
            }
            if let Some(next) = links.get(n) {
                todo.extend(next.iter().copied());
            }
        }
        seen
    }

    /// The groups of entities that can influence each other (through shapes and constraints).
    /// Each group is solved on its own: the equations are block diagonal, so the answer is the
    /// same, but the cost is the sum of small solves instead of one big dense one.
    fn components(&self) -> Vec<BTreeSet<String>> {
        let index: BTreeMap<&str, usize> = self
            .entities
            .keys()
            .enumerate()
            .map(|(i, k)| (k.as_str(), i))
            .collect();
        let mut parent: Vec<usize> = (0..index.len()).collect();
        fn find(parent: &mut [usize], mut x: usize) -> usize {
            while parent[x] != x {
                parent[x] = parent[parent[x]];
                x = parent[x];
            }
            x
        }
        let join = |parent: &mut Vec<usize>, a: usize, b: usize| {
            let (ra, rb) = (find(parent, a), find(parent, b));
            if ra != rb {
                // The smaller index becomes the root, so the result does not depend on order.
                parent[ra.max(rb)] = ra.min(rb);
            }
        };
        for (k, e) in &self.entities {
            for r in e.refs() {
                if let Some(&j) = index.get(r) {
                    join(&mut parent, index[k.as_str()], j);
                }
            }
        }
        for c in self.constraints.values() {
            let refs: Vec<usize> = c
                .refs()
                .into_iter()
                .filter_map(|r| index.get(r).copied())
                .collect();
            for w in refs.windows(2) {
                join(&mut parent, w[0], w[1]);
            }
        }
        let mut groups: BTreeMap<usize, BTreeSet<String>> = BTreeMap::new();
        for (k, &i) in &index {
            let root = find(&mut parent, i);
            groups.entry(root).or_default().insert((*k).to_owned());
        }
        groups.into_values().collect()
    }

    /// A sketch holding only `ids` and the constraints in `constraint_ids` (what the solver reads).
    fn restricted_to(&self, ids: &BTreeSet<String>, constraint_ids: &[&String]) -> Sketch {
        Sketch {
            plane: self.plane.clone(),
            entities: ids
                .iter()
                .filter_map(|k| self.entities.get(k).map(|e| (k.clone(), e.clone())))
                .collect(),
            constraints: constraint_ids
                .iter()
                .filter_map(|k| self.constraints.get(*k).map(|c| ((*k).clone(), c.clone())))
                .collect(),
            construction: self
                .construction
                .iter()
                .filter(|k| ids.contains(*k))
                .cloned()
                .collect(),
            infinite: self
                .infinite
                .iter()
                .filter(|k| ids.contains(*k))
                .cloned()
                .collect(),
            placements: BTreeMap::new(),
            driven: constraint_ids
                .iter()
                .filter(|k| self.driven.contains(**k))
                .map(|k| (*k).clone())
                .collect(),
            styles: BTreeMap::new(),
        }
    }

    fn solve_inner(&mut self, drag: &[Hold], blame_last: Option<&str>) -> SolveReport {
        let groups = self.components();
        if groups.len() <= 1 {
            return self.solve_cluster(drag, blame_last);
        }
        // Which group each constraint belongs to (all its references are in one group).
        let group_of: BTreeMap<&str, usize> = groups
            .iter()
            .enumerate()
            .flat_map(|(i, g)| g.iter().map(move |k| (k.as_str(), i)))
            .collect();
        let mut by_group: Vec<Vec<&String>> = vec![Vec::new(); groups.len()];
        for (id, c) in &self.constraints {
            if let Some(i) = c.refs().iter().find_map(|r| group_of.get(r)) {
                by_group[*i].push(id);
            }
        }
        let mut subs: Vec<Sketch> = Vec::with_capacity(groups.len());
        let mut reports: Vec<SolveReport> = Vec::with_capacity(groups.len());
        for (gi, g) in groups.iter().enumerate() {
            let mut sub = self.restricted_to(g, &by_group[gi]);
            let holds: Vec<Hold> = drag
                .iter()
                .filter(|h| match h {
                    Hold::Point(id, ..) | Hold::Radius(id, ..) => g.contains(*id),
                })
                .cloned()
                .collect();
            let blame = blame_last.filter(|b| sub.constraints.contains_key(*b));
            reports.push(sub.solve_cluster(&holds, blame));
            subs.push(sub);
        }
        let status = if reports
            .iter()
            .any(|r| r.status == SolveStatus::Inconsistent)
        {
            SolveStatus::Inconsistent
        } else if reports
            .iter()
            .any(|r| r.status == SolveStatus::DidNotConverge)
        {
            SolveStatus::DidNotConverge
        } else {
            SolveStatus::Solved
        };
        let applied = status == SolveStatus::Solved && reports.iter().all(|r| r.applied);
        if applied {
            for sub in subs {
                for (k, e) in sub.entities {
                    if matches!(e, Entity::Point { .. } | Entity::Circle { .. }) {
                        self.entities.insert(k, e);
                    }
                }
            }
        }
        let mut out = SolveReport {
            status,
            iterations: 0,
            max_residual: 0.0,
            variables: 0,
            independent_equations: 0,
            dof: 0,
            shape_dof: None,
            under_constrained: Vec::new(),
            motion: BTreeMap::new(),
            shape_motion: BTreeMap::new(),
            radius_free: Vec::new(),
            sweep_free: Vec::new(),
            redundant: Vec::new(),
            conflicting: Vec::new(),
            applied,
        };
        let mut shape_total = 0;
        for r in reports {
            out.iterations = out.iterations.max(r.iterations);
            out.max_residual = out.max_residual.max(r.max_residual);
            out.variables += r.variables;
            out.independent_equations += r.independent_equations;
            out.dof += r.dof;
            shape_total += r.shape_dof.unwrap_or(r.dof);
            out.under_constrained.extend(r.under_constrained);
            out.motion.extend(r.motion);
            out.shape_motion.extend(r.shape_motion);
            out.radius_free.extend(r.radius_free);
            out.sweep_free.extend(r.sweep_free);
            out.redundant.extend(r.redundant);
            out.conflicting.extend(r.conflicting);
        }
        out.shape_dof = (shape_total != out.dof).then_some(shape_total);
        out.under_constrained.sort();
        out.radius_free.sort();
        out.sweep_free.sort();
        out.redundant.sort();
        out.conflicting.sort();
        out
    }

    fn solve_cluster(&mut self, drag: &[Hold], blame_last: Option<&str>) -> SolveReport {
        let mut sys = System::build(self, drag, blame_last);
        let mut x = sys.x.clone();
        let n = x.len();
        let x0 = x.clone();
        let mut bad_arc_length: Vec<String> = Vec::new();
        let mut lm = levenberg_marquardt(&sys, &mut x);
        // Some equations are normalised by the size of the shapes when the system is built
        // (a direction relation divides by a length, say). If the shapes changed a lot while
        // solving, those factors are stale: build again at the answer and keep going until the
        // answer is also an answer for the system built from itself. Then solving it again
        // starts, and stays, solved.
        for _ in 0..4 {
            if lm.status != SolveStatus::Solved {
                break;
            }
            for &i in sys.radius_var.values() {
                if x[i] < 0.0 {
                    x[i] = -x[i];
                }
            }
            let mut moved = self.clone();
            store(&mut moved.entities, &sys, &x);
            let sys2 = System::build(&moved, drag, blame_last);
            let mut x2 = sys2.x.clone();
            let consistent = max_abs(&sys2.residuals(&x2)) <= SOLVED_TOL;
            if !consistent {
                lm = levenberg_marquardt(&sys2, &mut x2);
            }
            sys = sys2;
            x = x2;
            if consistent {
                break;
            }
        }
        let mut lm_status = lm.status;
        if lm_status == SolveStatus::Solved {
            // A radius is a length: the search may have wandered through zero to a negative one.
            // Mirror it (the same circle) and make sure the constraints still hold; a radius that
            // collapsed to nothing is not a solution.
            for &i in sys.radius_var.values() {
                if x[i] < 0.0 {
                    x[i] = -x[i];
                }
            }
            // Nor is a shape that shrank to nothing (a line of zero length, a circle of zero
            // radius) a solution: its directions are meaningless and everything built on it is
            // numerically fragile. Shapes that were already that small are left alone.
            let len = |v: &[f64], a: &str, b: &str| {
                let (i, j) = (sys.point_var[a], sys.point_var[b]);
                det::hypot(v[i] - v[j], v[i + 1] - v[j + 1])
            };
            // (A start that already satisfies everything is accepted as it is, so solving a
            // solved sketch again never changes its verdict.)
            let start_solved = max_abs(&sys.residuals(&x0)) <= SOLVED_TOL;
            let mut collapsed = false;
            if !start_solved {
                collapsed = sys
                    .radius_var
                    .values()
                    .any(|&i| x[i] < COLLAPSED && x0[i].abs() >= COLLAPSED);
                for e in self.entities.values() {
                    if let Entity::Line { a, b } = e {
                        if len(&x, a, b) < COLLAPSED && len(&x0, a, b) >= COLLAPSED {
                            collapsed = true;
                        }
                    }
                }
            }
            if collapsed || max_abs(&sys.residuals(&x)) > SOLVED_TOL {
                lm_status = SolveStatus::DidNotConverge;
            }
            // An arc length is only meaningful for a sweep inside one turn: the equation wraps,
            // so a solution can satisfy it with an angle beyond a full turn that the arc does
            // not actually have. Check against the real sweep.
            for (id, c) in &self.constraints {
                let Constraint::ArcLength { arc, value } = c else {
                    continue;
                };
                if self.driven.contains(id) {
                    continue;
                }
                let Some(Entity::Arc { center, start, end }) = self.entities.get(arc) else {
                    continue;
                };
                let p = |n: &str| (x[sys.point_var[n]], x[sys.point_var[n] + 1]);
                let (c0, s0, e0) = (p(center), p(start), p(end));
                let (sx, sy) = (s0.0 - c0.0, s0.1 - c0.1);
                let (ex, ey) = (e0.0 - c0.0, e0.1 - c0.1);
                let mut sweep = det::atan2(sx * ey - sy * ex, sx * ex + sy * ey);
                if sweep < 0.0 {
                    sweep += core::f64::consts::TAU;
                }
                let r = det::hypot(sx, sy);
                if (r * sweep - value).abs() > 1e-6 {
                    bad_arc_length.push(id.clone());
                }
            }
            if !bad_arc_length.is_empty() {
                lm_status = SolveStatus::Inconsistent;
            }
        }
        if lm_status == SolveStatus::Solved {
            // Snap to a 0.1 nm grid (far below the 1 nm tolerance) so solver noise like 1e-17 or
            // 79.99999999999999 never reaches diffs, hashes or files. Kept only if the
            // constraints still hold.
            let snapped: Vec<f64> = x.iter().map(|v| snap(*v)).collect();
            if max_abs(&sys.residuals(&snapped)) <= SOLVED_TOL {
                x = snapped;
            }
        }

        let (r, j) = sys.residuals_and_jacobian(&x);
        let m = r.len();
        let max_residual = max_abs(&r);

        // Degrees of freedom and free variables.
        let rr = rref(j.clone(), m, n);
        let independent_equations = rr.pivots.len();
        let free = rr.free_vars();
        let mut dof = n - independent_equations;
        let mut shape_dof: Option<usize> = None;

        // Redundant / conflicting constraints: Gram–Schmidt over rows in equation order.
        let mut basis: Vec<Vec<f64>> = Vec::new();
        let mut dependent: BTreeMap<&str, bool> = BTreeMap::new();
        let mut bad_residual: BTreeMap<&str, bool> = BTreeMap::new();
        let mut row = 0;
        // Gram–Schmidt costs about m²n, which dominates everything else for a big cluster. It is
        // only needed to find *which* rows are dependent, so when the row reduction above has
        // already found all `m` rows independent a large system skips it. (Small systems keep it
        // regardless, so their results do not depend on this shortcut at all.)
        let all_independent = m > GS_SKIP_FROM && independent_equations == m;
        for e in &sys.eqs {
            for _ in 0..e.kernel.arity() {
                if !all_independent {
                    let mut v: Vec<f64> = j[row * n..(row + 1) * n].to_vec();
                    let norm0 = det::sqrt(v.iter().map(|a| a * a).sum::<f64>());
                    for _pass in 0..2 {
                        for b in &basis {
                            let dot: f64 = v.iter().zip(b).map(|(a, c)| a * c).sum();
                            for (a, c) in v.iter_mut().zip(b) {
                                *a -= dot * c;
                            }
                        }
                    }
                    let norm = det::sqrt(v.iter().map(|a| a * a).sum::<f64>());
                    if norm0 <= 1e-12 || norm <= 1e-7 * norm0 {
                        *dependent.entry(e.owner.as_str()).or_insert(false) = true;
                    } else {
                        for a in &mut v {
                            *a /= norm;
                        }
                        basis.push(v);
                    }
                }
                if r[row].abs() > CONFLICT_TOL {
                    *bad_residual.entry(e.owner.as_str()).or_insert(false) = true;
                }
                row += 1;
            }
        }
        let mut redundant = Vec::new();
        let mut conflicting = bad_arc_length;
        for id in self.constraints.keys() {
            if dependent.contains_key(id.as_str()) {
                if bad_residual.contains_key(id.as_str()) {
                    conflicting.push(id.clone());
                } else {
                    redundant.push(id.clone());
                }
            }
        }

        // Per-entity freedom.
        let var_free = |i: usize| free[i];
        let point_free = |id: &str| {
            let i = sys.point_var[id];
            var_free(i) || var_free(i + 1)
        };
        // Construction geometry that nothing else uses (as SolidWorks does too) counts as free
        // when nothing pins it down, so the sketch as a whole is under defined; but it never
        // makes a real shape free, and `shape_dof` says how much freedom the real shapes have
        // on their own. (A point that a real shape also uses, or a free-standing point, counts
        // as part of the shapes.)
        let ignored: BTreeSet<&str> = {
            let mut used_by_real: BTreeSet<&str> = BTreeSet::new();
            let mut used_by_construction: BTreeSet<&str> = BTreeSet::new();
            for (id, e) in &self.entities {
                let into = if self.construction.contains(id) {
                    &mut used_by_construction
                } else {
                    &mut used_by_real
                };
                for r in e.refs() {
                    into.insert(r);
                }
            }
            let mut out: BTreeSet<&str> = self
                .entities
                .keys()
                .filter(|k| self.construction.contains(*k))
                .map(String::as_str)
                .collect();
            out.extend(
                used_by_construction
                    .iter()
                    .filter(|p| !used_by_real.contains(*p))
                    .copied(),
            );
            out
        };
        let (null, alone) = rr.null_basis();
        // Can this linear functional of the variables (weights on them) still change?
        let func_free = |f: &[(usize, f64)]| -> bool {
            let mut sum = 0.0;
            for &(i, w) in f {
                if alone[i] {
                    sum += w * w;
                }
            }
            for v in &null {
                let t: f64 = f.iter().map(|&(i, w)| w * v[i]).sum();
                sum += t * t;
            }
            sum > 1e-12
        };
        // Infinite construction lines have no ends: only where they are placed (their direction
        // and offset, the two ends' distances from the line) has to be defined, never how far
        // they reach. Those two quantities are linear functionals at the answer.
        let mut infinite_lines: Vec<(&str, &str, &str)> = Vec::new();
        let mut infinite_ignored: BTreeSet<&str> = BTreeSet::new();
        let mut placement: BTreeMap<&str, Vec<Vec<(usize, f64)>>> = BTreeMap::new();
        for (id, e) in &self.entities {
            if let Entity::Line { a, b } = e {
                if self.infinite.contains(id) {
                    infinite_lines.push((id.as_str(), a.as_str(), b.as_str()));
                }
            }
        }
        if !infinite_lines.is_empty() {
            let line_ids: BTreeSet<&str> = infinite_lines.iter().map(|l| l.0).collect();
            for &(id, a, b) in &infinite_lines {
                infinite_ignored.insert(id);
                for p in [a, b] {
                    let shared = self
                        .entities
                        .iter()
                        .any(|(k, e)| !line_ids.contains(k.as_str()) && e.refs().contains(&p));
                    if !shared {
                        infinite_ignored.insert(p);
                    }
                }
                let (ia, ib) = (sys.point_var[a], sys.point_var[b]);
                let (dx, dy) = (x[ib] - x[ia], x[ib + 1] - x[ia + 1]);
                let len = det::hypot(dx, dy);
                let (nx, ny) = if len > 1e-9 {
                    (-dy / len, dx / len)
                } else {
                    (0.0, 1.0)
                };
                placement.insert(
                    id,
                    vec![vec![(ia, nx), (ia + 1, ny)], vec![(ib, nx), (ib + 1, ny)]],
                );
            }
        }
        let mut under_constrained = Vec::new();
        for (id, e) in &self.entities {
            if infinite_ignored.contains(id.as_str()) && !placement.contains_key(id.as_str()) {
                continue;
            }
            let is_free = match e {
                Entity::Point { .. } => point_free(id),
                Entity::Line { a, b } => match placement.get(id.as_str()) {
                    Some(fs) => fs.iter().any(|f| func_free(f)),
                    None => point_free(a) || point_free(b),
                },
                Entity::Circle { center, .. } => {
                    point_free(center) || var_free(sys.radius_var[id.as_str()])
                }
                Entity::Arc { center, start, end } => {
                    point_free(center) || point_free(start) || point_free(end)
                }
            };
            if is_free {
                under_constrained.push(id.clone());
            }
        }

        // How many directions are still free as seen by every variable except those of the
        // `excluded` entities, plus the `extra` functionals (the dimension of the null space
        // seen through them).
        let free_without = |excluded: &BTreeSet<&str>, extra: &[&Vec<(usize, f64)>]| -> usize {
            let mut counted = vec![false; n];
            for (id, e) in &self.entities {
                if excluded.contains(id.as_str()) {
                    continue;
                }
                match e {
                    Entity::Point { .. } => {
                        let i = sys.point_var[id.as_str()];
                        counted[i] = true;
                        counted[i + 1] = true;
                    }
                    Entity::Circle { .. } => counted[sys.radius_var[id.as_str()]] = true,
                    _ => {}
                }
            }
            // Every way the sketch can move: the null space plus each unconstrained variable.
            let mut movers: Vec<Vec<f64>> = null.clone();
            for i in (0..n).filter(|&i| alone[i]) {
                let mut u = vec![0.0; n];
                u[i] = 1.0;
                movers.push(u);
            }
            let mut basis: Vec<Vec<f64>> = Vec::new();
            for v in &movers {
                let mut w: Vec<f64> = (0..n).filter(|&i| counted[i]).map(|i| v[i]).collect();
                for f in extra {
                    w.push(f.iter().map(|&(i, k)| k * v[i]).sum());
                }
                for _pass in 0..2 {
                    for b in &basis {
                        let dot: f64 = w.iter().zip(b).map(|(a, c)| a * c).sum();
                        for (a, c) in w.iter_mut().zip(b) {
                            *a -= dot * c;
                        }
                    }
                }
                let norm = det::sqrt(w.iter().map(|a| a * a).sum::<f64>());
                if norm > 1e-7 {
                    for a in &mut w {
                        *a /= norm;
                    }
                    basis.push(w);
                }
            }
            basis.len()
        };
        if !infinite_lines.is_empty() {
            let extra: Vec<&Vec<(usize, f64)>> = placement.values().flatten().collect();
            dof = free_without(&infinite_ignored, &extra);
        }
        if !ignored.is_empty() {
            let shapes = free_without(&ignored, &[]);
            shape_dof = (shapes != dof).then_some(shapes);
        }

        // Which way things can move. For any pair of linear functionals of the variables (a
        // point's x and y; a line's middle point), the 2x2 covariance over the null space of the
        // constraints has one eigenvalue per free direction (0, or about 1 for a fully free axis).
        let motion_of = |fx: &[(usize, f64)], fy: &[(usize, f64)]| -> Option<PointMotion> {
            let (mut a, mut b, mut d) = (0.0, 0.0, 0.0);
            for &(i, w) in fx {
                if alone[i] {
                    a += w * w;
                }
            }
            for &(i, w) in fy {
                if alone[i] {
                    d += w * w;
                }
            }
            for v in &null {
                let px: f64 = fx.iter().map(|&(i, w)| w * v[i]).sum();
                let py: f64 = fy.iter().map(|&(i, w)| w * v[i]).sum();
                a += px * px;
                b += px * py;
                d += py * py;
            }
            let mid = (a + d) / 2.0;
            let rad = det::hypot((a - d) / 2.0, b);
            let (big, small) = (mid + rad, mid - rad);
            const MOVES: f64 = 1e-6;
            if small > MOVES {
                Some(PointMotion {
                    dof: 2,
                    direction: None,
                })
            } else if big > MOVES {
                let t = 0.5 * det::atan2(2.0 * b, a - d);
                let (mut ux, mut uy) = (snap(det::cos(t)), snap(det::sin(t)));
                if ux < -1e-9 || (ux.abs() <= 1e-9 && uy < 0.0) {
                    ux = -ux;
                    uy = -uy;
                }
                Some(PointMotion {
                    dof: 1,
                    direction: Some([canon(ux), canon(uy)]),
                })
            } else {
                None
            }
        };
        // Can this one quantity (a radius, a sweep angle) still change?
        let quantity_free = |f: &BTreeMap<usize, f64>| -> bool {
            let mut sum = 0.0;
            for (&i, &w) in f {
                if alone[i] {
                    sum += w * w;
                }
            }
            for v in &null {
                let t: f64 = f.iter().map(|(&i, &w)| w * v[i]).sum();
                sum += t * t;
            }
            sum > 1e-12
        };
        let at = |id: &str| (x[sys.point_var[id]], x[sys.point_var[id] + 1]);
        // d|p - c| for a point p about a centre c, as weights on the variables.
        let radial = |p: &str, c: &str, k: f64, w: &mut BTreeMap<usize, f64>| {
            let (pp, cc) = (at(p), at(c));
            let r = det::hypot(pp.0 - cc.0, pp.1 - cc.1).max(1e-12);
            let (ux, uy) = ((pp.0 - cc.0) / r, (pp.1 - cc.1) / r);
            let (ip, ic) = (sys.point_var[p], sys.point_var[c]);
            for (i, v) in [(ip, ux), (ip + 1, uy), (ic, -ux), (ic + 1, -uy)] {
                *w.entry(i).or_insert(0.0) += k * v;
            }
        };
        // d(angle of p about c), as weights on the variables.
        let angular = |p: &str, c: &str, k: f64, w: &mut BTreeMap<usize, f64>| {
            let (pp, cc) = (at(p), at(c));
            let r = det::hypot(pp.0 - cc.0, pp.1 - cc.1).max(1e-12);
            let (tx, ty) = (-(pp.1 - cc.1) / (r * r), (pp.0 - cc.0) / (r * r));
            let (ip, ic) = (sys.point_var[p], sys.point_var[c]);
            for (i, v) in [(ip, tx), (ip + 1, ty), (ic, -tx), (ic + 1, -ty)] {
                *w.entry(i).or_insert(0.0) += k * v;
            }
        };
        let mut motion = BTreeMap::new();
        let mut shape_motion = BTreeMap::new();
        let mut radius_free: Vec<String> = Vec::new();
        let mut sweep_free: Vec<String> = Vec::new();
        for (id, e) in &self.entities {
            if infinite_ignored.contains(id.as_str()) {
                continue;
            }
            match e {
                Entity::Point { .. } => {
                    let i = sys.point_var[id.as_str()];
                    if let Some(m) = motion_of(&[(i, 1.0)], &[(i + 1, 1.0)]) {
                        motion.insert(id.clone(), m);
                    }
                }
                Entity::Line { a, b } => {
                    let (ia, ib) = (sys.point_var[a.as_str()], sys.point_var[b.as_str()]);
                    let fx = [(ia, 0.5), (ib, 0.5)];
                    let fy = [(ia + 1, 0.5), (ib + 1, 0.5)];
                    if let Some(m) = motion_of(&fx, &fy) {
                        shape_motion.insert(id.clone(), m);
                    }
                }
                Entity::Circle { .. } => {
                    if var_free(sys.radius_var[id.as_str()]) {
                        radius_free.push(id.clone());
                    }
                }
                Entity::Arc { center, start, end } => {
                    let mut r = BTreeMap::new();
                    radial(start, center, 1.0, &mut r);
                    if quantity_free(&r) {
                        radius_free.push(id.clone());
                    }
                    let mut sw = BTreeMap::new();
                    angular(end, center, 1.0, &mut sw);
                    angular(start, center, -1.0, &mut sw);
                    if quantity_free(&sw) {
                        sweep_free.push(id.clone());
                    }
                }
            }
        }

        conflicting.sort();
        conflicting.dedup();
        let status = match lm_status {
            SolveStatus::Solved => SolveStatus::Solved,
            SolveStatus::DidNotConverge if conflicting.is_empty() => SolveStatus::DidNotConverge,
            _ => SolveStatus::Inconsistent,
        };

        let applied = status == SolveStatus::Solved;
        if applied {
            store(&mut self.entities, &sys, &x);
        }

        SolveReport {
            status,
            iterations: lm.iterations,
            max_residual,
            variables: n,
            independent_equations,
            dof,
            shape_dof,
            under_constrained,
            motion,
            shape_motion,
            radius_free,
            sweep_free,
            redundant,
            conflicting,
            applied,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Constraint::*;

    /// A staircase of `n` lines: one connected cluster, fully dimensioned from a fixed point,
    /// started a little off the solution.
    fn staircase(n: usize) -> Sketch {
        let mut s = Sketch::new();
        let (mut x, mut y) = (0.0, 0.0);
        pt(&mut s, "p0", x, y);
        s.add_constraint(
            "fix",
            Fixed {
                point: "p0".into(),
                x: 0.0,
                y: 0.0,
            },
        )
        .unwrap();
        for i in 0..n {
            if i % 2 == 0 {
                x += 10.3;
            } else {
                y += 7.9;
            }
            pt(&mut s, &format!("p{}", i + 1), x, y);
            line(
                &mut s,
                &format!("l{i}"),
                &format!("p{i}"),
                &format!("p{}", i + 1),
            );
            s.add_constraint(
                &format!("d{i}"),
                if i % 2 == 0 {
                    Horizontal {
                        line: format!("l{i}"),
                    }
                } else {
                    Vertical {
                        line: format!("l{i}"),
                    }
                },
            )
            .unwrap();
            s.add_constraint(
                &format!("n{i}"),
                Length {
                    line: format!("l{i}"),
                    value: if i % 2 == 0 { 10.0 } else { 8.0 },
                },
            )
            .unwrap();
        }
        s
    }

    #[test]
    fn the_sparse_newton_step_finds_the_same_solution_as_the_dense_one() {
        for n in [20usize, 90, 150] {
            let s = staircase(n);
            let sys = System::build(&s, &[], None);
            let (mut xd, mut xs) = (sys.x.clone(), sys.x.clone());
            let d = lm(&sys, &mut xd, false);
            let sp = lm(&sys, &mut xs, true);
            assert_eq!(
                (d.status, sp.status),
                (SolveStatus::Solved, SolveStatus::Solved)
            );
            for (a, b) in xd.iter().zip(&xs) {
                assert!((a - b).abs() < 1e-9, "n={n}: {a} vs {b}");
            }
            assert!(max_abs(&sys.residuals(&xs)) < 1e-10);
        }
    }

    #[test]
    fn a_big_connected_sketch_solves_exactly_and_reports_its_freedom_correctly() {
        // 300 lines: 602 variables, well over the sparse threshold. Every line has the length
        // asked for, the corners are exactly where they must be, and the sketch is fully defined.
        let mut s = staircase(300);
        let r = s.solve();
        assert_eq!(r.status, SolveStatus::Solved, "{r:?}");
        assert!(r.redundant.is_empty() && r.conflicting.is_empty(), "{r:?}");
        assert_eq!(r.dof, 0, "fully dimensioned");
        // The last corner: 150 steps of 10 across and 150 of 8 up.
        let (x, y) = s.point("p300").expect("the last point");
        assert!(
            (x - 1500.0).abs() < 1e-6 && (y - 1200.0).abs() < 1e-6,
            "({x}, {y})"
        );
    }

    #[test]
    fn a_big_sketch_still_finds_a_redundant_and_a_conflicting_constraint() {
        let mut s = staircase(120);
        // Redundant: the same length again.
        s.add_constraint(
            "again",
            Length {
                line: "l0".into(),
                value: 10.0,
            },
        )
        .unwrap();
        let r = s.solve();
        assert_eq!(r.status, SolveStatus::Solved, "{r:?}");
        assert!(
            r.redundant.iter().any(|c| c == "again" || c == "n0"),
            "{:?}",
            r.redundant
        );
        // Conflicting: a different length for the same line.
        let mut t = staircase(120);
        t.add_constraint(
            "other",
            Length {
                line: "l0".into(),
                value: 12.0,
            },
        )
        .unwrap();
        let r = t.solve();
        assert!(
            r.status == SolveStatus::Inconsistent
                || r.conflicting.iter().any(|c| c == "other" || c == "n0"),
            "{r:?}"
        );
    }

    fn pt(s: &mut Sketch, id: &str, x: f64, y: f64) {
        s.add_entity(id, Entity::Point { x, y }).unwrap();
    }
    fn line(s: &mut Sketch, id: &str, a: &str, b: &str) {
        s.add_entity(
            id,
            Entity::Line {
                a: a.into(),
                b: b.into(),
            },
        )
        .unwrap();
    }
    fn near(s: &Sketch, id: &str, x: f64, y: f64) {
        let (px, py) = s.point(id).unwrap();
        assert!(
            (px - x).abs() < 1e-8 && (py - y).abs() < 1e-8,
            "{id} = ({px}, {py}), want ({x}, {y})"
        );
    }

    /// A deliberately sloppy hand-drawn rectangle.
    fn sloppy_rectangle() -> Sketch {
        let mut s = Sketch::new();
        pt(&mut s, "p1", 0.0, 0.0);
        pt(&mut s, "p2", 70.0, 5.0);
        pt(&mut s, "p3", 75.0, 45.0);
        pt(&mut s, "p4", 3.0, 38.0);
        line(&mut s, "l1", "p1", "p2");
        line(&mut s, "l2", "p2", "p3");
        line(&mut s, "l3", "p3", "p4");
        line(&mut s, "l4", "p4", "p1");
        s.add_constraint("c01_h1", Horizontal { line: "l1".into() })
            .unwrap();
        s.add_constraint("c02_v2", Vertical { line: "l2".into() })
            .unwrap();
        s.add_constraint("c03_h3", Horizontal { line: "l3".into() })
            .unwrap();
        s.add_constraint("c04_v4", Vertical { line: "l4".into() })
            .unwrap();
        s.add_constraint(
            "c05_w",
            Length {
                line: "l1".into(),
                value: 80.0,
            },
        )
        .unwrap();
        s.add_constraint(
            "c06_h",
            Length {
                line: "l2".into(),
                value: 40.0,
            },
        )
        .unwrap();
        s.add_constraint(
            "c07_anchor",
            Fixed {
                point: "p1".into(),
                x: 0.0,
                y: 0.0,
            },
        )
        .unwrap();
        s
    }

    /// Every constraint kind, at awkward non-axis-aligned positions.
    fn kitchen_sink() -> Sketch {
        let mut s = Sketch::new();
        for (id, x, y) in [
            ("p1", 1.3, 0.7),
            ("p2", 9.1, 2.9),
            ("p3", 4.4, 8.2),
            ("p4", -3.0, 5.5),
            ("p5", 7.7, -4.1),
            ("p6", 12.0, 6.0),
            ("c1", 2.0, -2.0),
            ("c2", 15.0, 3.0),
            ("as", 17.0, 4.5),
            ("ae", 14.0, 6.2),
            ("q", 3.3, 3.1),
            ("c3", 6.0, 9.0),
            ("as2", 7.5, 9.0),
            ("ae2", 6.0, 10.5),
        ] {
            pt(&mut s, id, x, y);
        }
        line(&mut s, "l1", "p1", "p2");
        line(&mut s, "l2", "p3", "p4");
        line(&mut s, "l3", "p5", "p6");
        s.add_entity(
            "circ",
            Entity::Circle {
                center: "c1".into(),
                radius: 2.5,
            },
        )
        .unwrap();
        s.add_entity(
            "arc",
            Entity::Arc {
                center: "c2".into(),
                start: "as".into(),
                end: "ae".into(),
            },
        )
        .unwrap();
        s.add_entity(
            "circ2",
            Entity::Circle {
                center: "c3".into(),
                radius: 1.5,
            },
        )
        .unwrap();
        s.add_entity(
            "arc2",
            Entity::Arc {
                center: "c3".into(),
                start: "as2".into(),
                end: "ae2".into(),
            },
        )
        .unwrap();
        let cs: Vec<(&str, Constraint)> = vec![
            (
                "k29",
                ArcLength {
                    arc: "arc".into(),
                    value: 3.0,
                },
            ),
            (
                "k30",
                Angle {
                    a: "l1".into(),
                    b: "l2".into(),
                    value: 35.0,
                },
            ),
            (
                "k31",
                SymmetricAxis {
                    a: "p4".into(),
                    b: "p5".into(),
                    axis: crate::SketchAxis::X,
                },
            ),
            (
                "k32",
                SymmetricAxis {
                    a: "q".into(),
                    b: "p6".into(),
                    axis: crate::SketchAxis::Y,
                },
            ),
            (
                "k33",
                TangentCurves {
                    a: "circ".into(),
                    b: "arc".into(),
                },
            ),
            (
                "k34",
                TangentCurves {
                    a: "arc".into(),
                    b: "circ2".into(),
                },
            ),
            (
                "k35",
                TangentCurves {
                    a: "circ".into(),
                    b: "circ2".into(),
                },
            ),
            (
                "k36",
                TangentCurves {
                    a: "arc".into(),
                    b: "arc2".into(),
                },
            ),
            (
                "k01",
                Coincident {
                    a: "p2".into(),
                    b: "p3".into(),
                },
            ),
            ("k02", Horizontal { line: "l1".into() }),
            ("k03", Vertical { line: "l2".into() }),
            (
                "k04",
                Parallel {
                    a: "l1".into(),
                    b: "l3".into(),
                },
            ),
            (
                "k05",
                Perpendicular {
                    a: "l1".into(),
                    b: "l2".into(),
                },
            ),
            (
                "k06",
                EqualLength {
                    a: "l1".into(),
                    b: "l3".into(),
                },
            ),
            (
                "k07",
                Length {
                    line: "l2".into(),
                    value: 7.0,
                },
            ),
            (
                "k08",
                Distance {
                    a: "p1".into(),
                    b: "p6".into(),
                    value: 11.0,
                },
            ),
            (
                "k09",
                Radius {
                    target: "circ".into(),
                    value: 3.0,
                },
            ),
            (
                "k10",
                Radius {
                    target: "arc".into(),
                    value: 4.0,
                },
            ),
            (
                "k11",
                Fixed {
                    point: "p1".into(),
                    x: 1.0,
                    y: 1.0,
                },
            ),
            (
                "k12",
                PointOnLine {
                    point: "q".into(),
                    line: "l1".into(),
                },
            ),
            (
                "k13",
                Tangent {
                    line: "l3".into(),
                    curve: "circ".into(),
                },
            ),
            (
                "k14",
                Tangent {
                    line: "l2".into(),
                    curve: "arc".into(),
                },
            ),
            (
                "k15",
                HorizontalDistance {
                    a: "p3".into(),
                    b: "p5".into(),
                    value: 3.2,
                },
            ),
            (
                "k16",
                VerticalDistance {
                    a: "p4".into(),
                    b: "p6".into(),
                    value: -2.5,
                },
            ),
            (
                "k17",
                PointLineDistance {
                    point: "p5".into(),
                    line: "l2".into(),
                    value: 1.7,
                },
            ),
            (
                "k18",
                PointLineDistance {
                    point: "p6".into(),
                    line: "l2".into(),
                    value: -2.0,
                },
            ),
            (
                "k21",
                Midpoint {
                    point: "q".into(),
                    line: "l3".into(),
                },
            ),
            (
                "k22",
                Symmetric {
                    a: "q".into(),
                    b: "p5".into(),
                    line: "l1".into(),
                },
            ),
            (
                "k23",
                EqualRadius {
                    a: "circ".into(),
                    b: "arc".into(),
                },
            ),
            (
                "k24",
                PointOnCurve {
                    point: "q".into(),
                    curve: "circ".into(),
                },
            ),
            (
                "k25",
                PointOnCurve {
                    point: "p6".into(),
                    curve: "arc".into(),
                },
            ),
            (
                "k19",
                OnAxis {
                    point: "p2".into(),
                    axis: crate::SketchAxis::X,
                },
            ),
            (
                "k20",
                OnAxis {
                    point: "p4".into(),
                    axis: crate::SketchAxis::Y,
                },
            ),
        ];
        for (id, c) in cs {
            s.add_constraint(id, c).unwrap();
        }
        s
    }

    #[test]
    fn autodiff_jacobian_matches_finite_differences_for_every_constraint() {
        let s = kitchen_sink();
        let sys = System::build(&s, &[], None);
        let n = sys.x.len();
        let (r0, j) = sys.residuals_and_jacobian(&sys.x);
        assert_eq!(
            r0,
            sys.residuals(&sys.x),
            "dual and plain evaluation must agree"
        );
        let h = 1e-6;
        let mut worst: (f64, usize, usize) = (0.0, 0, 0);
        for col in 0..n {
            let (mut xp, mut xm) = (sys.x.clone(), sys.x.clone());
            xp[col] += h;
            xm[col] -= h;
            let (rp, rm) = (sys.residuals(&xp), sys.residuals(&xm));
            for row in 0..r0.len() {
                let fd = (rp[row] - rm[row]) / (2.0 * h);
                let err = (fd - j[row * n + col]).abs() / (1.0 + fd.abs());
                if err > worst.0 {
                    worst = (err, row, col);
                }
            }
        }
        // Report which equation owns the worst row.
        let mut row = 0;
        let mut owner = String::new();
        'o: for e in &sys.eqs {
            for _ in 0..e.kernel.arity() {
                if row == worst.1 {
                    owner = format!("{} ({:?})", e.owner, e.kernel);
                    break 'o;
                }
                row += 1;
            }
        }
        assert!(
            worst.0 < 1e-6,
            "worst Jacobian error {:.3e} in {owner}, column {}",
            worst.0,
            worst.2
        );
    }

    /// An anchored line along +x from the origin and one free point.
    fn axis_line_and_point() -> Sketch {
        let mut s = Sketch::new();
        pt(&mut s, "o", 0.0, 0.0);
        pt(&mut s, "e", 10.0, 0.0);
        pt(&mut s, "p", 4.0, 3.0);
        line(&mut s, "l", "o", "e");
        s.add_constraint(
            "fo",
            Fixed {
                point: "o".into(),
                x: 0.0,
                y: 0.0,
            },
        )
        .unwrap();
        s.add_constraint(
            "fe",
            Fixed {
                point: "e".into(),
                x: 10.0,
                y: 0.0,
            },
        )
        .unwrap();
        s
    }

    #[test]
    fn horizontal_and_vertical_distances_place_a_point_exactly_and_keep_their_sign() {
        let mut s = axis_line_and_point();
        s.add_constraint(
            "dx",
            HorizontalDistance {
                a: "o".into(),
                b: "p".into(),
                value: 30.0,
            },
        )
        .unwrap();
        s.add_constraint(
            "dy",
            VerticalDistance {
                a: "o".into(),
                b: "p".into(),
                value: -20.0,
            },
        )
        .unwrap();
        let r = s.solve();
        assert_eq!(r.status, SolveStatus::Solved, "{r:?}");
        near(&s, "p", 30.0, -20.0);
        assert_eq!(r.dof, 0, "two equations pin a free point");
        // The dimension reads as a magnitude, and editing it keeps the side it was on.
        assert_eq!(s.constraints()["dy"].value(), Some(20.0));
        s.set_constraint_value("dy", 35.0).unwrap();
        assert_eq!(
            s.constraints()["dy"],
            VerticalDistance {
                a: "o".into(),
                b: "p".into(),
                value: -35.0
            }
        );
        assert_eq!(s.solve().status, SolveStatus::Solved);
        near(&s, "p", 30.0, -35.0);
    }

    #[test]
    fn point_line_distance_is_signed_by_the_side_of_the_line() {
        for (value, want_y) in [(5.0, 5.0), (-5.0, -5.0)] {
            let mut s = axis_line_and_point();
            s.add_constraint(
                "d",
                PointLineDistance {
                    point: "p".into(),
                    line: "l".into(),
                    value,
                },
            )
            .unwrap();
            // Pin x so the only freedom is the distance.
            s.add_constraint(
                "dx",
                HorizontalDistance {
                    a: "o".into(),
                    b: "p".into(),
                    value: 4.0,
                },
            )
            .unwrap();
            let r = s.solve();
            assert_eq!(r.status, SolveStatus::Solved, "{value}: {r:?}");
            near(&s, "p", 4.0, want_y);
            assert_eq!(r.dof, 0);
        }
    }

    #[test]
    fn distances_that_cannot_make_sense_are_refused_with_a_reason() {
        let mut s = axis_line_and_point();
        // A point that is an end of the line is always at distance zero from it.
        let e = s
            .add_constraint(
                "d",
                PointLineDistance {
                    point: "o".into(),
                    line: "l".into(),
                    value: 3.0,
                },
            )
            .unwrap_err();
        assert_eq!(e.code, crate::model::SketchErrorCode::InvalidValue);
        assert!(e.message.contains("end of that line"), "{e:?}");
        let e = s
            .add_constraint(
                "d",
                HorizontalDistance {
                    a: "o".into(),
                    b: "o".into(),
                    value: 1.0,
                },
            )
            .unwrap_err();
        assert_eq!(e.code, crate::model::SketchErrorCode::InvalidValue);
        let e = s
            .add_constraint(
                "d",
                VerticalDistance {
                    a: "o".into(),
                    b: "l".into(),
                    value: 1.0,
                },
            )
            .unwrap_err();
        assert_eq!(e.code, crate::model::SketchErrorCode::WrongKind);
        let e = s
            .add_constraint(
                "d",
                HorizontalDistance {
                    a: "o".into(),
                    b: "p".into(),
                    value: f64::NAN,
                },
            )
            .unwrap_err();
        assert_eq!(e.code, crate::model::SketchErrorCode::InvalidValue);
    }

    #[test]
    fn dimensioned_rectangle_solves_exactly_and_is_fully_constrained() {
        let mut s = sloppy_rectangle();
        let r = s.solve();
        assert_eq!(r.status, SolveStatus::Solved, "{r:?}");
        assert!(r.is_fully_constrained(), "{r:?}");
        assert_eq!((r.variables, r.independent_equations, r.dof), (8, 8, 0));
        assert!(
            r.under_constrained.is_empty() && r.redundant.is_empty() && r.conflicting.is_empty()
        );
        assert!(r.applied && r.max_residual <= SOLVED_TOL);
        near(&s, "p1", 0.0, 0.0);
        near(&s, "p2", 80.0, 0.0);
        near(&s, "p3", 80.0, 40.0);
        near(&s, "p4", 0.0, 40.0);
    }

    #[test]
    fn removing_a_dimension_leaves_exactly_one_dof_and_names_what_moves() {
        let mut s = sloppy_rectangle();
        s.remove_constraint("c06_h").unwrap();
        let r = s.solve();
        assert_eq!(r.status, SolveStatus::Solved);
        assert_eq!(r.dof, 1);
        // p2 is pinned by anchor + horizontal + length; the far corner points can still move.
        assert!(!r.under_constrained.contains(&"p1".to_string()));
        assert!(!r.under_constrained.contains(&"p2".to_string()));
        for id in ["p3", "p4", "l2", "l3", "l4"] {
            assert!(
                r.under_constrained.contains(&id.to_string()),
                "{id} should be free: {r:?}"
            );
        }
    }

    #[test]
    fn unanchored_rectangle_has_two_translational_dof() {
        let mut s = sloppy_rectangle();
        s.remove_constraint("c07_anchor").unwrap();
        let r = s.solve();
        assert_eq!(r.status, SolveStatus::Solved);
        assert_eq!(r.dof, 2);
    }

    #[test]
    fn redundant_constraints_are_identified_not_treated_as_errors() {
        let mut s = sloppy_rectangle();
        s.add_constraint(
            "c08_perp",
            Perpendicular {
                a: "l1".into(),
                b: "l2".into(),
            },
        )
        .unwrap();
        s.add_constraint("c09_dup", Horizontal { line: "l1".into() })
            .unwrap();
        let r = s.solve();
        assert_eq!(r.status, SolveStatus::Solved, "{r:?}");
        assert_eq!(r.redundant, ["c08_perp", "c09_dup"]);
        assert!(r.conflicting.is_empty());
        assert_eq!(r.dof, 0);
        near(&s, "p3", 80.0, 40.0);
    }

    #[test]
    fn conflicting_constraints_are_reported_and_geometry_is_left_untouched() {
        let mut s = sloppy_rectangle();
        s.add_constraint(
            "c08_clash",
            Distance {
                a: "p1".into(),
                b: "p2".into(),
                value: 90.0,
            },
        )
        .unwrap();
        let before = s.clone();
        let r = s.solve();
        assert_eq!(r.status, SolveStatus::Inconsistent, "{r:?}");
        assert_eq!(r.conflicting, ["c08_clash"]);
        assert!(!r.applied);
        assert_eq!(s, before, "a failed solve must not change the sketch");
        assert!(r.max_residual > 1e-3);
    }

    #[test]
    fn circle_tangent_to_line_picks_the_nearest_solution() {
        let mut s = Sketch::new();
        pt(&mut s, "p1", 0.0, 0.0);
        pt(&mut s, "p2", 10.0, 0.0);
        pt(&mut s, "c", 4.0, 7.0);
        line(&mut s, "l", "p1", "p2");
        s.add_entity(
            "circ",
            Entity::Circle {
                center: "c".into(),
                radius: 3.0,
            },
        )
        .unwrap();
        s.add_constraint(
            "a1",
            Fixed {
                point: "p1".into(),
                x: 0.0,
                y: 0.0,
            },
        )
        .unwrap();
        s.add_constraint(
            "a2",
            Fixed {
                point: "p2".into(),
                x: 10.0,
                y: 0.0,
            },
        )
        .unwrap();
        s.add_constraint(
            "r",
            Radius {
                target: "circ".into(),
                value: 5.0,
            },
        )
        .unwrap();
        s.add_constraint(
            "t",
            Tangent {
                line: "l".into(),
                curve: "circ".into(),
            },
        )
        .unwrap();
        s.add_constraint(
            "d",
            Distance {
                a: "p1".into(),
                b: "c".into(),
                value: 41f64.sqrt(),
            },
        )
        .unwrap();
        let rep = s.solve();
        assert_eq!(rep.status, SolveStatus::Solved, "{rep:?}");
        assert_eq!(rep.dof, 0, "{rep:?}");
        near(&s, "c", 4.0, 5.0);
        match s.entity("circ").unwrap() {
            Entity::Circle { radius, .. } => assert!((radius - 5.0).abs() < 1e-8),
            _ => unreachable!(),
        }
    }

    #[test]
    fn arcs_keep_both_radii_equal_implicitly() {
        let mut s = Sketch::new();
        pt(&mut s, "c", 0.0, 0.0);
        pt(&mut s, "s", 10.0, 0.0);
        pt(&mut s, "e", 1.0, 8.0);
        pt(&mut s, "a", 0.0, 0.0);
        pt(&mut s, "b", 0.0, 20.0);
        line(&mut s, "axis", "a", "b");
        s.add_entity(
            "arc",
            Entity::Arc {
                center: "c".into(),
                start: "s".into(),
                end: "e".into(),
            },
        )
        .unwrap();
        for (id, p, x, y) in [
            ("f1", "c", 0.0, 0.0),
            ("f2", "s", 10.0, 0.0),
            ("f3", "a", 0.0, 0.0),
            ("f4", "b", 0.0, 20.0),
        ] {
            s.add_constraint(
                id,
                Fixed {
                    point: p.into(),
                    x,
                    y,
                },
            )
            .unwrap();
        }
        s.add_constraint(
            "on",
            PointOnLine {
                point: "e".into(),
                line: "axis".into(),
            },
        )
        .unwrap();
        let r = s.solve();
        assert_eq!(r.status, SolveStatus::Solved, "{r:?}");
        assert_eq!(r.dof, 0, "{r:?}");
        near(&s, "e", 0.0, 10.0);
    }

    #[test]
    fn arc_radius_and_tangent_line() {
        let mut s = Sketch::new();
        pt(&mut s, "c", 0.0, 0.0);
        pt(&mut s, "s", 9.0, 1.0);
        pt(&mut s, "e", 0.5, 8.0);
        pt(&mut s, "p", -20.0, 5.5);
        pt(&mut s, "q", 20.0, 5.0);
        line(&mut s, "tl", "p", "q");
        s.add_entity(
            "arc",
            Entity::Arc {
                center: "c".into(),
                start: "s".into(),
                end: "e".into(),
            },
        )
        .unwrap();
        s.add_constraint(
            "f1",
            Fixed {
                point: "c".into(),
                x: 0.0,
                y: 0.0,
            },
        )
        .unwrap();
        s.add_constraint(
            "f2",
            Fixed {
                point: "p".into(),
                x: -20.0,
                y: 6.0,
            },
        )
        .unwrap();
        s.add_constraint("hz", Horizontal { line: "tl".into() })
            .unwrap();
        s.add_constraint(
            "r",
            Radius {
                target: "arc".into(),
                value: 6.0,
            },
        )
        .unwrap();
        s.add_constraint(
            "tg",
            Tangent {
                line: "tl".into(),
                curve: "arc".into(),
            },
        )
        .unwrap();
        let r = s.solve();
        assert_eq!(r.status, SolveStatus::Solved, "{r:?}");
        // Tangent horizontal line at y = 6 above a radius-6 arc centered on the origin.
        let (_, qy) = s.point("q").unwrap();
        assert!((qy - 6.0).abs() < 1e-8, "q.y = {qy}");
    }

    #[test]
    fn dragging_a_free_corner_lands_exactly_on_the_cursor() {
        let mut s = sloppy_rectangle();
        s.remove_constraint("c06_h").unwrap();
        assert!(s.solve().applied);
        let r = s.drag_point("p3", 80.0, 55.0).unwrap();
        assert_eq!(r.status, SolveStatus::Solved);
        near(&s, "p3", 80.0, 55.0); // held exactly at the cursor
        near(&s, "p4", 0.0, 55.0); // horizontal/vertical relationships followed
        near(&s, "p2", 80.0, 0.0); // anchored geometry did not move
        assert_eq!(
            r.dof, 1,
            "the report describes the sketch, not the temporary hold"
        );
    }

    #[test]
    fn dragging_in_a_fully_constrained_sketch_snaps_back_to_the_valid_shape() {
        let mut s = sloppy_rectangle();
        assert!(s.solve().is_fully_constrained());
        let r = s.drag_point("p3", 120.0, 90.0).unwrap();
        assert_eq!(r.status, SolveStatus::Solved);
        near(&s, "p3", 80.0, 40.0);
        near(&s, "p4", 0.0, 40.0);
    }

    #[test]
    fn dragging_a_point_on_a_line_slides_along_it() {
        let mut s = Sketch::new();
        pt(&mut s, "a", 0.0, 0.0);
        pt(&mut s, "b", 10.0, 0.0);
        pt(&mut s, "m", 4.0, 0.0);
        line(&mut s, "l", "a", "b");
        s.add_constraint(
            "fa",
            Fixed {
                point: "a".into(),
                x: 0.0,
                y: 0.0,
            },
        )
        .unwrap();
        s.add_constraint(
            "fb",
            Fixed {
                point: "b".into(),
                x: 10.0,
                y: 0.0,
            },
        )
        .unwrap();
        s.add_constraint(
            "on",
            PointOnLine {
                point: "m".into(),
                line: "l".into(),
            },
        )
        .unwrap();
        let r = s.drag_point("m", 7.0, 5.0).unwrap(); // cursor is off the line
        assert_eq!(r.status, SolveStatus::Solved);
        let (mx, my) = s.point("m").unwrap();
        assert!(my.abs() < 1e-8, "must stay on the line, y = {my}");
        assert!(
            mx > 5.0,
            "should have followed the cursor toward x=7, got {mx}"
        );
    }

    #[test]
    fn a_failed_drag_leaves_the_sketch_untouched() {
        let mut s = sloppy_rectangle();
        s.add_constraint(
            "c08_clash",
            Distance {
                a: "p1".into(),
                b: "p2".into(),
                value: 90.0,
            },
        )
        .unwrap();
        let before = s.clone();
        let r = s.drag_point("p3", 50.0, 50.0).unwrap();
        assert!(!r.applied);
        assert_eq!(s, before);
        assert!(s.drag_point("nope", 0.0, 0.0).is_err());
        assert!(s.drag_point("p3", f64::NAN, 0.0).is_err());
        assert_eq!(s, before, "invalid drags must not modify the sketch either");
    }

    #[test]
    fn arc_radius_converges_from_a_far_off_start() {
        // Regression: with Marquardt-scaled damping this never converged; ridge damping
        // (minimum-norm Newton) solves it in a handful of iterations.
        let mut s = Sketch::new();
        pt(&mut s, "c", 0.0, 0.0);
        pt(&mut s, "s", 9.0, 1.0);
        pt(&mut s, "e", 0.5, 8.0);
        s.add_entity(
            "arc",
            Entity::Arc {
                center: "c".into(),
                start: "s".into(),
                end: "e".into(),
            },
        )
        .unwrap();
        s.add_constraint(
            "f",
            Fixed {
                point: "c".into(),
                x: 0.0,
                y: 0.0,
            },
        )
        .unwrap();
        s.add_constraint(
            "r",
            Radius {
                target: "arc".into(),
                value: 6.0,
            },
        )
        .unwrap();
        let r = s.solve();
        assert_eq!(r.status, SolveStatus::Solved, "{r:?}");
        assert!(r.iterations <= 10, "took {} iterations", r.iterations);
        assert_eq!(r.dof, 2); // the two angles
    }

    #[test]
    fn solving_is_bit_for_bit_deterministic() {
        let mut a = sloppy_rectangle();
        let mut b = sloppy_rectangle();
        let ra = a.solve();
        let rb = b.solve();
        assert_eq!(ra, rb);
        assert_eq!(
            serde_json::to_string(&a).unwrap(),
            serde_json::to_string(&b).unwrap()
        );
        // And re-solving a solved sketch is a no-op.
        let before = serde_json::to_string(&a).unwrap();
        let again = a.solve();
        assert_eq!(again.iterations, 0);
        assert_eq!(serde_json::to_string(&a).unwrap(), before);
    }

    #[test]
    fn empty_and_unconstrained_sketches() {
        let mut s = Sketch::new();
        let r = s.solve();
        assert_eq!((r.status, r.dof), (SolveStatus::Solved, 0));
        pt(&mut s, "p", 1.0, 2.0);
        pt(&mut s, "q", 3.0, 4.0);
        let r = s.solve();
        assert_eq!(r.dof, 4);
        assert_eq!(r.under_constrained, ["p", "q"]);
    }

    #[test]
    fn scale_independence_millimetres_to_metres() {
        let mut s = sloppy_rectangle();
        s.remove_constraint("c05_w").unwrap();
        s.remove_constraint("c06_h").unwrap();
        s.add_constraint(
            "big_w",
            Length {
                line: "l1".into(),
                value: 12_000.0,
            },
        )
        .unwrap();
        s.add_constraint(
            "big_h",
            Length {
                line: "l2".into(),
                value: 7_500.0,
            },
        )
        .unwrap();
        let r = s.solve();
        assert_eq!(r.status, SolveStatus::Solved, "{r:?}");
        near(&s, "p3", 12_000.0, 7_500.0);
    }

    fn fix(s: &mut Sketch, id: &str, p: &str, x: f64, y: f64) {
        s.add_constraint(
            id,
            Fixed {
                point: p.into(),
                x,
                y,
            },
        )
        .unwrap();
    }

    #[test]
    fn motion_reports_which_way_each_point_can_still_move() {
        let mut s = Sketch::new();
        pt(&mut s, "free", 5.0, 5.0);
        pt(&mut s, "pinned", 0.0, 0.0);
        pt(&mut s, "a", 0.0, 10.0);
        pt(&mut s, "b", 10.0, 12.0);
        line(&mut s, "l", "a", "b");
        fix(&mut s, "f1", "pinned", 0.0, 0.0);
        fix(&mut s, "f2", "a", 0.0, 10.0);
        s.add_constraint("h", Horizontal { line: "l".into() })
            .unwrap();
        let r = s.solve();
        assert_eq!(r.status, SolveStatus::Solved, "{r:?}");
        assert_eq!(r.motion["free"].dof, 2);
        assert_eq!(r.motion["free"].direction, None);
        assert!(!r.motion.contains_key("pinned"));
        assert!(!r.motion.contains_key("a"));
        // `b` is level with `a`, so it can only slide along x.
        assert_eq!(r.motion["b"].dof, 1);
        let d = r.motion["b"].direction.unwrap();
        assert!((d[0] - 1.0).abs() < 1e-9 && d[1].abs() < 1e-9, "{d:?}");
    }

    #[test]
    fn a_point_on_a_slanted_line_moves_along_that_line() {
        let mut s = Sketch::new();
        pt(&mut s, "a", 0.0, 0.0);
        pt(&mut s, "b", 10.0, 10.0);
        pt(&mut s, "p", 4.0, 4.0);
        line(&mut s, "l", "a", "b");
        fix(&mut s, "fa", "a", 0.0, 0.0);
        fix(&mut s, "fb", "b", 10.0, 10.0);
        s.add_constraint(
            "on",
            PointOnLine {
                point: "p".into(),
                line: "l".into(),
            },
        )
        .unwrap();
        let r = s.solve();
        assert_eq!(r.motion["p"].dof, 1);
        let d = r.motion["p"].direction.unwrap();
        let k = core::f64::consts::FRAC_1_SQRT_2;
        assert!((d[0] - k).abs() < 1e-6 && (d[1] - k).abs() < 1e-6, "{d:?}");
    }

    #[test]
    fn a_fully_defined_sketch_has_no_motion_and_a_free_radius_is_listed() {
        let mut s = Sketch::new();
        pt(&mut s, "c", 0.0, 0.0);
        s.add_entity(
            "circ",
            Entity::Circle {
                center: "c".into(),
                radius: 3.0,
            },
        )
        .unwrap();
        fix(&mut s, "fc", "c", 0.0, 0.0);
        let r = s.solve();
        assert!(r.motion.is_empty());
        assert_eq!(r.radius_free, ["circ"]);
        s.add_constraint(
            "rad",
            Radius {
                target: "circ".into(),
                value: 3.0,
            },
        )
        .unwrap();
        let r = s.solve();
        assert!(r.radius_free.is_empty() && r.motion.is_empty() && r.dof == 0);
    }

    #[test]
    fn on_axis_puts_a_point_on_the_sketch_axis_without_an_origin_point() {
        let mut s = Sketch::new();
        pt(&mut s, "p", 3.0, 4.0);
        pt(&mut s, "q", 5.0, -2.0);
        s.add_constraint(
            "ax",
            OnAxis {
                point: "p".into(),
                axis: crate::SketchAxis::X,
            },
        )
        .unwrap();
        s.add_constraint(
            "ay",
            OnAxis {
                point: "q".into(),
                axis: crate::SketchAxis::Y,
            },
        )
        .unwrap();
        let r = s.solve();
        assert_eq!(r.status, SolveStatus::Solved, "{r:?}");
        assert_eq!(s.point("p").unwrap().1, 0.0);
        assert_eq!(s.point("q").unwrap().0, 0.0);
        assert_eq!(r.motion["p"].direction, Some([1.0, 0.0]));
        assert_eq!(r.motion["q"].direction, Some([0.0, 1.0]));
        assert!(s
            .add_constraint(
                "bad",
                OnAxis {
                    point: "nope".into(),
                    axis: crate::SketchAxis::X
                }
            )
            .is_err());
    }

    fn solved_ok(s: &mut Sketch) -> SolveReport {
        let r = s.solve();
        assert_eq!(r.status, SolveStatus::Solved, "{r:?}");
        r
    }

    #[test]
    fn midpoint_and_symmetric_place_points_exactly() {
        let mut s = Sketch::new();
        pt(&mut s, "a", 0.0, 0.0);
        pt(&mut s, "b", 10.0, 4.0);
        pt(&mut s, "m", 3.0, 3.0);
        line(&mut s, "l", "a", "b");
        fix(&mut s, "fa", "a", 0.0, 0.0);
        fix(&mut s, "fb", "b", 10.0, 4.0);
        s.add_constraint(
            "mid",
            Midpoint {
                point: "m".into(),
                line: "l".into(),
            },
        )
        .unwrap();
        // Mirror image of (3, 0) across the line from (0,0) to (10,4).
        pt(&mut s, "u", 3.0, 0.0);
        pt(&mut s, "v", 0.0, 1.0);
        fix(&mut s, "fu", "u", 3.0, 0.0);
        s.add_constraint(
            "sym",
            Symmetric {
                a: "u".into(),
                b: "v".into(),
                line: "l".into(),
            },
        )
        .unwrap();
        let r = solved_ok(&mut s);
        near(&s, "m", 5.0, 2.0);
        let (vx, vy) = s.point("v").unwrap();
        // Reflection of (3,0) across y = 0.4x.
        let (k, c) = (0.4_f64, 0.0);
        let d = (3.0 + k * (0.0 - c)) / (1.0 + k * k);
        let want = (2.0 * d - 3.0, 2.0 * d * k - 0.0);
        assert!(
            (vx - want.0).abs() < 1e-6 && (vy - want.1).abs() < 1e-6,
            "{vx} {vy} vs {want:?}"
        );
        assert_eq!(r.dof, 0);
    }

    #[test]
    fn equal_radius_and_point_on_curve_work_for_circles_and_arcs() {
        let mut s = Sketch::new();
        pt(&mut s, "c1", 0.0, 0.0);
        pt(&mut s, "c2", 20.0, 0.0);
        pt(&mut s, "as", 26.0, 0.0);
        pt(&mut s, "ae", 20.0, 6.0);
        pt(&mut s, "p", 3.0, 3.0);
        s.add_entity(
            "circ",
            Entity::Circle {
                center: "c1".into(),
                radius: 2.0,
            },
        )
        .unwrap();
        s.add_entity(
            "arc",
            Entity::Arc {
                center: "c2".into(),
                start: "as".into(),
                end: "ae".into(),
            },
        )
        .unwrap();
        fix(&mut s, "f1", "c1", 0.0, 0.0);
        fix(&mut s, "f2", "c2", 20.0, 0.0);
        fix(&mut s, "f3", "as", 26.0, 0.0);
        s.add_constraint(
            "eq",
            EqualRadius {
                a: "circ".into(),
                b: "arc".into(),
            },
        )
        .unwrap();
        s.add_constraint(
            "on",
            PointOnCurve {
                point: "p".into(),
                curve: "circ".into(),
            },
        )
        .unwrap();
        solved_ok(&mut s);
        let Some(Entity::Circle { radius, .. }) = s.entity("circ") else {
            panic!()
        };
        assert!((radius - 6.0).abs() < 1e-6, "{radius}");
        let (px, py) = s.point("p").unwrap();
        assert!((det::hypot(px, py) - 6.0).abs() < 1e-6);
        // Arc to arc.
        let mut t = Sketch::new();
        for (id, x, y) in [
            ("c1", 0.0, 0.0),
            ("s1", 5.0, 0.0),
            ("e1", 0.0, 5.0),
            ("c2", 20.0, 0.0),
            ("s2", 23.0, 0.0),
            ("e2", 20.0, 3.0),
        ] {
            pt(&mut t, id, x, y);
        }
        for (id, c, a, b) in [("a1", "c1", "s1", "e1"), ("a2", "c2", "s2", "e2")] {
            t.add_entity(
                id,
                Entity::Arc {
                    center: c.into(),
                    start: a.into(),
                    end: b.into(),
                },
            )
            .unwrap();
        }
        fix(&mut t, "g1", "c1", 0.0, 0.0);
        fix(&mut t, "g2", "s1", 5.0, 0.0);
        t.add_constraint(
            "eq",
            EqualRadius {
                a: "a1".into(),
                b: "a2".into(),
            },
        )
        .unwrap();
        solved_ok(&mut t);
        let (c, st) = (t.point("c2").unwrap(), t.point("s2").unwrap());
        assert!((det::hypot(st.0 - c.0, st.1 - c.1) - 5.0).abs() < 1e-6);
    }

    #[test]
    fn the_new_relations_refuse_nonsense() {
        let mut s = Sketch::new();
        pt(&mut s, "a", 0.0, 0.0);
        pt(&mut s, "b", 10.0, 0.0);
        pt(&mut s, "c", 5.0, 5.0);
        line(&mut s, "l", "a", "b");
        s.add_entity(
            "circ",
            Entity::Circle {
                center: "c".into(),
                radius: 2.0,
            },
        )
        .unwrap();
        let bad: Vec<Constraint> = vec![
            Midpoint {
                point: "a".into(),
                line: "l".into(),
            },
            Symmetric {
                a: "a".into(),
                b: "c".into(),
                line: "l".into(),
            },
            Symmetric {
                a: "c".into(),
                b: "c".into(),
                line: "l".into(),
            },
            EqualRadius {
                a: "circ".into(),
                b: "circ".into(),
            },
            EqualRadius {
                a: "circ".into(),
                b: "l".into(),
            },
            PointOnCurve {
                point: "c".into(),
                curve: "circ".into(),
            },
        ];
        for (i, c) in bad.into_iter().enumerate() {
            assert!(
                s.add_constraint(&format!("bad{i}"), c).is_err(),
                "constraint {i} should be refused"
            );
        }
    }

    #[test]
    fn a_local_drag_matches_the_full_drag_and_leaves_other_clusters_alone() {
        // Two independent rectangles; drag a corner of the first.
        let mut s = Sketch::new();
        for (r, ox) in [("a", 0.0), ("b", 50.0)] {
            let c = [(0.0, 0.0), (20.0, 0.0), (20.0, 10.0), (0.0, 10.0)];
            for (i, (x, y)) in c.iter().enumerate() {
                pt(&mut s, &format!("{r}p{i}"), ox + x, *y);
            }
            for i in 0..4 {
                let (p, q) = (format!("{r}p{i}"), format!("{r}p{}", (i + 1) % 4));
                line(&mut s, &format!("{r}l{i}"), &p, &q);
                let c = if i % 2 == 0 {
                    Horizontal {
                        line: format!("{r}l{i}"),
                    }
                } else {
                    Vertical {
                        line: format!("{r}l{i}"),
                    }
                };
                s.add_constraint(&format!("{r}c{i}"), c).unwrap();
            }
        }
        s.solve();
        let mut full = s.clone();
        let mut local = s.clone();
        full.drag_point("ap2", 35.0, 22.0).unwrap();
        assert!(local.drag_point_local("ap2", 35.0, 22.0).unwrap());
        assert_eq!(full, local, "same sketch either way");
        assert_eq!(
            local.point("bp2"),
            s.point("bp2"),
            "the other rectangle did not move"
        );
        assert!(local.drag_point_local("nope", 0.0, 0.0).is_err());
        assert!(local.drag_point_local("ap1", f64::NAN, 0.0).is_err());
    }

    fn plain_rectangle() -> Sketch {
        let mut s = Sketch::new();
        for (id, x, y) in [
            ("a", 0.0, 0.0),
            ("b", 40.0, 0.0),
            ("c", 40.0, 20.0),
            ("d", 0.0, 20.0),
        ] {
            pt(&mut s, id, x, y);
        }
        for (id, p, q) in [
            ("t", "a", "b"),
            ("r", "b", "c"),
            ("u", "c", "d"),
            ("l", "d", "a"),
        ] {
            line(&mut s, id, p, q);
        }
        for (id, l) in [("h1", "t"), ("h2", "u")] {
            s.add_constraint(id, Horizontal { line: l.into() }).unwrap();
        }
        for (id, l) in [("v1", "r"), ("v2", "l")] {
            s.add_constraint(id, Vertical { line: l.into() }).unwrap();
        }
        fix(&mut s, "fa", "a", 0.0, 0.0);
        s
    }

    #[test]
    fn dragging_an_edge_moves_it_and_the_rectangle_follows() {
        let mut s = plain_rectangle();
        s.solve();
        // The right edge (b, c) 10 mm to the right: the rectangle gets wider, still square-cornered.
        let t = |p: &str, x, y| DragTarget {
            point: p.into(),
            x,
            y,
        };
        let r = s
            .drag(&[t("b", 50.0, 0.0), t("c", 50.0, 20.0)], None)
            .unwrap();
        assert!(r.applied);
        near(&s, "b", 50.0, 0.0);
        near(&s, "c", 50.0, 20.0);
        near(&s, "d", 0.0, 20.0);
        // Dragging it diagonally: the edge can only move in x (it is vertical and its ends are
        // tied to horizontal edges), so it settles at the nearest valid shape.
        let mut s2 = plain_rectangle();
        s2.solve();
        let r = s2
            .drag(&[t("b", 50.0, 7.0), t("c", 50.0, 27.0)], None)
            .unwrap();
        assert!(r.applied);
        let (b, c) = (s2.point("b").unwrap(), s2.point("c").unwrap());
        assert!(
            (b.0 - 50.0).abs() < 1e-3 && (c.0 - 50.0).abs() < 1e-3,
            "{b:?} {c:?}"
        );
        assert!((b.0 - c.0).abs() < 1e-6, "still vertical");
    }

    #[test]
    fn dragging_a_circles_rim_sets_its_radius_unless_it_is_dimensioned() {
        let mut s = Sketch::new();
        pt(&mut s, "c", 0.0, 0.0);
        s.add_entity(
            "k",
            Entity::Circle {
                center: "c".into(),
                radius: 5.0,
            },
        )
        .unwrap();
        fix(&mut s, "fc", "c", 0.0, 0.0);
        let want = DragRadius {
            circle: "k".into(),
            value: 8.0,
        };
        assert!(s.drag(&[], Some(&want)).unwrap().applied);
        assert!(
            matches!(s.entity("k"), Some(Entity::Circle { radius, .. }) if (radius - 8.0).abs() < 1e-9)
        );
        s.add_constraint(
            "r",
            Radius {
                target: "k".into(),
                value: 8.0,
            },
        )
        .unwrap();
        // Dimensioned: the rim cannot be pulled; the circle snaps back to its dimension.
        let pull = DragRadius {
            circle: "k".into(),
            value: 12.0,
        };
        assert!(s.drag(&[], Some(&pull)).unwrap().applied);
        assert!(
            matches!(s.entity("k"), Some(Entity::Circle { radius, .. }) if (radius - 8.0).abs() < 1e-6)
        );
        // A local drag agrees, and bad targets are refused.
        let mut a = plain_rectangle();
        let mut b = a.clone();
        let t = [DragTarget {
            point: "c".into(),
            x: 55.0,
            y: 33.0,
        }];
        a.drag(&t, None).unwrap();
        assert!(b.drag_local(&t, None).unwrap());
        assert_eq!(a, b);
        assert!(
            b.drag_local(&[], Some(&pull)).is_err(),
            "not a circle in this sketch"
        );
        let nan = DragRadius {
            circle: "k".into(),
            value: f64::NAN,
        };
        assert!(s.drag(&[], Some(&nan)).is_err());
    }

    #[test]
    fn an_angle_between_lines_is_held_and_keeps_its_side() {
        let mut s = Sketch::new();
        pt(&mut s, "o", 0.0, 0.0);
        pt(&mut s, "a", 10.0, 0.0);
        pt(&mut s, "b", 7.0, 7.0);
        line(&mut s, "u", "o", "a");
        line(&mut s, "v", "o", "b");
        fix(&mut s, "fo", "o", 0.0, 0.0);
        fix(&mut s, "fa", "a", 10.0, 0.0);
        s.add_constraint(
            "ang",
            Angle {
                a: "u".into(),
                b: "v".into(),
                value: 60.0,
            },
        )
        .unwrap();
        solved_ok(&mut s);
        let (bx, by) = s.point("b").unwrap();
        let phi = det::atan2(by, bx).to_degrees();
        assert!((phi - 60.0).abs() < 1e-6, "{phi}");
        // Editing the value keeps the side (counter-clockwise stays counter-clockwise).
        s.set_constraint_value("ang", 120.0).unwrap();
        solved_ok(&mut s);
        let (bx, by) = s.point("b").unwrap();
        assert!((det::atan2(by, bx).to_degrees() - 120.0).abs() < 1e-6);
        // A negative (clockwise) angle stays clockwise.
        let mut t = Sketch::new();
        pt(&mut t, "o", 0.0, 0.0);
        pt(&mut t, "a", 10.0, 0.0);
        pt(&mut t, "b", 7.0, -7.0);
        line(&mut t, "u", "o", "a");
        line(&mut t, "v", "o", "b");
        fix(&mut t, "fo", "o", 0.0, 0.0);
        fix(&mut t, "fa", "a", 10.0, 0.0);
        t.add_constraint(
            "ang",
            Angle {
                a: "u".into(),
                b: "v".into(),
                value: -30.0,
            },
        )
        .unwrap();
        solved_ok(&mut t);
        let (bx, by) = t.point("b").unwrap();
        assert!((det::atan2(by, bx).to_degrees() + 30.0).abs() < 1e-6);
        t.set_constraint_value("ang", 45.0).unwrap();
        assert!(matches!(t.constraints()["ang"], Angle { value, .. } if value == -45.0));
        // 0 and 180 are what parallel is for; a line cannot make an angle with itself.
        for bad in [0.0, 180.0, 200.0] {
            assert!(t
                .add_constraint(
                    "x",
                    Angle {
                        a: "u".into(),
                        b: "v".into(),
                        value: bad
                    }
                )
                .is_err());
        }
        assert!(t.set_constraint_value("ang", 180.0).is_err());
        assert!(t
            .add_constraint(
                "x",
                Angle {
                    a: "u".into(),
                    b: "u".into(),
                    value: 30.0
                }
            )
            .is_err());
    }

    #[test]
    fn symmetric_about_an_axis_and_tangent_curves() {
        let mut s = Sketch::new();
        pt(&mut s, "a", 4.0, 3.0);
        pt(&mut s, "b", 5.0, -2.0);
        fix(&mut s, "fa", "a", 4.0, 3.0);
        s.add_constraint(
            "sx",
            SymmetricAxis {
                a: "a".into(),
                b: "b".into(),
                axis: crate::SketchAxis::X,
            },
        )
        .unwrap();
        solved_ok(&mut s);
        near(&s, "b", 4.0, -3.0);
        let mut t = Sketch::new();
        pt(&mut t, "a", 4.0, 3.0);
        pt(&mut t, "b", 1.0, 8.0);
        fix(&mut t, "fa", "a", 4.0, 3.0);
        t.add_constraint(
            "sy",
            SymmetricAxis {
                a: "a".into(),
                b: "b".into(),
                axis: crate::SketchAxis::Y,
            },
        )
        .unwrap();
        solved_ok(&mut t);
        near(&t, "b", -4.0, 3.0);

        // Two circles: outside each other they end a radius sum apart; the branch is chosen
        // from where they are now. A small circle inside a big one touches internally.
        let mut c = Sketch::new();
        pt(&mut c, "c1", 0.0, 0.0);
        pt(&mut c, "c2", 12.0, 1.0);
        c.add_entity(
            "k1",
            Entity::Circle {
                center: "c1".into(),
                radius: 5.0,
            },
        )
        .unwrap();
        c.add_entity(
            "k2",
            Entity::Circle {
                center: "c2".into(),
                radius: 3.0,
            },
        )
        .unwrap();
        fix(&mut c, "f1", "c1", 0.0, 0.0);
        c.add_constraint(
            "t",
            TangentCurves {
                a: "k1".into(),
                b: "k2".into(),
            },
        )
        .unwrap();
        c.add_constraint(
            "r1",
            Radius {
                target: "k1".into(),
                value: 5.0,
            },
        )
        .unwrap();
        c.add_constraint(
            "r2",
            Radius {
                target: "k2".into(),
                value: 3.0,
            },
        )
        .unwrap();
        solved_ok(&mut c);
        let (x, y) = c.point("c2").unwrap();
        assert!((det::hypot(x, y) - 8.0).abs() < 1e-6, "external: {x} {y}");
        let mut i = Sketch::new();
        pt(&mut i, "c1", 0.0, 0.0);
        pt(&mut i, "c2", 3.5, 0.5);
        i.add_entity(
            "k1",
            Entity::Circle {
                center: "c1".into(),
                radius: 10.0,
            },
        )
        .unwrap();
        i.add_entity(
            "k2",
            Entity::Circle {
                center: "c2".into(),
                radius: 3.0,
            },
        )
        .unwrap();
        fix(&mut i, "f1", "c1", 0.0, 0.0);
        i.add_constraint(
            "t",
            TangentCurves {
                a: "k1".into(),
                b: "k2".into(),
            },
        )
        .unwrap();
        i.add_constraint(
            "r1",
            Radius {
                target: "k1".into(),
                value: 10.0,
            },
        )
        .unwrap();
        i.add_constraint(
            "r2",
            Radius {
                target: "k2".into(),
                value: 3.0,
            },
        )
        .unwrap();
        solved_ok(&mut i);
        let (x, y) = i.point("c2").unwrap();
        assert!((det::hypot(x, y) - 7.0).abs() < 1e-6, "internal: {x} {y}");
        // Circle and arc, either order, and two arcs.
        for flip in [false, true] {
            let mut m = Sketch::new();
            pt(&mut m, "c1", 0.0, 0.0);
            pt(&mut m, "c2", 11.0, 0.0);
            pt(&mut m, "s", 11.0, 4.0);
            pt(&mut m, "e", 15.0, 0.0);
            m.add_entity(
                "k",
                Entity::Circle {
                    center: "c1".into(),
                    radius: 5.0,
                },
            )
            .unwrap();
            m.add_entity(
                "a",
                Entity::Arc {
                    center: "c2".into(),
                    start: "s".into(),
                    end: "e".into(),
                },
            )
            .unwrap();
            fix(&mut m, "f1", "c1", 0.0, 0.0);
            fix(&mut m, "f2", "s", 11.0, 4.0);
            let (p, q) = if flip { ("a", "k") } else { ("k", "a") };
            m.add_constraint(
                "t",
                TangentCurves {
                    a: p.into(),
                    b: q.into(),
                },
            )
            .unwrap();
            solved_ok(&mut m);
            let (c2, st) = (m.point("c2").unwrap(), m.point("s").unwrap());
            let r = det::hypot(st.0 - c2.0, st.1 - c2.1);
            let Some(Entity::Circle { radius: r1, .. }) = m.entity("k").cloned() else {
                panic!()
            };
            assert!(
                (det::hypot(c2.0, c2.1) - (r1 + r)).abs() < 1e-6,
                "flip {flip}"
            );
        }
        assert!(c
            .add_constraint(
                "bad",
                TangentCurves {
                    a: "k1".into(),
                    b: "k1".into()
                }
            )
            .is_err());
    }

    #[test]
    fn whole_lines_and_arcs_report_how_they_can_still_move() {
        let mut s = Sketch::new();
        // A free line, a line pinned at both ends, and a line pinned at one end.
        for (id, x, y) in [
            ("a", 0.0, 0.0),
            ("b", 10.0, 0.0),
            ("c", 0.0, 5.0),
            ("d", 10.0, 5.0),
            ("e", 0.0, 9.0),
            ("f", 10.0, 9.0),
        ] {
            pt(&mut s, id, x, y);
        }
        line(&mut s, "free", "a", "b");
        line(&mut s, "pinned", "c", "d");
        line(&mut s, "swing", "e", "f");
        fix(&mut s, "fc", "c", 0.0, 5.0);
        fix(&mut s, "fd", "d", 10.0, 5.0);
        fix(&mut s, "fe", "e", 0.0, 9.0);
        let r = solved_ok(&mut s);
        assert_eq!(r.shape_motion["free"].dof, 2, "a free line moves any way");
        assert!(!r.shape_motion.contains_key("pinned"));
        assert_eq!(
            r.shape_motion["swing"].dof, 2,
            "its middle swings round the fixed end"
        );
        // Hold the swinging line level: its middle can only slide with the free end, along x.
        s.add_constraint(
            "h",
            Horizontal {
                line: "swing".into(),
            },
        )
        .unwrap();
        let r = solved_ok(&mut s);
        let m = r.shape_motion["swing"];
        assert_eq!((m.dof, m.direction), (1, Some([1.0, 0.0])));

        // Arcs: radius and sweep are separate freedoms.
        let mut t = Sketch::new();
        pt(&mut t, "c", 0.0, 0.0);
        pt(&mut t, "s", 10.0, 0.0);
        pt(&mut t, "e", 0.0, 10.0);
        t.add_entity(
            "arc",
            Entity::Arc {
                center: "c".into(),
                start: "s".into(),
                end: "e".into(),
            },
        )
        .unwrap();
        fix(&mut t, "fc", "c", 0.0, 0.0);
        fix(&mut t, "fs", "s", 10.0, 0.0);
        let r = solved_ok(&mut t);
        assert!(
            r.sweep_free.contains(&"arc".to_string()),
            "the end can go round: {r:?}"
        );
        assert!(
            !r.radius_free.contains(&"arc".to_string()),
            "start and centre fix the radius"
        );
        // Pin the end as well: nothing is free.
        fix(&mut t, "fe", "e", 0.0, 10.0);
        let r = solved_ok(&mut t);
        assert!(r.sweep_free.is_empty() && r.radius_free.is_empty());
        // A free arc has both.
        let mut u = Sketch::new();
        pt(&mut u, "c", 0.0, 0.0);
        pt(&mut u, "s", 10.0, 0.0);
        pt(&mut u, "e", 0.0, 10.0);
        u.add_entity(
            "arc",
            Entity::Arc {
                center: "c".into(),
                start: "s".into(),
                end: "e".into(),
            },
        )
        .unwrap();
        fix(&mut u, "fc", "c", 0.0, 0.0);
        let r = solved_ok(&mut u);
        assert!(
            r.sweep_free.contains(&"arc".to_string()) && r.radius_free.contains(&"arc".to_string())
        );
    }

    #[test]
    fn an_arc_length_is_held_for_short_and_long_arcs() {
        // A quarter arc of radius 10 (length ~15.708) stretched to length 20, with the centre and
        // start fixed so only the end can go round.
        for (end, want_len) in [
            ((0.0, 10.0), 20.0),
            ((-10.0, 0.0), 12.0),
            ((0.0, -10.0), 30.0),
        ] {
            let mut s = Sketch::new();
            pt(&mut s, "c", 0.0, 0.0);
            pt(&mut s, "s", 10.0, 0.0);
            pt(&mut s, "e", end.0, end.1);
            s.add_entity(
                "arc",
                Entity::Arc {
                    center: "c".into(),
                    start: "s".into(),
                    end: "e".into(),
                },
            )
            .unwrap();
            fix(&mut s, "fc", "c", 0.0, 0.0);
            fix(&mut s, "fs", "s", 10.0, 0.0);
            s.add_constraint(
                "len",
                ArcLength {
                    arc: "arc".into(),
                    value: want_len,
                },
            )
            .unwrap();
            solved_ok(&mut s);
            let (ex, ey) = s.point("e").unwrap();
            let sweep = {
                let t = core::f64::consts::TAU;
                let d = det::atan2(ey, ex) % t;
                if d < 0.0 {
                    d + t
                } else {
                    d
                }
            };
            assert!(
                (10.0 * sweep - want_len).abs() < 1e-6,
                "{end:?}: {} vs {want_len}",
                10.0 * sweep
            );
            assert!(
                (det::hypot(ex, ey) - 10.0).abs() < 1e-6,
                "still on the circle"
            );
        }
    }

    #[test]
    fn a_reference_dimension_measures_without_constraining_and_cannot_be_edited() {
        let mut s = Sketch::new();
        pt(&mut s, "a", 0.0, 0.0);
        pt(&mut s, "b", 10.0, 0.0);
        line(&mut s, "l", "a", "b");
        fix(&mut s, "fa", "a", 0.0, 0.0);
        s.add_constraint("h", Horizontal { line: "l".into() })
            .unwrap();
        s.add_constraint(
            "len",
            Length {
                line: "l".into(),
                value: 99.0,
            },
        )
        .unwrap();
        // As a reference it does not pull the line to 99.
        s.set_driven("len", true).unwrap();
        let r = solved_ok(&mut s);
        assert_eq!(r.dof, 1, "the length is still free: {r:?}");
        near(&s, "b", 10.0, 0.0);
        assert!(s.is_driven("len"));
        // It cannot be edited, nor made a reference if it is a relation.
        assert!(s.set_constraint_value("len", 5.0).is_err());
        assert!(s.set_driven("h", true).is_err());
        assert!(s.set_driven("nope", true).is_err());
        // Back to driving, it constrains again.
        s.set_driven("len", false).unwrap();
        let r = solved_ok(&mut s);
        assert_eq!(r.dof, 0);
        near(&s, "b", 99.0, 0.0);
        // Its flag goes with it, and an empty set does not appear in the saved form.
        s.set_driven("len", true).unwrap();
        s.remove_constraint("len").unwrap();
        assert!(s.driven().is_empty());
        assert!(!serde_json::to_string(&s).unwrap().contains("driven"));
    }

    #[test]
    fn dimension_styles_are_presentation_only_and_a_default_style_leaves_no_trace() {
        let mut s = Sketch::new();
        pt(&mut s, "a", 0.0, 0.0);
        pt(&mut s, "b", 10.0, 0.0);
        line(&mut s, "l", "a", "b");
        s.add_constraint(
            "len",
            Length {
                line: "l".into(),
                value: 10.0,
            },
        )
        .unwrap();
        let plain = serde_json::to_string(&s).unwrap();
        s.set_style(
            "len",
            crate::DimStyle {
                decimals: Some(3),
                tol_plus: 0.1,
                tol_minus: 0.05,
            },
        )
        .unwrap();
        assert_eq!(s.style("len").unwrap().decimals, Some(3));
        let back: Sketch = serde_json::from_str(&serde_json::to_string(&s).unwrap()).unwrap();
        assert_eq!(back, s);
        s.set_style("len", crate::DimStyle::default()).unwrap();
        assert_eq!(
            serde_json::to_string(&s).unwrap(),
            plain,
            "back to nothing stored"
        );
        // Refusals.
        assert!(s.set_style("nope", crate::DimStyle::default()).is_err());
        let bad = crate::DimStyle {
            decimals: Some(12),
            ..Default::default()
        };
        assert!(s.set_style("len", bad).is_err());
        let nan = crate::DimStyle {
            tol_plus: f64::NAN,
            ..Default::default()
        };
        assert!(s.set_style("len", nan).is_err());
    }

    #[test]
    fn free_construction_geometry_makes_the_sketch_under_defined_but_never_the_shape() {
        let mut s = sloppy_rectangle();
        assert!(s.solve().is_fully_constrained());
        // A loose guide line far from the part: two free points and a line.
        pt(&mut s, "g1", 200.0, 0.0);
        pt(&mut s, "g2", 230.0, 20.0);
        line(&mut s, "guide", "g1", "g2");
        let r = s.solve();
        assert_eq!(
            r.dof, 4,
            "an ordinary loose line is four free coordinates: {r:?}"
        );
        assert_eq!(
            r.shape_dof, None,
            "as a real shape it is part of the shapes"
        );
        // As construction geometry the sketch is still under defined (as in SolidWorks), and
        // the guide shows as free, but the shape itself is defined.
        s.set_construction("guide", true).unwrap();
        let r = s.solve();
        assert_eq!(r.dof, 4, "{r:?}");
        assert!(!r.is_fully_constrained());
        assert_eq!(
            r.shape_dof,
            Some(0),
            "the real shapes are fully defined: {r:?}"
        );
        assert!(r.under_constrained.contains(&"guide".to_owned()));
        assert!(
            !r.under_constrained
                .iter()
                .any(|e| e.starts_with('l') && e != "guide" || e.starts_with('p')),
            "no part of the rectangle is free: {:?}",
            r.under_constrained
        );
        assert!(r.motion.contains_key("g1") && r.shape_motion.contains_key("guide"));
        // Pinning the guide down (horizontal, one end fixed, a length) defines everything.
        s.add_constraint(
            "hg",
            Horizontal {
                line: "guide".into(),
            },
        )
        .unwrap();
        s.add_constraint(
            "fg",
            Fixed {
                point: "g1".into(),
                x: 200.0,
                y: 0.0,
            },
        )
        .unwrap();
        s.add_constraint(
            "lg",
            Length {
                line: "guide".into(),
                value: 30.0,
            },
        )
        .unwrap();
        let r = s.solve();
        assert!(r.is_fully_constrained(), "{r:?}");
        assert_eq!(r.shape_dof, None);
    }

    #[test]
    fn construction_geometry_that_the_part_depends_on_still_counts() {
        // A centre line that the profile is tied to: the profile's own freedom is what shows.
        let mut s = Sketch::new();
        for (id, x, y) in [("a", 0.0, 0.0), ("b", 40.0, 0.0), ("m", 20.0, 0.0)] {
            pt(&mut s, id, x, y);
        }
        pt(&mut s, "c", 20.0, 20.0);
        line(&mut s, "base", "a", "b");
        line(&mut s, "axis", "m", "c");
        s.set_construction("axis", true).unwrap();
        s.add_constraint(
            "mid",
            Midpoint {
                point: "m".into(),
                line: "base".into(),
            },
        )
        .unwrap();
        let r = s.solve();
        // The base line (4) is free; m follows it. The axis end c is construction only: it is
        // free too (2 more) as far as the sketch goes, but the shapes' own freedom is the base's.
        assert_eq!(r.dof, 6, "{r:?}");
        assert_eq!(r.shape_dof, Some(4), "{r:?}");
        assert!(r.under_constrained.contains(&"base".to_owned()));
    }

    #[test]
    fn an_infinite_construction_line_only_has_to_be_placed_not_sized() {
        let mut s = sloppy_rectangle();
        assert!(s.solve().is_fully_constrained());
        pt(&mut s, "g1", -30.0, 0.0);
        pt(&mut s, "g2", 90.0, 0.0);
        line(&mut s, "centre", "g1", "g2");
        // Only construction lines can be infinite.
        assert!(s.set_infinite("centre", true).is_err());
        s.set_construction("centre", true).unwrap();
        // Finite and loose: four free coordinates, so the sketch is under defined.
        assert_eq!(s.solve().dof, 4);
        s.set_infinite("centre", true).unwrap();
        // Infinite and loose: its direction and offset are still open, its reach is not a thing.
        let r = s.solve();
        assert_eq!(r.dof, 2, "{r:?}");
        assert!(r.under_constrained.contains(&"centre".to_owned()));
        assert!(
            !r.under_constrained.contains(&"g1".to_owned()),
            "its ends are nobody's concern"
        );
        assert!(!r.shape_motion.contains_key("centre") && !r.motion.contains_key("g1"));
        // Lying on the horizontal axis places it completely, wherever its ends are.
        s.add_constraint(
            "a1",
            OnAxis {
                point: "g1".into(),
                axis: crate::SketchAxis::X,
            },
        )
        .unwrap();
        assert_eq!(
            s.solve().dof,
            1,
            "direction still open with one end on the axis"
        );
        s.add_constraint(
            "a2",
            OnAxis {
                point: "g2".into(),
                axis: crate::SketchAxis::X,
            },
        )
        .unwrap();
        let r = s.solve();
        assert_eq!(r.dof, 0, "{r:?}");
        assert!(r.is_fully_constrained());
        assert!(r.under_constrained.is_empty());
        // Finite again, the same line asks for its ends to be placed.
        s.set_infinite("centre", false).unwrap();
        assert_eq!(s.solve().dof, 2);
        // Making a line normal geometry again clears the flag.
        s.set_infinite("centre", true).unwrap();
        s.set_construction("centre", false).unwrap();
        assert!(!s.is_infinite("centre"));
    }
}
