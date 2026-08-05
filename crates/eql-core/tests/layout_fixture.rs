use eql_core::layout::Layout;

#[test]
fn dorskui_4k_layout_has_no_overlaps_or_offscreen_windows() {
    let json = include_str!("../../../fixtures/equi/layout.json");
    let layout: Layout = serde_json::from_str(json).unwrap();
    assert_eq!(layout.0.len(), 13);
    let problems = layout.validate(3840, 2160);
    assert!(problems.is_empty(), "{problems:?}");
}
