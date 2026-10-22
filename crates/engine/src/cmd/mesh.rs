//! Parametric reconstruction of imported mechanical mesh bodies.

use serde_json::Value;
use solidcraft_doc::FeatureKind;
use solidcraft_doc::expr::Kind;

use super::{
    CommandSpec,
    features::{add_feature, check_expr},
};
use crate::{
    Result, Sel, Session,
    params::{bad, expr, str_},
};

pub static COMMANDS: &[CommandSpec] = &[
    CommandSpec::new("mesh.convert", "Convert Mesh", convert)
        .icon("import")
        .params("body?: mesh body name (default: the selected mesh body; it may be in any component, the conversion is authored where the body is); tolerance?: length expression (default 0.05 mm): rebuild a closed mesh as a solid body, with a parametric timeline feature — coplanar triangles fuse into planar faces (holes and all) and a curved surface stays faceted, so booleans, fillets, shell, holes and sketches on its faces work; a constant-section part is rebuilt exactly with planar and circular cylindrical faces; name?"),
];

/// The mesh body to convert: `body`, else the one selected body.
fn target(s: &Session, p: &Value, cmd: &str) -> Result<String> {
    if let Some(name) = str_(p, "body").filter(|b| !b.trim().is_empty()) {
        return Ok(name.to_owned());
    }
    let selected: Vec<&str> = s.selection.iter().filter_map(|x| if let Sel::Body { name } = x { Some(name.as_str()) } else { None }).collect();
    match selected.as_slice() {
        [name] => Ok((*name).to_owned()),
        [] => Err(bad(cmd, "select a mesh body")),
        _ => Err(bad(cmd, "select one mesh body")),
    }
}

pub(super) fn convert(s: &mut Session, p: &Value) -> Result<Value> {
    let cmd = "mesh.convert";
    let state = s.model.state();
    let body = target(s, p, cmd)?;
    let source = state.body(&body).ok_or_else(|| bad(cmd, format!("no body `{body}`")))?;
    if !source.body.is_mesh() {
        return Err(bad(cmd, "select a mesh body"));
    }
    // The conversion is authored where the body is, so a mesh in another component converts
    // without activating that component first (the right-click menu, a script).
    let component = s.doc.model_body_component(source);
    let tolerance = expr(p, "tolerance").unwrap_or_else(|| "0.05 mm".into());
    check_expr(s, &tolerance, Kind::Length, cmd, "tolerance")?;
    let saved = s.active_component;
    s.active_component = component;
    let out = add_feature(s, p, FeatureKind::MeshConvert { body: body.into(), tolerance });
    s.active_component = saved;
    let out = out?;
    if component != saved {
        s.refresh();
    }
    Ok(out)
}
