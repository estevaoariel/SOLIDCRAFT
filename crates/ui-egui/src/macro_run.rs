//! The Play Macro prompt: the values a macro asks for, then `macro.play`.
//!
//! A Rhai macro can declare `// @param name Label | default`. With `{a, b}` after the label,
//! Play shows a dropdown.

use egui::RichText;
use serde_json::{Value, json};

use crate::SolveApp;
use crate::frame;
use crate::i18n::{tr, tr_label};
use crate::theme::Tokens;

/// Dialog state: the typed values (the macro itself lives in the session).
#[derive(Default)]
pub struct MacroRun {
    pub name: String,
    pub description: Option<String>,
    /// One row per declared parameter: name, label, typed value, dropdown choices (empty: text).
    pub fields: Vec<(String, String, String, Vec<String>)>,
    pub error: Option<String>,
}

/// Open the prompt for the loaded macro (no-op when none is loaded).
pub fn open(app: &mut SolveApp) {
    let Some(m) = app.session.macros.loaded.clone() else { return };
    app.macro_run = Some(MacroRun {
        name: m.name.unwrap_or_default(),
        description: m.description,
        fields: m
            .params
            .iter()
            .map(|p| {
                let default = p.default.as_ref().map(|d| match d {
                    Value::String(s) => s.clone(),
                    other => other.to_string(),
                });
                (p.name.clone(), p.label.clone().unwrap_or_else(|| p.name.clone()), default.unwrap_or_default(), p.choices.clone())
            })
            .collect(),
        error: None,
    });
}

/// Draw the prompt when it is open. Called once per frame.
pub fn show(app: &mut SolveApp, ctx: &egui::Context) {
    let Some(mut d) = app.macro_run.take() else { return };
    let t = Tokens::get();
    let mut open = true;
    let mut cancel = false;
    let mut close_and_run = false;
    let window = frame::window(ctx, RichText::new(tr_label("Play Macro")).strong().size(13.0), frame::Width::Normal)
        .id(egui::Id::new("sc_macro_run"))
        .open(&mut open)
        .default_width(420.0)
        .collapsible(false);
    window.show(ctx, |ui| {
        if !d.name.is_empty() {
            ui.label(RichText::new(&d.name).strong());
        }
        if let Some(text) = &d.description {
            ui.label(RichText::new(text).color(t.text_dim));
        }
        if !d.fields.is_empty() {
            ui.separator();
            egui::Grid::new("sc_macro_run_fields").num_columns(2).spacing([10.0, 6.0]).show(ui, |ui| {
                for (name, label, value, choices) in &mut d.fields {
                    ui.label(format!("{label}:"));
                    if choices.is_empty() {
                        ui.add(egui::TextEdit::singleline(value).hint_text(name.clone()).desired_width(220.0));
                    } else {
                        let mut sel = choices.iter().position(|c| c == value).unwrap_or(0);
                        let opts: Vec<&str> = choices.iter().map(String::as_str).collect();
                        let id = format!("macro_{name}");
                        crate::ui_kit::dropdown(ui, &id, &mut sel, &opts);
                        if let Some(c) = choices.get(sel) {
                            *value = c.clone();
                        }
                    }
                    ui.end_row();
                }
            });
        }
        if let Some(e) = &d.error {
            ui.label(RichText::new(e).color(t.error));
        }
        ui.separator();
        ui.horizontal(|ui| {
            if ui.button(tr("ui.run")).clicked() {
                close_and_run = true;
            }
            if ui.button(tr("ui.cancel")).clicked() {
                cancel = true;
            }
        });
    });
    if close_and_run {
        let mut params = json!({});
        for (name, _, value, _) in &d.fields {
            let value = value.trim();
            if !value.is_empty() {
                params[name] = json!(value);
            }
        }
        match app.run("macro.play", json!({"params": params})) {
            Ok(v) => {
                let steps = v.get("ran").and_then(Value::as_u64).unwrap_or(0);
                app.set_status(crate::i18n::format("macro.played", &[("steps", &steps.to_string())]), false);
            }
            Err(e) => {
                d.error = Some(e);
                app.macro_run = Some(d);
            }
        }
    } else if open && !cancel {
        app.macro_run = Some(d);
    }
}
