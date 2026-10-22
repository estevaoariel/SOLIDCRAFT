//! The Play Macro prompt: the values a macro asks for, then `macro.play`.
//!
//! A Rhai macro can declare `// @param name Label | default`. `{a, b}` is a dropdown.
//! `@if other=a,b` hides the row unless that field's value is one of them.

use egui::RichText;
use serde_json::{Value, json};

use crate::SolveApp;
use crate::frame;
use crate::i18n::{tr, tr_label};
use crate::theme::Tokens;

struct Field {
    name: String,
    label: String,
    value: String,
    choices: Vec<String>,
    when: Option<(String, Vec<String>)>,
}

/// Dialog state: the typed values (the macro itself lives in the session).
#[derive(Default)]
pub struct MacroRun {
    pub name: String,
    fields: Vec<Field>,
    pub error: Option<String>,
}

fn visible(fields: &[Field], f: &Field) -> bool {
    visible_from(fields, f, 0)
}

fn visible_from(fields: &[Field], f: &Field, depth: usize) -> bool {
    if depth > 16 {
        return false;
    }
    let Some((name, values)) = &f.when else { return true };
    let Some(other) = fields.iter().find(|o| o.name == *name) else { return false };
    if !visible_from(fields, other, depth + 1) {
        return false;
    }
    values.iter().any(|v| v == other.value.as_str())
}

/// Open the prompt for the loaded macro (no-op when none is loaded).
pub fn open(app: &mut SolveApp) {
    let Some(m) = app.session.macros.loaded.clone() else { return };
    app.macro_run = Some(MacroRun {
        name: m.name.unwrap_or_default(),
        fields: m
            .params
            .iter()
            .map(|p| {
                let default = p.default.as_ref().map(|d| match d {
                    Value::String(s) => s.clone(),
                    other => other.to_string(),
                });
                Field {
                    name: p.name.clone(),
                    label: p.label.clone().unwrap_or_else(|| p.name.clone()),
                    value: default.unwrap_or_default(),
                    choices: p.choices.clone(),
                    when: p.when.as_ref().map(|w| (w.name.clone(), w.values.clone())),
                }
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
        if !d.fields.is_empty() {
            ui.separator();
            egui::Grid::new("sc_macro_run_fields").num_columns(2).spacing([10.0, 6.0]).show(ui, |ui| {
                for i in 0..d.fields.len() {
                    if !visible(&d.fields, &d.fields[i]) {
                        continue;
                    }
                    ui.label(format!("{}:", d.fields[i].label));
                    if d.fields[i].choices.is_empty() {
                        let hint = d.fields[i].name.clone();
                        ui.add(egui::TextEdit::singleline(&mut d.fields[i].value).hint_text(hint).desired_width(220.0));
                    } else {
                        let mut sel = d.fields[i].choices.iter().position(|c| c == &d.fields[i].value).unwrap_or(0);
                        let opts: Vec<&str> = d.fields[i].choices.iter().map(String::as_str).collect();
                        let id = format!("macro_{}", d.fields[i].name);
                        crate::ui_kit::dropdown(ui, &id, &mut sel, &opts);
                        if let Some(c) = d.fields[i].choices.get(sel) {
                            d.fields[i].value = c.clone();
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
        for f in &d.fields {
            if !visible(&d.fields, f) {
                continue;
            }
            let value = f.value.trim();
            if !value.is_empty() {
                params[&f.name] = json!(value);
            }
        }
        match app.run("macro.play", json!({"params": params})) {
            Ok(v) => {
                let steps = v.get("steps").and_then(Value::as_u64).unwrap_or(0);
                app.set_status(crate::i18n::format("macro.played", &[("steps", &steps.to_string())]), false);
                app.home.open = false;
                app.fit_view();
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

#[cfg(test)]
mod tests {
    use super::{Field, visible};

    fn field(name: &str, value: &str, when: Option<(&str, &[&str])>) -> Field {
        Field {
            name: name.into(),
            label: name.into(),
            value: value.into(),
            choices: Vec::new(),
            when: when.map(|(n, vs)| (n.into(), vs.iter().map(|s| (*s).into()).collect())),
        }
    }

    #[test]
    fn module_hides_when_kind_is_sprocket() {
        let fields = [
            field("kind", "sprocket", None),
            field("sizing", "module", Some(("kind", &["gear"]))),
            field("gear_module", "2", Some(("sizing", &["module"]))),
        ];
        assert!(visible(&fields, &fields[0]));
        assert!(!visible(&fields, &fields[1]));
        assert!(!visible(&fields, &fields[2]), "Module follows Size, which is hidden for a sprocket");
    }

    #[test]
    fn module_shows_for_a_module_gear() {
        let fields = [
            field("kind", "gear", None),
            field("sizing", "module", Some(("kind", &["gear"]))),
            field("gear_module", "2", Some(("sizing", &["module"]))),
        ];
        assert!(visible(&fields, &fields[1]));
        assert!(visible(&fields, &fields[2]));
    }
}
