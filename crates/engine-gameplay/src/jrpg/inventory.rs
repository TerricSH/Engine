use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use thiserror::Error;

use super::progression::{CharacterProgress, StatModifier};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ItemKind {
    Consumable,
    Equipment,
    Material,
    KeyItem,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EquipmentDefinition {
    pub slot: String,
    #[serde(default)]
    pub modifiers: Vec<StatModifier>,
    #[serde(default)]
    pub granted_abilities: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ItemDefinition {
    pub id: String,
    pub display_name: String,
    pub kind: ItemKind,
    #[serde(default = "default_stack_limit")]
    pub stack_limit: u32,
    #[serde(default)]
    pub consumable_ability: Option<String>,
    #[serde(default)]
    pub equipment: Option<EquipmentDefinition>,
}

fn default_stack_limit() -> u32 {
    99
}

impl ItemDefinition {
    pub fn validate(&self) -> Result<(), InventoryError> {
        if self.id.is_empty() {
            return Err(InventoryError::InvalidDefinition(
                "item id cannot be empty".into(),
            ));
        }
        if self.stack_limit == 0 {
            return Err(InventoryError::InvalidDefinition(format!(
                "item '{}' has a zero stack limit",
                self.id
            )));
        }
        if self.kind == ItemKind::Equipment && self.equipment.is_none() {
            return Err(InventoryError::InvalidDefinition(format!(
                "equipment item '{}' has no equipment descriptor",
                self.id
            )));
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Inventory {
    #[serde(default = "default_distinct_capacity")]
    pub distinct_capacity: usize,
    #[serde(default)]
    stacks: BTreeMap<String, u32>,
}

fn default_distinct_capacity() -> usize {
    256
}

impl Default for Inventory {
    fn default() -> Self {
        Self {
            distinct_capacity: default_distinct_capacity(),
            stacks: BTreeMap::new(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum InventoryError {
    #[error("unknown item '{0}'")]
    UnknownItem(String),
    #[error("inventory has no free distinct-item slot")]
    CapacityReached,
    #[error("item '{item}' stack would exceed {limit}")]
    StackLimit { item: String, limit: u32 },
    #[error("inventory has {available} '{item}', but {requested} are required")]
    Insufficient {
        item: String,
        requested: u32,
        available: u32,
    },
    #[error("item '{0}' is not equipment")]
    NotEquipment(String),
    #[error("invalid item definition: {0}")]
    InvalidDefinition(String),
}

impl Inventory {
    pub fn quantity(&self, item_id: &str) -> u32 {
        self.stacks.get(item_id).copied().unwrap_or(0)
    }

    pub fn iter(&self) -> impl Iterator<Item = (&str, u32)> {
        self.stacks
            .iter()
            .map(|(id, quantity)| (id.as_str(), *quantity))
    }

    pub fn add(
        &mut self,
        definition: &ItemDefinition,
        quantity: u32,
    ) -> Result<u32, InventoryError> {
        definition.validate()?;
        if quantity == 0 {
            return Ok(self.quantity(&definition.id));
        }
        if !self.stacks.contains_key(&definition.id) && self.stacks.len() >= self.distinct_capacity
        {
            return Err(InventoryError::CapacityReached);
        }
        let current = self.quantity(&definition.id);
        let next = current
            .checked_add(quantity)
            .filter(|next| *next <= definition.stack_limit)
            .ok_or_else(|| InventoryError::StackLimit {
                item: definition.id.clone(),
                limit: definition.stack_limit,
            })?;
        self.stacks.insert(definition.id.clone(), next);
        Ok(next)
    }

    pub fn remove(&mut self, item_id: &str, quantity: u32) -> Result<u32, InventoryError> {
        let current = self.quantity(item_id);
        if quantity > current {
            return Err(InventoryError::Insufficient {
                item: item_id.into(),
                requested: quantity,
                available: current,
            });
        }
        let remaining = current - quantity;
        if remaining == 0 {
            self.stacks.remove(item_id);
        } else {
            self.stacks.insert(item_id.into(), remaining);
        }
        Ok(remaining)
    }

    /// Atomically swaps one equipment slot and returns the previously equipped item.
    pub fn equip(
        &mut self,
        actor: &mut CharacterProgress,
        definition: &ItemDefinition,
        definitions: &BTreeMap<String, ItemDefinition>,
    ) -> Result<Option<String>, InventoryError> {
        let equipment = definition
            .equipment
            .as_ref()
            .ok_or_else(|| InventoryError::NotEquipment(definition.id.clone()))?;
        if self.quantity(&definition.id) == 0 {
            return Err(InventoryError::Insufficient {
                item: definition.id.clone(),
                requested: 1,
                available: 0,
            });
        }
        let previous = actor.equipment.get(&equipment.slot).cloned();
        let previous_definition = previous
            .as_ref()
            .map(|id| {
                definitions
                    .get(id)
                    .ok_or_else(|| InventoryError::UnknownItem(id.clone()))
            })
            .transpose()?;
        if let Some(previous_definition) = previous_definition {
            if self.quantity(&previous_definition.id) >= previous_definition.stack_limit {
                return Err(InventoryError::StackLimit {
                    item: previous_definition.id.clone(),
                    limit: previous_definition.stack_limit,
                });
            }
        }

        self.remove(&definition.id, 1)?;
        let replaced = actor
            .equipment
            .insert(equipment.slot.clone(), definition.id.clone());
        if let Some(previous_definition) = previous_definition {
            // All fallible checks happened before the actor/inventory mutation.
            self.add(previous_definition, 1)
                .expect("prevalidated equipment return");
        }
        debug_assert_eq!(previous, replaced);
        Ok(replaced)
    }

    pub fn unequip(
        &mut self,
        actor: &mut CharacterProgress,
        slot: &str,
        definitions: &BTreeMap<String, ItemDefinition>,
    ) -> Result<Option<String>, InventoryError> {
        let Some(item_id) = actor.equipment.get(slot).cloned() else {
            return Ok(None);
        };
        let definition = definitions
            .get(&item_id)
            .ok_or_else(|| InventoryError::UnknownItem(item_id.clone()))?;
        self.add(definition, 1)?;
        actor.equipment.remove(slot);
        Ok(Some(item_id))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::jrpg::StatBlock;

    fn sword() -> ItemDefinition {
        ItemDefinition {
            id: "bronze-sword".into(),
            display_name: "Bronze Sword".into(),
            kind: ItemKind::Equipment,
            stack_limit: 10,
            consumable_ability: None,
            equipment: Some(EquipmentDefinition {
                slot: "weapon".into(),
                modifiers: vec![StatModifier::flat(
                    "bronze-sword",
                    StatBlock {
                        attack: 5,
                        ..StatBlock::default()
                    },
                )],
                granted_abilities: vec![],
            }),
        }
    }

    #[test]
    fn equipment_swap_is_atomic() {
        let mut inventory = Inventory::default();
        let sword = sword();
        let definitions = BTreeMap::from([(sword.id.clone(), sword.clone())]);
        inventory.add(&sword, 1).unwrap();
        let mut actor = CharacterProgress::new("hero", "Hero", StatBlock::default());
        assert_eq!(
            inventory.equip(&mut actor, &sword, &definitions).unwrap(),
            None
        );
        assert_eq!(inventory.quantity("bronze-sword"), 0);
        assert_eq!(actor.equipment["weapon"], "bronze-sword");
    }

    #[test]
    fn stack_limits_are_enforced() {
        let mut inventory = Inventory::default();
        inventory.add(&sword(), 10).unwrap();
        assert!(matches!(
            inventory.add(&sword(), 1),
            Err(InventoryError::StackLimit { .. })
        ));
    }
}
