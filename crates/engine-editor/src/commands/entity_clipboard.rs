use super::*;

// -------------------------------------------------------------------
// Entity copy / paste model
// -------------------------------------------------------------------

const ENTITY_CLIPBOARD_FORMAT_VERSION: u32 = 1;

/// A self-contained, serializable set of copied entity subtrees.
///
/// The records retain each copied root's original external parent so callers
/// can either preserve its hierarchy placement or explicitly paste under a
/// different parent. Parents and [`Value::Entity`] references within the
/// copied set are remapped to fresh persistent IDs when a paste is prepared.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EntityClipboard {
    format_version: u32,
    root_ids: Vec<PersistentId>,
    entities: Vec<EntityRecord>,
}

impl EntityClipboard {
    /// Capture complete subtrees rooted at `entity_ids` without mutating the
    /// scene. Selecting both an ancestor and one of its descendants records
    /// that subtree only once.
    pub fn capture(scene: &Scene, entity_ids: &[PersistentId]) -> Result<Self, EditorError> {
        if entity_ids.is_empty() {
            return Err(EditorError::InvalidEntityClipboard(
                "at least one root entity is required".into(),
            ));
        }
        let by_id = scene
            .entities
            .iter()
            .map(|entity| (entity.persistent_id.as_str(), entity))
            .collect::<BTreeMap<_, _>>();
        let requested = entity_ids.iter().cloned().collect::<BTreeSet<_>>();
        if requested.len() != entity_ids.len() {
            return Err(EditorError::InvalidEntityClipboard(
                "root entity list contains duplicates".into(),
            ));
        }
        for entity_id in entity_ids {
            if !by_id.contains_key(entity_id.as_str()) {
                return Err(EditorError::EntityNotFound(entity_id.clone()));
            }
        }

        let mut root_ids = Vec::new();
        for entity_id in entity_ids {
            let mut cursor = by_id[entity_id.as_str()].parent.as_deref();
            let mut visited = BTreeSet::from([entity_id.as_str()]);
            let mut has_requested_ancestor = false;
            while let Some(parent_id) = cursor {
                if !visited.insert(parent_id) {
                    return Err(EditorError::InvalidHierarchy(format!(
                        "cycle encountered while copying '{entity_id}'"
                    )));
                }
                if requested.contains(parent_id) {
                    has_requested_ancestor = true;
                    break;
                }
                cursor = by_id
                    .get(parent_id)
                    .and_then(|parent| parent.parent.as_deref());
            }
            if !has_requested_ancestor {
                root_ids.push(entity_id.clone());
            }
        }

        let mut captured_ids = BTreeSet::new();
        let mut entities = Vec::new();
        for root_id in &root_ids {
            capture_subtree_records(
                scene,
                root_id,
                &mut BTreeSet::new(),
                &mut captured_ids,
                &mut entities,
            )?;
        }
        Self::from_records(root_ids, entities)
    }

    /// Construct a clipboard from serialized records and validate that every
    /// record belongs to exactly one declared root subtree.
    pub fn from_records(
        root_ids: Vec<PersistentId>,
        entities: Vec<EntityRecord>,
    ) -> Result<Self, EditorError> {
        let clipboard = Self {
            format_version: ENTITY_CLIPBOARD_FORMAT_VERSION,
            root_ids,
            entities,
        };
        clipboard.validate()?;
        Ok(clipboard)
    }

    pub fn root_ids(&self) -> &[PersistentId] {
        &self.root_ids
    }

    pub fn entities(&self) -> &[EntityRecord] {
        &self.entities
    }

    /// Serialize this clipboard to the RON text used by the platform
    /// clipboard and editor tests.
    pub fn to_ron(&self) -> Result<String, EditorError> {
        self.validate()?;
        ron::ser::to_string_pretty(self, ron::ser::PrettyConfig::default())
            .map_err(|error| EditorError::EntityClipboardSerialization(error.to_string()))
    }

    /// Parse and fully validate RON clipboard text before it can be pasted.
    pub fn from_ron(serialized: &str) -> Result<Self, EditorError> {
        let clipboard: Self = ron::from_str(serialized)
            .map_err(|error| EditorError::EntityClipboardSerialization(error.to_string()))?;
        clipboard.validate()?;
        Ok(clipboard)
    }

    pub fn validate(&self) -> Result<(), EditorError> {
        if self.format_version != ENTITY_CLIPBOARD_FORMAT_VERSION {
            return Err(EditorError::InvalidEntityClipboard(format!(
                "unsupported format version {}",
                self.format_version
            )));
        }
        if self.root_ids.is_empty() || self.entities.is_empty() {
            return Err(EditorError::InvalidEntityClipboard(
                "clipboard must contain at least one rooted entity".into(),
            ));
        }
        let roots = self.root_ids.iter().cloned().collect::<BTreeSet<_>>();
        if roots.len() != self.root_ids.len() || roots.iter().any(String::is_empty) {
            return Err(EditorError::InvalidEntityClipboard(
                "root IDs must be non-empty and unique".into(),
            ));
        }
        let mut records = BTreeMap::new();
        for entity in &self.entities {
            if entity.persistent_id.is_empty() {
                return Err(EditorError::InvalidEntityClipboard(
                    "entity persistent IDs cannot be empty".into(),
                ));
            }
            if records
                .insert(entity.persistent_id.as_str(), entity)
                .is_some()
            {
                return Err(EditorError::InvalidEntityClipboard(format!(
                    "duplicate entity ID '{}'",
                    entity.persistent_id
                )));
            }
        }
        for root_id in &self.root_ids {
            let root = records.get(root_id.as_str()).ok_or_else(|| {
                EditorError::InvalidEntityClipboard(format!(
                    "root '{root_id}' has no entity record"
                ))
            })?;
            if root
                .parent
                .as_ref()
                .is_some_and(|parent| records.contains_key(parent.as_str()))
            {
                return Err(EditorError::InvalidEntityClipboard(format!(
                    "root '{root_id}' has a parent inside the copied set"
                )));
            }
        }
        for entity in &self.entities {
            let mut cursor = entity.persistent_id.as_str();
            let mut visited = BTreeSet::new();
            loop {
                if !visited.insert(cursor) {
                    return Err(EditorError::InvalidEntityClipboard(format!(
                        "entity hierarchy contains a cycle at '{cursor}'"
                    )));
                }
                if roots.contains(cursor) {
                    break;
                }
                let record = records.get(cursor).ok_or_else(|| {
                    EditorError::InvalidEntityClipboard(format!(
                        "entity '{cursor}' is not part of a copied root subtree"
                    ))
                })?;
                cursor = record.parent.as_deref().ok_or_else(|| {
                    EditorError::InvalidEntityClipboard(format!(
                        "entity '{}' does not descend from a declared root",
                        entity.persistent_id
                    ))
                })?;
                if !records.contains_key(cursor) {
                    return Err(EditorError::InvalidEntityClipboard(format!(
                        "non-root entity '{}' references external parent '{cursor}'",
                        entity.persistent_id
                    )));
                }
            }
        }
        Ok(())
    }
}

fn capture_subtree_records(
    scene: &Scene,
    entity_id: &PersistentId,
    visiting: &mut BTreeSet<PersistentId>,
    captured: &mut BTreeSet<PersistentId>,
    records: &mut Vec<EntityRecord>,
) -> Result<(), EditorError> {
    if !visiting.insert(entity_id.clone()) {
        return Err(EditorError::InvalidHierarchy(format!(
            "cycle encountered while copying '{entity_id}'"
        )));
    }
    if captured.insert(entity_id.clone()) {
        let entity = scene
            .entities
            .iter()
            .find(|entity| &entity.persistent_id == entity_id)
            .cloned()
            .ok_or_else(|| EditorError::EntityNotFound(entity_id.clone()))?;
        records.push(entity);
        for child in scene
            .entities
            .iter()
            .filter(|child| child.parent.as_ref() == Some(entity_id))
        {
            capture_subtree_records(scene, &child.persistent_id, visiting, captured, records)?;
        }
    }
    visiting.remove(entity_id);
    Ok(())
}

/// Controls how copied root records are attached when a paste is prepared.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum EntityPasteParent {
    /// Keep each root's original external parent. Missing parents are errors.
    PreserveOriginal,
    /// Detach every copied root into the scene root.
    SceneRoot,
    /// Attach every copied root under this entity.
    Entity(PersistentId),
}

/// An undoable insertion prepared from validated [`EntityClipboard`] data.
///
/// Preparation is read-only and allocates all fresh IDs up front. Execution
/// revalidates the destination, making a stale paste plan fail without any
/// partial scene mutation.
pub struct PasteEntityRecords {
    records: Vec<EntityRecord>,
    root_ids: Vec<PersistentId>,
    insertion_index: usize,
}

impl PasteEntityRecords {
    /// Prepare a paste at the end of the scene's serialized entity sequence.
    pub fn prepare(
        scene: &Scene,
        clipboard: &EntityClipboard,
        parent: EntityPasteParent,
    ) -> Result<Self, EditorError> {
        Self::prepare_at(scene, clipboard, parent, scene.entities.len())
    }

    fn prepare_at(
        scene: &Scene,
        clipboard: &EntityClipboard,
        parent: EntityPasteParent,
        insertion_index: usize,
    ) -> Result<Self, EditorError> {
        clipboard.validate()?;
        if insertion_index > scene.entities.len() {
            return Err(EditorError::InvalidEntityClipboard(
                "paste insertion point is outside the scene".into(),
            ));
        }
        let existing = scene
            .entities
            .iter()
            .map(|entity| entity.persistent_id.clone())
            .collect::<BTreeSet<_>>();
        if let EntityPasteParent::Entity(parent_id) = &parent {
            if !existing.contains(parent_id) {
                return Err(EditorError::EntityNotFound(parent_id.clone()));
            }
        }

        let mut used = existing;
        let mut id_map = BTreeMap::new();
        for entity in &clipboard.entities {
            let new_id = allocate_copy_id(&entity.persistent_id, &mut used);
            id_map.insert(entity.persistent_id.clone(), new_id);
        }
        let roots = clipboard.root_ids.iter().cloned().collect::<BTreeSet<_>>();
        let mut records = Vec::with_capacity(clipboard.entities.len());
        for entity in &clipboard.entities {
            let mut pasted = entity.clone();
            pasted.persistent_id = id_map[&entity.persistent_id].clone();
            pasted.parent = if roots.contains(&entity.persistent_id) {
                match &parent {
                    EntityPasteParent::PreserveOriginal => match &entity.parent {
                        Some(parent_id) if !used.contains(parent_id) => {
                            return Err(EditorError::EntityNotFound(parent_id.clone()));
                        }
                        original => original.clone(),
                    },
                    EntityPasteParent::SceneRoot => None,
                    EntityPasteParent::Entity(parent_id) => Some(parent_id.clone()),
                }
            } else {
                entity
                    .parent
                    .as_ref()
                    .and_then(|parent_id| id_map.get(parent_id))
                    .cloned()
                    .ok_or_else(|| {
                        EditorError::InvalidEntityClipboard(format!(
                            "cannot remap parent of '{}'",
                            entity.persistent_id
                        ))
                    })?
                    .into()
            };
            for component in pasted.components.values_mut() {
                for value in component.fields.values_mut() {
                    remap_entity_references(value, &id_map);
                }
            }
            records.push(pasted);
        }
        let root_ids = clipboard
            .root_ids
            .iter()
            .map(|root_id| id_map[root_id].clone())
            .collect();
        let command = Self {
            records,
            root_ids,
            insertion_index,
        };
        command.validate_destination(scene)?;
        Ok(command)
    }

    pub fn pasted_root_ids(&self) -> &[PersistentId] {
        &self.root_ids
    }

    pub fn pasted_records(&self) -> &[EntityRecord] {
        &self.records
    }

    fn validate_destination(&self, scene: &Scene) -> Result<(), EditorError> {
        if self.insertion_index > scene.entities.len() {
            return Err(EditorError::InvalidEntityClipboard(
                "paste insertion point is no longer valid".into(),
            ));
        }
        let existing = scene
            .entities
            .iter()
            .map(|entity| entity.persistent_id.as_str())
            .collect::<BTreeSet<_>>();
        let pasted = self
            .records
            .iter()
            .map(|entity| entity.persistent_id.as_str())
            .collect::<BTreeSet<_>>();
        if pasted.len() != self.records.len() {
            return Err(EditorError::InvalidEntityClipboard(
                "prepared paste contains duplicate IDs".into(),
            ));
        }
        for entity in &self.records {
            if existing.contains(entity.persistent_id.as_str()) {
                return Err(EditorError::EntityAlreadyExists(
                    entity.persistent_id.clone(),
                ));
            }
            if let Some(parent_id) = &entity.parent {
                if !existing.contains(parent_id.as_str()) && !pasted.contains(parent_id.as_str()) {
                    return Err(EditorError::EntityNotFound(parent_id.clone()));
                }
            }
        }
        Ok(())
    }
}

impl Command for PasteEntityRecords {
    fn name(&self) -> &str {
        "Paste Entities"
    }

    fn execute(&mut self, scene: &mut Scene) -> Result<(), EditorError> {
        self.validate_destination(scene)?;
        scene.entities.splice(
            self.insertion_index..self.insertion_index,
            self.records.iter().cloned(),
        );
        Ok(())
    }

    fn undo(&mut self, scene: &mut Scene) -> Result<(), EditorError> {
        let end = self.insertion_index.saturating_add(self.records.len());
        if end > scene.entities.len() || scene.entities[self.insertion_index..end] != self.records {
            return Err(EditorError::InvalidHierarchy(
                "pasted entity sequence changed before undo".into(),
            ));
        }
        scene.entities.drain(self.insertion_index..end);
        Ok(())
    }
}

/// Undoable recursive duplication of exactly one entity subtree.
pub struct DuplicateEntitySubtree {
    paste: PasteEntityRecords,
}

impl DuplicateEntitySubtree {
    /// Build a deterministic duplicate plan without mutating `scene`.
    pub fn prepare(scene: &Scene, root_id: &PersistentId) -> Result<Self, EditorError> {
        let clipboard = EntityClipboard::capture(scene, std::slice::from_ref(root_id))?;
        let copied_ids = clipboard
            .entities
            .iter()
            .map(|entity| entity.persistent_id.as_str())
            .collect::<BTreeSet<_>>();
        let insertion_index = scene
            .entities
            .iter()
            .enumerate()
            .filter(|(_, entity)| copied_ids.contains(entity.persistent_id.as_str()))
            .map(|(index, _)| index + 1)
            .max()
            .ok_or_else(|| EditorError::EntityNotFound(root_id.clone()))?;
        Ok(Self {
            paste: PasteEntityRecords::prepare_at(
                scene,
                &clipboard,
                EntityPasteParent::PreserveOriginal,
                insertion_index,
            )?,
        })
    }

    pub fn duplicated_root_id(&self) -> &PersistentId {
        &self.paste.root_ids[0]
    }

    pub fn duplicated_records(&self) -> &[EntityRecord] {
        &self.paste.records
    }
}

impl Command for DuplicateEntitySubtree {
    fn name(&self) -> &str {
        "Duplicate Entity Subtree"
    }

    fn execute(&mut self, scene: &mut Scene) -> Result<(), EditorError> {
        self.paste.execute(scene)
    }

    fn undo(&mut self, scene: &mut Scene) -> Result<(), EditorError> {
        self.paste.undo(scene)
    }
}

fn allocate_copy_id(base: &str, used: &mut BTreeSet<PersistentId>) -> PersistentId {
    let first = format!("{base}-copy");
    if used.insert(first.clone()) {
        return first;
    }
    for suffix in 2_u64.. {
        let candidate = format!("{base}-copy-{suffix}");
        if used.insert(candidate.clone()) {
            return candidate;
        }
    }
    unreachable!("the u64 copy-ID suffix space cannot be exhausted in memory")
}

fn remap_entity_references(value: &mut Value, id_map: &BTreeMap<PersistentId, PersistentId>) {
    match value {
        Value::Entity(entity_id) => {
            if let Some(remapped) = id_map.get(entity_id) {
                *entity_id = remapped.clone();
            }
        }
        Value::List(values) => {
            for value in values {
                remap_entity_references(value, id_map);
            }
        }
        Value::Map(values) => {
            for value in values.values_mut() {
                remap_entity_references(value, id_map);
            }
        }
        _ => {}
    }
}
