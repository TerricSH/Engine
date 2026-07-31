    #[test]
    fn camera_relative_shift_translates_debug_primitive_positions() {
        let offset = glam::Vec3::new(-100.0, 25.0, -40.0);
        let mut primitives = vec![
            DebugPrimitive {
                source_system: "test".into(),
                severity: DiagnosticSeverity::Info,
                primitive_kind: DebugPrimitiveKind::Line {
                    from: [101.0, -25.0, 41.0],
                    to: [100.0, -25.0, 40.0],
                },
                color: [1.0; 4],
                lifetime_frames: 1,
            },
            DebugPrimitive {
                source_system: "test".into(),
                severity: DiagnosticSeverity::Info,
                primitive_kind: DebugPrimitiveKind::Sphere {
                    center: [104.0, -27.0, 43.0],
                    radius: 2.5,
                },
                color: [1.0; 4],
                lifetime_frames: 1,
            },
            DebugPrimitive {
                source_system: "test".into(),
                severity: DiagnosticSeverity::Info,
                primitive_kind: DebugPrimitiveKind::Box {
                    center: [105.0, -28.0, 44.0],
                    half_extents: [1.0, 2.0, 3.0],
                    rotation: [0.0, 0.0, 0.0, 1.0],
                },
                color: [1.0; 4],
                lifetime_frames: 1,
            },
            DebugPrimitive {
                source_system: "test".into(),
                severity: DiagnosticSeverity::Info,
                primitive_kind: DebugPrimitiveKind::Triangle {
                    a: [100.0, -25.0, 40.0],
                    b: [101.0, -25.0, 40.0],
                    c: [100.0, -24.0, 40.0],
                },
                color: [1.0; 4],
                lifetime_frames: 1,
            },
            DebugPrimitive {
                source_system: "test".into(),
                severity: DiagnosticSeverity::Info,
                primitive_kind: DebugPrimitiveKind::Text {
                    position: [102.0, -26.0, 42.0],
                    text: "label".into(),
                    size_px: 12.0,
                },
                color: [1.0; 4],
                lifetime_frames: 1,
            },
        ];

        translate_debug_primitives(&mut primitives, offset);

        let shifted = |p: [f32; 3]| (glam::Vec3::from(p) + offset).to_array();
        match &primitives[0].primitive_kind {
            DebugPrimitiveKind::Line { from, to } => {
                assert_eq!(*from, shifted([101.0, -25.0, 41.0]));
                assert_eq!(*to, shifted([100.0, -25.0, 40.0]));
            }
            other => panic!("unexpected primitive: {other:?}"),
        }
        match &primitives[1].primitive_kind {
            DebugPrimitiveKind::Sphere { center, radius } => {
                assert_eq!(*center, shifted([104.0, -27.0, 43.0]));
                assert_eq!(*radius, 2.5, "radius is translation-invariant");
            }
            other => panic!("unexpected primitive: {other:?}"),
        }
        match &primitives[2].primitive_kind {
            DebugPrimitiveKind::Box {
                center,
                half_extents,
                rotation,
            } => {
                assert_eq!(*center, shifted([105.0, -28.0, 44.0]));
                assert_eq!(*half_extents, [1.0, 2.0, 3.0]);
                assert_eq!(*rotation, [0.0, 0.0, 0.0, 1.0]);
            }
            other => panic!("unexpected primitive: {other:?}"),
        }
        match &primitives[3].primitive_kind {
            DebugPrimitiveKind::Triangle { a, b, c } => {
                assert_eq!(*a, shifted([100.0, -25.0, 40.0]));
                assert_eq!(*b, shifted([101.0, -25.0, 40.0]));
                assert_eq!(*c, shifted([100.0, -24.0, 40.0]));
            }
            other => panic!("unexpected primitive: {other:?}"),
        }
        match &primitives[4].primitive_kind {
            DebugPrimitiveKind::Text {
                position, size_px, ..
            } => {
                assert_eq!(*position, shifted([102.0, -26.0, 42.0]));
                assert_eq!(*size_px, 12.0);
            }
            other => panic!("unexpected primitive: {other:?}"),
        }
    }

    #[test]
    fn camera_relative_rendering_is_disabled_by_default() {
        assert!(!crate::scene::SceneSettings::default().camera_relative_rendering);
        // Existing scenes deserialize with the flag off (serde default).
        let mut world = World::new();
        add_default_camera(&mut world);
        let input = extract_renderer_input_from_world(&world, 0).expect("extracts");
        let view = glam::Mat4::from_cols_array(&input.views[0].view_matrix);
        assert!(
            view.w_axis.truncate().length() <= 1.0e-6,
            "default camera at origin is translation-free in both modes"
        );
    }

    #[test]
    fn origin_shift_is_disabled_by_default_and_serde_compatible() {
        use crate::scene::{OriginShiftSettings, SceneSettings, DEFAULT_ORIGIN_SHIFT_THRESHOLD};

        let defaults = SceneSettings::default();
        assert!(!defaults.origin_shift.enabled);
        assert_eq!(
            defaults.origin_shift.threshold,
            DEFAULT_ORIGIN_SHIFT_THRESHOLD
        );
        assert_eq!(defaults.origin_shift.reference_entity, None);
        assert_eq!(OriginShiftSettings::default(), defaults.origin_shift);

        // Scenes authored before the setting existed deserialize disabled
        // (the whole block is serde-defaulted).
        let legacy: SceneSettings = ron::from_str(
            "(active_camera: None, default_render_layer: \"Default\", \
             fixed_timestep_seconds: 0.016, gravity: None, \
             ambient: (0.0, 0.0, 0.0, 1.0), environment_map: None, \
             tone_mapping: Aces, \
             pass_graph_config: (passes: [], enabled: true, output_mode: HdrThenToneMap), \
             camera_relative_rendering: false)",
        )
        .expect("legacy scene settings deserialize");
        assert_eq!(legacy.origin_shift, OriginShiftSettings::default());

        // A partially authored block fills in the documented defaults.
        let partial: SceneSettings = ron::from_str(
            "(active_camera: None, default_render_layer: \"Default\", \
             fixed_timestep_seconds: 0.016, gravity: None, \
             ambient: (0.0, 0.0, 0.0, 1.0), environment_map: None, \
             tone_mapping: Aces, \
             pass_graph_config: (passes: [], enabled: true, output_mode: HdrThenToneMap), \
             camera_relative_rendering: false, \
             origin_shift: (enabled: true))",
        )
        .expect("partial origin shift settings deserialize");
        assert!(partial.origin_shift.enabled);
        assert_eq!(
            partial.origin_shift.threshold,
            DEFAULT_ORIGIN_SHIFT_THRESHOLD
        );
        assert_eq!(partial.origin_shift.reference_entity, None);
    }
