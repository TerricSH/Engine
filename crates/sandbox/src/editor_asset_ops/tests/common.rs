    use super::*;

    struct Fixture {
        directory: tempfile::TempDir,
    }

    impl Fixture {
        fn new() -> Self {
            let directory = tempfile::tempdir().expect("temporary project");
            super::super::project_cli::create_project(
                directory.path(),
                Some("Asset Operations"),
                false,
            )
            .expect("create project");
            Self { directory }
        }

        fn root(&self) -> &Path {
            self.directory.path()
        }

        fn manifest_path(&self) -> PathBuf {
            self.root().join("assets/source/game.manifest")
        }

        fn manifest(&self) -> SourceManifest {
            serde_json::from_slice(
                &std::fs::read(self.manifest_path()).expect("read source manifest"),
            )
            .expect("parse source manifest")
        }

        fn create_material(&self, name: &str) -> AssetMutation {
            create_material_asset(
                self.root(),
                Path::new(""),
                name,
                &MaterialTemplate::default(),
            )
            .expect("create material")
        }

        fn declare_asset(
            &self,
            id: &str,
            asset_type: AssetType,
            relative_source: &str,
            bytes: &[u8],
        ) -> SourceAssetEntry {
            let source = self.root().join("assets/source").join(relative_source);
            if let Some(parent) = source.parent() {
                std::fs::create_dir_all(parent).expect("create declared source parent");
            }
            std::fs::write(&source, bytes).expect("write declared source");
            let entry = SourceAssetEntry {
                id: AssetId::new(id),
                asset_type,
                source_path: relative_source.to_string(),
                cook_rules: CookRules::default(),
            };
            let mut manifest = self.manifest();
            manifest.assets.push(entry.clone());
            manifest
                .assets
                .sort_by(|left, right| left.id.id.cmp(&right.id.id));
            std::fs::write(
                self.manifest_path(),
                serde_json::to_vec_pretty(&manifest).expect("serialize source manifest"),
            )
            .expect("write source manifest");
            entry
        }
    }

    fn prefab_source(
        self_id: &str,
        hierarchy_asset: &str,
        default_asset: &str,
        child_asset: &str,
    ) -> String {
        let root_id = "prefab-root".to_string();
        let mut prefab = engine_scene::Prefab::new(AssetId::new(self_id));
        prefab.add_entity(engine_scene::EntityRecord {
            persistent_id: root_id.clone(),
            parent: None,
            name: Some("Prefab Root".into()),
            enabled: true,
            components: BTreeMap::from([(
                "test.component".into(),
                engine_scene::ComponentRecord {
                    schema_version: engine_serialize::SchemaVersion::new(0, 1, 0),
                    enabled: true,
                    fields: BTreeMap::from([(
                        "asset".into(),
                        Value::List(vec![Value::Map(BTreeMap::from([(
                            "nested".into(),
                            Value::Asset(AssetId::new(hierarchy_asset)),
                        )]))]),
                    )]),
                },
            )]),
        });
        prefab.set_default(
            "test.component",
            "default_asset",
            Value::Asset(AssetId::new(default_asset)),
        );
        prefab.child_prefab_refs.push(engine_scene::PrefabChildRef {
            entity_persistent_id: root_id,
            prefab_asset: AssetId::new(child_asset),
        });
        engine_scene::serialize_prefab_source(&prefab).expect("serialize prefab source")
    }

    fn logic_source(
        self_id: &str,
        property_asset: &str,
        default_asset: &str,
        condition_asset: &str,
    ) -> Vec<u8> {
        use engine_serialize::{
            ComparisonOp, LogicAssetKind, LogicMetadata, LogicNode, LogicParam, LogicParamType,
            LogicTransition, LOGIC_ASSET_SCHEMA_V2,
        };

        let logic = LogicAsset {
            schema_version: LOGIC_ASSET_SCHEMA_V2,
            asset_id: self_id.into(),
            kind: LogicAssetKind::BehaviorTree,
            nodes: vec![LogicNode {
                id: "root".into(),
                node_type: "action".into(),
                label: Some("Root".into()),
                transitions: vec![LogicTransition {
                    target_node: "root".into(),
                    condition: Some(LogicCondition::Not(Box::new(LogicCondition::And(vec![
                        LogicCondition::Comparison {
                            param: "asset".into(),
                            op: ComparisonOp::Equal,
                            value: LogicValue::AssetRef(AssetId::new(condition_asset)),
                        },
                    ])))),
                    priority: 0,
                }],
                properties: BTreeMap::from([(
                    "asset".into(),
                    LogicValue::AssetRef(AssetId::new(property_asset)),
                )]),
                children: Vec::new(),
            }],
            entry_node: Some("root".into()),
            parameters: BTreeMap::from([(
                "asset".into(),
                LogicParam {
                    name: "asset".into(),
                    param_type: LogicParamType::AssetRef,
                    default: Some(LogicValue::AssetRef(AssetId::new(default_asset))),
                    description: None,
                },
            )]),
            metadata: LogicMetadata {
                author: None,
                description: None,
                tags: Vec::new(),
                version: "1.0.0".into(),
            },
        };
        serde_json::to_vec_pretty(&logic).expect("serialize logic source")
    }
