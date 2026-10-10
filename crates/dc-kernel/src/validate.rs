//! Solid validity checks. Run on every result a construction produces ("never corrupt, always
//! explain", AGENTS.md §11): a solid that fails here is a bug in the construction, reported with
//! what is wrong, never passed on.

use crate::solid::{CoEdge, Curve, Solid};
use dc_math::Tolerance;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum IssueCode {
    /// An index points outside its array, or a coordinate is NaN/infinite.
    Malformed,
    /// A face loop is empty or its edges do not join up end to end.
    OpenLoop,
    /// An edge is not used exactly once in each direction (not a closed, consistently oriented
    /// 2-manifold).
    EdgeUse,
    /// An edge's endpoints do not match its curve, or it has no length.
    EdgeGeometry,
    /// A vertex or edge of a face does not lie on that face's surface.
    OffSurface,
    /// Euler–Poincaré: V − E + F − R = 2(S − G) has no non-negative integer genus.
    Euler,
    /// Negative or zero volume (inside out), or a face with non-positive area.
    Orientation,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Issue {
    pub code: IssueCode,
    pub message: String,
}

fn issue(code: IssueCode, message: String) -> Issue {
    Issue { code, message }
}

impl Solid {
    /// Number of separate shells (faces joined through shared edges).
    pub fn shell_count(&self) -> usize {
        let nf = self.faces.len();
        let mut parent: Vec<usize> = (0..nf).collect();
        fn find(p: &mut [usize], x: usize) -> usize {
            let mut r = x;
            while p[r] != r {
                r = p[r];
            }
            let mut c = x;
            while p[c] != r {
                let n = p[c];
                p[c] = r;
                c = n;
            }
            r
        }
        let mut owner: Vec<Option<usize>> = vec![None; self.edges.len()];
        for (fi, f) in self.faces.iter().enumerate() {
            for c in f.loops.iter().flatten() {
                match owner[c.edge as usize] {
                    None => owner[c.edge as usize] = Some(fi),
                    Some(o) => {
                        let (a, b) = (find(&mut parent, o), find(&mut parent, fi));
                        parent[a] = b;
                    }
                }
            }
        }
        (0..nf).filter(|&i| find(&mut parent, i) == i).count()
    }

    /// All problems found; empty means the solid is valid.
    pub fn validate(&self) -> Vec<Issue> {
        let mut out = Vec::new();
        let tol = Tolerance::DEFAULT.linear;
        let (nv, ne) = (self.vertices.len(), self.edges.len());

        // Malformed data stops everything else: later checks index freely.
        for (i, p) in self.vertices.iter().enumerate() {
            if !(p.x.is_finite() && p.y.is_finite() && p.z.is_finite()) {
                out.push(issue(
                    IssueCode::Malformed,
                    format!("vertex {i} is not finite"),
                ));
            }
        }
        for (i, e) in self.edges.iter().enumerate() {
            if e.v0 as usize >= nv || e.v1 as usize >= nv {
                out.push(issue(
                    IssueCode::Malformed,
                    format!("edge '{}' ({i}) refers to a missing vertex", e.name),
                ));
            }
        }
        for f in &self.faces {
            for lp in &f.loops {
                for c in lp {
                    if c.edge as usize >= ne {
                        out.push(issue(
                            IssueCode::Malformed,
                            format!("face '{}' refers to a missing edge", f.name),
                        ));
                    }
                }
            }
        }
        if !out.is_empty() {
            return out;
        }

        // Edge geometry.
        for e in &self.edges {
            let (a, b) = (self.vertex(e.v0), self.vertex(e.v1));
            match &e.curve {
                Curve::Line => {
                    if (b - a).length() <= tol {
                        out.push(issue(
                            IssueCode::EdgeGeometry,
                            format!("line edge '{}' has no length", e.name),
                        ));
                    }
                }
                c => {
                    let (t0, sweep) = c.range().expect("curved edge");
                    let (p0, p1) = (
                        c.circle_point(t0).expect("curved"),
                        c.circle_point(t0 + sweep).expect("curved"),
                    );
                    if c.max_radius() <= tol || sweep <= 0.0 {
                        out.push(issue(
                            IssueCode::EdgeGeometry,
                            format!("curved edge '{}' is degenerate", e.name),
                        ));
                    }
                    if (p0 - a).length() > 1e-6 || (p1 - b).length() > 1e-6 {
                        out.push(issue(
                            IssueCode::EdgeGeometry,
                            format!("curved edge '{}' does not end at its vertices", e.name),
                        ));
                    }
                }
            }
        }

        // Loops are closed chains.
        for f in &self.faces {
            if f.loops.is_empty() {
                out.push(issue(
                    IssueCode::OpenLoop,
                    format!("face '{}' has no boundary", f.name),
                ));
            }
            for lp in &f.loops {
                if lp.is_empty() {
                    out.push(issue(
                        IssueCode::OpenLoop,
                        format!("face '{}' has an empty loop", f.name),
                    ));
                    continue;
                }
                for (k, &c) in lp.iter().enumerate() {
                    let next: CoEdge = lp[(k + 1) % lp.len()];
                    if self.coedge_ends(c).1 != self.coedge_ends(next).0 {
                        out.push(issue(
                            IssueCode::OpenLoop,
                            format!(
                                "face '{}': edges {k} and {} do not join",
                                f.name,
                                (k + 1) % lp.len()
                            ),
                        ));
                    }
                }
            }
        }

        // Every edge is used exactly twice, once in each direction.
        let mut fwd = vec![0u32; ne];
        let mut rev = vec![0u32; ne];
        for f in &self.faces {
            for c in f.loops.iter().flatten() {
                if c.reversed {
                    rev[c.edge as usize] += 1;
                } else {
                    fwd[c.edge as usize] += 1;
                }
            }
        }
        for (i, e) in self.edges.iter().enumerate() {
            if fwd[i] != 1 || rev[i] != 1 {
                out.push(issue(
                    IssueCode::EdgeUse,
                    format!("edge '{}' is used {} time(s) forward and {} backward (needs exactly 1 and 1)", e.name, fwd[i], rev[i]),
                ));
            }
        }

        // Boundaries lie on their surfaces.
        for f in &self.faces {
            for c in f.loops.iter().flatten() {
                for t in [0.0, 0.5, 1.0] {
                    let p = self.edge_point(c.edge, t);
                    let d = f.surface.deviation(p);
                    if d > 1e-6 {
                        out.push(issue(
                            IssueCode::OffSurface,
                            format!(
                                "edge '{}' is {d:.3e} off face '{}'",
                                self.edges[c.edge as usize].name, f.name
                            ),
                        ));
                        break;
                    }
                }
            }
        }

        // Euler–Poincaré, with shells found by face adjacency through shared edges.
        let nf = self.faces.len();
        let mut parent: Vec<usize> = (0..nf).collect();
        fn find(p: &mut [usize], x: usize) -> usize {
            let mut r = x;
            while p[r] != r {
                r = p[r];
            }
            let mut c = x;
            while p[c] != r {
                let n = p[c];
                p[c] = r;
                c = n;
            }
            r
        }
        let mut owner: Vec<Option<usize>> = vec![None; ne];
        for (fi, f) in self.faces.iter().enumerate() {
            for c in f.loops.iter().flatten() {
                match owner[c.edge as usize] {
                    None => owner[c.edge as usize] = Some(fi),
                    Some(o) => {
                        let (a, b) = (find(&mut parent, o), find(&mut parent, fi));
                        parent[a] = b;
                    }
                }
            }
        }
        let shells = (0..nf).filter(|&i| find(&mut parent, i) == i).count() as i64;
        let rings: i64 = self
            .faces
            .iter()
            .map(|f| f.loops.len().saturating_sub(1) as i64)
            .sum();
        let chi = nv as i64 - ne as i64 + nf as i64 - rings;
        if chi % 2 != 0 || shells - chi / 2 < 0 {
            out.push(issue(
                IssueCode::Euler,
                format!("V−E+F−R = {chi} with {shells} shell(s) gives no valid genus (V={nv}, E={ne}, F={nf}, R={rings})"),
            ));
        }

        // Orientation.
        for f in &self.faces {
            let a = self.face_area(f);
            if a <= 0.0 {
                out.push(issue(
                    IssueCode::Orientation,
                    format!("face '{}' has non-positive area {a}", f.name),
                ));
            }
        }
        let v = self.volume();
        if v <= 0.0 {
            out.push(issue(
                IssueCode::Orientation,
                format!("volume is {v}: the solid is inside out or empty"),
            ));
        }
        out
    }
}
