use super::*;

pub(super) fn extract_animations(
    document: &gltf::Document,
    buffers: &[gltf::buffer::Data],
) -> Result<Vec<GltfAnimation>, GltfImportError> {
    use gltf::animation::{util::ReadOutputs, Property};

    let mut result = Vec::with_capacity(document.animations().len());
    for animation in document.animations() {
        let animation_index = animation.index();
        let mut duration = 0.0f32;
        let mut channels = Vec::new();
        for (channel_index, channel) in animation.channels().enumerate() {
            let interpolation = channel.sampler().interpolation();
            let target = channel.target();
            if target.property() == Property::MorphTargetWeights {
                return Err(GltfImportError::UnsupportedAnimationWeights {
                    animation_index,
                    channel_index,
                });
            }
            let reader = channel.reader(|buffer| Some(&buffers[buffer.index()]));
            let Some(inputs) = reader.read_inputs() else {
                return Err(GltfImportError::InvalidAnimationChannel {
                    animation_index,
                    channel_index,
                });
            };
            let source_times = inputs.collect::<Vec<_>>();
            let minimum_baked_key_count = match interpolation {
                gltf::animation::Interpolation::Step => {
                    source_times.len().saturating_mul(2).saturating_sub(1)
                }
                _ => source_times.len(),
            };
            if minimum_baked_key_count > MAX_BAKED_ANIMATION_KEYS_PER_CHANNEL {
                return Err(GltfImportError::AnimationKeyLimitExceeded {
                    animation_index,
                    channel_index,
                    keys: minimum_baked_key_count,
                    max: MAX_BAKED_ANIMATION_KEYS_PER_CHANNEL,
                });
            }
            let source_times_valid = source_times
                .iter()
                .all(|time| time.is_finite() && *time >= 0.0)
                && source_times.windows(2).all(|pair| pair[0] <= pair[1]);
            if !source_times_valid {
                return Err(GltfImportError::InvalidAnimationChannel {
                    animation_index,
                    channel_index,
                });
            }
            let Some(outputs) = reader.read_outputs() else {
                return Err(GltfImportError::InvalidAnimationChannel {
                    animation_index,
                    channel_index,
                });
            };
            let (times, property, values) = match outputs {
                ReadOutputs::Translations(values) => {
                    let values = values.collect::<Vec<_>>();
                    validate_animation_output_count(
                        animation_index,
                        channel_index,
                        source_times.len(),
                        values.len(),
                        interpolation,
                    )?;
                    let (times, values) =
                        bake_vec3_animation_track(&source_times, &values, interpolation);
                    (
                        times,
                        GltfAnimationProperty::Translation,
                        GltfAnimationValues::Translations(values),
                    )
                }
                ReadOutputs::Rotations(values) => {
                    let values = values.into_f32().collect::<Vec<_>>();
                    validate_animation_output_count(
                        animation_index,
                        channel_index,
                        source_times.len(),
                        values.len(),
                        interpolation,
                    )?;
                    let (times, values) =
                        bake_quaternion_animation_track(&source_times, &values, interpolation);
                    (
                        times,
                        GltfAnimationProperty::Rotation,
                        GltfAnimationValues::Rotations(values),
                    )
                }
                ReadOutputs::Scales(values) => {
                    let values = values.collect::<Vec<_>>();
                    validate_animation_output_count(
                        animation_index,
                        channel_index,
                        source_times.len(),
                        values.len(),
                        interpolation,
                    )?;
                    let (times, values) =
                        bake_vec3_animation_track(&source_times, &values, interpolation);
                    (
                        times,
                        GltfAnimationProperty::Scale,
                        GltfAnimationValues::Scales(values),
                    )
                }
                ReadOutputs::MorphTargetWeights(_) => {
                    return Err(GltfImportError::UnsupportedAnimationWeights {
                        animation_index,
                        channel_index,
                    });
                }
            };
            if times.len() > MAX_BAKED_ANIMATION_KEYS_PER_CHANNEL {
                return Err(GltfImportError::AnimationKeyLimitExceeded {
                    animation_index,
                    channel_index,
                    keys: times.len(),
                    max: MAX_BAKED_ANIMATION_KEYS_PER_CHANNEL,
                });
            }
            let times_valid = times.iter().all(|time| time.is_finite() && *time >= 0.0)
                && times.windows(2).all(|pair| pair[0] <= pair[1]);
            let values_valid = match &values {
                GltfAnimationValues::Translations(values) | GltfAnimationValues::Scales(values) => {
                    values.iter().flatten().all(|value| value.is_finite())
                }
                GltfAnimationValues::Rotations(values) => values.iter().all(|value| {
                    value.iter().all(|component| component.is_finite())
                        && value
                            .iter()
                            .map(|component| component * component)
                            .sum::<f32>()
                            > f32::EPSILON
                }),
            };
            if !times_valid || !values_valid {
                return Err(GltfImportError::InvalidAnimationChannel {
                    animation_index,
                    channel_index,
                });
            }
            if let Some(last) = times.last() {
                duration = duration.max(*last);
            }
            channels.push(GltfAnimationChannel {
                target_node_index: target.node().index(),
                property,
                times,
                values,
            });
        }
        result.push(GltfAnimation {
            source_animation_index: animation_index,
            name: animation
                .name()
                .map(str::to_owned)
                .unwrap_or_else(|| format!("animation-{animation_index}")),
            duration,
            channels,
        });
    }
    Ok(result)
}

fn validate_animation_output_count(
    animation_index: usize,
    channel_index: usize,
    input_count: usize,
    output_count: usize,
    interpolation: gltf::animation::Interpolation,
) -> Result<(), GltfImportError> {
    let expected = match interpolation {
        gltf::animation::Interpolation::Linear | gltf::animation::Interpolation::Step => {
            input_count
        }
        gltf::animation::Interpolation::CubicSpline => input_count.saturating_mul(3),
    };
    if output_count == expected {
        Ok(())
    } else {
        Err(GltfImportError::AnimationKeyCountMismatch {
            animation_index,
            channel_index,
            inputs: input_count,
            outputs: output_count,
        })
    }
}

pub(super) fn bake_vec3_animation_track(
    times: &[f32],
    values: &[[f32; 3]],
    interpolation: gltf::animation::Interpolation,
) -> (Vec<f32>, Vec<[f32; 3]>) {
    match interpolation {
        gltf::animation::Interpolation::Linear => (times.to_vec(), values.to_vec()),
        gltf::animation::Interpolation::Step => bake_step_animation_track(times, values),
        gltf::animation::Interpolation::CubicSpline => {
            bake_cubic_vec3_animation_track(times, values)
        }
    }
}

pub(super) fn bake_quaternion_animation_track(
    times: &[f32],
    values: &[[f32; 4]],
    interpolation: gltf::animation::Interpolation,
) -> (Vec<f32>, Vec<[f32; 4]>) {
    match interpolation {
        gltf::animation::Interpolation::Linear => (
            times.to_vec(),
            values
                .iter()
                .copied()
                .map(normalize_animation_quaternion)
                .collect(),
        ),
        gltf::animation::Interpolation::Step => {
            let values = values
                .iter()
                .copied()
                .map(normalize_animation_quaternion)
                .collect::<Vec<_>>();
            bake_step_animation_track(times, &values)
        }
        gltf::animation::Interpolation::CubicSpline => {
            bake_cubic_quaternion_animation_track(times, values)
        }
    }
}

/// Preserve STEP semantics in a linear-only runtime by inserting a held value
/// immediately before every discontinuity.
fn bake_step_animation_track<T: Copy>(times: &[f32], values: &[T]) -> (Vec<f32>, Vec<T>) {
    if times.is_empty() {
        return (Vec::new(), Vec::new());
    }
    let mut baked_times = Vec::with_capacity(times.len().saturating_mul(2));
    let mut baked_values = Vec::with_capacity(values.len().saturating_mul(2));
    baked_times.push(times[0]);
    baked_values.push(values[0]);
    for index in 1..times.len() {
        let hold_time = times[index].next_down();
        if hold_time > times[index - 1] {
            baked_times.push(hold_time);
            baked_values.push(values[index - 1]);
        }
        baked_times.push(times[index]);
        baked_values.push(values[index]);
    }
    (baked_times, baked_values)
}

const CUBIC_SPLINE_SAMPLES_PER_SECOND: f32 = 60.0;
const MAX_CUBIC_SPLINE_SAMPLES_PER_SEGMENT: usize = 1024;
const MAX_BAKED_ANIMATION_KEYS_PER_CHANNEL: usize = 65_536;

fn bake_cubic_vec3_animation_track(
    times: &[f32],
    values: &[[f32; 3]],
) -> (Vec<f32>, Vec<[f32; 3]>) {
    if times.is_empty() {
        return (Vec::new(), Vec::new());
    }
    let mut baked_times = vec![times[0]];
    let mut baked_values = vec![values[1]];
    let segment_count = times.len().saturating_sub(1);
    for segment in 0..segment_count {
        let start_time = times[segment];
        let end_time = times[segment + 1];
        let duration = end_time - start_time;
        if duration <= 0.0 {
            baked_times.push(end_time);
            baked_values.push(values[(segment + 1) * 3 + 1]);
            continue;
        }
        let steps = cubic_segment_steps(duration, segment_count);
        let p0 = glam::Vec3::from_array(values[segment * 3 + 1]);
        let m0 = glam::Vec3::from_array(values[segment * 3 + 2]) * duration;
        let p1 = glam::Vec3::from_array(values[(segment + 1) * 3 + 1]);
        let m1 = glam::Vec3::from_array(values[(segment + 1) * 3]) * duration;
        for step in 1..=steps {
            let t = step as f32 / steps as f32;
            baked_times.push(start_time + duration * t);
            baked_values.push(hermite_vec3(p0, m0, p1, m1, t).to_array());
        }
    }
    (baked_times, baked_values)
}

fn bake_cubic_quaternion_animation_track(
    times: &[f32],
    values: &[[f32; 4]],
) -> (Vec<f32>, Vec<[f32; 4]>) {
    if times.is_empty() {
        return (Vec::new(), Vec::new());
    }
    let mut baked_times = vec![times[0]];
    let mut baked_values = vec![normalize_animation_quaternion(values[1])];
    let segment_count = times.len().saturating_sub(1);
    for segment in 0..segment_count {
        let start_time = times[segment];
        let end_time = times[segment + 1];
        let duration = end_time - start_time;
        if duration <= 0.0 {
            baked_times.push(end_time);
            baked_values.push(normalize_animation_quaternion(
                values[(segment + 1) * 3 + 1],
            ));
            continue;
        }
        let steps = cubic_segment_steps(duration, segment_count);
        let p0 = glam::Vec4::from_array(values[segment * 3 + 1]);
        let m0 = glam::Vec4::from_array(values[segment * 3 + 2]) * duration;
        let p1 = glam::Vec4::from_array(values[(segment + 1) * 3 + 1]);
        let m1 = glam::Vec4::from_array(values[(segment + 1) * 3]) * duration;
        for step in 1..=steps {
            let t = step as f32 / steps as f32;
            baked_times.push(start_time + duration * t);
            baked_values.push(normalize_animation_quaternion(
                hermite_vec4(p0, m0, p1, m1, t).to_array(),
            ));
        }
    }
    (baked_times, baked_values)
}

fn cubic_segment_steps(duration: f32, segment_count: usize) -> usize {
    let total_budget_per_segment = MAX_BAKED_ANIMATION_KEYS_PER_CHANNEL
        .saturating_sub(1)
        .checked_div(segment_count.max(1))
        .unwrap_or(1)
        .max(1);
    ((duration * CUBIC_SPLINE_SAMPLES_PER_SECOND).ceil() as usize).clamp(
        1,
        MAX_CUBIC_SPLINE_SAMPLES_PER_SEGMENT.min(total_budget_per_segment),
    )
}

fn hermite_vec3(
    p0: glam::Vec3,
    m0: glam::Vec3,
    p1: glam::Vec3,
    m1: glam::Vec3,
    t: f32,
) -> glam::Vec3 {
    let t2 = t * t;
    let t3 = t2 * t;
    p0 * (2.0 * t3 - 3.0 * t2 + 1.0)
        + m0 * (t3 - 2.0 * t2 + t)
        + p1 * (-2.0 * t3 + 3.0 * t2)
        + m1 * (t3 - t2)
}

fn hermite_vec4(
    p0: glam::Vec4,
    m0: glam::Vec4,
    p1: glam::Vec4,
    m1: glam::Vec4,
    t: f32,
) -> glam::Vec4 {
    let t2 = t * t;
    let t3 = t2 * t;
    p0 * (2.0 * t3 - 3.0 * t2 + 1.0)
        + m0 * (t3 - 2.0 * t2 + t)
        + p1 * (-2.0 * t3 + 3.0 * t2)
        + m1 * (t3 - t2)
}

fn normalize_animation_quaternion(value: [f32; 4]) -> [f32; 4] {
    let rotation = glam::Quat::from_array(value);
    let length_squared = rotation.length_squared();
    if rotation.is_finite() && length_squared.is_finite() && length_squared > f32::EPSILON {
        (rotation / length_squared.sqrt()).to_array()
    } else {
        value
    }
}
