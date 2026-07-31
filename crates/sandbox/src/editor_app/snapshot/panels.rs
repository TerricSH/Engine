impl EditorApp {
    pub(super) fn asset_browser_snapshot(&self) -> AssetBrowserDto {
        AssetBrowserDto {
            query: self.asset_browser.search_query().to_string(),
            folder: self.asset_browser.current_folder().to_string(),
            kind_filter: self.asset_browser.kind_filter().label().to_string(),
            view: match self.workspace_preferences.project_asset_view {
                ProjectAssetView::Grid => "grid",
                ProjectAssetView::List => "list",
            }
            .into(),
            page: self.asset_browser.page(),
            page_size: self.asset_browser.page_size(),
            page_count: self.asset_browser.page_count(),
            total: self.asset_browser.assets().len(),
            visible_asset_ids: self
                .asset_browser
                .visible_assets()
                .iter()
                .map(|asset| asset.id.id.clone())
                .collect(),
            folders: self
                .asset_browser
                .folders()
                .iter()
                .map(|folder| AssetFolderDto {
                    path: folder.path.clone(),
                    name: folder.name.clone(),
                    depth: folder.depth,
                    direct_asset_count: folder.direct_asset_count,
                })
                .collect(),
            selected_asset: self
                .asset_browser
                .selected_asset()
                .map(|asset| asset.id.clone()),
        }
    }

    pub(super) fn material_snapshot(&self) -> MaterialDto {
        let (writable, read_only_reason) = match self.material_editor.save_access() {
            MaterialSaveAccess::Writable => (true, None),
            MaterialSaveAccess::ReadOnly(reason) => (false, Some(reason.clone())),
        };
        MaterialDto {
            selected_material: self.material_editor.selected_material.clone(),
            parameters: self
                .material_editor
                .shader_params
                .iter()
                .map(|parameter| {
                    let (kind, value, options) = match parameter.param_type {
                        ShaderParamType::Float => {
                            ("float", json!(parameter.float_value), Vec::new())
                        }
                        ShaderParamType::Color => {
                            ("color", json!(parameter.color_value), Vec::new())
                        }
                        ShaderParamType::Texture => {
                            ("texture", json!(parameter.texture_value), Vec::new())
                        }
                        ShaderParamType::Choice => (
                            "choice",
                            json!(parameter.choice_value),
                            parameter.choice_options.clone(),
                        ),
                        ShaderParamType::Bool => ("bool", json!(parameter.bool_value), Vec::new()),
                    };
                    MaterialParameterDto {
                        name: parameter.name.clone(),
                        kind,
                        value,
                        options,
                    }
                })
                .collect(),
            writable,
            read_only_reason,
            save_status: self.material_editor.save_status().map(str::to_string),
        }
    }

    pub(super) fn animation_snapshot(&self) -> AnimationDto {
        AnimationDto {
            available_skeletons: self.animation_preview.available_skeletons.clone(),
            available_clips: self.animation_preview.available_clips.clone(),
            selected_skeleton: self.animation_preview.selected_skeleton.clone(),
            selected_clip: self.animation_preview.selected_clip.clone(),
            playback_time: self.animation_preview.playback_time,
            duration: self
                .animation_preview
                .clip_info()
                .map_or(0.0, |info| info.duration),
            playing: self.animation_preview.playing,
            looping: self.animation_preview.looping,
            speed: self.animation_preview.speed,
            events: self
                .animation_preview
                .events
                .iter()
                .map(|event| AnimationEventDto {
                    time: event.time,
                    name: event.name.clone(),
                })
                .collect(),
        }
    }

    pub(super) fn terrain_snapshot(&self, entities: &[EntityRecord]) -> TerrainDto {
        let defaults = engine_terrain::TerrainVolume::default();
        let authored = entities.iter().find_map(|entity| {
            entity
                .components
                .get("engine.terrain_volume")
                .map(|component| (entity.persistent_id.clone(), component))
        });
        let (runtime, last_error) = self.game_loop.as_ref().map_or_else(
            || (TerrainRuntimeStatsDto::default(), None),
            |game_loop| {
                let stats = game_loop.terrain_debug_snapshot().stats;
                (
                    TerrainRuntimeStatsDto {
                        queued: stats.queued,
                        generating: stats.generating,
                        ready_to_commit: stats.ready_to_commit,
                        resident: stats.resident,
                        failed: stats.failed,
                        resident_bytes: stats.resident_bytes,
                        stale_results_discarded: stats.stale_results_discarded,
                        cancelled: stats.cancelled,
                        generated: stats.generated,
                        committed: stats.committed,
                        evicted: stats.evicted,
                        last_tick_committed_bytes: stats.last_tick_committed_bytes,
                        last_generation_micros: stats.last_generation_micros,
                    },
                    game_loop.terrain.binding_stats().last_error.clone(),
                )
            },
        );
        let Some((entity_id, component)) = authored else {
            return TerrainDto {
                available: false,
                entity_id: None,
                enabled: defaults.enabled,
                seed: defaults.seed.to_string(),
                chunk_size: defaults.chunk_size,
                base_resolution: defaults.base_resolution,
                height_scale: defaults.height_scale,
                frequency: defaults.frequency,
                octaves: defaults.octaves,
                lacunarity: defaults.lacunarity,
                gain: defaults.gain,
                domain_warp_amplitude: defaults.domain_warp_amplitude,
                domain_warp_frequency: defaults.domain_warp_frequency,
                skirt_depth: defaults.skirt_depth,
                collision_enabled: defaults.collision_enabled,
                lod_distances: defaults.lod_distances,
                lod_hysteresis: defaults.lod_hysteresis,
                runtime,
                last_error,
            };
        };
        let fields = &component.fields;
        let float = |name: &str, fallback: f32| match fields.get(name) {
            Some(Value::Float32(value)) => *value,
            Some(Value::Float64(value)) => *value as f32,
            _ => fallback,
        };
        let uint = |name: &str, fallback: u64| match fields.get(name) {
            Some(Value::UInt(value)) => *value,
            Some(Value::Int(value)) if *value >= 0 => *value as u64,
            _ => fallback,
        };
        let boolean = |name: &str, fallback: bool| match fields.get(name) {
            Some(Value::Bool(value)) => *value,
            _ => fallback,
        };
        let lod_distances = match fields.get("lod_distances") {
            Some(Value::List(values)) => values
                .iter()
                .filter_map(|value| match value {
                    Value::Float32(value) => Some(*value),
                    Value::Float64(value) => Some(*value as f32),
                    _ => None,
                })
                .collect(),
            _ => defaults.lod_distances,
        };
        TerrainDto {
            available: true,
            entity_id: Some(entity_id),
            enabled: boolean("enabled", defaults.enabled),
            seed: uint("seed", defaults.seed).to_string(),
            chunk_size: float("chunk_size", defaults.chunk_size),
            base_resolution: uint("base_resolution", u64::from(defaults.base_resolution)) as u32,
            height_scale: float("height_scale", defaults.height_scale),
            frequency: float("frequency", defaults.frequency),
            octaves: uint("octaves", u64::from(defaults.octaves)) as u32,
            lacunarity: float("lacunarity", defaults.lacunarity),
            gain: float("gain", defaults.gain),
            domain_warp_amplitude: float("domain_warp_amplitude", defaults.domain_warp_amplitude),
            domain_warp_frequency: float("domain_warp_frequency", defaults.domain_warp_frequency),
            skirt_depth: float("skirt_depth", defaults.skirt_depth),
            collision_enabled: boolean("collision_enabled", defaults.collision_enabled),
            lod_distances,
            lod_hysteresis: float("lod_hysteresis", defaults.lod_hysteresis),
            runtime,
            last_error,
        }
    }
}
use super::*;
