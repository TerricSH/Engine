//! Screenshot capture utility.
//!
//! Provides [`save_framebuffer`] to read the current framebuffer via a
//! [`Device`](render_core::Device) and encode it as a PNG file.
//!
//! # Example
//!
//! ```ignore
//! use engine_renderer::screenshot::save_framebuffer;
//! use render_core::Device;
//!
//! // … render a frame …
//! save_framebuffer(&mut *device, "screenshot.png", 0, 0, 1280, 720)
//!     .expect("save screenshot");
//! ```

use std::path::Path;

use render_core::{Device, RhiError};
use thiserror::Error;

const _CHANNELS: usize = 4; // RGBA

/// Read the current framebuffer region and save it as a PNG file.
///
/// `device` is the rendering device with a completed frame.
/// `path` is the output `.png` file path.
/// `(x, y, width, height)` is the region in pixel coordinates.
pub fn save_framebuffer(
    device: &mut dyn Device,
    path: &Path,
    x: u32,
    y: u32,
    width: u32,
    height: u32,
) -> Result<(), ScreenshotError> {
    let pixels = device.read_pixels(x, y, width, height)?;
    let img = image::RgbaImage::from_raw(width, height, pixels).ok_or(ScreenshotError::Encode(
        "image dimensions do not match pixel data".to_string(),
    ))?;
    img.save(path)
        .map_err(|e| ScreenshotError::Io(e.to_string()))?;
    tracing::info!(?path, "screenshot saved");
    Ok(())
}

/// Errors that can occur during screenshot capture.
#[derive(Debug, Error)]
pub enum ScreenshotError {
    /// The device does not support framebuffer readback or encountered an error.
    #[error("device error: {0}")]
    Device(#[from] RhiError),
    /// PNG encoding failed.
    #[error("encode error: {0}")]
    Encode(String),
    /// File I/O failed.
    #[error("I/O error: {0}")]
    Io(String),
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn screenshot_error_display() {
        let err = ScreenshotError::Encode("bad data".to_string());
        assert_eq!(err.to_string(), "encode error: bad data");
    }
}
