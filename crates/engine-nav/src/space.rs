use std::{
    cmp::Ordering,
    collections::{BTreeMap, BTreeSet, BinaryHeap},
};

use glam::Vec3;
use serde::{Deserialize, Serialize};
use thiserror::Error;

/// Integer address of a voxel in a [`SpaceNavGrid`].
#[derive(
    Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize,
)]
pub struct SpaceCell {
    pub x: i32,
    pub y: i32,
    pub z: i32,
}

impl SpaceCell {
    pub const fn new(x: i32, y: i32, z: i32) -> Self {
        Self { x, y, z }
    }
}

/// Search controls for true three-dimensional A* navigation.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SpaceNavConfig {
    /// Enables all 26 neighbors instead of only the six axial neighbors.
    pub allow_diagonal: bool,
    /// Removes intermediate waypoints when the voxel segment is unobstructed.
    pub simplify_path: bool,
    /// Hard search budget used to bound work in malformed or huge grids.
    pub max_expansions: usize,
}

impl Default for SpaceNavConfig {
    fn default() -> Self {
        Self {
            allow_diagonal: true,
            simplify_path: true,
            max_expansions: 250_000,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct SpacePath {
    waypoints: Vec<Vec3>,
    length: f32,
}

impl SpacePath {
    pub fn waypoints(&self) -> &[Vec3] {
        &self.waypoints
    }

    pub fn length(&self) -> f32 {
        self.length
    }

    pub fn is_empty(&self) -> bool {
        self.waypoints.is_empty()
    }
}

#[derive(Clone, Debug, Error, PartialEq)]
pub enum SpaceNavError {
    #[error("space navigation cell size must be finite and greater than zero")]
    InvalidCellSize,
    #[error("space navigation dimensions must all be greater than zero")]
    InvalidDimensions,
    #[error("{which} point is outside the space navigation grid")]
    OutsideGrid { which: &'static str },
    #[error("{which} point occupies a blocked space navigation cell")]
    BlockedEndpoint { which: &'static str },
    #[error("no three-dimensional path was found")]
    NoPathFound,
    #[error("space navigation search exceeded its expansion budget")]
    SearchBudgetExceeded,
}

/// Bounded sparse-occupancy voxel grid for spacecraft, drones, and other
/// agents that move freely on all three axes.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct SpaceNavGrid {
    origin: Vec3,
    dimensions: [u32; 3],
    cell_size: f32,
    blocked: BTreeSet<SpaceCell>,
}

impl SpaceNavGrid {
    pub fn new(origin: Vec3, dimensions: [u32; 3], cell_size: f32) -> Result<Self, SpaceNavError> {
        if !cell_size.is_finite() || cell_size <= 0.0 {
            return Err(SpaceNavError::InvalidCellSize);
        }
        if dimensions.contains(&0) {
            return Err(SpaceNavError::InvalidDimensions);
        }
        Ok(Self {
            origin,
            dimensions,
            cell_size,
            blocked: BTreeSet::new(),
        })
    }

    pub fn origin(&self) -> Vec3 {
        self.origin
    }

    pub fn dimensions(&self) -> [u32; 3] {
        self.dimensions
    }

    pub fn cell_size(&self) -> f32 {
        self.cell_size
    }

    pub fn world_to_cell(&self, world: Vec3) -> Option<SpaceCell> {
        if !world.is_finite() {
            return None;
        }
        let local = (world - self.origin) / self.cell_size;
        let cell = SpaceCell::new(
            local.x.floor() as i32,
            local.y.floor() as i32,
            local.z.floor() as i32,
        );
        self.contains(cell).then_some(cell)
    }

    pub fn cell_center(&self, cell: SpaceCell) -> Option<Vec3> {
        self.contains(cell).then(|| {
            self.origin
                + (Vec3::new(cell.x as f32, cell.y as f32, cell.z as f32) + Vec3::splat(0.5))
                    * self.cell_size
        })
    }

    pub fn contains(&self, cell: SpaceCell) -> bool {
        cell.x >= 0
            && cell.y >= 0
            && cell.z >= 0
            && cell.x < self.dimensions[0] as i32
            && cell.y < self.dimensions[1] as i32
            && cell.z < self.dimensions[2] as i32
    }

    pub fn set_blocked(&mut self, cell: SpaceCell, blocked: bool) -> bool {
        if !self.contains(cell) {
            return false;
        }
        if blocked {
            self.blocked.insert(cell)
        } else {
            self.blocked.remove(&cell)
        }
    }

    pub fn is_blocked(&self, cell: SpaceCell) -> bool {
        !self.contains(cell) || self.blocked.contains(&cell)
    }

    pub fn clear_obstacles(&mut self) {
        self.blocked.clear();
    }

    pub fn find_path(&self, from: Vec3, to: Vec3) -> Result<SpacePath, SpaceNavError> {
        self.find_path_with_config(from, to, SpaceNavConfig::default())
    }

    pub fn find_path_with_config(
        &self,
        from: Vec3,
        to: Vec3,
        config: SpaceNavConfig,
    ) -> Result<SpacePath, SpaceNavError> {
        let start = self
            .world_to_cell(from)
            .ok_or(SpaceNavError::OutsideGrid { which: "start" })?;
        let goal = self
            .world_to_cell(to)
            .ok_or(SpaceNavError::OutsideGrid { which: "goal" })?;
        if self.is_blocked(start) {
            return Err(SpaceNavError::BlockedEndpoint { which: "start" });
        }
        if self.is_blocked(goal) {
            return Err(SpaceNavError::BlockedEndpoint { which: "goal" });
        }
        if start == goal {
            return Ok(path_from_world_points(
                if from.distance_squared(to) <= f32::EPSILON {
                    vec![from]
                } else {
                    vec![from, to]
                },
            ));
        }

        let mut open = BinaryHeap::new();
        let mut g_score = BTreeMap::new();
        let mut came_from = BTreeMap::new();
        g_score.insert(start, 0.0);
        open.push(SpaceOpenEntry {
            cell: start,
            f_score: cell_distance(start, goal),
        });

        let mut expansions = 0usize;
        while let Some(entry) = open.pop() {
            let Some(&current_g) = g_score.get(&entry.cell) else {
                continue;
            };
            let expected_f = current_g + cell_distance(entry.cell, goal);
            if entry.f_score > expected_f + 1.0e-5 {
                continue;
            }
            if entry.cell == goal {
                let mut cells = reconstruct_cells(&came_from, start, goal)?;
                if config.simplify_path {
                    cells = self.simplify_cells(&cells);
                }
                let mut waypoints = Vec::with_capacity(cells.len() + 2);
                waypoints.push(from);
                for &cell in cells.iter().skip(1).take(cells.len().saturating_sub(2)) {
                    if let Some(center) = self.cell_center(cell) {
                        waypoints.push(center);
                    }
                }
                waypoints.push(to);
                waypoints.dedup_by(|a, b| a.distance_squared(*b) <= f32::EPSILON);
                return Ok(path_from_world_points(waypoints));
            }

            expansions += 1;
            if expansions > config.max_expansions {
                return Err(SpaceNavError::SearchBudgetExceeded);
            }

            for (neighbor, step_cost) in self.neighbors(entry.cell, config.allow_diagonal) {
                let tentative = current_g + step_cost;
                if tentative + f32::EPSILON
                    < g_score.get(&neighbor).copied().unwrap_or(f32::INFINITY)
                {
                    came_from.insert(neighbor, entry.cell);
                    g_score.insert(neighbor, tentative);
                    open.push(SpaceOpenEntry {
                        cell: neighbor,
                        f_score: tentative + cell_distance(neighbor, goal),
                    });
                }
            }
        }
        Err(SpaceNavError::NoPathFound)
    }

    fn neighbors(&self, cell: SpaceCell, diagonal: bool) -> Vec<(SpaceCell, f32)> {
        let mut result = Vec::with_capacity(if diagonal { 26 } else { 6 });
        for dz in -1..=1 {
            for dy in -1..=1 {
                for dx in -1..=1 {
                    if dx == 0 && dy == 0 && dz == 0 {
                        continue;
                    }
                    let axis_count =
                        usize::from(dx != 0) + usize::from(dy != 0) + usize::from(dz != 0);
                    if !diagonal && axis_count != 1 {
                        continue;
                    }
                    let neighbor = SpaceCell::new(cell.x + dx, cell.y + dy, cell.z + dz);
                    if self.is_blocked(neighbor) || !self.diagonal_clear(cell, dx, dy, dz) {
                        continue;
                    }
                    result.push((neighbor, (axis_count as f32).sqrt()));
                }
            }
        }
        result
    }

    fn diagonal_clear(&self, cell: SpaceCell, dx: i32, dy: i32, dz: i32) -> bool {
        [(dx, 0, 0), (0, dy, 0), (0, 0, dz)]
            .into_iter()
            .filter(|&(x, y, z)| x != 0 || y != 0 || z != 0)
            .all(|(x, y, z)| !self.is_blocked(SpaceCell::new(cell.x + x, cell.y + y, cell.z + z)))
    }

    fn simplify_cells(&self, cells: &[SpaceCell]) -> Vec<SpaceCell> {
        if cells.len() <= 2 {
            return cells.to_vec();
        }
        let mut result = vec![cells[0]];
        let mut anchor = 0usize;
        while anchor < cells.len() - 1 {
            let mut furthest = anchor + 1;
            for candidate in (anchor + 2)..cells.len() {
                if self.cells_have_line_of_sight(cells[anchor], cells[candidate]) {
                    furthest = candidate;
                } else {
                    break;
                }
            }
            result.push(cells[furthest]);
            anchor = furthest;
        }
        result
    }

    fn cells_have_line_of_sight(&self, from: SpaceCell, to: SpaceCell) -> bool {
        let delta = Vec3::new(
            (to.x - from.x) as f32,
            (to.y - from.y) as f32,
            (to.z - from.z) as f32,
        );
        let steps = (delta.abs().max_element() * 2.0).ceil().max(1.0) as u32;
        (0..=steps).all(|step| {
            let position = Vec3::new(from.x as f32, from.y as f32, from.z as f32)
                + delta * (step as f32 / steps as f32);
            let cell = SpaceCell::new(
                position.x.round() as i32,
                position.y.round() as i32,
                position.z.round() as i32,
            );
            !self.is_blocked(cell)
        })
    }
}

#[derive(Clone, Copy, Debug)]
struct SpaceOpenEntry {
    cell: SpaceCell,
    f_score: f32,
}

impl PartialEq for SpaceOpenEntry {
    fn eq(&self, other: &Self) -> bool {
        self.cell == other.cell && self.f_score.to_bits() == other.f_score.to_bits()
    }
}

impl Eq for SpaceOpenEntry {}

impl PartialOrd for SpaceOpenEntry {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for SpaceOpenEntry {
    fn cmp(&self, other: &Self) -> Ordering {
        other
            .f_score
            .total_cmp(&self.f_score)
            .then_with(|| other.cell.cmp(&self.cell))
    }
}

fn cell_distance(a: SpaceCell, b: SpaceCell) -> f32 {
    let delta = Vec3::new((a.x - b.x) as f32, (a.y - b.y) as f32, (a.z - b.z) as f32);
    delta.length()
}

fn reconstruct_cells(
    came_from: &BTreeMap<SpaceCell, SpaceCell>,
    start: SpaceCell,
    goal: SpaceCell,
) -> Result<Vec<SpaceCell>, SpaceNavError> {
    let mut result = vec![goal];
    let mut current = goal;
    while current != start {
        current = *came_from.get(&current).ok_or(SpaceNavError::NoPathFound)?;
        result.push(current);
    }
    result.reverse();
    Ok(result)
}

fn path_from_world_points(waypoints: Vec<Vec3>) -> SpacePath {
    let length = waypoints
        .windows(2)
        .map(|pair| pair[0].distance(pair[1]))
        .sum();
    SpacePath { waypoints, length }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn three_dimensional_diagonal_path_uses_all_axes() {
        let grid = SpaceNavGrid::new(Vec3::ZERO, [8, 8, 8], 1.0).unwrap();
        let path = grid.find_path(Vec3::splat(0.5), Vec3::splat(7.5)).unwrap();
        assert_eq!(path.waypoints(), &[Vec3::splat(0.5), Vec3::splat(7.5)]);
        assert!((path.length() - 7.0 * 3.0f32.sqrt()).abs() < 1.0e-4);
    }

    #[test]
    fn obstacle_wall_requires_a_vertical_detour() {
        let mut grid = SpaceNavGrid::new(Vec3::ZERO, [7, 5, 3], 1.0).unwrap();
        for y in 0..4 {
            for z in 0..3 {
                grid.set_blocked(SpaceCell::new(3, y, z), true);
            }
        }
        let path = grid
            .find_path(Vec3::new(0.5, 0.5, 1.5), Vec3::new(6.5, 0.5, 1.5))
            .unwrap();
        assert!(path.waypoints().iter().any(|point| point.y > 4.0));
        assert!(path.length() > 6.0);
    }

    #[test]
    fn blocked_endpoint_is_reported_before_search() {
        let mut grid = SpaceNavGrid::new(Vec3::ZERO, [2, 2, 2], 1.0).unwrap();
        grid.set_blocked(SpaceCell::new(0, 0, 0), true);
        assert_eq!(
            grid.find_path(Vec3::splat(0.5), Vec3::splat(1.5)),
            Err(SpaceNavError::BlockedEndpoint { which: "start" })
        );
    }
}
