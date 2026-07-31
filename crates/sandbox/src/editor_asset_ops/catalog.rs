use super::*;

#[derive(Clone, Debug)]
pub(super) struct ManifestDocument {
    pub(super) path: PathBuf,
    pub(super) manifest: SourceManifest,
}

#[derive(Clone, Debug, Default)]
pub(super) struct ManifestCatalog {
    pub(super) documents: Vec<ManifestDocument>,
}

impl ManifestCatalog {
    pub(super) fn load(source_root: &Path) -> Result<Self, String> {
        if !source_root.is_dir() {
            return Err(format!(
                "asset source root is not a directory: {}",
                source_root.display()
            ));
        }
        let mut paths = Vec::new();
        for entry in std::fs::read_dir(source_root)
            .map_err(|error| format!("could not enumerate {}: {error}", source_root.display()))?
        {
            let path = entry
                .map_err(|error| format!("could not enumerate {}: {error}", source_root.display()))?
                .path();
            if path.is_file()
                && path
                    .extension()
                    .and_then(|extension| extension.to_str())
                    .is_some_and(|extension| extension.eq_ignore_ascii_case("manifest"))
            {
                paths.push(path);
            }
        }
        paths.sort();
        let mut documents = Vec::new();
        let mut ids = BTreeSet::new();
        let mut source_paths = BTreeMap::<String, String>::new();
        for path in paths {
            reject_symlink(&path)?;
            let bytes = std::fs::read(&path).map_err(io_read(&path))?;
            let manifest: SourceManifest = serde_json::from_slice(&bytes)
                .map_err(|error| format!("invalid source manifest {}: {error}", path.display()))?;
            if manifest.schema_version != CURRENT_MANIFEST_VERSION {
                return Err(format!(
                    "unsupported source manifest schema in {}",
                    path.display()
                ));
            }
            for asset in &manifest.assets {
                validate_asset_id(&asset.id)?;
                let id_key = asset.id.id.to_ascii_lowercase();
                if !ids.insert(id_key) {
                    return Err(format!(
                        "asset id '{}' is duplicated or differs only by case",
                        asset.id.id
                    ));
                }
                let relative = normalize_relative_path(
                    Path::new(&asset.source_path),
                    "manifest source path",
                    false,
                )?;
                let source_key = portable_path_key(&relative);
                if let Some(previous) = source_paths.insert(source_key, asset.id.id.clone()) {
                    return Err(format!(
                        "assets '{}' and '{}' share the same portable source path",
                        previous, asset.id.id
                    ));
                }
                let source = source_root.join(&relative);
                if !source.is_file() {
                    return Err(format!("source asset is missing: {}", source.display()));
                }
                reject_symlink(&source)?;
            }
            documents.push(ManifestDocument { path, manifest });
        }
        Ok(Self { documents })
    }

    pub(super) fn locate(&self, asset_id: &AssetId) -> Result<(usize, usize), String> {
        validate_asset_id(asset_id)?;
        let mut match_location = None;
        for (document_index, document) in self.documents.iter().enumerate() {
            for (asset_index, entry) in document.manifest.assets.iter().enumerate() {
                if entry.id.id.eq_ignore_ascii_case(&asset_id.id) {
                    if entry.id.id != asset_id.id {
                        return Err(format!(
                            "asset id '{}' differs in case from declared id '{}'",
                            asset_id.id, entry.id.id
                        ));
                    }
                    match_location = Some((document_index, asset_index));
                }
            }
        }
        match_location.ok_or_else(|| format!("asset '{}' is not declared", asset_id.id))
    }

    pub(super) fn contains_id(&self, id: &str) -> bool {
        self.documents.iter().any(|document| {
            document
                .manifest
                .assets
                .iter()
                .any(|entry| entry.id.id.eq_ignore_ascii_case(id))
        })
    }

    pub(super) fn source_path_is_declared(&self, relative: &Path) -> bool {
        let key = portable_path_key(relative);
        self.documents.iter().any(|document| {
            document.manifest.assets.iter().any(|entry| {
                normalize_relative_path(Path::new(&entry.source_path), "source path", false)
                    .is_ok_and(|path| portable_path_key(&path) == key)
            })
        })
    }

    pub(super) fn ensure_game_manifest(&mut self, source_root: &Path) -> Result<usize, String> {
        let matches = self
            .documents
            .iter()
            .enumerate()
            .filter(|(_, document)| {
                document
                    .path
                    .file_name()
                    .and_then(|name| name.to_str())
                    .is_some_and(|name| name.eq_ignore_ascii_case("game.manifest"))
            })
            .map(|(index, _)| index)
            .collect::<Vec<_>>();
        match matches.as_slice() {
            [index] => Ok(*index),
            [] => {
                let path = source_root.join("game.manifest");
                if path.exists() {
                    return Err(format!(
                        "game.manifest exists but is not a regular source manifest: {}",
                        path.display()
                    ));
                }
                self.documents.push(ManifestDocument {
                    path,
                    manifest: SourceManifest {
                        schema_version: CURRENT_MANIFEST_VERSION,
                        assets: Vec::new(),
                    },
                });
                Ok(self.documents.len() - 1)
            }
            _ => Err("multiple source manifests differ only by the name game.manifest".into()),
        }
    }

    pub(super) fn sort_document(&mut self, index: usize) {
        self.documents[index]
            .manifest
            .assets
            .sort_by(|left, right| {
                left.id
                    .id
                    .to_ascii_lowercase()
                    .cmp(&right.id.id.to_ascii_lowercase())
                    .then_with(|| left.id.id.cmp(&right.id.id))
            });
    }

    pub(super) fn write_document(&self, index: usize) -> Result<(), String> {
        let document = &self.documents[index];
        crate::project_cli::atomic_write_bytes(
            &document.path,
            &serialize_manifest(&document.manifest)?,
        )
    }
}

pub(super) struct StagedWorkspace {
    _directory: tempfile::TempDir,
    pub(super) source_root: PathBuf,
    pub(super) cooked_root: PathBuf,
}

impl StagedWorkspace {
    pub(super) fn prepare(project: &GameProject) -> Result<Self, String> {
        let transaction_root = project.root.join(".engine/asset-transactions");
        ensure_no_symlink_ancestors(&project.root, &transaction_root)?;
        std::fs::create_dir_all(&transaction_root).map_err(|error| {
            format!(
                "could not create asset transaction directory {}: {error}",
                transaction_root.display()
            )
        })?;
        let directory = tempfile::Builder::new()
            .prefix("asset-op-")
            .tempdir_in(&transaction_root)
            .map_err(|error| {
                format!(
                    "could not create asset transaction workspace in {}: {error}",
                    transaction_root.display()
                )
            })?;
        let source_root = directory.path().join("source");
        let cooked_root = directory.path().join("cooked");
        copy_directory_tree(&project.asset_source, &source_root)?;
        std::fs::create_dir(&cooked_root).map_err(|error| {
            format!(
                "could not create staged cooked directory {}: {error}",
                cooked_root.display()
            )
        })?;
        Ok(Self {
            _directory: directory,
            source_root,
            cooked_root,
        })
    }
}
