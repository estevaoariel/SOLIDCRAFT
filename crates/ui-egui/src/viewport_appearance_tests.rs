use super::build_scene;
use crate::{Services, SolveApp};
use serde_json::json;
use solidcraft_engine::{Session, doc, geom::Vec3, kernel};

#[test]
fn placed_step_body_uses_imported_opacity_until_a_design_look_overrides_it() {
    let body = kernel::box_solid(Vec3::ZERO, Vec3::new(10.0, 10.0, 10.0))
        .unwrap()
        .with_color(Some([0.2, 0.6, 0.8]))
        .with_paint(Some(kernel::Paint { opacity: 0.4, faces: Vec::new() }));
    let step =
        kernel::step_export_bodies(&[kernel::ExportBody { name: "Imported glass".into(), body: &body, color: body.color() }], &Default::default())
            .unwrap();
    let mut document = doc::Document::default();
    let component = document.add_component(Some("Imported part"), 0).unwrap();
    let feature = document
        .add_feature(doc::FeatureKind::Import { file: "glass.step".into(), step: step.into(), components: Vec::new(), skip: Vec::new() }, None)
        .unwrap();
    document.features.iter_mut().find(|f| f.id == feature).unwrap().component = component;
    let mut session = Session::new(document);
    let source = session.model.state().bodies[0].name.clone();
    assert!((session.doc.model_body_opacity(&session.model.state().bodies[0]) - 0.4).abs() < 1e-6);
    session.execute("occurrence.copy", &json!({"component":component,"translate":[30,0,0]})).unwrap();
    let mut app = SolveApp::new(session, Services::default());
    app.ui.ground_shadow = false;
    app.ui.show_origin = false;
    let placed = app.session.world_state().bodies.iter().find(|b| b.name != source).unwrap().name.clone();
    app.session.selection = vec![solidcraft_engine::Sel::Body { name: placed }];
    app.start("appearance.assign");
    let crate::dialogs::Kind::Appearance(appearance) = &app.dialog.as_ref().unwrap().kind else { panic!("appearance dialog") };
    assert!((appearance.opacity - 0.4).abs() < 1e-6);
    assert_eq!(appearance.color, [51, 153, 204]);
    crate::dialogs::cancel(&mut app);
    let scene = build_scene(&app).merged();
    assert!(!scene.glass.is_empty());
    assert!(
        scene.tris.is_empty(),
        "opaque triangles {}, bodies {:?}",
        scene.tris.len(),
        app.session.world_state().bodies.iter().map(|b| (&b.name, b.body.paint(), &b.imported)).collect::<Vec<_>>()
    );
    let (vertices, remainder) = scene.glass.as_chunks::<{ crate::gpu::TRI_SIZE }>();
    assert!(remainder.is_empty());
    assert!(vertices.iter().all(|v| v[27] == 102));
    app.run("appearance.assign", json!({"bodies":[source],"color":"#336699","opacity":0.7})).unwrap();
    let scene = build_scene(&app);
    let (vertices, remainder) = scene.glass.as_chunks::<{ crate::gpu::TRI_SIZE }>();
    assert!(remainder.is_empty());
    assert!(vertices.iter().all(|v| v[27] == 179));
    app.run("appearance.assign", json!({"bodies":[source]})).unwrap();
    let scene = build_scene(&app);
    let (vertices, remainder) = scene.glass.as_chunks::<{ crate::gpu::TRI_SIZE }>();
    assert!(remainder.is_empty());
    assert!(vertices.iter().all(|v| v[27] == 102));
}

#[test]
fn inspect_menu_toggles_component_colors_without_a_dialog_and_refreshes_scene() {
    let mut h = crate::scenario::Harness::new();
    h.frames(3);
    let command = solidcraft_engine::find_command("inspect.colored_components").unwrap();
    let info = command.info(&h.app.session);
    assert_eq!((info.label, info.icon), ("Enable Component Colors", "colored_components"));
    h.step(&json!({"click": {"handle": "tab:INSPECT"}})).unwrap();
    h.frames(2);
    let original_key = super::scene_key(&h.app);
    h.step(&json!({"click": {"handle": "panel:INSPECT:CHECK"}})).unwrap();
    h.frames(2);
    assert!(h.widget("Enable Component Colors").is_some());
    h.step(&json!({"click": {"handle": "dropdown:inspect.colored_components"}})).unwrap();
    assert!(h.app.session.colored_components.is_some());
    assert!(h.app.dialog.is_none());
    assert_ne!(super::scene_key(&h.app), original_key);
    let info = command.info(&h.app.session);
    assert_eq!((info.label, info.icon), ("Disable Component Colors", "colored_components_off"));
    h.step(&json!({"click": {"handle": "panel:INSPECT:CHECK"}})).unwrap();
    h.frames(2);
    assert!(h.widget("Disable Component Colors").is_some());
    h.step(&json!({"click": {"handle": "dropdown:inspect.colored_components"}})).unwrap();
    assert!(h.app.session.colored_components.is_none());
    let info = command.info(&h.app.session);
    assert_eq!((info.label, info.icon), ("Enable Component Colors", "colored_components"));
}
