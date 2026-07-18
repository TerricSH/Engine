# Shader Artifacts

The canonical SceneRenderer needs precompiled SPIR-V for the forward,
skybox, shadow, tone-map, and UI shader pairs. The source GLSL lives next to
this file; compile each stage with `glslc` from the Vulkan SDK (or
`glslangValidator -V`) into matching `.spv` files in this directory:

```powershell
glslc shaders/forward.vert -o shaders/forward.vert.spv
glslc shaders/forward.frag -o shaders/forward.frag.spv
glslc shaders/skybox.vert -o shaders/skybox.vert.spv
glslc shaders/skybox.frag -o shaders/skybox.frag.spv
glslc shaders/shadow.vert -o shaders/shadow.vert.spv
glslc shaders/shadow.frag -o shaders/shadow.frag.spv
glslc shaders/tonemap.vert -o shaders/tonemap.vert.spv
glslc shaders/tonemap.frag -o shaders/tonemap.frag.spv
glslc shaders/ui_overlay.vert -o shaders/ui_overlay.vert.spv
glslc shaders/ui_overlay.frag -o shaders/ui_overlay.frag.spv
```

For local development without the LunarG SDK, the checked-in sample
artifacts can also be regenerated with `naga-cli`:

```powershell
cargo install naga-cli --version 29.0.3 --locked
naga --input-kind glsl --shader-stage vert --entry-point main shaders/forward.vert shaders/forward.vert.spv
naga --input-kind glsl --shader-stage frag --entry-point main shaders/forward.frag shaders/forward.frag.spv
naga --input-kind glsl --shader-stage vert --entry-point main shaders/skybox.vert shaders/skybox.vert.spv
naga --input-kind glsl --shader-stage frag --entry-point main shaders/skybox.frag shaders/skybox.frag.spv
naga --input-kind glsl --shader-stage vert --entry-point main shaders/shadow.vert shaders/shadow.vert.spv
naga --input-kind glsl --shader-stage frag --entry-point main shaders/shadow.frag shaders/shadow.frag.spv
naga --input-kind glsl --shader-stage vert --entry-point main shaders/tonemap.vert shaders/tonemap.vert.spv
naga --input-kind glsl --shader-stage frag --entry-point main shaders/tonemap.frag shaders/tonemap.frag.spv
naga --input-kind glsl --shader-stage vert --entry-point main shaders/ui_overlay.vert shaders/ui_overlay.vert.spv
naga --input-kind glsl --shader-stage frag --entry-point main shaders/ui_overlay.frag shaders/ui_overlay.frag.spv
```

`build.rs` picks the artifacts up automatically. When any `.spv` is
missing the renderer compiles but refuses to start the selected scene
with a clear `VulkanError::MissingShader` diagnostic, and `cargo build`
emits a `cargo:warning` pointing at the missing path.
