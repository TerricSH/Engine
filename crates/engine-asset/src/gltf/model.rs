use super::*;

/// PBR material properties extracted from a glTF material.
#[derive(Clone, Debug)]
pub struct GltfMaterial {
    /// Original glTF material index.
    pub material_index: usize,
    pub name: String,
    pub base_color: [f32; 4],
    /// Original glTF texture index, or `None`.
    pub base_color_texture: Option<usize>,
    pub metallic: f32,
    pub roughness: f32,
    /// Original glTF texture index, or `None`.
    pub metallic_roughness_texture: Option<usize>,
    pub emissive: [f32; 3],
    /// Original glTF texture index, or `None`.
    pub emissive_texture: Option<usize>,
    /// Original glTF texture index, or `None`.
    pub normal_texture: Option<usize>,
    /// Original glTF texture index, or `None`.
    pub occlusion_texture: Option<usize>,
    pub occlusion_strength: f32,
    pub alpha_mode: gltf::material::AlphaMode,
    pub alpha_cutoff: Option<f32>,
    pub double_sided: bool,
}

/// Pixel storage produced by the importer.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum GltfTextureFormat {
    Rgba8,
}

/// Sampler state attached to one glTF texture object.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct GltfSampler {
    /// Original sampler index. `None` means the glTF default sampler.
    pub sampler_index: Option<usize>,
    pub mag_filter: Option<gltf::texture::MagFilter>,
    pub min_filter: Option<gltf::texture::MinFilter>,
    pub wrap_s: gltf::texture::WrappingMode,
    pub wrap_t: gltf::texture::WrappingMode,
}

/// One decoded glTF texture object.
///
/// A texture is kept separate from its source image so two texture objects
/// sharing one image but using different samplers remain distinct.
#[derive(Clone, Debug)]
pub struct GltfTexture {
    /// Original glTF texture index.
    pub texture_index: usize,
    /// Original glTF image index.
    pub image_index: usize,
    pub sampler: GltfSampler,
    pub format: GltfTextureFormat,
    /// Tightly packed RGBA8 pixels.
    pub data: Vec<u8>,
    pub width: u32,
    pub height: u32,
}

/// One glTF mesh primitive with stable source mappings.
#[derive(Clone, Debug)]
pub struct GltfPrimitive {
    pub name: String,
    pub mesh: MeshData,
    pub material_index: Option<usize>,
    pub topology: gltf::mesh::Mode,
    pub source_mesh_index: usize,
    pub source_primitive_index: usize,
    pub morph_targets: Vec<GltfMorphTarget>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct GltfMorphTarget {
    pub name: String,
    pub position_deltas: Vec<Vec3>,
    pub normal_deltas: Vec<Vec3>,
}

/// A node belonging to the selected glTF scene.
#[derive(Clone, Debug)]
pub struct GltfNode {
    /// Original glTF node index.
    pub source_node_index: usize,
    pub name: String,
    /// World-space transform accumulated from the complete parent chain.
    pub transform: Mat4,
    /// All primitive indices referenced by this node.
    pub primitive_indices: Vec<usize>,
    /// Original glTF skin index for a skinned mesh node.
    pub skin_index: Option<usize>,
    /// Node weights override mesh defaults according to glTF 2.0.
    pub morph_weights: Vec<f32>,
    /// Child indices into the owning scene's `nodes` vector.
    pub children: Vec<usize>,
}

/// One joint extracted from a glTF skin.
///
/// Joints are stored parent-before-child. `source_joint_slot` is the original
/// index used by `JOINTS_0`; the importer remaps vertex indices to this
/// topological order before returning the scene.
#[derive(Clone, Debug, PartialEq)]
pub struct GltfSkinJoint {
    pub source_node_index: usize,
    pub source_joint_slot: usize,
    pub name: String,
    pub parent_index: Option<u32>,
    pub translation: [f32; 3],
    pub rotation: [f32; 4],
    pub scale: [f32; 3],
    pub inverse_bind_matrix: [[f32; 4]; 4],
}

/// A glTF skin ready to convert into the engine animation asset.
#[derive(Clone, Debug, PartialEq)]
pub struct GltfSkin {
    pub source_skin_index: usize,
    pub name: String,
    pub skeleton_root_node: Option<usize>,
    pub joints: Vec<GltfSkinJoint>,
    /// Original glTF joint slot -> topological `joints` index.
    pub joint_remap: Vec<u32>,
}

/// One animation track property.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum GltfAnimationProperty {
    Translation,
    Rotation,
    Scale,
}

/// Values for one animation channel.
#[derive(Clone, Debug, PartialEq)]
pub enum GltfAnimationValues {
    Translations(Vec<[f32; 3]>),
    Rotations(Vec<[f32; 4]>),
    Scales(Vec<[f32; 3]>),
}

/// One animation channel converted to the engine's linear keyframe format.
///
/// STEP channels are expanded with held keys and CUBICSPLINE channels are
/// deterministically resampled during import.
#[derive(Clone, Debug, PartialEq)]
pub struct GltfAnimationChannel {
    pub target_node_index: usize,
    pub property: GltfAnimationProperty,
    pub times: Vec<f32>,
    pub values: GltfAnimationValues,
}

/// One named animation from the document.
#[derive(Clone, Debug, PartialEq)]
pub struct GltfAnimation {
    pub source_animation_index: usize,
    pub name: String,
    pub duration: f32,
    pub channels: Vec<GltfAnimationChannel>,
}

/// The complete contents of a selected glTF scene after import.
#[derive(Clone, Debug)]
pub struct GltfScene {
    /// The selected default scene index, or the first scene when no default is declared.
    pub selected_scene_index: Option<usize>,
    pub primitives: Vec<GltfPrimitive>,
    pub materials: Vec<GltfMaterial>,
    /// One entry per glTF texture, in original document order.
    pub textures: Vec<GltfTexture>,
    /// Only nodes reachable from the selected scene.
    pub nodes: Vec<GltfNode>,
    pub roots: Vec<usize>,
    /// Every skin in original document order.
    pub skins: Vec<GltfSkin>,
    /// Every animation in original document order.
    pub animations: Vec<GltfAnimation>,
}

/// Structured failures from the strict glTF importer.
#[derive(Debug, Error)]
pub enum GltfImportError {
    #[error("failed to open glTF {path}: {detail}", path = .path.display())]
    Open { path: PathBuf, detail: String },

    #[error("failed to load buffers for glTF {path}: {detail}", path = .path.display())]
    BufferLoad { path: PathBuf, detail: String },

    #[error(
        "glTF texture {texture_index} references image {image_index} at {image_source}, but decoding failed: {detail}"
    )]
    TextureDecode {
        texture_index: usize,
        image_index: usize,
        image_source: String,
        detail: String,
    },

    #[error("failed to encode imported glTF texture {texture_index} as project PNG: {detail}")]
    TextureEncode {
        texture_index: usize,
        detail: String,
    },

    #[error(
        "glTF mesh {mesh_index} primitive {primitive_index} uses unsupported topology {topology:?}; only triangle-list is supported"
    )]
    UnsupportedTopology {
        mesh_index: usize,
        primitive_index: usize,
        topology: gltf::mesh::Mode,
    },

    #[error(
        "glTF mesh {mesh_index} primitive {primitive_index} has {target_count} morph targets; at most 8 are supported"
    )]
    TooManyMorphTargets {
        mesh_index: usize,
        primitive_index: usize,
        target_count: usize,
    },

    #[error(
        "glTF mesh {mesh_index} primitive {primitive_index} morph target {target_index} has an attribute count that does not match {vertex_count} vertices"
    )]
    MorphTargetCountMismatch {
        mesh_index: usize,
        primitive_index: usize,
        target_index: usize,
        vertex_count: usize,
    },

    #[error(
        "glTF mesh {mesh_index} primitive {primitive_index} must provide JOINTS_0 and WEIGHTS_0 together"
    )]
    IncompleteSkinAttributes {
        mesh_index: usize,
        primitive_index: usize,
    },

    #[error(
        "glTF mesh {mesh_index} primitive {primitive_index} has {positions} positions, {joints} joint tuples, and {weights} weight tuples"
    )]
    SkinAttributeCountMismatch {
        mesh_index: usize,
        primitive_index: usize,
        positions: usize,
        joints: usize,
        weights: usize,
    },

    #[error(
        "glTF mesh {mesh_index} primitive {primitive_index} vertex {vertex_index} has invalid skin weights"
    )]
    InvalidSkinWeights {
        mesh_index: usize,
        primitive_index: usize,
        vertex_index: usize,
    },

    #[error(
        "glTF skin {skin_index} inverse-bind accessor has {matrices} matrices for {joints} joints"
    )]
    InverseBindCountMismatch {
        skin_index: usize,
        joints: usize,
        matrices: usize,
    },

    #[error("glTF skin {skin_index} joint {joint_slot} has invalid transform data")]
    InvalidJointTransform {
        skin_index: usize,
        joint_slot: usize,
    },

    #[error(
        "glTF mesh {mesh_index} primitive {primitive_index} has skin attributes but is not instantiated by a selected-scene node with a skin"
    )]
    MissingPrimitiveSkin {
        mesh_index: usize,
        primitive_index: usize,
    },

    #[error(
        "glTF mesh {mesh_index} primitive {primitive_index} is instantiated with incompatible skins {skin_indices:?}"
    )]
    AmbiguousPrimitiveSkin {
        mesh_index: usize,
        primitive_index: usize,
        skin_indices: Vec<usize>,
    },

    #[error(
        "glTF mesh {mesh_index} primitive {primitive_index} vertex {vertex_index} references joint slot {joint_slot}, but skin {skin_index} has only {joint_count} joints"
    )]
    JointIndexOutOfRange {
        mesh_index: usize,
        primitive_index: usize,
        vertex_index: usize,
        joint_slot: u32,
        skin_index: usize,
        joint_count: usize,
    },

    #[error(
        "glTF animation {animation_index} channel {channel_index} targets morph weights, which are unsupported"
    )]
    UnsupportedAnimationWeights {
        animation_index: usize,
        channel_index: usize,
    },

    #[error(
        "glTF animation {animation_index} channel {channel_index} has {inputs} input times and {outputs} output values"
    )]
    AnimationKeyCountMismatch {
        animation_index: usize,
        channel_index: usize,
        inputs: usize,
        outputs: usize,
    },

    #[error(
        "glTF animation {animation_index} channel {channel_index} would produce {keys} keys, exceeding the per-channel limit of {max}"
    )]
    AnimationKeyLimitExceeded {
        animation_index: usize,
        channel_index: usize,
        keys: usize,
        max: usize,
    },

    #[error(
        "glTF animation {animation_index} channel {channel_index} contains invalid or unsorted keyframe data"
    )]
    InvalidAnimationChannel {
        animation_index: usize,
        channel_index: usize,
    },

    #[error("glTF mesh {mesh_index} primitive {primitive_index} has no POSITION attribute")]
    MissingPositions {
        mesh_index: usize,
        primitive_index: usize,
    },

    #[error("glTF mesh {mesh_index} primitive {primitive_index} has an empty POSITION attribute")]
    EmptyPositions {
        mesh_index: usize,
        primitive_index: usize,
    },
}
