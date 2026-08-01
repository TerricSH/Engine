//! Versioned contract shared by engine runtime code and generated game scripts.
//!
//! This crate is deliberately data-only. It gives project tooling and script
//! hosts one dependency direction without pulling renderer, ECS, editor, or
//! platform implementation details into game code.

#![forbid(unsafe_code)]

/// Wire/API family understood by the engine-owned gameplay bridge.
pub const GAMEPLAY_SCRIPT_API_SCHEMA: &str = "ScriptAPI-v0";

/// Concrete SDK contract emitted for newly created game projects.
pub const GAMEPLAY_SCRIPT_API_VERSION: &str = "0.16.0";

/// Managed SDK assembly referenced by game-authored C# projects.
pub const MANAGED_SDK_ASSEMBLY_NAME: &str = "EngineGameplay";

/// Engine-owned generated C# source compiled into the managed SDK assembly.
pub const GENERATED_CSHARP_API_FILE: &str = "EngineGameplay.cs";

/// Shared gameplay-rule primitives compiled alongside every managed toolkit.
pub const GENERATED_CSHARP_RULES_FILE: &str = "EngineRules.cs";

/// Tactical-game toolkit compiled alongside the core managed API.
pub const GENERATED_CSHARP_TACTICS_FILE: &str = "EngineTactics.cs";

/// Party-RPG toolkit compiled alongside the core managed API.
pub const GENERATED_CSHARP_JRPG_FILE: &str = "EngineJrpg.cs";

/// Rendering, LOD, and VFX toolkit compiled alongside the core managed API.
pub const GENERATED_CSHARP_RENDERING_FILE: &str = "EngineRendering.cs";

/// Runtime asset generation and terrain editing API, split from the core
/// gameplay source to keep the managed SDK modular.
pub const GENERATED_CSHARP_RUNTIME_ASSETS_FILE: &str = "EngineRuntimeAssets.cs";

/// Sidecar that records ownership, version, and the canonical source hash.
pub const GENERATED_CONTRACT_FILE: &str = "EngineGameplay.contract.json";

/// MSBuild integration imported by game-authored C# projects.
pub const GENERATED_MSBUILD_TARGETS_FILE: &str = "EngineGameplay.targets";

/// Stable description of the gameplay script boundary.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct GameplayScriptApiContract {
    pub schema: &'static str,
    pub version: &'static str,
    pub managed_sdk_assembly: &'static str,
    pub generated_csharp_file: &'static str,
    pub generated_csharp_rules_file: &'static str,
    pub generated_csharp_tactics_file: &'static str,
    pub generated_csharp_jrpg_file: &'static str,
    pub generated_csharp_rendering_file: &'static str,
    pub generated_csharp_runtime_assets_file: &'static str,
    pub generated_contract_file: &'static str,
    pub generated_msbuild_targets_file: &'static str,
}

pub const GAMEPLAY_SCRIPT_API: GameplayScriptApiContract = GameplayScriptApiContract {
    schema: GAMEPLAY_SCRIPT_API_SCHEMA,
    version: GAMEPLAY_SCRIPT_API_VERSION,
    managed_sdk_assembly: MANAGED_SDK_ASSEMBLY_NAME,
    generated_csharp_file: GENERATED_CSHARP_API_FILE,
    generated_csharp_rules_file: GENERATED_CSHARP_RULES_FILE,
    generated_csharp_tactics_file: GENERATED_CSHARP_TACTICS_FILE,
    generated_csharp_jrpg_file: GENERATED_CSHARP_JRPG_FILE,
    generated_csharp_rendering_file: GENERATED_CSHARP_RENDERING_FILE,
    generated_csharp_runtime_assets_file: GENERATED_CSHARP_RUNTIME_ASSETS_FILE,
    generated_contract_file: GENERATED_CONTRACT_FILE,
    generated_msbuild_targets_file: GENERATED_MSBUILD_TARGETS_FILE,
};

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn gameplay_contract_is_explicit_and_versioned() {
        assert_eq!(GAMEPLAY_SCRIPT_API.schema, "ScriptAPI-v0");
        assert_eq!(GAMEPLAY_SCRIPT_API.version, "0.16.0");
        assert_eq!(GAMEPLAY_SCRIPT_API.managed_sdk_assembly, "EngineGameplay");
        assert!(GAMEPLAY_SCRIPT_API.generated_csharp_file.ends_with(".cs"));
        assert!(GAMEPLAY_SCRIPT_API
            .generated_csharp_rules_file
            .ends_with(".cs"));
        assert!(GAMEPLAY_SCRIPT_API
            .generated_csharp_tactics_file
            .ends_with(".cs"));
        assert!(GAMEPLAY_SCRIPT_API
            .generated_csharp_jrpg_file
            .ends_with(".cs"));
        assert!(GAMEPLAY_SCRIPT_API
            .generated_csharp_rendering_file
            .ends_with(".cs"));
        assert!(GAMEPLAY_SCRIPT_API
            .generated_csharp_runtime_assets_file
            .ends_with(".cs"));
        assert!(GAMEPLAY_SCRIPT_API
            .generated_contract_file
            .ends_with(".json"));
        assert!(GAMEPLAY_SCRIPT_API
            .generated_msbuild_targets_file
            .ends_with(".targets"));
    }
}
