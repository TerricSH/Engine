//! Portable metallic-roughness material cooking.
//!
//! Authoring data is a small, human-readable JSON contract. Cooking validates
//! every value and replaces the texture's string identifier with the engine's
//! strongly typed [`AssetId`] before serializing the runtime payload.

use std::path::Path;

use engine_serialize::{AssetId, SchemaVersion};
use serde::{Deserialize, Serialize};

use super::error::CookError;
use super::{write_cooked_artifact, AssetType, CookResult, CookedArtifact};

/// Authoring JSON contract accepted by [`cook_material`].
pub const MATERIAL_SOURCE_SCHEMA: &str = "MaterialSource-v0";

/// Payload schema written into every cooked material artifact.
pub const COOKED_MATERIAL_SCHEMA_VERSION: SchemaVersion = SchemaVersion::new(0, 6, 0);
const LEGACY_COOKED_MATERIAL_SCHEMA_VERSION: SchemaVersion = SchemaVersion::new(0, 1, 0);
const LEGACY_SURFACE_MATERIAL_SCHEMA_VERSION: SchemaVersion = SchemaVersion::new(0, 2, 0);
const LEGACY_EMISSIVE_MATERIAL_SCHEMA_VERSION: SchemaVersion = SchemaVersion::new(0, 3, 0);
const LEGACY_TEXTURE_MATERIAL_SCHEMA_VERSION: SchemaVersion = SchemaVersion::new(0, 4, 0);
const LEGACY_ADVANCED_MATERIAL_SCHEMA_VERSION: SchemaVersion = SchemaVersion::new(0, 5, 0);

#[derive(Clone, Copy, Debug, PartialEq, Deserialize, Serialize)]
#[serde(default, deny_unknown_fields)]
pub struct AdvancedMaterialSource {
    pub clearcoat: f32,
    pub clearcoat_roughness: f32,
    pub subsurface: f32,
    pub subsurface_color: [f32; 3],
    pub anisotropy: f32,
    pub sheen_color: [f32; 3],
    pub rim_color: [f32; 3],
    pub rim_power: f32,
}

impl Default for AdvancedMaterialSource {
    fn default() -> Self {
        Self {
            clearcoat: 0.0,
            clearcoat_roughness: 0.2,
            subsurface: 0.0,
            subsurface_color: [1.0, 0.35, 0.25],
            anisotropy: 0.0,
            sheen_color: [0.0; 3],
            rim_color: [0.0; 3],
            rim_power: 3.0,
        }
    }
}

/// Human-readable source representation of a portable material.
///
/// `base_color_texture` deliberately uses a plain asset-id string so material
/// files remain concise. It is converted to [`AssetId`] during cooking.
#[derive(Clone, Debug, PartialEq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct MaterialSource {
    pub schema: String,
    pub base_color: [f32; 4],
    pub metallic: f32,
    pub roughness: f32,
    pub ambient_occlusion: f32,
    #[serde(default)]
    pub emissive: [f32; 3],
    #[serde(default)]
    pub base_color_texture: Option<String>,
    #[serde(default)]
    pub normal_texture: Option<String>,
    #[serde(default)]
    pub metallic_roughness_texture: Option<String>,
    #[serde(default)]
    pub occlusion_texture: Option<String>,
    #[serde(default)]
    pub emissive_texture: Option<String>,
    #[serde(default)]
    pub advanced: AdvancedMaterialSource,
    pub transparency: String,
    #[serde(default = "default_alpha_cutoff")]
    pub alpha_cutoff: f32,
    pub double_sided: bool,
}

/// Transparency states represented by the cooked material contract.
///
/// Keeping the state typed in the artifact prevents runtime consumers from
/// interpreting arbitrary source strings.
#[derive(Clone, Copy, Debug, PartialEq, Deserialize, Serialize)]
pub enum MaterialTransparency {
    Opaque,
    Masked { cutoff: f32 },
    Blend,
    Additive,
}

/// Validated, authoring-independent material payload used at runtime.
#[derive(Clone, Debug, PartialEq, Deserialize, Serialize)]
pub struct CookedMaterial {
    pub base_color: [f32; 4],
    pub metallic: f32,
    pub roughness: f32,
    pub ambient_occlusion: f32,
    pub emissive: [f32; 3],
    pub base_color_texture: Option<AssetId>,
    pub normal_texture: Option<AssetId>,
    pub metallic_roughness_texture: Option<AssetId>,
    pub occlusion_texture: Option<AssetId>,
    pub emissive_texture: Option<AssetId>,
    pub advanced: AdvancedMaterialSource,
    pub transparency: MaterialTransparency,
    pub double_sided: bool,
}

#[derive(Clone, Debug, PartialEq, Deserialize, Serialize)]
struct LegacyCookedMaterial {
    base_color: [f32; 4],
    metallic: f32,
    roughness: f32,
    ambient_occlusion: f32,
    base_color_texture: Option<AssetId>,
    transparency: MaterialTransparency,
    double_sided: bool,
}

#[derive(Clone, Debug, PartialEq, Deserialize, Serialize)]
struct LegacyEmissiveCookedMaterial {
    base_color: [f32; 4],
    metallic: f32,
    roughness: f32,
    ambient_occlusion: f32,
    emissive: [f32; 3],
    base_color_texture: Option<AssetId>,
    transparency: MaterialTransparency,
    double_sided: bool,
}

#[derive(Clone, Debug, PartialEq, Deserialize, Serialize)]
struct LegacyTextureCookedMaterial {
    base_color: [f32; 4],
    metallic: f32,
    roughness: f32,
    ambient_occlusion: f32,
    emissive: [f32; 3],
    base_color_texture: Option<AssetId>,
    normal_texture: Option<AssetId>,
    metallic_roughness_texture: Option<AssetId>,
    occlusion_texture: Option<AssetId>,
    emissive_texture: Option<AssetId>,
    transparency: MaterialTransparency,
    double_sided: bool,
}

/// Parse, validate, and cook a `MaterialSource-v0` JSON file.
pub fn cook_material(source: &Path, output: &Path) -> Result<CookResult, CookError> {
    let source_bytes = std::fs::read(source)?;
    let source: MaterialSource = serde_json::from_slice(&source_bytes).map_err(|error| {
        CookError::Parse(format!(
            "failed to parse material source {}: {error}",
            source.display()
        ))
    })?;
    let cooked = source.into_cooked()?;
    let payload =
        bincode::serialize(&cooked).map_err(|error| CookError::InvalidAsset(error.to_string()))?;

    write_cooked_artifact(
        output,
        AssetType::Material.kind_code(),
        &payload,
        COOKED_MATERIAL_SCHEMA_VERSION,
    )
}

impl MaterialSource {
    fn into_cooked(self) -> Result<CookedMaterial, CookError> {
        if self.schema != MATERIAL_SOURCE_SCHEMA {
            return Err(CookError::UnsupportedFormat(format!(
                "material schema '{}' is not supported; expected '{MATERIAL_SOURCE_SCHEMA}'",
                self.schema
            )));
        }

        for (index, value) in self.base_color.iter().copied().enumerate() {
            validate_unit_value(&format!("base_color[{index}]"), value)?;
        }
        validate_unit_value("metallic", self.metallic)?;
        validate_unit_value("roughness", self.roughness)?;
        validate_unit_value("ambient_occlusion", self.ambient_occlusion)?;
        for (index, value) in self.emissive.iter().copied().enumerate() {
            validate_unit_value(&format!("emissive[{index}]"), value)?;
        }
        for (field, value) in [
            ("advanced.clearcoat", self.advanced.clearcoat),
            (
                "advanced.clearcoat_roughness",
                self.advanced.clearcoat_roughness,
            ),
            ("advanced.subsurface", self.advanced.subsurface),
        ] {
            validate_unit_value(field, value)?;
        }
        for (field, values) in [
            ("advanced.subsurface_color", self.advanced.subsurface_color),
            ("advanced.sheen_color", self.advanced.sheen_color),
            ("advanced.rim_color", self.advanced.rim_color),
        ] {
            for (index, value) in values.into_iter().enumerate() {
                validate_unit_value(&format!("{field}[{index}]"), value)?;
            }
        }
        if !self.advanced.anisotropy.is_finite()
            || !(-1.0..=1.0).contains(&self.advanced.anisotropy)
        {
            return Err(CookError::InvalidAsset(
                "material field 'advanced.anisotropy' must be finite and in the range -1..=1"
                    .into(),
            ));
        }
        if !self.advanced.rim_power.is_finite() || !(0.01..=32.0).contains(&self.advanced.rim_power)
        {
            return Err(CookError::InvalidAsset(
                "material field 'advanced.rim_power' must be finite and in the range 0.01..=32"
                    .into(),
            ));
        }

        validate_unit_value("alpha_cutoff", self.alpha_cutoff)?;
        let transparency = match self.transparency.as_str() {
            "Opaque" => MaterialTransparency::Opaque,
            "Masked" => MaterialTransparency::Masked {
                cutoff: self.alpha_cutoff,
            },
            "Blend" => MaterialTransparency::Blend,
            "Additive" => MaterialTransparency::Additive,
            value => {
                return Err(CookError::UnsupportedFormat(format!(
                    "material transparency '{value}' is not supported; expected 'Opaque', 'Masked', 'Blend', or 'Additive'"
                )));
            }
        };

        let resolve_texture =
            |field: &str, texture_id: Option<String>| -> Result<Option<AssetId>, CookError> {
                texture_id
                    .map(|texture_id| {
                        validate_texture_id(field, &texture_id)?;
                        Ok(AssetId::new(texture_id))
                    })
                    .transpose()
            };
        let base_color_texture = resolve_texture("base_color_texture", self.base_color_texture)?;
        let normal_texture = resolve_texture("normal_texture", self.normal_texture)?;
        let metallic_roughness_texture = resolve_texture(
            "metallic_roughness_texture",
            self.metallic_roughness_texture,
        )?;
        let occlusion_texture = resolve_texture("occlusion_texture", self.occlusion_texture)?;
        let emissive_texture = resolve_texture("emissive_texture", self.emissive_texture)?;

        Ok(CookedMaterial {
            base_color: self.base_color,
            metallic: self.metallic,
            roughness: self.roughness,
            ambient_occlusion: self.ambient_occlusion,
            emissive: self.emissive,
            base_color_texture,
            normal_texture,
            metallic_roughness_texture,
            occlusion_texture,
            emissive_texture,
            advanced: self.advanced,
            transparency,
            double_sided: self.double_sided,
        })
    }
}

const fn default_alpha_cutoff() -> f32 {
    0.5
}

fn validate_unit_value(field: &str, value: f32) -> Result<(), CookError> {
    if !value.is_finite() || !(0.0..=1.0).contains(&value) {
        return Err(CookError::InvalidAsset(format!(
            "material field '{field}' must be finite and in the range 0..=1; found {value}"
        )));
    }
    Ok(())
}

fn validate_texture_id(field: &str, texture_id: &str) -> Result<(), CookError> {
    if texture_id.is_empty() || texture_id.len() > 128 {
        return Err(CookError::InvalidAsset(format!(
            "{field} must contain between 1 and 128 ASCII characters"
        )));
    }
    if !texture_id
        .chars()
        .next()
        .is_some_and(|character| character.is_ascii_alphanumeric())
        || !texture_id.chars().all(|character| {
            character.is_ascii_alphanumeric() || matches!(character, '-' | '_' | '.')
        })
    {
        return Err(CookError::InvalidAsset(
            format!("{field} must be a portable asset id using ASCII letters, digits, hyphens, underscores, or dots"),
        ));
    }
    Ok(())
}

/// Decode a validated cooked artifact as a material.
///
/// Kind and payload schema checks are intentionally explicit: a valid generic
/// cooked header does not imply that a consumer understands its payload.
pub fn decode_cooked_material(artifact: &CookedArtifact) -> Result<CookedMaterial, CookError> {
    if artifact.header.asset_kind != AssetType::Material.kind_code() {
        return Err(CookError::InvalidAsset(format!(
            "expected material kind {}, found {}",
            AssetType::Material.kind_code(),
            artifact.header.asset_kind
        )));
    }
    if artifact.header.schema_version != COOKED_MATERIAL_SCHEMA_VERSION
        && artifact.header.schema_version != LEGACY_SURFACE_MATERIAL_SCHEMA_VERSION
        && artifact.header.schema_version != LEGACY_EMISSIVE_MATERIAL_SCHEMA_VERSION
        && artifact.header.schema_version != LEGACY_TEXTURE_MATERIAL_SCHEMA_VERSION
        && artifact.header.schema_version != LEGACY_ADVANCED_MATERIAL_SCHEMA_VERSION
        && artifact.header.schema_version != LEGACY_COOKED_MATERIAL_SCHEMA_VERSION
    {
        let actual = artifact.header.schema_version;
        return Err(CookError::UnsupportedFormat(format!(
            "cooked material schema {}.{}.{} is not supported; expected {}.{}.{} or legacy 0.1.0/0.2.0/0.3.0/0.4.0/0.5.0",
            actual.major,
            actual.minor,
            actual.patch,
            COOKED_MATERIAL_SCHEMA_VERSION.major,
            COOKED_MATERIAL_SCHEMA_VERSION.minor,
            COOKED_MATERIAL_SCHEMA_VERSION.patch,
        )));
    }

    if artifact.header.schema_version == COOKED_MATERIAL_SCHEMA_VERSION
        || artifact.header.schema_version == LEGACY_ADVANCED_MATERIAL_SCHEMA_VERSION
    {
        bincode::deserialize(&artifact.payload).map_err(|error| {
            CookError::InvalidAsset(format!("invalid cooked material payload: {error}"))
        })
    } else if artifact.header.schema_version == LEGACY_TEXTURE_MATERIAL_SCHEMA_VERSION {
        let legacy: LegacyTextureCookedMaterial =
            bincode::deserialize(&artifact.payload).map_err(|error| {
                CookError::InvalidAsset(format!("invalid legacy texture material payload: {error}"))
            })?;
        Ok(CookedMaterial {
            base_color: legacy.base_color,
            metallic: legacy.metallic,
            roughness: legacy.roughness,
            ambient_occlusion: legacy.ambient_occlusion,
            emissive: legacy.emissive,
            base_color_texture: legacy.base_color_texture,
            normal_texture: legacy.normal_texture,
            metallic_roughness_texture: legacy.metallic_roughness_texture,
            occlusion_texture: legacy.occlusion_texture,
            emissive_texture: legacy.emissive_texture,
            advanced: AdvancedMaterialSource::default(),
            transparency: legacy.transparency,
            double_sided: legacy.double_sided,
        })
    } else if artifact.header.schema_version == LEGACY_EMISSIVE_MATERIAL_SCHEMA_VERSION {
        let legacy: LegacyEmissiveCookedMaterial = bincode::deserialize(&artifact.payload)
            .map_err(|error| {
                CookError::InvalidAsset(format!("invalid legacy cooked material payload: {error}"))
            })?;
        Ok(CookedMaterial {
            base_color: legacy.base_color,
            metallic: legacy.metallic,
            roughness: legacy.roughness,
            ambient_occlusion: legacy.ambient_occlusion,
            emissive: legacy.emissive,
            base_color_texture: legacy.base_color_texture,
            normal_texture: None,
            metallic_roughness_texture: None,
            occlusion_texture: None,
            emissive_texture: None,
            advanced: AdvancedMaterialSource::default(),
            transparency: legacy.transparency,
            double_sided: legacy.double_sided,
        })
    } else {
        let legacy: LegacyCookedMaterial =
            bincode::deserialize(&artifact.payload).map_err(|error| {
                CookError::InvalidAsset(format!("invalid legacy cooked material payload: {error}"))
            })?;
        Ok(CookedMaterial {
            base_color: legacy.base_color,
            metallic: legacy.metallic,
            roughness: legacy.roughness,
            ambient_occlusion: legacy.ambient_occlusion,
            emissive: [0.0; 3],
            base_color_texture: legacy.base_color_texture,
            normal_texture: None,
            metallic_roughness_texture: None,
            occlusion_texture: None,
            emissive_texture: None,
            advanced: AdvancedMaterialSource::default(),
            transparency: legacy.transparency,
            double_sided: legacy.double_sided,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cook::{read_cooked_artifact, write_cooked_artifact};

    fn source_json(overrides: serde_json::Value) -> serde_json::Value {
        let mut source = serde_json::json!({
            "schema": MATERIAL_SOURCE_SCHEMA,
            "base_color": [0.8, 0.7, 0.6, 1.0],
            "metallic": 0.25,
            "roughness": 0.5,
            "ambient_occlusion": 1.0,
            "emissive": [0.1, 0.2, 0.3],
            "base_color_texture": "sample-texture",
            "transparency": "Opaque",
            "alpha_cutoff": 0.5,
            "double_sided": false
        });
        for (key, value) in overrides.as_object().unwrap() {
            source[key] = value.clone();
        }
        source
    }

    fn case_dir(name: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "engine_asset_material_{name}_{}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn cooks_and_decodes_material_source() {
        let dir = case_dir("roundtrip");
        let source = dir.join("material.json");
        let output = dir.join("material.cooked");
        std::fs::write(
            &source,
            serde_json::to_vec(&source_json(serde_json::json!({}))).unwrap(),
        )
        .unwrap();

        cook_material(&source, &output).unwrap();
        let artifact = read_cooked_artifact(&output).unwrap();
        let material = decode_cooked_material(&artifact).unwrap();

        assert_eq!(artifact.header.asset_kind, 5);
        assert_eq!(artifact.header.schema_version, SchemaVersion::new(0, 6, 0));
        assert_eq!(material.base_color, [0.8, 0.7, 0.6, 1.0]);
        assert_eq!(
            material.base_color_texture,
            Some(AssetId::new("sample-texture"))
        );
        assert_eq!(material.emissive, [0.1, 0.2, 0.3]);
        assert_eq!(material.transparency, MaterialTransparency::Opaque);
        assert!(!material.double_sided);
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn cooks_all_portable_pbr_texture_references() {
        let source: MaterialSource = serde_json::from_value(source_json(serde_json::json!({
            "normal_texture": "sample-normal",
            "metallic_roughness_texture": "sample-metallic-roughness",
            "occlusion_texture": "sample-occlusion",
            "emissive_texture": "sample-emissive"
        })))
        .unwrap();
        let cooked = source.into_cooked().unwrap();
        assert_eq!(cooked.normal_texture, Some(AssetId::new("sample-normal")));
        assert_eq!(
            cooked.metallic_roughness_texture,
            Some(AssetId::new("sample-metallic-roughness"))
        );
        assert_eq!(
            cooked.occlusion_texture,
            Some(AssetId::new("sample-occlusion"))
        );
        assert_eq!(
            cooked.emissive_texture,
            Some(AssetId::new("sample-emissive"))
        );
    }

    #[test]
    fn optional_texture_may_be_omitted() {
        let source: MaterialSource = serde_json::from_value(serde_json::json!({
            "schema": MATERIAL_SOURCE_SCHEMA,
            "base_color": [1.0, 1.0, 1.0, 1.0],
            "metallic": 0.0,
            "roughness": 1.0,
            "ambient_occlusion": 1.0,
            "transparency": "Opaque",
            "double_sided": false
        }))
        .unwrap();
        assert!(source.into_cooked().unwrap().base_color_texture.is_none());
    }

    #[test]
    fn rejects_wrong_source_schema() {
        let source: MaterialSource = serde_json::from_value(source_json(
            serde_json::json!({"schema": "MaterialSource-v1"}),
        ))
        .unwrap();
        assert!(matches!(
            source.into_cooked(),
            Err(CookError::UnsupportedFormat(message)) if message.contains("schema")
        ));
    }

    #[test]
    fn rejects_non_finite_and_out_of_range_values() {
        let mut source: MaterialSource =
            serde_json::from_value(source_json(serde_json::json!({"metallic": 1.1}))).unwrap();
        assert!(matches!(
            source.clone().into_cooked(),
            Err(CookError::InvalidAsset(_))
        ));

        source.metallic = f32::NAN;
        assert!(matches!(
            source.into_cooked(),
            Err(CookError::InvalidAsset(_))
        ));
    }

    #[test]
    fn cooks_masked_blended_additive_and_double_sided_sources() {
        let masked: MaterialSource = serde_json::from_value(source_json(
            serde_json::json!({"transparency": "Masked", "alpha_cutoff": 0.37}),
        ))
        .unwrap();
        assert_eq!(
            masked.into_cooked().unwrap().transparency,
            MaterialTransparency::Masked { cutoff: 0.37 }
        );

        let blended: MaterialSource = serde_json::from_value(source_json(
            serde_json::json!({"transparency": "Blend", "double_sided": true}),
        ))
        .unwrap();
        let blended = blended.into_cooked().unwrap();
        assert_eq!(blended.transparency, MaterialTransparency::Blend);
        assert!(blended.double_sided);

        let additive: MaterialSource =
            serde_json::from_value(source_json(serde_json::json!({"transparency": "Additive"})))
                .unwrap();
        assert_eq!(
            additive.into_cooked().unwrap().transparency,
            MaterialTransparency::Additive
        );
    }

    #[test]
    fn rejects_unknown_transparency_and_invalid_cutoff() {
        let unknown: MaterialSource =
            serde_json::from_value(source_json(serde_json::json!({"transparency": "Refract"})))
                .unwrap();
        assert!(matches!(
            unknown.into_cooked(),
            Err(CookError::UnsupportedFormat(message)) if message.contains("Masked")
        ));

        let cutoff: MaterialSource = serde_json::from_value(source_json(
            serde_json::json!({"transparency": "Masked", "alpha_cutoff": 1.1}),
        ))
        .unwrap();
        assert!(matches!(
            cutoff.into_cooked(),
            Err(CookError::InvalidAsset(message)) if message.contains("alpha_cutoff")
        ));
    }

    #[test]
    fn decode_rejects_wrong_kind_and_schema() {
        let legacy_material = LegacyCookedMaterial {
            base_color: [1.0; 4],
            metallic: 0.0,
            roughness: 1.0,
            ambient_occlusion: 1.0,
            base_color_texture: None,
            transparency: MaterialTransparency::Opaque,
            double_sided: false,
        };
        let legacy_payload = bincode::serialize(&legacy_material).unwrap();

        let dir = case_dir("decode_guards");
        let legacy = dir.join("legacy.cooked");
        write_cooked_artifact(
            &legacy,
            AssetType::Material.kind_code(),
            &legacy_payload,
            LEGACY_COOKED_MATERIAL_SCHEMA_VERSION,
        )
        .unwrap();
        let decoded = decode_cooked_material(&read_cooked_artifact(&legacy).unwrap()).unwrap();
        assert_eq!(decoded.base_color, legacy_material.base_color);
        assert_eq!(decoded.emissive, [0.0; 3]);

        let emissive_legacy_material = LegacyEmissiveCookedMaterial {
            base_color: [0.5; 4],
            metallic: 0.2,
            roughness: 0.6,
            ambient_occlusion: 0.8,
            emissive: [0.1, 0.2, 0.3],
            base_color_texture: Some(AssetId::new("legacy-base")),
            transparency: MaterialTransparency::Blend,
            double_sided: true,
        };
        let emissive_legacy = dir.join("legacy-emissive.cooked");
        write_cooked_artifact(
            &emissive_legacy,
            AssetType::Material.kind_code(),
            &bincode::serialize(&emissive_legacy_material).unwrap(),
            LEGACY_EMISSIVE_MATERIAL_SCHEMA_VERSION,
        )
        .unwrap();
        let decoded =
            decode_cooked_material(&read_cooked_artifact(&emissive_legacy).unwrap()).unwrap();
        assert_eq!(decoded.emissive, [0.1, 0.2, 0.3]);
        assert!(decoded.normal_texture.is_none());
        assert!(decoded.emissive_texture.is_none());

        let material = CookedMaterial {
            base_color: [1.0; 4],
            metallic: 0.0,
            roughness: 1.0,
            ambient_occlusion: 1.0,
            emissive: [0.25, 0.5, 0.75],
            base_color_texture: None,
            normal_texture: None,
            metallic_roughness_texture: None,
            occlusion_texture: None,
            emissive_texture: None,
            advanced: AdvancedMaterialSource::default(),
            transparency: MaterialTransparency::Opaque,
            double_sided: false,
        };
        let payload = bincode::serialize(&material).unwrap();
        let advanced_legacy = dir.join("legacy-advanced.cooked");
        write_cooked_artifact(
            &advanced_legacy,
            AssetType::Material.kind_code(),
            &payload,
            LEGACY_ADVANCED_MATERIAL_SCHEMA_VERSION,
        )
        .unwrap();
        assert_eq!(
            decode_cooked_material(&read_cooked_artifact(&advanced_legacy).unwrap())
                .unwrap()
                .advanced,
            AdvancedMaterialSource::default()
        );

        let wrong_kind = dir.join("wrong-kind.cooked");
        write_cooked_artifact(
            &wrong_kind,
            AssetType::Texture.kind_code(),
            &payload,
            COOKED_MATERIAL_SCHEMA_VERSION,
        )
        .unwrap();
        let error =
            decode_cooked_material(&read_cooked_artifact(&wrong_kind).unwrap()).unwrap_err();
        assert!(error.to_string().contains("expected material kind"));

        let wrong_schema = dir.join("wrong-schema.cooked");
        write_cooked_artifact(
            &wrong_schema,
            AssetType::Material.kind_code(),
            &payload,
            SchemaVersion::new(9, 0, 0),
        )
        .unwrap();
        let error =
            decode_cooked_material(&read_cooked_artifact(&wrong_schema).unwrap()).unwrap_err();
        assert!(error.to_string().contains("schema 9.0.0"));
        let _ = std::fs::remove_dir_all(dir);
    }
}
