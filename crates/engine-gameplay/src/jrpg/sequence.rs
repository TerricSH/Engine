use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use super::StoryValue;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum SequenceCommand {
    PlayAnimation {
        entity_id: String,
        clip_asset: String,
        #[serde(default)]
        looping: bool,
        #[serde(default = "default_speed")]
        speed: f32,
    },
    PlayAudio {
        entity_id: String,
        clip_asset: String,
        #[serde(default = "default_volume")]
        volume: f32,
        #[serde(default)]
        looping: bool,
    },
    ActivateCamera {
        camera_entity_id: String,
        #[serde(default)]
        blend_millis: u32,
    },
    Fade {
        from: f32,
        to: f32,
        duration_millis: u32,
    },
    StartDialogue {
        dialogue_id: String,
    },
    LoadScene {
        scene_id: String,
    },
    Custom {
        command: String,
        #[serde(default)]
        payload: BTreeMap<String, StoryValue>,
    },
}

fn default_speed() -> f32 {
    1.0
}

fn default_volume() -> f32 {
    1.0
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum SequenceStep {
    Wait { duration_millis: u32 },
    Emit { command: SequenceCommand },
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SequenceDefinition {
    pub id: String,
    #[serde(default)]
    pub steps: Vec<SequenceStep>,
}

/// Save-friendly interpreter for authored cutscene steps.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct SequenceRunner {
    pub sequence_id: String,
    pub next_step: usize,
    pub wait_remaining_millis: u32,
    pub finished: bool,
}

impl SequenceRunner {
    pub fn new(sequence: &SequenceDefinition) -> Self {
        Self {
            sequence_id: sequence.id.clone(),
            next_step: 0,
            wait_remaining_millis: 0,
            finished: sequence.steps.is_empty(),
        }
    }

    pub fn advance(
        &mut self,
        sequence: &SequenceDefinition,
        mut delta_millis: u32,
    ) -> Vec<SequenceCommand> {
        if self.finished || self.sequence_id != sequence.id {
            return Vec::new();
        }
        let mut commands = Vec::new();
        loop {
            if self.wait_remaining_millis > 0 {
                let consumed = delta_millis.min(self.wait_remaining_millis);
                self.wait_remaining_millis -= consumed;
                delta_millis -= consumed;
                if self.wait_remaining_millis > 0 {
                    break;
                }
            }
            let Some(step) = sequence.steps.get(self.next_step) else {
                self.finished = true;
                break;
            };
            self.next_step += 1;
            match step {
                SequenceStep::Wait { duration_millis } => {
                    self.wait_remaining_millis = *duration_millis;
                    if delta_millis == 0 && self.wait_remaining_millis > 0 {
                        break;
                    }
                }
                SequenceStep::Emit { command } => commands.push(command.clone()),
            }
        }
        commands
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sequence_waits_and_resumes_from_snapshot_state() {
        let sequence = SequenceDefinition {
            id: "intro".into(),
            steps: vec![
                SequenceStep::Emit {
                    command: SequenceCommand::StartDialogue {
                        dialogue_id: "opening".into(),
                    },
                },
                SequenceStep::Wait {
                    duration_millis: 100,
                },
                SequenceStep::Emit {
                    command: SequenceCommand::LoadScene {
                        scene_id: "field".into(),
                    },
                },
            ],
        };
        let mut runner = SequenceRunner::new(&sequence);
        assert_eq!(runner.advance(&sequence, 0).len(), 1);
        assert!(runner.advance(&sequence, 99).is_empty());
        assert_eq!(runner.advance(&sequence, 1).len(), 1);
        assert!(runner.finished);
    }
}
