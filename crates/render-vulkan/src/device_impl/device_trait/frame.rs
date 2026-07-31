macro_rules! vulkan_device_frame_methods {
    () => {
        fn begin_frame(
            &mut self,
            _: SwapchainHandle,
        ) -> Result<(u32, Box<dyn CmdEncoderTrait>), render_core::RhiError> {
            self.ensure_sc()
                .map_err(|e| render_core::RhiError::Backend {
                    detail: format!("{e}"),
                })?;
            if self.frame_sync.is_empty() {
                self.build_frames()
                    .map_err(|e| render_core::RhiError::Backend {
                        detail: format!("{e}"),
                    })?;
            }
            let fi = self.current_frame;
            let (ii, _) = self
                .acquire(fi)
                .map_err(|e| render_core::RhiError::Backend {
                    detail: format!("{e}"),
                })?;
            self.last_image_index = ii;

            self.begin_cb(fi)
                .map_err(|e| render_core::RhiError::Backend {
                    detail: format!("{e}"),
                })?;
            let f = &self.frame_sync[fi];
            let desc_set = self
                .frame_desc_sets
                .get(fi)
                .copied()
                .unwrap_or(vk::DescriptorSet::null());
            let encoder = Box::new(VkCmdEncoder {
                device: self.logical_device.device.clone(),
                cmd: f.command_buffer,
                // Snapshot slab entries into owned Vec caches — no raw pointers.
                pipeline_cache: self
                    .pipelines
                    .slots
                    .iter()
                    .map(|s| s.as_ref().map(|(g, e)| (*g, e.pipeline)))
                    .collect(),
                buffer_cache: self
                    .buffers
                    .slots
                    .iter()
                    .map(|s| s.as_ref().map(|(g, e)| (*g, e.buffer)))
                    .collect(),
                render_pass_cache: self.render_passes.slots.clone(),
                framebuffer_cache: self
                    .framebuffers
                    .slots
                    .iter()
                    .map(|slot| {
                        slot.as_ref().map(|(generation, entry)| {
                            (
                                *generation,
                                entry.framebuffer,
                                entry.color_attachment_count,
                                entry.has_depth,
                            )
                        })
                    })
                    .collect(),
                pipeline_layout_cache: self
                    .pipeline_layouts
                    .slots
                    .iter()
                    .map(|s| s.as_ref().map(|(g, e)| (*g, e.layout)))
                    .collect(),
                current_desc_set: desc_set,
                render_pass_active: false,
            });

            // Pre-bind the shadow descriptor set at set=1 (if available) so that
            // subsequent encoder operations do not leave it unbound.  The encoder
            // later binds the UBO at set=0 via `bind_descriptor_sets`.
            if let Some(sds) = self.shadow_desc_set {
                if let Some(bind_pll) = self.shadow_bind_layout {
                    let shadow_sets = [sds];
                    // SAFETY: command buffer is in recording state; descriptor set,
                    // pipeline layout, and command buffer are valid Vulkan objects
                    // created by the same device.
                    unsafe {
                        self.logical_device.device.cmd_bind_descriptor_sets(
                            f.command_buffer,
                            vk::PipelineBindPoint::GRAPHICS,
                            bind_pll,
                            1,
                            &shadow_sets,
                            &[],
                        );
                    }
                }
            }

            Ok((ii, encoder))
        }

        fn end_frame(
            &mut self,
            _: SwapchainHandle,
            _: Box<dyn CmdEncoderTrait>,
            ii: u32,
        ) -> Result<RendererStatistics, render_core::RhiError> {
            let fi = self.current_frame;
            let subopt =
                self.submit_and_present(fi, ii)
                    .map_err(|e| render_core::RhiError::Backend {
                        detail: format!("{e}"),
                    })?;
            if subopt {
                // SAFETY: `self.logical_device` is alive by type invariant
                // (ManuallyDrop ensures destruction order).
                unsafe {
                    let _ = self.logical_device.device.device_wait_idle();
                };
                // Keep the swapchain and every framebuffer that references its
                // image views alive until SceneRenderer starts the next frame. It
                // can then destroy its own framebuffers before device-owned HDR/UI
                // resources and the swapchain are torn down in dependency order.
                self.swapchain_recreate_pending = true;
            }
            self.current_frame = (fi + 1) % 2;
            if let Some(instance) = self.instance.as_ref() {
                let validation_errors = instance.validation_error_count();
                if validation_errors > 0 {
                    return Err(render_core::RhiError::Backend {
                        detail: format!("Vulkan validation reported {validation_errors} error(s)"),
                    });
                }
            }
            Ok(RendererStatistics {
                // Pass implementations own draw accounting. The device lifecycle
                // itself records no scene draw and must not fabricate one.
                draw_calls: 0,
                triangles: 0,
                gpu_frame_ms: 0.0,
            })
        }

        fn recreate_swapchain(
            &mut self,
            _: SwapchainHandle,
            w: u32,
            h: u32,
        ) -> Result<(), render_core::RhiError> {
            // SAFETY: `self.logical_device` is alive by type invariant
            // (ManuallyDrop ensures destruction order).
            unsafe {
                let _ = self.logical_device.device.device_wait_idle();
            };
            self.window_width = w.max(1);
            self.window_height = h.max(1);
            self.swapchain = None;
            Ok(())
        }

        fn wait_idle(&self) {
            // SAFETY: `self.logical_device` is alive by type invariant
            // (ManuallyDrop ensures destruction order).
            unsafe {
                let _ = self.logical_device.device.device_wait_idle();
            };
        }

        fn read_pixels(
            &mut self,
            x: u32,
            y: u32,
            width: u32,
            height: u32,
        ) -> Result<Vec<u8>, render_core::RhiError> {
            // Flush all pending GPU work so the swapchain images are in a
            // deterministic layout (PRESENT_SRC_KHR after the last render pass).
            // SAFETY: `self.logical_device` is alive by type invariant (ManuallyDrop
            // ensures destruction order).
            unsafe {
                let _ = self.logical_device.device.device_wait_idle();
            };

            let sc = self
                .swapchain
                .as_ref()
                .ok_or_else(|| render_core::RhiError::Backend {
                    detail: "no swapchain".into(),
                })?;

            // Validate the requested region against the swapchain extent.
            if x + width > sc.extent.width
                || y + height > sc.extent.height
                || width == 0
                || height == 0
            {
                return Err(render_core::RhiError::Backend {
                    detail: format!(
                        "readback region ({x},{y}) {width}×{height} exceeds swapchain {}×{}",
                        sc.extent.width, sc.extent.height
                    ),
                });
            }

            // Pixel buffer: 4 bytes per pixel (RGBA return format).
            let pixel_size: vk::DeviceSize = 4;
            let buffer_size = (width as vk::DeviceSize) * (height as vk::DeviceSize) * pixel_size;

            let d = &self.logical_device;
            let device = &d.device;

            // -----------------------------------------------------------------
            // 1. Create a staging buffer (GPU write → CPU read).
            //    NOTE: The swapchain images MUST have been created with
            //    VK_IMAGE_USAGE_TRANSFER_SRC_BIT for vkCmdCopyImageToBuffer to
            //    work.  Add this to the usage flags in swapchain::new().
            // -----------------------------------------------------------------
            // SAFETY: `device` is a valid AshDevice; buffer creation describes a
            // valid TRANSFER_DST buffer; `None` means no custom allocator.
            let staging_buffer = unsafe {
                device.create_buffer(
                    &vk::BufferCreateInfo::default()
                        .size(buffer_size)
                        .usage(vk::BufferUsageFlags::TRANSFER_DST)
                        .sharing_mode(vk::SharingMode::EXCLUSIVE),
                    None,
                )
            }
            .map_err(|r| render_core::RhiError::Backend {
                detail: format!("create staging buffer: {r:?}"),
            })?;

            // SAFETY: `staging_buffer` was just created by this device; querying
            // memory requirements for a valid buffer is safe.
            let req = unsafe { device.get_buffer_memory_requirements(staging_buffer) };
            let alloc_handle = d.allocator();
            let mut staging_alloc = alloc_handle
                .lock()
                .map_err(|e| render_core::RhiError::Backend {
                    detail: format!("allocator lock: {e}"),
                })?
                .allocate(&AllocationCreateDesc {
                    name: "read_pixels staging",
                    requirements: req,
                    location: MemoryLocation::GpuToCpu,
                })
                .map_err(|e| {
                    // SAFETY: buffer was just created by this device and is not
                    // in use; destroying it on allocation failure is correct.
                    unsafe { device.destroy_buffer(staging_buffer, None) };
                    render_core::RhiError::Backend {
                        detail: format!("alloc staging: {e}"),
                    }
                })?;

            // SAFETY: `staging_buffer` was created by this device; `staging_alloc`
            // was created for this buffer's memory requirements; memory and offset
            // are valid.
            if let Err(r) = unsafe {
                device.bind_buffer_memory(
                    staging_buffer,
                    staging_alloc.memory(),
                    staging_alloc.offset(),
                )
            } {
                // SAFETY: buffer/allocation were just created and are not in use
                // after the failed bind; cleanup is safe.
                if let Ok(mut guard) = alloc_handle.lock() {
                    guard.free(&mut staging_alloc);
                }
                unsafe { device.destroy_buffer(staging_buffer, None) };
                return Err(render_core::RhiError::Backend {
                    detail: format!("bind staging: {r:?}"),
                });
            }

            // -----------------------------------------------------------------
            // 2. One-shot command pool + command buffer.
            // -----------------------------------------------------------------
            // SAFETY: `device` is a valid AshDevice; the queue family index is
            // valid for this device; `None` means no custom allocator.
            let cmd_pool = unsafe {
                device.create_command_pool(
                    &vk::CommandPoolCreateInfo::default()
                        .queue_family_index(d.queue_family_index)
                        .flags(vk::CommandPoolCreateFlags::TRANSIENT),
                    None,
                )
            }
            .map_err(|r| {
                if let Ok(mut guard) = alloc_handle.lock() {
                    guard.free(&mut staging_alloc);
                }
                // SAFETY: cleanup only happen on error; all handles are valid.
                unsafe { device.destroy_buffer(staging_buffer, None) };
                render_core::RhiError::Backend {
                    detail: format!("create pool: {r:?}"),
                }
            })?;

            // SAFETY: `cmd_pool` was just created and is valid; allocation info
            // correctly references the pool with PRIMARY level and 1 buffer.
            let cmd_buffer = unsafe {
                device.allocate_command_buffers(
                    &vk::CommandBufferAllocateInfo::default()
                        .command_pool(cmd_pool)
                        .level(vk::CommandBufferLevel::PRIMARY)
                        .command_buffer_count(1),
                )
            }
            .map_err(|r| {
                // SAFETY: cleanup only on error; all handles created so far are valid.
                unsafe { device.destroy_command_pool(cmd_pool, None) };
                if let Ok(mut guard) = alloc_handle.lock() {
                    guard.free(&mut staging_alloc);
                }
                unsafe { device.destroy_buffer(staging_buffer, None) };
                render_core::RhiError::Backend {
                    detail: format!("alloc cb: {r:?}"),
                }
            })?[0];

            // -----------------------------------------------------------------
            // 3. Record the copy command buffer.
            // -----------------------------------------------------------------
            // SAFETY: command buffer is in the initial state (just allocated from
            // a transient pool); begin transitions it to recording state.
            unsafe {
                device.begin_command_buffer(
                    cmd_buffer,
                    &vk::CommandBufferBeginInfo::default()
                        .flags(vk::CommandBufferUsageFlags::ONE_TIME_SUBMIT),
                )
            }
            .map_err(|r| {
                // SAFETY: cleanup only on error; all handles created so far are valid.
                unsafe { device.destroy_command_pool(cmd_pool, None) };
                if let Ok(mut guard) = alloc_handle.lock() {
                    guard.free(&mut staging_alloc);
                }
                unsafe { device.destroy_buffer(staging_buffer, None) };
                render_core::RhiError::Backend {
                    detail: format!("begin cb: {r:?}"),
                }
            })?;

            // Use the last image acquired by the canonical frame lifecycle.
            let img_idx = self.last_image_index.min(sc.images.len() as u32 - 1);
            let swapchain_image = sc.images[img_idx as usize];

            // 3a. PRESENT_SRC_KHR → TRANSFER_SRC_OPTIMAL
            let to_transfer_barrier = vk::ImageMemoryBarrier::default()
                .old_layout(vk::ImageLayout::PRESENT_SRC_KHR)
                .new_layout(vk::ImageLayout::TRANSFER_SRC_OPTIMAL)
                .src_queue_family_index(vk::QUEUE_FAMILY_IGNORED)
                .dst_queue_family_index(vk::QUEUE_FAMILY_IGNORED)
                .image(swapchain_image)
                .subresource_range(vk::ImageSubresourceRange {
                    aspect_mask: vk::ImageAspectFlags::COLOR,
                    base_mip_level: 0,
                    level_count: 1,
                    base_array_layer: 0,
                    layer_count: 1,
                })
                .src_access_mask(vk::AccessFlags::empty())
                .dst_access_mask(vk::AccessFlags::TRANSFER_READ);
            // SAFETY: command buffer is in recording state; barrier references a
            // live swapchain image; stage and access masks match the layout
            // transition semantics.
            unsafe {
                device.cmd_pipeline_barrier(
                    cmd_buffer,
                    vk::PipelineStageFlags::TOP_OF_PIPE,
                    vk::PipelineStageFlags::TRANSFER,
                    vk::DependencyFlags::empty(),
                    &[],
                    &[],
                    &[to_transfer_barrier],
                );
            }

            // 3b. Copy the requested region from image → staging buffer.
            let copy_region = vk::BufferImageCopy::default()
                .buffer_offset(0)
                .buffer_row_length(0)
                .buffer_image_height(0)
                .image_subresource(vk::ImageSubresourceLayers {
                    aspect_mask: vk::ImageAspectFlags::COLOR,
                    mip_level: 0,
                    base_array_layer: 0,
                    layer_count: 1,
                })
                .image_offset(vk::Offset3D {
                    x: x as i32,
                    y: y as i32,
                    z: 0,
                })
                .image_extent(vk::Extent3D {
                    width,
                    height,
                    depth: 1,
                });
            // SAFETY: both image and buffer are valid Vulkan objects; image is in
            // TRANSFER_SRC_OPTIMAL layout; copy region is within bounds.
            unsafe {
                device.cmd_copy_image_to_buffer(
                    cmd_buffer,
                    swapchain_image,
                    vk::ImageLayout::TRANSFER_SRC_OPTIMAL,
                    staging_buffer,
                    &[copy_region],
                );
            }

            // 3c. TRANSFER_SRC_OPTIMAL → PRESENT_SRC_KHR (restore).
            let to_present_barrier = vk::ImageMemoryBarrier::default()
                .old_layout(vk::ImageLayout::TRANSFER_SRC_OPTIMAL)
                .new_layout(vk::ImageLayout::PRESENT_SRC_KHR)
                .src_queue_family_index(vk::QUEUE_FAMILY_IGNORED)
                .dst_queue_family_index(vk::QUEUE_FAMILY_IGNORED)
                .image(swapchain_image)
                .subresource_range(vk::ImageSubresourceRange {
                    aspect_mask: vk::ImageAspectFlags::COLOR,
                    base_mip_level: 0,
                    level_count: 1,
                    base_array_layer: 0,
                    layer_count: 1,
                })
                .src_access_mask(vk::AccessFlags::TRANSFER_READ)
                .dst_access_mask(vk::AccessFlags::empty());
            // SAFETY: command buffer is still recording; image is live; restoring
            // the original layout matches the swapchain contract.
            unsafe {
                device.cmd_pipeline_barrier(
                    cmd_buffer,
                    vk::PipelineStageFlags::TRANSFER,
                    vk::PipelineStageFlags::BOTTOM_OF_PIPE,
                    vk::DependencyFlags::empty(),
                    &[],
                    &[],
                    &[to_present_barrier],
                );
            }

            // SAFETY: command buffer is in recording state; after this call it
            // transitions to completed state, ready for submission.
            unsafe { device.end_command_buffer(cmd_buffer) }.map_err(|r| {
                // SAFETY: cleanup only on error; all handles created so far are valid.
                unsafe { device.destroy_command_pool(cmd_pool, None) };
                if let Ok(mut guard) = alloc_handle.lock() {
                    guard.free(&mut staging_alloc);
                }
                unsafe { device.destroy_buffer(staging_buffer, None) };
                render_core::RhiError::Backend {
                    detail: format!("end cb: {r:?}"),
                }
            })?;

            // -----------------------------------------------------------------
            // 4. Submit and wait for completion.
            // -----------------------------------------------------------------
            // SAFETY: `device` is a valid AshDevice; fence is created with default
            // (unsignaled) state; `None` means no custom allocator.
            let fence = unsafe { device.create_fence(&vk::FenceCreateInfo::default(), None) }
                .map_err(|r| {
                    // SAFETY: cleanup only on error; all handles are valid.
                    unsafe { device.destroy_command_pool(cmd_pool, None) };
                    if let Ok(mut guard) = alloc_handle.lock() {
                        guard.free(&mut staging_alloc);
                    }
                    unsafe { device.destroy_buffer(staging_buffer, None) };
                    render_core::RhiError::Backend {
                        detail: format!("create fence: {r:?}"),
                    }
                })?;

            let cmd_buffers = [cmd_buffer];
            let submit_info = vk::SubmitInfo::default().command_buffers(&cmd_buffers);
            // SAFETY: `d.queue` is a valid VkQueue; command buffer is in completed
            // state; fence is valid and unsignaled; submit info is correctly
            // structured.
            unsafe { device.queue_submit(d.queue, &[submit_info], fence) }.map_err(|r| {
                // SAFETY: cleanup only on error; all handles are valid.
                unsafe { device.destroy_fence(fence, None) };
                unsafe { device.destroy_command_pool(cmd_pool, None) };
                if let Ok(mut guard) = alloc_handle.lock() {
                    guard.free(&mut staging_alloc);
                }
                unsafe { device.destroy_buffer(staging_buffer, None) };
                render_core::RhiError::Backend {
                    detail: format!("queue submit: {r:?}"),
                }
            })?;

            // SAFETY: fence is valid and associated with the submitted work;
            // waiting with `u64::MAX` timeout and `true` (waitAll) is standard.
            unsafe { device.wait_for_fences(&[fence], true, u64::MAX) }.map_err(|r| {
                // SAFETY: cleanup only on error; all handles are valid.
                unsafe { device.destroy_fence(fence, None) };
                unsafe { device.destroy_command_pool(cmd_pool, None) };
                if let Ok(mut guard) = alloc_handle.lock() {
                    guard.free(&mut staging_alloc);
                }
                unsafe { device.destroy_buffer(staging_buffer, None) };
                render_core::RhiError::Backend {
                    detail: format!("wait fence: {r:?}"),
                }
            })?;

            // SAFETY: fence has been waited on and is no longer needed; destroying
            // a signaled fence is safe.
            unsafe { device.destroy_fence(fence, None) };

            // -----------------------------------------------------------------
            // 5. Map staging buffer and copy pixel data to a Vec<u8>.
            // -----------------------------------------------------------------
            let raw_pixels = match staging_alloc.mapped_slice_mut() {
                Some(slice) => slice[..buffer_size as usize].to_vec(),
                None => {
                    // SAFETY: cleanup only on error; all handles are valid.
                    unsafe { device.destroy_command_pool(cmd_pool, None) };
                    if let Ok(mut guard) = alloc_handle.lock() {
                        guard.free(&mut staging_alloc);
                    }
                    unsafe { device.destroy_buffer(staging_buffer, None) };
                    return Err(render_core::RhiError::Backend {
                        detail: "staging buffer is not CPU mapped".into(),
                    });
                }
            };

            // -----------------------------------------------------------------
            // 6. Convert BGRA → RGBA if the swapchain uses a B8G8R8A8 format.
            //    The custom allocator's GpuToCpu allocations are host-mapped, so the
            //    raw data is available immediately after fence wait.
            // -----------------------------------------------------------------
            let result: Vec<u8> = if sc.format == vk::Format::B8G8R8A8_UNORM
                || sc.format == vk::Format::B8G8R8A8_SRGB
            {
                raw_pixels
                    .chunks_exact(4)
                    .flat_map(|p| [p[2], p[1], p[0], p[3]])
                    .collect()
            } else {
                raw_pixels
            };

            // -----------------------------------------------------------------
            // 7. Clean up temporary resources.
            // -----------------------------------------------------------------
            // SAFETY: all objects were created from this device and are no longer
            // in use after fence wait; reverse order of creation is respected.
            unsafe { device.destroy_command_pool(cmd_pool, None) };
            if let Ok(mut guard) = alloc_handle.lock() {
                guard.free(&mut staging_alloc);
            }
            unsafe { device.destroy_buffer(staging_buffer, None) };

            Ok(result)
        }
    };
}

pub(super) use vulkan_device_frame_methods;
