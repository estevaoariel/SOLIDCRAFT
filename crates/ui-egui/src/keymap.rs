//! Keyboard shortcuts: every command's default key comes from its `CommandSpec`, or from the
//! shortcut set chosen (`SETS`); the user can rebind or clear any of them on top of the set
//! (Preferences › Keyboard Shortcuts). A new key that another command already uses is reported
//! and only taken over on request; keys the app itself handles (Esc, S, V, Delete, Ctrl+Z…, and
//! the set's view keys) can't be bound. Changes are kept between runs.

use std::collections::BTreeMap;

use serde_json::{Value, json};
use solidcraft_engine::{CommandSpec, command_specs};

use crate::SolveApp;
use crate::ui_kit;

/// Repeats the last tool or dialog started (`SolveApp::repeat_last`, #3045).
pub const REPEAT_KEY: &str = "Shift+R";

/// Keys the application handles before any command shortcut.
pub const RESERVED: &[&str] = &[
    REPEAT_KEY,
    "Escape",
    "S",
    "V",
    "F2",
    "F6",
    "Delete",
    "Backspace",
    "Ctrl+Z",
    "Ctrl+Y",
    "Ctrl+S",
    "Ctrl+N",
    "Ctrl+O",
    "Ctrl+Comma",
    "Ctrl+Shift+A",
    "Ctrl+C",
    "Ctrl+X",
    "Ctrl+V",
    // View keys (`navigation::view_key`).
    "Left",
    "Right",
    "Up",
    "Down",
    "Shift+Left",
    "Shift+Right",
    "Shift+Up",
    "Shift+Down",
    "Ctrl+Left",
    "Ctrl+Right",
    "Ctrl+Up",
    "Ctrl+Down",
    "Ctrl+8",
];

/// What a set's view key does; the window handles it, like F6 and the arrows.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ViewKey {
    /// A view name `SolveApp::animate_view` takes (`front`…, `iso`, `fit`).
    View(&'static str),
    /// Look straight at the selected face, plane or sketch (Ctrl+8).
    LookAt,
    ZoomIn,
    ZoomOut,
}

/// A named set of default keys.
pub struct ShortcutSet {
    pub id: &'static str,
    /// Its name, or the catalog id of it.
    pub label: &'static str,
    /// Catalog id of the note the page shows (`""`: none).
    pub note: &'static str,
    /// Command id → key, over the command's own (`""`: no key).
    pub keys: &'static [(&'static str, &'static str)],
    /// Keys the window handles while the set is chosen.
    pub views: &'static [(&'static str, ViewKey)],
}

impl ShortcutSet {
    pub fn name(&self) -> &str {
        crate::i18n::tr_label(self.label)
    }
}

pub const DEFAULT_SET: &str = "default";
pub const ONSHAPE_SET: &str = "onshape";

/// The shortcut sets; the first is the default. Onshape's follows its public shortcut list
/// (docs/book/src/guide/shortcuts.md).
pub const SETS: [ShortcutSet; 2] = [
    ShortcutSet { id: DEFAULT_SET, label: "keymap.set.default", note: "", keys: &[], views: &[] },
    ShortcutSet {
        id: ONSHAPE_SET,
        label: "Onshape",
        note: "keymap.set.onshape_note",
        keys: &[
            ("sketch.create", "Shift+S"),
            ("solid.extrude", "Shift+E"),
            ("solid.revolve", "Shift+W"),
            ("solid.fillet", "Shift+F"),
            ("inspect.section", "Shift+X"),
            ("inspect.measure", "OpenBracket"),
            ("sketch.rectangle.center", "R"),
            ("sketch.rectangle.two_point", "G"),
            ("sketch.arc.three_point", "A"),
            ("sketch.arc.tangent", "Shift+A"),
            ("sketch.point", "Shift+S"),
            ("sketch.construction", "Q"),
            ("sketch.extend", "X"),
            ("sketch.project", "U"),
            ("sketch.trim", "M"),
            ("sketch.fillet", "Shift+F"),
            ("sketch.constraint.coincident", "I"),
            ("sketch.constraint.concentric", "Shift+O"),
            ("sketch.constraint.curvature", "Shift+U"),
            ("sketch.constraint.equal", "E"),
            ("sketch.constraint.fix", "Shift+J"),
            ("sketch.constraint.horizontal_vertical", "H"),
            ("sketch.constraint.midpoint", "Shift+M"),
            ("sketch.constraint.parallel", "B"),
            ("sketch.constraint.perpendicular", "Shift+L"),
            ("sketch.constraint.symmetry", "Shift+Q"),
            ("sketch.constraint.tangent", "T"),
            ("inspect.curvature_comb", "Shift+C"),
            // Default keys this set gives to other commands.
            ("solid.press_pull", ""),
            ("solid.hole", ""),
            ("solid.move", ""),
            ("appearance.assign", ""),
        ],
        views: &[
            ("Shift+1", ViewKey::View("front")),
            ("Shift+2", ViewKey::View("back")),
            ("Shift+3", ViewKey::View("left")),
            ("Shift+4", ViewKey::View("right")),
            ("Shift+5", ViewKey::View("top")),
            ("Shift+6", ViewKey::View("bottom")),
            ("Shift+7", ViewKey::View("iso")),
            ("F", ViewKey::View("fit")),
            ("N", ViewKey::LookAt),
            ("Z", ViewKey::ZoomOut),
            ("Shift+Z", ViewKey::ZoomIn),
        ],
    },
];

/// The set with this id (the default for an unknown one).
pub fn shortcut_set(id: &str) -> &'static ShortcutSet {
    SETS.iter().find(|s| s.id == id).unwrap_or(&SETS[0])
}

/// A command's key in a set, before the user's changes (`""`: none).
pub fn default_key(set: &str, id: &str) -> &'static str {
    shortcut_set(set)
        .keys
        .iter()
        .find(|(c, _)| *c == id)
        .map(|(_, k)| *k)
        .or_else(|| solidcraft_engine::find_command(id).and_then(|c| c.shortcut))
        .unwrap_or("")
}

/// The chosen set's view key, if `key` is one and none of the user's own keys holds it.
pub fn view_key(app: &SolveApp, key: &str) -> Option<ViewKey> {
    if held(app, key) {
        return None;
    }
    shortcut_set(&app.preferences.shortcut_set).views.iter().find(|(k, _)| *k == key).map(|(_, v)| *v)
}

/// Is `key` one the user bound to a command themselves?
fn held(app: &SolveApp, key: &str) -> bool {
    !key.is_empty() && app.keymap.custom.iter().any(|(id, k)| k == key && solidcraft_engine::find_command(id).is_some())
}

/// A key as its shortcut name sees it: a digit-row key is its digit, with Shift held too (the
/// layout would make Shift+1 a `!`).
pub fn layout_key(key: egui::Key, physical: Option<egui::Key>) -> egui::Key {
    use egui::Key::*;
    match (key, physical) {
        // The numpad without Num Lock: its arrows and paging keys stay what they are.
        (ArrowUp | ArrowDown | ArrowLeft | ArrowRight | Home | End | PageUp | PageDown | Insert | Delete, _) => key,
        (_, Some(p @ (Num0 | Num1 | Num2 | Num3 | Num4 | Num5 | Num6 | Num7 | Num8 | Num9))) => p,
        _ => key,
    }
}

/// The key the layout typed when `layout_key` read the digit instead: keys saved under it
/// before (German Shift+7 as `Shift+Slash`) still run.
pub fn layout_alias(key: egui::Key, physical: Option<egui::Key>) -> Option<egui::Key> {
    Some(key).filter(|k| *k != layout_key(key, physical))
}

#[derive(Default)]
pub struct Keymap {
    /// Command id → key (`""`: no key). Only the changed ones.
    pub custom: BTreeMap<String, String>,
    /// Asks for the Keyboard Shortcuts page of Preferences (File and Help menus).
    pub open: bool,
    /// Opened from Help: a list to read, not to edit.
    pub read_only: bool,
    filter: String,
    /// The command waiting for a key press.
    capturing: Option<String>,
    /// A key that another command uses: (command, key, the other commands).
    conflict: Option<(String, String, Vec<String>)>,
    message: Option<String>,
}

impl Keymap {
    pub fn prefs(&self) -> Value {
        json!(self.custom)
    }
    pub fn load(&mut self, v: &Value) {
        self.custom =
            v.as_object().map(|o| o.iter().take(2000).filter_map(|(k, v)| Some((k.clone(), v.as_str()?.to_string()))).collect()).unwrap_or_default();
    }
}

/// A key press as a shortcut name: `E`, `Shift+E`, `Ctrl+Alt+K`, `F5`, `Delete`.
pub fn key_name(key: egui::Key, mods: egui::Modifiers) -> String {
    let mut s = String::new();
    if mods.command || mods.ctrl {
        s.push_str("Ctrl+");
    }
    if mods.alt {
        s.push_str("Alt+");
    }
    if mods.shift {
        s.push_str("Shift+");
    }
    s.push_str(key.name());
    s
}

/// The key a command runs from now: the user's own, else its default unless one of the user's
/// keys took it.
pub fn effective(app: &SolveApp, id: &str) -> Option<String> {
    match app.keymap.custom.get(id) {
        Some(k) if k.is_empty() => None,
        Some(k) => Some(k.clone()),
        None => Some(default_key(&app.preferences.shortcut_set, id)).filter(|k| !k.is_empty() && !held(app, k)).map(str::to_string),
    }
}

/// Commands bound to `key` now.
pub fn commands_for(app: &SolveApp, key: &str) -> Vec<&'static str> {
    command_specs().into_iter().filter(|c| effective(app, c.id).as_deref() == Some(key)).map(|c| c.id).collect()
}

/// The enabled command a key press runs, if any; in a sketch, a sketch command first (a set can
/// give one key to a sketch and a solid command, as Shift+F to both Fillets).
pub fn command_for(app: &SolveApp, key: &str) -> Option<&'static str> {
    let enabled: Vec<&'static CommandSpec> =
        commands_for(app, key).into_iter().filter_map(solidcraft_engine::find_command).filter(|c| c.info(&app.session).enabled).collect();
    let sketching = app.session.active_sketch.is_some();
    enabled.iter().find(|c| !sketching || c.tab == "SKETCH").or(enabled.first()).map(|c| c.id)
}

/// What a command's key starts: a split family's last variant, unless the set names the command.
fn to_start(app: &SolveApp, id: &'static str) -> &'static str {
    if shortcut_set(&app.preferences.shortcut_set).keys.iter().any(|(c, _)| *c == id) { id } else { app.preferences.sketch_variants.resolve(id) }
}

/// Run a shortcut key: its enabled command; outside a sketch, a sketch tool's key opens Create
/// Sketch (on the selected plane or planar face, else on the plane picked next) and the tool
/// starts in the new sketch. Returns whether the key started something.
pub fn press(app: &mut SolveApp, key: &str) -> bool {
    if let Some(id) = command_for(app, key) {
        app.start(to_start(app, id));
    } else if let Some(id) = sketch_tool_for(app, key) {
        let id = to_start(app, id);
        app.start("sketch.create");
        if let Some(d) = app.dialog.as_mut().filter(|d| matches!(d.kind, crate::dialogs::Kind::Sketch)) {
            d.then_start = Some(id);
        }
    } else {
        return false;
    }
    true
}

/// `press` for a key event: its `layout_key` name, else the name of its `layout_alias`.
pub fn press_event(app: &mut SolveApp, key: egui::Key, alias: Option<egui::Key>, mods: egui::Modifiers) {
    if !press(app, &key_name(key, mods))
        && let Some(alias) = alias
    {
        press(app, &key_name(alias, mods));
    }
}

/// Outside a sketch: the SKETCH-tab command bound to `key` that a new sketch would start. Not
/// Normal/Construction (a mode of the drawing in progress), the Finish Sketch panel nor a
/// constraint (nothing to constrain yet).
fn sketch_tool_for(app: &SolveApp, key: &str) -> Option<&'static str> {
    if app.session.active_sketch.is_some() {
        return None;
    }
    commands_for(app, key).into_iter().find(|id| {
        solidcraft_engine::find_command(id)
            .is_some_and(|c| c.tab == "SKETCH" && !matches!(c.panel, "FINISH SKETCH" | "CONSTRAINTS") && c.id != "sketch.construction")
    })
}

/// Bind `key` to a command (`""` clears it). Refused for reserved keys and the set's view keys,
/// and when other commands use the key unless `replace` (they lose it).
pub fn bind(app: &mut SolveApp, id: &str, key: &str, replace: bool) -> Result<(), String> {
    if solidcraft_engine::find_command(id).is_none() {
        return Err(format!("no command `{id}`"));
    }
    if RESERVED.contains(&key) || view_key(app, key).is_some() {
        return Err(format!("{key} is used by SolidCraft itself"));
    }
    if !key.is_empty() {
        let others: Vec<&'static str> = commands_for(app, key).into_iter().filter(|o| *o != id).collect();
        if !others.is_empty() && !replace {
            let names: Vec<&str> = others.iter().filter_map(|o| solidcraft_engine::find_command(o).map(|c| c.label)).collect();
            return Err(format!("{key} is already used by {}", names.join(", ")));
        }
        for o in others {
            app.keymap.custom.insert(o.to_string(), String::new());
        }
    }
    if default_key(&app.preferences.shortcut_set, id) == key {
        app.keymap.custom.remove(id);
    } else {
        app.keymap.custom.insert(id.to_string(), key.to_string());
    }
    Ok(())
}

pub fn reset(app: &mut SolveApp, id: Option<&str>) {
    match id {
        Some(id) => drop(app.keymap.custom.remove(id)),
        None => app.keymap.custom.clear(),
    }
}

/// The Keyboard Shortcuts page of Preferences: search, rebind (click, then press the key), clear,
/// reset; a key another command uses is reported and taken over only on request.
pub fn page(app: &mut SolveApp, ui: &mut egui::Ui) {
    let ctx = ui.ctx().clone();
    // A key press while waiting for one binds it (Esc cancels).
    if let Some(id) = app.keymap.capturing.clone() {
        let press = ctx.input(|i| {
            i.events.iter().find_map(|e| match e {
                egui::Event::Key { key, physical_key, pressed: true, modifiers, .. } => Some((layout_key(*key, *physical_key), *modifiers)),
                _ => None,
            })
        });
        if let Some((k, m)) = press {
            app.keymap.capturing = None;
            if k != egui::Key::Escape {
                let name = key_name(k, m);
                match bind(app, &id, &name, false) {
                    Ok(()) => app.keymap.message = None,
                    Err(e) if e.contains("already used") => {
                        app.keymap.conflict = Some((id.clone(), name.clone(), commands_for(app, &name).iter().map(|s| s.to_string()).collect()));
                        app.keymap.message = Some(e);
                    }
                    Err(e) => app.keymap.message = Some(e),
                }
            }
        }
    }
    let mut action: Option<(String, &str)> = None;
    ui_kit::scope(ui, |ui| {
        set_row(app, ui);
        let actions = [
            ui_kit::SearchAction { label: app.tr_label("Reset All"), icon: None, visible: !app.keymap.read_only, tooltip: None },
            ui_kit::SearchAction { label: app.tr_label("Import…"), icon: Some("import_tray"), visible: !app.keymap.read_only, tooltip: None },
            ui_kit::SearchAction {
                label: app.tr_label("Export…"),
                icon: Some("export_tray"),
                visible: true,
                tooltip: Some(app.tr_label("Keys, S box favourites and the toolbar, as a file")),
            },
        ];
        let (filter, toolbar_action) = ui_kit::search_actions_row(
            ui,
            egui::Id::new("sc_keymap_filter"),
            &mut app.keymap.filter,
            crate::i18n::tr_label("Filter commands or keys"),
            &actions,
        );
        crate::scenario::publish_target("keymap:filter", filter.rect);
        if let Some(index) = toolbar_action {
            action = Some((
                String::new(),
                match index {
                    0 => "reset_all",
                    1 => "import",
                    _ => "export",
                },
            ));
        }
        if let Some(m) = &app.keymap.message {
            ui_kit::text(ui, m, ui_kit::Text::Report("keymap_message", true));
        }
        if let Some((id, key, _)) = app.keymap.conflict.clone() {
            ui.horizontal_wrapped(|ui| {
                let replace = ui_kit::button(ui, crate::i18n::format("keymap.use_anyway", &[("key", &key)]), ui_kit::Button::Normal);
                crate::scenario::publish_target("keymap:replace", replace.rect);
                if replace.clicked() {
                    action = Some((format!("{id}\n{key}"), "replace"));
                }
                if ui_kit::button(ui, app.tr_label("Cancel"), ui_kit::Button::Normal).clicked() {
                    action = Some((String::new(), "cancel"));
                }
            });
        }
        ui_kit::text(ui, app.tr_label("Select Other: hold the left button still on the model (or right-click › Select Other…) to list everything under the cursor; click again on the same spot to select the next thing there. Double-click the middle button to go Home (Preferences › Navigation)."), ui_kit::Text::Note("navigation"));
        let q = app.keymap.filter.to_lowercase();
        let commands = command_specs();
        let t = crate::theme::Tokens::get();
        let key_width = commands
            .iter()
            .map(|c| {
                let key = effective(app, c.id).unwrap_or_default();
                ui_kit::ContentWidth::button(ui, &key)
            })
            .fold(ui_kit::ContentWidth::button(ui, app.tr_label("press a key…")), f32::max)
            .max(ui_kit::ContentWidth::label(ui, app.tr_label("Keyboard Shortcuts")) + t.space_8);
        let action_width = if app.keymap.read_only {
            t.control_height
        } else {
            ui_kit::ContentWidth::button(ui, app.tr_label("Clear")) + ui_kit::ContentWidth::button(ui, app.tr_label("Reset")) + t.space_4
        };
        ui_kit::settings_scroll(ui, "sc_keymap_scroll", |ui| {
            let columns = [
                ui_kit::Column::new("Command", 1.5).flex().min_content_width(t.form_label_width),
                ui_kit::Column::new(crate::i18n::tr("keymap.command_id"), 1.8).flex().min_content_width(t.form_label_width),
                ui_kit::Column::new("Keyboard Shortcuts", 1.0).flex().min_content_width(key_width),
                ui_kit::Column::new("", 0.0).icon_only().min_content_width(action_width),
            ];
            ui_kit::table(ui, "keymap", &columns, crate::i18n::tr("keymap.empty"), |ui, table| {
                for c in commands {
                    let key = effective(app, c.id).unwrap_or_default();
                    let translated_label = app.tr_label(c.label);
                    if !q.is_empty() && ![c.label, c.id, key.as_str(), translated_label].iter().any(|s| s.to_lowercase().contains(&q)) {
                        continue;
                    }
                    table.row(ui, c.id, app.keymap.capturing.as_deref() == Some(c.id), |row| {
                        row.cell(|ui| {
                            ui_kit::text(ui, translated_label, ui_kit::Text::Cell);
                        });
                        row.cell(|ui| {
                            ui_kit::text(ui, c.id, ui_kit::Text::Cell);
                        });
                        let capturing = app.keymap.capturing.as_deref() == Some(c.id);
                        let changed = app.keymap.custom.contains_key(c.id);
                        let text = if capturing {
                            app.tr_label("press a key…").to_string()
                        } else if key.is_empty() {
                            "—".to_string()
                        } else {
                            key.clone()
                        };
                        row.cell(|ui| {
                            if app.keymap.read_only {
                                ui_kit::text(ui, &text, ui_kit::Text::Cell);
                            } else {
                                let capture = if key.is_empty() && !capturing {
                                    let (rect, _) = ui.allocate_exact_size(egui::vec2(t.control_height, t.control_height), egui::Sense::hover());
                                    ui_kit::quiet_icon_button_at(ui, rect, "plus", app.tr_label("Click, then press the new key"))
                                } else {
                                    ui_kit::button(ui, &text, ui_kit::Button::Normal).on_hover_text(app.tr_label("Click, then press the new key"))
                                };
                                crate::scenario::publish_target(&format!("keymap:capture:{}", c.id), capture.rect);
                                if capture.clicked() {
                                    action = Some((c.id.to_string(), "capture"));
                                }
                            }
                        });
                        row.cell(|ui| {
                            if !app.keymap.read_only {
                                ui.horizontal(|ui| {
                                    let clear = ui_kit::button(ui, app.tr_label("Clear"), ui_kit::Button::Enabled(!key.is_empty()));
                                    crate::scenario::publish_target(&format!("keymap:clear:{}", c.id), clear.rect);
                                    if clear.clicked() {
                                        action = Some((c.id.to_string(), "clear"));
                                    }
                                    let reset = ui_kit::button(ui, app.tr_label("Reset"), ui_kit::Button::Enabled(changed));
                                    crate::scenario::publish_target(&format!("keymap:reset:{}", c.id), reset.rect);
                                    if reset.clicked() {
                                        action = Some((c.id.to_string(), "reset"));
                                    }
                                });
                            }
                        });
                    });
                }
            });
        });
    });
    match action {
        Some((id, "capture")) => {
            app.keymap.capturing = Some(id);
            app.keymap.conflict = None;
            app.keymap.message = None;
        }
        Some((id, "clear")) => drop(bind(app, &id, "", false)),
        Some((id, "reset")) => reset(app, Some(&id)),
        Some((_, "reset_all")) => reset(app, None),
        Some((_, "export")) => {
            let text = crate::toolbar_custom::export(app).to_string();
            app.pick_save("SolidCraft shortcuts.json", &["json"], move |app, path| {
                if let Some(path) = path {
                    app.keymap.message = std::fs::write(&path, text).err().map(|e| format!("export: {e}"));
                }
            });
        }
        Some((_, "import")) => {
            app.pick_open(|app, path| {
                let Some(path) = path else { return };
                let r = std::fs::read_to_string(&path)
                    .map_err(|e| e.to_string())
                    .and_then(|t| serde_json::from_str::<Value>(&t).map_err(|e| e.to_string()))
                    .and_then(|v| crate::toolbar_custom::import(app, &v));
                app.keymap.message = r.err().map(|e| format!("import: {e}"));
            });
        }
        Some((pair, "replace")) => {
            if let Some((id, key)) = pair.split_once('\n') {
                let _ = bind(app, id, key, true);
            }
            app.keymap.conflict = None;
            app.keymap.message = None;
        }
        Some((_, "cancel")) => {
            app.keymap.conflict = None;
            app.keymap.message = None;
        }
        _ => {}
    }
}

/// The shortcut set: a choice on the page, its name when read only, and its note.
fn set_row(app: &mut SolveApp, ui: &mut egui::Ui) {
    let current = shortcut_set(&app.preferences.shortcut_set);
    ui_kit::row(ui, crate::i18n::tr("keymap.set"), |ui| {
        if app.keymap.read_only {
            ui_kit::text(ui, current.name(), ui_kit::Text::Cell);
            return;
        }
        let picker = ui_kit::dropdown_menu(ui, "pref_shortcut_set", current.name(), |ui| {
            for set in &SETS {
                let item = ui_kit::list_picker(ui, set.id, set.name(), current.id == set.id);
                crate::scenario::publish_handle(&format!("keymap:set:{}", set.id), item.rect.center());
                if item.clicked() {
                    app.preferences.shortcut_set = set.id.into();
                }
            }
        });
        crate::scenario::publish_handle("keymap:set", picker.rect.center());
    });
    if !current.note.is_empty() {
        ui_kit::text(ui, crate::i18n::tr(current.note), ui_kit::Text::Note("keymap_set"));
    }
}

/// Stop waiting for a key (the dialog closed).
pub fn stop_capture(app: &mut SolveApp) {
    app.keymap.capturing = None;
    app.keymap.conflict = None;
    app.keymap.message = None;
}

/// Is the page waiting for a key (so the key must not also run a command)?
pub fn capturing(app: &SolveApp) -> bool {
    app.prefs_window.open && app.prefs_window.section == crate::prefs::PAGE_SHORTCUTS && app.keymap.capturing.is_some()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn app() -> SolveApp {
        SolveApp::new(solidcraft_engine::Session::default(), Default::default())
    }

    #[test]
    fn rebinding_detects_conflicts_and_resets() {
        let mut a = app();
        assert_eq!(effective(&a, "solid.extrude").as_deref(), Some("E"));
        // Shift+E is free.
        bind(&mut a, "solid.extrude", "Shift+E", false).unwrap();
        assert_eq!(commands_for(&a, "Shift+E"), vec!["solid.extrude"]);
        assert!(commands_for(&a, "E").is_empty());
        // M belongs to Move/Copy: refused, then taken over on request.
        let e = bind(&mut a, "solid.revolve", "M", false).unwrap_err();
        assert!(e.contains("already used by Move/Copy"), "{e}");
        bind(&mut a, "solid.revolve", "M", true).unwrap();
        assert_eq!(commands_for(&a, "M"), vec!["solid.revolve"]);
        assert_eq!(effective(&a, "solid.move"), None);
        assert!(bind(&mut a, "solid.revolve", "Ctrl+Z", false).is_err(), "reserved");
        // Kept between runs.
        let mut b = app();
        b.keymap.load(&a.keymap.prefs());
        assert_eq!(effective(&b, "solid.revolve").as_deref(), Some("M"));
        reset(&mut b, None);
        assert_eq!(effective(&b, "solid.move").as_deref(), Some("M"));
        assert_eq!(key_name(egui::Key::K, egui::Modifiers { ctrl: true, shift: true, ..Default::default() }), "Ctrl+Shift+K");
    }

    /// #3045: Shift+R starts the last tool again (here a sketch's Line after Escape put it
    /// down), R alone is still the rectangle; the menus show the key on Repeat, and no command
    /// can take it.
    #[test]
    fn shift_r_repeats_the_last_tool() {
        let mut h = crate::scenario::Harness::new();
        h.app.home.open = false;
        h.app.run("sketch.create", json!({"plane": "XY"})).unwrap();
        h.frames(2);
        h.step(&json!({"start": "sketch.line"})).unwrap();
        h.step(&json!({"key": "Escape"})).unwrap();
        h.frames(1);
        assert!(h.app.tool.is_none());
        h.step(&json!({"key": "R", "shift": true})).unwrap();
        h.frames(1);
        assert_eq!(h.app.tool.as_ref().map(|t| t.cmd.to_string()).as_deref(), Some("sketch.line"), "Shift+R starts Line again");
        h.step(&json!({"key": "Escape"})).unwrap();
        h.step(&json!({"key": "R"})).unwrap();
        h.frames(1);
        let rectangle = h.app.tool.as_ref().map(|t| t.cmd.to_string()).unwrap_or_default();
        assert!(rectangle.starts_with("sketch.rectangle"), "R alone: {rectangle}");
        let repeat = crate::context_menu::radial_items(&h.app).into_iter().find(|i| i.id == "ui.repeat").unwrap();
        assert_eq!(repeat.shortcut, REPEAT_KEY);
        let mut a = app();
        assert!(bind(&mut a, "solid.extrude", REPEAT_KEY, true).is_err(), "Shift+R is the app's");
    }
}

#[cfg(test)]
mod set_tests {
    use super::*;
    use crate::scenario::Harness;

    fn app() -> SolveApp {
        SolveApp::new(solidcraft_engine::Session::default(), Default::default())
    }

    fn choose(app: &mut SolveApp, set: &str) {
        crate::prefs::set(app, &json!({"shortcut_set": set})).unwrap();
    }

    /// The Onshape set moves the defaults, a key it shares picks the command for the context,
    /// the user's own keys stay through switching, and Default puts the defaults back.
    #[test]
    fn onshape_set_changes_the_defaults_under_the_users_keys() {
        let mut a = app();
        bind(&mut a, "solid.revolve", "Ctrl+Shift+K", false).unwrap();
        choose(&mut a, ONSHAPE_SET);
        for (id, key) in [
            ("solid.extrude", Some("Shift+E")),
            ("solid.fillet", Some("Shift+F")),
            ("sketch.create", Some("Shift+S")),
            ("inspect.measure", Some("OpenBracket")),
            ("sketch.trim", Some("M")),
            ("sketch.rectangle.two_point", Some("G")),
            ("sketch.line", Some("L")),
            ("inspect.curvature_comb", Some("Shift+C")),
            ("solid.move", None),
            ("solid.revolve", Some("Ctrl+Shift+K")),
        ] {
            assert_eq!(effective(&a, id).as_deref(), key, "{id}");
        }
        assert_eq!(command_for(&a, "Shift+F"), Some("solid.fillet"));
        assert_eq!(command_for(&a, "Shift+S"), Some("sketch.create"));
        assert_eq!(command_for(&a, "E"), None, "Equal needs a sketch, and E no longer extrudes");
        a.run("sketch.create", json!({"plane": "XY"})).unwrap();
        assert_eq!(command_for(&a, "Shift+F"), Some("sketch.fillet"), "in a sketch, its own Fillet");
        assert_eq!(command_for(&a, "Shift+S"), Some("sketch.point"));
        assert_eq!(command_for(&a, "E"), Some("sketch.constraint.equal"));
        assert!(bind(&mut a, "sketch.line", "F", false).unwrap_err().contains("SolidCraft itself"), "F is the set's Fit");
        bind(&mut a, "sketch.line", "K", false).unwrap();
        choose(&mut a, DEFAULT_SET);
        assert_eq!(effective(&a, "solid.extrude").as_deref(), Some("E"));
        assert_eq!(effective(&a, "solid.move").as_deref(), Some("M"));
        assert_eq!(effective(&a, "sketch.line").as_deref(), Some("K"));
        assert_eq!(effective(&a, "solid.revolve").as_deref(), Some("Ctrl+Shift+K"));
        assert_eq!(view_key(&a, "F"), None);
        // Binding a command back to the set's own key leaves nothing to keep.
        choose(&mut a, ONSHAPE_SET);
        bind(&mut a, "solid.extrude", "Shift+E", false).unwrap();
        assert!(!a.keymap.custom.contains_key("solid.extrude"));
    }

    /// The set is a preference: kept between runs, refused when unknown over `ui.prefs`, reset by
    /// Restore Defaults on the page, and carried by a shortcuts file.
    #[test]
    fn shortcut_set_persists_and_is_set_over_the_control_channel() {
        let mut h = Harness::new();
        h.app.home.open = false;
        let out = h.call("ui.prefs", json!({"shortcut_set": "onshape"}));
        assert_eq!(out["result"]["shortcut_set"], "onshape", "{out}");
        assert_eq!(h.call("ui.shortcut", json!({"command": "solid.extrude"}))["result"]["key"], "Shift+E");
        let bad = h.call("ui.prefs", json!({"shortcut_set": "nope"}));
        assert!(bad["error"].as_str().is_some_and(|e| e.contains("shortcut_set")), "{bad}");
        assert_eq!(h.app.preferences.shortcut_set, ONSHAPE_SET);
        bind(&mut h.app, "sketch.line", "K", false).unwrap();

        let mut other = app();
        other.load_prefs(&h.app.prefs());
        assert_eq!(other.preferences.shortcut_set, ONSHAPE_SET);
        assert_eq!(effective(&other, "solid.extrude").as_deref(), Some("Shift+E"));
        assert_eq!(effective(&other, "sketch.line").as_deref(), Some("K"));

        let file = crate::toolbar_custom::export(&h.app);
        let mut imported = app();
        crate::toolbar_custom::import(&mut imported, &file).unwrap();
        assert_eq!((imported.preferences.shortcut_set.as_str(), effective(&imported, "sketch.line").as_deref()), (ONSHAPE_SET, Some("K")));

        h.call("ui.prefs", json!({"open": "Keyboard Shortcuts"}));
        h.frames(2);
        assert!(crate::scenario::handle_at("keymap:set").is_some(), "the page shows the set");
        h.call("ui.prefs", json!({"button": "defaults"}));
        assert_eq!(h.app.preferences.shortcut_set, DEFAULT_SET);
        assert_eq!(effective(&h.app, "sketch.line").as_deref(), Some("L"));
    }

    /// Picking the set on the page, then its keys from the keyboard: E alone opens no sketch,
    /// Shift+E opens Extrude, the view keys turn, zoom and fit the view (Shift+1 also as the
    /// layout sends it, `!` on the 1 key), and in a sketch M trims.
    #[test]
    fn onshape_keys_start_commands_and_turn_the_view() {
        use solidcraft_engine::render::StandardView;
        let mut h = Harness::new();
        h.app.home.open = false;
        h.app.run("solid.box", json!({"length": 10, "width": 10, "height": 10})).unwrap();
        h.call("ui.prefs", json!({"open": "Keyboard Shortcuts"}));
        h.frames(2);
        h.step(&json!({"click": {"handle": "keymap:set"}})).unwrap();
        h.frames(2);
        h.step(&json!({"click": {"handle": "keymap:set:onshape"}})).unwrap();
        assert_eq!(h.app.preferences.shortcut_set, ONSHAPE_SET);
        h.call("ui.prefs", json!({"button": "ok"}));
        h.frames(2);

        h.step(&json!({"key": "E"})).unwrap();
        h.frames(2);
        assert!(h.app.dialog.is_none() && h.app.session.active_sketch.is_none(), "E (Equal) doesn't open a sketch to constrain");
        h.step(&json!({"key": "E", "shift": true})).unwrap();
        h.frames(2);
        assert!(matches!(h.app.dialog.as_ref().map(|d| &d.kind), Some(crate::dialogs::Kind::Extrude { .. })), "Shift+E extrudes");
        h.step(&json!({"key": "Escape"})).unwrap();
        h.frames(2);

        let view = |h: &Harness, v: StandardView| {
            let (yaw, pitch) = v.angles();
            (h.app.cam.yaw - yaw).abs() < 1e-6 && (h.app.cam.pitch - pitch).abs() < 1e-6
        };
        h.step(&json!({"key": "5", "shift": true})).unwrap();
        h.settle();
        assert!(view(&h, StandardView::Top), "Shift+5 is the top view");
        let modifiers = egui::Modifiers::SHIFT;
        for pressed in [true, false] {
            h.app.synthetic.push(egui::Event::Key {
                key: egui::Key::Exclamationmark,
                physical_key: Some(egui::Key::Num1),
                pressed,
                repeat: false,
                modifiers,
            });
        }
        h.settle();
        assert!(view(&h, StandardView::Front), "Shift+1 on the 1 key is the front view");
        let far = h.app.cam.distance;
        h.step(&json!({"key": "Z"})).unwrap();
        h.frames(2);
        assert!((h.app.cam.distance / far - 1.25).abs() < 1e-6, "Z zooms out");
        h.step(&json!({"key": "Z", "shift": true})).unwrap();
        h.frames(2);
        assert!((h.app.cam.distance / far - 1.0).abs() < 1e-6, "Shift+Z zooms in");
        h.step(&json!({"key": "Z"})).unwrap();
        h.frames(2);
        h.step(&json!({"key": "F"})).unwrap();
        h.settle();
        let fitted = h.app.fitted(h.app.cam).distance;
        assert!((h.app.cam.distance / fitted - 1.0).abs() < 1e-3, "F fits: {} {fitted}", h.app.cam.distance);
        assert!(h.app.dialog.is_none(), "F is not Fillet in this set");

        h.app.run("sketch.create", json!({"plane": "XY"})).unwrap();
        h.frames(2);
        h.step(&json!({"key": "M"})).unwrap();
        h.frames(2);
        assert_eq!(h.app.tool.as_ref().map(|t| t.cmd), Some("sketch.trim"), "M trims in a sketch");
        // R and G are two rectangles: G stays the corner one after R started the centre one.
        for (key, cmd) in [("R", "sketch.rectangle.center"), ("G", "sketch.rectangle.two_point")] {
            h.step(&json!({"key": "Escape"})).unwrap();
            h.step(&json!({"key": key})).unwrap();
            h.frames(2);
            assert_eq!(h.app.tool.as_ref().map(|t| t.cmd), Some(cmd), "{key}");
        }
    }

    /// Keys the user bound under Default win over the Onshape set: a set command and a set view
    /// key on one of them give way, and show no key of their own.
    #[test]
    fn the_users_own_keys_win_over_the_set() {
        let mut h = Harness::new();
        h.app.home.open = false;
        bind(&mut h.app, "sketch.circle.center", "G", false).unwrap();
        bind(&mut h.app, "sketch.line", "N", false).unwrap();
        choose(&mut h.app, ONSHAPE_SET);
        assert_eq!(effective(&h.app, "sketch.rectangle.two_point"), None, "G is the user's");
        assert_eq!(view_key(&h.app, "N"), None, "N is the user's");
        assert_eq!(view_key(&h.app, "F"), Some(ViewKey::View("fit")));
        h.app.run("sketch.create", json!({"plane": "XY"})).unwrap();
        h.frames(2);
        for (key, cmd) in [("G", "sketch.circle.center"), ("N", "sketch.line")] {
            h.step(&json!({"key": "Escape"})).unwrap();
            let cam = h.app.cam;
            h.step(&json!({"key": key})).unwrap();
            h.frames(2);
            assert_eq!(h.app.tool.as_ref().map(|t| t.cmd), Some(cmd), "{key}");
            assert_eq!(h.app.cam, cam, "{key} doesn't move the view");
        }
    }

    /// Menus show the key that runs now: under the Onshape set Extrude's Shift+E, no key for
    /// Move/Copy and Appearance, and Q for construction mode.
    #[test]
    fn menus_show_the_sets_keys() {
        use crate::context_menu::{Target, items};
        let mut a = app();
        a.run("solid.box", json!({"length": 10, "width": 10, "height": 10, "body_name": "Box"})).unwrap();
        a.run("sketch.create", json!({"plane": "XY"})).unwrap();
        let id = a.session.active_sketch.unwrap();
        a.finish_sketch();
        let key = |a: &SolveApp, target: &Target, id: &str| items(a, target).into_iter().find(|i| i.id == id).map(|i| i.shortcut).unwrap();
        let bodies = Target::Body { name: "Box".into() };
        let sketch = Target::Sketch { id };
        assert_eq!(key(&a, &bodies, "ui.moveBodies"), "M");
        assert_eq!(key(&a, &sketch, "ui.extrudeSketch"), "E");
        assert_eq!(crate::sketch_palette::construction_label(&a), "Construction mode (X)");
        choose(&mut a, ONSHAPE_SET);
        assert_eq!(key(&a, &bodies, "ui.moveBodies"), "");
        assert_eq!(key(&a, &bodies, "ui.appearance"), "");
        assert_eq!(key(&a, &sketch, "ui.extrudeSketch"), "Shift+E");
        assert_eq!(crate::sketch_palette::construction_label(&a), "Construction mode (Q)");
        bind(&mut a, "sketch.construction", "", false).unwrap();
        assert_eq!(crate::sketch_palette::construction_label(&a), "Construction mode");
    }

    /// Digit-row keys are read by position, Shift or not; a key saved under what the layout typed
    /// (German Shift+7 was `Shift+Slash`) still runs.
    #[test]
    fn digit_row_keys_by_position_and_the_old_names() {
        use egui::Key;
        assert_eq!(layout_key(Key::Slash, Some(Key::Num7)), Key::Num7);
        assert_eq!(layout_alias(Key::Slash, Some(Key::Num7)), Some(Key::Slash));
        assert_eq!(layout_alias(Key::Num7, Some(Key::Num7)), None);
        assert_eq!(layout_alias(Key::A, Some(Key::Q)), None, "only the digit row");
        assert_eq!(layout_key(Key::ArrowUp, Some(Key::Num8)), Key::ArrowUp, "the numpad without Num Lock");
        assert_eq!(layout_alias(Key::ArrowUp, Some(Key::Num8)), None);
        let mut h = Harness::new();
        h.app.home.open = false;
        let press = |h: &mut Harness, key: Key, modifiers: egui::Modifiers| {
            for pressed in [true, false] {
                h.app.synthetic.push(egui::Event::Key { key, physical_key: Some(Key::Num7), pressed, repeat: false, modifiers });
            }
            h.frames(2);
            let started = h.app.dialog.is_some();
            h.step(&json!({"key": "Escape"})).unwrap();
            h.frames(1);
            started
        };
        h.app.run("solid.box", json!({"length": 10, "width": 10, "height": 10})).unwrap();
        h.app.keymap.load(&json!({"solid.shell": "Shift+Slash"}));
        assert!(press(&mut h, Key::Slash, egui::Modifiers::SHIFT), "the old name of Shift+7 still runs");
        h.app.keymap.load(&json!({"solid.shell": "7"}));
        assert!(press(&mut h, Key::Slash, egui::Modifiers::NONE), "the 7 key unshifted, whatever the layout types");
    }
}

#[cfg(test)]
#[path = "sketch_shortcut_tests.rs"]
mod sketch_shortcut_tests;

#[cfg(test)]
mod card_tests {
    use super::*;
    use crate::scenario::{self, Harness};

    #[test]
    fn shortcut_card_captures_clears_resets_and_esc_cancels_capture_only() {
        let mut h = Harness::new();
        h.app.home.open = false;
        h.app.keymap.filter = "solid.extrude".into();
        h.call("ui.prefs", json!({"open":"Keyboard Shortcuts"}));
        for (action, expected) in [("capture", "Shift+E"), ("clear", ""), ("reset", "E")] {
            h.step(&json!({"click":{"handle":format!("keymap:{action}:solid.extrude")}})).unwrap();
            if action == "capture" {
                h.step(&json!({"key":"E","shift":true})).unwrap();
            }
            assert_eq!(effective(&h.app, "solid.extrude").unwrap_or_default(), expected);
        }
        h.step(&json!({"click":{"handle":"keymap:capture:solid.extrude"}})).unwrap();
        h.step(&json!({"key":"Escape"})).unwrap();
        assert!(h.app.prefs_window.open && h.app.keymap.capturing.is_none());
        h.step(&json!({"key":"Escape"})).unwrap();
        assert!(!h.app.prefs_window.open);
    }

    #[test]
    fn help_shortcuts_card_keeps_rebinding_read_only() {
        let mut h = Harness::new();
        h.app.home.open = false;
        h.app.keymap.filter = "solid.extrude".into();
        h.call("ui.help", json!({"item":"shortcuts"}));
        assert!(h.app.keymap.read_only);
        assert!(scenario::handle_at("keymap:filter").is_some());
        for action in ["capture", "clear", "reset"] {
            assert!(scenario::handle_at(&format!("keymap:{action}:solid.extrude")).is_none());
        }
        assert_eq!(effective(&h.app, "solid.extrude").as_deref(), Some("E"));
    }
}
