use engine_renderer::{Rect, UiBatch};
use engine_serialize::AssetId;
use tracing::debug;

use crate::batch;
use crate::layout::{Layout, ScaleMode};
use crate::types::{ElementId, UiElement, UiElementKind, UiRect};
use crate::DEFAULT_UI_MATERIAL;

// ---------------------------------------------------------------------------
// Canvas
// ---------------------------------------------------------------------------

/// A 2D UI canvas that owns a list of anchor-laid-out elements and produces
/// [`engine_renderer::UiBatch`]es for the render pipeline.
///
/// Elements are ordered by [`UiElement::z_order`] at batch-creation time.
/// Elements sharing the same `z_order` *and* texture are merged into a single
/// batch to reduce draw calls.
///
/// Call [`Canvas::layout_all`] after mutating element layouts to recompute
/// the pixel rectangles used by rendering and hit-testing.
pub struct Canvas {
    /// Canvas logical width in pixels.
    pub width: f32,
    /// Canvas logical height in pixels.
    pub height: f32,
    /// Ordered list of UI elements.
    pub elements: Vec<UiElement>,
    /// How the canvas scales when the viewport size changes.
    pub scale_mode: ScaleMode,
    /// Monotonically-increasing ID counter.
    next_id: u32,
}

impl Canvas {
    /// Create a new canvas with the given logical dimensions.
    ///
    /// `scale_mode` defaults to [`ScaleMode::Fixed`].
    pub fn new(width: f32, height: f32) -> Self {
        debug!(width, height, "Canvas created");
        Self {
            width,
            height,
            elements: Vec::new(),
            scale_mode: ScaleMode::Fixed,
            next_id: 1,
        }
    }

    /// Resize the canvas (does not automatically re-layout).
    pub fn resize(&mut self, width: f32, height: f32) {
        debug!(
            old_width = self.width,
            old_height = self.height,
            new_width = width,
            new_height = height,
            "Canvas resized"
        );
        self.width = width;
        self.height = height;
    }

    /// Add a [`UiElement`], assigning it a new [`ElementId`].
    ///
    /// The element's `id` field is overwritten.  Returns the assigned id.
    pub fn add_element(&mut self, mut element: UiElement) -> ElementId {
        let id = ElementId(self.next_id);
        self.next_id += 1;
        element.id = id;
        debug!(element_id = ?id, "Element added to canvas");
        self.elements.push(element);
        id
    }

    /// Remove an element by id.
    ///
    /// Also removes it from any parent's children list.
    /// Returns `true` if the element was found and removed.
    pub fn remove_element(&mut self, id: ElementId) -> bool {
        let pos = self.elements.iter().position(|e| e.id == id);
        if let Some(idx) = pos {
            self.elements.remove(idx);
            // Remove from any parent's children list.
            for el in &mut self.elements {
                el.children.retain(|c| *c != id);
            }
            debug!(element_id = ?id, "Element removed from canvas");
            true
        } else {
            false
        }
    }

    /// Borrow an element by id.
    pub fn get_element(&self, id: ElementId) -> Option<&UiElement> {
        self.elements.iter().find(|e| e.id == id)
    }

    /// Mutably borrow an element by id.
    pub fn get_element_mut(&mut self, id: ElementId) -> Option<&mut UiElement> {
        self.elements.iter_mut().find(|e| e.id == id)
    }

    /// Remove all elements.
    pub fn clear(&mut self) {
        let count = self.elements.len();
        self.elements.clear();
        self.next_id = 1;
        debug!(count, "Canvas cleared");
    }

    /// Resolve all element layouts into pixel rectangles.
    ///
    /// For each element, [`Layout::compute`] is called with the canvas as the
    /// parent rect.  Child elements use their parent's computed rect as the
    /// parent rect.
    ///
    /// Elements are processed in list order so parents are guaranteed to be
    /// resolved before their children.
    pub fn layout_all(&mut self) {
        let canvas_rect = UiRect::new(0.0, 0.0, self.width, self.height);

        // Build a lookup: ElementId -> index in elements slice.
        let mut id_to_idx: std::collections::HashMap<ElementId, usize> =
            std::collections::HashMap::with_capacity(self.elements.len());
        for (i, el) in self.elements.iter().enumerate() {
            id_to_idx.insert(el.id, i);
        }

        // Compute all rects in topological order (parents before children).
        // We iterate multiple times: first resolve roots (no parent), then
        // children whose parent has been resolved, until all are done.
        let n = self.elements.len();
        let mut resolved = vec![false; n];
        let mut rects = vec![UiRect::ZERO; n];

        // Compute parent for each element: which element claims this as child.
        let mut parent_of: Vec<Option<ElementId>> = vec![None; n];
        for (i, el) in self.elements.iter().enumerate() {
            for (j, other) in self.elements.iter().enumerate() {
                if i != j && other.children.contains(&el.id) {
                    parent_of[i] = Some(other.id);
                    break;
                }
            }
        }

        // Resolve iteratively: roots first, then their children, etc.
        let mut changed = true;
        while changed {
            changed = false;
            for i in 0..n {
                if resolved[i] { continue; }
                let parent_rect = match parent_of[i] {
                    None => canvas_rect, // root → canvas
                    Some(pid) => {
                        if let Some(&p_idx) = id_to_idx.get(&pid) {
                            if resolved[p_idx] { rects[p_idx] } else { continue; }
                        } else {
                            canvas_rect // parent missing → canvas
                        }
                    }
                };
                rects[i] = Layout::compute(&parent_rect, &self.elements[i].layout);
                self.elements[i].rect = rects[i];
                resolved[i] = true;
                changed = true;
            }
        }
    }

    /// Build a list of [`UiBatch`]es from the enabled elements on this canvas.
    ///
    /// Elements are sorted by `z_order` (ascending).  Consecutive elements
    /// sharing the same `z_order` *and* texture are merged into one batch.
    /// Returns an empty Vec when there are no enabled elements.
    ///
    /// Call [`Canvas::layout_all`] before this to ensure pixel rects are current.
    pub fn build_batches(&self) -> Vec<UiBatch> {
        let mut visible: Vec<&UiElement> = self.elements.iter().filter(|e| e.enabled).collect();
        if visible.is_empty() {
            return Vec::new();
        }
        visible.sort_by_key(|e| e.z_order);

        let clip = Rect {
            min: [0.0, 0.0],
            max: [self.width, self.height],
        };

        let mut batches: Vec<UiBatch> = Vec::new();

        for element in &visible {
            let texture = batch::element_kind_texture(&element.kind);

            // Start a new batch when z_order or texture changes.
            let new_batch = batches.last().map_or(true, |b: &UiBatch| {
                b.z_order != element.z_order || b.texture != texture
            });

            if new_batch {
                batches.push(UiBatch {
                    canvas_id: String::new(), // no persistent id on new Canvas
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
                UiElementKind::Panel { color } => {
                    batch::add_quad(
                        batch,
                        &element.rect,
                        &[0.0, 0.0],
                        &[1.0, 1.0],
                        &batch::color_to_array(*color),
                    );
                }
                UiElementKind::Image { color, .. } => {
                    batch::add_quad(
                        batch,
                        &element.rect,
                        &[0.0, 0.0],
                        &[1.0, 1.0],
                        &batch::color_to_array(*color),
                    );
                }
                UiElementKind::Text { color, .. } => {
                    // Placeholder: render as a semi-transparent quad
                    let mut c = batch::color_to_array(*color);
                    c[3] /= 2;
                    batch::add_quad(batch, &element.rect, &[0.0, 0.0], &[1.0, 1.0], &c);
                }
                UiElementKind::Button {
                    normal_color, ..
                } => {
                    batch::add_quad(
                        batch,
                        &element.rect,
                        &[0.0, 0.0],
                        &[1.0, 1.0],
                        &batch::color_to_array(*normal_color),
                    );
                }
            }
        }

        batches
    }
}

// ---------------------------------------------------------------------------
// ECS Component
// ---------------------------------------------------------------------------

impl engine_scene::Component for Canvas {
    const TYPE_ID: &'static str = "engine.canvas";
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use crate::*;
    use engine_serialize::AssetId;
    use glam::Vec2;

    fn test_canvas() -> Canvas {
        Canvas::new(800.0, 600.0)
    }

    fn panel_element(layout: Layout, z: i32, color: Color) -> UiElement {
        UiElement::new(UiElementKind::Panel { color }, layout).with_z_order(z)
    }

    fn image_element(layout: Layout, z: i32, texture_id: &str, color: Color) -> UiElement {
        UiElement::new(
            UiElementKind::Image {
                texture_id: texture_id.to_string(),
                color,
            },
            layout,
        )
        .with_z_order(z)
    }

    #[test]
    fn canvas_new_and_accessors() {
        let canvas = Canvas::new(800.0, 600.0);
        assert_eq!(canvas.width, 800.0);
        assert_eq!(canvas.height, 600.0);
        assert_eq!(canvas.scale_mode, ScaleMode::Fixed);
    }

    #[test]
    fn canvas_resize() {
        let mut canvas = test_canvas();
        canvas.resize(1024.0, 768.0);
        assert_eq!(canvas.width, 1024.0);
        assert_eq!(canvas.height, 768.0);
    }

    #[test]
    fn add_and_remove_element() {
        let mut canvas = test_canvas();
        let id = canvas.add_element(panel_element(Layout::FILL, 0, Color::WHITE));
        assert!(canvas.get_element(id).is_some());
        assert!(canvas.remove_element(id));
        assert!(canvas.get_element(id).is_none());
    }

    #[test]
    fn add_element_overwrites_id() {
        let mut canvas = test_canvas();
        let mut el = panel_element(Layout::FILL, 0, Color::WHITE);
        el.id = ElementId(999); // should be overwritten
        let id = canvas.add_element(el);
        let stored = canvas.get_element(id).unwrap();
        assert_eq!(stored.id, id);
        assert_ne!(stored.id, ElementId(999));
    }

    #[test]
    fn get_element_mut_allows_mutation() {
        let mut canvas = test_canvas();
        let id = canvas.add_element(panel_element(Layout::FILL, 0, Color::WHITE));
        {
            let el = canvas.get_element_mut(id).unwrap();
            el.enabled = false;
        }
        assert!(!canvas.get_element(id).unwrap().enabled);
    }

    #[test]
    fn clear_removes_all_elements() {
        let mut canvas = test_canvas();
        canvas.add_element(panel_element(Layout::FILL, 0, Color::WHITE));
        canvas.add_element(panel_element(Layout::FILL, 1, Color::WHITE));
        canvas.clear();
        assert!(canvas.build_batches().is_empty());
    }

    #[test]
    fn build_batches_empty_canvas() {
        let canvas = test_canvas();
        assert!(canvas.build_batches().is_empty());
    }

    #[test]
    fn build_batches_skips_disabled() {
        let mut canvas = test_canvas();
        canvas.add_element(
            panel_element(Layout::FILL, 0, Color::WHITE).with_enabled(false),
        );
        assert!(canvas.build_batches().is_empty());
    }

    #[test]
    fn build_batches_single_panel() {
        let mut canvas = test_canvas();
        let layout = Layout::new(Vec2::ZERO, Vec2::ZERO, Vec2::new(0.0, 0.0), Vec2::new(100.0, 50.0));
        canvas.add_element(panel_element(layout, 0, Color::WHITE));
        canvas.layout_all();
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
        let l1 = Layout::new(Vec2::ZERO, Vec2::ZERO, Vec2::ZERO, Vec2::new(10.0, 10.0));
        let l2 = Layout::new(Vec2::ZERO, Vec2::ZERO, Vec2::new(0.0, 0.0), Vec2::new(10.0, 10.0));
        canvas.add_element(panel_element(l1, 0, Color::WHITE));
        canvas.add_element(panel_element(l2, 1, Color::WHITE));
        canvas.layout_all();
        let batches = canvas.build_batches();
        assert_eq!(batches.len(), 2);
        assert_eq!(batches[0].z_order, 0);
        assert_eq!(batches[1].z_order, 1);
    }

    #[test]
    fn build_batches_merges_same_z_and_texture() {
        let mut canvas = test_canvas();
        let l1 = Layout::new(Vec2::ZERO, Vec2::ZERO, Vec2::ZERO, Vec2::new(10.0, 10.0));
        let l2 = Layout::new(Vec2::ZERO, Vec2::ZERO, Vec2::new(10.0, 0.0), Vec2::new(20.0, 10.0));
        canvas.add_element(panel_element(l1, 0, Color::WHITE));
        canvas.add_element(panel_element(l2, 0, Color::WHITE));
        canvas.layout_all();
        let batches = canvas.build_batches();
        assert_eq!(batches.len(), 1);
        assert_eq!(batches[0].vertices.len(), 8);
        assert_eq!(batches[0].indices.len(), 12);
    }

    #[test]
    fn build_batches_vertex_positions() {
        let mut canvas = test_canvas();
        let layout = Layout::new(
            Vec2::ZERO,
            Vec2::ZERO,
            Vec2::new(10.0, 20.0),
            Vec2::new(40.0, 60.0),
        );
        canvas.add_element(panel_element(layout, 0, Color::WHITE));
        canvas.layout_all();
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
        let layout = Layout::new(Vec2::ZERO, Vec2::ZERO, Vec2::ZERO, Vec2::new(10.0, 10.0));
        canvas.add_element(panel_element(layout, 0, Color::WHITE));
        canvas.layout_all();
        let batches = canvas.build_batches();
        let v = &batches[0].vertices;
        assert_eq!(v[0].uv, [0.0, 0.0]);
        assert_eq!(v[1].uv, [1.0, 0.0]);
        assert_eq!(v[2].uv, [1.0, 1.0]);
        assert_eq!(v[3].uv, [0.0, 1.0]);
    }

    #[test]
    fn build_batches_panel_color() {
        let color = Color::new(64, 128, 192, 255);
        let mut canvas = test_canvas();
        let layout = Layout::new(Vec2::ZERO, Vec2::ZERO, Vec2::ZERO, Vec2::new(10.0, 10.0));
        canvas.add_element(panel_element(layout, 0, color));
        canvas.layout_all();
        let batches = canvas.build_batches();
        for v in &batches[0].vertices {
            assert_eq!(v.color, [64, 128, 192, 255]);
        }
    }

    #[test]
    fn build_batches_text_is_semitransparent() {
        let mut canvas = test_canvas();
        let layout = Layout::new(Vec2::ZERO, Vec2::ZERO, Vec2::ZERO, Vec2::new(50.0, 20.0));
        canvas.add_element(
            UiElement::new(
                UiElementKind::Text {
                    content: "Hello".into(),
                    font_size: 16.0,
                    color: Color::new(255, 0, 0, 255),
                },
                layout,
            )
            .with_z_order(0),
        );
        canvas.layout_all();
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
        let layout = Layout::new(Vec2::ZERO, Vec2::ZERO, Vec2::ZERO, Vec2::new(64.0, 64.0));
        canvas.add_element(image_element(layout, 0, "ui/button", Color::WHITE));
        canvas.layout_all();
        let batches = canvas.build_batches();
        assert_eq!(batches.len(), 1);
        assert_eq!(batches[0].texture, Some(AssetId::new("ui/button")));
    }

    #[test]
    fn batch_clip_rect_matches_canvas() {
        let mut canvas = Canvas::new(1920.0, 1080.0);
        let layout = Layout::new(Vec2::ZERO, Vec2::ZERO, Vec2::ZERO, Vec2::new(10.0, 10.0));
        canvas.add_element(panel_element(layout, 0, Color::WHITE));
        canvas.layout_all();
        let batches = canvas.build_batches();
        assert_eq!(batches[0].clip_rect.min, [0.0, 0.0]);
        assert_eq!(batches[0].clip_rect.max, [1920.0, 1080.0]);
    }

    #[test]
    fn batch_material_default() {
        let mut canvas = test_canvas();
        let layout = Layout::new(Vec2::ZERO, Vec2::ZERO, Vec2::ZERO, Vec2::new(10.0, 10.0));
        canvas.add_element(panel_element(layout, 0, Color::WHITE));
        canvas.layout_all();
        let batches = canvas.build_batches();
        assert_eq!(batches[0].material, AssetId::new(DEFAULT_UI_MATERIAL));
    }

    #[test]
    fn layout_all_computes_panel_rect() {
        let mut canvas = test_canvas();
        let layout = Layout::new(
            Vec2::new(0.25, 0.25),
            Vec2::new(0.75, 0.75),
            Vec2::ZERO,
            Vec2::ZERO,
        );
        let id = canvas.add_element(panel_element(layout, 0, Color::WHITE));
        canvas.layout_all();
        let el = canvas.get_element(id).unwrap();
        // 25% of 800 = 200, 75% of 800 = 600 → width = 400
        // 25% of 600 = 150, 75% of 600 = 450 → height = 300
        assert_eq!(el.rect, UiRect::new(200.0, 150.0, 400.0, 300.0));
    }

    #[test]
    fn layout_all_child_relative_to_parent() {
        let mut canvas = Canvas::new(800.0, 600.0);

        // Parent: left half of canvas
        let parent_layout = Layout::new(Vec2::ZERO, Vec2::new(0.5, 1.0), Vec2::ZERO, Vec2::ZERO);
        let parent_id = canvas.add_element(panel_element(parent_layout, 0, Color::WHITE));

        // Child: fills its parent (the left half)
        let child_layout = Layout::FILL;
        let child_id = canvas.add_element(
            UiElement::new(UiElementKind::Panel { color: Color::WHITE }, child_layout)
                .with_z_order(0)
                .with_children(vec![]),
        );

        // Register parent-child relationship
        canvas.get_element_mut(parent_id).unwrap().children.push(child_id);

        canvas.layout_all();

        let parent = canvas.get_element(parent_id).unwrap();
        assert_eq!(parent.rect, UiRect::new(0.0, 0.0, 400.0, 600.0));

        let child = canvas.get_element(child_id).unwrap();
        // Child should compute relative to parent: fills parent = (0,0,400,600)
        assert_eq!(child.rect, UiRect::new(0.0, 0.0, 400.0, 600.0));
    }

    #[test]
    fn scale_mode_default() {
        let canvas = test_canvas();
        assert_eq!(canvas.scale_mode, ScaleMode::Fixed);
    }
}
