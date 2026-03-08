use std::collections::{HashSet, VecDeque};
use std::hash::Hash;

pub fn collect_reachable<State, Item, Key, Expand, NextState, KeyOf>(
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
