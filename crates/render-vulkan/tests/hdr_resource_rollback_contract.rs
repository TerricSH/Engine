const TONE_MAPPING_SOURCE: &str = include_str!("../src/device_impl/hdr/tone_mapping.rs");

#[test]
fn pending_tone_resources_have_a_dependency_ordered_drop_rollback() {
    let drop_start = TONE_MAPPING_SOURCE
        .find("impl Drop for PendingToneMappingResources")
        .expect("pending tone resource guard must implement Drop");
    let drop_end = TONE_MAPPING_SOURCE[drop_start..]
        .find("impl VulkanDevice")
        .map(|offset| drop_start + offset)
        .expect("guard must be declared before VulkanDevice implementation");
    let rollback = &TONE_MAPPING_SOURCE[drop_start..drop_end];

    let pipeline = rollback
        .find("destroy_pipeline(pipeline")
        .expect("pipeline must be rolled back");
    let pipeline_layout = rollback
        .find("destroy_pipeline_layout(layout")
        .expect("pipeline layout must be rolled back");
    let descriptor_pool = rollback
        .find("destroy_descriptor_pool(pool")
        .expect("descriptor pool must be rolled back");
    let descriptor_layout = rollback
        .find("destroy_descriptor_set_layout(layout")
        .expect("descriptor layout must be rolled back");
    let render_pass = rollback
        .find("destroy_render_pass(render_pass")
        .expect("render pass must be rolled back");

    assert!(pipeline < pipeline_layout);
    assert!(pipeline_layout < descriptor_pool);
    assert!(descriptor_pool < descriptor_layout);
    assert!(descriptor_layout < render_pass);
    assert!(
        rollback.matches("destroy_shader_module(module").count() >= 2,
        "both shader modules must participate in rollback"
    );
}

#[test]
fn framebuffer_batch_failure_destroys_already_created_framebuffers() {
    let function_start = TONE_MAPPING_SOURCE
        .find("pub(crate) fn create_tone_framebuffers")
        .expect("tone framebuffer creator must exist");
    let function_end = TONE_MAPPING_SOURCE[function_start..]
        .find("pub(crate) fn update_tone_descriptor_set")
        .map(|offset| function_start + offset)
        .expect("framebuffer creator must precede descriptor update");
    let function = &TONE_MAPPING_SOURCE[function_start..function_end];

    assert!(function.contains("for fb in fbs.drain(..)"));
    assert!(function.contains("d.destroy_framebuffer(fb, None)"));
    assert!(function.contains("return Err(VulkanError::vk(\"cfb_tone\", result))"));
}

#[test]
fn hdr_initializer_rolls_back_the_whole_graph_and_preserves_the_error() {
    let function_start = TONE_MAPPING_SOURCE
        .find("pub(crate) fn ensure_hdr_resources")
        .expect("HDR initializer must exist");
    let function = &TONE_MAPPING_SOURCE[function_start..];

    let failure = function
        .find("if let Err(error) = creation_result")
        .expect("HDR initialization must classify creation failure");
    let rollback = function[failure..]
        .find("self.destroy_hdr_resources();")
        .expect("HDR initialization failure must destroy the partial graph");
    let original_error = function[failure..]
        .find("return Err(error);")
        .expect("HDR initialization must return the original error");

    assert!(rollback < original_error);
}
