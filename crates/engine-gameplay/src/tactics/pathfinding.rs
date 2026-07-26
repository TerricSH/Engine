use std::cmp::Ordering;
use std::collections::{BTreeMap, BinaryHeap};

use serde::{Deserialize, Deserializer, Serialize, Serializer};

use super::grid::{GridCoord, TacticalBoard};

pub trait MovementCostPolicy {
    fn step_cost(
        &self,
        board: &TacticalBoard,
        from: GridCoord,
        to: GridCoord,
        actor: Option<&str>,
    ) -> Option<u32>;
}

#[derive(Debug, Default, Clone, Copy)]
pub struct DefaultMovementCostPolicy;

impl MovementCostPolicy for DefaultMovementCostPolicy {
    fn step_cost(
        &self,
        board: &TacticalBoard,
        _from: GridCoord,
        to: GridCoord,
        actor: Option<&str>,
    ) -> Option<u32> {
        let cell = board.cell(to)?;
        if !cell.walkable {
            return None;
        }
        if board
            .occupant_at(to)
            .is_some_and(|occupant| Some(occupant) != actor)
        {
            return None;
        }
        if board
            .reservation_at(to)
            .is_some_and(|owner| Some(owner) != actor)
        {
            return None;
        }
        Some(u32::from(cell.movement_cost.max(1)))
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TacticalPath {
    pub cells: Vec<GridCoord>,
    pub total_cost: u32,
}

#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct ReachabilityMap {
    costs: BTreeMap<GridCoord, u32>,
    previous: BTreeMap<GridCoord, GridCoord>,
}

#[derive(Serialize, Deserialize)]
struct ReachabilitySnapshot {
    costs: Vec<(GridCoord, u32)>,
    previous: Vec<(GridCoord, GridCoord)>,
}

impl Serialize for ReachabilityMap {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        ReachabilitySnapshot {
            costs: self
                .costs
                .iter()
                .map(|(coord, cost)| (*coord, *cost))
                .collect(),
            previous: self
                .previous
                .iter()
                .map(|(coord, previous)| (*coord, *previous))
                .collect(),
        }
        .serialize(serializer)
    }
}

impl<'de> Deserialize<'de> for ReachabilityMap {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let snapshot = ReachabilitySnapshot::deserialize(deserializer)?;
        Ok(Self {
            costs: snapshot.costs.into_iter().collect(),
            previous: snapshot.previous.into_iter().collect(),
        })
    }
}

impl ReachabilityMap {
    pub fn cost(&self, coord: GridCoord) -> Option<u32> {
        self.costs.get(&coord).copied()
    }

    pub fn reachable_cells(&self) -> impl Iterator<Item = (GridCoord, u32)> + '_ {
        self.costs.iter().map(|(coord, cost)| (*coord, *cost))
    }

    pub fn path_to(&self, start: GridCoord, goal: GridCoord) -> Option<TacticalPath> {
        let total_cost = self.cost(goal)?;
        let cells = reconstruct_path(&self.previous, start, goal)?;
        Some(TacticalPath { cells, total_cost })
    }
}

pub struct TacticalPathfinder<P = DefaultMovementCostPolicy> {
    movement: P,
}

impl Default for TacticalPathfinder<DefaultMovementCostPolicy> {
    fn default() -> Self {
        Self {
            movement: DefaultMovementCostPolicy,
        }
    }
}

impl<P: MovementCostPolicy> TacticalPathfinder<P> {
    pub fn new(movement: P) -> Self {
        Self { movement }
    }

    pub fn find_path(
        &self,
        board: &TacticalBoard,
        start: GridCoord,
        goal: GridCoord,
        actor: Option<&str>,
    ) -> Option<TacticalPath> {
        board.cell(start)?;
        board.cell(goal)?;
        let mut frontier = BinaryHeap::new();
        let mut costs = BTreeMap::from([(start, 0)]);
        let mut previous = BTreeMap::new();
        frontier.push(QueueNode::new(start, 0, start.manhattan_distance(goal)));

        while let Some(node) = frontier.pop() {
            let known_cost = *costs.get(&node.coord)?;
            if node.path_cost != known_cost {
                continue;
            }
            if node.coord == goal {
                return Some(TacticalPath {
                    cells: reconstruct_path(&previous, start, goal)?,
                    total_cost: known_cost,
                });
            }
            for neighbor in board.neighbors(node.coord) {
                let Some(step_cost) = self.movement.step_cost(board, node.coord, neighbor, actor)
                else {
                    continue;
                };
                let next_cost = known_cost.saturating_add(step_cost);
                let improves = costs
                    .get(&neighbor)
                    .is_none_or(|current| next_cost < *current);
                if improves {
                    costs.insert(neighbor, next_cost);
                    previous.insert(neighbor, node.coord);
                    frontier.push(QueueNode::new(
                        neighbor,
                        next_cost,
                        neighbor.manhattan_distance(goal),
                    ));
                }
            }
        }
        None
    }

    pub fn reachable(
        &self,
        board: &TacticalBoard,
        start: GridCoord,
        movement_budget: u32,
        actor: Option<&str>,
    ) -> ReachabilityMap {
        if board.cell(start).is_none() {
            return ReachabilityMap::default();
        }
        let mut result = ReachabilityMap {
            costs: BTreeMap::from([(start, 0)]),
            previous: BTreeMap::new(),
        };
        let mut frontier = BinaryHeap::from([QueueNode::new(start, 0, 0)]);
        while let Some(node) = frontier.pop() {
            let known_cost = result.costs[&node.coord];
            if node.path_cost != known_cost {
                continue;
            }
            for neighbor in board.neighbors(node.coord) {
                let Some(step_cost) = self.movement.step_cost(board, node.coord, neighbor, actor)
                else {
                    continue;
                };
                let next_cost = known_cost.saturating_add(step_cost);
                if next_cost > movement_budget {
                    continue;
                }
                let improves = result
                    .costs
                    .get(&neighbor)
                    .is_none_or(|current| next_cost < *current);
                if improves {
                    result.costs.insert(neighbor, next_cost);
                    result.previous.insert(neighbor, node.coord);
                    frontier.push(QueueNode::new(neighbor, next_cost, 0));
                }
            }
        }
        result
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct QueueNode {
    coord: GridCoord,
    path_cost: u32,
    estimated_total: u32,
}

impl QueueNode {
    fn new(coord: GridCoord, path_cost: u32, heuristic: u32) -> Self {
        Self {
            coord,
            path_cost,
            estimated_total: path_cost.saturating_add(heuristic),
        }
    }
}

impl Ord for QueueNode {
    fn cmp(&self, other: &Self) -> Ordering {
        other
            .estimated_total
            .cmp(&self.estimated_total)
            .then_with(|| other.path_cost.cmp(&self.path_cost))
            .then_with(|| other.coord.cmp(&self.coord))
    }
}

impl PartialOrd for QueueNode {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

fn reconstruct_path(
    previous: &BTreeMap<GridCoord, GridCoord>,
    start: GridCoord,
    goal: GridCoord,
) -> Option<Vec<GridCoord>> {
    let mut current = goal;
    let mut cells = vec![current];
    while current != start {
        current = *previous.get(&current)?;
        cells.push(current);
    }
    cells.reverse();
    Some(cells)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tactics::grid::TacticalCell;

    fn square_board(size: i32) -> TacticalBoard {
        let mut board = TacticalBoard::default();
        for y in 0..size {
            for x in 0..size {
                board.insert_cell(TacticalCell::walkable(
                    GridCoord::new(x, y, 0),
                    [x as f32, 0.0, y as f32],
                ));
            }
        }
        board
    }

    #[test]
    fn pathfinder_avoids_blocked_cells() {
        let mut board = square_board(3);
        board.cell_mut(GridCoord::new(1, 0, 0)).unwrap().walkable = false;
        let path = TacticalPathfinder::default()
            .find_path(
                &board,
                GridCoord::new(0, 0, 0),
                GridCoord::new(2, 0, 0),
                None,
            )
            .unwrap();
        assert_eq!(path.total_cost, 4);
        assert!(!path.cells.contains(&GridCoord::new(1, 0, 0)));
    }

    #[test]
    fn reachability_respects_movement_budget() {
        let board = square_board(4);
        let reachable =
            TacticalPathfinder::default().reachable(&board, GridCoord::new(0, 0, 0), 2, None);
        assert_eq!(reachable.cost(GridCoord::new(1, 1, 0)), Some(2));
        assert_eq!(reachable.cost(GridCoord::new(2, 1, 0)), None);
    }

    #[test]
    fn links_support_multi_level_traversal() {
        let mut board = TacticalBoard::default();
        let lower = GridCoord::new(0, 0, 0);
        let upper = GridCoord::new(0, 0, 1);
        board.insert_cell(TacticalCell::walkable(lower, [0.0, 0.0, 0.0]));
        board.insert_cell(TacticalCell::walkable(upper, [0.0, 3.0, 0.0]));
        board.add_bidirectional_link(lower, upper).unwrap();
        assert!(TacticalPathfinder::default()
            .find_path(&board, lower, upper, None)
            .is_some());
    }
}
