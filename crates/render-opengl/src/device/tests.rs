use super::*;

fn shader_descriptor(source_bytes: Vec<u8>) -> ShaderModuleDescriptor {
    ShaderModuleDescriptor {
        format: ShaderFormat::Glsl,
        stage: ShaderStage::Vertex,
        source_bytes,
        entry_points: vec!["main".to_string()],
        source_hash: [7; 32],
        debug_label: Some("unit-test".to_string()),
    }
}

#[test]
fn glsl_source_decode_preserves_real_source() {
    let descriptor = shader_descriptor(b"#version 450\nvoid main() {}\n".to_vec());
    assert_eq!(
        decode_glsl_source(&descriptor).unwrap(),
        "#version 450\nvoid main() {}\n"
    );
}

#[test]
fn glsl_source_decode_rejects_invalid_input() {
    let mut invalid_utf8 = shader_descriptor(vec![0xff, 0xfe]);
    assert!(matches!(
        decode_glsl_source(&invalid_utf8),
        Err(RhiError::InvalidDescriptor { field, .. }) if field == "shader_module.source_bytes"
    ));

    invalid_utf8.source_bytes = b"void entry() {}".to_vec();
    invalid_utf8.entry_points = vec!["entry".to_string()];
    assert!(matches!(
        decode_glsl_source(&invalid_utf8),
        Err(RhiError::InvalidDescriptor { field, .. }) if field == "shader_module.entry_points"
    ));

    invalid_utf8.entry_points = vec!["main".to_string()];
    invalid_utf8.format = ShaderFormat::SpirV;
    assert!(matches!(
        decode_glsl_source(&invalid_utf8),
        Err(RhiError::InvalidDescriptor { field, .. }) if field == "shader_module.format"
    ));
}

#[test]
fn vertex_attribute_formats_map_to_gl_pointer_kinds() {
    let position = parse_vertex_attribute_format("float32x3").unwrap();
    assert_eq!(position.component_count, 3);
    assert_eq!(position.gl_type, glow::FLOAT);
    assert!(!position.integer);
    assert!(!position.normalized);
    assert_eq!(position.size_bytes, 12);

    let joints = parse_vertex_attribute_format("uint32x4").unwrap();
    assert_eq!(joints.component_count, 4);
    assert_eq!(joints.gl_type, glow::UNSIGNED_INT);
    assert!(joints.integer);
    assert_eq!(joints.size_bytes, 16);

    let color = parse_vertex_attribute_format("rgba8unorm").unwrap();
    assert_eq!(color.gl_type, glow::UNSIGNED_BYTE);
    assert!(color.normalized);
    assert!(!color.integer);
    assert_eq!(color.size_bytes, 4);
    assert!(parse_vertex_attribute_format("mysteryx7").is_none());
}

#[test]
fn vertex_layout_validation_assigns_locations_and_checks_stride() {
    let layout = VertexLayout {
        stride_bytes: 20,
        attributes: vec![
            VertexAttribute {
                semantic: "position".to_string(),
                format: "float32x3".to_string(),
                offset_bytes: 0,
            },
            VertexAttribute {
                semantic: "uv".to_string(),
                format: "float32x2".to_string(),
                offset_bytes: 12,
            },
        ],
    };
    let bindings = parse_vertex_layout(&layout).unwrap();
    assert_eq!(bindings[0].location, 0);
    assert_eq!(bindings[1].location, 1);

    let mut invalid = layout;
    invalid.stride_bytes = 16;
    assert!(matches!(
        parse_vertex_layout(&invalid),
        Err(RhiError::InvalidDescriptor { field, .. })
            if field == "pipeline.vertex_layout.attributes.offset_bytes"
    ));
}

#[test]
fn portable_state_and_binding_names_have_deterministic_mappings() {
    assert_eq!(
        parse_topology(Some("triangle_list")).unwrap(),
        glow::TRIANGLES
    );
    assert_eq!(
        parse_topology(Some("line_strip")).unwrap(),
        glow::LINE_STRIP
    );
    assert!(parse_topology(Some("patches")).is_err());
    assert_eq!(push_uniform_offset("u_pc_64"), Some(64));
    assert_eq!(push_uniform_offset("u_push_constants[0]"), Some(0));
    assert_eq!(push_uniform_offset("u_light_color"), Some(80));
    assert_eq!(gl_binding_point(2, 3), Some(35));
}

#[test]
fn presentation_without_a_platform_swap_callback_fails_closed() {
    assert!(matches!(
        opengl_presentation_unsupported(),
        RhiError::UnsupportedFeature { feature }
            if feature.contains("swap callback")
    ));
}
