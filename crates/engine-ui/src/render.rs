//! UI rendering extraction.
//!
//! Walks a [`Canvas`](crate::Canvas) and produces a flat list of quads
//! ([`UiRenderBatch`]) that can be converted into
//! [`engine_renderer::UiBatch`] for the rendering pipeline.

use engine_renderer::{Rect, UiBatch, UiVertex};
use engine_serialize::AssetId;

use crate::color::Color;
use crate::font;
use crate::layout::ScaleMode;
use crate::types::{ElementId, UiElement, UiElementKind};
use crate::Canvas;
use crate::DEFAULT_UI_MATERIAL;

// ---------------------------------------------------------------------------
// UiQuad
// ---------------------------------------------------------------------------

/// A single axis-aligned textured quad extracted from a UI element.
#[derive(Clone, Debug, PartialEq)]
pub struct UiQuad {
    /// World-space X position (top-left corner).
    pub x: f32,
    /// World-space Y position (top-left corner).
    pub y: f32,
    /// Width in pixels.
    pub w: f32,
    /// Height in pixels.
    pub h: f32,
    /// Tint / fill colour.
    pub color: Color,
    /// Optional texture identifier.
    pub texture_id: Option<String>,
    /// UV coordinates [u0, v0, u1, v1] for texture atlas sampling.
    /// Defaults to [0,0,1,1] (full texture) when None.
    pub uv: Option<[f32; 4]>,
    /// The source element (for debugging / tooling).
    pub source_element: ElementId,
}

// ---------------------------------------------------------------------------
// UiRenderBatch
// ---------------------------------------------------------------------------

/// A collection of [`UiQuad`]s extracted from a canvas.
///
/// This is an intermediate representation that can be converted into
/// [`engine_renderer::UiBatch`] for the GPU-driven render pipeline.
#[derive(Clone, Debug, Default)]
pub struct UiRenderBatch {
    /// Flat list of quads in draw order.
    pub quads: Vec<UiQuad>,
    /// Canvas dimensions at extraction time.
    pub canvas_width: f32,
    pub canvas_height: f32,
}

impl UiRenderBatch {
    /// Returns `true` when no quads are present.
    pub fn is_empty(&self) -> bool {
        self.quads.is_empty()
    }

    /// Number of quads in this batch.
    pub fn len(&self) -> usize {
        self.quads.len()
    }
}

// ---------------------------------------------------------------------------
// Conversion: UiRenderBatch → engine_renderer::UiBatch
// ---------------------------------------------------------------------------

impl From<&UiRenderBatch> for UiBatch {
    fn from(batch: &UiRenderBatch) -> Self {
        let clip = Rect {
            min: [0.0, 0.0],
            max: [batch.canvas_width, batch.canvas_height],
        };

        let mut vertices: Vec<UiVertex> = Vec::with_capacity(batch.quads.len() * 4);
        let mut indices: Vec<u32> = Vec::with_capacity(batch.quads.len() * 6);

        for quad in &batch.quads {
            let base = vertices.len() as u32;
            let left = quad.x;
            let right = quad.x + quad.w;
            let top = quad.y;
            let bottom = quad.y + quad.h;
            let color = [quad.color.r, quad.color.g, quad.color.b, quad.color.a];

            // Skip degenerate quads
            if quad.w <= 0.0 || quad.h <= 0.0 {
                continue;
            }

            let uv = quad.uv.unwrap_or([0.0, 0.0, 1.0, 1.0]);
            let (u0, v0, u1, v1) = (uv[0], uv[1], uv[2], uv[3]);
            vertices.push(UiVertex {
                position: [left, top],
                uv: [u0, v0],
                color,
            });
            vertices.push(UiVertex {
                position: [right, top],
                uv: [u1, v0],
                color,
            });
            vertices.push(UiVertex {
                position: [right, bottom],
                uv: [u1, v1],
                color,
            });
            vertices.push(UiVertex {
                position: [left, bottom],
                uv: [u0, v1],
                color,
            });

            indices.push(base);
            indices.push(base + 1);
            indices.push(base + 2);
            indices.push(base);
            indices.push(base + 2);
            indices.push(base + 3);
        }

        // Determine texture: use the first quad's texture if all match,
        // otherwise break into multiple batches would be needed in a more
        // advanced implementation.  For simplicity, use the first non-None
        // texture found.
        let texture = batch.quads.iter().find_map(|q| {
            q.texture_id
                .as_ref()
                .map(|id| Some(AssetId::new(id.clone())))
                .unwrap_or(None)
        });

        UiBatch {
            canvas_id: String::new(),
            z_order: 0,
            clip_rect: clip,
            texture,
            vertices,
            indices,
            material: AssetId::new(DEFAULT_UI_MATERIAL),
        }
    }
}

// ---------------------------------------------------------------------------
// Extraction
// ---------------------------------------------------------------------------

/// Walk the enabled elements of `canvas` and produce a flat list of quads.
///
/// Elements are sorted by `z_order` (ascending) before extraction so that
/// the resulting quad list is already in back-to-front draw order.
///
/// Call [`Canvas::layout_all`] before this to ensure pixel rects are current.
pub fn extract_ui_quads(canvas: &Canvas) -> UiRenderBatch {
    let mut visible: Vec<&UiElement> = canvas.elements.iter().filter(|e| e.enabled).collect();
    visible.sort_by_key(|e| e.z_order);

    let mut batch = UiRenderBatch {
        canvas_width: canvas.width,
        canvas_height: canvas.height,
        quads: Vec::with_capacity(visible.len()),
    };

    for element in &visible {
        // Handle text elements specially: emit one quad per glyph.
        if let UiElementKind::Text {
            content,
            font_size,
            color,
        } = &element.kind
        {
            let rect = element.rect;
            if rect.width > 0.0 && rect.height > 0.0 {
                if let Some(verts) = font::render_text(content, *font_size, *color, &rect) {
                    for chunk in verts.chunks(4) {
                        if chunk.len() < 4 {
                            continue;
                        }
                        let gx = chunk[0].position[0];
                        let gy = chunk[0].position[1];
                        let gx2 = chunk[2].position[0];
                        let gy2 = chunk[2].position[1];
                        batch.quads.push(UiQuad {
                            x: gx,
                            y: gy,
                            w: (gx2 - gx).max(1.0),
                            h: (gy2 - gy).max(1.0),
                            color: *color,
                            texture_id: Some(crate::font::FONT_ATLAS_ASSET.to_string()),
                            uv: Some([
                                chunk[0].uv[0],
                                chunk[0].uv[1],
                                chunk[2].uv[0],
                                chunk[2].uv[1],
                            ]),
                            source_element: element.id,
                        });
                    }
                    continue;
                }
            }
            // No font or empty glyphs: fall through to placeholder below.
        }

        let quad = element_to_quad(element);
        if let Some(q) = quad {
            batch.quads.push(q);
        }
    }

    batch
}

/// Convert a single [`UiElement`] into an optional [`UiQuad`].
///
/// Returns `None` for degenerate (zero-size) elements.
fn element_to_quad(element: &UiElement) -> Option<UiQuad> {
    let rect = element.rect;
    if rect.width <= 0.0 || rect.height <= 0.0 {
        return None;
    }

    let source_element = element.id;

    match &element.kind {
        UiElementKind::Panel { color } => Some(UiQuad {
            x: rect.x,
            y: rect.y,
            w: rect.width,
            h: rect.height,
            color: *color,
            texture_id: None,
            uv: None,
            source_element,
        }),

        UiElementKind::Image { texture_id, color } => Some(UiQuad {
            x: rect.x,
            y: rect.y,
            w: rect.width,
            h: rect.height,
            color: *color,
            texture_id: Some(texture_id.clone()),
            uv: None,
            source_element,
        }),

        UiElementKind::Text {
            content: _content,
            font_size: _font_size,
            color,
        } => {
            // Font rendering is handled by extract_ui_quads for
            // multi-glyph output.  Here we only produce the fallback
            // placeholder quad when no font is available.
            let mut c = *color;
            c.a /= 2;
            Some(UiQuad {
                x: rect.x,
                y: rect.y,
                w: rect.width,
                h: rect.height,
                color: c,
                texture_id: None,
                uv: None,
                source_element,
            })
        }

        UiElementKind::Button { normal_color, .. } => Some(UiQuad {
            x: rect.x,
            y: rect.y,
            w: rect.width,
            h: rect.height,
            color: *normal_color,
            texture_id: None,
            uv: None,
            source_element,
        }),

        UiElementKind::Toggle {
            color_on, is_on, ..
        } => {
            let c = if *is_on {
                *color_on
            } else {
                crate::color::Color::new(100, 100, 100, 255)
            };
            Some(UiQuad {
                x: rect.x,
                y: rect.y,
                w: rect.width,
                h: rect.height,
                color: c,
                texture_id: None,
                uv: None,
                source_element,
            })
        }

        UiElementKind::Checkbox { checked, color, .. } => {
            let c = if *checked {
                *color
            } else {
                crate::color::Color::new(80, 80, 80, 255)
            };
            Some(UiQuad {
                x: rect.x,
                y: rect.y,
                w: rect.width,
                h: rect.height,
                color: c,
                texture_id: None,
                uv: None,
                source_element,
            })
        }

        UiElementKind::Slider {
            value, min, max, ..
        } => {
            let t = if (*max - *min).abs() > 1e-6 {
                ((*value - *min) / (*max - *min)).clamp(0.0, 1.0)
            } else {
                0.0
            };
            let mut c = crate::color::Color::new(60, 60, 60, 255);
            c.r = (c.r as f32 * (1.0 - t) + 200.0 * t) as u8;
            Some(UiQuad {
                x: rect.x,
                y: rect.y,
                w: rect.width,
                h: rect.height,
                color: c,
                texture_id: None,
                uv: None,
                source_element,
            })
        }

        UiElementKind::ScrollView { color, .. } => Some(UiQuad {
            x: rect.x,
            y: rect.y,
            w: rect.width,
            h: rect.height,
            color: *color,
            texture_id: None,
            uv: None,
            source_element,
        }),
    }
}

// ---------------------------------------------------------------------------
// Scale helper
// ---------------------------------------------------------------------------

/// Return the scale factor to apply to canvas coordinates given the
/// viewport size and the canvas's scale mode.
pub fn canvas_scale(canvas: &Canvas, viewport_width: f32, viewport_height: f32) -> f32 {
    match canvas.scale_mode {
        ScaleMode::Fixed => 1.0,
        ScaleMode::FitWidth => {
            if canvas.width <= 0.0 {
                1.0
            } else {
                viewport_width / canvas.width
            }
        }
        ScaleMode::FitHeight => {
            if canvas.height <= 0.0 {
                1.0
            } else {
                viewport_height / canvas.height
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::color::Color;
    use crate::layout::Layout;
    use crate::types::{UiElement, UiElementKind};
    use crate::Canvas;
    use glam::Vec2;

    fn panel_element(layout: Layout, z: i32, color: Color) -> UiElement {
        UiElement::new(UiElementKind::Panel { color }, layout).with_z_order(z)
    }

    fn make_canvas() -> Canvas {
        let mut canvas = Canvas::new(800.0, 600.0);
        let l1 = Layout::new(
            Vec2::ZERO,
            Vec2::ZERO,
            Vec2::new(10.0, 20.0),
            Vec2::new(210.0, 120.0),
        );
        canvas.add_element(panel_element(l1, 0, Color::WHITE));
        canvas.layout_all();
        canvas
    }

    #[test]
    fn extract_quads_empty_when_no_enabled() {
        let mut canvas = Canvas::new(100.0, 100.0);
        canvas.add_element(
            UiElement::new(
                UiElementKind::Panel {
                    color: Color::WHITE,
                },
                Layout::FILL,
            )
            .with_enabled(false),
        );
        canvas.layout_all();
        let batch = extract_ui_quads(&canvas);
        assert!(batch.is_empty());
    }

    #[test]
    fn extract_quads_single_panel() {
        let canvas = make_canvas();
        let batch = extract_ui_quads(&canvas);
        assert_eq!(batch.len(), 1);
        assert_eq!(batch.quads[0].x, 10.0);
        assert_eq!(batch.quads[0].y, 20.0);
        assert_eq!(batch.quads[0].w, 200.0);
        assert_eq!(batch.quads[0].h, 100.0);
        assert!(batch.quads[0].texture_id.is_none());
    }

    #[test]
    fn extract_quads_image_has_texture() {
        let mut canvas = Canvas::new(800.0, 600.0);
        let layout = Layout::new(Vec2::ZERO, Vec2::ZERO, Vec2::ZERO, Vec2::new(64.0, 64.0));
        canvas.add_element(
            UiElement::new(
                UiElementKind::Image {
                    texture_id: "ui/icon".into(),
                    color: Color::WHITE,
                },
                layout,
            )
            .with_z_order(0),
        );
        canvas.layout_all();
        let batch = extract_ui_quads(&canvas);
        assert_eq!(batch.len(), 1);
        assert_eq!(batch.quads[0].texture_id.as_deref(), Some("ui/icon"));
    }

    #[test]
    fn extract_quads_text_semitransparent() {
        let mut canvas = Canvas::new(800.0, 600.0);
        let layout = Layout::new(Vec2::ZERO, Vec2::ZERO, Vec2::ZERO, Vec2::new(50.0, 20.0));
        canvas.add_element(
            UiElement::new(
                UiElementKind::Text {
                    content: "hello".into(),
                    font_size: 16.0,
                    color: Color::new(255, 0, 0, 255),
                },
                layout,
            )
            .with_z_order(0),
        );
        canvas.layout_all();
        let batch = extract_ui_quads(&canvas);
        assert_eq!(batch.quads[0].color.a, 127);
    }

    #[test]
    fn extract_quads_button() {
        let mut canvas = Canvas::new(800.0, 600.0);
        let layout = Layout::new(Vec2::ZERO, Vec2::ZERO, Vec2::ZERO, Vec2::new(100.0, 40.0));
        canvas.add_element(
            UiElement::new(
                UiElementKind::Button {
                    label: "Click".into(),
                    normal_color: Color::new(100, 100, 200, 255),
                    hover_color: Color::new(120, 120, 220, 255),
                    pressed_color: Color::new(80, 80, 180, 255),
                    callback_id: Some("btn_ok".into()),
                },
                layout,
            )
            .with_z_order(0),
        );
        canvas.layout_all();
        let batch = extract_ui_quads(&canvas);
        assert_eq!(batch.len(), 1);
        // Button uses normal_color
        assert_eq!(batch.quads[0].color, Color::new(100, 100, 200, 255));
    }

    #[test]
    fn extract_quads_skips_negative_size() {
        let mut canvas = Canvas::new(800.0, 600.0);
        let layout = Layout::new(
            Vec2::new(1.0, 1.0),
            Vec2::ZERO, // inverted → negative size
            Vec2::ZERO,
            Vec2::ZERO,
        );
        canvas.add_element(
            UiElement::new(
                UiElementKind::Panel {
                    color: Color::WHITE,
                },
                layout,
            )
            .with_z_order(0),
        );
        canvas.layout_all();
        let batch = extract_ui_quads(&canvas);
        assert!(batch.is_empty());
    }

    #[test]
    fn render_batch_conversion_to_uibatch() {
        let canvas = make_canvas();
        let render_batch = extract_ui_quads(&canvas);
        let ui_batch: UiBatch = UiBatch::from(&render_batch);
        assert_eq!(ui_batch.vertices.len(), 4);
        assert_eq!(ui_batch.indices.len(), 6);
        assert_eq!(ui_batch.vertices[0].position, [10.0, 20.0]);
        assert_eq!(ui_batch.clip_rect.max, [800.0, 600.0]);
    }

    #[test]
    fn canvas_scale_fixed() {
        let canvas = Canvas::new(800.0, 600.0);
        assert!((canvas_scale(&canvas, 1920.0, 1080.0) - 1.0).abs() < f32::EPSILON);
    }

    #[test]
    fn canvas_scale_fit_width() {
        let mut canvas = Canvas::new(800.0, 600.0);
        canvas.scale_mode = ScaleMode::FitWidth;
        let s = canvas_scale(&canvas, 1920.0, 1080.0);
        assert!((s - 2.4).abs() < f32::EPSILON);
    }

    #[test]
    fn canvas_scale_fit_height() {
        let mut canvas = Canvas::new(800.0, 600.0);
        canvas.scale_mode = ScaleMode::FitHeight;
        let s = canvas_scale(&canvas, 1920.0, 1080.0);
        assert!((s - 1.8).abs() < f32::EPSILON);
    }
}
