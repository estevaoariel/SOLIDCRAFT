//! Insert SVG / Insert DXF: once the file is picked, the drawing is placed on the sketch (an
//! offset and a scale) with a live preview, and OK adds it (#2605, #3443).

use serde_json::{Value, json};

use crate::SolveApp;
use crate::dialogs::{Dialog, Kind, form_row};

/// Largest scale the insert commands take.
const MAX_SCALE: f64 = 1e5;

/// The drawing being placed: the insert command, its file, and where and how big it goes.
#[derive(Clone, Debug, PartialEq)]
pub struct Place {
    pub command: String,
    pub path: String,
    pub x: f64,
    pub y: f64,
    pub scale: f64,
}

/// Open the placement for `path`, picked for `command` (`sketch.insert_svg` / `_dxf`).
pub fn open(app: &mut SolveApp, command: &str, path: String) {
    app.dialog = Some(Dialog::new(Kind::InsertDrawing(Place { command: command.to_string(), path, x: 0.0, y: 0.0, scale: 1.0 }), Vec::new()));
}

pub fn title(p: &Place) -> &'static str {
    if p.command.ends_with("dxf") { "INSERT DXF" } else { "INSERT SVG" }
}

pub fn rows(ui: &mut egui::Ui, p: &mut Place) {
    let name = std::path::Path::new(&p.path).file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_else(|| p.path.clone());
    form_row(ui, "File", |ui| {
        ui.label(name).on_hover_text(&p.path);
    });
    for (label, v, handle) in [("X", &mut p.x, "dialog:insert:x"), ("Y", &mut p.y, "dialog:insert:y")] {
        form_row(ui, label, |ui| {
            let r = crate::ui_kit::number_field(ui, v, -1e9..=1e9, 0.5, " mm");
            crate::scenario::publish_handle(handle, r.rect.center());
        });
    }
    form_row(ui, "Scale", |ui| {
        let r = crate::ui_kit::number_field(ui, &mut p.scale, 0.0001..=MAX_SCALE, 0.01, "");
        crate::scenario::publish_handle("dialog:insert:scale", r.rect.center());
    });
}

/// The command OK runs (and the preview shows).
pub fn commands(p: &Place) -> Result<Vec<(String, Value)>, String> {
    if !(p.x.is_finite() && p.y.is_finite()) {
        return Err("the position must be a number".into());
    }
    if !(p.scale.is_finite() && p.scale > 1e-9 && p.scale <= MAX_SCALE) {
        return Err("the scale must be above 0".into());
    }
    Ok(vec![(p.command.clone(), json!({"path": p.path, "at": [p.x, p.y], "scale": p.scale}))])
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::scenario::{Harness, handle_at};

    fn svg_file(name: &str) -> String {
        let dir = std::env::temp_dir().join(format!("solidcraft-insert-place-{}", std::process::id()));
        let _ = std::fs::create_dir_all(&dir);
        let path = dir.join(name);
        std::fs::write(&path, r#"<svg xmlns="http://www.w3.org/2000/svg" width="20mm" height="10mm" viewBox="0 0 20 10"><rect x="0" y="0" width="20" height="10"/></svg>"#).unwrap();
        path.to_string_lossy().to_string()
    }

    fn lines(h: &Harness) -> Vec<(solidcraft_engine::geom::Vec2, solidcraft_engine::geom::Vec2)> {
        let st = h.app.session.world_state();
        let sk = &st.sketch(h.app.session.active_sketch.unwrap()).unwrap().sketch;
        (0..sk.curves.len())
            .filter_map(|i| match sk.shape(i) {
                Some(solidcraft_engine::sketch::Shape::Line { a, b }) => Some((a, b)),
                _ => None,
            })
            .collect()
    }

    /// Insert SVG asks for the file, then places it: X, Y and Scale move and size the drawing
    /// before OK adds it, in one undo step.
    #[test]
    fn a_picked_svg_is_placed_and_scaled_before_it_is_added() {
        let mut h = Harness::new();
        let path = svg_file("plate.svg");
        let picked = path.clone();
        h.app.services.pick_open = Some(Box::new(move |_| Box::pin(std::future::ready(Some(picked.clone())))));
        h.app.run("sketch.create", json!({"plane": "XY"})).unwrap();
        h.settle();
        h.app.start("sketch.insert_svg");
        h.frames(3);
        let Some(Kind::InsertDrawing(p)) = h.app.dialog.as_ref().map(|d| d.kind.clone()) else { panic!("no placement: {:?}", h.app.status) };
        assert_eq!((p.path.as_str(), p.x, p.y, p.scale), (path.as_str(), 0.0, 0.0, 1.0));
        assert!(handle_at("dialog:insert:scale").is_some(), "the scale is drawn");
        assert!(lines(&h).is_empty(), "nothing is added before OK");
        // The drawing is previewed where it would go.
        h.step(&json!({"until": {"count": {"name": "sketch_preview_curves", "value": 4}}})).unwrap();
        if let Some(Kind::InsertDrawing(p)) = h.app.dialog.as_mut().map(|d| &mut d.kind) {
            (p.x, p.y, p.scale) = (5.0, -3.0, 2.0);
        }
        h.step(&json!({"key": "Enter"})).unwrap();
        h.settle();
        assert!(h.app.dialog.is_none(), "{:?}", h.app.dialog.as_ref().and_then(|d| d.error.clone()));
        let pts: Vec<_> = lines(&h).into_iter().flat_map(|(a, b)| [a, b]).collect();
        assert_eq!(pts.len(), 8, "the rectangle's four lines");
        let (lo_x, hi_x) = pts.iter().fold((f64::MAX, f64::MIN), |(l, u), p| (l.min(p.x), u.max(p.x)));
        let (lo_y, hi_y) = pts.iter().fold((f64::MAX, f64::MIN), |(l, u), p| (l.min(p.y), u.max(p.y)));
        assert!((hi_x - lo_x - 40.0).abs() < 1e-6 && (hi_y - lo_y - 20.0).abs() < 1e-6, "twice its size: {lo_x}..{hi_x} × {lo_y}..{hi_y}");
        assert!((lo_x - 5.0).abs() < 1e-6, "moved by X: {lo_x}");
        h.app.run("edit.undo", json!({})).unwrap();
        h.settle();
        assert!(lines(&h).is_empty(), "one undo removes the drawing");
    }

    /// Cancel adds nothing; a bad scale is refused.
    #[test]
    fn cancelling_the_placement_adds_nothing() {
        let mut h = Harness::new();
        h.app.run("sketch.create", json!({"plane": "XY"})).unwrap();
        h.settle();
        open(&mut h.app, "sketch.insert_svg", svg_file("cancel.svg"));
        h.frames(2);
        let bad = Place { command: "sketch.insert_svg".into(), path: "x.svg".into(), x: 0.0, y: 0.0, scale: 0.0 };
        assert!(commands(&bad).is_err());
        h.step(&json!({"key": "Escape"})).unwrap();
        h.settle();
        assert!(h.app.dialog.is_none());
        assert!(lines(&h).is_empty(), "nothing added");
    }
}
