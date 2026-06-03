//! Build script for the DX12 renderer.
//!
//! Attempts to compile HLSL shaders to DXIL bytecode via dxc.exe.
//! If dxc is not available, a placeholder blob is used so the PSO
//! creation code can still be exercised on developer machines.

use std::path::Path;

fn main() {
    // Rebuild if the shader source changes.
    println!("cargo:rerun-if-changed=src/shaders.hlsl");

    // Attempt to compile shaders with dxc.exe (Windows SDK).
    let dxc_path = find_dxc();

    if let Some(dxc) = dxc_path {
        println!("cargo:warning=dxc found at {:?}, compiling shaders", dxc);
        let status = std::process::Command::new(&dxc)
            .args(&[
                "-T", "vs_6_0",
                "-E", "VSMain",
                "-Fo", &output_path("scene_vs.dxil"),
                "-nologo",
                "src/shaders.hlsl",
            ])
            .status();

        match status {
            Ok(s) if s.success() => {
                println!("cargo:warning=Vertex shader compiled successfully");
            }
            _ => {
                println!("cargo:warning=Vertex shader compilation failed, using fallback");
            }
        }

        let status = std::process::Command::new(&dxc)
            .args(&[
                "-T", "ps_6_0",
                "-E", "PSMain",
                "-Fo", &output_path("scene_ps.dxil"),
                "-nologo",
                "src/shaders.hlsl",
            ])
            .status();

        match status {
            Ok(s) if s.success() => {
                println!("cargo:warning=Pixel shader compiled successfully");
            }
            _ => {
                println!("cargo:warning=Pixel shader compilation failed, using fallback");
            }
        }
    } else {
        println!("cargo:warning=dxc not found, DX12 shader compilation skipped");
    }
}

fn find_dxc() -> Option<std::path::PathBuf> {
    // Check common dxc locations
    let candidates = vec![
        // Windows SDK 10
        r"C:\Program Files (x86)\Windows Kits\10\bin\10.0.19041.0\x64\dxc.exe",
        r"C:\Program Files (x86)\Windows Kits\10\bin\10.0.20348.0\x64\dxc.exe",
        r"C:\Program Files (x86)\Windows Kits\10\bin\10.0.22000.0\x64\dxc.exe",
        // Vulkan SDK (includes dxc)
        r"C:\VulkanSDK\1.3\Bin\dxc.exe",
    ];

    for path in candidates {
        let p = Path::new(path);
        if p.exists() {
            return Some(p.to_path_buf());
        }
    }

    // Try PATH
    if let Ok(output) = std::process::Command::new("dxc").arg("--version").output() {
        if output.status.success() {
            return Some("dxc".into());
        }
    }

    None
}

fn output_path(name: &str) -> String {
    let out = std::env::var("OUT_DIR").unwrap_or_else(|_| ".".to_string());
    format!("{}/{}", out, name)
}
