//! Font atlas and glyph rendering for the UI system.
//!
//! Uses `ab_glyph` to rasterise a project or platform font into a shared
//! texture atlas. If no usable font is available, text is deliberately
//! skipped and a structured, one-shot diagnostic explains how to fix it.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{LazyLock, Mutex};

use ab_glyph::{point, Font, FontArc, Glyph, GlyphId, PxScale, ScaleFont};
use engine_renderer::{
    ColorSpace, SamplerAddressMode, SamplerDescriptor, TextureMipLevel, TextureUpload,
    TextureUploadFormat,
};
use engine_serialize::{AssetId, Diagnostic, DiagnosticSeverity};
use sha2::{Digest, Sha256};

use crate::color::Color;
use crate::types::UiRect;

// ---------------------------------------------------------------------------
// Constants
// ---------------------------------------------------------------------------

/// Default font search paths (checked in order).
const PROJECT_FONT_PATHS: &[&str] = &[
    "assets/fonts/DejaVuSans.ttf",
    "assets/fonts/NotoSans-Regular.ttf",
    "assets/fonts/OpenSans-Regular.ttf",
    "assets/fonts/Roboto-Regular.ttf",
    "assets/fonts/Arial.ttf",
];

fn font_search_paths() -> Vec<PathBuf> {
    let mut paths = PROJECT_FONT_PATHS
        .iter()
        .map(PathBuf::from)
        .collect::<Vec<_>>();

    // A packaged project font remains the preferred, deterministic source.
    // These platform fonts keep development builds usable before a project
    // has copied a font into assets/fonts.
    #[cfg(target_os = "windows")]
    paths.extend([
        PathBuf::from(r"C:\Windows\Fonts\segoeui.ttf"),
        PathBuf::from(r"C:\Windows\Fonts\arial.ttf"),
    ]);
    #[cfg(target_os = "linux")]
    paths.extend([
        PathBuf::from("/usr/share/fonts/truetype/dejavu/DejaVuSans.ttf"),
        PathBuf::from("/usr/share/fonts/truetype/liberation2/LiberationSans-Regular.ttf"),
    ]);
    #[cfg(target_os = "macos")]
    paths.extend([
        PathBuf::from("/System/Library/Fonts/Supplemental/Arial.ttf"),
        PathBuf::from("/System/Library/Fonts/SFNS.ttf"),
    ]);

    paths
}

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

/// Detectable state of the shared runtime font atlas.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum FontAtlasAvailability {
    /// A real font was loaded from this project or platform path.
    Ready { source: PathBuf },
    /// No usable font was found. Text extraction skips glyph output.
    Unavailable { searched_paths: Vec<PathBuf> },
}

impl FontAtlasAvailability {
    pub fn is_ready(&self) -> bool {
        matches!(self, Self::Ready { .. })
    }
}

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
    availability: FontAtlasAvailability,
    diagnostic_pending: bool,
}

impl FontAtlas {
    /// Create a new font atlas, attempting to load a font from the search paths.
    pub fn new() -> Self {
        Self::from_search_paths(font_search_paths())
    }

    fn from_search_paths(search_paths: Vec<PathBuf>) -> Self {
        let loaded = search_paths.iter().find_map(|full_path| {
            if full_path.exists() {
                let bytes = std::fs::read(full_path).ok()?;
                FontArc::try_from_vec(bytes)
                    .ok()
                    .map(|font| (full_path.clone(), font))
            } else {
                None
            }
        });
        let (font, availability) = match loaded {
            Some((source, font)) => (Some(font), FontAtlasAvailability::Ready { source }),
            None => (
                None,
                FontAtlasAvailability::Unavailable {
                    searched_paths: search_paths.clone(),
                },
            ),
        };
        let is_ready = font.is_some();
        if !is_ready {
            tracing::warn!(
                "no usable runtime UI font found at {:?}; text output will be skipped",
                search_paths
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
            availability,
            diagnostic_pending: !is_ready,
        }
    }

    /// Return the exact source or missing-font state used by this atlas.
    pub fn availability(&self) -> &FontAtlasAvailability {
        &self.availability
    }

    fn take_diagnostics(&mut self) -> Vec<Diagnostic> {
        if !self.diagnostic_pending {
            return Vec::new();
        }
        self.diagnostic_pending = false;
        let FontAtlasAvailability::Unavailable { searched_paths } = &self.availability else {
            return Vec::new();
        };
        let paths = searched_paths
            .iter()
            .map(|path| path.display().to_string())
            .collect::<Vec<_>>()
            .join(", ");
        let mut diagnostic = Diagnostic::new(
            "UI_FONT_UNAVAILABLE",
            DiagnosticSeverity::Warning,
            "engine-ui.font",
            "No usable runtime UI font is available; text elements are skipped instead of rendering placeholder geometry.",
        );
        diagnostic.recoverable = true;
        diagnostic.suggested_action = Some(
            "Add a supported .ttf font under assets/fonts (for example NotoSans-Regular.ttf) and rebuild the project."
                .to_string(),
        );
        diagnostic.fields.insert("searched_paths".into(), paths);
        vec![diagnostic]
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
        let mut pen_y = rect.y + font_size;
        for c in text.chars() {
            if c == '\n' {
                pen_x = rect.x;
                pen_y += font_size * 1.2;
                continue;
            }
            self.cache_glyph(c, font_size);
            let key = (c, (font_size * 10.0) as u32);
            let Some(g) = self.cache.get(&key) else {
                continue;
            };
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
// Global font atlas singleton (created on first use)
// ---------------------------------------------------------------------------

static FONT_ATLAS: LazyLock<Mutex<FontAtlas>> = LazyLock::new(|| Mutex::new(FontAtlas::new()));

/// Render a text string into glyph quads using the global font atlas.
/// Returns `None` when no real font is available. Callers must skip text in
/// that case; placeholder geometry would misrepresent the authored content.
pub fn render_text(
    text: &str,
    font_size: f32,
    color: Color,
    rect: &UiRect,
) -> Option<Vec<engine_renderer::UiVertex>> {
    let mut atlas = FONT_ATLAS
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    if !atlas.is_ready {
        return None;
    }
    Some(atlas.text_quads(text, font_size, color, rect))
}

/// Return the current global font source or unavailable state.
pub fn font_atlas_availability() -> FontAtlasAvailability {
    FONT_ATLAS
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .availability()
        .clone()
}

/// Drain the missing-font diagnostic once for the global atlas.
///
/// A ready atlas always returns an empty vector. An unavailable atlas returns
/// `UI_FONT_UNAVAILABLE` on the first call and an empty vector afterwards.
pub fn take_font_atlas_diagnostics() -> Vec<Diagnostic> {
    FONT_ATLAS
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .take_diagnostics()
}

/// Snapshot the current global glyph atlas as a renderer texture upload.
///
/// Calling [`render_text`] may add glyphs, so hosts should request this upload
/// after building UI batches for the frame. The content hash lets the runtime
/// and backend deduplicate unchanged snapshots.
pub fn font_atlas_texture_upload() -> Option<TextureUpload> {
    let atlas = FONT_ATLAS
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    if !atlas.is_ready {
        return None;
    }

    let content_hash = Sha256::digest(&atlas.pixels).into();
    Some(TextureUpload {
        texture_id: AssetId::new(FONT_ATLAS_ASSET),
        width: atlas.width,
        height: atlas.height,
        format: TextureUploadFormat::Rgba8,
        color_space: ColorSpace::Linear,
        mip_levels: vec![TextureMipLevel {
            width: atlas.width,
            height: atlas.height,
            bytes: atlas.pixels.clone(),
        }],
        sampler: SamplerDescriptor {
            address_u: SamplerAddressMode::ClampToEdge,
            address_v: SamplerAddressMode::ClampToEdge,
            address_w: SamplerAddressMode::ClampToEdge,
            ..SamplerDescriptor::default()
        },
        content_hash,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn missing_font_path() -> PathBuf {
        let nonce = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("system time must follow the Unix epoch")
            .as_nanos();
        std::env::temp_dir().join(format!(
            "engine-ui-definitely-missing-font-{}-{nonce}.ttf",
            std::process::id()
        ))
    }

    #[test]
    fn unavailable_font_skips_text_and_reports_one_structured_diagnostic() {
        let path = missing_font_path();
        assert!(!path.exists());
        let mut atlas = FontAtlas::from_search_paths(vec![path.clone()]);

        assert_eq!(
            atlas.availability(),
            &FontAtlasAvailability::Unavailable {
                searched_paths: vec![path]
            }
        );
        assert!(!atlas.is_ready);
        assert!(atlas
            .text_quads(
                "This text must not become a fake rectangle",
                16.0,
                Color::WHITE,
                &UiRect::new(0.0, 0.0, 320.0, 40.0),
            )
            .is_empty());

        let diagnostics = atlas.take_diagnostics();
        assert_eq!(diagnostics.len(), 1);
        assert_eq!(diagnostics[0].code, "UI_FONT_UNAVAILABLE");
        assert_eq!(diagnostics[0].severity, DiagnosticSeverity::Warning);
        assert!(diagnostics[0].recoverable);
        assert!(diagnostics[0]
            .suggested_action
            .as_deref()
            .is_some_and(|action| action.contains("assets/fonts")));
        assert!(atlas.take_diagnostics().is_empty());
    }

    #[test]
    fn global_availability_matches_texture_upload_contract() {
        assert_eq!(
            font_atlas_availability().is_ready(),
            font_atlas_texture_upload().is_some()
        );
    }
}
