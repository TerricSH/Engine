macro_rules! impl_device_frame {
    () => {
        fn begin_frame(
            &mut self,
            _swapchain: SwapchainHandle,
        ) -> Result<(u32, Box<dyn CommandEncoder>), RhiError> {
            Err(opengl_presentation_unsupported())
        }

        fn end_frame(
            &mut self,
            _swapchain: SwapchainHandle,
            _encoder: Box<dyn CommandEncoder>,
            _image_index: u32,
        ) -> Result<RendererStatistics, RhiError> {
            Err(opengl_presentation_unsupported())
        }

        fn recreate_swapchain(
            &mut self,
            _swapchain: SwapchainHandle,
            _width: u32,
            _height: u32,
        ) -> Result<(), RhiError> {
            Err(opengl_presentation_unsupported())
        }

        fn wait_idle(&self) {
            // SAFETY: `self.gl` is a valid `glow::Context` created by this device;
            // the device is alive and not yet destroyed.
            unsafe {
                self.gl.finish();
            }
        }

        // ██ framebuffer readback ███████████████████████████████████████████████████████████████

        fn read_pixels(
            &mut self,
            x: u32,
            y: u32,
            width: u32,
            height: u32,
        ) -> Result<Vec<u8>, RhiError> {
            if width == 0 || height == 0 {
                return Ok(Vec::new());
            }

            let size = (width as usize)
                .checked_mul(height as usize)
                .and_then(|v| v.checked_mul(4))
                .ok_or(RhiError::Backend {
                    detail: "read_pixels: integer overflow in buffer size".to_string(),
                })?;

            let mut pixels = vec![0u8; size];

            // SAFETY: glow's read_pixels writes RGBA data into the pixel buffer.
            // The buffer is sized exactly to hold (width × height × 4) bytes.
            unsafe {
                self.gl.read_pixels(
                    x as i32,
                    y as i32,
                    width as i32,
                    height as i32,
                    glow::RGBA,
                    glow::UNSIGNED_BYTE,
                    glow::PixelPackData::Slice(Some(&mut pixels)),
                );
            }

            // OpenGL reads rows bottom-to-top; the trait contract specifies
            // top-to-bottom rows. Flip the rows in a second buffer.
            let row_size = (width as usize) * 4;
            let mut flipped = vec![0u8; size];
            for row in 0..height as usize {
                let src_start = (height as usize - 1 - row) * row_size;
                let dst_start = row * row_size;
                flipped[dst_start..dst_start + row_size]
                    .copy_from_slice(&pixels[src_start..src_start + row_size]);
            }

            Ok(flipped)
        }

        // ============================================================================
        // Public constructor
        // ============================================================================
    };
}

pub(super) use impl_device_frame;
