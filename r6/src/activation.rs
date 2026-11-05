//! Dependency-graph analysis used for activation (r6-activate).
//!
//! Services are nodes; `depends` and existing `wants` are ordering edges
//! (dependency -> dependent). The supervisor starts a service as soon as all of its
//! ordering dependencies are ready, so independent branches start concurrently.
//! This module computes deterministic orderings, detects cycles and produces the
//! "wave" plan shown by `r6-activate`.

use crate::config::{ConfigError, ServiceSet};
use std::collections::{BTreeSet, VecDeque};

pub struct Graph<'a> {
    pub names: Vec<&'a str>,
    /// deps[i] = indices that i must wait for.
    pub deps: Vec<Vec<usize>>,
    /// rdeps[i] = indices that wait for i.
    pub rdeps: Vec<Vec<usize>>,
}

impl<'a> Graph<'a> {
    pub fn new(set: &'a ServiceSet) -> Graph<'a> {
        let names: Vec<&str> = set.services.keys().map(|s| s.as_str()).collect();
        let idx = |n: &str| names.binary_search(&n).ok();
        let mut deps = vec![Vec::new(); names.len()];
        let mut rdeps = vec![Vec::new(); names.len()];
        for (i, name) in names.iter().enumerate() {
            let svc = &set.services[*name];
            for d in set.order_deps(svc) {
                if let Some(j) = idx(d) {
                    if !deps[i].contains(&j) {
                        deps[i].push(j);
                        rdeps[j].push(i);
                    }
                }
            }
        }
        Graph { names, deps, rdeps }
    }

    pub fn index(&self, name: &str) -> Option<usize> {
        self.names.binary_search(&name).ok()
    }

    /// Kahn's algorithm with a sorted ready queue, so the order is stable across runs.
    pub fn topo_order(&self) -> Result<Vec<usize>, ConfigError> {
        let n = self.names.len();
        let mut indeg: Vec<usize> = self.deps.iter().map(|d| d.len()).collect();
        let mut ready: BTreeSet<usize> = (0..n).filter(|&i| indeg[i] == 0).collect();
        let mut out = Vec::with_capacity(n);
        while let Some(i) = ready.pop_first() {
            out.push(i);
            for &r in &self.rdeps[i] {
                indeg[r] -= 1;
                if indeg[r] == 0 {
                    ready.insert(r);
                }
            }
        }
        if out.len() == n {
            Ok(out)
        } else {
            Err(ConfigError::Cycle(self.find_cycle(&indeg)))
        }
    }

    /// Walk dependencies among nodes that still have unresolved in-degree to report one cycle.
    fn find_cycle(&self, indeg: &[usize]) -> Vec<String> {
        let start = match (0..self.names.len()).find(|&i| indeg[i] > 0) {
            Some(s) => s,
            None => return vec![],
        };
        let mut path = vec![start];
        let mut pos = vec![usize::MAX; self.names.len()];
        pos[start] = 0;
        let mut cur = start;
        while let Some(next) = self.deps[cur].iter().copied().find(|&d| indeg[d] > 0) {
            if pos[next] != usize::MAX {
                let mut cyc: Vec<String> = path[pos[next]..]
                    .iter()
                    .map(|&i| self.names[i].to_string())
                    .collect();
                cyc.push(self.names[next].to_string());
                return cyc;
            }
            pos[next] = path.len();
            path.push(next);
            cur = next;
        }
        path.iter().map(|&i| self.names[i].to_string()).collect()
    }

    /// The transitive closure of `roots` over ordering dependencies.
    pub fn closure(&self, roots: impl IntoIterator<Item = usize>) -> BTreeSet<usize> {
        let mut seen = BTreeSet::new();
        let mut q: VecDeque<usize> = roots.into_iter().collect();
        while let Some(i) = q.pop_front() {
            if seen.insert(i) {
                q.extend(self.deps[i].iter().copied());
            }
        }
        seen
    }

    /// All services that (transitively) depend on `root`, excluding `root`.
    pub fn dependents(&self, root: usize) -> BTreeSet<usize> {
        let mut seen = BTreeSet::new();
        let mut q: VecDeque<usize> = self.rdeps[root].iter().copied().collect();
        while let Some(i) = q.pop_front() {
            if seen.insert(i) {
                q.extend(self.rdeps[i].iter().copied());
            }
        }
        seen
    }

    /// Group `members` into waves: every service in wave k only depends on services in
    /// waves < k. Services in the same wave can be started in parallel.
    pub fn waves(&self, members: &BTreeSet<usize>) -> Result<Vec<Vec<&'a str>>, ConfigError> {
        let order = self.topo_order()?;
        let mut level = vec![0usize; self.names.len()];
        let mut max = 0;
        for &i in &order {
            if !members.contains(&i) {
                continue;
            }
            let l = self.deps[i]
                .iter()
                .filter(|d| members.contains(d))
                .map(|&d| level[d] + 1)
                .max()
                .unwrap_or(0);
            level[i] = l;
            max = max.max(l);
        }
        let mut waves = vec![Vec::new(); if members.is_empty() { 0 } else { max + 1 }];
        for &i in members {
            waves[level[i]].push(self.names[i]);
        }
        Ok(waves)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::Service;
    use std::path::Path;

    fn svc(name: &str, deps: &[&str], wants: &[&str]) -> Service {
        let t = format!(
            "command = [\"/bin/true\"]\ndepends = {:?}\nwants = {:?}",
            deps, wants
        );
        Service::parse(name, &t, Path::new("x")).unwrap()
    }

    fn set(list: Vec<Service>) -> ServiceSet {
        ServiceSet {
            services: list.into_iter().map(|s| (s.name.clone(), s)).collect(),
        }
    }

    #[test]
    fn topo_is_deterministic_and_respects_deps() {
        let s = set(vec![
            svc("net", &["udev"], &[]),
            svc("udev", &[], &[]),
            svc("sshd", &["net"], &["syslog"]),
            svc("syslog", &[], &[]),
        ]);
        let g = Graph::new(&s);
        let order: Vec<&str> = g
            .topo_order()
            .unwrap()
            .into_iter()
            .map(|i| g.names[i])
            .collect();
        assert_eq!(order, vec!["syslog", "udev", "net", "sshd"]);
    }

    #[test]
    fn waves_group_parallel_services() {
        let s = set(vec![
            svc("a", &[], &[]),
            svc("b", &[], &[]),
            svc("c", &["a", "b"], &[]),
            svc("d", &["a"], &[]),
            svc("e", &["c", "d"], &[]),
        ]);
        let g = Graph::new(&s);
        let all: BTreeSet<usize> = (0..5).collect();
        let w = g.waves(&all).unwrap();
        assert_eq!(w, vec![vec!["a", "b"], vec!["c", "d"], vec!["e"]]);
    }

    #[test]
    fn closure_and_dependents() {
        let s = set(vec![
            svc("a", &[], &[]),
            svc("b", &["a"], &[]),
            svc("c", &["b"], &[]),
            svc("x", &[], &[]),
        ]);
        let g = Graph::new(&s);
        let c = g.closure([g.index("c").unwrap()]);
        let names: Vec<&str> = c.iter().map(|&i| g.names[i]).collect();
        assert_eq!(names, vec!["a", "b", "c"]);
        let d: Vec<&str> = g
            .dependents(g.index("a").unwrap())
            .iter()
            .map(|&i| g.names[i])
            .collect();
        assert_eq!(d, vec!["b", "c"]);
    }

    #[test]
    fn reports_cycle_path() {
        let s = set(vec![
            svc("a", &["c"], &[]),
            svc("b", &["a"], &[]),
            svc("c", &["b"], &[]),
            svc("z", &[], &[]),
        ]);
        let g = Graph::new(&s);
        match g.topo_order() {
            Err(ConfigError::Cycle(c)) => {
                assert_eq!(c.first(), c.last());
                assert_eq!(c.len(), 4, "{c:?}");
            }
            other => panic!("expected cycle, got {:?}", other.map(|_| ())),
        }
    }

    #[test]
    fn large_graph_is_fast() {
        // 5000-node chain + fan-out: must stay well under a millisecond-scale budget per node.
        let mut v = Vec::new();
        for i in 0..5000 {
            let deps: Vec<String> = if i == 0 {
                vec![]
            } else {
                vec![format!("s{}", i - 1)]
            };
            let deps: Vec<&str> = deps.iter().map(|s| s.as_str()).collect();
            v.push(svc(&format!("s{i}"), &deps, &[]));
        }
        let s = set(v);
        let t = std::time::Instant::now();
        let g = Graph::new(&s);
        assert_eq!(g.topo_order().unwrap().len(), 5000);
        assert!(t.elapsed().as_millis() < 2000);
    }
}
