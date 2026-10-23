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

/// Two segmented choices in one panel, outside labelled rows (the Airflow results' map and
/// section): each takes its own clicks. Their segments used to share ids by position, so egui
/// gave both rows' clicks to whichever came last and the first could not be changed.
#[test]
fn two_segmented_choices_in_one_panel_each_take_their_clicks() {
    let ctx = crate::fonts::test_context();
    let (map, section) = (std::cell::Cell::new(0), std::cell::Cell::new(0));
    let draw = |ui: &mut Ui| {
        let compact = Segments { compact: true, ..Default::default() };
        let (mut m, mut s) = (map.get(), section.get());
        segmented(ui, &mut m, &["Speed", "Pressure"], compact);
        segmented(ui, &mut s, &["Off", "X", "Y", "Z"], Segments { compact: true, ..Default::default() });
        map.set(m);
        section.set(s);
    };
    frame(&ctx, vec![], draw);
    let pressure = scenario::handle_at("dialog::Pressure").unwrap();
    frame(&ctx, pointer(pressure, true), draw);
    frame(&ctx, pointer(pressure, false), draw);
    assert_eq!((map.get(), section.get()), (1, 0), "the first row changes, the second stays");
    let y = scenario::handle_at("dialog::Y").unwrap();
    frame(&ctx, pointer(y, true), draw);
    frame(&ctx, pointer(y, false), draw);
    assert_eq!((map.get(), section.get()), (1, 2));
}
