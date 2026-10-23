use serde_json::json;

use super::*;
use crate::context_menu::{self, Target};
use crate::scenario::Harness;

fn harness_with_body() -> Harness {
    let mut h = Harness::new();
    h.app.run("solid.box", json!({"length": 40, "width": 20, "height": 10, "body_name": "Duct"})).unwrap();
    h.app.session.mark_saved();
    h.frames(3);
    h
}

/// The design as saved, plus its history: what a workspace switch must never change.
fn design_of(h: &Harness) -> (String, usize, usize, bool) {
    let s = &h.app.session;
    (serde_json::to_string(&*s.doc).unwrap(), s.undo.len(), s.redo.len(), s.is_dirty())
}

fn workspace_item<'a>(items: &'a [context_menu::Item], label: &str) -> &'a context_menu::Item {
    items.iter().find(|i| i.label == label).unwrap_or_else(|| panic!("no `{label}` in the workspace menu"))
}

#[test]
fn ids_tabs_and_workspaces_round_trip() {
    for w in Workspace::ALL {
        assert_eq!(Workspace::from_id(w.id()), Some(w));
        assert_eq!(Workspace::of_tab(w.tab()), w);
    }
    assert_eq!(Workspace::of_tab("SURFACE"), Workspace::Design);
    assert_eq!(Workspace::of_tab("SKETCH"), Workspace::Design);
    assert_eq!(Workspace::from_id("animation"), None);
}

#[test]
fn the_simulation_menu_item_is_enabled_like_render_and_marks_the_current_workspace() {
    let mut h = harness_with_body();
    let items = context_menu::items(&h.app, &Target::Workspace);
    assert!(workspace_item(&items, "Simulation").enabled);
    assert_eq!(workspace_item(&items, "Simulation").id, "ui.workspace.simulation");
    assert_eq!(workspace_item(&items, "Design").shortcut, "current");
    for later in ["Animation", "Manufacture", "Drawing"] {
        assert!(!workspace_item(&items, later).enabled, "{later} is still to come");
    }
    assert_eq!(items.iter().filter(|i| i.label == "Simulation").count(), 1, "no second, greyed Simulation entry");

    // A sketch being edited holds Render and Simulation back alike.
    h.app.run("sketch.create", json!({"plane": "XY"})).unwrap();
    let items = context_menu::items(&h.app, &Target::Workspace);
    assert!(!workspace_item(&items, "Simulation").enabled);
    assert!(!workspace_item(&items, "Render").enabled);
    assert!(workspace_item(&items, "Design").enabled);
    assert!(enter(&mut h.app, Workspace::Simulation).is_err());
    assert_eq!(current(&h.app), Workspace::Design);
}

#[test]
fn design_to_simulation_and_back_leaves_the_design_untouched() {
    let mut h = harness_with_body();
    let before = design_of(&h);
    let camera = h.app.cam;
    let at = crate::scenario::rect_of("workspace_switcher").unwrap().center();
    h.step(&json!({"click": [at.x, at.y]})).unwrap();
    h.step(&json!({"click": {"handle": "menu:ui.workspace.simulation"}})).unwrap();
    assert_eq!(current(&h.app), Workspace::Simulation);
    assert_eq!(h.app.ui.tab, "SIMULATION");
    assert!(crate::scenario::rect_of("toolbar_tab:SIMULATION").is_some(), "the SIMULATION tab is shown");
    assert!(crate::scenario::rect_of("toolbar_tab:MODEL").is_none(), "Design's tabs are not");
    assert_eq!(design_of(&h), before);

    let reply = h.call("ui.workspace", json!({"workspace": "design"}));
    assert_eq!(reply["result"]["workspace"], "design", "{reply}");
    h.frames(2);
    assert_eq!(current(&h.app), Workspace::Design);
    assert_eq!(h.app.ui.tab, "MODEL");
    assert_eq!(h.app.cam, camera);
    assert_eq!(design_of(&h), before);
}

#[test]
fn control_channel_names_the_simulation_workspace() {
    let mut h = harness_with_body();
    let reply = h.call("ui.workspace", json!({"workspace": "simulation"}));
    assert_eq!(reply["result"]["workspace"], "simulation", "{reply}");
    assert_eq!(h.call("ui.workspace", json!({}))["result"]["workspace"], "simulation");
    let wrong = h.call("ui.workspace", json!({"workspace": "animation"}));
    assert_eq!(wrong["error"], "workspace must be design, render or simulation", "{wrong}");
    // `ui.set {tab}` switches workspaces as the menu does, both ways.
    h.call("ui.set", json!({"tab": "SURFACE"}));
    assert_eq!((current(&h.app), h.app.ui.tab.as_str()), (Workspace::Design, "SURFACE"));
    h.call("ui.set", json!({"tab": "SIMULATION"}));
    assert_eq!(current(&h.app), Workspace::Simulation);
}

#[test]
fn the_unbuilt_studies_open_their_docked_dialog_and_ok_reports_the_missing_solver() {
    let mut h = harness_with_body();
    let before = design_of(&h);
    enter(&mut h.app, Workspace::Simulation).unwrap();
    h.frames(2);
    for (id, title) in [("sim.aero.new", "AERODYNAMICS STUDY"), ("sim.gas.new", "GAS DYNAMICS STUDY")] {
        h.step(&json!({"start": id})).unwrap();
        h.frames(3);
        let d = h.app.dialog.as_ref().unwrap_or_else(|| panic!("{id} opens a dialog"));
        assert_eq!(crate::dialogs::title(&d.kind), title);
        h.step(&json!({"expect": {"dialog_docked": {}}})).unwrap();
        assert!(h.widget(crate::i18n::tr("sim.solver_not_built")).is_some(), "{id}: the note says the solver is not built");
        // OK with nothing picked asks for the body; with the body picked the command refuses.
        h.step(&json!({"widget": "OK"})).unwrap();
        h.frames(2);
        let error = h.app.dialog.as_ref().and_then(|d| d.error.clone()).unwrap_or_default();
        assert!(error.contains("select the body to study"), "{id}: {error}");
        h.step(&json!({"click": {"world": [20, 10, 10]}})).unwrap();
        h.step(&json!({"widget": "OK"})).unwrap();
        h.frames(2);
        let refused = h.app.dialog.as_ref().and_then(|d| d.error.clone()).or_else(|| h.app.status.as_ref().map(|s| s.0.clone()));
        assert!(refused.as_deref().is_some_and(|e| e.contains("not supported yet")), "{id}: {refused:?}");
        crate::dialogs::cancel(&mut h.app);
        h.frames(2);
        assert!(h.app.dialog.is_none());
    }
    assert_eq!(design_of(&h), before, "an unbuilt study changes nothing");
}

/// The Airflow dialog end to end: a duct's body and its two rims picked, a set speed, OK. The
/// study runs in the background (the CPU reference here), the results panel shows its flows,
/// and the view draws the result inside see-through bodies. The design never changes.
#[test]
fn an_airflow_study_runs_from_its_dialog_and_shows_its_result() {
    let mut h = Harness::new();
    h.app.run("solid.box", json!({"length": 24, "width": 12, "height": 12, "body_name": "Duct"})).unwrap();
    h.app.run("solid.box", json!({"corner": [-1, 2, 2], "length": 26, "width": 8, "height": 8, "operation": "cut", "targets": ["Duct"]})).unwrap();
    h.app.session.mark_saved();
    h.frames(3);
    let before = design_of(&h);
    enter(&mut h.app, Workspace::Simulation).unwrap();
    h.step(&json!({"start": "sim.flow.new"})).unwrap();
    h.frames(3);
    assert_eq!(crate::dialogs::title(&h.app.dialog.as_ref().unwrap().kind), "AIRFLOW STUDY");
    for label in ["Inlets", "Outlets"] {
        assert!(h.widget(crate::i18n::tr_label(label)).is_some(), "the {label} picker shows");
    }
    // OK with nothing picked asks for the body, then for the inlets.
    h.step(&json!({"widget": "OK"})).unwrap();
    h.frames(2);
    assert!(h.app.dialog.as_ref().and_then(|d| d.error.clone()).unwrap_or_default().contains("select the body"));
    let st = h.app.session.world_state();
    let duct = st.body("Duct").unwrap();
    let rim = |x: f64| {
        let point = solidcraft_engine::geom::Vec3::new(x, 1.0, 6.0);
        let index = solidcraft_engine::doc::appearance::face_index_at(duct, point).unwrap();
        solidcraft_engine::Sel::Face { body: "Duct".into(), index, point }
    };
    {
        let d = h.app.dialog.as_mut().unwrap();
        d.inputs[0].items = vec![solidcraft_engine::Sel::Body { name: "Duct".into() }];
        d.inputs[1].items = vec![rim(0.0)];
        d.inputs[2].items = vec![rim(24.0)];
        let crate::dialogs::Kind::Sim(k) = &mut d.kind else { panic!("the study dialog") };
        k.flow.drive = crate::dialogs_sim::Drive::Speed;
        k.flow.speed = "0.05".into();
        k.flow.cell = "1".into();
        k.flow.pad = "4".into();
        k.flow.turbulence = false;
    }
    // A frame for the dialog to take the picks in (and drop the earlier error).
    h.frames(1);
    h.step(&json!({"widget": "OK"})).unwrap();
    // OK applies on a worker, so the dialog closes a few frames later.
    for _ in 0..600 {
        if h.app.dialog.is_none() {
            break;
        }
        h.frames(1);
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
    assert!(h.app.dialog.is_none(), "OK sets the study up and closes: {:?} {:?}", h.app.status, h.app.dialog.as_ref().and_then(|d| d.error.clone()));
    assert!(h.app.session.flow_studies().get("Airflow 1").is_some());
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(120);
    while h.app.session.flow_studies().get("Airflow 1").and_then(|s| s.result.as_ref()).is_none() {
        assert!(std::time::Instant::now() < deadline, "the study finishes");
        h.frames(1);
        std::thread::sleep(std::time::Duration::from_millis(20));
    }
    h.frames(3);
    assert_eq!(h.app.simulation.shown.as_deref(), Some("Airflow 1"), "a finished study is shown");
    assert!(h.widget(crate::i18n::tr("sim.results")).is_some(), "the results panel is up");
    assert!(crate::simulation_workspace::see_through(&h.app));
    let mut sc = crate::gpu::GpuScene::default();
    crate::simulation_workspace::draw(&h.app, &mut sc);
    assert!(!sc.tris.is_empty() && !sc.overlay.is_empty(), "a section plane and streamlines are drawn");
    // Turning both off draws nothing; leaving the workspace hides the result.
    h.app.simulation.display.section = None;
    h.app.simulation.display.streamlines = false;
    let mut none = crate::gpu::GpuScene::default();
    crate::simulation_workspace::draw(&h.app, &mut none);
    assert!(none.tris.is_empty() && none.lines.is_empty() && none.overlay.is_empty());
    enter(&mut h.app, Workspace::Design).unwrap();
    assert!(!crate::simulation_workspace::see_through(&h.app));
    assert_eq!(design_of(&h), before, "a study never touches the design");
}

#[test]
fn leaving_simulation_closes_an_open_study_dialog() {
    let mut h = harness_with_body();
    enter(&mut h.app, Workspace::Simulation).unwrap();
    h.step(&json!({"start": "sim.gas.new"})).unwrap();
    assert!(h.app.dialog.is_some());
    // A dialog is open, so Render waits; Design is always reachable and closes the study.
    assert!(enter(&mut h.app, Workspace::Render).is_err());
    enter(&mut h.app, Workspace::Design).unwrap();
    assert!(h.app.dialog.is_none());
    assert_eq!(current(&h.app), Workspace::Design);
}

#[test]
fn render_and_simulation_switch_directly_and_design_gets_its_camera_back() {
    let mut h = harness_with_body();
    let camera = h.app.cam;
    enter(&mut h.app, Workspace::Render).unwrap();
    enter(&mut h.app, Workspace::Simulation).unwrap();
    assert_eq!(current(&h.app), Workspace::Simulation);
    assert_eq!(h.app.cam, camera, "Simulation shows the Design view");
    enter(&mut h.app, Workspace::Render).unwrap();
    enter(&mut h.app, Workspace::Design).unwrap();
    assert_eq!(h.app.cam, camera);
}
