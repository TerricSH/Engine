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
pub const COOKED_MATERIAL_SCHEMA_VERSION: SchemaVersion = SchemaVersion::new(0, 1, 0);

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
    pub base_color_texture: Option<String>,
    pub transparency: String,
    pub double_sided: bool,
}

/// Transparency states represented by the cooked material contract.
///
/// Version 0 currently cooks only opaque materials. Keeping the state typed in
/// the artifact prevents runtime consumers from interpreting arbitrary source
/// strings.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize, Serialize)]
pub enum MaterialTransparency {
    Opaque,
}

/// Validated, authoring-independent material payload used at runtime.
#[derive(Clone, Debug, PartialEq, Deserialize, Serialize)]
pub struct CookedMaterial {
    pub base_color: [f32; 4],
    pub metallic: f32,
    pub roughness: f32,
    pub ambient_occlusion: f32,
    pub base_color_texture: Option<AssetId>,
    pub transparency: MaterialTransparency,
    pub double_sided: bool,
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

        if self.transparency != "Opaque" {
            return Err(CookError::UnsupportedFormat(format!(
                "material transparency '{}' is not supported by MaterialSource-v0; expected 'Opaque'",
                self.transparency
            )));
        }
        if self.double_sided {
            return Err(CookError::UnsupportedFormat(
                "double-sided materials are not supported by MaterialSource-v0".into(),
            ));
        }

        let base_color_texture = self
            .base_color_texture
            .map(|texture_id| -> Result<AssetId, CookError> {
                validate_texture_id(&texture_id)?;
                Ok(AssetId::new(texture_id))
            })
            .transpose()?;

        Ok(CookedMaterial {
            base_color: self.base_color,
            metallic: self.metallic,
            roughness: self.roughness,
            ambient_occlusion: self.ambient_occlusion,
            base_color_texture,
            transparency: MaterialTransparency::Opaque,
            double_sided: false,
        })
    }
}

fn validate_unit_value(field: &str, value: f32) -> Result<(), CookError> {
    if !value.is_finite() || !(0.0..=1.0).contains(&value) {
        return Err(CookError::InvalidAsset(format!(
            "material field '{field}' must be finite and in the range 0..=1; found {value}"
        )));
    }
    Ok(())
}

fn validate_texture_id(texture_id: &str) -> Result<(), CookError> {
    if texture_id.is_empty() || texture_id.len() > 128 {
        return Err(CookError::InvalidAsset(
            "base_color_texture must contain between 1 and 128 ASCII characters".into(),
        ));
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
            "base_color_texture must be a portable asset id using ASCII letters, digits, hyphens, underscores, or dots"
                .into(),
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
    if artifact.header.schema_version != COOKED_MATERIAL_SCHEMA_VERSION {
        let actual = artifact.header.schema_version;
        return Err(CookError::UnsupportedFormat(format!(
            "cooked material schema {}.{}.{} is not supported; expected {}.{}.{}",
            actual.major,
            actual.minor,
            actual.patch,
            COOKED_MATERIAL_SCHEMA_VERSION.major,
            COOKED_MATERIAL_SCHEMA_VERSION.minor,
            COOKED_MATERIAL_SCHEMA_VERSION.patch,
        )));
    }

    bincode::deserialize(&artifact.payload).map_err(|error| {
        CookError::InvalidAsset(format!("invalid cooked material payload: {error}"))
    })
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
            "base_color_texture": "sample-texture",
            "transparency": "Opaque",
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
        assert_eq!(artifact.header.schema_version, SchemaVersion::new(0, 1, 0));
        assert_eq!(material.base_color, [0.8, 0.7, 0.6, 1.0]);
        assert_eq!(
            material.base_color_texture,
            Some(AssetId::new("sample-texture"))
        );
        assert_eq!(material.transparency, MaterialTransparency::Opaque);
        assert!(!material.double_sided);
        let _ = std::fs::remove_dir_all(dir);
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
    fn rejects_non_opaque_and_double_sided_sources() {
        let transparent: MaterialSource =
            serde_json::from_value(source_json(serde_json::json!({"transparency": "Blend"})))
                .unwrap();
        assert!(matches!(
            transparent.into_cooked(),
            Err(CookError::UnsupportedFormat(message)) if message.contains("Opaque")
        ));

        let double_sided: MaterialSource =
            serde_json::from_value(source_json(serde_json::json!({"double_sided": true}))).unwrap();
        assert!(matches!(
            double_sided.into_cooked(),
            Err(CookError::UnsupportedFormat(message)) if message.contains("double-sided")
        ));
    }

    #[test]
    fn decode_rejects_wrong_kind_and_schema() {
        let material = CookedMaterial {
            base_color: [1.0; 4],
            metallic: 0.0,
            roughness: 1.0,
            ambient_occlusion: 1.0,
            base_color_texture: None,
            transparency: MaterialTransparency::Opaque,
            double_sided: false,
        };
        let payload = bincode::serialize(&material).unwrap();

        let dir = case_dir("decode_guards");
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
