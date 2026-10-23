//! Compact navigation preference compositions and explanatory figures.
use super::{metrics::dimension, text::paint_text};
use crate::theme::Tokens;
use egui::{FontId, Rect, Sense, Stroke, Ui, pos2, vec2};

/// One mouse silhouette beside the actual mapping selector and binding legend.
/// The figure is descriptive content, not an icon asset or an application viewport.
pub(crate) fn settings_mouse_mapping(ui: &mut Ui, bindings: &[(Vec<String>, String)], mapping: impl FnOnce(&mut Ui)) {
    let t = Tokens::get();
    ui.horizontal_top(|ui| {
        ui.spacing_mut().item_spacing.x = t.space_24 - t.space_2;
        let figure_size = vec2(t.form_label_width + t.space_24 + t.space_16, t.space_24 * 6.0 + t.space_16);
        let (figure, _) = ui.allocate_exact_size(figure_size, Sense::hover());
        let width = dimension(ui.available_width());
        let content = ui.allocate_ui_with_layout(vec2(width, 0.0), egui::Layout::top_down(egui::Align::Min), |ui| {
            mapping(ui);
            ui.add_space(t.space_6);
            for (keys, action) in bindings.iter().take(10) {
                settings_gesture(ui, keys, action);
            }
        });
        let height = figure.height().max(content.response.rect.height());
        let figure = Rect::from_min_size(figure.min + vec2(0.0, (height - figure.height()) * 0.5), figure_size);
        let point = |x: f32, y: f32| figure.min + vec2(figure.width() * x / 110.0, figure.height() * y / 160.0);
        let mouse = Rect::from_min_max(point(12.0, 6.0), point(98.0, 154.0));
        let p = ui.painter();
        p.rect_filled(mouse, mouse.width() * 0.5, t.control_fill);
        let left = Rect::from_min_max(mouse.min, pos2(mouse.center().x, point(0.0, 68.0).y));
        p.with_clip_rect(left).rect_filled(mouse, mouse.width() * 0.5, t.selected_bg);
        let right = Rect::from_min_max(pos2(mouse.center().x, mouse.top()), pos2(mouse.right(), point(0.0, 68.0).y));
        p.with_clip_rect(right).rect_filled(mouse, mouse.width() * 0.5, t.warning.gamma_multiply(0.18));
        p.rect_stroke(mouse, mouse.width() * 0.5, Stroke::new(1.5, t.control_border), egui::StrokeKind::Inside);
        p.vline(mouse.center().x, mouse.top()..=point(0.0, 68.0).y, Stroke::new(1.2, t.control_border));
        p.hline(mouse.x_range(), point(0.0, 68.0).y, Stroke::new(1.2, t.control_border));
        p.rect_filled(Rect::from_min_max(point(48.0, 24.0), point(62.0, 54.0)), t.radius_lg, t.form_primary);
    });
}

/// Keyboard and pointer gestures use the same quiet keycap treatment as shortcuts.
fn settings_gesture(ui: &mut Ui, keys: &[String], action: &str) {
    let t = Tokens::get();
    let width = dimension(ui.available_width());
    let (rect, response) = ui.allocate_exact_size(vec2(width, t.control_height + t.space_2), Sense::hover());
    response.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Label, ui.is_enabled(), format!("{}: {action}", keys.join(" + "))));
    ui.painter().hline(rect.x_range(), rect.bottom(), Stroke::new(1.0, t.dialog_rule));
    let key_width = (t.form_label_width * 2.0 + t.space_24 + t.space_6).min(width * 0.57);
    let key_rect = Rect::from_min_size(rect.min, vec2(key_width, rect.height()));
    let p = ui.painter().with_clip_rect(key_rect);
    let mut x = key_rect.left();
    for (index, key) in keys.iter().enumerate() {
        if index > 0 {
            let join = Rect::from_min_size(pos2(x, rect.top()), vec2(t.space_12, rect.height()));
            paint_text(ui, join, "+", FontId::proportional(t.font_small), t.form_dim, true);
            x += t.space_12;
        }
        let galley = p.layout_no_wrap(key.to_owned(), FontId::proportional(t.font_small), t.form_ink);
        let cap = Rect::from_center_size(
            pos2(x + (galley.size().x + t.space_12) / 2.0, rect.center().y),
            vec2(galley.size().x + t.space_12, t.space_24 - t.space_2),
        );
        p.rect(cap, t.radius_sm, t.control_fill, Stroke::new(1.0, t.control_border), egui::StrokeKind::Inside);
        p.galley(pos2(cap.center().x - galley.size().x / 2.0, cap.center().y - galley.size().y / 2.0), galley, t.form_ink);
        x = cap.right() + t.space_4;
    }
    let action_rect = Rect::from_min_max(pos2(key_rect.right() + t.space_8, rect.top()), rect.max);
    paint_text(ui, action_rect, action, FontId::proportional(t.font_label), t.form_ink, false);
}
