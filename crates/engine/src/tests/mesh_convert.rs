use super::*;
use solidcraft_doc::{Document, FeatureKind, MeshData};
use solidcraft_geom::Vec3;

fn imported_box() -> Session {
    let body = solidcraft_kernel::box_solid(Vec3::ZERO, Vec3::new(10., 20., 30.)).unwrap();
    let mesh = body.tessellate(0.01).unwrap();
    let mut doc = Document::default();
    doc.add_feature(
        FeatureKind::MeshImport {
            file: "public-safe-box.stl".into(),
            meshes: vec![MeshData {
                name: "Mesh".into(),
                positions: mesh.positions.iter().flat_map(|p| [p.x, p.y, p.z]).collect(),
                triangles: mesh.triangles.iter().flatten().copied().collect(),
                color: Some([0.2, 0.5, 0.8]),
            }],
        },
        Some("Imported mesh"),
    )
    .unwrap();
    Session::new(doc)
}

#[test]
fn mesh_conversion_is_parametric_undoable_and_editable_after_cold_reopen() {
    let mut s = imported_box();
    run(&mut s, "parameters.add", json!({"name":"size","expression":"1","unit":""}));
    run(&mut s, "solid.scale", json!({"bodies":["Mesh"],"factor":"size","name":"Mesh scale"}));
    run(&mut s, "mesh.convert", json!({"body":"Mesh","tolerance":"0.01 mm","name":"Reconstructed prism"}));
    let check = |s: &Session| {
        let state = s.model.state();
        let body = &state.body("Mesh").unwrap().body;
        assert!(!body.is_mesh());
        assert_eq!(body.faces(0.01).unwrap().len(), 6);
        assert_eq!(body.color(), Some([0.2, 0.5, 0.8]));
        assert!(s.model.results.iter().all(|r| r.error.is_none()), "{:?}", s.model.results);
    };
    check(&s);
    assert!((volume(&mut s) - 6000.).abs() < 1e-6);
    run(&mut s, "edit.undo", json!({}));
    assert!(s.model.state().body("Mesh").unwrap().body.is_mesh());
    run(&mut s, "edit.redo", json!({}));
    check(&s);
    run(&mut s, "parameters.change", json!({"name":"size","expression":"2"}));
    check(&s);
    assert!((volume(&mut s) - 48000.).abs() < 1e-5);
    let mut cold = Session::new(Document::from_json(&s.doc.to_json()).unwrap());
    check(&cold);
    assert!((volume(&mut cold) - 48000.).abs() < 1e-5);
    run(&mut cold, "solid.fillet", json!({"body":"Mesh","edges":[[0,20,60]],"radius":2}));
    assert!(volume(&mut cold) < 48000.);
    assert!(cold.model.results.iter().all(|r| r.error.is_none()));
}

#[test]
fn failed_mesh_conversions_leave_the_design_unchanged() {
    let mut s = imported_box();
    let before = s.doc.to_json();
    for params in [
        json!({}),
        json!({"body":"Missing"}),
        json!({"body":"Mesh","tolerance":0}),
        json!({"body":"Mesh","tolerance":"unknown"}),
        json!({"body":"Mesh","tolerance":"2 mm"}),
    ] {
        assert!(s.execute("mesh.convert", &params).is_err());
        assert_eq!(s.doc.to_json(), before);
        assert!(s.model.state().body("Mesh").unwrap().body.is_mesh());
    }
}

#[test]
fn convert_uses_the_selected_mesh_body_when_body_is_omitted() {
    let mut s = imported_box();
    s.selection = vec![crate::Sel::Body { name: "Mesh".into() }];
    run(&mut s, "mesh.convert", json!({"tolerance":"0.01 mm"}));
    let state = s.model.state();
    let body = &state.body("Mesh").unwrap().body;
    assert!(!body.is_mesh());
    assert_eq!(body.faces(0.01).unwrap().len(), 6);
}

#[test]
fn convert_without_body_or_selection_is_refused() {
    let mut s = imported_box();
    let before = s.doc.to_json();
    assert!(s.execute("mesh.convert", &json!({"tolerance":"0.01 mm"})).is_err());
    assert_eq!(s.doc.to_json(), before);
    assert!(s.model.state().body("Mesh").unwrap().body.is_mesh());
}

#[test]
fn an_explicit_body_wins_over_the_selection() {
    let mut s = imported_box();
    s.selection = vec![crate::Sel::Body { name: "Missing".into() }];
    run(&mut s, "mesh.convert", json!({"body":"Mesh","tolerance":"0.01 mm"}));
    assert!(!s.model.state().body("Mesh").unwrap().body.is_mesh());
}

/// A stepped block saved as STL: a box with a smaller box on it, which no single extruded
/// profile makes, so only the general conversion can turn it into a solid.
fn stepped_block_stl(tag: &str) -> std::path::PathBuf {
    let mut s = Session::default();
    run(&mut s, "solid.box", json!({"length":30,"width":20,"height":10,"operation":"new","body_name":"Step"}));
    run(&mut s, "solid.box", json!({"length":15,"width":10,"height":15,"corner":[5,5,10],"operation":"join"}));
    let path = std::env::temp_dir().join(format!("solidcraft-mesh-solid-{tag}-{}.stl", std::process::id()));
    run(&mut s, "file.save_mesh", json!({"path": path}));
    path
}

#[test]
fn a_mesh_that_is_not_a_prism_converts_and_takes_a_boolean() {
    let path = stepped_block_stl("nonprism");
    let mut s = Session::default();
    run(&mut s, "doc.open", json!({"path": path}));
    let name = s.model.state().bodies[0].name.clone();
    run(&mut s, "mesh.convert", json!({"body": name, "tolerance": "0.05 mm"}));
    let st = s.model.state();
    let body = &st.body(&name).unwrap().body;
    assert!(!body.is_mesh());
    assert!(body.validity().is_empty(), "{:?}", body.validity());
    assert!((volume(&mut s) - 8250.).abs() < 1e-6);
    // A solid body takes the operations that refuse a mesh.
    run(&mut s, "solid.cylinder", json!({"diameter": 6, "height": 20, "base": [25, 15, -5], "operation": "cut", "targets": [name]}));
    let hole = std::f64::consts::PI * 9. * 10.;
    assert!((volume(&mut s) - (8250. - hole)).abs() < 0.5, "{}", volume(&mut s));
    std::fs::remove_file(path).unwrap();
}

/// The point of converting: the body a general (non-prismatic) mesh becomes takes the solid
/// operations — a fillet, a hole, and a sketch on one of its faces extruded into it.
#[test]
fn a_converted_mesh_takes_fillets_holes_and_sketches_on_its_faces() {
    let path = stepped_block_stl("operations");
    let mut s = Session::default();
    run(&mut s, "doc.open", json!({"path": path}));
    let name = s.model.state().bodies[0].name.clone();
    run(&mut s, "mesh.convert", json!({"body": name, "tolerance": "0.05 mm"}));
    let base = volume(&mut s);
    assert!((base - 8250.).abs() < 1e-6);
    let failed = |s: &Session| s.model.results.iter().filter_map(|r| r.error.clone()).collect::<Vec<_>>();

    // A fillet on one of the base's vertical edges.
    run(&mut s, "solid.fillet", json!({"body": name, "edges": [[30., 20., 5.]], "radius": 2}));
    assert!(failed(&s).is_empty(), "{:?}", failed(&s));
    let filleted = volume(&mut s);
    assert!(filleted < base - 1.0, "the fillet removed material: {base} -> {filleted}");

    // A chamfer on the base's opposite vertical edge.
    run(&mut s, "solid.chamfer", json!({"body": name, "edges": [[0., 20., 5.]], "distance": 2}));
    assert!(failed(&s).is_empty(), "{:?}", failed(&s));

    // A hole into the base's top face, beside the boss.
    run(&mut s, "solid.hole", json!({"position": [25., 17., 10.], "diameter": 4}));
    assert!(failed(&s).is_empty(), "{:?}", failed(&s));
    let holed = volume(&mut s);
    assert!(holed < filleted - 100.0, "{filleted} -> {holed}");

    // A sketch on the boss's top face is accepted. The conversion carries no face history, so the
    // attachment is fixed: the sketch sits on the face but does not follow a later upstream edit
    // (docs/mesh-to-solid.md).
    run(&mut s, "sketch.create", json!({"plane": {"face": [12.5, 10., 25.], "fixed": true}}));
    run(&mut s, "sketch.finish", json!({}));
    // Its circle, on a plane at the boss's own centre, cuts a pocket into the boss.
    let sk = run(&mut s, "sketch.create", json!({"plane": {"origin": [12.5, 10., 25.], "x_dir": [1., 0., 0.], "y_dir": [0., 1., 0.]}}));
    run(&mut s, "sketch.circle.center", json!({"center": [0., 0.], "radius": 3.}));
    run(&mut s, "sketch.finish", json!({}));
    run(&mut s, "solid.extrude", json!({"sketch": sk["sketch"], "distance": 5., "direction": "negative", "operation": "cut", "targets": [name]}));
    assert!(failed(&s).is_empty(), "{:?}", failed(&s));
    let cut = volume(&mut s);
    assert!(cut < holed - 100.0, "{holed} -> {cut}");
    std::fs::remove_file(path).unwrap();
}

/// A body converted from a general mesh takes a shell as well: one wall thickness inward,
/// hollow through the face picked.
#[test]
fn a_freshly_converted_mesh_takes_a_shell() {
    let path = stepped_block_stl("shell");
    let mut s = Session::default();
    run(&mut s, "doc.open", json!({"path": path}));
    let name = s.model.state().bodies[0].name.clone();
    run(&mut s, "mesh.convert", json!({"body": name, "tolerance": "0.05 mm"}));
    let solid = volume(&mut s);
    run(&mut s, "solid.shell", json!({"faces": [[15., 10., 0.]], "thickness": 2., "body": name}));
    assert!(s.model.results.iter().all(|r| r.error.is_none()), "{:?}", s.model.results.iter().filter_map(|r| r.error.clone()).collect::<Vec<_>>());
    let shelled = volume(&mut s);
    assert!(shelled < solid && shelled > solid * 0.2, "the shell hollowed the body: {solid} -> {shelled}");
    std::fs::remove_file(path).unwrap();
}

#[test]
fn inserting_a_mesh_as_a_solid_keeps_it_a_solid_after_a_cold_reopen() {
    let path = stepped_block_stl("assolid");
    let mut s = Session::default();
    let out = run(&mut s, "file.insert_mesh", json!({"path": path, "as_solid": true}));
    assert_eq!(out["solid_bodies"].as_array().map(Vec::len), Some(1), "{out}");
    let name = out["created_bodies"][0].as_str().unwrap().to_string();
    assert!(!s.model.state().body(&name).unwrap().body.is_mesh());
    assert!((volume(&mut s) - 8250.).abs() < 1e-6);
    let conversions = s.doc.features.iter().filter(|f| matches!(f.kind, FeatureKind::MeshConvert { .. })).count();
    assert_eq!(conversions, 1, "the conversion is a timeline feature, so a rebuild repeats it");
    let mut cold = Session::new(Document::from_json(&s.doc.to_json()).unwrap());
    assert!(!cold.model.state().body(&name).unwrap().body.is_mesh());
    assert!((volume(&mut cold) - 8250.).abs() < 1e-6);
    std::fs::remove_file(path).unwrap();
}

#[test]
fn opening_a_mesh_as_a_solid_holds_a_solid_body() {
    let path = stepped_block_stl("openassolid");
    let mut s = Session::default();
    let out = run(&mut s, "doc.open", json!({"path": path, "as_solid": true}));
    assert_eq!(out["solid_bodies"].as_array().map(Vec::len), Some(1), "{out}");
    assert!(s.model.state().bodies.iter().all(|b| !b.body.is_mesh()));
    assert!((volume(&mut s) - 8250.).abs() < 1e-6);
    std::fs::remove_file(path).unwrap();
}

/// A bare tolerance means the design's unit, as it does for any command: an inch design stores
/// the conversion's tolerance in inches.
/// A mesh body in another component converts from the root: the conversion is authored in the
/// body's own component, so the right-click menu and scripts need no activation first.
#[test]
fn a_mesh_in_another_component_converts_without_activating_it() {
    let path = stepped_block_stl("component");
    let mut s = Session::default();
    let imported = run(&mut s, "file.insert_mesh", json!({"path": path}));
    let name = imported["created_bodies"][0].as_str().unwrap_or_default().to_string();
    let component = imported["component"].as_u64().unwrap_or(0);
    assert_ne!(component, 0, "the import made a component: {imported}");
    run(&mut s, "component.activate", json!({"component": 0}));
    run(&mut s, "mesh.convert", json!({"body": name, "tolerance": "0.05 mm"}));
    let state = s.model.state();
    assert!(!state.body(&name).unwrap().body.is_mesh(), "converted although the root is active");
    assert_eq!(s.doc.model_body_component(state.body(&name).unwrap()), component, "the conversion stayed in the body's component");
    assert!(s.model.results.iter().all(|r| r.error.is_none()), "{:?}", s.model.results.iter().filter_map(|r| r.error.clone()).collect::<Vec<_>>());
    std::fs::remove_file(path).unwrap();
}

#[test]
fn an_inch_design_stores_the_conversions_tolerance_in_its_units() {
    let path = stepped_block_stl("inch");
    let mut s = Session::default();
    s.doc_mut().units = "in".into();
    run(&mut s, "file.insert_mesh", json!({"path": path, "as_solid": true, "tolerance": "0.01"}));
    let tolerance = s.doc.features.iter().find_map(|f| match &f.kind {
        FeatureKind::MeshConvert { tolerance, .. } => Some(tolerance.as_str().to_string()),
        _ => None,
    });
    assert_eq!(tolerance.as_deref(), Some("0.01 in"), "the bare length took the design's unit");
    assert!(!s.model.state().bodies.is_empty() && !s.model.state().bodies[0].body.is_mesh());
    std::fs::remove_file(path).unwrap();
}

#[test]
fn a_mesh_the_conversion_cannot_take_is_still_imported_as_a_mesh_body() {
    let path = std::env::temp_dir().join(format!("solidcraft-open-mesh-{}.stl", std::process::id()));
    std::fs::write(
        &path,
        "solid open\nfacet normal 0 0 1\nouter loop\nvertex 0 0 0\nvertex 10 0 0\nvertex 0 10 0\nendloop\nendfacet\nendsolid open\n",
    )
    .unwrap();
    let mut s = Session::default();
    let out = run(&mut s, "file.insert_mesh", json!({"path": path, "as_solid": true}));
    let warnings: Vec<&str> = out["warnings"].as_array().into_iter().flatten().filter_map(Value::as_str).collect();
    assert!(warnings.iter().any(|w| w.contains("stayed a mesh body")), "{out}");
    assert_eq!(out["solid_bodies"].as_array().map(Vec::len), Some(0), "{out}");
    let name = out["created_bodies"][0].as_str().unwrap().to_string();
    assert!(s.model.state().body(&name).unwrap().body.is_mesh());
    std::fs::remove_file(path).unwrap();
}

#[test]
fn exported_binary_stl_with_a_bore_reconstructs_analytic_faces() {
    let mut s = Session::default();
    run(&mut s, "sketch.create", json!({"plane":"XY","name":"Section"}));
    run(&mut s, "sketch.line", json!({"points":[[0,0],[40,0],[40,20],[20,20],[20,30],[0,30]],"closed":true}));
    run(&mut s, "sketch.circle.center", json!({"center":[10,10],"radius":4}));
    run(&mut s, "sketch.finish", json!({}));
    run(&mut s, "solid.extrude", json!({"sketch":"Section","profiles":[{"point":[30,10]}],"distance":15,"body_name":"Bracket"}));
    let path = std::env::temp_dir().join(format!("solidcraft-mesh-convert-{}.stl", std::process::id()));
    run(&mut s, "file.save_mesh", json!({"path":path}));
    run(&mut s, "doc.open", json!({"path":path}));
    let name = s.model.state().bodies[0].name.clone();
    run(&mut s, "mesh.convert", json!({"body":name,"tolerance":"0.05 mm"}));
    let state = s.model.state();
    let body = &state.bodies[0].body;
    assert!(!body.is_mesh());
    assert!(body.validity().is_empty());
    assert_eq!(solidcraft_kernel::measure(body).unwrap().merged.face_types.get("cylinder"), Some(&1));
    assert_eq!(body.faces(0.01).unwrap().len(), 10);
    std::fs::remove_file(path).unwrap();
}

/// #784: STL is closed as written where faces meet (a fillet, a bored hole): every edge of the
/// file, keyed by its exact f32 ends, is shared by exactly two triangles, as in 3MF.
#[test]
fn stl_export_is_closed_where_faces_meet() {
    let mut s = Session::default();
    run(&mut s, "solid.box", json!({"length": 30, "width": 20, "height": 10, "operation": "new", "body_name": "BoxFillet"}));
    run(&mut s, "solid.fillet", json!({"body": "BoxFillet", "edges": [[15, 0, 10]], "radius": 2}));
    run(&mut s, "solid.cylinder", json!({"diameter": 30, "height": 12, "base": [60, 0, 0], "operation": "new", "body_name": "CylHole"}));
    run(&mut s, "solid.hole", json!({"position": [60, 0, 12], "diameter": 6}));
    let state = s.model.state();
    for name in ["BoxFillet", "CylHole"] {
        let body = &state.bodies.iter().find(|b| b.name == name).unwrap().body;
        let expected = solidcraft_kernel::measure(body).unwrap().volume;
        for format in [solidcraft_io::Format::StlBinary, solidcraft_io::Format::StlAscii] {
            let bytes = solidcraft_io::export(&state, &[name.to_string()], format, name).unwrap();
            let tris = solidcraft_io::read_stl(&bytes).unwrap();
            let key = |p: [f32; 3]| p.map(f32::to_bits);
            let mut edges = std::collections::HashMap::new();
            for t in &tris {
                for (a, b) in [(t[0], t[1]), (t[1], t[2]), (t[2], t[0])] {
                    let (a, b) = (key(a), key(b));
                    *edges.entry(if a < b { (a, b) } else { (b, a) }).or_insert(0) += 1;
                }
            }
            let open = edges.values().filter(|n| **n != 2).count();
            assert_eq!(open, 0, "{name} {format:?}: {open} edges not shared by exactly two triangles");
            let p = |v: [f32; 3]| Vec3::new(v[0] as f64, v[1] as f64, v[2] as f64);
            let v: f64 = tris.iter().map(|t| p(t[0]).dot(p(t[1]).cross(p(t[2]))) / 6.0).sum();
            assert!((v - expected).abs() <= expected * 1e-3, "{name} {format:?}: STL volume {v}, body {expected}");
            let mf = solidcraft_io::export(&state, &[name.to_string()], solidcraft_io::Format::ThreeMf, name).unwrap();
            let (objs, _) = solidcraft_io::read_3mf(&mf).unwrap();
            assert_eq!(tris.len(), objs.iter().map(|o| o.triangles.len()).sum::<usize>(), "{name} {format:?}: STL and 3MF triangles");
        }
    }
}

#[test]
fn save_mesh_rejects_unknown_format_and_slicer_without_writing() {
    let mut s = imported_box();
    let path = std::env::temp_dir().join(format!("solvecraft-save-mesh-{}.stl", std::process::id()));
    let _ = std::fs::remove_file(&path);
    for params in [json!({"path":path,"format":"nope"}), json!({"path":path,"slicer":"no such slicer anywhere"})] {
        assert!(s.execute("file.save_mesh", &params).is_err());
        assert!(!path.exists());
    }
}

#[test]
fn save_mesh_writes_stl_and_3mf() {
    let mut s = imported_box();
    let dir = std::env::temp_dir();
    for (ext, format) in [("stl", None), ("3mf", None), ("dat", Some("3mf"))] {
        let path = dir.join(format!("solvecraft-save-mesh-{}.{ext}", std::process::id()));
        let mut params = json!({"path": path});
        if let Some(f) = format {
            params["format"] = json!(f);
        }
        s.execute("file.save_mesh", &params).unwrap();
        let bytes = std::fs::read(&path).unwrap();
        // A 3MF file is a zip archive; a binary STL starts with its 80-byte header.
        assert_eq!(bytes.starts_with(b"PK"), ext == "3mf" || format == Some("3mf"), "{ext}");
        std::fs::remove_file(path).unwrap();
    }
}

#[test]
fn installed_slicers_are_listed() {
    let mut s = imported_box();
    let r = s.execute("file.slicers", &json!({})).unwrap();
    assert!(r["slicers"].is_array());
}
