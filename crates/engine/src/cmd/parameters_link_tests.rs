use std::f64::consts::PI;
use std::path::{Path, PathBuf};

use serde_json::{Value, json};

use crate::Session;

fn run(s: &mut Session, id: &str, p: Value) -> Value {
    match s.execute(id, &p) {
        Ok(v) => v,
        Err(e) => panic!("{id} {p}: {e}"),
    }
}

fn err(s: &mut Session, id: &str, p: Value) -> String {
    match s.execute(id, &p) {
        Ok(v) => panic!("{id} {p} should fail, gave {v}"),
        Err(e) => e.to_string(),
    }
}

fn volume(s: &mut Session) -> f64 {
    run(s, "inspect.measure", json!({}))["total"]["volume_mm3"].as_f64().unwrap_or(f64::NAN)
}

fn close(a: f64, b: f64) -> bool {
    (a - b).abs() <= 1e-6 * b.abs().max(1.0)
}

/// A measured volume against an analytic one: the measure is within 0.1% of a cylinder's volume,
/// while the changes these tests make move it by 5% or more.
fn near(a: f64, b: f64) -> bool {
    (a - b).abs() <= 1e-3 * b.abs()
}

fn row(s: &mut Session, name: &str) -> Value {
    let l = run(s, "parameters.list", json!({}));
    l["parameters"].as_array().and_then(|a| a.iter().find(|p| p["name"] == name).cloned()).unwrap_or(Value::Null)
}

/// A fresh folder for one test, with `std/` and `parts/` in it.
fn folder(test: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("solidcraft-param-link-{test}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(dir.join("std")).unwrap();
    std::fs::create_dir_all(dir.join("parts")).unwrap();
    dir
}

/// Rows of a standards design's parameter table (CSV as `parameters.import` reads it): plain
/// values, expressions of other rows and an angle.
const STANDARD: &str = "name,unit,expression,comment
Clr_Slide,mm,0.20 mm,\"Sliding fit per side (keys, cradles)\"
Key_W,mm,4.0 mm,R/X divot width
Key_Ang,deg,45 deg,Ramp angle on key ends
KeyM_W,mm,Key_W - 2 * Clr_Slide,R/X male key width
Q_InsHole,mm,8.2 mm,1/4-20 insert hole diameter
Q_InsLen,mm,9.5 mm,1/4-20 insert length
Q_InsDepth,mm,Q_InsLen + 1.4 mm,1/4-20 insert hole depth
";

/// The standards design, saved at `path`, still open for editing.
fn standard(path: &Path) -> Session {
    let mut s = Session::default();
    run(&mut s, "parameters.import", json!({"text": STANDARD}));
    run(&mut s, "file.save", json!({"path": path}));
    s
}

/// An insert hole's worth of material: a cylinder Q_InsHole across and Q_InsDepth tall.
fn insert_volume(hole: f64, depth: f64) -> f64 {
    PI * (hole / 2.0).powi(2) * depth
}

#[test]
fn a_part_uses_the_standards_parameters_and_follows_them_on_update() {
    let dir = folder("follow");
    let std_file = dir.join("std").join("CC-STD-Interface.solidcraft");
    let part_file = dir.join("parts").join("core.solidcraft");
    let mut std = standard(&std_file);
    // Given a part that links all of the standard's parameters and uses two of them.
    let mut part = Session::default();
    run(&mut part, "file.save", json!({"path": part_file}));
    let linked = run(&mut part, "parameters.link_design", json!({"path": std_file, "scope": "all"}));
    assert_eq!(linked["source"], "../std/CC-STD-Interface.solidcraft", "{linked}");
    assert_eq!(linked["parameters"].as_array().map(Vec::len), Some(7), "{linked}");
    run(&mut part, "solid.cylinder", json!({"diameter": "Q_InsHole", "height": "Q_InsDepth"}));
    assert!(near(volume(&mut part), insert_volume(8.2, 10.9)), "{}", volume(&mut part));
    // The linked rows carry their values in their own units, worked out in the standard.
    let r = row(&mut part, "KeyM_W");
    assert_eq!(r["source"], "linked", "{r}");
    assert_eq!(r["link"], "../std/CC-STD-Interface.solidcraft", "{r}");
    assert_eq!(r["expression"], "3.6 mm", "{r}");
    let r = row(&mut part, "Key_Ang");
    assert!(r["expression"] == "45 deg" && close(r["value"].as_f64().unwrap_or(f64::NAN), 45.0), "{r}");
    assert_eq!(row(&mut part, "Q_InsDepth")["used_by"], json!(["d2"]));
    let links = run(&mut part, "parameters.links", json!({}));
    assert_eq!(links["sources"][0]["status"], "up_to_date", "{links}");
    // When the standard's insert hole grows, the part says so and takes it on update.
    run(&mut std, "parameters.change", json!({"name": "Q_InsHole", "expression": "8.5 mm"}));
    run(&mut std, "file.save", json!({"path": std_file}));
    let links = run(&mut part, "parameters.links", json!({}));
    assert_eq!(links["sources"][0]["status"], "out_of_date", "{links}");
    assert_eq!(links["sources"][0]["changed"], json!(["Q_InsHole"]), "{links}");
    assert!(near(volume(&mut part), insert_volume(8.2, 10.9)), "the part keeps its values until it is updated");
    let up = run(&mut part, "parameters.update_links", json!({}));
    assert_eq!(up["updated"][0]["changed"], json!([{"name": "Q_InsHole", "from": "8.2 mm", "to": "8.5 mm"}]), "{up}");
    assert!(near(volume(&mut part), insert_volume(8.5, 10.9)), "{}", volume(&mut part));
    assert_eq!(run(&mut part, "parameters.links", json!({}))["sources"][0]["status"], "up_to_date");
    assert_eq!(run(&mut part, "parameters.update_links", json!({}))["unchanged"], json!(["../std/CC-STD-Interface.solidcraft"]));
    // An expression change upstream arrives as its new value.
    run(&mut std, "parameters.change", json!({"name": "Q_InsLen", "expression": "8 mm"}));
    run(&mut std, "file.save", json!({"path": std_file}));
    run(&mut part, "parameters.update_links", json!({"source": "CC-STD-Interface"}));
    assert!(near(volume(&mut part), insert_volume(8.5, 9.4)), "{}", volume(&mut part));
    // Undo takes the update back.
    run(&mut part, "edit.undo", json!({}));
    assert!(near(volume(&mut part), insert_volume(8.5, 10.9)), "{}", volume(&mut part));
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn linked_parameters_are_read_only_here() {
    let dir = folder("read-only");
    let std_file = dir.join("std").join("standard.solidcraft");
    standard(&std_file);
    let mut part = Session::default();
    run(&mut part, "parameters.link_design", json!({"path": std_file, "scope": "all"}));
    run(&mut part, "parameters.add", json!({"name": "Local", "expression": "Q_InsHole * 2"}));
    let before = part.doc.clone();
    for (id, p) in [
        ("parameters.change", json!({"name": "Q_InsHole", "expression": "9 mm"})),
        ("parameters.change", json!({"changes": {"Q_InsHole": "9 mm"}})),
        ("parameters.change", json!({"name": "Q_InsHole", "new_name": "Hole"})),
        ("parameters.change", json!({"delete": "Q_InsHole"})),
        ("parameters.rename", json!({"name": "Q_InsHole", "new_name": "Hole"})),
        ("parameters.delete", json!({"name": "Q_InsHole"})),
        ("parameters.comment", json!({"name": "Q_InsHole", "comment": "mine"})),
        ("parameters.add", json!({"name": "Q_InsHole", "expression": "9 mm"})),
        ("parameters.import", json!({"text": "name,expression\nQ_InsHole,9 mm\n"})),
    ] {
        let e = err(&mut part, id, p.clone());
        assert!(e.contains("linked design") && e.contains("standard.solidcraft"), "{id} {p}: {e}");
    }
    // A local parameter can't take a linked name either.
    let e = err(&mut part, "parameters.rename", json!({"name": "Local", "new_name": "Key_W"}));
    assert!(e.contains("already"), "{e}");
    assert_eq!(part.doc, before);
    // Linked parameters can still be favourites here, and local ones use them.
    run(&mut part, "parameters.favorite", json!({"name": "Q_InsHole"}));
    assert!(close(row(&mut part, "Local")["value"].as_f64().unwrap_or(f64::NAN), 16.4));
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn only_favourites_come_in_unless_all_are_asked_for() {
    let dir = folder("favourites");
    let std_file = dir.join("std").join("standard.solidcraft");
    let mut std = standard(&std_file);
    let mut part = Session::default();
    let e = err(&mut part, "parameters.link_design", json!({"path": std_file}));
    assert!(e.contains("favourite") && e.contains("scope"), "{e}");
    run(&mut std, "parameters.favorite", json!({"name": "Q_InsHole"}));
    run(&mut std, "parameters.favorite", json!({"name": "Q_InsDepth"}));
    run(&mut std, "file.save", json!({"path": std_file}));
    let linked = run(&mut part, "parameters.link_design", json!({"path": std_file}));
    assert_eq!(linked["scope"], "favorites");
    assert_eq!(linked["parameters"], json!(["Q_InsHole", "Q_InsDepth"]), "{linked}");
    // Its value is worked out in the standard, from a parameter that doesn't come in.
    assert_eq!(row(&mut part, "Q_InsDepth")["expression"], "10.9 mm");
    assert_eq!(row(&mut part, "Q_InsLen"), Value::Null);
    // A new favourite there comes in on update.
    run(&mut std, "parameters.favorite", json!({"name": "Key_W"}));
    run(&mut std, "file.save", json!({"path": std_file}));
    let up = run(&mut part, "parameters.update_links", json!({}));
    assert_eq!(up["updated"][0]["added"], json!(["Key_W"]), "{up}");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn a_name_this_design_already_uses_refuses_the_link_and_the_update() {
    let dir = folder("clash");
    let std_file = dir.join("std").join("standard.solidcraft");
    let mut std = standard(&std_file);
    let mut part = Session::default();
    run(&mut part, "parameters.add", json!({"name": "Q_InsHole", "expression": "7 mm"}));
    run(&mut part, "parameters.add", json!({"name": "Wall", "expression": "2 mm"}));
    let before = part.doc.clone();
    let e = err(&mut part, "parameters.link_design", json!({"path": std_file, "scope": "all"}));
    assert!(e.contains("Q_InsHole") && !e.contains("Key_W"), "{e}");
    assert_eq!(part.doc, before);
    // Once linked, a new parameter there with a name used here keeps the part as it was.
    run(&mut part, "parameters.delete", json!({"name": "Q_InsHole"}));
    run(&mut part, "parameters.link_design", json!({"path": std_file, "scope": "all"}));
    run(&mut std, "parameters.add", json!({"name": "Wall", "expression": "3 mm"}));
    run(&mut std, "file.save", json!({"path": std_file}));
    let before = part.doc.clone();
    let e = err(&mut part, "parameters.update_links", json!({}));
    assert!(e.contains("Wall"), "{e}");
    assert_eq!(part.doc, before);
    // A second link may not bring a name the first one has.
    let other = dir.join("std").join("other.solidcraft");
    standard(&other);
    let e = err(&mut part, "parameters.link_design", json!({"path": other, "scope": "all"}));
    assert!(e.contains("Q_InsHole"), "{e}");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn a_parameter_the_part_uses_cant_vanish_on_update() {
    let dir = folder("vanish");
    let std_file = dir.join("std").join("standard.solidcraft");
    let mut std = standard(&std_file);
    let mut part = Session::default();
    run(&mut part, "parameters.link_design", json!({"path": std_file, "scope": "all"}));
    run(&mut part, "solid.cylinder", json!({"diameter": "Q_InsHole", "height": 5}));
    run(&mut std, "parameters.rename", json!({"name": "Q_InsHole", "new_name": "Q_Hole"}));
    run(&mut std, "file.save", json!({"path": std_file}));
    let before = part.doc.clone();
    let e = err(&mut part, "parameters.update_links", json!({}));
    assert!(e.contains("Q_InsHole") && e.contains("no longer"), "{e}");
    assert_eq!(part.doc, before);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn a_part_opens_with_its_last_values_when_the_standard_is_gone() {
    let dir = folder("missing");
    let std_file = dir.join("std").join("standard.solidcraft");
    let part_file = dir.join("parts").join("core.solidcraft");
    standard(&std_file);
    let mut part = Session::default();
    run(&mut part, "parameters.link_design", json!({"path": std_file, "scope": "all"}));
    run(&mut part, "solid.cylinder", json!({"diameter": "Q_InsHole", "height": "Q_InsDepth"}));
    // Linked before the part had a file: the path is kept whole, and saving makes it relative.
    run(&mut part, "file.save", json!({"path": part_file}));
    assert_eq!(part.doc.param_sources[0].path, "../std/standard.solidcraft");
    // Reopened, the part is as it was.
    let mut back = Session::default();
    let opened = run(&mut back, "doc.open", json!({"path": part_file}));
    assert!(opened.get("warnings").is_none(), "{opened}");
    assert!(near(volume(&mut back), insert_volume(8.2, 10.9)));
    assert_eq!(row(&mut back, "Q_InsHole")["source"], "linked");
    // Without the standard it still opens on the values last read, and says the file is missing.
    std::fs::remove_file(&std_file).unwrap();
    let mut gone = Session::default();
    let opened = run(&mut gone, "doc.open", json!({"path": part_file}));
    assert!(opened["warnings"][0].as_str().is_some_and(|w| w.contains("missing")), "{opened}");
    assert!(near(volume(&mut gone), insert_volume(8.2, 10.9)));
    assert_eq!(run(&mut gone, "parameters.links", json!({}))["sources"][0]["status"], "missing");
    let before = gone.doc.clone();
    let e = err(&mut gone, "parameters.update_links", json!({}));
    assert!(e.contains("standard.solidcraft"), "{e}");
    assert_eq!(gone.doc, before);
    // Saved elsewhere, the link still names the same file.
    std::fs::create_dir_all(dir.join("parts").join("sub")).unwrap();
    run(&mut gone, "file.save", json!({"path": dir.join("parts").join("sub").join("core.solidcraft")}));
    assert_eq!(gone.doc.param_sources[0].path, "../../std/standard.solidcraft");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn a_design_cant_link_itself_or_close_a_loop() {
    let dir = folder("loop");
    let a_file = dir.join("std").join("a.solidcraft");
    let b_file = dir.join("parts").join("b.solidcraft");
    let mut a = standard(&a_file);
    let e = err(&mut a, "parameters.link_design", json!({"path": a_file, "scope": "all"}));
    assert!(e.contains("own"), "{e}");
    // b links a; a may not then link b, directly or through c.
    let mut b = Session::default();
    run(&mut b, "parameters.add", json!({"name": "B_only", "expression": "1 mm"}));
    run(&mut b, "file.save", json!({"path": b_file}));
    run(&mut b, "parameters.link_design", json!({"path": a_file, "scope": "all"}));
    run(&mut b, "file.save", json!({"path": b_file}));
    let e = err(&mut a, "parameters.link_design", json!({"path": b_file, "scope": "all"}));
    assert!(e.contains("loop"), "{e}");
    let c_file = dir.join("parts").join("c.solidcraft");
    let mut c = Session::default();
    run(&mut c, "parameters.add", json!({"name": "C_only", "expression": "1 mm"}));
    run(&mut c, "file.save", json!({"path": c_file}));
    run(&mut c, "parameters.link_design", json!({"path": b_file, "scope": "all"}));
    run(&mut c, "file.save", json!({"path": c_file}));
    let e = err(&mut a, "parameters.link_design", json!({"path": c_file, "scope": "all"}));
    assert!(e.contains("loop"), "{e}");
    assert!(a.doc.param_sources.is_empty());
    // Linking the same design twice is refused too.
    let e = err(&mut b, "parameters.link_design", json!({"path": a_file, "scope": "all"}));
    assert!(e.contains("already"), "{e}");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn break_link_keeps_the_values_as_user_parameters() {
    let dir = folder("break");
    let std_file = dir.join("std").join("standard.solidcraft");
    standard(&std_file);
    let mut part = Session::default();
    run(&mut part, "parameters.link_design", json!({"path": std_file, "scope": "all"}));
    run(&mut part, "solid.cylinder", json!({"diameter": "Q_InsHole", "height": "Q_InsDepth"}));
    let broken = run(&mut part, "parameters.break_link", json!({}));
    assert_eq!(broken["parameters"].as_array().map(Vec::len), Some(7), "{broken}");
    assert!(part.doc.param_sources.is_empty());
    let r = row(&mut part, "Q_InsDepth");
    assert_eq!(
        (r["source"].as_str(), r["expression"].as_str(), r["comment"].as_str()),
        (Some("user"), Some("10.9 mm"), Some("1/4-20 insert hole depth"))
    );
    assert!(near(volume(&mut part), insert_volume(8.2, 10.9)));
    // They are this design's own now.
    run(&mut part, "parameters.change", json!({"name": "Q_InsHole", "expression": "6 mm"}));
    assert!(near(volume(&mut part), insert_volume(6.0, 10.9)));
    assert!(err(&mut part, "parameters.break_link", json!({})).contains("links no parameters"));
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn hostile_link_params_never_panic() {
    let mut s = Session::default();
    for (id, p) in [
        ("parameters.link_design", json!({})),
        ("parameters.link_design", json!({"path": 7})),
        ("parameters.link_design", json!({"path": ""})),
        ("parameters.link_design", json!({"path": "x".repeat(5000)})),
        ("parameters.link_design", json!({"path": "/no/such/design.solidcraft"})),
        ("parameters.link_design", json!({"path": "/no/such/design.solidcraft", "scope": "some"})),
        ("parameters.link_design", json!({"path": "/no/such/design.solidcraft", "scope": 3})),
        ("parameters.update_links", json!({})),
        ("parameters.update_links", json!({"source": 5})),
        ("parameters.update_links", json!({"source": ""})),
        ("parameters.break_link", json!({"source": "nothing"})),
        ("parameters.break_link", json!({"source": null})),
    ] {
        assert!(s.execute(id, &p).is_err(), "{id} {p}");
    }
    assert_eq!(run(&mut s, "parameters.links", json!({})), json!({"sources": []}));
    assert!(s.doc.param_sources.is_empty() && s.doc.params.is_empty());
}
