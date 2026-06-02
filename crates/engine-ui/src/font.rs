//! Font atlas and glyph rendering for the UI system.
//!
//! Uses `ab_glyph` to rasterise glyphs into a shared texture atlas.
//! The font binary is loaded from a well-known path at init time.
//! If no font is available text falls back to the legacy placeholder-quad
//! render path, so the system is always functional.

use std::collections::HashMap;
use std::sync::{LazyLock, Mutex};

use ab_glyph::{point, Font, FontArc, Glyph, GlyphId, PxScale, ScaleFont};

use crate::color::Color;
use crate::types::UiRect;

// ---------------------------------------------------------------------------
// Constants
// ---------------------------------------------------------------------------

/// Default font search paths (checked in order).
const FONT_PATHS: &[&str] = &[
    "assets/fonts/DejaVuSans.ttf",
    "assets/fonts/NotoSans-Regular.ttf",
    "assets/fonts/OpenSans-Regular.ttf",
    "assets/fonts/Roboto-Regular.ttf",
    "assets/fonts/Arial.ttf",
];

/// Atlas padding between glyphs (pixels).
const ATLAS_PADDING: u32 = 2;

/// Default atlas size.
const ATLAS_SIZE: u32 = 512;

// ---------------------------------------------------------------------------
// Cached glyph entry
// ---------------------------------------------------------------------------

struct CachedGlyph {
    uv: [f32; 4],
    advance: f32,
    bearing_x: f32,
    bearing_y: f32,
    gw: f32,
    gh: f32,
}

// ---------------------------------------------------------------------------
// FontAtlas
// ---------------------------------------------------------------------------

/// A texture atlas that caches rasterised glyphs.
///
/// Created on first use and shared across all text elements.
pub struct FontAtlas {
    pub(crate) pixels: Vec<u8>,
    pub width: u32,
    pub height: u32,
    pub is_ready: bool,
    cache: HashMap<(char, u32), CachedGlyph>,
    cursor_x: u32,
    cursor_y: u32,
    row_height: u32,
    font: Option<FontArc>,
}

impl FontAtlas {
    /// Create a new font atlas, attempting to load a font from the search paths.
    pub fn new() -> Self {
        let font = FONT_PATHS.iter().find_map(|path| {
            let full_path = std::path::Path::new(path);
            if full_path.exists() {
                let bytes = std::fs::read(full_path).ok()?;
                FontArc::try_from_vec(bytes).ok()
            } else {
                None
            }
        });

        let is_ready = font.is_some();
        if !is_ready {
            tracing::warn!(
                "no font found at {:?}; text elements will render as placeholder",
                FONT_PATHS
            );
        }

        Self {
            pixels: vec![0u8; (ATLAS_SIZE * ATLAS_SIZE * 4) as usize],
            width: ATLAS_SIZE,
            height: ATLAS_SIZE,
            is_ready,
            cache: HashMap::new(),
            cursor_x: 1,
            cursor_y: 1,
            row_height: 0,
            font,
        }
    }

    /// Ensure a glyph is rasterised and cached.
    fn cache_glyph(&mut self, c: char, font_size: f32) {
        let font = match &self.font {
            Some(f) => f,
            None => return,
        };
        let key = (c, (font_size * 10.0) as u32);
        if self.cache.contains_key(&key) {
            return;
        }

        let scale = PxScale::from(font_size);
        let scaled = font.as_scaled(scale);
        let glyph_id: GlyphId = scaled.font.glyph_id(c);

        let entry = if let Some(outline) = scaled.outline_glyph(Glyph {
            id: glyph_id,
            scale,
            position: point(0.0, 0.0),
        }) {
            let bounds = outline.px_bounds();
            let gw = bounds.width() as u32;
            let gh = bounds.height() as u32;

            if gw == 0 || gh == 0 {
                CachedGlyph {
                    uv: [0.0; 4],
                    advance: scaled.h_advance(glyph_id),
                    bearing_x: scaled.h_side_bearing(glyph_id),
                    bearing_y: 0.0,
                    gw: 0.0,
                    gh: 0.0,
                }
            } else {
                let gw_padded = gw + ATLAS_PADDING;
                let gh_padded = gh + ATLAS_PADDING;
                if self.cursor_x + gw_padded > self.width {
                    self.cursor_x = 1;
                    self.cursor_y += self.row_height + ATLAS_PADDING;
                    self.row_height = 0;
                }
                if self.cursor_y + gh_padded > self.height {
                    tracing::warn!("font atlas full, glyph '{c}' skipped");
                    return;
                }
                let ax = self.cursor_x;
                let ay = self.cursor_y;
                self.row_height = self.row_height.max(gh_padded);
                self.cursor_x += gw_padded;
                outline.draw(|x, y, cover| {
                    let idx = (((ay + y) * self.width + (ax + x)) * 4) as usize;
                    if idx + 3 < self.pixels.len() {
                        self.pixels[idx] = 255;
                        self.pixels[idx + 1] = 255;
                        self.pixels[idx + 2] = 255;
                        self.pixels[idx + 3] = (cover * 255.0) as u8;
                    }
                });
                CachedGlyph {
                    uv: [
                        ax as f32 / self.width as f32,
                        ay as f32 / self.height as f32,
                        (ax + gw) as f32 / self.width as f32,
                        (ay + gh) as f32 / self.height as f32,
                    ],
                    advance: scaled.h_advance(glyph_id),
                    bearing_x: bounds.min.x,
                    bearing_y: bounds.min.y,
                    gw: gw as f32,
                    gh: gh as f32,
                }
            }
        } else {
            CachedGlyph {
                uv: [0.0; 4],
                advance: scaled.h_advance(glyph_id),
                bearing_x: scaled.h_side_bearing(glyph_id),
                bearing_y: 0.0,
                gw: 0.0,
                gh: 0.0,
            }
        };
        self.cache.insert(key, entry);
    }

    /// Generate textured quads for a line of text.
    pub fn text_quads(
        &mut self,
        text: &str,
        font_size: f32,
        color: Color,
        rect: &crate::types::UiRect,
    ) -> Vec<engine_renderer::UiVertex> {
        let mut verts = Vec::new();
        let font = match &self.font {
            Some(f) => f,
            None => return verts,
        };
        let scale = PxScale::from(font_size);
        let _ = font.as_scaled(scale); // ensures glyph advance cache is populated
        let mut pen_x = rect.x;
        let pen_y = rect.y + font_size;
        for c in text.chars() {
            self.cache_glyph(c, font_size);
            let key = (c, (font_size * 10.0) as u32);
            let Some(g) = self.cache.get(&key) else {
                continue;
            };
            if c == '\n' {
                pen_x = rect.x;
                continue;
            }
            if g.gw > 0.0 && g.gh > 0.0 {
                let gx = pen_x + g.bearing_x;
                let gy = pen_y + g.bearing_y;
                let c4 = [color.r, color.g, color.b, color.a];
                verts.push(engine_renderer::UiVertex {
                    position: [gx, gy],
                    uv: [g.uv[0], g.uv[1]],
                    color: c4,
                });
                verts.push(engine_renderer::UiVertex {
                    position: [gx + g.gw, gy],
                    uv: [g.uv[2], g.uv[1]],
                    color: c4,
                });
                verts.push(engine_renderer::UiVertex {
                    position: [gx + g.gw, gy + g.gh],
                    uv: [g.uv[2], g.uv[3]],
                    color: c4,
                });
                verts.push(engine_renderer::UiVertex {
                    position: [gx, gy + g.gh],
                    uv: [g.uv[0], g.uv[3]],
                    color: c4,
                });
            }
            pen_x += g.advance;
        }
        verts
    }
}

/// Asset ID used for the font atlas texture in the rendering pipeline.
pub const FONT_ATLAS_ASSET: &str = "engine/font-atlas";

// ---------------------------------------------------------------------------
// Global font atlas singleton (lazy — created on first use)
// ---------------------------------------------------------------------------

static FONT_ATLAS: LazyLock<Mutex<FontAtlas>> = LazyLock::new(|| Mutex::new(FontAtlas::new()));

/// Render a text string into glyph quads using the global font atlas.
/// Returns `None` when no font is available (caller should fall back to placeholder).
pub fn render_text(
    text: &str,
    font_size: f32,
    color: Color,
    rect: &UiRect,
) -> Option<Vec<engine_renderer::UiVertex>> {
    let mut atlas = FONT_ATLAS.lock().ok()?;
    if !atlas.is_ready {
        return None;
    }
    Some(atlas.text_quads(text, font_size, color, rect))
}

/// Access the global atlas pixel data (for texture upload).
#[allow(dead_code)]
pub fn atlas_pixels() -> Option<(Vec<u8>, u32, u32)> {
    let atlas = FONT_ATLAS.lock().ok()?;
    if !atlas.is_ready {
        return None;
    }
    Some((atlas.pixels.clone(), atlas.width, atlas.height))
}
