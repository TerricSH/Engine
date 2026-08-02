//! Project-level C# script build and runtime integration.
//!
//! Source projects are authoring inputs. Runtime players consume the compiled
//! `script_assembly` declared by `game.project.json` and an engine-owned
//! JSON-line protocol host.

#[cfg(feature = "subsystem-scripting-csharp")]
use std::collections::BTreeSet;
use std::ffi::OsString;
use std::path::{Component, Path, PathBuf};
use std::process::Command;

use engine_asset::project::GameProject;
use engine_core::EngineRuntime;
use engine_scene::Scene;
use engine_serialize::{DiagnosticSeverity, Value};
use serde::Serialize;
use sha2::{Digest, Sha256};

const SCRIPT_COMPONENT_TYPE: &str = "engine.script";
#[cfg(feature = "subsystem-scripting-csharp")]
const SCRIPT_HOST_NAME: &str = "project-dotnet";
const SCRIPT_HOST_SOURCE: &str = include_str!("../../../scripts/csharp/EngineSample/Program.cs");
const SCRIPT_HOST_PROJECT: &str = r#"<Project Sdk="Microsoft.NET.Sdk">
  <PropertyGroup>
    <OutputType>Exe</OutputType>
    <TargetFramework>net8.0</TargetFramework>
    <ImplicitUsings>enable</ImplicitUsings>
    <Nullable>enable</Nullable>
    <AssemblyName>EngineScriptHost</AssemblyName>
    <RootNamespace>EngineScriptHost</RootNamespace>
  </PropertyGroup>
</Project>
"#;

const SCRIPT_SDK_PROJECT: &str = r#"<Project Sdk="Microsoft.NET.Sdk">
  <PropertyGroup>
    <TargetFramework>net8.0</TargetFramework>
    <ImplicitUsings>enable</ImplicitUsings>
    <Nullable>enable</Nullable>
    <AssemblyName>EngineGameplay</AssemblyName>
    <RootNamespace>Engine</RootNamespace>
    <Version>0.17.0</Version>
    <AssemblyVersion>0.17.0.0</AssemblyVersion>
    <FileVersion>0.17.0.0</FileVersion>
    <Deterministic>true</Deterministic>
  </PropertyGroup>
</Project>
"#;

const SCRIPT_SDK_TARGETS: &str = r#"<Project>
  <PropertyGroup>
    <EngineGameplaySdkPath Condition="'$(EngineGameplaySdkPath)' == ''">$(MSBuildThisFileDirectory)../../build/script-sdk/EngineGameplay.dll</EngineGameplaySdkPath>
  </PropertyGroup>
  <ItemGroup>
    <Compile Remove="EngineGameplay.cs" />
    <Compile Remove="EngineRules.cs" />
    <Compile Remove="EngineTactics.cs" />
    <Compile Remove="EngineJrpg.cs" />
    <Compile Remove="EngineRendering.cs" />
    <Compile Remove="EngineRuntimeAssets.cs" />
    <Compile Remove="EngineOnlineXr.cs" />
    <Reference Include="EngineGameplay">
      <HintPath>$(EngineGameplaySdkPath)</HintPath>
      <Private>true</Private>
    </Reference>
  </ItemGroup>
  <Target Name="RequireEngineGameplaySdk" BeforeTargets="ResolveReferences" Condition="!Exists('$(EngineGameplaySdkPath)')">
    <Error Text="Engine Gameplay SDK is missing. Run 'sandbox project build-scripts &lt;project&gt;' to build the engine-owned SDK before compiling game scripts directly." />
  </Target>
</Project>
"#;

pub(crate) const STARTER_SCRIPT_PROJECT: &str = r#"<Project Sdk="Microsoft.NET.Sdk">
  <PropertyGroup>
    <TargetFramework>net8.0</TargetFramework>
    <ImplicitUsings>enable</ImplicitUsings>
    <Nullable>enable</Nullable>
    <AssemblyName>GameScripts</AssemblyName>
    <RootNamespace>GameScripts</RootNamespace>
  </PropertyGroup>
  <Import Project="EngineGameplay.targets" />
</Project>
"#;

pub(crate) const STARTER_SCRIPT_API_SOURCE: &str =
    include_str!("../../../scripts/csharp/EngineGameplay/EngineGameplay.cs");

const TACTICS_SCRIPT_API_SOURCE: &str =
    include_str!("../../../scripts/csharp/EngineGameplay/EngineTactics.cs");
const RULES_SCRIPT_API_SOURCE: &str =
    include_str!("../../../scripts/csharp/EngineGameplay/EngineRules.cs");
const JRPG_SCRIPT_API_SOURCE: &str =
    include_str!("../../../scripts/csharp/EngineGameplay/EngineJrpg.cs");
const RENDERING_SCRIPT_API_SOURCE: &str =
    include_str!("../../../scripts/csharp/EngineGameplay/EngineRendering.cs");
const RUNTIME_ASSETS_SCRIPT_API_SOURCE: &str =
    include_str!("../../../scripts/csharp/EngineGameplay/EngineRuntimeAssets.cs");
const ONLINE_XR_SCRIPT_API_SOURCE: &str =
    include_str!("../../../scripts/csharp/EngineGameplay/EngineOnlineXr.cs");

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct ScriptProjectInspection {
    pub assembly_id: Option<String>,
    pub component_count: usize,
}

#[derive(Clone, Debug, Serialize)]
pub(crate) struct ScriptBuildReport {
    pub schema: &'static str,
    pub project: String,
    pub script_api: &'static str,
    pub script_api_version: &'static str,
    pub script_api_sha256: String,
    pub sdk_assembly: String,
    pub assembly_id: String,
    pub assembly: String,
    pub host: String,
    pub dependency_assemblies: usize,
    pub passed: bool,
}

#[derive(Clone, Debug, Serialize)]
pub(crate) struct ScriptApiSyncReport {
    pub schema: &'static str,
    pub project: String,
    pub script_api: &'static str,
    pub version: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source: Option<String>,
    pub contract: String,
    pub msbuild_targets: String,
    pub sdk_assembly: String,
    pub sha256: String,
    pub passed: bool,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct PreparedScriptRuntime {
    pub assemblies: usize,
}

#[derive(Serialize)]
struct GeneratedScriptApiManifest {
    schema: &'static str,
    script_api: &'static str,
    version: &'static str,
    owner: &'static str,
    managed_sdk_assembly: &'static str,
    generated_sources: [&'static str; 7],
    msbuild_targets: &'static str,
    sha256: String,
}

#[cfg(any(feature = "tooling-editor", test))]
mod authoring;
mod compilation;
mod filesystem;
mod inspection;
mod runtime;
mod sdk;

#[cfg(any(feature = "tooling-editor", test))]
pub(crate) use authoring::*;
pub(crate) use compilation::*;
pub(crate) use filesystem::*;
pub(crate) use inspection::*;
pub(crate) use runtime::*;
pub(crate) use sdk::*;

#[cfg(test)]
#[path = "project_scripts/tests.rs"]
mod tests;
