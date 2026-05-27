use engine_renderer::{Rect, UiBatch};
use engine_serialize::{AssetId, PersistentId};
use std::collections::BTreeMap;
use tracing::debug;

use crate::batch;
use crate::types::{UiElement, UiElementKind};
use crate::{DEFAULT_UI_MATERIAL, ElementId};

// ---------------------------------------------------------------------------
// Canvas
// ---------------------------------------------------------------------------

/// A 2D drawing canvas that produces [`engine_renderer::UiBatch`]es.
///
/// Elements are ordered by `z_order` (ascending) at batch-creation time.
/// Elements sharing the same `z_order` *and* texture are merged into a single
/// batch to reduce draw calls.
pub struct Canvas {
    id: PersistentId,
    width: u32,
    height: u32,
    elements: BTreeMap<ElementId, UiElement>,
    next_id: u32,
}

impl Canvas {
    /// Create a new canvas with the given persistent id and pixel dimensions.
    pub fn new(id: impl Into<PersistentId>, width: u32, height: u32) -> Self {
        let id = id.into();
        debug!(
            canvas_id = %id,
            width,
            height,
            "Canvas created"
        );
        Self {
            id,
            width,
            height,
            elements: BTreeMap::new(),
            next_id: 1,
        }
    }

    /// Persistent identifier for this canvas.
    pub fn id(&self) -> &PersistentId {
        &self.id
    }

    /// Canvas width in pixels.
    pub fn width(&self) -> u32 {
        self.width
    }

    /// Canvas height in pixels.
    pub fn height(&self) -> u32 {
        self.height
    }

    /// Resize the canvas (does not reposition existing elements).
    pub fn resize(&mut self, width: u32, height: u32) {
        debug!(
            canvas_id = %self.id,
            old_width = self.width,
            old_height = self.height,
            new_width = width,
            new_height = height,
            "Canvas resized"
        );
        self.width = width;
        self.height = height;
    }

    /// Add a [`UiElement`] to the canvas, assigning it a new [`ElementId`].
    ///
    /// The element's `id` field is overwritten.  Returns the assigned id.
    pub fn add_element(&mut self, mut element: UiElement) -> ElementId {
        let id = ElementId(self.next_id);
        self.next_id += 1;
        element.id = id;
        self.elements.insert(id, element);
        debug!(
            canvas_id = %self.id,
            element_id = ?id,
            "Element added to canvas"
        );
        id
    }

    /// Remove an element from the canvas by id.
    ///
    /// Returns `true` if the element existed and was removed.
    pub fn remove_element(&mut self, id: ElementId) -> bool {
        let removed = self.elements.remove(&id).is_some();
        if removed {
            debug!(
                canvas_id = %self.id,
                element_id = ?id,
                "Element removed from canvas"
            );
        }
        removed
    }

    /// Borrow an element by id.
    pub fn get_element(&self, id: ElementId) -> Option<&UiElement> {
        self.elements.get(&id)
    }

    /// Mutable borrow of an element by id.
    pub fn get_element_mut(&mut self, id: ElementId) -> Option<&mut UiElement> {
        self.elements.get_mut(&id)
    }

    /// Remove all elements from the canvas.
    pub fn clear(&mut self) {
        let count = self.elements.len();
        self.elements.clear();
        self.next_id = 1;
        debug!(
            canvas_id = %self.id,
            count,
            "Canvas cleared"
        );
    }

    /// Build a list of [`UiBatch`]es from the visible elements on this canvas.
    ///
    /// Elements are sorted by `z_order` (ascending).  Consecutive elements
    /// sharing the same `z_order` *and* texture are merged into one batch to
    /// minimize draw-calls.  Returns an empty Vec when there are no visible
    /// elements.
    pub fn build_batches(&self) -> Vec<UiBatch> {
        let mut visible: Vec<&UiElement> = self.elements.values().filter(|e| e.visible).collect();
        if visible.is_empty() {
            return Vec::new();
        }
        visible.sort_by_key(|e| e.z_order);

        let canvas_w = self.width as f32;
        let canvas_h = self.height as f32;
        let clip = Rect {
            min: [0.0, 0.0],
            max: [canvas_w, canvas_h],
        };

        let mut batches: Vec<UiBatch> = Vec::new();

        for element in &visible {
            let texture = batch::element_kind_texture(&element.kind);

            // Start a new batch when z_order or texture changes
            let new_batch = batches.last().map_or(true, |b: &UiBatch| {
                b.z_order != element.z_order || b.texture != texture
            });

            if new_batch {
                batches.push(UiBatch {
                    canvas_id: self.id.clone(),
                    z_order: element.z_order,
                    clip_rect: clip,
                    texture,
                    vertices: Vec::new(),
                    indices: Vec::new(),
                    material: AssetId::new(DEFAULT_UI_MATERIAL),
                });
            }

            let batch = batches.last_mut().expect("batch just created");

            match &element.kind {
                UiElementKind::Quad { color, .. } => {
                    batch::add_quad(batch, &element.rect, &[0.0, 0.0], &[1.0, 1.0], &batch::color_to_array(*color));
                }
                UiElementKind::Border { color, thickness } => {
                    batch::add_border(batch, &element.rect, *thickness, batch::color_to_array(*color));
                }
                UiElementKind::Text { color, .. } => {
                    // Placeholder: render as a semi-transparent quad
                    let mut c = batch::color_to_array(*color);
                    c[3] = c[3] / 2;
                    batch::add_quad(batch, &element.rect, &[0.0, 0.0], &[1.0, 1.0], &c);
                }
                UiElementKind::Image { tint, .. } => {
                    batch::add_quad(batch, &element.rect, &[0.0, 0.0], &[1.0, 1.0], &batch::color_to_array(*tint));
                }
                UiElementKind::NineSlice { border, tint, .. } => {
                    batch::add_nine_slice(batch, &element.rect, border, batch::color_to_array(*tint));
                }
            }
        }

        batches
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use crate::*;
    use engine_serialize::AssetId;

    fn test_canvas() -> Canvas {
        Canvas::new("test-canvas", 800, 600)
    }

    fn quad_element(x: f32, y: f32, w: f32, h: f32, z: i32, color: Color) -> UiElement {
        UiElement {
            id: ElementId(0), // will be overwritten
            rect: UiRect::new(x, y, w, h),
            z_order: z,
            visible: true,
            kind: UiElementKind::Quad {
                color,
                corner_radius: 0.0,
            },
        }
    }

    #[test]
    fn canvas_new_and_accessors() {
        let canvas = test_canvas();
        assert_eq!(canvas.id(), "test-canvas");
        assert_eq!(canvas.width(), 800);
        assert_eq!(canvas.height(), 600);
    }

    #[test]
    fn canvas_resize() {
        let mut canvas = test_canvas();
        canvas.resize(1024, 768);
        assert_eq!(canvas.width(), 1024);
        assert_eq!(canvas.height(), 768);
    }

    #[test]
    fn add_and_remove_element() {
        let mut canvas = test_canvas();
        let id = canvas.add_element(quad_element(10.0, 10.0, 100.0, 50.0, 0, Color::WHITE));
        assert!(canvas.get_element(id).is_some());
        assert!(canvas.remove_element(id));
        assert!(canvas.get_element(id).is_none());
    }

    #[test]
    fn add_element_overwrites_id() {
        let mut canvas = test_canvas();
        let elem = quad_element(0.0, 0.0, 50.0, 50.0, 0, Color::WHITE);
        let id = canvas.add_element(elem);
        let stored = canvas.get_element(id).unwrap();
        assert_eq!(stored.id, id);
    }

    #[test]
    fn get_element_mut_allows_mutation() {
        let mut canvas = test_canvas();
        let id = canvas.add_element(quad_element(0.0, 0.0, 50.0, 50.0, 0, Color::WHITE));
        {
            let el = canvas.get_element_mut(id).unwrap();
            el.visible = false;
        }
        assert!(!canvas.get_element(id).unwrap().visible);
    }

    #[test]
    fn clear_removes_all_elements() {
        let mut canvas = test_canvas();
        canvas.add_element(quad_element(0.0, 0.0, 10.0, 10.0, 0, Color::WHITE));
        canvas.add_element(quad_element(0.0, 0.0, 10.0, 10.0, 1, Color::WHITE));
        canvas.clear();
        assert!(canvas.build_batches().is_empty());
    }

    #[test]
    fn build_batches_empty_canvas() {
        let canvas = test_canvas();
        assert!(canvas.build_batches().is_empty());
    }

    #[test]
    fn build_batches_hides_invisible() {
        let mut canvas = test_canvas();
        let mut el = quad_element(0.0, 0.0, 10.0, 10.0, 0, Color::WHITE);
        el.visible = false;
        canvas.add_element(el);
        assert!(canvas.build_batches().is_empty());
    }

    #[test]
    fn build_batches_single_quad() {
        let mut canvas = test_canvas();
        canvas.add_element(quad_element(0.0, 0.0, 100.0, 50.0, 0, Color::WHITE));
        let batches = canvas.build_batches();
        assert_eq!(batches.len(), 1);
        assert_eq!(batches[0].z_order, 0);
        assert_eq!(batches[0].vertices.len(), 4);
        assert_eq!(batches[0].indices.len(), 6);
        assert!(batches[0].texture.is_none());
    }

    #[test]
    fn build_batches_z_order_splits() {
        let mut canvas = test_canvas();
        canvas.add_element(quad_element(0.0, 0.0, 10.0, 10.0, 0, Color::WHITE));
        canvas.add_element(quad_element(0.0, 0.0, 10.0, 10.0, 1, Color::WHITE));
        let batches = canvas.build_batches();
        assert_eq!(batches.len(), 2);
        assert_eq!(batches[0].z_order, 0);
        assert_eq!(batches[1].z_order, 1);
    }

    #[test]
    fn build_batches_merges_same_z_and_texture() {
        let mut canvas = test_canvas();
        canvas.add_element(quad_element(0.0, 0.0, 10.0, 10.0, 0, Color::WHITE));
        canvas.add_element(quad_element(10.0, 0.0, 10.0, 10.0, 0, Color::WHITE));
        let batches = canvas.build_batches();
        assert_eq!(batches.len(), 1);
        assert_eq!(batches[0].vertices.len(), 8);
        assert_eq!(batches[0].indices.len(), 12);
    }

    #[test]
    fn build_batches_vertex_positions() {
        let mut canvas = test_canvas();
        canvas.add_element(quad_element(10.0, 20.0, 30.0, 40.0, 0, Color::WHITE));
        let batches = canvas.build_batches();
        let v = &batches[0].vertices;
        assert_eq!(v[0].position, [10.0, 20.0]); // top-left
        assert_eq!(v[1].position, [40.0, 20.0]); // top-right
        assert_eq!(v[2].position, [40.0, 60.0]); // bottom-right
        assert_eq!(v[3].position, [10.0, 60.0]); // bottom-left
    }

    #[test]
    fn build_batches_quad_uvs() {
        let mut canvas = test_canvas();
        canvas.add_element(quad_element(0.0, 0.0, 10.0, 10.0, 0, Color::WHITE));
        let batches = canvas.build_batches();
        let v = &batches[0].vertices;
        assert_eq!(v[0].uv, [0.0, 0.0]);
        assert_eq!(v[1].uv, [1.0, 0.0]);
        assert_eq!(v[2].uv, [1.0, 1.0]);
        assert_eq!(v[3].uv, [0.0, 1.0]);
    }

    #[test]
    fn build_batches_quad_color() {
        let color = Color::new(64, 128, 192, 255);
        let mut canvas = test_canvas();
        canvas.add_element(quad_element(0.0, 0.0, 10.0, 10.0, 0, color));
        let batches = canvas.build_batches();
        for v in &batches[0].vertices {
            assert_eq!(v.color, [64, 128, 192, 255]);
        }
    }

    #[test]
    fn build_batches_border_produces_four_quads() {
        let mut canvas = test_canvas();
        canvas.add_element(UiElement {
            id: ElementId(0),
            rect: UiRect::new(0.0, 0.0, 100.0, 100.0),
            z_order: 0,
            visible: true,
            kind: UiElementKind::Border {
                color: Color::WHITE,
                thickness: 2.0,
            },
        });
        let batches = canvas.build_batches();
        assert_eq!(batches.len(), 1);
        // 4 quads × 4 vertices = 16, × 6 indices = 24
        assert_eq!(batches[0].vertices.len(), 16);
        assert_eq!(batches[0].indices.len(), 24);
    }

    #[test]
    fn build_batches_text_is_semitransparent() {
        let mut canvas = test_canvas();
        canvas.add_element(UiElement {
            id: ElementId(0),
            rect: UiRect::new(0.0, 0.0, 50.0, 20.0),
            z_order: 0,
            visible: true,
            kind: UiElementKind::Text {
                content: "Hello".to_string(),
                font_size: 16.0,
                color: Color::new(255, 0, 0, 255),
            },
        });
        let batches = canvas.build_batches();
        assert_eq!(batches[0].vertices.len(), 4);
        // Alpha should be halved
        for v in &batches[0].vertices {
            assert_eq!(v.color[0], 255);
            assert_eq!(v.color[1], 0);
            assert_eq!(v.color[2], 0);
            assert_eq!(v.color[3], 127); // 255/2 = 127
        }
    }

    #[test]
    fn build_batches_image_has_texture() {
        let mut canvas = test_canvas();
        canvas.add_element(UiElement {
            id: ElementId(0),
            rect: UiRect::new(0.0, 0.0, 64.0, 64.0),
            z_order: 0,
            visible: true,
            kind: UiElementKind::Image {
                texture: AssetId::new("ui/button"),
                tint: Color::WHITE,
            },
        });
        let batches = canvas.build_batches();
        assert_eq!(batches.len(), 1);
        assert_eq!(batches[0].texture, Some(AssetId::new("ui/button")));
    }

    #[test]
    fn build_batches_nine_slice() {
        let mut canvas = test_canvas();
        canvas.add_element(UiElement {
            id: ElementId(0),
            rect: UiRect::new(0.0, 0.0, 200.0, 100.0),
            z_order: 0,
            visible: true,
            kind: UiElementKind::NineSlice {
                texture: AssetId::new("ui/panel"),
                border: UiRect::new(0.125, 0.125, 0.125, 0.125),
                tint: Color::WHITE,
            },
        });
        let batches = canvas.build_batches();
        assert_eq!(batches.len(), 1);
        assert_eq!(batches[0].texture, Some(AssetId::new("ui/panel")));
        // 9 quads × 4 vertices = 36
        assert_eq!(batches[0].vertices.len(), 36);
        assert_eq!(batches[0].indices.len(), 54);
    }

    #[test]
    fn batch_clip_rect_matches_canvas() {
        let mut canvas = Canvas::new("clip-test", 1920, 1080);
        canvas.add_element(quad_element(0.0, 0.0, 10.0, 10.0, 0, Color::WHITE));
        let batches = canvas.build_batches();
        assert_eq!(batches[0].clip_rect.min, [0.0, 0.0]);
        assert_eq!(batches[0].clip_rect.max, [1920.0, 1080.0]);
    }

    #[test]
    fn batch_material_default() {
        let mut canvas = test_canvas();
        canvas.add_element(quad_element(0.0, 0.0, 10.0, 10.0, 0, Color::WHITE));
        let batches = canvas.build_batches();
        assert_eq!(batches[0].material, AssetId::new(DEFAULT_UI_MATERIAL));
    }

    #[test]
    fn batch_canvas_id() {
        let mut canvas = Canvas::new("my-ui", 400, 300);
        canvas.add_element(quad_element(0.0, 0.0, 10.0, 10.0, 0, Color::WHITE));
        let batches = canvas.build_batches();
        assert_eq!(batches[0].canvas_id, "my-ui");
    }
}
