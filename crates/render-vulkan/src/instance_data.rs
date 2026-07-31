//! Backend-neutral draw batches converted to Vulkan instance streams.
//!
//! The packing code is pure and independently testable; `SceneRenderer`
//! remains responsible for allocating/uploading the resulting byte streams.

use engine_renderer::{ParticleBatch, RenderableItem};

pub(crate) const VFX_INSTANCE_STRIDE: u64 = 32;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct PreparedParticleDraw {
    pub(crate) batch_index: usize,
    pub(crate) first_instance: u32,
    pub(crate) instance_count: u32,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct PreparedParticleInstances {
    pub(crate) instance_bytes: Vec<u8>,
    pub(crate) draws: Vec<PreparedParticleDraw>,
}

pub(crate) fn prepare_particle_instances(
    batches: &[ParticleBatch],
) -> Result<PreparedParticleInstances, String> {
    let total_instances = batches
        .iter()
        .try_fold(0usize, |total, batch| {
            total.checked_add(batch.instances.len())
        })
        .ok_or_else(|| "particle instance count overflow".to_string())?;
    let total_bytes = total_instances
        .checked_mul(VFX_INSTANCE_STRIDE as usize)
        .ok_or_else(|| "particle instance byte size overflow".to_string())?;
    let mut prepared = PreparedParticleInstances {
        instance_bytes: Vec::with_capacity(total_bytes),
        draws: Vec::with_capacity(batches.len()),
    };
    let mut first_instance = 0_u32;
    for (batch_index, batch) in batches.iter().enumerate() {
        if batch.instances.is_empty() {
            continue;
        }
        let instance_count = u32::try_from(batch.instances.len())
            .map_err(|_| format!("particle batch {batch_index} exceeds u32 instance capacity"))?;
        prepared.draws.push(PreparedParticleDraw {
            batch_index,
            first_instance,
            instance_count,
        });
        first_instance = first_instance
            .checked_add(instance_count)
            .ok_or_else(|| "total particle instance count exceeds u32".to_string())?;
        for instance in &batch.instances {
            for value in [
                instance.position[0],
                instance.position[1],
                instance.position[2],
                instance.size,
                instance.rotation_radians,
                instance.normalized_age,
            ] {
                prepared
                    .instance_bytes
                    .extend_from_slice(&value.to_ne_bytes());
            }
            prepared.instance_bytes.extend_from_slice(&instance.color);
            prepared
                .instance_bytes
                .extend_from_slice(&0_u32.to_ne_bytes());
        }
    }
    Ok(prepared)
}

pub(crate) const STATIC_INSTANCE_STRIDE: u64 = 64;

/// Pack the complete static-surface push block consumed by `forward.vert`.
///
/// The final two vectors are deliberately per draw rather than per material:
/// one material may be shared by many terrain chunks with different local
/// projection origins.
pub(crate) fn static_draw_push_constants(drawable: &RenderableItem) -> [u8; 128] {
    let mut bytes = [0_u8; 128];
    let mut write_values = |offset: usize, values: &[f32]| {
        for (index, value) in values.iter().copied().enumerate() {
            let start = offset + index * std::mem::size_of::<f32>();
            bytes[start..start + 4].copy_from_slice(&value.to_ne_bytes());
        }
    };
    write_values(0, &drawable.world_transform);
    if let Some(morph) = &drawable.radial_vertex_morph {
        write_values(64, &[morph.factor, morph.delta_scale, 1.0, 0.0]);
        write_values(
            80,
            &[
                morph.local_origin[0],
                morph.local_origin[1],
                morph.local_origin[2],
                0.0,
            ],
        );
    }
    if let Some(mapping) = &drawable.triplanar_material_mapping {
        if mapping.meters_per_tile.is_finite()
            && mapping.meters_per_tile > 0.0
            && mapping.blend_sharpness.is_finite()
            && mapping.local_origin.into_iter().all(f32::is_finite)
        {
            write_values(
                96,
                &[
                    1.0,
                    mapping.meters_per_tile.recip(),
                    mapping.blend_sharpness.clamp(1.0, 32.0),
                    0.0,
                ],
            );
            write_values(
                112,
                &[
                    mapping.local_origin[0],
                    mapping.local_origin[1],
                    mapping.local_origin[2],
                    0.0,
                ],
            );
        }
    }
    bytes
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct PreparedStaticDraw {
    pub(crate) first_drawable: usize,
    pub(crate) drawable_count: usize,
    pub(crate) first_instance: u32,
    pub(crate) instance_count: u32,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct PreparedStaticInstances {
    pub(crate) instance_bytes: Vec<u8>,
    pub(crate) draws: Vec<PreparedStaticDraw>,
}

pub(crate) fn prepare_static_instances(
    drawables: &[&RenderableItem],
) -> Result<PreparedStaticInstances, String> {
    let estimated_bytes = drawables
        .len()
        .checked_mul(STATIC_INSTANCE_STRIDE as usize)
        .ok_or_else(|| "static instance byte size overflow".to_string())?;
    let mut prepared = PreparedStaticInstances {
        instance_bytes: Vec::with_capacity(estimated_bytes),
        draws: Vec::new(),
    };
    let mut cursor = 0_usize;
    let mut first_instance = 0_u32;
    while cursor < drawables.len() {
        let first = drawables[cursor];
        let mut end = cursor + 1;
        while end < drawables.len()
            && drawables[end].mesh == first.mesh
            && drawables[end].material == first.material
            && first.radial_vertex_morph.is_none()
            && drawables[end].radial_vertex_morph.is_none()
            && first.triplanar_material_mapping.is_none()
            && drawables[end].triplanar_material_mapping.is_none()
        {
            end += 1;
        }
        let count = end - cursor;
        if count >= 2 {
            let instance_count = u32::try_from(count)
                .map_err(|_| "static instance batch exceeds u32 capacity".to_string())?;
            prepared.draws.push(PreparedStaticDraw {
                first_drawable: cursor,
                drawable_count: count,
                first_instance,
                instance_count,
            });
            first_instance = first_instance
                .checked_add(instance_count)
                .ok_or_else(|| "total static instance count exceeds u32".to_string())?;
            for drawable in &drawables[cursor..end] {
                for value in drawable.world_transform {
                    prepared
                        .instance_bytes
                        .extend_from_slice(&value.to_ne_bytes());
                }
            }
        }
        cursor = end;
    }
    Ok(prepared)
}
