    use ash::vk::Handle;

    use super::*;

    fn layout(raw: u64) -> vk::DescriptorSetLayout {
        vk::DescriptorSetLayout::from_raw(raw)
    }

    #[test]
    fn fallback_pipeline_layouts_require_a_contiguous_prefix() {
        let err = fallback_pipeline_set_layouts(None, None, Some(layout(3)))
            .expect_err("set 2 without sets 0 and 1 must be rejected");
        assert!(matches!(
            err,
            render_core::RhiError::InvalidDescriptor { .. }
        ));
    }

    #[test]
    fn fallback_pipeline_layouts_preserve_initialized_set_order() {
        let layouts =
            fallback_pipeline_set_layouts(Some(layout(1)), Some(layout(2)), Some(layout(3)))
                .expect("contiguous layouts should be accepted");
        assert_eq!(layouts, vec![layout(1), layout(2), layout(3)]);
        assert!(layouts
            .iter()
            .all(|layout| *layout != vk::DescriptorSetLayout::null()));
    }

    #[test]
    fn explicit_pipeline_layouts_reject_set_index_gaps() {
        let layouts = [BindGroupLayoutDescriptor {
            set_index: 1,
            bindings: Vec::new(),
        }];
        let err = validate_contiguous_bind_group_layouts(&layouts)
            .expect_err("set 1 without set 0 must be rejected");
        assert!(matches!(
            err,
            render_core::RhiError::InvalidDescriptor { .. }
        ));
    }

    #[test]
    fn explicit_pipeline_layouts_are_ordered_by_set_index() {
        let layouts = [
            BindGroupLayoutDescriptor {
                set_index: 1,
                bindings: Vec::new(),
            },
            BindGroupLayoutDescriptor {
                set_index: 0,
                bindings: Vec::new(),
            },
        ];
        let ordered = ordered_bind_group_layouts(&layouts)
            .expect("contiguous layouts should be sorted by set index");
        assert_eq!(ordered[0].set_index, 0);
        assert_eq!(ordered[1].set_index, 1);
    }

    #[test]
    fn descriptor_bindings_reject_duplicates_and_unknown_resource_kinds() {
        let duplicate = BindGroupLayoutDescriptor {
            set_index: 0,
            bindings: vec![
                render_core::BindGroupLayoutBinding {
                    binding: 1,
                    resource_kind: "uniform_buffer".into(),
                },
                render_core::BindGroupLayoutBinding {
                    binding: 1,
                    resource_kind: "sampler".into(),
                },
            ],
        };
        assert!(vulkan_descriptor_bindings(&duplicate).is_err());

        let unknown = BindGroupLayoutDescriptor {
            set_index: 0,
            bindings: vec![render_core::BindGroupLayoutBinding {
                binding: 0,
                resource_kind: "mystery_resource".into(),
            }],
        };
        assert!(vulkan_descriptor_bindings(&unknown).is_err());
        assert_eq!(
            resource_kind_to_descriptor_type("sampler").unwrap(),
            vk::DescriptorType::SAMPLER
        );
    }

    #[test]
    fn graphics_pipeline_validation_rejects_silent_fallback_inputs() {
        let descriptor = PipelineDescriptor {
            topology: Some("unknown".into()),
            ..PipelineDescriptor::default()
        };
        assert!(validate_graphics_pipeline_descriptor(&descriptor).is_err());

        let descriptor = PipelineDescriptor {
            vertex_layout: render_core::VertexLayout {
                stride_bytes: 8,
                attributes: vec![render_core::VertexAttribute {
                    semantic: "position".into(),
                    format: "float32x3".into(),
                    offset_bytes: 0,
                }],
            },
            ..PipelineDescriptor::default()
        };
        assert!(validate_graphics_pipeline_descriptor(&descriptor).is_err());
    }

    #[test]
    fn specialization_data_uses_four_byte_vulkan_scalars() {
        let constants = [
            render_core::SpecConstant {
                id: 4,
                value: render_core::SpecValue::Bool(true),
            },
            render_core::SpecConstant {
                id: 9,
                value: render_core::SpecValue::F32(2.5),
            },
        ];
        let (data, entries) = vulkan_specialization_data(&constants);
        assert_eq!(data.len(), 8);
        assert_eq!(&data[..4], &1u32.to_ne_bytes());
        assert_eq!(&data[4..], &2.5f32.to_ne_bytes());
        assert_eq!(entries[0].constant_id, 4);
        assert_eq!(entries[0].size, 4);
        assert_eq!(entries[1].offset, 4);
    }

    #[test]
    fn bgra_present_targets_use_the_actual_swapchain_format() {
        assert_eq!(
            color_attachment_format(
                Some(&TextureFormat::Bgra8Unorm),
                Some(vk::Format::B8G8R8A8_SRGB),
            ),
            vk::Format::B8G8R8A8_SRGB
        );
    }

    #[test]
    fn buffer_write_range_accepts_exact_end_and_empty_end_write() {
        assert_eq!(checked_buffer_write_range(16, 4, 12).unwrap(), 4..16);
        assert_eq!(checked_buffer_write_range(16, 16, 0).unwrap(), 16..16);
    }

    #[test]
    fn buffer_write_range_rejects_out_of_bounds_without_truncation() {
        let error = checked_buffer_write_range(16, 15, 2).unwrap_err();
        assert!(matches!(
            error,
            render_core::RhiError::InvalidDescriptor { .. }
        ));
        assert!(checked_buffer_write_range(16, 17, 0).is_err());
        assert!(checked_buffer_write_range(u64::MAX, u64::MAX, 1).is_err());
    }
