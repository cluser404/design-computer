//! Sparse symmetric positive-definite solves for the constraint solver.
//!
//! The Newton step of the solver solves `(JᵀJ + λI) δ = -Jᵀr`. Each constraint touches only a few
//! variables, so the matrix is sparse: its pattern is the graph "two variables are linked if some
//! constraint uses both". A dense factorisation costs n³; here the variables are first reordered
//! with reverse Cuthill–McKee (which keeps linked variables close together), and a *profile*
//! (envelope, skyline) Cholesky factorisation stores and computes only from each row's first
//! linked column to the diagonal. Fill-in stays inside that envelope, so for the chain- and
//! grid-like graphs sketches make, the cost is roughly n times the squared bandwidth.
//!
//! Everything is deterministic: ordering ties are broken by index, the arithmetic is plain IEEE
//! in a fixed order.

use dc_math::det;

/// The ordering and storage layout for one sparsity pattern.
pub(crate) struct Profile {
    n: usize,
    /// `new -> old` variable index.
    perm: Vec<usize>,
    /// `old -> new`.
    inv: Vec<usize>,
    /// First stored column of each (new) row.
    first: Vec<usize>,
    /// Start of each row in the value array (row `i` holds columns `first[i]..=i`).
    offset: Vec<usize>,
    size: usize,
}

impl Profile {
    /// Builds the layout for `n` variables where each pair in `links` (old indices) is coupled.
    pub(crate) fn new(n: usize, links: &[(usize, usize)]) -> Profile {
        let mut adj: Vec<Vec<usize>> = vec![Vec::new(); n];
        for &(a, b) in links {
            if a != b {
                adj[a].push(b);
                adj[b].push(a);
            }
        }
        for l in &mut adj {
            l.sort_unstable();
            l.dedup();
        }
        let perm = reverse_cuthill_mckee(&adj);
        let mut inv = vec![0; n];
        for (new, &old) in perm.iter().enumerate() {
            inv[old] = new;
        }
        let mut first: Vec<usize> = (0..n).collect();
        for (old, nbrs) in adj.iter().enumerate() {
            let i = inv[old];
            for &q in nbrs {
                let j = inv[q];
                if j < first[i] {
                    first[i] = j;
                }
            }
        }
        let mut offset = Vec::with_capacity(n);
        let mut size = 0;
        for (i, f) in first.iter().enumerate() {
            offset.push(size);
            size += i - f + 1;
        }
        Profile {
            n,
            perm,
            inv,
            first,
            offset,
            size,
        }
    }

    /// A zeroed value array for this layout.
    pub(crate) fn zeros(&self) -> Vec<f64> {
        vec![0.0; self.size]
    }

    /// Number of stored entries (a measure of the cost).
    #[cfg(test)]
    pub(crate) fn stored(&self) -> usize {
        self.size
    }

    /// Adds `v` to the entry at old indices `(a, b)` (and so to its mirror).
    pub(crate) fn add(&self, vals: &mut [f64], a: usize, b: usize, v: f64) {
        let (mut i, mut j) = (self.inv[a], self.inv[b]);
        if j > i {
            core::mem::swap(&mut i, &mut j);
        }
        debug_assert!(j >= self.first[i], "entry outside the declared pattern");
        vals[self.offset[i] + j - self.first[i]] += v;
    }

    /// Adds `v` to every diagonal entry.
    pub(crate) fn add_diagonal(&self, vals: &mut [f64], v: f64) {
        for i in 0..self.n {
            vals[self.offset[i] + i - self.first[i]] += v;
        }
    }

    /// Factors the matrix in `vals` in place (Cholesky, `L Lᵀ`) and solves for the right-hand
    /// side `b` (old order), leaving the solution in `b`. `false` if the matrix is not positive
    /// definite (then `b` is unspecified).
    pub(crate) fn solve(&self, vals: &mut [f64], b: &mut [f64]) -> bool {
        let n = self.n;
        for i in 0..n {
            let fi = self.first[i];
            let oi = self.offset[i];
            for j in fi..=i {
                let fj = self.first[j];
                let oj = self.offset[j];
                let mut sum = vals[oi + j - fi];
                // The overlap of the two rows' stored ranges, strictly left of column j.
                for k in fi.max(fj)..j {
                    sum -= vals[oi + k - fi] * vals[oj + k - fj];
                }
                if i == j {
                    if sum <= 0.0 || !sum.is_finite() {
                        return false;
                    }
                    vals[oi + i - fi] = det::sqrt(sum);
                } else {
                    vals[oi + j - fi] = sum / vals[oj + j - fj];
                }
            }
        }
        // Forward: L y = P b.
        let mut y: Vec<f64> = self.perm.iter().map(|&old| b[old]).collect();
        for i in 0..n {
            let (fi, oi) = (self.first[i], self.offset[i]);
            let mut sum = y[i];
            for k in fi..i {
                sum -= vals[oi + k - fi] * y[k];
            }
            y[i] = sum / vals[oi + i - fi];
        }
        // Backward: Lᵀ z = y, column by column.
        for i in (0..n).rev() {
            let (fi, oi) = (self.first[i], self.offset[i]);
            y[i] /= vals[oi + i - fi];
            let yi = y[i];
            for k in fi..i {
                y[k] -= vals[oi + k - fi] * yi;
            }
        }
        for (new, &old) in self.perm.iter().enumerate() {
            b[old] = y[new];
        }
        true
    }
}

/// Reverse Cuthill–McKee ordering (`new -> old`), starting each connected component from a
/// pseudo-peripheral vertex. Neighbours are visited by (degree, index), so it is deterministic.
fn reverse_cuthill_mckee(adj: &[Vec<usize>]) -> Vec<usize> {
    let n = adj.len();
    let mut visited = vec![false; n];
    let mut order = Vec::with_capacity(n);
    // Vertices by degree then index, as start candidates.
    let mut starts: Vec<usize> = (0..n).collect();
    starts.sort_by_key(|&v| (adj[v].len(), v));
    for &s in &starts {
        if visited[s] {
            continue;
        }
        let root = pseudo_peripheral(adj, s, &visited);
        let mut queue = std::collections::VecDeque::new();
        visited[root] = true;
        queue.push_back(root);
        while let Some(v) = queue.pop_front() {
            order.push(v);
            let mut next: Vec<usize> = adj[v].iter().copied().filter(|&w| !visited[w]).collect();
            next.sort_by_key(|&w| (adj[w].len(), w));
            for w in next {
                visited[w] = true;
                queue.push_back(w);
            }
        }
    }
    order.reverse();
    order
}

/// A vertex far from `start` in its component: repeatedly jump to a farthest vertex of least
/// degree until the distance stops growing.
fn pseudo_peripheral(adj: &[Vec<usize>], start: usize, taken: &[bool]) -> usize {
    let mut root = start;
    let mut depth = 0;
    loop {
        let (last_level, d) = farthest(adj, root, taken);
        if d <= depth && root != start {
            return root;
        }
        let cand = last_level
            .iter()
            .copied()
            .min_by_key(|&v| (adj[v].len(), v))
            .unwrap_or(root);
        if d <= depth || cand == root {
            return root;
        }
        depth = d;
        root = cand;
    }
}

/// The vertices at the greatest distance from `root` (within untaken vertices) and that distance.
fn farthest(adj: &[Vec<usize>], root: usize, taken: &[bool]) -> (Vec<usize>, usize) {
    let mut dist = vec![usize::MAX; adj.len()];
    let mut queue = std::collections::VecDeque::new();
    dist[root] = 0;
    queue.push_back(root);
    let mut best = 0;
    let mut level = vec![root];
    while let Some(v) = queue.pop_front() {
        for &w in &adj[v] {
            if !taken[w] && dist[w] == usize::MAX {
                dist[w] = dist[v] + 1;
                if dist[w] > best {
                    best = dist[w];
                    level.clear();
                }
                if dist[w] == best {
                    level.push(w);
                }
                queue.push_back(w);
            }
        }
    }
    (level, best)
}

#[cfg(test)]
#[allow(clippy::needless_range_loop)] // matrix code reads best with explicit indices
mod tests {
    use super::*;

    /// Dense reference: Gaussian elimination with partial pivoting.
    fn dense_solve(a: &[Vec<f64>], b: &[f64]) -> Vec<f64> {
        let n = b.len();
        let mut m: Vec<Vec<f64>> = a.to_vec();
        let mut x = b.to_vec();
        for c in 0..n {
            let p = (c..n)
                .max_by(|&i, &j| m[i][c].abs().total_cmp(&m[j][c].abs()))
                .unwrap();
            m.swap(c, p);
            x.swap(c, p);
            for r in c + 1..n {
                let f = m[r][c] / m[c][c];
                for k in c..n {
                    let t = m[c][k];
                    m[r][k] -= f * t;
                }
                let t = x[c];
                x[r] -= f * t;
            }
        }
        for c in (0..n).rev() {
            let mut s = x[c];
            for k in c + 1..n {
                s -= m[c][k] * x[k];
            }
            x[c] = s / m[c][c];
        }
        x
    }

    /// A symmetric positive-definite matrix with the given links: diagonally dominant with
    /// pseudo-random off-diagonals.
    fn spd(n: usize, links: &[(usize, usize)], seed: u64) -> Vec<Vec<f64>> {
        let mut a = vec![vec![0.0; n]; n];
        let mut s = seed;
        let mut rnd = || {
            s = s
                .wrapping_mul(6364136223846793005)
                .wrapping_add(1442695040888963407);
            ((s >> 33) as f64 / (1u64 << 31) as f64) - 0.5
        };
        for &(i, j) in links {
            if i != j {
                let v = rnd();
                a[i][j] = v;
                a[j][i] = v;
            }
        }
        for i in 0..n {
            let off: f64 = (0..n).filter(|&j| j != i).map(|j| a[i][j].abs()).sum();
            a[i][i] = off + 0.5 + rnd().abs();
        }
        a
    }

    fn check(n: usize, links: &[(usize, usize)], seed: u64) -> usize {
        let a = spd(n, links, seed);
        let p = Profile::new(n, links);
        let mut vals = p.zeros();
        for i in 0..n {
            for j in 0..=i {
                if a[i][j] != 0.0 {
                    p.add(&mut vals, i, j, if i == j { a[i][i] } else { a[i][j] });
                }
            }
        }
        let b: Vec<f64> = (0..n).map(|i| det::sin(i as f64 * 0.37) + 1.0).collect();
        let mut x = b.clone();
        assert!(p.solve(&mut vals, &mut x));
        let want = dense_solve(&a, &b);
        for (g, w) in x.iter().zip(&want) {
            assert!((g - w).abs() < 1e-9 * (1.0 + w.abs()), "{g} vs {w}");
        }
        p.stored()
    }

    #[test]
    fn it_matches_a_dense_solve_on_chains_grids_and_random_graphs() {
        // A chain, scrambled so that the natural order is bad.
        let n = 60;
        let scramble = |i: usize| (i * 37) % n;
        let chain: Vec<(usize, usize)> =
            (0..n - 1).map(|i| (scramble(i), scramble(i + 1))).collect();
        let stored = check(n, &chain, 1);
        // Reordering recovers a band: about 2 entries per row, not n/2.
        assert!(stored < 3 * n, "chain profile {stored}");
        // A grid.
        let w = 8;
        let mut grid = Vec::new();
        for y in 0..w {
            for x in 0..w {
                if x + 1 < w {
                    grid.push((y * w + x, y * w + x + 1));
                }
                if y + 1 < w {
                    grid.push((y * w + x, (y + 1) * w + x));
                }
            }
        }
        let stored = check(w * w, &grid, 2);
        assert!(stored < w * w * 2 * w, "grid profile {stored}");
        // Random sparse graphs, including disconnected ones and isolated variables.
        for seed in 0..20u64 {
            let n = 40;
            let mut links = Vec::new();
            let mut s = seed + 100;
            for _ in 0..50 {
                s = s.wrapping_mul(6364136223846793005).wrapping_add(1);
                let a = (s >> 20) as usize % n;
                s = s.wrapping_mul(6364136223846793005).wrapping_add(1);
                let b = (s >> 20) as usize % n;
                links.push((a, b));
            }
            check(n, &links, seed);
        }
        // No links at all: a diagonal matrix.
        check(5, &[], 3);
    }

    #[test]
    fn a_matrix_that_is_not_positive_definite_is_refused() {
        let p = Profile::new(2, &[(0, 1)]);
        let mut vals = p.zeros();
        p.add(&mut vals, 0, 0, 1.0);
        p.add(&mut vals, 1, 1, 1.0);
        p.add(&mut vals, 0, 1, 2.0); // |off-diagonal| > diagonal: indefinite
        let mut b = vec![1.0, 1.0];
        assert!(!p.solve(&mut vals, &mut b));
    }

    #[test]
    fn the_ordering_is_deterministic_and_a_permutation() {
        let links: Vec<(usize, usize)> =
            (0..30).map(|i| ((i * 7) % 31, (i * 11 + 3) % 31)).collect();
        let a = Profile::new(31, &links);
        let b = Profile::new(31, &links);
        assert_eq!(a.perm, b.perm);
        let mut seen = a.perm.clone();
        seen.sort_unstable();
        assert_eq!(seen, (0..31).collect::<Vec<_>>());
    }
}
