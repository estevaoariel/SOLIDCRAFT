//! Insert Part: the standard parts library (screws, bolts, nuts, washers, set screws, pins,
//! bearings, and the families of open part libraries) by family, size and length, with the
//! size's table values and a picture of the part.
//! Picking a hole (its wall or rim) seats the part in it with a rigid joint; without one the part
//! goes at the origin. **Fill hole stacks** puts one in every stack of holes lined up through
//! touching parts instead (`parts.fasten`), each sized to its holes and grip. The live preview
//! shows them placed.

use crate::ui_kit::{choice_row, text_row};
use egui::vec2;
use serde_json::{Value, json};
use solidcraft_engine::{Sel, Session};

use crate::SolveApp;
use crate::dialogs::{Kind, combo, form_row};
use crate::selection::{EDGES, FACES, SelInput};
use crate::theme::Tokens;

/// One family of the library.
#[derive(Clone, Debug, PartialEq)]
pub struct Family {
    pub id: String,
    pub name: String,
    pub standard: String,
    /// Size names and their table rows.
    pub sizes: Vec<(String, Value)>,
    pub lengths: Option<Vec<f64>>,
}

impl Family {
    /// The lengths size `size` comes in: a library family lists them per size (each with the
    /// grip it holds), a built-in one for the whole family.
    pub fn lengths_of(&self, size: usize) -> Option<Vec<f64>> {
        match self.sizes.get(size).and_then(|(_, row)| row.get("lengths")).and_then(Value::as_array) {
            Some(ls) => Some(ls.iter().filter_map(|l| l.get("length").and_then(Value::as_f64)).collect()),
            None => self.lengths.clone(),
        }
    }
}

/// The library as `parts.library` gives it.
pub fn families(v: &Value) -> Vec<Family> {
    v.get("families")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|f| {
            let id = f.get("family")?.as_str()?.to_string();
            let sizes = f
                .get("sizes")
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
                .filter_map(|s| Some((s.get("size")?.as_str().map(str::to_string).or_else(|| s.get("size").map(Value::to_string))?, s.clone())))
                .collect();
            Some(Family {
                name: f.get("name").and_then(Value::as_str).unwrap_or(&id).to_string(),
                standard: f.get("standard").and_then(Value::as_str).unwrap_or("").to_string(),
                sizes,
                lengths: f.get("lengths").and_then(Value::as_array).map(|a| a.iter().filter_map(Value::as_f64).collect()),
                id,
            })
        })
        .collect()
}

#[derive(Clone, Debug)]
pub struct Pt {
    pub family: usize,
    pub size: usize,
    /// Index into the family's lengths (none: the default length).
    pub length: Option<usize>,
    pub name: String,
    /// The (family, size, length) the picture shows (the texture is kept in egui's memory).
    pub picture: Option<(usize, usize, Option<usize>)>,
    /// The library, read once.
    pub lib: Option<Vec<Family>>,
    /// Fill every hole stack (`parts.fasten`) instead of inserting one part.
    pub fill: bool,
}

/// The dialog for Insert Part.
pub fn start(_app: &SolveApp, id: &str) -> Option<(Kind, Vec<SelInput>)> {
    matches!(id, "parts.fastener" | "parts.insert").then(|| {
        (
            Kind::Part(Pt { family: 0, size: 0, length: None, name: String::new(), picture: None, lib: None, fill: false }),
            vec![SelInput::new("Hole (optional)", FACES | EDGES, false)],
        )
    })
}

fn library(app: &mut SolveApp) -> Vec<Family> {
    app.session.execute("parts.library", &json!({})).map(|v| families(&v)).unwrap_or_default()
}

/// The insert command's parameters for the picks.
pub fn insert_params(lib: &[Family], k: &Pt, hole: Option<&Sel>) -> Result<Value, String> {
    let f = lib.get(k.family).ok_or("the parts library is empty")?;
    let (size, _) = f.sizes.get(k.size).ok_or("pick a size")?;
    let mut p = json!({"family": f.id, "size": size});
    if let Some(l) = k.length.and_then(|i| f.lengths_of(k.size).and_then(|ls| ls.get(i).copied())) {
        p["length"] = json!(l);
    }
    match hole {
        Some(Sel::Face { point, .. } | Sel::Edge { point, .. }) => p["at"] = json!([point.x, point.y, point.z]),
        _ => {
            p["point"] = json!([0.0, 0.0, 0.0]);
            p["direction"] = json!([0.0, 0.0, 1.0]);
        }
    }
    if !k.name.trim().is_empty() {
        p["name"] = json!(k.name.trim());
    }
    Ok(p)
}

/// The fill command's parameters: the family, each stack's size and length chosen by its holes
/// and grip.
pub fn fill_params(lib: &[Family], k: &Pt) -> Result<Value, String> {
    let f = lib.get(k.family).ok_or("the parts library is empty")?;
    Ok(json!({"family": f.id}))
}

/// The part libraries a picture of a part is built with: the open ones, then a library of the
/// families only the design keeps (a part inserted from a library that is not open now).
fn picture_libraries(session: &Session) -> Vec<solidcraft_engine::cmd::PartLibrary> {
    let mut libs = session.part_libraries.clone();
    let mut design: Vec<solidcraft_engine::doc::part_family::FastenerFamily> = Vec::new();
    for f in session.doc.parts.values().filter_map(|p| p.definition.as_ref()) {
        let known = libs.iter().any(|l| l.library.families.iter().any(|o| o.name == f.name)) || design.iter().any(|o| o.name == f.name);
        if !known {
            design.push(f.clone());
        }
    }
    if !design.is_empty() {
        let library = solidcraft_engine::doc::part_family::Library {
            format: solidcraft_engine::doc::part_family::FORMAT.into(),
            id: "design".into(),
            name: "Design".into(),
            provenance: None,
            families: design,
        };
        libs.push(solidcraft_engine::cmd::PartLibrary { path: String::new(), library });
    }
    libs
}

/// A picture of the part alone, built with `libraries` (see [`picture_libraries`]) so a
/// library family's part is drawn too.
fn picture(p: &Value, libraries: Vec<solidcraft_engine::cmd::PartLibrary>) -> Option<egui::ColorImage> {
    let mut s = Session::default();
    s.part_libraries = libraries;
    let mut q = p.clone();
    if let Value::Object(m) = &mut q {
        m.remove("at");
        m.insert("point".into(), json!([0.0, 0.0, 0.0]));
        m.insert("direction".into(), json!([0.0, 0.0, 1.0]));
    }
    s.execute("parts.insert", &q).ok()?;
    crate::home::thumbnail(&s)
}

/// The dialog's rows.
pub fn rows(app: &mut SolveApp, ui: &mut egui::Ui, k: &mut Pt, inputs: &[SelInput]) -> bool {
    let t = Tokens::get();
    let lib = match &k.lib {
        Some(l) => l.clone(),
        None => {
            let l = library(app);
            k.lib = Some(l.clone());
            l
        }
    };
    if lib.is_empty() {
        ui.vertical(|ui| {
            crate::ui_kit::text(ui, crate::i18n::tr_label("the parts library is not available"), crate::ui_kit::Text::Note("parts_unavailable"));
        });
        return false;
    }
    let names: Vec<String> =
        lib.iter().map(|f| if f.standard.is_empty() { f.name.clone() } else { format!("{} ({})", f.name, f.standard) }).collect();
    let refs: Vec<&str> = names.iter().map(String::as_str).collect();
    let before = k.family;
    choice_row(ui, "Type", "pt_family", &mut k.family, &refs);
    if k.family != before {
        k.size = 0;
        k.length = None;
    }
    let Some(f) = lib.get(k.family) else { return false };
    // Filling picks each stack's size and length from its holes and grip, so the Size and
    // Length rows (which it would not use) are left out while it is ticked.
    form_row(ui, "Fill hole stacks", |ui| crate::dialogs::check(ui, &mut k.fill));
    if k.fill {
        ui.vertical(|ui| {
            crate::ui_kit::text(
                ui,
                crate::i18n::tr_label("one in every hole stack, sized to its holes and grip"),
                crate::ui_kit::Text::Note("parts_fill"),
            );
        });
    } else {
        let sizes: Vec<&str> = f.sizes.iter().map(|s| s.0.as_str()).collect();
        size_rows(app, ui, f, k, &sizes);
        text_row(ui, "Name", &mut k.name, "automatic");
    }
    // The part, and its table values.
    let key = (k.family, k.size, k.length);
    let tex_id = egui::Id::new("sc_part_picture");
    if k.picture != Some(key) {
        let tex = insert_params(&lib, k, None)
            .ok()
            .and_then(|p| picture(&p, picture_libraries(&app.session)))
            .map(|img| ui.ctx().load_texture("sc_part_picture", img, egui::TextureOptions::LINEAR));
        ui.data_mut(|d| match tex {
            Some(t) => {
                d.insert_temp(tex_id, t);
            }
            None => d.remove::<egui::TextureHandle>(tex_id),
        });
        k.picture = Some(key);
    }
    ui.vertical(|ui| match ui.data(|d| d.get_temp::<egui::TextureHandle>(tex_id)) {
        Some(tx) => {
            ui.add(egui::Image::new(&tx).fit_to_exact_size(vec2(180.0, 112.0)).corner_radius(t.radius_sm));
        }
        None => {
            crate::ui_kit::text(ui, crate::i18n::tr_label("no picture"), crate::ui_kit::Text::Note("parts_picture"));
        }
    });
    if !k.fill
        && let Some((_, row)) = f.sizes.get(k.size)
        && let Some(o) = row.as_object()
    {
        // A library size's lengths are the Length row's choices already.
        for (key, v) in o.iter().filter(|(key, _)| !matches!(key.as_str(), "size" | "lengths")) {
            // Numbers in the length unit, text (a head style) as it is, without JSON quotes.
            let shown =
                v.as_f64().map(|x| crate::prefs::show_mm(app, x, 1)).or_else(|| v.as_str().map(str::to_string)).unwrap_or_else(|| v.to_string());
            form_row(ui, &key.replace('_', " "), |ui| {
                crate::ui_kit::text(ui, &shown, crate::ui_kit::Text::Cell);
            });
        }
    }
    if !k.fill && inputs.first().is_some_and(|i| i.items.is_empty()) {
        ui.vertical(|ui| {
            crate::ui_kit::text(
                ui,
                crate::i18n::tr_label("pick a hole to seat it, or OK places it at the origin"),
                crate::ui_kit::Text::Note("parts_placement"),
            );
        });
    }
    false
}

/// The Size row, and the Length row where the family lists lengths.
fn size_rows(app: &SolveApp, ui: &mut egui::Ui, f: &Family, k: &mut Pt, sizes: &[&str]) {
    let before = k.size;
    choice_row(ui, "Size", "pt_size", &mut k.size, sizes);
    if k.size != before && f.sizes.get(before).is_some_and(|(_, row)| row.get("lengths").is_some()) {
        k.length = None;
    }
    if let Some(ls) = &f.lengths_of(k.size) {
        let mut labels = vec!["Default".to_string()];
        labels.extend(ls.iter().map(|l| crate::prefs::show_mm(app, *l, 1)));
        let refs: Vec<&str> = labels.iter().map(String::as_str).collect();
        let mut i = k.length.map_or(0, |x| x + 1);
        form_row(ui, "Length", |ui| {
            combo(ui, "pt_len", &refs, &mut i);
            k.length = i.checked_sub(1);
        });
    }
}

/// The commands OK runs.
pub fn commands(app: &SolveApp, k: &Pt, inputs: &[SelInput]) -> Result<Vec<(String, Value)>, String> {
    let lib = match &k.lib {
        Some(l) => l.clone(),
        None => app.session.scratch().execute("parts.library", &json!({})).map(|v| families(&v)).unwrap_or_default(),
    };
    if k.fill {
        return Ok(vec![("parts.fasten".into(), fill_params(&lib, k)?)]);
    }
    let p = insert_params(&lib, k, inputs.first().and_then(|i| i.items.first()))?;
    // The toolbar's command when the engine has it (same parameters).
    let id = if solidcraft_engine::find_command("parts.fastener").is_some() { "parts.fastener" } else { "parts.insert" };
    Ok(vec![(id.into(), p)])
}

#[cfg(test)]
mod tests {
    use super::*;
    use solidcraft_engine::geom::Vec3;

    fn lib() -> Vec<Family> {
        families(&json!({"families": [
            {"family": "socket_head_cap_screw", "name": "Socket Head Cap Screw", "standard": "ISO 4762",
             "sizes": [{"size": "M5", "head_diameter": 8.5}, {"size": "M6", "head_diameter": 10}], "lengths": [10, 16, 20]},
            {"family": "bearing", "name": "Deep Groove Ball Bearing", "standard": "", "sizes": [{"size": "608", "bore": 8}], "lengths": null},
        ]}))
    }

    #[test]
    fn library_rows_parse() {
        let l = lib();
        assert_eq!(l.len(), 2);
        assert_eq!(l[0].sizes[1].0, "M6");
        assert_eq!(l[0].lengths.as_deref(), Some(&[10.0, 16.0, 20.0][..]));
        assert!(l[1].lengths.is_none());
    }

    /// A hole pick seats the part (`at`); without one it goes to the origin along Z.
    #[test]
    fn insert_params_for_picks() {
        let l = lib();
        let k = Pt { family: 0, size: 1, length: Some(2), name: String::new(), picture: None, lib: None, fill: false };
        let hole = Sel::Face { body: "Body1".into(), index: 3, point: Vec3::new(5.0, 0.0, 10.0) };
        let p = insert_params(&l, &k, Some(&hole)).unwrap();
        assert_eq!(p, json!({"family": "socket_head_cap_screw", "size": "M6", "length": 20.0, "at": [5.0, 0.0, 10.0]}));
        let k = Pt { family: 1, size: 0, length: None, name: "Front bearing".into(), picture: None, lib: None, fill: false };
        let p = insert_params(&l, &k, None).unwrap();
        assert_eq!(p["point"], json!([0.0, 0.0, 0.0]));
        assert_eq!(p["name"], "Front bearing");
        assert!(p.get("length").is_none());
        assert!(insert_params(&[], &k, None).is_err());
    }

    /// Fill hole stacks runs parts.fasten for the family, sized by each stack.
    #[test]
    fn fill_runs_fasten_for_the_family() {
        let l = lib();
        let k = Pt { family: 0, size: 1, length: Some(2), name: String::new(), picture: None, lib: None, fill: true };
        assert_eq!(fill_params(&l, &k).unwrap(), json!({"family": "socket_head_cap_screw"}));
        assert!(fill_params(&[], &k).is_err());
    }

    /// A library family's lengths come from the picked size.
    #[test]
    fn library_sizes_carry_their_own_lengths() {
        let l = families(&json!({"families": [
            {"family": "rivet_dome", "name": "Rivet", "standard": "", "source": "example.rivets", "lengths": [6, 8, 12],
             "sizes": [{"size": "4.0", "lengths": [{"length": 6, "grip": [1, 3]}, {"length": 8, "grip": [3, 5]}]},
                       {"size": "4.8", "lengths": [{"length": 12, "grip": [6, 8]}]}]},
        ]}));
        assert_eq!(l[0].lengths_of(0), Some(vec![6.0, 8.0]));
        assert_eq!(l[0].lengths_of(1), Some(vec![12.0]));
        let k = Pt { family: 0, size: 1, length: Some(0), name: String::new(), picture: None, lib: None, fill: false };
        assert_eq!(insert_params(&l, &k, None).unwrap()["length"], json!(12.0));
    }

    /// A library family's part is built for its picture with the open libraries, and with the
    /// design's copy once the library is closed.
    #[test]
    fn a_library_part_is_pictured_with_its_family() {
        let path = std::env::temp_dir().join(format!("solidcraft-picture-lib-{}.json", std::process::id()));
        let lib = json!({"format": "solidcraft-parts/1", "id": "example.rivets", "name": "Example rivets", "families": [
            {"kind": "fastener_family", "name": "rivet_dome", "label": "Rivet", "shape": "blind_rivet",
             "sizes": [{"size": "4.8", "d": 4.8, "head": {"d": 9.5, "h": 1.6}, "lengths": [{"length": 12, "grip": [6, 8]}]}]}]});
        std::fs::write(&path, lib.to_string()).unwrap();
        let mut s = Session::default();
        s.execute("parts.library.load", &json!({"path": path.to_string_lossy()})).unwrap();
        let _ = std::fs::remove_file(&path);
        let part = json!({"family": "rivet_dome", "size": "4.8", "point": [0, 0, 0], "direction": [0, 0, 1]});
        let builds = |libs| {
            let mut scratch = Session::default();
            scratch.part_libraries = libs;
            scratch.execute("parts.insert", &part).is_ok()
        };
        assert!(!builds(Vec::new()), "a bare session does not know the family");
        assert!(builds(picture_libraries(&s)));
        // Inserted, then the library closed: the design's copy is used.
        s.execute("parts.insert", &part).unwrap();
        s.part_libraries.clear();
        let libs = picture_libraries(&s);
        assert_eq!(libs.iter().map(|l| l.library.id.as_str()).collect::<Vec<_>>(), ["design"]);
        assert!(builds(libs));
    }
}
