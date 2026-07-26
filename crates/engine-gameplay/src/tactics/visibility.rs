use std::collections::{BTreeMap, BTreeSet};

use serde::{Deserialize, Deserializer, Serialize, Serializer};

use super::grid::{GridCoord, TacticalBoard};
use super::types::FactionId;

pub trait LineOfSightPolicy {
    fn has_line_of_sight(
        &self,
        board: &TacticalBoard,
        origin: GridCoord,
        target: GridCoord,
    ) -> bool;
}

#[derive(Debug, Default, Clone, Copy)]
pub struct GridLineOfSight;

impl LineOfSightPolicy for GridLineOfSight {
    fn has_line_of_sight(
        &self,
        board: &TacticalBoard,
        origin: GridCoord,
        target: GridCoord,
    ) -> bool {
        if origin.level != target.level
            || board.cell(origin).is_none()
            || board.cell(target).is_none()
        {
            return false;
        }
        let line = raster_line(origin, target);
        line.into_iter()
            .skip(1)
            .take_while(|coord| *coord != target)
            .all(|coord| board.cell(coord).is_some_and(|cell| !cell.blocks_sight))
    }
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub enum VisibilityState {
    #[default]
    Unseen,
    Explored,
    Visible,
}

#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct VisibilityMap {
    by_faction: BTreeMap<FactionId, BTreeMap<GridCoord, VisibilityState>>,
}

#[derive(Serialize, Deserialize)]
struct VisibilitySnapshot {
    factions: Vec<(FactionId, Vec<(GridCoord, VisibilityState)>)>,
}

impl Serialize for VisibilityMap {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        VisibilitySnapshot {
            factions: self
                .by_faction
                .iter()
                .map(|(faction, cells)| {
                    (
                        *faction,
                        cells
                            .iter()
                            .map(|(coord, state)| (*coord, *state))
                            .collect(),
                    )
                })
                .collect(),
        }
        .serialize(serializer)
    }
}

impl<'de> Deserialize<'de> for VisibilityMap {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let snapshot = VisibilitySnapshot::deserialize(deserializer)?;
        Ok(Self {
            by_faction: snapshot
                .factions
                .into_iter()
                .map(|(faction, cells)| (faction, cells.into_iter().collect()))
                .collect(),
        })
    }
}

impl VisibilityMap {
    pub fn state(&self, faction: FactionId, coord: GridCoord) -> VisibilityState {
        self.by_faction
            .get(&faction)
            .and_then(|cells| cells.get(&coord))
            .copied()
            .unwrap_or_default()
    }

    pub fn update_faction<P: LineOfSightPolicy>(
        &mut self,
        faction: FactionId,
        board: &TacticalBoard,
        observers: impl IntoIterator<Item = (GridCoord, u32)>,
        policy: &P,
    ) {
        let cells = self.by_faction.entry(faction).or_default();
        for state in cells.values_mut() {
            if *state == VisibilityState::Visible {
                *state = VisibilityState::Explored;
            }
        }

        let mut visible = BTreeSet::new();
        for (origin, range) in observers {
            for cell in board.cells() {
                if origin.manhattan_distance(cell.coord) <= range
                    && policy.has_line_of_sight(board, origin, cell.coord)
                {
                    visible.insert(cell.coord);
                }
            }
        }
        for coord in visible {
            cells.insert(coord, VisibilityState::Visible);
        }
    }
}

fn raster_line(start: GridCoord, end: GridCoord) -> Vec<GridCoord> {
    let mut result = Vec::new();
    let mut x = start.x;
    let mut y = start.y;
    let dx = (end.x - start.x).abs();
    let sx = if start.x < end.x { 1 } else { -1 };
    let dy = -(end.y - start.y).abs();
    let sy = if start.y < end.y { 1 } else { -1 };
    let mut error = dx + dy;
    loop {
        result.push(GridCoord::new(x, y, start.level));
        if x == end.x && y == end.y {
            break;
        }
        let twice_error = error * 2;
        if twice_error >= dy {
            error += dy;
            x += sx;
        }
        if twice_error <= dx {
            error += dx;
            y += sy;
        }
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tactics::grid::TacticalCell;

    fn line_board(length: i32) -> TacticalBoard {
        let mut board = TacticalBoard::default();
        for x in 0..length {
            board.insert_cell(TacticalCell::walkable(
                GridCoord::new(x, 0, 0),
                [x as f32, 0.0, 0.0],
            ));
        }
        board
    }

    #[test]
    fn opaque_intermediate_cell_blocks_sight() {
        let mut board = line_board(4);
        board
            .cell_mut(GridCoord::new(2, 0, 0))
            .unwrap()
            .blocks_sight = true;
        assert!(!GridLineOfSight.has_line_of_sight(
            &board,
            GridCoord::new(0, 0, 0),
            GridCoord::new(3, 0, 0)
        ));
    }

    #[test]
    fn visible_cells_become_explored() {
        let board = line_board(4);
        let faction = FactionId(1);
        let mut visibility = VisibilityMap::default();
        visibility.update_faction(
            faction,
            &board,
            [(GridCoord::new(0, 0, 0), 1)],
            &GridLineOfSight,
        );
        assert_eq!(
            visibility.state(faction, GridCoord::new(1, 0, 0)),
            VisibilityState::Visible
        );
        visibility.update_faction(
            faction,
            &board,
            [(GridCoord::new(3, 0, 0), 0)],
            &GridLineOfSight,
        );
        assert_eq!(
            visibility.state(faction, GridCoord::new(1, 0, 0)),
            VisibilityState::Explored
        );
    }
}
