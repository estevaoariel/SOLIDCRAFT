//! Simulation studies: Airflow (steady, low-speed flow through ducts, fans and cooling paths),
//! Aerodynamics (external flow around a body: drag and lift forces) and Gas Dynamics (transient
//! compressible flow with shocks: pressure pulses, blast, nozzles).
//!
//! Airflow studies run: `sim.flow.new` sets one up (inlets and outlets picked as faces), `sim.flow.run`
//! starts it on a background thread, `sim.flow.status` follows it, `sim.flow.results` reports the
//! flows and pressures. Studies live beside the design, never in it, so nothing here touches the
//! design or its undo history. `sim.seek.*` (Goal Seek) runs a study again with design parameters
//! varied, to meet goals; only `sim.seek.apply` changes the design (one undo step). Aerodynamics and
//! Gas Dynamics check their inputs and are refused with "not supported yet".

use serde_json::{Value, json};
use solidcraft_sim as sim;

use super::CommandSpec;
use crate::flow::{self, OpeningSpec, Region, Spec};
use crate::params::{bad, bool_, num, str_};
use crate::seek;
use crate::{EngineError, Result, Session};

const OPENING_DOC: &str = "Openings are faces: `{\"face\": [x, y, z] (a point on it) or its name, \"body\"?: name (default the study's body), \"name\"?: label, \"region\"?: \"hole\" | \"face\"}`. A face with a hole in it (a duct's rim) stands for the hole (holes under a tenth of the largest, such as screw holes, are left out); a face without one (a fan's outlet face) for itself. On a mesh body (an STL) the flat piece of surface around the point is the face. Or a section across the duct: `{\"section\": {\"point\": [x, y, z] inside the passage, \"normal\": the way the air goes}}`, the passage where that plane cuts the body (an inlet's section closes the duct behind it, where the fan sits).";

pub static COMMANDS: &[CommandSpec] = &[
CommandSpec::new("sim.flow.new", "Airflow Study", flow_new).noundo().at("SIMULATION", "STUDY").icon("sim_flow")
    .params("Set up an Airflow study (steady, low-speed air through ducts, fans and cooling paths); `sim.flow.run` runs it. body: the duct's body; inlets: list of openings each with `speed` (m/s) or `fan`: {\"type\": \"axial\" | \"radial\" | \"linear\", \"free_flow_cfm\", \"max_pressure_pa\"} or {\"curve\": [[cfm, pa], ...]}; outlets: list of openings (where the air is expected to leave; measured, and the air flows on past them into open air); bodies?: every body in the air's way (default: all visible bodies); cell?: cell size, mm (default: about 8 cells across the narrowest opening); pad?: open air around the bodies, mm (default 20); turbulence?: bool (default true); surface?: the walls' finish, smooth | rough (an FDM print's walls; default smooth), or roughness_mm?: their equivalent sand-grain roughness (it acts through the turbulence model); max_steps?: (default 200000); name?: the study's name. The design is unchanged. Openings: see sim.flow.edit."),
CommandSpec::new("sim.flow.edit", "Edit Airflow Study", flow_edit).noundo().icon("sim_flow")
    .params("Change a study's setup: study: its name; any of sim.flow.new's keys (inlets/outlets replace the lists). Its results are cleared."),
CommandSpec::new("sim.flow.run", "Run Airflow Study", flow_run).noundo().icon("sim_flow")
    .params("Run a study until the flow settles: study: its name; wait?: bool (default false: it runs in the background and sim.flow.status follows it); max_steps?, tolerance? (settled when the flows and pressures change by less than this fraction, default 0.005). Uses the GPU when the app has one, else the CPU (small grids only)."),
CommandSpec::new("sim.flow.status", "Airflow Study Status", flow_status).noundo().icon("sim_flow")
    .params("study?: its name (default: every study). State (ready, running, done, failed), steps taken, the solver."),
CommandSpec::new("sim.flow.results", "Airflow Study Results", flow_results).noundo().icon("sim_flow")
    .params("study: its name. Flow (m³/s and CFM), mean pressure (Pa, gauge), mean and top speed (m/s) per inlet and outlet, each outlet's share of the flow, the pressure drop, the ranges of speed and pressure, and setup warnings (leaks, thin walls)."),
CommandSpec::new("sim.flow.probe", "Probe Airflow Result", flow_probe).noundo().icon("sim_flow")
    .params("study: its name; point: [x, y, z] mm. The velocity (m/s) and pressure (Pa) there, or null outside the air. Or grid: {min, max: [x, y, z] mm, step: mm, min_speed?: m/s}: every grid point in the air (at or over min_speed) as [x, y, z, ux, uy, uz, pressure_pa], at most 4,000,000 points."),
CommandSpec::new("sim.flow.cancel", "Cancel Airflow Study", flow_cancel).noundo().icon("sim_flow")
    .params("study: its name. Stops its run; the last results stay."),
CommandSpec::new("sim.flow.delete", "Delete Airflow Study", flow_delete).noundo().icon("sim_flow")
    .params("study: its name."),
CommandSpec::new("sim.flow.list", "Airflow Studies", flow_list).noundo().icon("sim_flow")
    .params("The studies set up in this session with their setups."),
CommandSpec::new("sim.seek.new", "Goal Seek", seek_new).noundo().icon("sim_flow")
    .params("Set up a goal seek over an Airflow study: study; vary: 1–3 of {\"parameter\": a design parameter's name, \"min\", \"max\" (in its unit), \"count\"?: values a sweep takes, default 5}; mode?: sweep (every combination; default) | optimize (Nelder–Mead), budget?: cycles for optimize (default 20); goals (see sim.seek.results); regions?: [{\"name\", \"box\": {\"min\", \"max\"}} | {\"name\", \"disc\": {\"center\", \"normal\", \"radius\"}}] mm. Returns the number of cycles and, after a run of the study, a time estimate. Nothing changes in the design."),
CommandSpec::new("sim.seek.run", "Run Goal Seek", seek_run).noundo().icon("sim_flow")
    .params("seek: its name; wait?: bool. Runs its cycles in the background (each on a scratch copy of the design with the cycle's parameters, starting from the last cycle's flow); sim.seek.status follows it."),
CommandSpec::new("sim.seek.status", "Goal Seek Status", seek_status).noundo().icon("sim_flow")
    .params("seek?: its name (default: all). State, cycles done of how many, the best cycle so far."),
CommandSpec::new("sim.seek.results", "Goal Seek Results", seek_results).noundo().icon("sim_flow")
    .params("seek: its name. Every cycle: its parameter values, its measures (each opening, pressure_drop, each region: flow m³/s and CFM, pressure Pa, speed and top speed m/s), its score (lower is better) and whether it keeps to the limits; and the best cycle. Goals: [{goal: maximize | minimize, measure, quantity?, weight?} | {goal: target, measure, quantity?, value, weight?} | {goal: equalize, measures, quantity?, weight?} | {goal: limit, measure, quantity?, min?, max?}], quantity flow | pressure | speed | max_speed."),
CommandSpec::new("sim.seek.apply", "Apply Goal Seek Cycle", seek_apply).noundo().icon("sim_flow")
    .params("seek: its name; cycle?: its number (default: the best). Sets the design's parameters to that cycle's values, as one undo step."),
CommandSpec::new("sim.seek.cancel", "Cancel Goal Seek", seek_cancel).noundo().icon("sim_flow")
    .params("seek: its name. Stops after the running cycle; its cycles stay."),
CommandSpec::new("sim.seek.delete", "Delete Goal Seek", seek_delete).noundo().icon("sim_flow")
    .params("seek: its name."),
CommandSpec::new("sim.aero.new", "Aerodynamics Study", aero_new).noundo().at("SIMULATION", "STUDY").icon("sim_aero")
    .params("body: the solid body the air flows around. External flow at a wind speed, reporting drag and lift forces. Not supported yet: the solver is not built, so a valid request returns an error and the design is unchanged."),
CommandSpec::new("sim.gas.new", "Gas Dynamics Study", gas_new).noundo().at("SIMULATION", "STUDY").icon("sim_gas")
    .params("body: the solid body the gas flows through or around. Transient compressible flow with shocks (pressure pulses, blast, nozzles). Not supported yet: the solver is not built, so a valid request returns an error and the design is unchanged."),
];

fn aero_new(s: &mut Session, params: &Value) -> Result<Value> {
    refused(s, params, "sim.aero.new", "aerodynamics")
}

fn gas_new(s: &mut Session, params: &Value) -> Result<Value> {
    refused(s, params, "sim.gas.new", "gas dynamics")
}

fn body_param<'a>(s: &Session, params: &'a Value, cmd: &str) -> Result<&'a str> {
    let body = params.get("body").and_then(Value::as_str).ok_or_else(|| bad(cmd, "`body` is required: the name of the solid body to study"))?;
    if s.world_state().body(body).is_none() {
        return Err(bad(cmd, format!("no body `{body}`")));
    }
    Ok(body)
}

/// Check the request as the finished study will, then refuse it: there is no solver to run.
fn refused(s: &mut Session, params: &Value, cmd: &str, solver: &str) -> Result<Value> {
    body_param(s, params, cmd)?;
    Err(EngineError::Other(format!("not supported yet: the {solver} solver is not built")))
}

const CFM: f64 = sim::study::CFM_PER_M3S;

fn source(cmd: &str, o: &Value, name: &str) -> Result<sim::Source> {
    if let Some(v) = o.get("speed") {
        let v = v
            .as_f64()
            .filter(|v| v.is_finite() && *v > 0.0 && *v <= 100.0)
            .ok_or_else(|| bad(cmd, format!("`{name}`: `speed` is m/s, above 0 and at most 100")))?;
        return Ok(sim::Source::Speed(v));
    }
    let fan = o.get("fan").ok_or_else(|| bad(cmd, format!("the inlet `{name}` needs a `speed` (m/s) or a `fan`")))?;
    let fan_err = |e: sim::SimError| bad(cmd, format!("`{name}`: {e}"));
    if let Some(points) = fan.get("curve").and_then(Value::as_array) {
        let mut pts = Vec::new();
        for p in points.iter().take(65) {
            let (Some(q), Some(pa)) = (p.get(0).and_then(Value::as_f64), p.get(1).and_then(Value::as_f64)) else {
                return Err(bad(cmd, format!("`{name}`: a fan curve is a list of [cfm, pa] points")));
            };
            pts.push((q / CFM, pa));
        }
        return sim::FanCurve::new(pts).map(sim::Source::Fan).map_err(fan_err);
    }
    let free = num(fan, "free_flow_cfm")
        .ok_or_else(|| bad(cmd, format!("`{name}`: a fan needs `free_flow_cfm` and `max_pressure_pa`, or a `curve`")))?
        / CFM;
    let pmax = num(fan, "max_pressure_pa").ok_or_else(|| bad(cmd, format!("`{name}`: a fan needs `max_pressure_pa`")))?;
    let curve = match str_(fan, "type").unwrap_or("axial") {
        "axial" => sim::FanCurve::axial(free, pmax),
        "radial" | "blower" => sim::FanCurve::radial(free, pmax),
        "linear" => sim::FanCurve::linear(free, pmax),
        other => return Err(bad(cmd, format!("`{name}`: fan type `{other}`: use axial, radial or linear"))),
    };
    curve.map(sim::Source::Fan).map_err(fan_err)
}

fn openings(s: &Session, cmd: &str, list: &Value, body: &str, inlet: bool) -> Result<Vec<OpeningSpec>> {
    let what = if inlet { "inlets" } else { "outlets" };
    let a = list.as_array().ok_or_else(|| bad(cmd, format!("`{what}` is a list of openings. {OPENING_DOC}")))?;
    if a.len() > 64 {
        return Err(bad(cmd, format!("at most 64 {what}")));
    }
    let mut out = Vec::new();
    for (i, o) in a.iter().enumerate() {
        let name = str_(o, "name").map(str::to_string).unwrap_or_else(|| format!("{} {}", if inlet { "Inlet" } else { "Outlet" }, i + 1));
        if name.is_empty() || name.len() > 128 {
            return Err(bad(cmd, "an opening's name is 1…128 characters"));
        }
        let on = str_(o, "body").unwrap_or(body);
        if s.world_state().body(on).is_none() {
            return Err(bad(cmd, format!("`{name}`: no body `{on}`")));
        }
        let (point, section) = match (o.get("face"), o.get("section")) {
            (Some(face), None) => (super::face::face_ref(s, face, Some(on), cmd)?, None),
            (None, Some(sec)) => {
                let point = vec3_of(sec.get("point"), cmd, &format!("`{name}`: section.point"))?;
                let normal = vec3_of(sec.get("normal"), cmd, &format!("`{name}`: section.normal"))?;
                if normal.normalized().is_none() {
                    return Err(bad(cmd, format!("`{name}`: section.normal must not be zero")));
                }
                (point, Some(normal))
            }
            _ => return Err(bad(cmd, format!("`{name}` needs a `face` or a `section`. {OPENING_DOC}"))),
        };
        let region = match str_(o, "region") {
            None => None,
            Some("hole") => Some(Region::Hole),
            Some("face") => Some(Region::Face),
            Some(other) => return Err(bad(cmd, format!("`{name}`: region `{other}`: use hole or face"))),
        };
        let source = if inlet { Some(source(cmd, o, &name)?) } else { None };
        out.push(OpeningSpec { name, body: on.to_string(), point, region, section, source });
    }
    Ok(out)
}

/// The spec from `p`, starting from `base` (an existing study's) where given.
fn spec(s: &Session, p: &Value, cmd: &str, base: Option<Spec>) -> Result<Spec> {
    let body = match (&base, p.get("body")) {
        (Some(b), None) => b.body.clone(),
        _ => body_param(s, p, cmd)?.to_string(),
    };
    let inlets = match (p.get("inlets"), &base) {
        (Some(l), _) => openings(s, cmd, l, &body, true)?,
        (None, Some(b)) => b.inlets.clone(),
        (None, None) => return Err(bad(cmd, format!("`inlets` is required: where the air comes in. {OPENING_DOC}"))),
    };
    if inlets.is_empty() {
        return Err(bad(cmd, "a study needs at least one inlet"));
    }
    let outlets = match (p.get("outlets"), &base) {
        (Some(l), _) => openings(s, cmd, l, &body, false)?,
        (None, Some(b)) => b.outlets.clone(),
        (None, None) => Vec::new(),
    };
    let st = s.world_state();
    let bodies = match (p.get("bodies"), &base) {
        (Some(v), _) => {
            let a = v.as_array().ok_or_else(|| bad(cmd, "`bodies` is a list of body names"))?;
            let mut names = Vec::new();
            for b in a {
                let n = b.as_str().ok_or_else(|| bad(cmd, "`bodies` is a list of body names"))?;
                if st.body(n).is_none() {
                    return Err(bad(cmd, format!("no body `{n}`")));
                }
                names.push(n.to_string());
            }
            names
        }
        (None, Some(b)) if p.get("body").is_none() => b.bodies.clone(),
        _ => st.bodies.iter().map(|b| b.name.clone()).filter(|n| !s.visibility.hidden_bodies.contains(n)).collect(),
    };
    let mut bodies = bodies;
    for b in std::iter::once(&body).chain(inlets.iter().map(|o| &o.body)).chain(outlets.iter().map(|o| &o.body)) {
        if !bodies.contains(b) {
            bodies.push(b.clone());
        }
    }
    let cell = match p.get("cell") {
        Some(Value::Null) | None => base.as_ref().and_then(|b| b.cell),
        Some(v) => {
            Some(v.as_f64().filter(|c| c.is_finite() && *c >= 0.05 && *c <= 50.0).ok_or_else(|| bad(cmd, "`cell` is the cell size in mm, 0.05…50"))?)
        }
    };
    let pad = match num(p, "pad") {
        Some(v) if v.is_finite() && (0.0..=1000.0).contains(&v) => v,
        Some(_) => return Err(bad(cmd, "`pad` is the open air around the bodies in mm, 0…1000")),
        None => base.as_ref().map_or(20.0, |b| b.pad),
    };
    let turbulence = bool_(p, "turbulence").or(base.as_ref().map(|b| b.turbulence)).unwrap_or(true);
    // A roughness given but not a usable number is an error, not the default.
    let given = p.get("roughness_mm").map(|_| num(p, "roughness_mm").unwrap_or(f64::NAN));
    let roughness = match (str_(p, "surface"), given) {
        (Some(_), Some(_)) => return Err(bad(cmd, "give `surface` or `roughness_mm`, not both")),
        (Some(name), None) => match flow::SURFACES.iter().find(|(n, _)| *n == name) {
            Some((_, k)) => *k,
            None => return Err(bad(cmd, format!("surface `{name}`: use smooth or rough"))),
        },
        (None, Some(k)) if k.is_finite() && (0.0..=5.0).contains(&k) => k,
        (None, Some(_)) => return Err(bad(cmd, "`roughness_mm` is 0…5")),
        (None, None) => base.as_ref().map_or(0.0, |b| b.roughness),
    };
    let max_steps = match num(p, "max_steps") {
        Some(v) if v.is_finite() && (1000.0..=10_000_000.0).contains(&v) => v as u64,
        Some(_) => return Err(bad(cmd, "`max_steps` is 1000…10000000")),
        None => base.as_ref().map_or(200_000, |b| b.max_steps),
    };
    Ok(Spec { body, bodies, inlets, outlets, cell, pad, turbulence, roughness, max_steps })
}

fn max_cells(s: &Session) -> u64 {
    flow::max_cells(s.flow.backend.as_ref())
}

/// Prepare `spec` on the design as it is now: the check `sim.flow.new` makes, and the lattice a
/// run starts from.
fn prepare(s: &Session, spec: &Spec, cmd: &str) -> Result<(sim::Prepared, flow::Remake)> {
    let st = s.world_state();
    let (setup, meshes) = flow::setup(&st, spec, max_cells(s)).map_err(|e| bad(cmd, e))?;
    let refs: Vec<&solidcraft_geom::Mesh> = meshes.iter().map(|m| m.as_ref()).collect();
    let prepared = sim::prepare(&refs, &setup).map_err(|e| bad(cmd, e.to_string()))?;
    drop(refs);
    Ok((prepared, flow::Remake { meshes, setup }))
}

fn summary(name: &str, prepared: &sim::Prepared, setup: &sim::FlowSetup) -> Value {
    json!({
        "study": name,
        "cell_mm": setup.cell,
        "grid": prepared.grid.dims,
        "cells": prepared.grid.len(),
        "inlets": prepared.inlets.iter().map(|l| json!({"name": l.name, "cells": l.cells.len(), "area_mm2": l.cells.len() as f64 * setup.cell * setup.cell})).collect::<Vec<_>>(),
        "outlets": prepared.outlets.iter().map(|l| json!({"name": l.name, "cells": l.cells.len()})).collect::<Vec<_>>(),
        "warnings": prepared.warnings.iter().map(flow::warning_text).collect::<Vec<_>>(),
    })
}

fn flow_new(s: &mut Session, p: &Value) -> Result<Value> {
    let cmd = "sim.flow.new";
    let spec = spec(s, p, cmd, None)?;
    let name = match str_(p, "name") {
        Some(n) if !n.is_empty() && n.len() <= 128 => n.to_string(),
        Some(_) => return Err(bad(cmd, "a study's name is 1…128 characters")),
        None => (1..).map(|i| format!("Airflow {i}")).find(|n| s.flow.get(n).is_none()).unwrap_or_else(|| "Airflow".into()),
    };
    let (prepared, flow::Remake { setup, .. }) = prepare(s, &spec, cmd)?;
    let out = summary(&name, &prepared, &setup);
    s.flow.list.retain(|st| st.name != name);
    s.flow.list.push(flow::Study { name, spec, job: None, result: None, revision: 0 });
    Ok(out)
}

fn study_name<'a>(s: &Session, p: &'a Value, cmd: &str) -> Result<&'a str> {
    let name = str_(p, "study").ok_or_else(|| bad(cmd, "`study` is required: the study's name (sim.flow.list)"))?;
    if s.flow.get(name).is_none() {
        return Err(bad(cmd, format!("no study `{name}`")));
    }
    Ok(name)
}

fn flow_edit(s: &mut Session, p: &Value) -> Result<Value> {
    let cmd = "sim.flow.edit";
    let name = study_name(s, p, cmd)?.to_string();
    let base = s.flow.get(&name).map(|st| st.spec.clone());
    let spec = spec(s, p, cmd, base)?;
    let (prepared, flow::Remake { setup, .. }) = prepare(s, &spec, cmd)?;
    let out = summary(&name, &prepared, &setup);
    if let Some(st) = s.flow.get_mut(&name) {
        st.spec = spec;
        st.job = None;
        st.result = None;
        st.revision += 1;
    }
    Ok(out)
}

fn flow_run(s: &mut Session, p: &Value) -> Result<Value> {
    let cmd = "sim.flow.run";
    let name = study_name(s, p, cmd)?.to_string();
    s.poll_flow();
    if s.flow.get(&name).and_then(|st| st.job.as_ref()).is_some_and(|j| j.progress.lock().is_ok_and(|p| p.done.is_none())) {
        return Err(bad(cmd, format!("`{name}` is running; sim.flow.cancel stops it")));
    }
    let Some(spec) = s.flow.get(&name).map(|st| st.spec.clone()) else { return Err(bad(cmd, format!("no study `{name}`"))) };
    let tolerance = match num(p, "tolerance") {
        Some(t) if t.is_finite() && (1e-5..=0.2).contains(&t) => t,
        Some(_) => return Err(bad(cmd, "`tolerance` is a fraction, 0.00001…0.2")),
        None => 0.005,
    };
    let max_steps = match num(p, "max_steps") {
        Some(v) if v.is_finite() && (100.0..=10_000_000.0).contains(&v) => v as u64,
        Some(_) => return Err(bad(cmd, "`max_steps` is 100…10000000")),
        None => spec.max_steps,
    };
    let (prepared, remake) = prepare(s, &spec, cmd)?;
    // Measure about sixteen times per crossing of the box, at least every 50 steps.
    let longest = prepared.grid.dims.iter().copied().max().unwrap_or(1) as f64;
    let check_every = ((longest / sim::study::LATTICE_SPEED) / 16.0).clamp(50.0, 2000.0) as u32;
    let limits = sim::Limits { max_steps, check_every, tolerance, soft_start: true };
    let backend: flow::FlowBackend = s.flow.backend.clone().unwrap_or_else(|| std::sync::Arc::new(sim::Cpu));
    let names = (spec.inlets.iter().map(|o| o.name.clone()).collect(), spec.outlets.iter().map(|o| o.name.clone()).collect());
    let wait = bool_(p, "wait").unwrap_or(false);
    let cells = prepared.grid.len();
    let job = flow::start(prepared, backend, limits, names, wait, Some(remake));
    if let Some(st) = s.flow.get_mut(&name) {
        st.job = Some(job);
    }
    if wait {
        s.poll_flow();
        let st = s.flow.get(&name).ok_or_else(|| bad(cmd, "the study went away"))?;
        if let Some(r) = &st.result {
            return Ok(flow::results(&name, r));
        }
        let status = flow::status(st);
        return Err(EngineError::Other(status.get("error").and_then(Value::as_str).unwrap_or("the run did not finish").to_string()));
    }
    Ok(json!({"study": name, "state": "running", "cells": cells, "check_every": check_every}))
}

fn flow_status(s: &mut Session, p: &Value) -> Result<Value> {
    s.poll_flow();
    if str_(p, "study").is_some() {
        let name = study_name(s, p, "sim.flow.status")?;
        return s.flow.get(name).map(flow::status).ok_or_else(|| bad("sim.flow.status", "no such study"));
    }
    Ok(json!({"studies": s.flow.list.iter().map(flow::status).collect::<Vec<_>>()}))
}

fn flow_results(s: &mut Session, p: &Value) -> Result<Value> {
    let cmd = "sim.flow.results";
    s.poll_flow();
    let name = study_name(s, p, cmd)?;
    let st = s.flow.get(name).ok_or_else(|| bad(cmd, "no such study"))?;
    let r = st.result.as_ref().ok_or_else(|| {
        bad(cmd, format!("`{name}` has no results yet ({})", flow::status(st).get("state").and_then(Value::as_str).unwrap_or("ready")))
    })?;
    Ok(flow::results(name, r))
}

fn flow_probe(s: &mut Session, p: &Value) -> Result<Value> {
    let cmd = "sim.flow.probe";
    s.poll_flow();
    let name = study_name(s, p, cmd)?;
    let r = s.flow.get(name).and_then(|st| st.result.clone()).ok_or_else(|| bad(cmd, format!("`{name}` has no results yet")))?;
    if let Some(g) = p.get("grid") {
        // A grid of samples: [x, y, z, ux, uy, uz, pressure] at each point in the air at or over
        // `min_speed`.
        let lo = vec3_of(g.get("min"), cmd, "grid.min")?;
        let hi = vec3_of(g.get("max"), cmd, "grid.max")?;
        let step = g
            .get("step")
            .and_then(Value::as_f64)
            .filter(|x| x.is_finite() && *x > 0.0)
            .ok_or_else(|| bad(cmd, "`grid.step` must be a positive number, mm"))?;
        let floor = g.get("min_speed").and_then(Value::as_f64).filter(|x| x.is_finite()).unwrap_or(0.0);
        let count = |a: f64, b: f64| ((b - a) / step).floor() + 1.0;
        let (nx, ny, nz) = (count(lo.x, hi.x), count(lo.y, hi.y), count(lo.z, hi.z));
        if !(nx >= 1.0 && ny >= 1.0 && nz >= 1.0) || nx * ny * nz > 4.0e6 {
            return Err(bad(cmd, "`grid` must run from min to max in at most 4,000,000 points"));
        }
        let mut out = Vec::new();
        for i in 0..nx as usize {
            for j in 0..ny as usize {
                for k in 0..nz as usize {
                    let at = lo + solidcraft_geom::Vec3::new(i as f64, j as f64, k as f64) * step;
                    if let Some((u, pressure)) = r.fields.sample(at)
                        && u.len() >= floor
                    {
                        out.push([at.x, at.y, at.z, u.x, u.y, u.z, pressure]);
                    }
                }
            }
        }
        return Ok(json!({"columns": ["x", "y", "z", "ux", "uy", "uz", "pressure_pa"], "samples": out}));
    }
    let at = crate::params::req_vec3(cmd, p, "point")?;
    Ok(match r.fields.sample(at) {
        Some((u, pressure)) => json!({"velocity_ms": [u.x, u.y, u.z], "speed_ms": u.len(), "pressure_pa": pressure}),
        None => json!(null),
    })
}

fn flow_cancel(s: &mut Session, p: &Value) -> Result<Value> {
    let name = study_name(s, p, "sim.flow.cancel")?.to_string();
    if let Some(job) = s.flow.get(&name).and_then(|st| st.job.as_ref()) {
        job.cancel();
    }
    Ok(json!({"study": name}))
}

fn flow_delete(s: &mut Session, p: &Value) -> Result<Value> {
    let name = study_name(s, p, "sim.flow.delete")?.to_string();
    s.flow.list.retain(|st| st.name != name);
    Ok(json!({"deleted": name}))
}

fn opening_json(o: &OpeningSpec) -> Value {
    let p = [o.point.x, o.point.y, o.point.z];
    let mut v = match o.section {
        Some(n) => json!({"name": o.name, "body": o.body, "section": {"point": p, "normal": [n.x, n.y, n.z]}}),
        None => json!({"name": o.name, "body": o.body, "face": p}),
    };
    if let Some(r) = o.region {
        v["region"] = json!(if r == Region::Hole { "hole" } else { "face" });
    }
    match &o.source {
        Some(sim::Source::Speed(u)) => v["speed"] = json!(u),
        Some(sim::Source::Fan(c)) => v["fan"] = json!({"curve": c.points().iter().map(|(q, pa)| [q * CFM, *pa]).collect::<Vec<_>>()}),
        None => {}
    }
    v
}

fn flow_list(s: &mut Session, _: &Value) -> Result<Value> {
    s.poll_flow();
    let studies: Vec<Value> = s
        .flow
        .list
        .iter()
        .map(|st| {
            json!({
                "study": st.name,
                "body": st.spec.body,
                "bodies": st.spec.bodies,
                "inlets": st.spec.inlets.iter().map(opening_json).collect::<Vec<_>>(),
                "outlets": st.spec.outlets.iter().map(opening_json).collect::<Vec<_>>(),
                "cell": st.spec.cell,
                "pad": st.spec.pad,
                "turbulence": st.spec.turbulence,
                "roughness_mm": st.spec.roughness,
                "max_steps": st.spec.max_steps,
                "state": flow::status(st).get("state").cloned().unwrap_or(Value::Null),
            })
        })
        .collect();
    Ok(json!({"studies": studies}))
}

// ---------------------------------------------------------------------------------------------
// Goal Seek

const GOAL_DOC: &str = "goals: [{\"goal\": \"maximize\" | \"minimize\", \"measure\", \"quantity\"?, \"weight\"?} | {\"goal\": \"target\", \"measure\", \"quantity\"?, \"value\", \"weight\"?} | {\"goal\": \"equalize\", \"measures\": [...], \"quantity\"?, \"weight\"?} | {\"goal\": \"limit\", \"measure\", \"quantity\"?, \"min\"?, \"max\"?}]; a measure is an opening's name, `pressure_drop`, or a region's name; quantity: flow (m³/s, default) | pressure (Pa) | speed (m/s) | max_speed";

fn seek_name<'a>(s: &Session, p: &'a Value, cmd: &str) -> Result<&'a str> {
    let name = str_(p, "seek").ok_or_else(|| bad(cmd, "`seek` is required: the goal seek's name"))?;
    if !s.flow.seeks.iter().any(|k| k.name == name) {
        return Err(bad(cmd, format!("no goal seek `{name}`")));
    }
    Ok(name)
}

fn vec3_of(v: Option<&Value>, cmd: &str, what: &str) -> Result<solidcraft_geom::Vec3> {
    v.and_then(crate::params::vec3).filter(|p| p.is_finite()).ok_or_else(|| bad(cmd, format!("`{what}` must be [x, y, z] mm")))
}

fn seek_spec(s: &mut Session, p: &Value, cmd: &str) -> Result<seek::SeekSpec> {
    let study = study_name(s, p, cmd)?.to_string();
    let rows = s.execute("parameters.list", &json!({}))?;
    let rows = rows.get("parameters").and_then(Value::as_array).cloned().unwrap_or_default();
    let vary_list = p
        .get("vary")
        .and_then(Value::as_array)
        .ok_or_else(|| bad(cmd, "`vary` lists 1 to 3 parameters: {\"parameter\", \"min\", \"max\", \"count\"?}"))?;
    if vary_list.is_empty() || vary_list.len() > 3 {
        return Err(bad(cmd, "vary 1 to 3 parameters"));
    }
    let mut vary = Vec::new();
    for v in vary_list {
        let name = str_(v, "parameter").ok_or_else(|| bad(cmd, "each `vary` entry names its `parameter`"))?;
        let row = rows
            .iter()
            .find(|r| r.get("name").and_then(Value::as_str) == Some(name))
            .ok_or_else(|| bad(cmd, format!("no parameter `{name}` (parameters.list)")))?;
        let unit = row.get("unit").and_then(Value::as_str).unwrap_or("").to_string();
        let (min, max) = match (num(v, "min"), num(v, "max")) {
            (Some(a), Some(b)) if a.is_finite() && b.is_finite() && a < b => (a, b),
            _ => {
                return Err(bad(
                    cmd,
                    format!("`{name}`: give `min` below `max`, in the parameter's unit ({})", if unit.is_empty() { "none" } else { &unit }),
                ));
            }
        };
        let count = match num(v, "count") {
            None => 5,
            Some(c) if c.fract() == 0.0 && (2.0..=50.0).contains(&c) => c as usize,
            Some(_) => return Err(bad(cmd, format!("`{name}`: `count` is 2…50 values"))),
        };
        vary.push(seek::Vary { parameter: name.to_string(), min, max, count, unit });
    }
    let mode = match p.get("mode").and_then(Value::as_str).unwrap_or("sweep") {
        "sweep" => seek::Mode::Sweep,
        "optimize" | "optimise" => {
            let budget = match num(p, "budget") {
                None => 20,
                Some(b) if b.fract() == 0.0 && (3.0..=seek::MAX_CYCLES as f64).contains(&b) => b as usize,
                Some(_) => return Err(bad(cmd, format!("`budget` is 3…{} cycles", seek::MAX_CYCLES))),
            };
            seek::Mode::Optimize { budget }
        }
        other => return Err(bad(cmd, format!("mode `{other}`: use sweep or optimize"))),
    };
    let mut regions = Vec::new();
    for r in p.get("regions").and_then(Value::as_array).into_iter().flatten() {
        let name = str_(r, "name").filter(|n| !n.is_empty() && n.len() <= 128).ok_or_else(|| bad(cmd, "each region has a `name`"))?.to_string();
        let shape = if let Some(b) = r.get("box") {
            let (min, max) = (vec3_of(b.get("min"), cmd, "box.min")?, vec3_of(b.get("max"), cmd, "box.max")?);
            if !(min.x < max.x && min.y < max.y && min.z < max.z) {
                return Err(bad(cmd, format!("region `{name}`: the box's min must be below its max")));
            }
            seek::Shape::Box { min, max }
        } else if let Some(d) = r.get("disc") {
            let radius = num(d, "radius")
                .filter(|r| r.is_finite() && *r > 0.0 && *r < 1e5)
                .ok_or_else(|| bad(cmd, format!("region `{name}`: the disc needs a positive `radius`")))?;
            let normal = vec3_of(d.get("normal"), cmd, "disc.normal")?;
            if normal.normalized().is_none() {
                return Err(bad(cmd, format!("region `{name}`: the disc's normal is zero")));
            }
            seek::Shape::Disc { centre: vec3_of(d.get("center"), cmd, "disc.center")?, normal, radius }
        } else {
            return Err(bad(cmd, format!("region `{name}`: give a `box` {{min, max}} or a `disc` {{center, normal, radius}}")));
        };
        regions.push(seek::Region { name, shape });
    }
    let spec = s.flow.get(&study).map(|st| st.spec.clone()).ok_or_else(|| bad(cmd, "no such study"))?;
    let known: Vec<String> = spec
        .inlets
        .iter()
        .chain(&spec.outlets)
        .map(|o| o.name.clone())
        .chain(std::iter::once("pressure_drop".to_string()))
        .chain(regions.iter().map(|r| r.name.clone()))
        .collect();
    let measure = |g: &Value, k: &str| -> Result<String> {
        let m = str_(g, k).ok_or_else(|| bad(cmd, format!("a goal needs `{k}`. {GOAL_DOC}")))?;
        if !known.iter().any(|n| n == m) {
            return Err(bad(cmd, format!("no measure `{m}`: use {}", known.join(", "))));
        }
        Ok(m.to_string())
    };
    let goals_list = p.get("goals").and_then(Value::as_array).ok_or_else(|| bad(cmd, format!("`goals` is required. {GOAL_DOC}")))?;
    if goals_list.is_empty() || goals_list.len() > 16 {
        return Err(bad(cmd, "give 1 to 16 goals"));
    }
    let mut goals = Vec::new();
    for g in goals_list {
        let quantity = match str_(g, "quantity") {
            None => seek::Quantity::Flow,
            Some(q) => seek::Quantity::parse(q).ok_or_else(|| bad(cmd, format!("quantity `{q}`: use flow, pressure, speed or max_speed")))?,
        };
        let weight = match num(g, "weight") {
            None => 1.0,
            Some(w) if w.is_finite() && w > 0.0 && w <= 1e6 => w,
            Some(_) => return Err(bad(cmd, "a goal's `weight` is positive")),
        };
        goals.push(match str_(g, "goal") {
            Some("maximize") => seek::Goal::Maximize { measure: measure(g, "measure")?, quantity, weight },
            Some("minimize") => seek::Goal::Minimize { measure: measure(g, "measure")?, quantity, weight },
            Some("target") => {
                let value = num(g, "value").filter(|v| v.is_finite()).ok_or_else(|| bad(cmd, "a target goal needs its `value`"))?;
                seek::Goal::Target { measure: measure(g, "measure")?, quantity, value, weight }
            }
            Some("equalize") => {
                let ms = g.get("measures").and_then(Value::as_array).ok_or_else(|| bad(cmd, "an equalize goal lists its `measures`"))?;
                let mut names = Vec::new();
                for m in ms {
                    let m = m.as_str().ok_or_else(|| bad(cmd, "`measures` are names"))?;
                    if !known.iter().any(|n| n == m) {
                        return Err(bad(cmd, format!("no measure `{m}`")));
                    }
                    names.push(m.to_string());
                }
                if names.len() < 2 {
                    return Err(bad(cmd, "equalize at least 2 measures"));
                }
                seek::Goal::Equalize { measures: names, quantity, weight }
            }
            Some("limit") => {
                let (min, max) = (num(g, "min").filter(|v| v.is_finite()), num(g, "max").filter(|v| v.is_finite()));
                if min.is_none() && max.is_none() {
                    return Err(bad(cmd, "a limit needs `min`, `max` or both"));
                }
                seek::Goal::Limit { measure: measure(g, "measure")?, quantity, min, max }
            }
            _ => return Err(bad(cmd, format!("each goal names its kind. {GOAL_DOC}"))),
        });
    }
    Ok(seek::SeekSpec { study, vary, mode, goals, regions })
}

fn seek_new(s: &mut Session, p: &Value) -> Result<Value> {
    let cmd = "sim.seek.new";
    let spec = seek_spec(s, p, cmd)?;
    let name = match str_(p, "name") {
        Some(n) if !n.is_empty() && n.len() <= 128 => n.to_string(),
        Some(_) => return Err(bad(cmd, "a goal seek's name is 1…128 characters")),
        None => (1..).map(|i| format!("Goal Seek {i}")).find(|n| !s.flow.seeks.iter().any(|k| k.name == *n)).unwrap_or_else(|| "Goal Seek".into()),
    };
    let k = seek::Seek::new(name.clone(), spec);
    let cycles = k.plan.total();
    if cycles > seek::MAX_CYCLES {
        return Err(bad(cmd, format!("{cycles} cycles: at most {}; use fewer values", seek::MAX_CYCLES)));
    }
    // A cycle starts from the last one's flow, so it takes about half a cold run.
    let last = s.flow.get(&k.spec.study).and_then(|st| st.result.as_ref()).map(|r| r.seconds);
    let estimate = last.map(|t| t * (1.0 + 0.5 * (cycles.saturating_sub(1)) as f64));
    s.flow.seeks.retain(|x| x.name != name);
    s.flow.seeks.push(k);
    Ok(json!({"seek": name, "cycles": cycles, "estimate_s": estimate}))
}

fn seek_run(s: &mut Session, p: &Value) -> Result<Value> {
    let cmd = "sim.seek.run";
    let name = seek_name(s, p, cmd)?.to_string();
    let wait = bool_(p, "wait").unwrap_or(false);
    let i = s.flow.seeks.iter().position(|k| k.name == name).ok_or_else(|| bad(cmd, "no such goal seek"))?;
    if let Some(k) = s.flow.seeks.get_mut(i) {
        if k.state == seek::SeekState::Running {
            return Err(bad(cmd, format!("`{name}` is running; sim.seek.cancel stops it")));
        }
        // A run starts over.
        let spec = k.spec.clone();
        *k = seek::Seek::new(name.clone(), spec);
        k.state = seek::SeekState::Running;
        k.wait = wait;
    }
    s.poll_flow();
    if wait {
        while s.flow.seeks.get(i).is_some_and(|k| k.state == seek::SeekState::Running) {
            s.poll_flow();
            #[cfg(not(target_arch = "wasm32"))]
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
        return seek_results(s, &json!({"seek": name}));
    }
    Ok(json!({"seek": name, "state": "running"}))
}

fn seek_state(k: &seek::Seek) -> &'static str {
    match k.state {
        seek::SeekState::Ready => "ready",
        seek::SeekState::Running => "running",
        seek::SeekState::Done => "done",
        seek::SeekState::Cancelled => "cancelled",
    }
}

fn seek_status(s: &mut Session, p: &Value) -> Result<Value> {
    s.poll_flow();
    let one = |k: &seek::Seek| {
        let best = k.best_cycle().map(|(i, c)| seek::cycle_json(&k.spec, i, c));
        json!({"seek": k.name, "study": k.spec.study, "state": seek_state(k), "cycles_done": k.cycles.len(), "cycles": k.plan.total(), "best": best})
    };
    if str_(p, "seek").is_some() {
        let name = seek_name(s, p, "sim.seek.status")?;
        return s.flow.seeks.iter().find(|k| k.name == name).map(one).ok_or_else(|| bad("sim.seek.status", "no such goal seek"));
    }
    Ok(json!({"seeks": s.flow.seeks.iter().map(one).collect::<Vec<_>>()}))
}

fn seek_results(s: &mut Session, p: &Value) -> Result<Value> {
    s.poll_flow();
    let name = seek_name(s, p, "sim.seek.results")?;
    let k = s.flow.seeks.iter().find(|k| k.name == name).ok_or_else(|| bad("sim.seek.results", "no such goal seek"))?;
    Ok(json!({
        "seek": k.name,
        "state": seek_state(k),
        "spec": seek::spec_json(&k.spec),
        "cycles": k.cycles.iter().enumerate().map(|(i, c)| seek::cycle_json(&k.spec, i, c)).collect::<Vec<_>>(),
        "best": k.best_cycle().map(|(i, _)| i),
    }))
}

fn seek_apply(s: &mut Session, p: &Value) -> Result<Value> {
    let cmd = "sim.seek.apply";
    let name = seek_name(s, p, cmd)?.to_string();
    let k = s.flow.seeks.iter().find(|k| k.name == name).ok_or_else(|| bad(cmd, "no such goal seek"))?;
    let i = match num(p, "cycle") {
        Some(c) if c.fract() == 0.0 && c >= 0.0 => c as usize,
        Some(_) => return Err(bad(cmd, "`cycle` is a cycle's number")),
        None => k.best_cycle().map(|(i, _)| i).ok_or_else(|| bad(cmd, format!("`{name}` has no good cycle yet")))?,
    };
    let c = k.cycles.get(i).ok_or_else(|| bad(cmd, format!("`{name}` has no cycle {i}")))?;
    let changes: serde_json::Map<String, Value> =
        k.spec.vary.iter().zip(&c.values).map(|(v, x)| (v.parameter.clone(), json!(format!("{x} {}", v.unit).trim().to_string()))).collect();
    // One parameters.change: one undo step for the whole cycle.
    let out = s.execute("parameters.change", &json!({"changes": changes}))?;
    Ok(json!({"seek": name, "cycle": i, "applied": changes, "changed": out.get("changed")}))
}

fn seek_cancel(s: &mut Session, p: &Value) -> Result<Value> {
    let name = seek_name(s, p, "sim.seek.cancel")?.to_string();
    if let Some(k) = s.flow.seeks.iter_mut().find(|k| k.name == name) {
        k.cancel = true;
        if let Some((_, job)) = &k.running {
            job.cancel();
        }
    }
    Ok(json!({"seek": name}))
}

fn seek_delete(s: &mut Session, p: &Value) -> Result<Value> {
    let name = seek_name(s, p, "sim.seek.delete")?.to_string();
    s.flow.seeks.retain(|k| k.name != name);
    Ok(json!({"deleted": name}))
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use crate::Session;

    #[test]
    fn studies_are_registered_on_the_simulation_tab() {
        for (id, label) in [("sim.flow.new", "Airflow Study"), ("sim.aero.new", "Aerodynamics Study"), ("sim.gas.new", "Gas Dynamics Study")] {
            let spec = crate::find_command(id).unwrap();
            assert_eq!((spec.label, spec.tab, spec.panel), (label, "SIMULATION", "STUDY"));
            assert!(!spec.undoable, "{id}");
            assert!(spec.planned.is_none(), "{id} opens its dialog, so it is not a greyed planned command");
        }
    }

    #[test]
    fn the_other_studies_check_their_body_then_say_the_solver_is_not_built() {
        let mut s = Session::default();
        s.execute("solid.box", &json!({"length": 10, "width": 10, "height": 10, "body_name": "Duct"})).unwrap();
        let before = serde_json::to_string(&*s.doc).unwrap();
        let undo = s.undo.len();
        for id in ["sim.aero.new", "sim.gas.new"] {
            let missing = s.execute(id, &json!({})).unwrap_err().to_string();
            assert!(missing.contains("`body` is required"), "{id}: {missing}");
            let wrong = s.execute(id, &json!({"body": "Nope"})).unwrap_err().to_string();
            assert!(wrong.contains("no body `Nope`"), "{id}: {wrong}");
            let refused = s.execute(id, &json!({"body": "Duct"})).unwrap_err().to_string();
            assert!(refused.contains("not supported yet"), "{id}: {refused}");
        }
        let flow = s.execute("sim.flow.new", &json!({"body": "Duct"})).unwrap_err().to_string();
        assert!(flow.contains("`inlets` is required"), "{flow}");
        assert_eq!(serde_json::to_string(&*s.doc).unwrap(), before, "a refused study leaves the design alone");
        assert_eq!(s.undo.len(), undo);
    }

    /// A square duct (8 mm bore, 2 mm walls, 24 mm long along x) cut from a box.
    fn duct(s: &mut Session) {
        s.execute("solid.box", &json!({"length": 24, "width": 12, "height": 12, "body_name": "Duct"})).unwrap();
        s.execute("solid.box", &json!({"corner": [-1, 2, 2], "length": 26, "width": 8, "height": 8, "operation": "cut", "targets": ["Duct"]}))
            .unwrap();
    }

    /// The whole path headless: a duct cut in the modeller, its open ends picked by their rims, run
    /// on the CPU to a settled flow; what goes in comes out, and nothing in the design changed.
    #[test]
    fn an_airflow_study_runs_through_a_modelled_duct() {
        let mut s = Session::default();
        duct(&mut s);
        let before = serde_json::to_string(&*s.doc).unwrap();
        let set = s
            .execute(
                "sim.flow.new",
                &json!({"body": "Duct", "inlets": [{"face": [0, 1, 6], "speed": 0.05, "name": "Fan"}], "outlets": [{"face": [24, 1, 6], "name": "Nozzle"}], "cell": 1, "pad": 4, "turbulence": false}),
            )
            .unwrap();
        assert_eq!(set["study"], "Airflow 1");
        assert_eq!(set["inlets"][0]["cells"], 64, "{set}");
        assert!(set["warnings"].as_array().unwrap().is_empty(), "{set}");
        let r = s.execute("sim.flow.run", &json!({"study": "Airflow 1", "wait": true})).unwrap();
        assert_eq!(r["state"], "settled", "{r}");
        let (q_in, q_out) = (r["inlets"][0]["flow_m3s"].as_f64().unwrap(), r["outlets"][0]["flow_m3s"].as_f64().unwrap());
        assert!((q_in - 0.05 * 64e-6).abs() / (0.05 * 64e-6) < 0.08, "{r}");
        assert!((q_out - q_in).abs() / q_in < 0.03, "{r}");
        assert!(r["pressure_drop_pa"].as_f64().unwrap() > 0.0, "{r}");
        assert_eq!(r["outlet_shares"][0]["share"], 1.0);
        let probe = s.execute("sim.flow.probe", &json!({"study": "Airflow 1", "point": [12, 6, 6]})).unwrap();
        assert!(probe["velocity_ms"][0].as_f64().unwrap() > 0.05, "the centre runs faster than the mean: {probe}");
        assert!(s.execute("sim.flow.probe", &json!({"study": "Airflow 1", "point": [12, 0.5, 6]})).unwrap().is_null(), "inside the wall");
        let grid = s
            .execute(
                "sim.flow.probe",
                &json!({"study": "Airflow 1", "grid": {"min": [12, 0.5, 6], "max": [12, 11.5, 6], "step": 1, "min_speed": 0.01}}),
            )
            .unwrap();
        let rows = grid["samples"].as_array().unwrap();
        assert!(!rows.is_empty() && rows.len() < 12, "the bore's points, not the walls': {grid}");
        assert!(
            s.execute("sim.flow.probe", &json!({"study": "Airflow 1", "grid": {"min": [0, 0, 0], "max": [1e6, 1e6, 1e6], "step": 0.1}})).is_err()
        );
        assert_eq!(s.execute("sim.flow.status", &json!({"study": "Airflow 1"})).unwrap()["state"], "done");
        assert_eq!(serde_json::to_string(&*s.doc).unwrap(), before, "studies never touch the design");
        let listed = s.execute("sim.flow.list", &json!({})).unwrap();
        assert_eq!(listed["studies"][0]["inlets"][0]["speed"], 0.05);
        // Editing clears the results; deleting removes the study.
        s.execute("sim.flow.edit", &json!({"study": "Airflow 1", "inlets": [{"face": [0, 1, 6], "fan": {"type": "radial", "free_flow_cfm": 0.01, "max_pressure_pa": 0.05}}]})).unwrap();
        assert!(s.execute("sim.flow.results", &json!({"study": "Airflow 1"})).is_err());
        s.execute("sim.flow.delete", &json!({"study": "Airflow 1"})).unwrap();
        assert!(s.execute("sim.flow.list", &json!({})).unwrap()["studies"].as_array().unwrap().is_empty());
    }

    /// A rim with screw holes through it (a fan flange): the bore is the opening, the screw holes
    /// are not.
    #[test]
    fn a_flanges_screw_holes_are_not_part_of_the_opening() {
        let mut s = Session::default();
        duct(&mut s);
        s.execute("solid.box", &json!({"corner": [-2, -4, -4], "length": 2, "width": 20, "height": 20, "operation": "join", "targets": ["Duct"]}))
            .unwrap();
        s.execute("solid.box", &json!({"corner": [-3, 2, 2], "length": 4, "width": 8, "height": 8, "operation": "cut", "targets": ["Duct"]}))
            .unwrap();
        for (y, z) in [(-3, -3), (-3, 14), (14, -3), (14, 14)] {
            s.execute("solid.box", &json!({"corner": [-3, y, z], "length": 4, "width": 1, "height": 1, "operation": "cut", "targets": ["Duct"]}))
                .unwrap();
        }
        let set = s
            .execute(
                "sim.flow.new",
                &json!({"body": "Duct", "inlets": [{"face": [-2, 0, 6], "speed": 0.05}], "outlets": [{"face": [24, 1, 6]}], "cell": 1, "pad": 4}),
            )
            .unwrap();
        assert_eq!(set["inlets"][0]["cells"], 64, "{set}");
    }

    /// A duct that comes in as a mesh (an STL has a face per triangle): picking its rim takes the
    /// whole flat rim, so the openings are the same as the modelled duct's.
    #[test]
    fn a_mesh_ducts_rim_is_picked_whole() {
        let dir = std::env::temp_dir().join(format!("solidcraft-sim-mesh-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let stl = dir.join("duct.stl");
        let mut s = Session::default();
        duct(&mut s);
        s.execute("file.export", &json!({"path": stl.to_string_lossy()})).unwrap();
        let mut m = Session::default();
        let opened = m.execute("doc.open", &json!({"path": stl.to_string_lossy()})).unwrap();
        let body = opened["bodies"][0].as_str().or_else(|| opened["bodies"][0]["name"].as_str()).unwrap().to_string();
        let set = m
            .execute(
                "sim.flow.new",
                &json!({"body": body, "inlets": [{"face": [0, 1, 6], "speed": 0.05}], "outlets": [{"face": [24, 1, 6]}], "cell": 1, "pad": 4}),
            )
            .unwrap();
        assert_eq!(set["inlets"][0]["cells"], 64, "{set}");
        assert_eq!(set["outlets"][0]["cells"], 64, "{set}");
        // A section across it partway along takes the bore there, and closes the duct behind an inlet.
        let sec = m
            .execute(
                "sim.flow.new",
                &json!({"body": body, "name": "Sections", "inlets": [{"section": {"point": [6, 6, 6], "normal": [1, 0, 0]}, "speed": 0.05}], "outlets": [{"section": {"point": [18, 6, 6], "normal": [1, 0, 0]}}], "cell": 1, "pad": 4}),
            )
            .unwrap();
        assert_eq!(sec["inlets"][0]["cells"], 64, "{sec}");
        assert_eq!(sec["outlets"][0]["cells"], 64, "{sec}");
        let listed = m.execute("sim.flow.list", &json!({})).unwrap();
        assert_eq!(listed["studies"][1]["inlets"][0]["section"]["normal"], json!([1.0, 0.0, 0.0]), "{listed}");
        let outside =
            m.execute("sim.flow.new", &json!({"body": body, "inlets": [{"section": {"point": [6, 1, 1], "normal": [1, 0, 0]}, "speed": 1}]}));
        assert!(outside.unwrap_err().to_string().contains("not inside the duct"));
        std::fs::remove_dir_all(&dir).ok();
    }

    /// Goal Seek on a duct whose bore is a design parameter: the sweep runs every value on a
    /// scratch copy (the design keeps its bore), the widest bore carries the most air at a set
    /// speed, applying the best cycle is one undo step, and an optimization aimed at the flow a
    /// bore of 8 mm gives finds a bore that gives it.
    #[test]
    fn goal_seek_sweeps_and_optimizes_a_design_parameter() {
        let mut s = Session::default();
        s.execute("parameters.add", &json!({"name": "bore", "expression": "8 mm"})).unwrap();
        s.execute("solid.box", &json!({"length": 24, "width": 12, "height": 12, "body_name": "Duct"})).unwrap();
        s.execute("solid.box", &json!({"corner": [-1, "6 mm - bore / 2", "6 mm - bore / 2"], "length": 26, "width": "bore", "height": "bore", "operation": "cut", "targets": ["Duct"]})).unwrap();
        s.execute(
            "sim.flow.new",
            &json!({"body": "Duct", "name": "D", "inlets": [{"face": [0, 0.5, 6], "speed": 0.05, "name": "In"}], "outlets": [{"face": [24, 0.5, 6], "name": "Out"}], "cell": 1, "pad": 4, "turbulence": false}),
        )
        .unwrap();
        let made = s.execute("sim.seek.new", &json!({"study": "D", "vary": [{"parameter": "bore", "min": 6, "max": 10, "count": 3}], "goals": [{"goal": "maximize", "measure": "Out"}]})).unwrap();
        assert_eq!(made["cycles"], 3);
        let r = s.execute("sim.seek.run", &json!({"seek": "Goal Seek 1", "wait": true})).unwrap();
        let cycles = r["cycles"].as_array().unwrap();
        assert_eq!(cycles.len(), 3, "{r}");
        let flow = |c: &serde_json::Value| c["measures"]["Out"]["flow_m3s"].as_f64().unwrap();
        assert!(flow(&cycles[0]) < flow(&cycles[1]) && flow(&cycles[1]) < flow(&cycles[2]), "{r}");
        // Area grows as bore²: 6 → 10 mm is (10/6)² ≈ 2.8 times the air.
        let ratio = flow(&cycles[2]) / flow(&cycles[0]);
        assert!(ratio > 2.2 && ratio < 3.2, "{ratio}");
        assert_eq!(r["best"], 2);
        let bore = |s: &mut Session| {
            s.execute("parameters.list", &json!({})).unwrap()["parameters"].as_array().unwrap().iter().find(|p| p["name"] == "bore").unwrap()["value"]
                .as_f64()
                .unwrap()
        };
        assert!((bore(&mut s) - 8.0).abs() < 1e-9, "the design keeps its bore while seeking");
        let undo = s.undo.len();
        s.execute("sim.seek.apply", &json!({"seek": "Goal Seek 1"})).unwrap();
        assert!((bore(&mut s) - 10.0).abs() < 1e-9);
        assert_eq!(s.undo.len(), undo + 1, "one undo step");
        s.execute("edit.undo", &json!({})).unwrap();
        assert!((bore(&mut s) - 8.0).abs() < 1e-9);
        // Optimize toward the flow the 8 mm bore gives (the middle cycle).
        let target = flow(&cycles[1]);
        s.execute("sim.seek.new", &json!({"study": "D", "name": "Hit", "mode": "optimize", "budget": 10, "vary": [{"parameter": "bore", "min": 5, "max": 11}], "goals": [{"goal": "target", "measure": "Out", "value": target}]})).unwrap();
        let o = s.execute("sim.seek.run", &json!({"seek": "Hit", "wait": true})).unwrap();
        let best = &o["cycles"][o["best"].as_u64().unwrap() as usize];
        // On 1 mm cells every bore that cuts the same 8 cells carries that flow: the optimizer
        // lands on that plateau.
        let found = best["values"]["bore"].as_f64().unwrap();
        assert!((flow(best) - target).abs() / target < 0.01, "flow {} for the target {target}: {o}", flow(best));
        assert!((6.9..=9.1).contains(&found), "bore {found} for the target flow: {o}");
        assert!(o["cycles"].as_array().unwrap().len() <= 10);
        for bad in [
            json!({"study": "D", "vary": [], "goals": [{"goal": "maximize", "measure": "Out"}]}),
            json!({"study": "D", "vary": [{"parameter": "nope", "min": 1, "max": 2}], "goals": [{"goal": "maximize", "measure": "Out"}]}),
            json!({"study": "D", "vary": [{"parameter": "bore", "min": 9, "max": 2}], "goals": [{"goal": "maximize", "measure": "Out"}]}),
            json!({"study": "D", "vary": [{"parameter": "bore", "min": 6, "max": 9}], "goals": [{"goal": "maximize", "measure": "Elsewhere"}]}),
            json!({"study": "D", "vary": [{"parameter": "bore", "min": 6, "max": 9}], "goals": [{"goal": "limit", "measure": "Out"}]}),
            json!({"study": "D", "vary": [{"parameter": "bore", "min": 6, "max": 9, "count": 50}, {"parameter": "bore", "min": 6, "max": 9, "count": 50}], "goals": [{"goal": "maximize", "measure": "Out"}]}),
        ] {
            assert!(s.execute("sim.seek.new", &bad).is_err(), "{bad}");
        }
    }

    /// Studies and goal seeks live beside the design: saving writes `<design>.sim.json` (the design
    /// file is the same with or without them), opening the design brings them back, the seek with
    /// its cycles.
    #[test]
    fn studies_and_seeks_are_kept_beside_the_design() {
        let dir = std::env::temp_dir().join(format!("solidcraft-sim-sidecar-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("duct.solidcraft");
        let mut s = Session::default();
        s.execute("parameters.add", &json!({"name": "bore", "expression": "8 mm"})).unwrap();
        duct(&mut s);
        s.execute("sim.flow.new", &json!({"body": "Duct", "name": "D", "inlets": [{"face": [0, 1, 6], "speed": 0.05}], "outlets": [{"face": [24, 1, 6], "name": "Out"}], "cell": 1, "pad": 4, "turbulence": false})).unwrap();
        s.execute("sim.seek.new", &json!({"study": "D", "name": "Sweep", "vary": [{"parameter": "bore", "min": 6, "max": 8, "count": 2}], "goals": [{"goal": "maximize", "measure": "Out"}]})).unwrap();
        // Two cycles, as if run (the duct's cut does not use the parameter; the cycles' values do).
        if let Some(k) = s.flow.seeks.first_mut() {
            for (i, v) in [6.0, 8.0].into_iter().enumerate() {
                let m = [("Out".to_string(), crate::seek::Measured { flow: 1e-6 * (i + 1) as f64, ..Default::default() })].into();
                k.cycles.push(crate::seek::Cycle {
                    values: vec![v],
                    measures: m,
                    score: -(i as f64),
                    feasible: true,
                    error: None,
                    steps: 100,
                    seconds: 1.0,
                });
            }
        }
        s.execute("file.save", &json!({"path": path})).unwrap();
        let sidecar = dir.join("duct.sim.json");
        assert!(sidecar.exists(), "the studies go beside the design");
        let mut plain = Session::default();
        plain.execute("doc.open", &json!({"path": path})).unwrap();
        let design = plain.doc.to_json();
        let listed = plain.execute("sim.flow.list", &json!({})).unwrap();
        assert_eq!(listed["studies"][0]["study"], "D", "{listed}");
        assert_eq!(listed["studies"][0]["outlets"][0]["name"], "Out");
        let seek = plain.execute("sim.seek.results", &json!({"seek": "Sweep"})).unwrap();
        assert_eq!(seek["cycles"].as_array().unwrap().len(), 2, "{seek}");
        assert_eq!(seek["best"], 1);
        assert_eq!(seek["cycles"][1]["values"]["bore"], 8.0);
        // Saving a design without studies leaves its sidecar alone; the design file is unchanged
        // by having studies.
        let mut other = Session::default();
        other.execute("doc.open", &json!({"path": path})).unwrap();
        other.flow.list.clear();
        other.flow.seeks.clear();
        other.execute("file.save", &json!({})).unwrap();
        assert!(sidecar.exists());
        // The design as saved, not the file's bytes: its model cache keeps each feature's build
        // time, which differs from one save to the next.
        let mut again = Session::default();
        again.execute("doc.open", &json!({"path": path})).unwrap();
        assert_eq!(again.doc.to_json(), design);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn hostile_study_requests_are_errors() {
        let mut s = Session::default();
        duct(&mut s);
        let inlet = json!([{"face": [0, 1, 6], "speed": 0.05}]);
        for p in [
            json!({"body": "Duct", "inlets": []}),
            json!({"body": "Duct", "inlets": "x"}),
            json!({"body": "Duct", "inlets": [{"speed": 1}]}),
            json!({"body": "Duct", "inlets": [{"face": [0, 1, 6]}]}),
            json!({"body": "Duct", "inlets": [{"face": [0, 1, 6], "speed": -1}]}),
            json!({"body": "Duct", "inlets": [{"face": [0, 1, 6], "speed": 1e9}]}),
            json!({"body": "Duct", "inlets": [{"face": [0, 1, 6], "fan": {"type": "jet"}}]}),
            json!({"body": "Duct", "inlets": [{"face": [0, 1, 6], "fan": {"curve": [[1, 0], [0, 5]]}}]}),
            json!({"body": "Duct", "inlets": [{"face": [0, 1, 6], "speed": 1, "region": "middle"}]}),
            json!({"body": "Duct", "inlets": [{"face": [6, 1, 6], "speed": 1, "region": "hole"}]}),
            json!({"body": "Duct", "inlets": [{"face": [500, 500, 500], "speed": 1}]}),
            json!({"body": "Duct", "inlets": inlet, "cell": 0}),
            json!({"body": "Duct", "inlets": inlet, "cell": f64::MAX}),
            json!({"body": "Duct", "inlets": inlet, "cell": 0.05}),
            json!({"body": "Duct", "inlets": inlet, "pad": -1}),
            json!({"body": "Duct", "inlets": inlet, "bodies": ["Nope"]}),
            json!({"body": "Duct", "inlets": inlet, "surface": "sandpaper"}),
            json!({"body": "Duct", "inlets": inlet, "roughness_mm": -0.1}),
            json!({"body": "Duct", "inlets": inlet, "roughness_mm": f64::MAX}),
            json!({"body": "Duct", "inlets": inlet, "surface": "rough", "roughness_mm": 0.1}),
            json!({"body": "Duct", "inlets": inlet, "surface": "fdm"}),
        ] {
            assert!(s.execute("sim.flow.new", &p).is_err(), "{p}");
        }
        for (id, p) in [
            ("sim.flow.run", json!({"study": "Nope"})),
            ("sim.flow.results", json!({})),
            ("sim.flow.probe", json!({"study": "Nope", "point": [0, 0, 0]})),
        ] {
            assert!(s.execute(id, &p).is_err(), "{id} {p}");
        }
        assert!(s.flow.list.is_empty());
    }

    /// A surface finish is kept as its roughness, listed, and saved with the study; an edit
    /// that names neither keeps it.
    #[test]
    fn a_study_keeps_its_surface_finish() {
        let mut s = Session::default();
        duct(&mut s);
        let inlet = json!([{"face": [0, 1, 6], "speed": 0.05}]);
        s.execute("sim.flow.new", &json!({"body": "Duct", "name": "D", "inlets": inlet, "cell": 1, "pad": 4, "surface": "rough"})).unwrap();
        assert_eq!(s.flow.get("D").map(|st| st.spec.roughness), Some(0.06));
        let listed = s.execute("sim.flow.list", &json!({})).unwrap();
        assert!(listed.to_string().contains("\"roughness_mm\":0.06"), "{listed}");
        assert_eq!(crate::flow::spec_params("D", &s.flow.get("D").unwrap().spec)["roughness_mm"], json!(0.06));
        s.execute("sim.flow.edit", &json!({"study": "D", "pad": 5})).unwrap();
        assert_eq!(s.flow.get("D").map(|st| st.spec.roughness), Some(0.06));
        s.execute("sim.flow.edit", &json!({"study": "D", "roughness_mm": 0.2})).unwrap();
        assert_eq!(s.flow.get("D").map(|st| st.spec.roughness), Some(0.2));
    }
}
