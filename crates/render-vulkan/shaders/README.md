# Shader Artifacts

Gate 2 needs precompiled SPIR-V for the triangle and textured-object
samples. The source GLSL lives next to this file; compile each stage
with `glslc` from the Vulkan SDK (or `glslangValidator -V`) into
matching `.spv` files in this directory:

```powershell
glslc shaders/triangle.vert -o shaders/triangle.vert.spv
glslc shaders/triangle.frag -o shaders/triangle.frag.spv
glslc shaders/textured.vert -o shaders/textured.vert.spv
glslc shaders/textured.frag -o shaders/textured.frag.spv
```

For local development without the LunarG SDK, the checked-in sample
artifacts can also be regenerated with `naga-cli`:

```powershell
cargo install naga-cli --version 29.0.3 --locked
naga --input-kind glsl --shader-stage vert --entry-point main shaders/triangle.vert shaders/triangle.vert.spv
naga --input-kind glsl --shader-stage frag --entry-point main shaders/triangle.frag shaders/triangle.frag.spv
naga --input-kind glsl --shader-stage vert --entry-point main shaders/textured.vert shaders/textured.vert.spv
naga --input-kind glsl --shader-stage frag --entry-point main shaders/textured.frag shaders/textured.frag.spv
```

`build.rs` picks the artifacts up automatically. When either `.spv` is
missing the renderer compiles but refuses to start the selected scene
with a clear `VulkanError::MissingShader` diagnostic, and `cargo build`
emits a `cargo:warning` pointing at the missing path.
