use std::collections::{HashSet, VecDeque};
use std::hash::Hash;
use std::marker::PhantomData;

use crate::models::GraphDirection;

#[derive(Clone)]
pub(crate) struct TraversalPathStep<N, E> {
    pub(crate) node: N,
    pub(crate) edge: Option<E>,
}

pub(crate) fn collect_paths_dfs<K, N, E, P, Err, KeyOf, Neighbors, Materialize>(
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
    Materialize: FnMut(GraphDirection, &[TraversalPathStep<N, E>]) -> P,
{
    let mut walker = DfsWalker::<K, N, E, P, Err, KeyOf, Neighbors, Materialize> {
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

struct DfsWalker<'a, K, N, E, P, Err, KeyOf, Neighbors, Materialize>
where
    K: Clone + Eq + Hash,
    N: Clone,
    E: Clone,
    KeyOf: FnMut(&N) -> K,
    Neighbors: FnMut(&N) -> Result<Vec<(N, E)>, Err>,
    Materialize: FnMut(GraphDirection, &[TraversalPathStep<N, E>]) -> P,
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
    DfsWalker<'a, K, N, E, P, Err, KeyOf, Neighbors, Materialize>
where
    K: Clone + Eq + Hash,
    N: Clone,
    E: Clone,
    KeyOf: FnMut(&N) -> K,
    Neighbors: FnMut(&N) -> Result<Vec<(N, E)>, Err>,
    Materialize: FnMut(GraphDirection, &[TraversalPathStep<N, E>]) -> P,
{
    fn run(&mut self, start_nodes: &[N], max_depth: usize) -> Result<(), Err> {
        for start in start_nodes {
            let start_key = (self.key_of)(start);
            let mut path = vec![TraversalPathStep {
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
        path: &mut Vec<TraversalPathStep<N, E>>,
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
            path.push(TraversalPathStep {
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

pub(crate) fn collect_reachable_bfs<State, Item, Key, Expand, NextState, KeyOf>(
    start_states: impl IntoIterator<Item = State>,
    initial_visited: impl IntoIterator<Item = Key>,
    mut expand: Expand,
    mut next_state: NextState,
    mut key_of: KeyOf,
) -> Vec<Item>
where
    State: Clone,
    Item: Clone,
    Key: Clone + Eq + Hash,
    Expand: FnMut(&State) -> Vec<Item>,
    NextState: FnMut(&Item) -> State,
    KeyOf: FnMut(&Item) -> Key,
{
    let mut results = Vec::new();
    let mut visited: HashSet<Key> = initial_visited.into_iter().collect();
    let mut queue: VecDeque<State> = start_states.into_iter().collect();

    while let Some(state) = queue.pop_front() {
        for item in expand(&state) {
            let key = key_of(&item);
            if !visited.insert(key) {
                continue;
            }
            queue.push_back(next_state(&item));
            results.push(item);
        }
    }

    results
}
