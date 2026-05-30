use glam::{Mat4, Vec3};

/// A single debug line.
#[derive(Clone, Debug)]
pub struct DebugLine {
    pub start: Vec3,
    pub end: Vec3,
    pub color: [f32; 4],
    pub duration: f32, // seconds (0 = single frame)
}

/// A single debug shape.
#[derive(Clone, Debug)]
pub enum DebugShape {
    Box {
        center: Vec3,
        half_extents: Vec3,
        color: [f32; 4],
    },
    Sphere {
        center: Vec3,
        radius: f32,
        color: [f32; 4],
    },
    Circle {
        center: Vec3,
        normal: Vec3,
        radius: f32,
        color: [f32; 4],
    },
    Arrow {
        from: Vec3,
        to: Vec3,
        color: [f32; 4],
    },
}

/// A debug label in 3D space.
#[derive(Clone, Debug)]
pub struct DebugLabel {
    pub position: Vec3,
    pub text: String,
    pub color: [f32; 4],
    pub size: f32,
}

/// Accumulated debug draw state for a single frame.
#[derive(Clone, Debug, Default)]
pub struct DebugDrawBuffer {
    pub lines: Vec<DebugLine>,
    pub shapes: Vec<DebugShape>,
    pub labels: Vec<DebugLabel>,
}

impl DebugDrawBuffer {
    /// Create a new empty buffer.
    pub fn new() -> Self {
        Self::default()
    }

    /// Clear all debug primitives from the buffer.
    pub fn clear(&mut self) {
        self.lines.clear();
        self.shapes.clear();
        self.labels.clear();
    }

    /// Returns `true` if the buffer contains no primitives.
    pub fn is_empty(&self) -> bool {
        self.lines.is_empty() && self.shapes.is_empty() && self.labels.is_empty()
    }

    /// Merge another buffer's contents into this one.
    pub fn merge(&mut self, other: DebugDrawBuffer) {
        self.lines.extend(other.lines);
        self.shapes.extend(other.shapes);
        self.labels.extend(other.labels);
    }

    // ── Convenience methods ──────────────────────────────────────────────

    /// Add a debug line.
    pub fn line(&mut self, start: Vec3, end: Vec3, color: [f32; 4]) {
        self.lines.push(DebugLine {
            start,
            end,
            color,
            duration: 0.0,
        });
    }

    /// Add a wireframe box.
    pub fn box_wireframe(&mut self, center: Vec3, half_extents: Vec3, color: [f32; 4]) {
        self.shapes.push(DebugShape::Box {
            center,
            half_extents,
            color,
        });
    }

    /// Add a wireframe sphere.
    pub fn sphere_wireframe(&mut self, center: Vec3, radius: f32, color: [f32; 4]) {
        self.shapes.push(DebugShape::Sphere {
            center,
            radius,
            color,
        });
    }

    /// Add an arrow from `from` to `to`.
    pub fn arrow(&mut self, from: Vec3, to: Vec3, color: [f32; 4]) {
        self.shapes.push(DebugShape::Arrow { from, to, color });
    }

    /// Add a text label in 3D space.
    pub fn label(&mut self, position: Vec3, text: impl Into<String>, color: [f32; 4]) {
        self.labels.push(DebugLabel {
            position,
            text: text.into(),
            color,
            size: 12.0,
        });
    }
}

/// Debug draw provider trait — subsystems implement this to contribute debug visuals.
pub trait DebugDrawProvider: Send {
    /// Human-readable name of this provider (for diagnostics).
    fn name(&self) -> &str;

    /// Populate the buffer with debug draw primitives for the current frame.
    ///
    /// `view_matrix` and `projection_matrix` are provided so that providers
    /// can cull or adjust debug visuals based on the camera state.
    fn populate(&self, buffer: &mut DebugDrawBuffer, view_matrix: &Mat4, projection_matrix: &Mat4);
}

/// Registry for debug draw providers.
pub struct DebugDrawRegistry {
    providers: Vec<Box<dyn DebugDrawProvider>>,
}

impl DebugDrawRegistry {
    /// Create a new empty registry.
    pub fn new() -> Self {
        Self {
            providers: Vec::new(),
        }
    }

    /// Register a debug draw provider.
    pub fn register(&mut self, provider: Box<dyn DebugDrawProvider>) {
        self.providers.push(provider);
    }

    /// Populate the buffer from all registered providers.
    pub fn populate_all(&self, buffer: &mut DebugDrawBuffer, view: &Mat4, proj: &Mat4) {
        for provider in &self.providers {
            provider.populate(buffer, view, proj);
        }
    }

    /// Returns the number of registered providers.
    pub fn provider_count(&self) -> usize {
        self.providers.len()
    }
}

impl Default for DebugDrawRegistry {
    fn default() -> Self {
        Self::new()
    }
}

// ══════════════════════════════════════════════════════════════════════════
// Tests
// ══════════════════════════════════════════════════════════════════════════

#[cfg(test)]
mod tests {
    use super::*;

    // ── DebugDrawBuffer tests ────────────────────────────────────────────

    #[test]
    fn new_buffer_is_empty() {
        let buf = DebugDrawBuffer::new();
        assert!(buf.is_empty());
        assert!(buf.lines.is_empty());
        assert!(buf.shapes.is_empty());
        assert!(buf.labels.is_empty());
    }

    #[test]
    fn clear_removes_all_primitives() {
        let mut buf = DebugDrawBuffer::new();
        buf.line(Vec3::ZERO, Vec3::ONE, [1.0, 0.0, 0.0, 1.0]);
        buf.box_wireframe(Vec3::ZERO, Vec3::splat(1.0), [0.0, 1.0, 0.0, 1.0]);
        buf.label(Vec3::ZERO, "test", [1.0, 1.0, 1.0, 1.0]);
        assert!(!buf.is_empty());
        buf.clear();
        assert!(buf.is_empty());
    }

    #[test]
    fn line_convenience() {
        let mut buf = DebugDrawBuffer::new();
        buf.line(
            Vec3::new(0.0, 0.0, 0.0),
            Vec3::new(1.0, 1.0, 1.0),
            [1.0, 0.0, 0.0, 1.0],
        );
        assert_eq!(buf.lines.len(), 1);
        assert_eq!(buf.lines[0].start, Vec3::new(0.0, 0.0, 0.0));
        assert_eq!(buf.lines[0].end, Vec3::new(1.0, 1.0, 1.0));
    }

    #[test]
    fn box_wireframe_convenience() {
        let mut buf = DebugDrawBuffer::new();
        buf.box_wireframe(Vec3::ZERO, Vec3::splat(0.5), [0.0, 1.0, 0.0, 1.0]);
        assert_eq!(buf.shapes.len(), 1);
        assert!(matches!(buf.shapes[0], DebugShape::Box { .. }));
    }

    #[test]
    fn sphere_wireframe_convenience() {
        let mut buf = DebugDrawBuffer::new();
        buf.sphere_wireframe(Vec3::ZERO, 1.0, [0.0, 0.0, 1.0, 1.0]);
        assert_eq!(buf.shapes.len(), 1);
        assert!(matches!(buf.shapes[0], DebugShape::Sphere { .. }));
    }

    #[test]
    fn arrow_convenience() {
        let mut buf = DebugDrawBuffer::new();
        buf.arrow(Vec3::ZERO, Vec3::X, [1.0, 1.0, 0.0, 1.0]);
        assert_eq!(buf.shapes.len(), 1);
        assert!(matches!(buf.shapes[0], DebugShape::Arrow { .. }));
    }

    #[test]
    fn label_convenience() {
        let mut buf = DebugDrawBuffer::new();
        buf.label(Vec3::new(0.0, 1.0, 0.0), "hello", [1.0, 1.0, 1.0, 1.0]);
        assert_eq!(buf.labels.len(), 1);
        assert_eq!(buf.labels[0].text, "hello");
    }

    #[test]
    fn label_accepts_string_types() {
        let mut buf = DebugDrawBuffer::new();
        buf.label(Vec3::ZERO, "&str", [1.0; 4]);
        buf.label(Vec3::ZERO, String::from("String"), [1.0; 4]);
        assert_eq!(buf.labels.len(), 2);
    }

    #[test]
    fn merge_combines_buffers() {
        let mut buf_a = DebugDrawBuffer::new();
        buf_a.line(Vec3::ZERO, Vec3::X, [1.0; 4]);

        let mut buf_b = DebugDrawBuffer::new();
        buf_b.arrow(Vec3::ZERO, Vec3::Y, [0.0; 4]);
        buf_b.label(Vec3::ZERO, "b", [1.0; 4]);

        buf_a.merge(buf_b);
        assert_eq!(buf_a.lines.len(), 1);
        assert_eq!(buf_a.shapes.len(), 1);
        assert_eq!(buf_a.labels.len(), 1);
    }

    #[test]
    fn merge_into_empty() {
        let mut buf_a = DebugDrawBuffer::new();
        let mut buf_b = DebugDrawBuffer::new();
        buf_b.line(Vec3::ZERO, Vec3::X, [1.0; 4]);
        buf_a.merge(buf_b);
        assert_eq!(buf_a.lines.len(), 1);
    }

    // ── DebugDrawRegistry tests ──────────────────────────────────────────

    /// A dummy provider that adds a single line.
    struct DummyLineProvider;

    impl DebugDrawProvider for DummyLineProvider {
        fn name(&self) -> &str {
            "DummyLineProvider"
        }

        fn populate(&self, buffer: &mut DebugDrawBuffer, _view: &Mat4, _proj: &Mat4) {
            buffer.line(Vec3::ZERO, Vec3::ONE, [1.0, 0.0, 0.0, 1.0]);
        }
    }

    #[test]
    fn registry_new_is_empty() {
        let reg = DebugDrawRegistry::new();
        assert_eq!(reg.provider_count(), 0);
    }

    #[test]
    fn registry_register_increases_count() {
        let mut reg = DebugDrawRegistry::new();
        reg.register(Box::new(DummyLineProvider));
        assert_eq!(reg.provider_count(), 1);
    }

    #[test]
    fn registry_populate_all_calls_providers() {
        let mut reg = DebugDrawRegistry::new();
        reg.register(Box::new(DummyLineProvider));
        reg.register(Box::new(DummyLineProvider));

        let mut buf = DebugDrawBuffer::new();
        let view = Mat4::IDENTITY;
        let proj = Mat4::IDENTITY;
        reg.populate_all(&mut buf, &view, &proj);

        assert_eq!(buf.lines.len(), 2);
    }

    #[test]
    fn registry_populate_with_no_providers() {
        let reg = DebugDrawRegistry::new();
        let mut buf = DebugDrawBuffer::new();
        reg.populate_all(&mut buf, &Mat4::IDENTITY, &Mat4::IDENTITY);
        assert!(buf.is_empty());
    }

    /// A dummy provider that can be used to verify name().
    struct NamedProvider(&'static str);

    impl DebugDrawProvider for NamedProvider {
        fn name(&self) -> &str {
            self.0
        }

        fn populate(&self, _buffer: &mut DebugDrawBuffer, _view: &Mat4, _proj: &Mat4) {}
    }

    #[test]
    fn provider_name_is_reported() {
        let provider = NamedProvider("TestProv");
        assert_eq!(provider.name(), "TestProv");
    }
}
