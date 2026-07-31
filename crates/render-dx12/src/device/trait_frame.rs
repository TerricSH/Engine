macro_rules! dx12_device_frame_methods {
    () => {
        // --- Frame lifecycle ---
        fn begin_frame(
            &mut self,
            swapchain: SwapchainHandle,
        ) -> Result<(u32, Box<dyn CommandEncoder>), RhiError> {
            unsafe {
                let (_, sc_idx) = Self::decode_handle(swapchain.index);
                let sc = self
                    .swapchains
                    .get_mut(sc_idx)
                    .ok_or(RhiError::InvalidHandle)?;

                let fi = self.frame_index;

                // Wait for the most recently submitted frame before reusing any
                // allocator. `fence_value` tracks submissions only; merely
                // beginning a frame must not create a fence value that can never
                // be signalled if recording is later aborted.
                let prev_value = self.fence_value;
                if prev_value > 0 && self.fence.GetCompletedValue() < prev_value {
                    self.fence
                        .SetEventOnCompletion(prev_value, self.fence_event)
                        .map_err(|e| RhiError::Backend {
                            detail: format!("DX12: SetEventOnCompletion: {e}"),
                        })?;
                    WaitForSingleObject(self.fence_event, u32::MAX);
                }
                self.descriptor_heaps_in_flight
                    .get_mut()
                    .map_err(|_| RhiError::Backend {
                        detail: "DX12: transient descriptor heap lock is poisoned".into(),
                    })?
                    .clear();

                // Reset allocator and command list
                self.allocators[fi].Reset().map_err(|e| RhiError::Backend {
                    detail: format!("DX12: Reset allocator: {e}"),
                })?;
                self.cmd_lists[fi]
                    .Reset(&self.allocators[fi], None)
                    .map_err(|e| RhiError::Backend {
                        detail: format!("DX12: Reset cmd list: {e}"),
                    })?;

                // Get current back buffer index
                let image_index = sc.swapchain.GetCurrentBackBufferIndex();

                // Transition back buffer to render target
                let bb = &sc.back_buffers[image_index as usize];
                Self::transition_resource(
                    &self.cmd_lists[fi],
                    bb,
                    D3D12_RESOURCE_STATE_PRESENT,
                    D3D12_RESOURCE_STATE_RENDER_TARGET,
                );

                // Clear back buffer
                let rtv_handle = {
                    let cpu_start = sc.rtv_heap.GetCPUDescriptorHandleForHeapStart();
                    D3D12_CPU_DESCRIPTOR_HANDLE {
                        ptr: cpu_start.ptr + image_index as usize * sc.rtv_size as usize,
                    }
                };
                let clear_color = self.next_frame_clear_color;
                self.cmd_lists[fi].ClearRenderTargetView(rtv_handle, &clear_color, None);
                let dsv_handle = sc.dsv_heap.GetCPUDescriptorHandleForHeapStart();
                self.cmd_lists[fi].ClearDepthStencilView(
                    dsv_handle,
                    D3D12_CLEAR_FLAG_DEPTH,
                    1.0,
                    0,
                    &[],
                );

                // Bind the matching color/depth pair for the whole scene pass.
                self.cmd_lists[fi].OMSetRenderTargets(
                    1,
                    Some(&rtv_handle as *const _ as *const _),
                    false,
                    Some(&dsv_handle),
                );

                let encoder = Dx12CommandEncoder::new(
                    self.cmd_lists[fi].clone(),
                    self as *const _,
                    rtv_handle,
                    dsv_handle,
                );
                self.frame_index = (fi + 1) % Self::FRAMES_IN_FLIGHT;
                Ok((image_index, Box::new(encoder)))
            }
        }

        fn end_frame(
            &mut self,
            swapchain: SwapchainHandle,
            _encoder: Box<dyn CommandEncoder>,
            image_index: u32,
        ) -> Result<RendererStatistics, RhiError> {
            unsafe {
                let (_, sc_idx) = Self::decode_handle(swapchain.index);
                let sc = self
                    .swapchains
                    .get_mut(sc_idx)
                    .ok_or(RhiError::InvalidHandle)?;

                let fi = (self.frame_index + Self::FRAMES_IN_FLIGHT - 1) % Self::FRAMES_IN_FLIGHT;

                // Transition back buffer to present
                let bb = &sc.back_buffers[image_index as usize];
                Self::transition_resource(
                    &self.cmd_lists[fi],
                    bb,
                    D3D12_RESOURCE_STATE_RENDER_TARGET,
                    D3D12_RESOURCE_STATE_PRESENT,
                );

                // Close command list
                self.cmd_lists[fi].Close().map_err(|e| RhiError::Backend {
                    detail: format!("DX12: Close: {e}"),
                })?;

                // Reserve the submission fence value before execution. Failing
                // after ExecuteCommandLists would leave GPU work with no fence that
                // the engine can safely wait on.
                let submitted_fence_value =
                    self.fence_value
                        .checked_add(1)
                        .ok_or_else(|| RhiError::Backend {
                            detail: "DX12: fence value overflow".to_string(),
                        })?;

                // Execute
                let cmd_lists: [Option<ID3D12CommandList>; 1] =
                    [Some(self.cmd_lists[fi].clone().cast().map_err(|e| {
                        RhiError::Backend {
                            detail: format!("DX12: cast to ID3D12CommandList: {e}"),
                        }
                    })?)];
                self.queue.ExecuteCommandLists(&cmd_lists);

                // Signal the reserved fence value after command-list execution.
                self.queue
                    .Signal(&self.fence, submitted_fence_value)
                    .map_err(|e| RhiError::Backend {
                        detail: format!("DX12: Signal: {e}"),
                    })?;
                self.fence_value = submitted_fence_value;

                // Present
                let sync_interval = 1u32; // vsync
                sc.swapchain
                    .Present(sync_interval, DXGI_PRESENT(0))
                    .ok()
                    .map_err(|e| RhiError::Backend {
                        detail: format!("DX12: Present: {e}"),
                    })?;

                let draws = 0u32;
                let triangles = 0u64;

                Ok(RendererStatistics {
                    draw_calls: draws,
                    triangles,
                    gpu_frame_ms: 0.0,
                })
            }
        }

        fn wait_idle(&self) {
            unsafe {
                let value = self.fence_value;
                if self.fence.GetCompletedValue() < value {
                    let _ = self.fence.SetEventOnCompletion(value, self.fence_event);
                    WaitForSingleObject(self.fence_event, u32::MAX);
                }
            }
        }

        fn read_pixels(
            &mut self,
            _x: u32,
            _y: u32,
            _width: u32,
            _height: u32,
        ) -> Result<Vec<u8>, RhiError> {
            Err(RhiError::UnsupportedFeature {
                feature: "DX12 framebuffer readback".to_string(),
            })
        }
    };
}

pub(super) use dx12_device_frame_methods;
