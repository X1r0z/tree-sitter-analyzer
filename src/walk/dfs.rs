use std::collections::HashSet;
use std::convert::Infallible;
use std::hash::Hash;
use std::marker::PhantomData;

use crate::models::GraphDirection;

#[derive(Clone)]
pub struct PathStep<N, E> {
    pub node: N,
    pub edge: Option<E>,
}

pub fn collect_paths<K, N, E, P, KeyOf, Neighbors, Materialize>(
    start_nodes: &[N],
    direction: GraphDirection,
    max_depth: usize,
    key_of: KeyOf,
    mut neighbors_for: Neighbors,
    materialize: Materialize,
) -> Vec<P>
where
    K: Clone + Eq + Hash,
    N: Clone,
    E: Clone,
    KeyOf: FnMut(&N) -> K,
    Neighbors: FnMut(&N) -> Vec<(N, E)>,
    Materialize: FnMut(GraphDirection, &[PathStep<N, E>]) -> P,
{
    try_collect_paths(
        start_nodes,
        direction,
        max_depth,
        key_of,
        |node| Ok::<_, Infallible>(neighbors_for(node)),
        materialize,
    )
    .unwrap_or_else(|never| match never {})
}

pub fn try_collect_paths<K, N, E, P, Err, KeyOf, Neighbors, Materialize>(
    start_nodes: &[N],
    direction: GraphDirection,
    max_depth: usize,
    mut key_of: KeyOf,
    mut neighbors_for: Neighbors,
    mut materialize: Materialize,
) -> Result<Vec<P>, Err>
where
    K: Clone + Eq + Hash,
    N: Clone,
    E: Clone,
    KeyOf: FnMut(&N) -> K,
    Neighbors: FnMut(&N) -> Result<Vec<(N, E)>, Err>,
    Materialize: FnMut(GraphDirection, &[PathStep<N, E>]) -> P,
{
    let mut walker = Walker::<K, N, E, P, Err, KeyOf, Neighbors, Materialize> {
        direction,
        key_of: &mut key_of,
        neighbors_for: &mut neighbors_for,
        materialize: &mut materialize,
        seen_paths: HashSet::new(),
        results: Vec::new(),
        _marker: PhantomData,
    };
    walker.run(start_nodes, max_depth)?;

    Ok(walker.results)
}

struct Walker<'a, K, N, E, P, Err, KeyOf, Neighbors, Materialize>
where
    K: Clone + Eq + Hash,
    N: Clone,
    E: Clone,
    KeyOf: FnMut(&N) -> K,
    Neighbors: FnMut(&N) -> Result<Vec<(N, E)>, Err>,
    Materialize: FnMut(GraphDirection, &[PathStep<N, E>]) -> P,
{
    direction: GraphDirection,
    key_of: &'a mut KeyOf,
    neighbors_for: &'a mut Neighbors,
    materialize: &'a mut Materialize,
    seen_paths: HashSet<Vec<K>>,
    results: Vec<P>,
    _marker: PhantomData<(N, E)>,
}

impl<'a, K, N, E, P, Err, KeyOf, Neighbors, Materialize>
    Walker<'a, K, N, E, P, Err, KeyOf, Neighbors, Materialize>
where
    K: Clone + Eq + Hash,
    N: Clone,
    E: Clone,
    KeyOf: FnMut(&N) -> K,
    Neighbors: FnMut(&N) -> Result<Vec<(N, E)>, Err>,
    Materialize: FnMut(GraphDirection, &[PathStep<N, E>]) -> P,
{
    fn run(&mut self, start_nodes: &[N], max_depth: usize) -> Result<(), Err> {
        for start in start_nodes {
            let start_key = (self.key_of)(start);
            let mut path = vec![PathStep {
                node: start.clone(),
                edge: None,
            }];
            let mut visited = HashSet::from([start_key]);
            self.walk(max_depth, &mut path, &mut visited)?;
        }
        Ok(())
    }

    fn walk(
        &mut self,
        remaining_depth: usize,
        path: &mut Vec<PathStep<N, E>>,
        visited: &mut HashSet<K>,
    ) -> Result<(), Err> {
        let current = path
            .last()
            .map(|step| step.node.clone())
            .unwrap_or_else(|| unreachable!());

        let mut next_nodes = Vec::new();
        for (node, edge) in (self.neighbors_for)(&current)? {
            let key = (self.key_of)(&node);
            if !visited.contains(&key) {
                next_nodes.push((key, node, edge));
            }
        }

        if remaining_depth == 0 || next_nodes.is_empty() {
            let keys: Vec<_> = path.iter().map(|step| (self.key_of)(&step.node)).collect();
            if self.seen_paths.insert(keys) {
                self.results.push((self.materialize)(self.direction, path));
            }
            return Ok(());
        }

        for (next_key, next_node, next_edge) in next_nodes {
            visited.insert(next_key.clone());
            path.push(PathStep {
                node: next_node,
                edge: Some(next_edge),
            });
            self.walk(remaining_depth - 1, path, visited)?;
            path.pop();
            visited.remove(&next_key);
        }

        Ok(())
    }
}
