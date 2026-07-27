use std::collections::BTreeMap;

use engine_serialize::Value;
use serde::{Deserialize, Serialize};

use crate::Component;

use super::field_as_f32;

/// Runtime role of an entity in an automatic HLOD cluster.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum HlodRole {
    #[default]
    Source,
    Proxy,
}

/// Groups detailed source entities and one or more proxy entities.
///
/// A proxy activates when its bounds center reaches `activation_distance`
/// from the base camera. While any proxy in the cluster is active, all source
/// entities in that cluster are suppressed automatically.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct HlodCluster {
    pub cluster_id: String,
    pub role: HlodRole,
    /// Used by proxy members. Source members ignore this field.
    pub activation_distance: f32,
    /// Zero disables far culling of the proxy.
    pub cull_distance: f32,
}

impl Default for HlodCluster {
    fn default() -> Self {
        Self {
            cluster_id: String::new(),
            role: HlodRole::Source,
            activation_distance: 0.0,
            cull_distance: 0.0,
        }
    }
}

impl Component for HlodCluster {
    const TYPE_ID: &'static str = "engine.hlod_cluster";
}

impl HlodCluster {
    pub fn proxy_is_active(&self, distance: f32) -> bool {
        self.role == HlodRole::Proxy
            && distance >= self.activation_distance
            && (self.cull_distance == 0.0 || distance < self.cull_distance)
    }

    pub fn validate(&self) -> Result<(), String> {
        if self.cluster_id.trim().is_empty() {
            return Err("HLOD cluster id must not be empty".into());
        }
        if !self.activation_distance.is_finite()
            || self.activation_distance < 0.0
            || !self.cull_distance.is_finite()
            || self.cull_distance < 0.0
            || (self.role == HlodRole::Proxy
                && self.cull_distance > 0.0
                && self.cull_distance <= self.activation_distance)
        {
            return Err(
                "HLOD distances must be finite, non-negative, and proxy cull distance must follow activation"
                    .into(),
            );
        }
        Ok(())
    }
}

pub fn serialize_hlod_cluster_fields(cluster: &HlodCluster) -> BTreeMap<String, Value> {
    BTreeMap::from([
        ("cluster_id".into(), Value::Str(cluster.cluster_id.clone())),
        (
            "role".into(),
            Value::Str(
                match cluster.role {
                    HlodRole::Source => "source",
                    HlodRole::Proxy => "proxy",
                }
                .into(),
            ),
        ),
        (
            "activation_distance".into(),
            Value::Float32(cluster.activation_distance),
        ),
        (
            "cull_distance".into(),
            Value::Float32(cluster.cull_distance),
        ),
    ])
}

pub fn deserialize_hlod_cluster_fields(fields: &BTreeMap<String, Value>) -> HlodCluster {
    let cluster_id = match fields.get("cluster_id") {
        Some(Value::Str(value)) => value.clone(),
        _ => String::new(),
    };
    let role = match fields.get("role") {
        Some(Value::Str(value)) if value.eq_ignore_ascii_case("proxy") => HlodRole::Proxy,
        _ => HlodRole::Source,
    };
    HlodCluster {
        cluster_id,
        role,
        activation_distance: fields
            .get("activation_distance")
            .map(field_as_f32)
            .unwrap_or(0.0),
        cull_distance: fields.get("cull_distance").map(field_as_f32).unwrap_or(0.0),
    }
}

pub fn serialize_hlod_cluster(component: &dyn std::any::Any) -> BTreeMap<String, Value> {
    serialize_hlod_cluster_fields(
        component
            .downcast_ref::<HlodCluster>()
            .expect("HlodCluster expected"),
    )
}

pub fn deserialize_hlod_cluster(fields: &BTreeMap<String, Value>) -> Box<dyn std::any::Any> {
    Box::new(deserialize_hlod_cluster_fields(fields))
}

pub fn validate_hlod_cluster_fields(fields: &BTreeMap<String, Value>) -> Result<(), String> {
    deserialize_hlod_cluster_fields(fields).validate()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn proxy_activation_and_field_roundtrip_are_deterministic() {
        let cluster = HlodCluster {
            cluster_id: "capital-city".into(),
            role: HlodRole::Proxy,
            activation_distance: 100.0,
            cull_distance: 1000.0,
        };
        assert!(!cluster.proxy_is_active(99.9));
        assert!(cluster.proxy_is_active(100.0));
        assert!(!cluster.proxy_is_active(1000.0));
        let fields = serialize_hlod_cluster_fields(&cluster);
        assert_eq!(deserialize_hlod_cluster_fields(&fields), cluster);
        assert!(validate_hlod_cluster_fields(&fields).is_ok());
    }
}
