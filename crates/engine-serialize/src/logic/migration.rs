use super::{
    LogicAsset, LOGIC_ASSET_COOKED_V2_MAGIC, LOGIC_ASSET_SCHEMA_V1, LOGIC_ASSET_SCHEMA_V2,
};
use crate::SchemaVersion;
use bincode::Options;
use std::fmt;

mod legacy;

use legacy::{
    migrate_production_v1, migrate_serialize_v1, LegacyProductionLogicAssetV1,
    LegacySerializeLogicAssetV1,
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LogicAssetSourceSchema {
    CanonicalV2,
    ProductionV1,
    SerializeV1,
}

#[derive(Clone, Debug, PartialEq)]
pub struct LogicAssetMigration {
    pub asset: LogicAsset,
    pub source_schema: LogicAssetSourceSchema,
}

/// Parse canonical V2 or explicitly migrate either former V1 JSON schema.
pub fn parse_logic_asset_json_compatible(
    source: &[u8],
) -> Result<LogicAssetMigration, LogicAssetMigrationError> {
    let value: serde_json::Value = serde_json::from_slice(source)
        .map_err(|error| LogicAssetMigrationError::Json(error.to_string()))?;
    let object = value.as_object().ok_or_else(|| {
        LogicAssetMigrationError::Schema("logic asset root must be a JSON object".into())
    })?;
    let version_value = object.get("schema_version").ok_or_else(|| {
        LogicAssetMigrationError::Schema("logic asset is missing schema_version".into())
    })?;
    let version: SchemaVersion =
        serde_json::from_value(version_value.clone()).map_err(|error| {
            LogicAssetMigrationError::Schema(format!("invalid schema_version: {error}"))
        })?;

    if version == LOGIC_ASSET_SCHEMA_V2 {
        let asset: LogicAsset = serde_json::from_value(value)
            .map_err(|error| LogicAssetMigrationError::Json(error.to_string()))?;
        return Ok(LogicAssetMigration {
            asset,
            source_schema: LogicAssetSourceSchema::CanonicalV2,
        });
    }
    if version != LOGIC_ASSET_SCHEMA_V1 {
        return Err(LogicAssetMigrationError::UnsupportedVersion(version));
    }

    let production_marker = object.contains_key("asset_id")
        || object.contains_key("metadata")
        || object
            .get("parameters")
            .is_some_and(serde_json::Value::is_object);
    let serialize_marker = object.contains_key("entry_node")
        || object
            .get("nodes")
            .and_then(serde_json::Value::as_array)
            .is_some_and(|nodes| {
                nodes.iter().any(|node| {
                    node.as_object()
                        .is_some_and(|node| node.contains_key("parameters"))
                })
            });

    match (production_marker, serialize_marker) {
        (true, false) => {
            let legacy: LegacyProductionLogicAssetV1 = serde_json::from_value(value)
                .map_err(|error| LogicAssetMigrationError::Json(error.to_string()))?;
            Ok(LogicAssetMigration {
                asset: migrate_production_v1(legacy),
                source_schema: LogicAssetSourceSchema::ProductionV1,
            })
        }
        (false, true) => {
            let legacy: LegacySerializeLogicAssetV1 = serde_json::from_value(value)
                .map_err(|error| LogicAssetMigrationError::Json(error.to_string()))?;
            Ok(LogicAssetMigration {
                asset: migrate_serialize_v1(legacy)?,
                source_schema: LogicAssetSourceSchema::SerializeV1,
            })
        }
        (true, true) => Err(LogicAssetMigrationError::AmbiguousV1),
        (false, false) => Err(LogicAssetMigrationError::Schema(
            "V1 logic asset has no production or serialize schema discriminator".into(),
        )),
    }
}

/// Encode a validated V2 cooked payload with an unambiguous format prefix.
pub fn encode_logic_asset_cooked_v2(
    asset: &LogicAsset,
) -> Result<Vec<u8>, LogicAssetMigrationError> {
    if asset.schema_version != LOGIC_ASSET_SCHEMA_V2 {
        return Err(LogicAssetMigrationError::UnsupportedVersion(
            asset.schema_version,
        ));
    }
    let mut bytes = LOGIC_ASSET_COOKED_V2_MAGIC.to_vec();
    bytes.extend(
        bincode::serialize(asset)
            .map_err(|error| LogicAssetMigrationError::Bincode(error.to_string()))?,
    );
    Ok(bytes)
}

/// Decode V2 cooked data or explicitly migrate the former production V1
/// bincode layout. Corrupt V2-prefixed data never falls back to legacy decode.
pub fn decode_logic_asset_cooked_compatible(
    bytes: &[u8],
) -> Result<LogicAssetMigration, LogicAssetMigrationError> {
    if let Some(payload) = bytes.strip_prefix(LOGIC_ASSET_COOKED_V2_MAGIC) {
        let asset: LogicAsset = deserialize_bincode_strict(payload)?;
        if asset.schema_version != LOGIC_ASSET_SCHEMA_V2 {
            return Err(LogicAssetMigrationError::UnsupportedVersion(
                asset.schema_version,
            ));
        }
        return Ok(LogicAssetMigration {
            asset,
            source_schema: LogicAssetSourceSchema::CanonicalV2,
        });
    }

    let legacy: LegacyProductionLogicAssetV1 =
        deserialize_bincode_strict(bytes).map_err(|error| {
            LogicAssetMigrationError::Bincode(format!(
                "payload has no V2 magic and is not an exact production V1 payload: {error}"
            ))
        })?;
    if legacy.schema_version != LOGIC_ASSET_SCHEMA_V1 {
        return Err(LogicAssetMigrationError::UnsupportedVersion(
            legacy.schema_version,
        ));
    }
    Ok(LogicAssetMigration {
        asset: migrate_production_v1(legacy),
        source_schema: LogicAssetSourceSchema::ProductionV1,
    })
}

fn deserialize_bincode_strict<T: serde::de::DeserializeOwned>(
    bytes: &[u8],
) -> Result<T, LogicAssetMigrationError> {
    bincode::DefaultOptions::new()
        .with_fixint_encoding()
        .reject_trailing_bytes()
        .deserialize(bytes)
        .map_err(|error| LogicAssetMigrationError::Bincode(error.to_string()))
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum LogicAssetMigrationError {
    Json(String),
    Bincode(String),
    Schema(String),
    UnsupportedVersion(SchemaVersion),
    AmbiguousV1,
    ConflictingLegacyParameter(String),
}

impl fmt::Display for LogicAssetMigrationError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Json(error) => write!(formatter, "logic asset JSON is invalid: {error}"),
            Self::Bincode(error) => write!(formatter, "logic asset bincode is invalid: {error}"),
            Self::Schema(error) => write!(formatter, "logic asset schema is invalid: {error}"),
            Self::UnsupportedVersion(version) => write!(
                formatter,
                "unsupported logic asset schema {}.{}.{}",
                version.major, version.minor, version.patch
            ),
            Self::AmbiguousV1 => write!(
                formatter,
                "ambiguous V1 logic asset contains both production and serialize schema markers"
            ),
            Self::ConflictingLegacyParameter(name) => write!(
                formatter,
                "legacy logic asset defines incompatible values for parameter '{name}'"
            ),
        }
    }
}

impl std::error::Error for LogicAssetMigrationError {}
