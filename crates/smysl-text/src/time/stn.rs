//! A simple temporal network: difference constraints, shortest paths, tightest bounds.
//!
//! Rule E's step 3 says "solve each stratum's network by shortest paths", and this is that
//! solver. One variable per subject, a reference variable fixed at zero, and every constraint
//! in the form `t_j − t_i ≤ w`:
//!
//! - an upper bound `t_i ≤ b` is `t_i − t_0 ≤ b`
//! - a lower bound `t_i ≥ a` is `t_0 − t_i ≤ −a`
//! - "`i` no later than `j`" is `t_i − t_j ≤ 0`
//!
//! The tightest upper bound on `t_i` is then the shortest path from the reference to `i`, and
//! the tightest lower bound is the negation of the shortest path from `i` back to the reference.
//! A **negative cycle** means the constraints cannot all hold, which is rule E's step 5.
//!
//! **Why this is a separate module.** A network knows nothing about datings, statuses or
//! liveness; it takes integers and gives integers. That is what makes the engine's stratified
//! loop readable — each stratum is one call — and what makes the property harness able to check
//! the solver against a brute-force reference ([`Network::solve_floyd_warshall`], P-E6) without
//! building a store.
//!
//! **Determinism** (rule U). The bound values are a property of the constraint set: shortest
//! paths are unique in *value* whatever order the relaxations happen in. The *path* is not, so
//! the predecessor each bound records — which is what a why-chain prints — is tie-broken on the
//! constraint's own id, smallest first. Without that, two runs could agree about every number
//! and disagree about which dating to blame.

use std::collections::VecDeque;

/// One difference constraint: `t[to] − t[from] ≤ weight`, attributed to `by`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Edge {
    pub from: usize,
    pub to: usize,
    pub weight: i64,
    /// Which constraint this came from, for the why-chain and for the tie-break.
    pub by: u32,
}

/// The reference variable, fixed at zero. Every unary bound is a constraint against it.
pub const REFERENCE: usize = 0;

/// A network over `vars` variables plus the reference.
#[derive(Debug, Clone, Default)]
pub struct Network {
    vars: usize,
    edges: Vec<Edge>,
}

/// What a solve found.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Solution {
    /// The tightest upper bound of each variable, or `None` where the constraints give none.
    pub upper: Vec<Option<i64>>,
    /// The tightest lower bound of each variable.
    pub lower: Vec<Option<i64>>,
    /// The constraint that set each upper bound, and each lower bound.
    pub upper_by: Vec<Option<u32>>,
    pub lower_by: Vec<Option<u32>>,
    /// The variables on a negative cycle: the constraints over them cannot all hold.
    ///
    /// Empty on a consistent network. When it is not empty the bounds are **not** to be used —
    /// rule E's step 5 says nothing is chosen — and the engine marks every subject here
    /// contested rather than picking a value.
    pub inconsistent: Vec<usize>,
}

impl Network {
    pub fn new(vars: usize) -> Network {
        Network {
            vars,
            edges: Vec::new(),
        }
    }

    pub fn vars(&self) -> usize {
        self.vars
    }

    pub fn edges(&self) -> &[Edge] {
        &self.edges
    }

    /// `t[to] − t[from] ≤ weight`.
    pub fn constrain(&mut self, from: usize, to: usize, weight: i64, by: u32) {
        self.edges.push(Edge {
            from,
            to,
            weight,
            by,
        });
    }

    /// `t[v] ≤ at`.
    pub fn upper(&mut self, v: usize, at: i64, by: u32) {
        self.constrain(REFERENCE, node(v), at, by);
    }

    /// `t[v] ≥ at`.
    pub fn lower(&mut self, v: usize, at: i64, by: u32) {
        self.constrain(node(v), REFERENCE, -at, by);
    }

    /// `t[a] ≤ t[b] + slack`: a is no later than b, by at least `-slack`.
    pub fn no_later(&mut self, a: usize, b: usize, slack: i64, by: u32) {
        self.constrain(node(b), node(a), slack, by);
    }

    /// `t[a] = t[b]`, as two inequalities.
    pub fn same(&mut self, a: usize, b: usize, by: u32) {
        self.no_later(a, b, 0, by);
        self.no_later(b, a, 0, by);
    }

    /// Solve: the tightest bound on every variable, or the cycle that makes it impossible.
    ///
    /// Two single-source runs rather than all-pairs. The upper bounds are the distances *from*
    /// the reference and the lower bounds are the distances *to* it, which is the same graph
    /// reversed — and both are O(V·E) worst case while all-pairs is O(V³), on a graph whose
    /// variables are the units of one connected component.
    pub fn solve(&self) -> Solution {
        let n = self.vars + 1;
        let (upper, upper_by, _) = self.shortest(n, false);
        let (to_ref, lower_by, _) = self.shortest(n, true);
        let lower = to_ref
            .iter()
            .map(|d| d.map(|v| -v))
            .collect::<Vec<Option<i64>>>();
        // Detection is its **own** pass, from a virtual source reaching every node, and that
        // is not a refinement — it is the difference between finding a contradiction and not.
        // A cycle among variables the reference cannot reach is still a set of constraints that
        // cannot all hold: "A strictly before B, B strictly before A" is unsatisfiable whether
        // or not anybody also said when A was. A search from the reference never visits those
        // nodes, so it reported them consistent, which the Floyd–Warshall reference caught.
        let mut inconsistent: Vec<usize> = self
            .negative_cycles(n)
            .into_iter()
            .filter(|v| *v != REFERENCE)
            .map(|v| v - 1)
            .collect();
        inconsistent.sort_unstable();
        inconsistent.dedup();
        Solution {
            upper: upper[1..].to_vec(),
            lower: lower[1..].to_vec(),
            upper_by: upper_by[1..].to_vec(),
            lower_by: lower_by[1..].to_vec(),
            inconsistent,
        }
    }

    /// Every node a negative cycle reaches, from a virtual source over all of them.
    ///
    /// The virtual source is not a variable and adds no constraint: initialising every distance
    /// to zero is the same thing as a source joined to every node by a zero-weight edge, and it
    /// is what makes the search reach a cycle nobody dated.
    fn negative_cycles(&self, n: usize) -> Vec<usize> {
        let mut out: Vec<Vec<usize>> = vec![Vec::new(); n];
        for (i, e) in self.edges.iter().enumerate() {
            out[e.from].push(i);
        }
        let mut dist: Vec<i64> = vec![0; n];
        let mut relaxations: Vec<usize> = vec![0; n];
        let mut queued = vec![true; n];
        let mut queue: VecDeque<usize> = (0..n).collect();
        let mut on_cycle: Vec<usize> = Vec::new();
        while let Some(u) = queue.pop_front() {
            queued[u] = false;
            for &i in &out[u] {
                let e = &self.edges[i];
                let cand = dist[u].saturating_add(e.weight);
                if cand >= dist[e.to] {
                    continue;
                }
                dist[e.to] = cand;
                relaxations[e.to] += 1;
                if relaxations[e.to] > n {
                    on_cycle.push(e.to);
                    continue;
                }
                if !queued[e.to] {
                    queued[e.to] = true;
                    queue.push_back(e.to);
                }
            }
        }
        if on_cycle.is_empty() {
            return on_cycle;
        }
        // The inconsistency spreads along the edges: a subject ordered against one on a
        // negative cycle is unbounded below too, and rule E marks every subject on it.
        let mut seen = vec![false; n];
        let mut stack = on_cycle.clone();
        for v in &stack {
            seen[*v] = true;
        }
        while let Some(u) = stack.pop() {
            for &i in &out[u] {
                let v = self.edges[i].to;
                if !seen[v] {
                    seen[v] = true;
                    on_cycle.push(v);
                    stack.push(v);
                }
            }
        }
        // And the other way: a cycle between two subjects is reached from each of them, so the
        // pair is named rather than whichever one the queue happened to relax first.
        let mut into: Vec<Vec<usize>> = vec![Vec::new(); n];
        for e in &self.edges {
            into[e.to].push(e.from);
        }
        let mut stack: Vec<usize> = on_cycle.clone();
        while let Some(u) = stack.pop() {
            for &v in &into[u] {
                if !seen[v] && is_on_cycle(&self.edges, v, n) {
                    seen[v] = true;
                    on_cycle.push(v);
                    stack.push(v);
                }
            }
        }
        on_cycle.sort_unstable();
        on_cycle.dedup();
        on_cycle
    }

    /// SPFA from the reference, over the graph or its reverse.
    ///
    /// Returns the distance to each node, the edge that set it, and the nodes a negative cycle
    /// reaches. `None` means unreachable, which is a variable the constraints bound on that
    /// side not at all — an open bound rather than infinity written down as a number.
    fn shortest(
        &self,
        n: usize,
        reverse: bool,
    ) -> (Vec<Option<i64>>, Vec<Option<u32>>, Vec<usize>) {
        // Adjacency in edge order, so the relaxation order is a function of the edge list.
        let mut out: Vec<Vec<usize>> = vec![Vec::new(); n];
        for (i, e) in self.edges.iter().enumerate() {
            let from = if reverse { e.to } else { e.from };
            out[from].push(i);
        }
        let mut dist: Vec<Option<i64>> = vec![None; n];
        let mut by: Vec<Option<u32>> = vec![None; n];
        let mut relaxations: Vec<usize> = vec![0; n];
        let mut queued = vec![false; n];
        let mut queue: VecDeque<usize> = VecDeque::new();
        let mut on_cycle: Vec<usize> = Vec::new();
        dist[REFERENCE] = Some(0);
        queue.push_back(REFERENCE);
        queued[REFERENCE] = true;
        while let Some(u) = queue.pop_front() {
            queued[u] = false;
            let du = match dist[u] {
                Some(d) => d,
                None => continue,
            };
            for &i in &out[u] {
                let e = &self.edges[i];
                let v = if reverse { e.from } else { e.to };
                let cand = du.saturating_add(e.weight);
                match dist[v] {
                    Some(d) if cand > d => continue,
                    // A tie changes **only the attribution**, and must not enqueue the node.
                    //
                    // Not an optimisation: enqueueing here does not terminate. A negative
                    // self-loop lowers a node every time it is popped, and a zero-weight edge
                    // beside it then ties at the new distance with a smaller id — so the node
                    // re-enqueues itself for ever while its distance runs away, and the
                    // relaxation counter never fires because the tie did not move anything.
                    // Found by the P-E6 generator on a five-variable network.
                    //
                    // Not propagating is also correct. What a node records is the *edge it was
                    // reached by*, so the tie-break at `v` cannot change the tie-break at
                    // anything downstream of `v`; there is nothing to tell them.
                    Some(d) if cand == d => {
                        if by[v].is_none_or(|b| e.by < b) {
                            by[v] = Some(e.by);
                        }
                        continue;
                    }
                    _ => {}
                }
                dist[v] = Some(cand);
                by[v] = Some(e.by);
                relaxations[v] += 1;
                // More than `n` relaxations of one node means a cycle whose total weight is
                // negative: the constraints over it cannot all hold. Detection proper is
                // `negative_cycles`; this is only what stops the search.
                if relaxations[v] > n {
                    on_cycle.push(v);
                    continue;
                }
                if !queued[v] {
                    queued[v] = true;
                    queue.push_back(v);
                }
            }
        }
        (dist, by, on_cycle)
    }

    /// The same answer by Floyd–Warshall over all pairs: the reference P-E6 compares against.
    ///
    /// Deliberately the textbook form, with no tie-break and no attribution. Its job is to be
    /// obviously right on a small graph, so that a disagreement with [`Network::solve`] is a
    /// defect in the fast path rather than in both.
    pub fn solve_floyd_warshall(&self) -> (Vec<Option<i64>>, Vec<Option<i64>>, bool) {
        let n = self.vars + 1;
        let mut d: Vec<Vec<Option<i64>>> = vec![vec![None; n]; n];
        for (i, row) in d.iter_mut().enumerate() {
            row[i] = Some(0);
        }
        for e in &self.edges {
            let cur = d[e.from][e.to];
            if cur.is_none_or(|c| e.weight < c) {
                d[e.from][e.to] = Some(e.weight);
            }
        }
        for k in 0..n {
            for i in 0..n {
                for j in 0..n {
                    if let (Some(a), Some(b)) = (d[i][k], d[k][j]) {
                        let via = a.saturating_add(b);
                        if d[i][j].is_none_or(|c| via < c) {
                            d[i][j] = Some(via);
                        }
                    }
                }
            }
        }
        let negative = (0..n).any(|i| d[i][i].is_some_and(|v| v < 0));
        let upper = (1..n).map(|i| d[REFERENCE][i]).collect();
        let lower = (1..n).map(|i| d[i][REFERENCE].map(|v| -v)).collect();
        (upper, lower, negative)
    }
}

/// The node of a variable. Variable 0 is node 1, because node 0 is the reference.
const fn node(v: usize) -> usize {
    v + 1
}

/// Whether `start` lies on a cycle of negative total weight, by a bounded search.
///
/// Used only to widen the set a detection pass names, and bounded by `n` steps because a
/// walk longer than that has repeated a node and is therefore already a cycle.
fn is_on_cycle(edges: &[Edge], start: usize, n: usize) -> bool {
    // Reachability back to `start`: if the node is on any cycle at all, and that cycle was
    // reached by the negative search, the whole cycle is part of the same inconsistency.
    let mut seen = vec![false; n];
    let mut stack = vec![start];
    let mut steps = 0usize;
    while let Some(u) = stack.pop() {
        steps += 1;
        if steps > n * n {
            return false;
        }
        for e in edges.iter().filter(|e| e.from == u) {
            if e.to == start {
                return true;
            }
            if !seen[e.to] {
                seen[e.to] = true;
                stack.push(e.to);
            }
        }
    }
    false
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_bound_on_one_variable_comes_back_as_itself() {
        let mut n = Network::new(1);
        n.lower(0, 10, 1);
        n.upper(0, 20, 2);
        let s = n.solve();
        assert_eq!(s.lower[0], Some(10));
        assert_eq!(s.upper[0], Some(20));
        assert_eq!(s.lower_by[0], Some(1));
        assert_eq!(s.upper_by[0], Some(2));
        assert!(s.inconsistent.is_empty());
    }

    #[test]
    fn an_unbounded_side_stays_unbounded() {
        let mut n = Network::new(1);
        n.lower(0, 10, 1);
        let s = n.solve();
        assert_eq!(s.lower[0], Some(10));
        assert_eq!(s.upper[0], None, "nothing bounds it above");
    }

    /// The point of a network: a bound reaches a variable it was not written about.
    #[test]
    fn an_ordering_propagates_a_bound_along_the_chain() {
        // a ≤ b ≤ c, and a is known to be at least 10: so is b, and so is c.
        let mut n = Network::new(3);
        n.lower(0, 10, 1);
        n.no_later(0, 1, 0, 2);
        n.no_later(1, 2, 0, 3);
        let s = n.solve();
        assert_eq!(s.lower[1], Some(10));
        assert_eq!(s.lower[2], Some(10));
        // And an upper bound on c comes back down the chain to a.
        let mut n = Network::new(3);
        n.upper(2, 50, 1);
        n.no_later(0, 1, 0, 2);
        n.no_later(1, 2, 0, 3);
        let s = n.solve();
        assert_eq!(s.upper[0], Some(50));
        assert_eq!(s.upper[1], Some(50));
    }

    #[test]
    fn equality_moves_bounds_both_ways() {
        let mut n = Network::new(2);
        n.lower(0, 10, 1);
        n.upper(1, 30, 2);
        n.same(0, 1, 3);
        let s = n.solve();
        assert_eq!((s.lower[0], s.upper[0]), (Some(10), Some(30)));
        assert_eq!((s.lower[1], s.upper[1]), (Some(10), Some(30)));
    }

    /// Rule E step 5: a cycle that cannot be satisfied names every subject on it.
    #[test]
    fn a_contradiction_is_a_negative_cycle_and_names_its_subjects() {
        // a strictly before b, and b strictly before a.
        let mut n = Network::new(2);
        n.no_later(0, 1, -1, 1);
        n.no_later(1, 0, -1, 2);
        let s = n.solve();
        assert_eq!(s.inconsistent, vec![0, 1]);
    }

    #[test]
    fn an_impossible_pair_of_bounds_is_a_negative_cycle_too() {
        let mut n = Network::new(1);
        n.lower(0, 30, 1);
        n.upper(0, 20, 2);
        let s = n.solve();
        assert_eq!(s.inconsistent, vec![0], "a variable above 30 and below 20");
    }

    /// P-E6 on hand-written networks. The generator does the random half in
    /// `tests/time_algebra.rs`; this is the half that has to agree before the generator means
    /// anything.
    #[test]
    fn the_solver_agrees_with_floyd_warshall() {
        let cases: Vec<Network> = vec![
            {
                let mut n = Network::new(3);
                n.lower(0, 10, 1);
                n.upper(2, 90, 2);
                n.no_later(0, 1, 0, 3);
                n.no_later(1, 2, -5, 4);
                n
            },
            {
                let mut n = Network::new(4);
                n.lower(0, 0, 1);
                n.upper(0, 100, 2);
                n.same(1, 2, 3);
                n.no_later(2, 3, 0, 4);
                n.no_later(0, 1, -10, 5);
                n
            },
            {
                let mut n = Network::new(2);
                n.no_later(0, 1, -1, 1);
                n.no_later(1, 0, -1, 2);
                n
            },
        ];
        for (i, n) in cases.iter().enumerate() {
            let fast = n.solve();
            let (upper, lower, negative) = n.solve_floyd_warshall();
            assert_eq!(fast.inconsistent.is_empty(), !negative, "case {i}");
            if negative {
                continue;
            }
            assert_eq!(fast.upper, upper, "case {i} upper");
            assert_eq!(fast.lower, lower, "case {i} lower");
        }
    }

    /// Rule U at the solver's own level: the answer is a function of the constraint set.
    #[test]
    fn the_answer_does_not_depend_on_the_order_the_constraints_arrive_in() {
        type Add = Box<dyn Fn(&mut Network)>;
        let build = |order: &[usize]| {
            let mut n = Network::new(3);
            let add: Vec<Add> = vec![
                Box::new(|n: &mut Network| n.lower(0, 10, 1)),
                Box::new(|n: &mut Network| n.upper(2, 90, 2)),
                Box::new(|n: &mut Network| n.no_later(0, 1, 0, 3)),
                Box::new(|n: &mut Network| n.no_later(1, 2, -5, 4)),
            ];
            for i in order {
                add[*i](&mut n);
            }
            n.solve()
        };
        let a = build(&[0, 1, 2, 3]);
        for order in [[3, 2, 1, 0], [1, 3, 0, 2], [2, 0, 3, 1]] {
            assert_eq!(build(&order), a, "order {order:?}");
        }
    }

    /// Two constraints that give one bound the same value: the smaller id is the one blamed.
    #[test]
    fn a_tie_is_broken_by_the_constraints_own_id() {
        let mut n = Network::new(1);
        n.upper(0, 20, 7);
        n.upper(0, 20, 3);
        assert_eq!(n.solve().upper_by[0], Some(3));
        // And the other order gives the same answer, which is the point.
        let mut n = Network::new(1);
        n.upper(0, 20, 3);
        n.upper(0, 20, 7);
        assert_eq!(n.solve().upper_by[0], Some(3));
    }
}
