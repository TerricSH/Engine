use std::collections::{BTreeMap, BTreeSet};

use serde::de::Error as _;
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use thiserror::Error;

use super::types::TacticalEntityId;

#[derive(
    Debug, Default, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize,
)]
pub struct GridCoord {
    pub x: i32,
    pub y: i32,
    pub level: i16,
}

impl GridCoord {
    pub const fn new(x: i32, y: i32, level: i16) -> Self {
        Self { x, y, level }
    }

    pub fn manhattan_distance(self, other: Self) -> u32 {
        self.x.abs_diff(other.x)
            + self.y.abs_diff(other.y)
            + u32::from(self.level.abs_diff(other.level)) * 2
    }

    pub fn offset(self, direction: CardinalDirection) -> Self {
        let (x, y) = direction.delta();
        Self::new(self.x + x, self.y + y, self.level)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub enum CardinalDirection {
    North,
    East,
    South,
    West,
}

impl CardinalDirection {
    pub const ALL: [Self; 4] = [Self::North, Self::East, Self::South, Self::West];

    pub const fn delta(self) -> (i32, i32) {
        match self {
            Self::North => (0, 1),
            Self::East => (1, 0),
            Self::South => (0, -1),
            Self::West => (-1, 0),
        }
    }

    pub const fn opposite(self) -> Self {
        match self {
            Self::North => Self::South,
            Self::East => Self::West,
            Self::South => Self::North,
            Self::West => Self::East,
        }
    }

    const fn index(self) -> usize {
        match self {
            Self::North => 0,
            Self::East => 1,
            Self::South => 2,
            Self::West => 3,
        }
    }

    pub fn between(from: GridCoord, to: GridCoord) -> Option<Self> {
        if from.level != to.level {
            return None;
        }
        Self::ALL
            .into_iter()
            .find(|direction| from.offset(*direction) == to)
    }
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub enum CoverKind {
    #[default]
    None,
    Half,
    Full,
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct CoverEdges {
    edges: [CoverKind; 4],
}

impl CoverEdges {
    pub fn get(self, direction: CardinalDirection) -> CoverKind {
        self.edges[direction.index()]
    }

    pub fn set(&mut self, direction: CardinalDirection, cover: CoverKind) {
        self.edges[direction.index()] = cover;
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TacticalCell {
    pub coord: GridCoord,
    pub world_position: [f32; 3],
    pub movement_cost: u16,
    pub walkable: bool,
    pub blocks_sight: bool,
    pub cover: CoverEdges,
}

impl TacticalCell {
    pub fn walkable(coord: GridCoord, world_position: [f32; 3]) -> Self {
        Self {
            coord,
            world_position,
            movement_cost: 1,
            walkable: true,
            blocks_sight: false,
            cover: CoverEdges::default(),
        }
    }
}

#[derive(Debug, Error, Clone, PartialEq, Eq)]
pub enum BoardError {
    #[error("tactical cell {0:?} does not exist")]
    MissingCell(GridCoord),
    #[error("tactical cell {0:?} is not walkable")]
    NotWalkable(GridCoord),
    #[error("tactical cell {coord:?} is occupied by {occupant}")]
    Occupied {
        coord: GridCoord,
        occupant: TacticalEntityId,
    },
    #[error("tactical entity {0} is not on the board")]
    MissingEntity(TacticalEntityId),
    #[error("tactical cell {coord:?} is reserved by {owner}")]
    Reserved {
        coord: GridCoord,
        owner: TacticalEntityId,
    },
}

/// Aggregate root for tactical spatial state.
///
/// Occupancy, reservations and traversal links can only be modified through
/// this type, which keeps their forward and reverse indexes consistent.
#[derive(Debug, Default, Clone, PartialEq)]
pub struct TacticalBoard {
    cells: BTreeMap<GridCoord, TacticalCell>,
    occupants: BTreeMap<GridCoord, TacticalEntityId>,
    positions: BTreeMap<TacticalEntityId, GridCoord>,
    reservations: BTreeMap<GridCoord, TacticalEntityId>,
    links: BTreeMap<GridCoord, BTreeSet<GridCoord>>,
}

#[derive(Serialize, Deserialize)]
struct TacticalBoardSnapshot {
    cells: Vec<(GridCoord, TacticalCell)>,
    occupants: Vec<(GridCoord, TacticalEntityId)>,
    reservations: Vec<(GridCoord, TacticalEntityId)>,
    links: Vec<(GridCoord, Vec<GridCoord>)>,
}

impl Serialize for TacticalBoard {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        TacticalBoardSnapshot {
            cells: self
                .cells
                .iter()
                .map(|(coord, cell)| (*coord, cell.clone()))
                .collect(),
            occupants: self
                .occupants
                .iter()
                .map(|(coord, occupant)| (*coord, occupant.clone()))
                .collect(),
            reservations: self
                .reservations
                .iter()
                .map(|(coord, owner)| (*coord, owner.clone()))
                .collect(),
            links: self
                .links
                .iter()
                .map(|(coord, links)| (*coord, links.iter().copied().collect()))
                .collect(),
        }
        .serialize(serializer)
    }
}

impl<'de> Deserialize<'de> for TacticalBoard {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let snapshot = TacticalBoardSnapshot::deserialize(deserializer)?;
        let cells = snapshot.cells.into_iter().collect::<BTreeMap<_, _>>();
        let occupants = snapshot.occupants.into_iter().collect::<BTreeMap<_, _>>();
        let mut positions = BTreeMap::new();
        for (coord, entity) in &occupants {
            if !cells.contains_key(coord) {
                return Err(D::Error::custom(format!(
                    "occupant '{entity}' references missing cell {coord:?}"
                )));
            }
            if positions.insert(entity.clone(), *coord).is_some() {
                return Err(D::Error::custom(format!(
                    "occupant '{entity}' appears on more than one cell"
                )));
            }
        }
        let reservations = snapshot
            .reservations
            .into_iter()
            .collect::<BTreeMap<_, _>>();
        if let Some(coord) = reservations.keys().find(|coord| !cells.contains_key(coord)) {
            return Err(D::Error::custom(format!(
                "reservation references missing cell {coord:?}"
            )));
        }
        let links = snapshot
            .links
            .into_iter()
            .map(|(coord, links)| (coord, links.into_iter().collect::<BTreeSet<_>>()))
            .collect::<BTreeMap<_, _>>();
        if let Some(coord) = links
            .iter()
            .flat_map(|(from, to)| std::iter::once(from).chain(to.iter()))
            .find(|coord| !cells.contains_key(coord))
        {
            return Err(D::Error::custom(format!(
                "traversal link references missing cell {coord:?}"
            )));
        }
        Ok(Self {
            cells,
            occupants,
            positions,
            reservations,
            links,
        })
    }
}

impl TacticalBoard {
    pub fn insert_cell(&mut self, cell: TacticalCell) -> Option<TacticalCell> {
        self.cells.insert(cell.coord, cell)
    }

    pub fn cell(&self, coord: GridCoord) -> Option<&TacticalCell> {
        self.cells.get(&coord)
    }

    pub fn cell_mut(&mut self, coord: GridCoord) -> Option<&mut TacticalCell> {
        self.cells.get_mut(&coord)
    }

    pub fn cells(&self) -> impl Iterator<Item = &TacticalCell> {
        self.cells.values()
    }

    pub fn occupant_at(&self, coord: GridCoord) -> Option<&str> {
        self.occupants.get(&coord).map(String::as_str)
    }

    pub fn position_of(&self, entity: &str) -> Option<GridCoord> {
        self.positions.get(entity).copied()
    }

    pub fn reservation_at(&self, coord: GridCoord) -> Option<&str> {
        self.reservations.get(&coord).map(String::as_str)
    }

    pub fn occupy(
        &mut self,
        entity: impl Into<TacticalEntityId>,
        coord: GridCoord,
    ) -> Result<(), BoardError> {
        let entity = entity.into();
        self.validate_destination(&entity, coord)?;
        if let Some(previous) = self.positions.insert(entity.clone(), coord) {
            self.occupants.remove(&previous);
        }
        self.occupants.insert(coord, entity);
        Ok(())
    }

    pub fn move_entity(&mut self, entity: &str, coord: GridCoord) -> Result<(), BoardError> {
        if !self.positions.contains_key(entity) {
            return Err(BoardError::MissingEntity(entity.to_string()));
        }
        self.validate_destination(entity, coord)?;
        let previous = self
            .positions
            .insert(entity.to_string(), coord)
            .expect("position was checked above");
        self.occupants.remove(&previous);
        self.occupants.insert(coord, entity.to_string());
        Ok(())
    }

    pub fn vacate(&mut self, entity: &str) -> Option<GridCoord> {
        let coord = self.positions.remove(entity)?;
        self.occupants.remove(&coord);
        self.reservations.retain(|_, owner| owner != entity);
        Some(coord)
    }

    pub fn reserve(&mut self, entity: &str, coord: GridCoord) -> Result<(), BoardError> {
        self.validate_destination(entity, coord)?;
        self.reservations.retain(|_, owner| owner != entity);
        self.reservations.insert(coord, entity.to_string());
        Ok(())
    }

    pub fn release_reservations(&mut self, entity: &str) {
        self.reservations.retain(|_, owner| owner != entity);
    }

    pub fn add_bidirectional_link(
        &mut self,
        from: GridCoord,
        to: GridCoord,
    ) -> Result<(), BoardError> {
        if !self.cells.contains_key(&from) {
            return Err(BoardError::MissingCell(from));
        }
        if !self.cells.contains_key(&to) {
            return Err(BoardError::MissingCell(to));
        }
        self.links.entry(from).or_default().insert(to);
        self.links.entry(to).or_default().insert(from);
        Ok(())
    }

    pub fn neighbors(&self, coord: GridCoord) -> Vec<GridCoord> {
        let mut result = BTreeSet::new();
        for direction in CardinalDirection::ALL {
            let neighbor = coord.offset(direction);
            if self.cells.contains_key(&neighbor) {
                result.insert(neighbor);
            }
        }
        if let Some(links) = self.links.get(&coord) {
            result.extend(links);
        }
        result.into_iter().collect()
    }

    pub fn cover_between(&self, from: GridCoord, to: GridCoord) -> CoverKind {
        let Some(direction) = CardinalDirection::between(from, to) else {
            return CoverKind::None;
        };
        let from_cover = self
            .cell(from)
            .map(|cell| cell.cover.get(direction))
            .unwrap_or_default();
        let to_cover = self
            .cell(to)
            .map(|cell| cell.cover.get(direction.opposite()))
            .unwrap_or_default();
        from_cover.max(to_cover)
    }

    fn validate_destination(&self, entity: &str, coord: GridCoord) -> Result<(), BoardError> {
        let cell = self
            .cells
            .get(&coord)
            .ok_or(BoardError::MissingCell(coord))?;
        if !cell.walkable {
            return Err(BoardError::NotWalkable(coord));
        }
        if let Some(occupant) = self.occupants.get(&coord) {
            if occupant != entity {
                return Err(BoardError::Occupied {
                    coord,
                    occupant: occupant.clone(),
                });
            }
        }
        if let Some(owner) = self.reservations.get(&coord) {
            if owner != entity {
                return Err(BoardError::Reserved {
                    coord,
                    owner: owner.clone(),
                });
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
    fn moving_keeps_occupancy_indexes_consistent() {
        let mut board = line_board(3);
        board.occupy("alpha", GridCoord::new(0, 0, 0)).unwrap();
        board.move_entity("alpha", GridCoord::new(1, 0, 0)).unwrap();
        assert_eq!(board.occupant_at(GridCoord::new(0, 0, 0)), None);
        assert_eq!(board.occupant_at(GridCoord::new(1, 0, 0)), Some("alpha"));
        assert_eq!(board.position_of("alpha"), Some(GridCoord::new(1, 0, 0)));
    }

    #[test]
    fn reservations_prevent_conflicting_plans() {
        let mut board = line_board(2);
        board.reserve("alpha", GridCoord::new(1, 0, 0)).unwrap();
        assert!(matches!(
            board.reserve("bravo", GridCoord::new(1, 0, 0)),
            Err(BoardError::Reserved { .. })
        ));
    }

    #[test]
    fn cover_uses_both_sides_of_an_edge() {
        let mut board = line_board(2);
        board
            .cell_mut(GridCoord::new(1, 0, 0))
            .unwrap()
            .cover
            .set(CardinalDirection::West, CoverKind::Full);
        assert_eq!(
            board.cover_between(GridCoord::new(0, 0, 0), GridCoord::new(1, 0, 0)),
            CoverKind::Full
        );
    }
}
