//! Build script for the DX12 renderer.
//!
//! Compiles HLSL with the Windows system shader compiler. Shader Model 5.1
//! DXBC remains compatible with the D3D12 runtime shipped by supported
//! Windows 10 installations and needs no external DXIL validator.

#[cfg(windows)]
use std::ffi::CString;
#[cfg(windows)]
use windows::{
    core::PCSTR,
    Win32::Graphics::Direct3D::{Fxc::*, ID3DBlob, ID3DInclude},
};

fn main() {
    println!("cargo:rerun-if-changed=src/shaders.hlsl");

    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() != Ok("windows") {
        return;
    }
    #[cfg(windows)]
    {
        compile("vs_5_1", "VSMain", "scene_vs.dxil");
        compile("vs_5_1", "SkinnedVSMain", "scene_skinned_vs.dxil");
        compile("ps_5_1", "PSMain", "scene_ps.dxil");
        compile("vs_5_1", "ShadowVSMain", "shadow_vs.dxil");
        compile("vs_5_1", "SkinnedShadowVSMain", "shadow_skinned_vs.dxil");
    }
    #[cfg(not(windows))]
    panic!("cross-compiling the DX12 backend requires precompiled Windows shader objects");
}

#[cfg(windows)]
fn compile(profile: &str, entry: &str, output: &str) {
    let source = std::fs::read("src/shaders.hlsl").expect("read DX12 HLSL source");
    let entry = CString::new(entry).expect("shader entry point contains no NUL");
    let profile = CString::new(profile).expect("shader profile contains no NUL");
    let mut bytecode: Option<ID3DBlob> = None;
    let mut errors: Option<ID3DBlob> = None;
    let result = unsafe {
        D3DCompile(
            source.as_ptr().cast(),
            source.len(),
            PCSTR(c"shaders.hlsl".as_ptr().cast()),
            None,
            None::<&ID3DInclude>,
            PCSTR(entry.as_ptr().cast()),
            PCSTR(profile.as_ptr().cast()),
            D3DCOMPILE_ENABLE_STRICTNESS,
            0,
            &mut bytecode,
            Some(&mut errors as *mut _),
        )
    };
    if let Err(error) = result {
        let compiler_message = errors
            .as_ref()
            .map(|blob| unsafe {
                let bytes = std::slice::from_raw_parts(
                    blob.GetBufferPointer().cast::<u8>(),
                    blob.GetBufferSize(),
                );
                String::from_utf8_lossy(bytes)
                    .trim_end_matches('\0')
                    .to_owned()
            })
            .unwrap_or_default();
        panic!(
            "D3DCompile failed for {}: {error}; {compiler_message}",
            entry.to_string_lossy()
        );
    }
    let bytecode = bytecode.expect("D3DCompile succeeded without bytecode");
    let bytes = unsafe {
        std::slice::from_raw_parts(
            bytecode.GetBufferPointer().cast::<u8>(),
            bytecode.GetBufferSize(),
        )
    };
    std::fs::write(output_path(output), bytes).expect("write compiled DX12 shader object");
    println!(
        "cargo:warning={} compiled successfully for {}",
        entry.to_string_lossy(),
        profile.to_string_lossy()
    );
}

fn output_path(name: &str) -> String {
    let out = std::env::var("OUT_DIR").unwrap_or_else(|_| ".".to_string());
    format!("{out}/{name}")
}
