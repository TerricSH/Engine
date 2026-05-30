# Shader Artifacts

Gate 2 needs precompiled SPIR-V for the triangle, textured-object,
forward, shadow, and tonemap shader pairs.  The source GLSL lives next to
this file; compile each stage with `glslc` from the Vulkan SDK (or
`glslangValidator -V`) into matching `.spv` files in this directory:

```powershell
glslc shaders/triangle.vert -o shaders/triangle.vert.spv
glslc shaders/triangle.frag -o shaders/triangle.frag.spv
glslc shaders/textured.vert -o shaders/textured.vert.spv
glslc shaders/textured.frag -o shaders/textured.frag.spv
glslc shaders/forward.vert -o shaders/forward.vert.spv
glslc shaders/forward.frag -o shaders/forward.frag.spv
glslc shaders/shadow.vert -o shaders/shadow.vert.spv
glslc shaders/shadow.frag -o shaders/shadow.frag.spv
glslc shaders/tonemap.vert -o shaders/tonemap.vert.spv
glslc shaders/tonemap.frag -o shaders/tonemap.frag.spv
```

For local development without the LunarG SDK, the checked-in sample
artifacts can also be regenerated with `naga-cli`:

```powershell
cargo install naga-cli --version 29.0.3 --locked
naga --input-kind glsl --shader-stage vert --entry-point main shaders/triangle.vert shaders/triangle.vert.spv
naga --input-kind glsl --shader-stage frag --entry-point main shaders/triangle.frag shaders/triangle.frag.spv
naga --input-kind glsl --shader-stage vert --entry-point main shaders/textured.vert shaders/textured.vert.spv
naga --input-kind glsl --shader-stage frag --entry-point main shaders/textured.frag shaders/textured.frag.spv
naga --input-kind glsl --shader-stage vert --entry-point main shaders/forward.vert shaders/forward.vert.spv
naga --input-kind glsl --shader-stage frag --entry-point main shaders/forward.frag shaders/forward.frag.spv
naga --input-kind glsl --shader-stage vert --entry-point main shaders/shadow.vert shaders/shadow.vert.spv
naga --input-kind glsl --shader-stage frag --entry-point main shaders/shadow.frag shaders/shadow.frag.spv
naga --input-kind glsl --shader-stage vert --entry-point main shaders/tonemap.vert shaders/tonemap.vert.spv
naga --input-kind glsl --shader-stage frag --entry-point main shaders/tonemap.frag shaders/tonemap.frag.spv
```

`build.rs` picks the artifacts up automatically. When any `.spv` is
missing the renderer compiles but refuses to start the selected scene
with a clear `VulkanError::MissingShader` diagnostic, and `cargo build`
emits a `cargo:warning` pointing at the missing path.
