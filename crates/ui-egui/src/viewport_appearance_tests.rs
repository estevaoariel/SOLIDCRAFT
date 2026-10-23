use super::build_scene;
use crate::{Services, SolveApp};
use serde_json::json;
use solidcraft_engine::{Session, doc, geom::Vec3, kernel};

#[test]
fn a_finished_sketch_on_a_face_draws_a_halo_under_its_own_curves_only() {
    use crate::theme::{self, ColorVision, Tokens};

    let mut app = SolveApp::new(Session::default(), Services::default());
    app.run("solid.box", json!({"length":40,"width":30,"height":20})).unwrap();
    app.run("sketch.create", json!({"plane":{"face":[20,15,20]},"project_edges":true})).unwrap();
    app.run("sketch.rectangle.two_point", json!({"p0":[-5,-5],"p1":[5,5]})).unwrap();
    app.run("sketch.circle.center", json!({"center":[0,0],"radius":3})).unwrap();
    app.run("sketch.line", json!({"points":[[-4,0],[4,0]]})).unwrap();
    let rect = egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(800.0, 600.0));
    let original_dark = theme::is_dark();
    let original_vision = theme::vision();
    for finished in [false, true] {
        if finished {
            app.run("sketch.finish", json!({})).unwrap();
        }
        for dark in [false, true] {
            for vision in ColorVision::ALL {
                theme::set_dark(dark);
                theme::set_vision(vision);
                let tokens = Tokens::get();
                let scene = super::build_sketch_scene(&app, &super::projection(&app, rect));
                // The sketch being edited draws on top; a finished one is depth-tested.
                let bytes = if finished { &scene.lines } else { &scene.overlay };
                let (lines, remainder) = bytes.as_chunks::<{ crate::gpu::LINE_SIZE }>();
                assert!(remainder.is_empty());
                let halo = super::c4(tokens.sketch_halo);
                let halo_count = lines.iter().filter(|line| line[24..28] == halo).count();
                if !finished {
                    assert_eq!(halo_count, 0, "a sketch being edited has no halo");
                    continue;
                }
                assert!(halo_count > 5, "rectangle, circle and line emit halos");
                let width = |line: &[u8]| f32::from_le_bytes(line[28..32].try_into().unwrap());
                let projected = super::c4(tokens.sketch_projected);
                let mut cores = 0;
                let mut references = 0;
                for (index, core) in lines.iter().enumerate().filter(|(_, line)| line[24..28] != halo) {
                    let rim = lines[..index].iter().find(|line| line[..24] == core[..24] && line[24..28] == halo);
                    if core[24..28] == projected {
                        assert!(rim.is_none(), "the face's projected outline has no halo");
                        references += 1;
                    } else {
                        assert_eq!(core[24..28], super::c4(tokens.sketch_fixed));
                        let rim = rim.unwrap();
                        assert_eq!(width(rim), width(core) + super::SKETCH_HALO_WIDTH, "halo precedes the narrower core");
                        cores += 1;
                    }
                }
                assert!(references > 0, "the projected face outline is drawn");
                assert_eq!(cores, halo_count, "one halo per curve segment");
            }
        }
    }
    theme::set_dark(original_dark);
    theme::set_vision(original_vision);
}

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
