use crate::common::{CountryId, StateId};
use crate::state::data::StateRegistry;
use std::cmp::Ordering;
use std::collections::{BinaryHeap, HashMap};

#[derive(Copy, Clone, Eq, PartialEq)]
struct StateNode {
    id: StateId,
    cost: u32,
}

impl Ord for StateNode {
    fn cmp(&self, other: &Self) -> Ordering {
        // BinaryHeap is a max-heap, so reverse cost ordering for min-heap
        other
            .cost
            .cmp(&self.cost)
            .then_with(|| other.id.0.cmp(&self.id.0))
    }
}

impl PartialOrd for StateNode {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

/// 地形などのコストを返せる拡張関数（現段階では基本コスト1）
pub fn calculate_step_cost(_from: StateId, _to: StateId, _state_registry: &StateRegistry) -> u32 {
    1
}

pub fn find_path(
    start: StateId,
    end: StateId,
    state_registry: &StateRegistry,
    allowed_countries: &[CountryId],
    hostile_countries: &[CountryId],
) -> Option<Vec<StateId>> {
    if start == end {
        return Some(Vec::new());
    }

    // スタート地点とゴール地点の存在確認
    let start_state = state_registry.get(start)?;
    let end_state = state_registry.get(end)?;

    // ゴール地点が移動許可国に属しているか確認
    let end_controller = end_state
        .controller_country
        .unwrap_or(end_state.owner_country_id);
    if !allowed_countries.contains(&end_controller) && !hostile_countries.contains(&end_controller)
    {
        return None;
    }

    let _ = start_state;

    let mut distances: HashMap<StateId, u32> = HashMap::new();
    let mut previous: HashMap<StateId, StateId> = HashMap::new();
    let mut heap = BinaryHeap::new();

    distances.insert(start, 0);
    heap.push(StateNode { id: start, cost: 0 });

    while let Some(StateNode { id, cost }) = heap.pop() {
        if id == end {
            let mut path = Vec::new();
            let mut current = end;
            while current != start {
                path.push(current);
                if let Some(&prev) = previous.get(&current) {
                    current = prev;
                } else {
                    break;
                }
            }
            path.reverse();
            return Some(path);
        }

        if cost > *distances.get(&id).unwrap_or(&u32::MAX) {
            continue;
        }

        if let Some(state) = state_registry.get(id) {
            for &neighbor in &state.neighbors {
                if let Some(neighbor_state) = state_registry.get(neighbor) {
                    let controller = neighbor_state
                        .controller_country
                        .unwrap_or(neighbor_state.owner_country_id);

                    if !allowed_countries.contains(&controller)
                        && !hostile_countries.contains(&controller)
                    {
                        continue;
                    }
                } else {
                    continue;
                }

                let step_cost = calculate_step_cost(id, neighbor, state_registry);
                let next_cost = cost + step_cost;

                if next_cost < *distances.get(&neighbor).unwrap_or(&u32::MAX) {
                    distances.insert(neighbor, next_cost);
                    previous.insert(neighbor, id);
                    heap.push(StateNode {
                        id: neighbor,
                        cost: next_cost,
                    });
                }
            }
        }
    }

    None
}
