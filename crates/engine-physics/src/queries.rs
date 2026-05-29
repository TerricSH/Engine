use crate::Entity;

/// A raycast query that can be performed against the physics world.
///
/// Safe to hold across frames — does not reference backend state.
#[derive(Clone, Debug)]
pub struct RaycastQuery {
    pub origin: glam::Vec3,
    pub direction: glam::Vec3,
    pub max_distance: f32,
}

/// An overlap (proximity) query.
///
/// Safe to hold across frames.
#[derive(Clone, Debug)]
pub struct OverlapQuery {
    pub shape: crate::ColliderShape,
    pub position: glam::Vec3,
}

/// A sweep (cast shape) query.
///
/// Safe to hold across frames.
#[derive(Clone, Debug)]
pub struct SweepQuery {
    pub shape: crate::ColliderShape,
    pub from: glam::Vec3,
    pub to: glam::Vec3,
}

/// Results from a query.
#[derive(Clone, Debug, Default)]
pub struct QueryResults {
    pub hits: Vec<Entity>,
}

impl QueryResults {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn is_empty(&self) -> bool {
        self.hits.is_empty()
    }

    pub fn len(&self) -> usize {
        self.hits.len()
    }
}
