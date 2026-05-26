# Shader Artifacts

Gate 2 MVP needs precompiled SPIR-V for the triangle sample. The source
GLSL lives next to this file; compile both stages with `glslc` from the
Vulkan SDK (or `glslangValidator -V`) into matching `.spv` files in this
directory:

```powershell
glslc shaders/triangle.vert -o shaders/triangle.vert.spv
glslc shaders/triangle.frag -o shaders/triangle.frag.spv
```

`build.rs` picks the artifacts up automatically. When either `.spv` is
missing the renderer compiles but refuses to start the triangle scene
with a clear `VulkanError::MissingShader` diagnostic, and `cargo build`
emits a `cargo:warning` pointing at the missing path.
