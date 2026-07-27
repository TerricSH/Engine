use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use thiserror::Error;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", content = "value", rename_all = "snake_case")]
pub enum StoryValue {
    Bool(bool),
    Integer(i64),
    Text(String),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum QuestStatus {
    Inactive,
    Active,
    Completed,
    Failed,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct QuestObjective {
    pub id: String,
    pub target_count: u32,
    pub text_key: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct QuestDefinition {
    pub id: String,
    pub title_key: String,
    pub description_key: String,
    #[serde(default)]
    pub objectives: Vec<QuestObjective>,
    #[serde(default)]
    pub prerequisites: Vec<StoryCondition>,
    #[serde(default)]
    pub completion_commands: Vec<NarrativeCommand>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct QuestProgress {
    pub status: QuestStatus,
    #[serde(default)]
    pub objective_counts: BTreeMap<String, u32>,
}

impl Default for QuestProgress {
    fn default() -> Self {
        Self {
            status: QuestStatus::Inactive,
            objective_counts: BTreeMap::new(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct StoryState {
    #[serde(default)]
    pub flags: BTreeMap<String, StoryValue>,
    #[serde(default)]
    pub quests: BTreeMap<String, QuestProgress>,
}

impl Default for StoryState {
    fn default() -> Self {
        Self {
            flags: BTreeMap::new(),
            quests: BTreeMap::new(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum StoryCondition {
    FlagEquals {
        key: String,
        value: StoryValue,
    },
    IntegerAtLeast {
        key: String,
        value: i64,
    },
    QuestIs {
        quest_id: String,
        status: QuestStatus,
    },
    All {
        conditions: Vec<StoryCondition>,
    },
    Any {
        conditions: Vec<StoryCondition>,
    },
    Not {
        condition: Box<StoryCondition>,
    },
}

impl StoryCondition {
    pub fn evaluate(&self, state: &StoryState) -> bool {
        match self {
            Self::FlagEquals { key, value } => state.flags.get(key) == Some(value),
            Self::IntegerAtLeast { key, value } => {
                matches!(state.flags.get(key), Some(StoryValue::Integer(found)) if found >= value)
            }
            Self::QuestIs { quest_id, status } => {
                state
                    .quests
                    .get(quest_id)
                    .map(|progress| progress.status)
                    .unwrap_or(QuestStatus::Inactive)
                    == *status
            }
            Self::All { conditions } => conditions.iter().all(|item| item.evaluate(state)),
            Self::Any { conditions } => conditions.iter().any(|item| item.evaluate(state)),
            Self::Not { condition } => !condition.evaluate(state),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum NarrativeCommand {
    GrantItem {
        item_id: String,
        quantity: u32,
    },
    GrantCurrency {
        amount: u64,
    },
    GrantExperience {
        amount: u64,
    },
    LoadScene {
        scene_id: String,
    },
    PlaySequence {
        sequence_id: String,
    },
    Custom {
        command: String,
        #[serde(default)]
        payload: BTreeMap<String, StoryValue>,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum StoryEffect {
    SetFlag {
        key: String,
        value: StoryValue,
    },
    AddInteger {
        key: String,
        amount: i64,
    },
    StartQuest {
        quest_id: String,
    },
    AdvanceObjective {
        quest_id: String,
        objective_id: String,
        amount: u32,
    },
    CompleteQuest {
        quest_id: String,
    },
    FailQuest {
        quest_id: String,
    },
    Emit {
        command: NarrativeCommand,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum NarrativeError {
    #[error("unknown quest '{0}'")]
    UnknownQuest(String),
    #[error("quest '{0}' prerequisites are not satisfied")]
    PrerequisitesNotMet(String),
    #[error("quest '{0}' is not active")]
    QuestNotActive(String),
    #[error("quest '{0}' still has incomplete objectives")]
    ObjectivesIncomplete(String),
    #[error("unknown objective '{objective}' in quest '{quest}'")]
    UnknownObjective { quest: String, objective: String },
    #[error("unknown dialogue node '{0}'")]
    UnknownNode(String),
    #[error("dialogue is waiting for a different input")]
    InvalidDialogueState,
    #[error("dialogue choice {0} is unavailable")]
    InvalidChoice(usize),
    #[error("dialogue graph exceeded its automatic transition limit")]
    TransitionLimit,
}

impl StoryState {
    pub fn start_quest(&mut self, definition: &QuestDefinition) -> Result<(), NarrativeError> {
        if !definition
            .prerequisites
            .iter()
            .all(|condition| condition.evaluate(self))
        {
            return Err(NarrativeError::PrerequisitesNotMet(definition.id.clone()));
        }
        let progress = self.quests.entry(definition.id.clone()).or_default();
        progress.status = QuestStatus::Active;
        for objective in &definition.objectives {
            progress
                .objective_counts
                .entry(objective.id.clone())
                .or_insert(0);
        }
        Ok(())
    }

    pub fn advance_objective(
        &mut self,
        definition: &QuestDefinition,
        objective_id: &str,
        amount: u32,
    ) -> Result<bool, NarrativeError> {
        let objective = definition
            .objectives
            .iter()
            .find(|objective| objective.id == objective_id)
            .ok_or_else(|| NarrativeError::UnknownObjective {
                quest: definition.id.clone(),
                objective: objective_id.into(),
            })?;
        let progress = self
            .quests
            .get_mut(&definition.id)
            .ok_or_else(|| NarrativeError::QuestNotActive(definition.id.clone()))?;
        if progress.status != QuestStatus::Active {
            return Err(NarrativeError::QuestNotActive(definition.id.clone()));
        }
        let count = progress
            .objective_counts
            .entry(objective_id.into())
            .or_insert(0);
        *count = count.saturating_add(amount).min(objective.target_count);
        Ok(definition.objectives.iter().all(|objective| {
            progress
                .objective_counts
                .get(&objective.id)
                .copied()
                .unwrap_or(0)
                >= objective.target_count
        }))
    }

    pub fn complete_quest(
        &mut self,
        definition: &QuestDefinition,
    ) -> Result<Vec<NarrativeCommand>, NarrativeError> {
        let progress = self
            .quests
            .get_mut(&definition.id)
            .ok_or_else(|| NarrativeError::QuestNotActive(definition.id.clone()))?;
        if progress.status != QuestStatus::Active {
            return Err(NarrativeError::QuestNotActive(definition.id.clone()));
        }
        let objectives_complete = definition.objectives.iter().all(|objective| {
            progress
                .objective_counts
                .get(&objective.id)
                .copied()
                .unwrap_or(0)
                >= objective.target_count
        });
        if !objectives_complete {
            return Err(NarrativeError::ObjectivesIncomplete(definition.id.clone()));
        }
        progress.status = QuestStatus::Completed;
        Ok(definition.completion_commands.clone())
    }

    pub fn apply_effects(
        &mut self,
        effects: &[StoryEffect],
        quests: &BTreeMap<String, QuestDefinition>,
    ) -> Result<Vec<NarrativeCommand>, NarrativeError> {
        let mut commands = Vec::new();
        for effect in effects {
            match effect {
                StoryEffect::SetFlag { key, value } => {
                    self.flags.insert(key.clone(), value.clone());
                }
                StoryEffect::AddInteger { key, amount } => {
                    let current = match self.flags.get(key) {
                        Some(StoryValue::Integer(value)) => *value,
                        _ => 0,
                    };
                    self.flags.insert(
                        key.clone(),
                        StoryValue::Integer(current.saturating_add(*amount)),
                    );
                }
                StoryEffect::StartQuest { quest_id } => {
                    let definition = quests
                        .get(quest_id)
                        .ok_or_else(|| NarrativeError::UnknownQuest(quest_id.clone()))?;
                    self.start_quest(definition)?;
                }
                StoryEffect::AdvanceObjective {
                    quest_id,
                    objective_id,
                    amount,
                } => {
                    let definition = quests
                        .get(quest_id)
                        .ok_or_else(|| NarrativeError::UnknownQuest(quest_id.clone()))?;
                    let _ = self.advance_objective(definition, objective_id, *amount)?;
                }
                StoryEffect::CompleteQuest { quest_id } => {
                    let definition = quests
                        .get(quest_id)
                        .ok_or_else(|| NarrativeError::UnknownQuest(quest_id.clone()))?;
                    commands.extend(self.complete_quest(definition)?);
                }
                StoryEffect::FailQuest { quest_id } => {
                    let progress = self
                        .quests
                        .get_mut(quest_id)
                        .ok_or_else(|| NarrativeError::UnknownQuest(quest_id.clone()))?;
                    progress.status = QuestStatus::Failed;
                }
                StoryEffect::Emit { command } => commands.push(command.clone()),
            }
        }
        Ok(commands)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DialogueChoice {
    pub text_key: String,
    pub next: String,
    #[serde(default)]
    pub conditions: Vec<StoryCondition>,
    #[serde(default)]
    pub effects: Vec<StoryEffect>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum DialogueNode {
    Line {
        speaker_id: String,
        text_key: String,
        #[serde(default)]
        voice_asset: Option<String>,
        #[serde(default)]
        portrait_asset: Option<String>,
        next: String,
    },
    Choice {
        prompt_key: String,
        choices: Vec<DialogueChoice>,
    },
    Branch {
        branches: Vec<(StoryCondition, String)>,
        fallback: String,
    },
    Effects {
        effects: Vec<StoryEffect>,
        next: String,
    },
    End,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DialogueGraph {
    pub id: String,
    pub entry: String,
    pub nodes: BTreeMap<String, DialogueNode>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum DialogueBeat {
    Line {
        speaker_id: String,
        text_key: String,
        voice_asset: Option<String>,
        portrait_asset: Option<String>,
    },
    Choices {
        prompt_key: String,
        choices: Vec<(usize, String)>,
    },
    End {
        commands: Vec<NarrativeCommand>,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DialogueRunner {
    pub graph_id: String,
    pub current_node: String,
    #[serde(default)]
    pending_next: Option<String>,
    #[serde(default)]
    emitted_commands: Vec<NarrativeCommand>,
}

impl DialogueRunner {
    pub fn new(graph: &DialogueGraph) -> Self {
        Self {
            graph_id: graph.id.clone(),
            current_node: graph.entry.clone(),
            pending_next: None,
            emitted_commands: Vec::new(),
        }
    }

    pub fn beat(
        &mut self,
        graph: &DialogueGraph,
        story: &mut StoryState,
        quests: &BTreeMap<String, QuestDefinition>,
    ) -> Result<DialogueBeat, NarrativeError> {
        for _ in 0..128 {
            let node = graph
                .nodes
                .get(&self.current_node)
                .ok_or_else(|| NarrativeError::UnknownNode(self.current_node.clone()))?;
            match node {
                DialogueNode::Line {
                    speaker_id,
                    text_key,
                    voice_asset,
                    portrait_asset,
                    next,
                } => {
                    self.pending_next = Some(next.clone());
                    return Ok(DialogueBeat::Line {
                        speaker_id: speaker_id.clone(),
                        text_key: text_key.clone(),
                        voice_asset: voice_asset.clone(),
                        portrait_asset: portrait_asset.clone(),
                    });
                }
                DialogueNode::Choice {
                    prompt_key,
                    choices,
                } => {
                    let available = choices
                        .iter()
                        .enumerate()
                        .filter(|(_, choice)| {
                            choice
                                .conditions
                                .iter()
                                .all(|condition| condition.evaluate(story))
                        })
                        .map(|(index, choice)| (index, choice.text_key.clone()))
                        .collect();
                    return Ok(DialogueBeat::Choices {
                        prompt_key: prompt_key.clone(),
                        choices: available,
                    });
                }
                DialogueNode::Branch { branches, fallback } => {
                    self.current_node = branches
                        .iter()
                        .find(|(condition, _)| condition.evaluate(story))
                        .map(|(_, next)| next)
                        .unwrap_or(fallback)
                        .clone();
                }
                DialogueNode::Effects { effects, next } => {
                    self.emitted_commands
                        .extend(story.apply_effects(effects, quests)?);
                    self.current_node = next.clone();
                }
                DialogueNode::End => {
                    return Ok(DialogueBeat::End {
                        commands: std::mem::take(&mut self.emitted_commands),
                    });
                }
            }
        }
        Err(NarrativeError::TransitionLimit)
    }

    pub fn advance_line(&mut self) -> Result<(), NarrativeError> {
        self.current_node = self
            .pending_next
            .take()
            .ok_or(NarrativeError::InvalidDialogueState)?;
        Ok(())
    }

    pub fn choose(
        &mut self,
        graph: &DialogueGraph,
        story: &mut StoryState,
        quests: &BTreeMap<String, QuestDefinition>,
        index: usize,
    ) -> Result<Vec<NarrativeCommand>, NarrativeError> {
        let DialogueNode::Choice { choices, .. } = graph
            .nodes
            .get(&self.current_node)
            .ok_or_else(|| NarrativeError::UnknownNode(self.current_node.clone()))?
        else {
            return Err(NarrativeError::InvalidDialogueState);
        };
        let choice = choices
            .get(index)
            .ok_or(NarrativeError::InvalidChoice(index))?;
        if !choice
            .conditions
            .iter()
            .all(|condition| condition.evaluate(story))
        {
            return Err(NarrativeError::InvalidChoice(index));
        }
        let commands = story.apply_effects(&choice.effects, quests)?;
        self.emitted_commands.extend(commands.clone());
        self.current_node = choice.next.clone();
        Ok(commands)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn quest_progress_is_bounded_by_objective_target() {
        let definition = QuestDefinition {
            id: "slimes".into(),
            title_key: "quest.slimes.title".into(),
            description_key: "quest.slimes.body".into(),
            objectives: vec![QuestObjective {
                id: "defeat".into(),
                target_count: 3,
                text_key: "quest.slimes.objective".into(),
            }],
            prerequisites: vec![],
            completion_commands: vec![NarrativeCommand::GrantCurrency { amount: 100 }],
        };
        let mut state = StoryState::default();
        state.start_quest(&definition).unwrap();
        assert!(state.advance_objective(&definition, "defeat", 99).unwrap());
        assert_eq!(state.quests["slimes"].objective_counts["defeat"], 3);
        assert_eq!(
            state.complete_quest(&definition).unwrap(),
            vec![NarrativeCommand::GrantCurrency { amount: 100 }]
        );
    }

    #[test]
    fn dialogue_branches_and_emits_project_commands() {
        let graph = DialogueGraph {
            id: "intro".into(),
            entry: "branch".into(),
            nodes: BTreeMap::from([
                (
                    "branch".into(),
                    DialogueNode::Branch {
                        branches: vec![(
                            StoryCondition::FlagEquals {
                                key: "met".into(),
                                value: StoryValue::Bool(true),
                            },
                            "again".into(),
                        )],
                        fallback: "first".into(),
                    },
                ),
                (
                    "first".into(),
                    DialogueNode::Line {
                        speaker_id: "npc".into(),
                        text_key: "dialogue.first".into(),
                        voice_asset: None,
                        portrait_asset: None,
                        next: "end".into(),
                    },
                ),
                ("again".into(), DialogueNode::End),
                ("end".into(), DialogueNode::End),
            ]),
        };
        let mut runner = DialogueRunner::new(&graph);
        let mut story = StoryState::default();
        assert!(matches!(
            runner.beat(&graph, &mut story, &BTreeMap::new()).unwrap(),
            DialogueBeat::Line { .. }
        ));
        runner.advance_line().unwrap();
        assert!(matches!(
            runner.beat(&graph, &mut story, &BTreeMap::new()).unwrap(),
            DialogueBeat::End { .. }
        ));
    }
}
