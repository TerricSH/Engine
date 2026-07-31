use super::batching::{
    build_canvas_batches, increment_element_id, normalize_element_id, FIRST_ELEMENT_ID,
    LAST_ELEMENT_ID,
};
use super::*;

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
#[derive(Clone, Debug, Serialize, Deserialize)]
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
    pub(super) next_id: u32,
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

    /// Set the next element ID counter (used during deserialization).
    pub fn set_next_id(&mut self, id: u32) {
        self.next_id = self.next_available_id(id);
    }

    /// Add a [`UiElement`], assigning it a new [`ElementId`].
    ///
    /// The element's `id` field is overwritten.  Returns the assigned id.
    pub fn add_element(&mut self, mut element: UiElement) -> ElementId {
        let id = ElementId(self.next_available_id(self.next_id));
        self.next_id = self.next_available_id(increment_element_id(id.0));
        element.id = id;
        debug!(element_id = ?id, "Element added to canvas");
        self.elements.push(element);
        id
    }

    /// Insert an element with an explicit stable ID.
    ///
    /// This is used by deferred script and network command streams where the
    /// producer must receive a usable element handle before the command is
    /// applied on the engine thread. Normal in-process callers should prefer
    /// [`Self::add_element`].
    pub fn insert_element(
        &mut self,
        id: ElementId,
        mut element: UiElement,
    ) -> Result<ElementId, crate::UiError> {
        if !(FIRST_ELEMENT_ID..=LAST_ELEMENT_ID).contains(&id.0) {
            return Err(crate::UiError::InvalidElementId(id));
        }
        if self.elements.iter().any(|existing| existing.id == id) {
            return Err(crate::UiError::DuplicateElementId(id));
        }
        element.id = id;
        self.elements.push(element);
        let next_candidate = if id.0 >= self.next_id {
            increment_element_id(id.0)
        } else {
            self.next_id
        };
        self.next_id = self.next_available_id(next_candidate);
        debug!(element_id = ?id, "Element inserted into canvas");
        Ok(id)
    }

    fn next_available_id(&self, requested: u32) -> u32 {
        let start = normalize_element_id(requested);
        let mut candidate = start;
        loop {
            if self
                .elements
                .iter()
                .all(|element| element.id.0 != candidate)
            {
                return candidate;
            }
            candidate = increment_element_id(candidate);
            assert_ne!(candidate, start, "Canvas exhausted all valid element IDs");
        }
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
                if resolved[i] {
                    continue;
                }
                let parent_rect = match parent_of[i] {
                    None => canvas_rect, // root → canvas
                    Some(pid) => {
                        if let Some(&p_idx) = id_to_idx.get(&pid) {
                            if resolved[p_idx] {
                                rects[p_idx]
                            } else {
                                continue;
                            }
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
        build_canvas_batches(self, self.width, self.height, None)
    }

    /// Build batches for a viewport and include current hover/press visuals.
    ///
    /// Element layout stays in logical Canvas coordinates. Fit-width and
    /// fit-height canvases scale the generated vertices and clipping region
    /// to the supplied viewport while retaining their aspect ratio.
    pub fn build_batches_for_viewport(
        &self,
        viewport_width: f32,
        viewport_height: f32,
        input: Option<&UiInputState>,
    ) -> Vec<UiBatch> {
        build_canvas_batches(self, viewport_width, viewport_height, input)
    }
}
