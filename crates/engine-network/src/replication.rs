use std::collections::{BTreeMap, BTreeSet};

use crate::{
    NetworkEntityId, NetworkError, NetworkRole, PeerId, ReplicationUpdate,
    MAX_REPLICATION_PAYLOAD_BYTES,
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Ownership {
    owner: Option<PeerId>,
    revision: u64,
}

#[derive(Clone, Debug, Default)]
pub struct ReplicationRegistry {
    role: Option<NetworkRole>,
    local_peer: PeerId,
    ownership: BTreeMap<NetworkEntityId, Ownership>,
    components: BTreeMap<(NetworkEntityId, String), ReplicationUpdate>,
    dirty: BTreeSet<(NetworkEntityId, String)>,
}

impl ReplicationRegistry {
    pub fn new(role: NetworkRole, local_peer: PeerId) -> Self {
        Self {
            role: Some(role),
            local_peer,
            ..Self::default()
        }
    }

    pub fn set_owner(
        &mut self,
        entity: NetworkEntityId,
        owner: Option<PeerId>,
    ) -> Result<u64, NetworkError> {
        if !self.role.is_some_and(NetworkRole::is_authority) {
            return Err(NetworkError::AuthorityRequired);
        }
        let revision = self
            .ownership
            .get(&entity)
            .map_or(1, |ownership| ownership.revision.saturating_add(1));
        self.ownership.insert(entity, Ownership { owner, revision });
        Ok(revision)
    }

    pub fn apply_owner(&mut self, entity: NetworkEntityId, owner: Option<PeerId>, revision: u64) {
        let current = self
            .ownership
            .get(&entity)
            .map_or(0, |value| value.revision);
        if revision > current {
            self.ownership.insert(entity, Ownership { owner, revision });
        }
    }

    pub fn owner(&self, entity: NetworkEntityId) -> Option<PeerId> {
        self.ownership.get(&entity).and_then(|value| value.owner)
    }

    pub fn ownership_snapshot(&self) -> Vec<(NetworkEntityId, Option<PeerId>, u64)> {
        self.ownership
            .iter()
            .map(|(entity, ownership)| (*entity, ownership.owner, ownership.revision))
            .collect()
    }

    pub fn can_write(&self, entity: NetworkEntityId, peer: PeerId) -> bool {
        self.role.is_some_and(NetworkRole::is_authority)
            || self.owner(entity).is_some_and(|owner| owner == peer)
    }

    pub fn write_component(
        &mut self,
        entity: NetworkEntityId,
        component: impl Into<String>,
        payload: Vec<u8>,
    ) -> Result<u64, NetworkError> {
        if payload.len() > MAX_REPLICATION_PAYLOAD_BYTES {
            return Err(NetworkError::LimitExceeded("replication component payload"));
        }
        if !self.can_write(entity, self.local_peer) {
            return Err(NetworkError::OwnershipDenied(entity));
        }
        let component = component.into();
        if component.is_empty() || component.len() > 128 {
            return Err(NetworkError::LimitExceeded("replication component name"));
        }
        let key = (entity, component.clone());
        let revision = self
            .components
            .get(&key)
            .map_or(1, |update| update.revision.saturating_add(1));
        self.components.insert(
            key.clone(),
            ReplicationUpdate {
                entity,
                component,
                revision,
                payload,
            },
        );
        self.dirty.insert(key);
        Ok(revision)
    }

    pub fn apply_updates(&mut self, updates: impl IntoIterator<Item = ReplicationUpdate>) -> usize {
        let mut applied = 0;
        for update in updates {
            if update.payload.len() > MAX_REPLICATION_PAYLOAD_BYTES {
                continue;
            }
            let key = (update.entity, update.component.clone());
            let current = self.components.get(&key).map_or(0, |value| value.revision);
            if update.revision > current {
                self.components.insert(key, update);
                applied += 1;
            }
        }
        applied
    }

    /// Validate client-authored component updates against entity ownership,
    /// apply only newer revisions and queue accepted state for authoritative
    /// rebroadcast. Clients cannot write unowned or another peer's entities.
    pub fn apply_peer_updates(
        &mut self,
        peer: PeerId,
        updates: impl IntoIterator<Item = ReplicationUpdate>,
    ) -> usize {
        if !self.role.is_some_and(NetworkRole::is_authority) {
            return 0;
        }
        let mut applied = 0;
        for update in updates {
            if update.payload.len() > MAX_REPLICATION_PAYLOAD_BYTES
                || self.owner(update.entity) != Some(peer)
                || update.component.is_empty()
                || update.component.len() > 128
            {
                continue;
            }
            let key = (update.entity, update.component.clone());
            let current = self.components.get(&key).map_or(0, |value| value.revision);
            if update.revision > current {
                self.components.insert(key.clone(), update);
                self.dirty.insert(key);
                applied += 1;
            }
        }
        applied
    }

    pub fn drain_dirty(&mut self, limit: usize) -> Vec<ReplicationUpdate> {
        let selected = self
            .dirty
            .iter()
            .take(limit.max(1))
            .cloned()
            .collect::<Vec<_>>();
        for key in &selected {
            self.dirty.remove(key);
        }
        selected
            .into_iter()
            .filter_map(|key| self.components.get(&key).cloned())
            .collect()
    }

    pub fn restore_dirty(&mut self, updates: impl IntoIterator<Item = ReplicationUpdate>) {
        self.dirty.extend(
            updates
                .into_iter()
                .map(|update| (update.entity, update.component)),
        );
    }

    pub fn snapshot(&self) -> Vec<ReplicationUpdate> {
        self.components.values().cloned().collect()
    }

    pub fn component(&self, entity: NetworkEntityId, component: &str) -> Option<&[u8]> {
        self.components
            .get(&(entity, component.to_string()))
            .map(|value| value.payload.as_slice())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stale_replication_cannot_replace_newer_state() {
        let mut client = ReplicationRegistry::new(NetworkRole::Client, PeerId(2));
        let entity = NetworkEntityId(7);
        assert_eq!(
            client.apply_updates([
                ReplicationUpdate {
                    entity,
                    component: "transform".into(),
                    revision: 2,
                    payload: vec![2],
                },
                ReplicationUpdate {
                    entity,
                    component: "transform".into(),
                    revision: 1,
                    payload: vec![1],
                },
            ]),
            1
        );
        assert_eq!(client.component(entity, "transform"), Some([2].as_slice()));
    }

    #[test]
    fn clients_only_write_entities_they_own() {
        let mut client = ReplicationRegistry::new(NetworkRole::Client, PeerId(2));
        let entity = NetworkEntityId(4);
        client.apply_owner(entity, Some(PeerId(3)), 1);
        assert!(matches!(
            client.write_component(entity, "health", vec![1]),
            Err(NetworkError::OwnershipDenied(id)) if id == entity
        ));
    }

    #[test]
    fn authority_rejects_peer_updates_without_matching_ownership() {
        let mut server = ReplicationRegistry::new(NetworkRole::AuthoritativeServer, PeerId(1));
        let entity = NetworkEntityId(9);
        server.set_owner(entity, Some(PeerId(2))).unwrap();
        let update = ReplicationUpdate {
            entity,
            component: "transform".into(),
            revision: 1,
            payload: vec![7],
        };
        assert_eq!(server.apply_peer_updates(PeerId(3), [update.clone()]), 0);
        assert_eq!(server.apply_peer_updates(PeerId(2), [update]), 1);
        assert_eq!(server.component(entity, "transform"), Some([7].as_slice()));
    }
}
