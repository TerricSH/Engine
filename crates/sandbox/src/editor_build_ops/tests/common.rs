    use super::*;
    use engine_asset::project::ProjectManifest;
    use std::io::Write;

    fn workspace_root() -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .and_then(Path::parent)
            .unwrap()
            .to_path_buf()
    }

    fn project_fixture() -> (tempfile::TempDir, PathBuf) {
        let directory = tempfile::tempdir().unwrap();
        let root = directory.path().join("project");
        std::fs::create_dir_all(root.join("assets/source")).unwrap();
        std::fs::create_dir_all(root.join("assets/scenes")).unwrap();
        std::fs::create_dir_all(root.join("build/cooked")).unwrap();
        std::fs::create_dir_all(root.join("config")).unwrap();
        std::fs::write(
            root.join("assets/scenes/main.scene.ron"),
            "(scene_id:\"main\",name:\"Main\",entities:[],dependencies:[])\n",
        )
        .unwrap();
        std::fs::write(
            root.join("config/input.actions.json"),
            crate::project_input::starter_input_json(),
        )
        .unwrap();
        let manifest = ProjectManifest::new("Build Service Test");
        let manifest_path = manifest.write_to_root(&root).unwrap();
        (directory, manifest_path)
    }

    fn scripted_project_fixture() -> (tempfile::TempDir, PathBuf) {
        let (directory, manifest_path) = project_fixture();
        let root = manifest_path.parent().unwrap();
        let script_project = root.join("scripts/GameScripts/GameScripts.csproj");
        std::fs::create_dir_all(script_project.parent().unwrap()).unwrap();
        std::fs::write(&script_project, "<Project />\n").unwrap();
        let mut manifest = ProjectManifest::new("Scripted Build Service Test");
        manifest.script_project = Some(PathBuf::from("scripts/GameScripts/GameScripts.csproj"));
        manifest.script_assembly = Some(PathBuf::from("build/scripts/GameScripts.dll"));
        let manifest_path = manifest.write_to_root(root).unwrap();
        (directory, manifest_path)
    }

    fn service() -> EditorBuildService {
        EditorBuildService::with_powershell(
            workspace_root(),
            std::env::current_exe().unwrap(),
            PathBuf::from("powershell.exe"),
        )
        .unwrap()
    }

    fn installed_service() -> (tempfile::TempDir, EditorBuildService) {
        use crate::engine_installation::{
            EngineInstallation, EngineInstallationManifest, ENGINE_INSTALLATION_FILE_NAME,
            ENGINE_INSTALLATION_SCHEMA,
        };
        use std::collections::BTreeMap;

        let directory = tempfile::tempdir().unwrap();
        let root = directory.path().join("installed-engine");
        for folder in [
            "bin",
            "runtime/windows-x86_64",
            "tools",
            "sdk",
            "sdk/script-host",
        ] {
            std::fs::create_dir_all(root.join(folder)).unwrap();
        }
        let files = [
            "bin/EngineEditor.exe",
            "runtime/windows-x86_64/GameRuntime.exe",
            "runtime/windows-x86_64/GameRuntime.pdb",
            "tools/asset-cook.exe",
            "tools/package-windows.ps1",
            "sdk/EngineGameplay.dll",
            "sdk/script-host/EngineScriptHost.exe",
            "THIRD_PARTY_NOTICES.txt",
        ];
        for path in files {
            std::fs::write(root.join(path), path.as_bytes()).unwrap();
        }
        let hashes = files
            .into_iter()
            .map(|path| {
                (
                    path.to_string(),
                    format!(
                        "{:x}",
                        Sha256::digest(std::fs::read(root.join(path)).unwrap())
                    ),
                )
            })
            .collect::<BTreeMap<_, _>>();
        let manifest = EngineInstallationManifest {
            schema: ENGINE_INSTALLATION_SCHEMA.into(),
            engine_version: "v-test".into(),
            editor: "bin/EngineEditor.exe".into(),
            windows_runtime: "runtime/windows-x86_64/GameRuntime.exe".into(),
            windows_symbols: "runtime/windows-x86_64/GameRuntime.pdb".into(),
            asset_cooker: "tools/asset-cook.exe".into(),
            package_script: "tools/package-windows.ps1".into(),
            managed_sdk: "sdk/EngineGameplay.dll".into(),
            script_host: "sdk/script-host".into(),
            notices: "THIRD_PARTY_NOTICES.txt".into(),
            script_api: engine_script_api::GAMEPLAY_SCRIPT_API_SCHEMA.into(),
            script_api_version: engine_script_api::GAMEPLAY_SCRIPT_API_VERSION.into(),
            script_api_sha256: "a".repeat(64),
            source_commit: "fixture".into(),
            source_date_epoch: 1_700_000_000,
            rustc: "rustc fixture".into(),
            files: hashes,
        };
        std::fs::write(
            root.join(ENGINE_INSTALLATION_FILE_NAME),
            serde_json::to_vec_pretty(&manifest).unwrap(),
        )
        .unwrap();
        let installation = EngineInstallation::load(root).unwrap();
        let service =
            EditorBuildService::with_installed_powershell(installation, "powershell.exe").unwrap();
        (directory, service)
    }

    fn argument_strings(plan: &ProcessPlan) -> Vec<String> {
        plan.arguments
            .iter()
            .map(|argument| argument.to_string_lossy().into_owned())
            .collect()
    }
