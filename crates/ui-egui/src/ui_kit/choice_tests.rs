use crate::ui_kit::test_support::*;
use crate::ui_kit::*;

use crate::scenario;
use egui::Ui;

#[test]
fn segmented_click_changes_selection_and_keeps_all_choices_visible() {
    let ctx = crate::fonts::test_context();
    let mut selected = 0;
    let mut draw = |ui: &mut Ui| {
        row(ui, "Continuity", |ui| segmented(ui, &mut selected, &["G1", "G2", "G3"], Default::default()));
    };
    frame(&ctx, vec![], &mut draw);
    let pos = scenario::handle_at("dialog:Continuity:G2").unwrap();
    frame(&ctx, pointer(pos, true), &mut draw);
    frame(&ctx, pointer(pos, false), &mut draw);
    assert_eq!(selected, 1);
    let control = scenario::rect_of("dialog:control:Continuity").unwrap();
    for option in ["G1", "G2", "G3"] {
        assert!(control.contains(scenario::handle_at(&format!("dialog:Continuity:{option}")).unwrap()));
    }
}

#[test]
fn optional_field_keeps_empty_default_and_options_use_two_columns() {
    let ctx = crate::fonts::test_context();
    let mut value = String::new();
    let (mut flip, mut copy) = (false, true);
    frame(&ctx, vec![], |ui| {
        row(ui, "Depth", |ui| hint_field(ui, &mut value, "to the next face"));
        options(ui, "options", &mut [("Flip", &mut flip), ("Create Copy", &mut copy)]);
    });
    assert!(value.is_empty(), "displaying a hint must not introduce a parameter");
    let first = scenario::handle_at("dialog:Flip").unwrap();
    let second = scenario::handle_at("dialog:Create Copy").unwrap();
    assert_eq!(first.y, second.y);
    assert!(second.x > first.x);
    let mut draw = |ui: &mut Ui| options(ui, "options", &mut [("Flip", &mut flip), ("Create Copy", &mut copy)]);
    frame(&ctx, vec![], &mut draw);
    let first = scenario::handle_at("dialog:Flip").unwrap();
    frame(&ctx, pointer(first, true), &mut draw);
    frame(&ctx, pointer(first, false), &mut draw);
    assert!(flip && copy);
}

#[test]
fn beveled_segments_share_fill_corners_at_internal_joins() {
    use crate::theme::Tokens;
    use egui::{Color32, Shape};
    let mut t = Tokens::light();
    t.control_radius = 8;
    t.borders.bevel = Some(crate::theme::Bevel {
        width: 0.5,
        highlight: Color32::WHITE,
        light: Color32::LIGHT_GRAY,
        shadow: Color32::GRAY,
        dark_shadow: Color32::BLACK,
    });
    let _restore = crate::theme::scoped_tokens(t);
    for selected in 0..3 {
        let ctx = crate::fonts::test_context();
        let mut selected = selected;
        let output = crate::test_support::frame(&ctx, egui::RawInput::default(), |ui| {
            ui.set_width(300.0);
            segmented(ui, &mut selected, &["Left", "Middle", "Right"], Default::default());
        });
        let fill = output
            .shapes
            .iter()
            .find_map(|s| match &s.shape {
                Shape::Rect(r) if r.fill == t.selected_bg => Some(r),
                _ => None,
            })
            .expect("selected segment fill");
        for edge in [fill.rect.left(), fill.rect.right()] {
            let internal = if edge == fill.rect.left() { selected > 0 } else { selected < 2 };
            if internal {
                // A rounded bevel used to curve away while selection filled the square
                // internal corner. Its top band must now reach the segment join.
                assert!(output.shapes.iter().any(|s| match &s.shape {
                    Shape::LineSegment { points, .. } =>
                        points.iter().any(|p| (p.y - fill.rect.top() - 0.25).abs() < 0.01 && (p.x - edge).abs() <= 0.26),
                    _ => false,
                }));
            }
        }
    }
}
