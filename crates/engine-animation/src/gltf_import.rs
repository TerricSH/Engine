use std::collections::{BTreeMap, HashMap};

use engine_asset::gltf::{GltfAnimationProperty, GltfAnimationValues, GltfScene, GltfSkin};

use crate::assets::{AnimationChannel, AnimationClip, Joint, JointTransform, Keyframe, Skeleton};

/// Animation assets associated with one skin in a glTF document.
#[derive(Clone, Debug, PartialEq)]
pub struct ImportedGltfSkin {
    pub source_skin_index: usize,
    pub name: String,
    pub skeleton: Skeleton,
    pub animations: Vec<AnimationClip>,
}

/// Convert every glTF skin and all animation channels targeting its joints into
/// the engine's serializable skeleton and animation asset types.
pub fn import_gltf_animation_assets(scene: &GltfScene) -> Result<Vec<ImportedGltfSkin>, String> {
    scene
        .skins
        .iter()
        .map(|skin| {
            let skeleton = skeleton_from_gltf_skin(skin)?;
            let source_node_to_joint: HashMap<usize, u32> = skin
                .joints
                .iter()
                .enumerate()
                .map(|(joint_index, joint)| (joint.source_node_index, joint_index as u32))
                .collect();

            let mut animations = Vec::new();
            for animation in &scene.animations {
                let mut channels = BTreeMap::<u32, AnimationChannel>::new();
                for source_channel in &animation.channels {
                    let Some(&joint_index) =
                        source_node_to_joint.get(&source_channel.target_node_index)
                    else {
                        continue;
                    };
                    let channel = channels
                        .entry(joint_index)
                        .or_insert_with(|| AnimationChannel {
                            joint_index,
                            translations: Vec::new(),
                            rotations: Vec::new(),
                            scales: Vec::new(),
                        });

                    match (source_channel.property, &source_channel.values) {
                        (
                            GltfAnimationProperty::Translation,
                            GltfAnimationValues::Translations(values),
                        ) => set_track(
                            &mut channel.translations,
                            &source_channel.times,
                            values,
                            &animation.name,
                            joint_index,
                            "translation",
                        )?,
                        (
                            GltfAnimationProperty::Rotation,
                            GltfAnimationValues::Rotations(values),
                        ) => set_track(
                            &mut channel.rotations,
                            &source_channel.times,
                            values,
                            &animation.name,
                            joint_index,
                            "rotation",
                        )?,
                        (GltfAnimationProperty::Scale, GltfAnimationValues::Scales(values)) => {
                            set_track(
                                &mut channel.scales,
                                &source_channel.times,
                                values,
                                &animation.name,
                                joint_index,
                                "scale",
                            )?
                        }
                        _ => {
                            return Err(format!(
                                "animation '{}' has mismatched values for joint {} {:?}",
                                animation.name, joint_index, source_channel.property
                            ));
                        }
                    }
                }

                if channels.is_empty() {
                    continue;
                }
                let channels: Vec<_> = channels.into_values().collect();
                let clip = AnimationClip {
                    name: animation.name.clone(),
                    duration: animation.duration,
                    joint_indices: channels.iter().map(|channel| channel.joint_index).collect(),
                    channels,
                };
                clip.validate().map_err(|detail| {
                    format!(
                        "glTF animation {} ('{}') is invalid for skin {}: {detail}",
                        animation.source_animation_index, animation.name, skin.source_skin_index
                    )
                })?;
                animations.push(clip);
            }

            Ok(ImportedGltfSkin {
                source_skin_index: skin.source_skin_index,
                name: skin.name.clone(),
                skeleton,
                animations,
            })
        })
        .collect()
}

/// Convert one neutral glTF skin into the serializable engine skeleton format.
pub fn skeleton_from_gltf_skin(skin: &GltfSkin) -> Result<Skeleton, String> {
    let skeleton = Skeleton {
        joints: skin
            .joints
            .iter()
            .map(|joint| Joint {
                name: joint.name.clone(),
                parent_index: joint.parent_index,
                local_transform: JointTransform {
                    translation: joint.translation,
                    rotation: joint.rotation,
                    scale: joint.scale,
                },
            })
            .collect(),
        inverse_bind_matrices: skin
            .joints
            .iter()
            .map(|joint| joint.inverse_bind_matrix)
            .collect(),
    };
    skeleton.validate().map_err(|detail| {
        format!(
            "glTF skin {} ('{}') is invalid: {detail}",
            skin.source_skin_index, skin.name
        )
    })?;
    Ok(skeleton)
}

fn set_track<T: Copy>(
    destination: &mut Vec<Keyframe<T>>,
    times: &[f32],
    values: &[T],
    animation_name: &str,
    joint_index: u32,
    property: &str,
) -> Result<(), String> {
    if !destination.is_empty() {
        return Err(format!(
            "animation '{animation_name}' contains duplicate {property} tracks for joint {joint_index}"
        ));
    }
    if times.len() != values.len() {
        return Err(format!(
            "animation '{animation_name}' {property} track for joint {joint_index} has {} times but {} values",
            times.len(),
            values.len()
        ));
    }
    destination.extend(
        times
            .iter()
            .copied()
            .zip(values.iter().copied())
            .map(|(time, value)| Keyframe { time, value }),
    );
    Ok(())
}

#[cfg(test)]
mod tests {
    use engine_asset::gltf::{
        GltfAnimation, GltfAnimationChannel, GltfAnimationProperty, GltfAnimationValues, GltfScene,
        GltfSkin, GltfSkinJoint,
    };

    use super::import_gltf_animation_assets;

    const IDENTITY: [[f32; 4]; 4] = [
        [1.0, 0.0, 0.0, 0.0],
        [0.0, 1.0, 0.0, 0.0],
        [0.0, 0.0, 1.0, 0.0],
        [0.0, 0.0, 0.0, 1.0],
    ];

    #[test]
    fn converts_skin_hierarchy_inverse_binds_and_matching_animation_tracks() {
        let scene = GltfScene {
            selected_scene_index: Some(0),
            primitives: Vec::new(),
            materials: Vec::new(),
            textures: Vec::new(),
            nodes: Vec::new(),
            roots: Vec::new(),
            skins: vec![GltfSkin {
                source_skin_index: 2,
                name: "Rig".into(),
                skeleton_root_node: Some(4),
                joints: vec![
                    GltfSkinJoint {
                        source_node_index: 4,
                        source_joint_slot: 1,
                        name: "Root".into(),
                        parent_index: None,
                        translation: [0.0, 1.0, 0.0],
                        rotation: [0.0, 0.0, 0.0, 1.0],
                        scale: [1.0; 3],
                        inverse_bind_matrix: IDENTITY,
                    },
                    GltfSkinJoint {
                        source_node_index: 8,
                        source_joint_slot: 0,
                        name: "Hand".into(),
                        parent_index: Some(0),
                        translation: [0.0, 2.0, 0.0],
                        rotation: [0.0, 0.0, 0.0, 1.0],
                        scale: [1.0; 3],
                        inverse_bind_matrix: IDENTITY,
                    },
                ],
                joint_remap: vec![1, 0],
            }],
            animations: vec![GltfAnimation {
                source_animation_index: 3,
                name: "Wave".into(),
                duration: 1.0,
                channels: vec![
                    GltfAnimationChannel {
                        target_node_index: 8,
                        property: GltfAnimationProperty::Translation,
                        times: vec![0.0, 1.0],
                        values: GltfAnimationValues::Translations(vec![
                            [0.0, 2.0, 0.0],
                            [1.0, 2.0, 0.0],
                        ]),
                    },
                    GltfAnimationChannel {
                        target_node_index: 99,
                        property: GltfAnimationProperty::Scale,
                        times: vec![0.0],
                        values: GltfAnimationValues::Scales(vec![[2.0; 3]]),
                    },
                ],
            }],
        };

        let imported = import_gltf_animation_assets(&scene).unwrap();
        assert_eq!(imported.len(), 1);
        assert_eq!(imported[0].skeleton.joints[1].parent_index, Some(0));
        assert_eq!(
            imported[0].skeleton.inverse_bind_matrices,
            vec![IDENTITY; 2]
        );
        assert_eq!(imported[0].animations.len(), 1);
        assert_eq!(imported[0].animations[0].channels.len(), 1);
        assert_eq!(imported[0].animations[0].channels[0].joint_index, 1);
        assert_eq!(
            imported[0].animations[0].channels[0].translations[1].value,
            [1.0, 2.0, 0.0]
        );
    }
}
