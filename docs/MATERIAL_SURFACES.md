# Material surface states

`MaterialSource-v0` supports portable metallic-roughness and emissive factors,
base-color, normal, metallic-roughness, occlusion, and emissive texture slots,
four surface modes, and single- or double-sided rasterization.

```json
{
  "schema": "MaterialSource-v0",
  "base_color": [1.0, 1.0, 1.0, 0.65],
  "metallic": 0.0,
  "roughness": 0.8,
  "ambient_occlusion": 1.0,
  "emissive": [0.0, 0.15, 0.3],
  "base_color_texture": "foliage-albedo",
  "normal_texture": "foliage-normal",
  "metallic_roughness_texture": "foliage-metallic-roughness",
  "occlusion_texture": "foliage-occlusion",
  "emissive_texture": "foliage-emissive",
  "transparency": "Masked",
  "alpha_cutoff": 0.42,
  "double_sided": true
}
```

`transparency` accepts:

- `Opaque`: depth-writing opaque rendering;
- `Masked`: depth-writing rendering with an alpha cutoff in `0..=1`;
- `Blend`: no depth writes and rendered after opaque surfaces. With
  `SceneSettings.transparency_mode = Sorted` it uses source-alpha blending in
  back-to-front order; with `WeightedBlendedOit` it writes weighted color and
  optical depth and is resolved before tone mapping;
- `Additive`: source-alpha weighted additive blending, no depth writes,
  rendered in the transparent phase for emissive particles and magic effects.

The cooker writes schema `0.6.0`. Runtime decoding remains compatible with
`0.1.0` through `0.5.0` cooked materials. Older payloads default missing
emissive factors to black and all newly introduced texture slots to empty.
Invalid factors, alpha cutoffs, asset IDs, or unresolved texture dependencies
fail before an asset batch is committed.

Vulkan and DirectX 12 both create explicit pipeline variants for alpha,
additive, weighted OIT, and double-sided culling. Their forward shaders multiply base-color
factor alpha by texture alpha, apply alpha test for masked surfaces, flip
back-face normals for double-sided lighting, and preserve blend alpha.
The shaders use glTF channel conventions: metallic-roughness uses green for
roughness and blue for metallic, occlusion uses red, and normal maps contain
tangent-space RGB normals. Tangents are reconstructed from position/UV
derivatives when a normal map is present. The emissive RGB factor and optional
texture are combined and added in linear lighting space. Texture slots and
presence flags use the same ordering in static and skinned Vulkan/DX12 paths.
Blended surfaces do not cast directional shadows. Masked surfaces currently
cast an uncut silhouette in the directional shadow pass; alpha-tested shadow
maps remain follow-up work.

The material editor exposes `AlbedoMap`, `NormalMap`,
`MetallicRoughnessMap`, `OcclusionMap`, and `EmissiveMap`, plus `Alpha Mode`,
`Alpha Cutoff`, and `Double Sided`. Saving preserves these fields instead of
silently dropping authored surface data.

Refraction, water, authored tangent streams, and material instances remain on
the readiness backlog. Clearcoat and the other advanced character-material
parameters are described in [RENDERING_ARCHITECTURE.md](RENDERING_ARCHITECTURE.md).
