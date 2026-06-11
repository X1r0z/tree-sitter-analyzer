use std::collections::{HashSet, VecDeque};
use std::hash::Hash;

use crate::models::GraphDirection;

#[derive(Clone)]
pub(crate) struct TraversalPathStep<N, E> {
    pub(crate) node: N,
    pub(crate) edge: Option<E>,
}

enum WalkStep<N, E, K> {
    Enter {
        node: N,
        edge: Option<E>,
        key: K,
        depth: usize,
    },
    Leave {
        key: K,
    },
}

pub(crate) fn collect_paths<K, PK, N, E, P, Err, KeyOf, Neighbors, PathIdentity, Materialize>(
    start_nodes: &[N],
    direction: GraphDirection,
    max_depth: usize,
    mut key_of: KeyOf,
    mut neighbors_for: Neighbors,
    mut path_identity: PathIdentity,
    mut materialize: Materialize,
) -> Result<Vec<P>, Err>
where
    K: Clone + Eq + Hash,
    PK: Eq + Hash,
    N: Clone,
    E: Clone,
    KeyOf: FnMut(&N) -> K,
    Neighbors: FnMut(&N) -> Result<Vec<(N, E)>, Err>,
    PathIdentity: FnMut(&[TraversalPathStep<N, E>]) -> PK,
    Materialize: FnMut(GraphDirection, &[TraversalPathStep<N, E>]) -> P,
{
    let mut seen_paths: HashSet<PK> = HashSet::new();
    let mut results: Vec<P> = Vec::new();
    let mut path: Vec<TraversalPathStep<N, E>> = Vec::new();
    let mut visited: HashSet<K> = HashSet::new();

    let mut stack: Vec<WalkStep<N, E, K>> = start_nodes
        .iter()
        .rev()
        .map(|start| WalkStep::Enter {
            node: start.clone(),
            edge: None,
            key: key_of(start),
            depth: max_depth,
        })
        .collect();

    while let Some(step) = stack.pop() {
        let (node, edge, key, depth) = match step {
            WalkStep::Leave { key } => {
                path.pop();
                visited.remove(&key);
                continue;
            }
            WalkStep::Enter {
                node,
                edge,
                key,
                depth,
            } => (node, edge, key, depth),
        };

        visited.insert(key.clone());

        let mut next_nodes = Vec::new();
        for (neighbor, neighbor_edge) in neighbors_for(&node)? {
            let neighbor_key = key_of(&neighbor);
            if !visited.contains(&neighbor_key) {
                next_nodes.push((neighbor_key, neighbor, neighbor_edge));
            }
        }

        path.push(TraversalPathStep { node, edge });
        stack.push(WalkStep::Leave { key });

        if depth == 0 || next_nodes.is_empty() {
            let identity = path_identity(&path);
            if seen_paths.insert(identity) {
                results.push(materialize(direction, &path));
            }
            continue;
        }

        for (neighbor_key, neighbor, neighbor_edge) in next_nodes.into_iter().rev() {
            stack.push(WalkStep::Enter {
                node: neighbor,
                edge: Some(neighbor_edge),
                key: neighbor_key,
                depth: depth - 1,
            });
        }
    }

    Ok(results)
}

pub(crate) fn collect_reachable<State, Item, Key, Expand, NextState, KeyOf>(
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
