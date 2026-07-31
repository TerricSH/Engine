use super::primitive_count;

#[test]
fn primitive_count_tracks_triangle_topologies_only() {
    assert_eq!(primitive_count(glow::TRIANGLES, 8), 2);
    assert_eq!(primitive_count(glow::TRIANGLE_STRIP, 8), 6);
    assert_eq!(primitive_count(glow::TRIANGLE_FAN, 1), 0);
    assert_eq!(primitive_count(glow::LINES, 8), 0);
}
