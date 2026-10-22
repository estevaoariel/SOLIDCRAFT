//! UI macros: record commands as Rhai, play them in the engine (one undo, rollback on failure).

use std::cell::RefCell;
use std::path::{Path, PathBuf};
use std::rc::Rc;

use rhai::{Dynamic, Engine, EvalAltResult, ImmutableString, Scope};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use super::CommandSpec;
use crate::frames::TopLevel;
use crate::params::{bad, str_};
use crate::{EngineError, Result, Session};

/// The most commands one recording holds. Recording stops at it.
pub const MAX_STEPS: usize = 10_000;
const MAX_MACRO_BYTES: usize = 4 << 20;
const MAX_LISTED: usize = 200;
const MAX_OPS: u64 = 1_000_000;

/// A recorded or loaded Rhai macro.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct Macro {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub params: Vec<MacroParam>,
    /// Rhai source `macro.play` evaluates.
    #[serde(default)]
    pub source: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct MacroParam {
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub default: Option<Value>,
    /// Play shows a dropdown when this is not empty (`// @param name Label {a, b} | a`).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub choices: Vec<String>,
    /// Play shows this row only when another field's value is in `values` (`@if kind=sprocket`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub when: Option<MacroWhen>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct MacroWhen {
    pub name: String,
    pub values: Vec<String>,
}

#[derive(Clone, Debug, Default)]
pub struct Recorder {
    pub recording: bool,
    pub paused: bool,
    pub playing: bool,
    pub captured: Macro,
    pub loaded: Option<Macro>,
}

pub fn recordable(id: &str) -> bool {
    !(id.starts_with("macro.")
        || id.starts_with("file.")
        || id.starts_with("doc.")
        || id.starts_with("view.")
        || id.starts_with("ui.")
        || matches!(id, "edit.undo" | "edit.redo"))
}

impl Recorder {
    pub(crate) fn note(&mut self, id: &str, params: &Value) {
        if !self.recording || self.paused || self.playing || !recordable(id) {
            return;
        }
        if run_count(&self.captured.source) >= MAX_STEPS {
            self.recording = false;
            self.paused = false;
            if run_count(&self.captured.source) > 0 {
                self.loaded = Some(self.captured.clone());
            }
            return;
        }
        let params = match params.as_object() {
            Some(obj) => {
                let mut p = obj.clone();
                p.remove("live");
                if p.is_empty() { Value::Null } else { Value::Object(p) }
            }
            None => Value::Null,
        };
        append_run(&mut self.captured.source, id, &params);
    }

    pub fn steps(&self) -> usize {
        let n = run_count(&self.captured.source);
        if n > 0 { n } else { self.loaded.as_ref().map(|m| run_count(&m.source)).unwrap_or(0) }
    }
}

pub static COMMANDS: &[CommandSpec] = &[
    CommandSpec::new("macro.record", "Record Macro", record)
        .at("MACRO", "RECORD")
        .icon("record")
        .noundo()
        .doc_only()
        .enabled(idle)
        .params("name?: the macro's name → {recording, paused, steps}"),
    CommandSpec::new("macro.pause", "Pause Recording", pause)
        .at("MACRO", "RECORD")
        .icon("pause")
        .noundo()
        .doc_only()
        .enabled(capturing)
        .params("→ {recording, paused: true, steps}"),
    CommandSpec::new("macro.resume", "Resume Recording", resume)
        .at("MACRO", "RECORD")
        .icon("record")
        .noundo()
        .doc_only()
        .enabled(paused)
        .params("→ {recording, paused: false, steps}"),
    CommandSpec::new("macro.stop", "Stop Recording", stop)
        .at("MACRO", "RECORD")
        .icon("stop")
        .noundo()
        .doc_only()
        .enabled(recording)
        .params("→ {recording: false, steps}: the recording is loaded as Rhai"),
    CommandSpec::new("macro.play", "Play Macro", play)
        .at("MACRO", "PLAYBACK")
        .icon("play")
        .enabled(ready)
        .params("source?: Rhai text | path?: .rhai file | name?: file in the macros folder | params?: {name: value} (default: the loaded macro). Host: run(command, params), list_dir(path), write_file(path, text). One undo; a failure rolls back"),
    CommandSpec::new("macro.open", "Open Macro", open)
        .at("MACRO", "PLAYBACK")
        .icon("open")
        .noundo()
        .doc_only()
        .enabled(idle)
        .params("path: a .rhai file → name, steps, params"),
    CommandSpec::new("macro.save", "Save Macro", save)
        .at("MACRO", "PLAYBACK")
        .icon("export")
        .noundo()
        .doc_only()
        .enabled(has_steps)
        .params("name?: file in the macros folder ending in .rhai, path?: any .rhai file → {path, steps}"),
    CommandSpec::new("macro.clear", "Clear Macro", clear_macro)
        .at("MACRO", "PLAYBACK")
        .icon("delete")
        .noundo()
        .doc_only()
        .enabled(has_steps)
        .params("→ {cleared: true}"),
    CommandSpec::new("macro.list", "List Macros", list).icon("folder").noundo().doc_only().params("→ {recording, paused, steps, dir, macros}"),
    CommandSpec::new("macro.delete", "Delete Macro", delete).icon("delete").noundo().doc_only().params("name: a saved .rhai name"),
];

fn idle(s: &Session) -> std::result::Result<(), String> {
    if s.macros.recording {
        Err("already recording; stop the recording first".into())
    } else if s.macros.playing {
        Err("a macro is playing".into())
    } else {
        Ok(())
    }
}

fn recording(s: &Session) -> std::result::Result<(), String> {
    if s.macros.recording { Ok(()) } else { Err("nothing is being recorded".into()) }
}

fn capturing(s: &Session) -> std::result::Result<(), String> {
    if s.macros.recording && !s.macros.paused {
        Ok(())
    } else if s.macros.paused {
        Err("recording is paused; resume it first".into())
    } else {
        Err("nothing is being recorded".into())
    }
}

fn paused(s: &Session) -> std::result::Result<(), String> {
    if s.macros.paused { Ok(()) } else { Err("recording is not paused".into()) }
}

fn ready(s: &Session) -> std::result::Result<(), String> {
    idle(s).map_err(|why| match why.as_str() {
        "already recording; stop the recording first" => "stop the recording before playing a macro".into(),
        other => other.into(),
    })
}

fn has_steps(s: &Session) -> std::result::Result<(), String> {
    let loaded = s.macros.loaded.as_ref().is_some_and(|m| run_count(&m.source) > 0);
    if run_count(&s.macros.captured.source) == 0 && !loaded { Err("nothing has been recorded yet".into()) } else { Ok(()) }
}

fn record(s: &mut Session, p: &Value) -> Result<Value> {
    let cmd = "macro.record";
    if let Some(v) = p.get("name")
        && !v.is_null()
        && str_(p, "name").is_none()
    {
        return Err(bad(cmd, "`name` must be text"));
    }
    let name = str_(p, "name").map(str::to_string).or_else(|| Some(s.doc.name.clone()));
    let mut source = String::new();
    if let Some(n) = name.as_ref().filter(|n| !n.is_empty()) {
        source.push_str("// ");
        source.push_str(n);
        source.push('\n');
    }
    s.macros.captured = Macro { name, source, ..Default::default() };
    s.macros.recording = true;
    s.macros.paused = false;
    Ok(json!({"recording": true, "paused": false, "steps": 0}))
}

fn pause(s: &mut Session, _p: &Value) -> Result<Value> {
    s.macros.paused = true;
    Ok(json!({"recording": true, "paused": true, "steps": s.macros.steps()}))
}

fn resume(s: &mut Session, _p: &Value) -> Result<Value> {
    s.macros.paused = false;
    Ok(json!({"recording": true, "paused": false, "steps": s.macros.steps()}))
}

fn stop(s: &mut Session, _p: &Value) -> Result<Value> {
    s.macros.recording = false;
    s.macros.paused = false;
    let steps = run_count(&s.macros.captured.source);
    if steps > 0 {
        s.macros.loaded = Some(s.macros.captured.clone());
    }
    Ok(json!({"recording": false, "paused": false, "steps": steps}))
}

fn clear_macro(s: &mut Session, _p: &Value) -> Result<Value> {
    s.macros.captured = Macro::default();
    s.macros.loaded = None;
    Ok(json!({"cleared": true}))
}

pub fn macros_dir() -> Option<PathBuf> {
    crate::recovery::data_dir().map(|d| d.join("macros"))
}

fn named(cmd: &str, name: &str) -> Result<PathBuf> {
    if name.is_empty()
        || name.len() > 128
        || !name.ends_with(".rhai")
        || name.contains('/')
        || name.contains('\\')
        || name.contains("..")
        || !name.chars().all(|c| c.is_alphanumeric() || matches!(c, '.' | '_' | '-' | ' '))
    {
        return Err(bad(cmd, "`name` must be one plain file name ending in .rhai (letters, numbers, spaces, -_.)"));
    }
    let dir = macros_dir().ok_or_else(|| bad(cmd, "no macros folder on this system; give a path instead"))?;
    Ok(dir.join(name))
}

fn is_param_name(s: &str) -> bool {
    !s.is_empty() && s.len() <= 64 && s.chars().all(|c| c.is_alphanumeric() || c == '_' || c == '-')
}

fn is_choice(s: &str) -> bool {
    !s.is_empty() && s.len() <= 64 && s.chars().all(|c| c.is_alphanumeric() || matches!(c, '_' | '-' | '.'))
}

/// Split `… @if kind=sprocket` off a @param line.
fn param_when(cmd: &str, spec: &str) -> Result<(String, Option<MacroWhen>)> {
    let Some((left, rest)) = spec.split_once("@if") else {
        return Ok((spec.to_string(), None));
    };
    let rest = rest.trim();
    let Some((name, vals)) = rest.split_once('=') else {
        return Err(bad(cmd, "`@if` needs `name=value`"));
    };
    let name = name.trim();
    if !is_param_name(name) {
        return Err(bad(cmd, "`@if` needs a parameter name (letters, digits, `_` and `-`)"));
    }
    let mut values = Vec::new();
    for part in vals.split(',') {
        let c = part.trim();
        if c.is_empty() {
            return Err(bad(cmd, "`@if` values cannot be empty"));
        }
        if !is_choice(c) {
            return Err(bad(cmd, format!("`@if` value `{c}` uses letters, digits, `_`, `-` and `.`")));
        }
        if values.iter().any(|q: &String| q == c) {
            return Err(bad(cmd, format!("`@if` value `{c}` is given twice")));
        }
        values.push(c.to_string());
        if values.len() > 16 {
            return Err(bad(cmd, "`@if` has at most 16 values"));
        }
    }
    if values.is_empty() {
        return Err(bad(cmd, "`@if` needs at least one value"));
    }
    Ok((left.trim().to_string(), Some(MacroWhen { name: name.to_string(), values })))
}

/// Split `name Label {a, b}` into (`name Label`, [a, b]).
fn param_choices(cmd: &str, left: &str) -> Result<(String, Vec<String>)> {
    let Some(start) = left.find('{') else {
        return Ok((left.to_string(), Vec::new()));
    };
    let Some(end) = left.rfind('}') else {
        return Err(bad(cmd, "a @param choice list needs a closing `}`"));
    };
    if end < start {
        return Err(bad(cmd, "a @param choice list needs `{a, b}`"));
    }
    let inner = left.get(start + 1..end).unwrap_or("").trim();
    if inner.is_empty() {
        return Err(bad(cmd, "a @param choice list cannot be empty"));
    }
    let mut choices = Vec::new();
    for part in inner.split(',') {
        let c = part.trim();
        if c.is_empty() {
            return Err(bad(cmd, "a @param choice cannot be empty"));
        }
        if !is_choice(c) {
            return Err(bad(cmd, format!("choice `{c}` uses letters, digits, `_`, `-` and `.`")));
        }
        if choices.iter().any(|q: &String| q == c) {
            return Err(bad(cmd, format!("choice `{c}` is given twice")));
        }
        choices.push(c.to_string());
        if choices.len() > 64 {
            return Err(bad(cmd, "a @param has at most 64 choices"));
        }
    }
    let before = left.get(..start).unwrap_or("").trim();
    let after = left.get(end + 1..).unwrap_or("").trim();
    let rest = [before, after].into_iter().filter(|s| !s.is_empty()).collect::<Vec<_>>().join(" ");
    Ok((rest, choices))
}

fn read_macro_file(cmd: &str, path: &Path) -> Result<String> {
    let text =
        solidcraft_io::vfs::read(&path.display().to_string(), MAX_MACRO_BYTES).map_err(|e| EngineError::Other(format!("{}: {e}", path.display())))?;
    String::from_utf8(text).map_err(|_| bad(cmd, format!("{} is not text", path.display())))
}

pub(crate) fn parse_rhai_macro(cmd: &str, text: &str) -> Result<Macro> {
    if text.len() > MAX_MACRO_BYTES {
        return Err(bad(cmd, "the macro file is too large"));
    }
    if text.trim().is_empty() {
        return Err(bad(cmd, "the macro holds no source"));
    }
    let mut params = Vec::new();
    let mut name = None;
    let mut description = None;
    for line in text.lines() {
        let t = line.trim();
        let Some(rest) = t.strip_prefix("//") else { continue };
        let rest = rest.trim();
        if let Some(spec) = rest.strip_prefix("@param") {
            let spec = spec.trim();
            let (spec, when) = param_when(cmd, spec)?;
            let (left, default) = match spec.split_once('|') {
                Some((l, r)) => (l.trim(), Some(Value::String(r.trim().to_string()))),
                None => (spec.as_str(), None),
            };
            let (left, choices) = param_choices(cmd, left)?;
            let mut default = default;
            if default.is_none() {
                if let Some(first) = choices.first() {
                    default = Some(Value::String(first.clone()));
                }
            } else if !choices.is_empty() {
                let want = default.as_ref().and_then(Value::as_str).unwrap_or("");
                if !choices.iter().any(|c| c == want) {
                    return Err(bad(cmd, format!("default `{want}` is not one of the choices for `{left}`")));
                }
            }
            let mut bits = left.split_whitespace();
            let n = bits.next().unwrap_or("").trim();
            if !is_param_name(n) {
                return Err(bad(cmd, "every @param needs a name (letters, digits, `_` and `-`)"));
            }
            if params.iter().any(|q: &MacroParam| q.name == n) {
                return Err(bad(cmd, format!("parameter `{n}` is given twice")));
            }
            if let Some(w) = &when
                && w.name == n
            {
                return Err(bad(cmd, format!("parameter `{n}` cannot depend on itself")));
            }
            let label = bits.collect::<Vec<_>>().join(" ");
            params.push(MacroParam { name: n.to_string(), label: if label.is_empty() { None } else { Some(label) }, default, choices, when });
            if params.len() > 100 {
                return Err(bad(cmd, "a macro asks for at most 100 parameters"));
            }
        } else if name.is_none() && !rest.is_empty() && !rest.starts_with('@') {
            name = Some(rest.to_string());
        } else if description.is_none() && !rest.is_empty() && !rest.starts_with('@') {
            description = Some(rest.to_string());
        }
    }
    Ok(Macro { name, description, params, source: text.to_string() })
}

fn open(s: &mut Session, p: &Value) -> Result<Value> {
    let cmd = "macro.open";
    let path = str_(p, "path").ok_or_else(|| bad(cmd, "`path` must be a .rhai file"))?;
    if !path.rsplit('.').next().is_some_and(|e| e.eq_ignore_ascii_case("rhai")) {
        return Err(bad(cmd, "`path` must end in .rhai"));
    }
    let text = read_macro_file(cmd, Path::new(path))?;
    let m = parse_rhai_macro(cmd, &text)?;
    let out = json!({"name": m.name.clone().unwrap_or_default(), "steps": m.source.matches("run(").count(), "params": json!(m.params)});
    s.macros.loaded = Some(m);
    Ok(out)
}

fn play(s: &mut Session, p: &Value) -> Result<Value> {
    let cmd = "macro.play";
    if s.macros.playing {
        return Err(bad(cmd, "a macro is already playing"));
    }
    let loaded = match (str_(p, "source"), str_(p, "name"), str_(p, "path")) {
        (Some(src), None, None) => parse_rhai_macro(cmd, src)?,
        (None, Some(name), None) => parse_rhai_macro(cmd, &read_macro_file(cmd, &named(cmd, name)?)?)?,
        (None, None, Some(path)) => parse_rhai_macro(cmd, &read_macro_file(cmd, Path::new(path))?)?,
        (None, None, None) => s
            .macros
            .loaded
            .clone()
            .filter(|m| run_count(&m.source) > 0)
            .ok_or_else(|| bad(cmd, "nothing has been recorded; give source, a path or a saved macro's name"))?,
        _ => return Err(bad(cmd, "give source, name or path — one of them")),
    };
    let given = match p.get("params") {
        None | Some(Value::Null) => &Value::Null,
        Some(v @ Value::Object(_)) => v,
        Some(_) => return Err(bad(cmd, "`params` is an object of {name: value}")),
    };
    let mut missing: Vec<&str> = Vec::new();
    let mut bound: Vec<(String, Value)> = Vec::new();
    for mp in &loaded.params {
        match given.get(&mp.name).or(mp.default.as_ref()) {
            Some(v) => bound.push((mp.name.clone(), v.clone())),
            None => missing.push(mp.name.as_str()),
        }
    }
    if !missing.is_empty() {
        return Err(bad(cmd, format!("macro `{}` needs values for: {}", loaded.name.as_deref().unwrap_or("macro"), missing.join(", "))));
    }
    let mark = (s.doc.clone(), s.active_sketch, s.pending_moves.clone(), s.redo.clone(), s.undo.len());
    s.macros.playing = true;
    let _top = TopLevel::enter();
    let outcome = eval_rhai(s, &loaded.source, &bound);
    s.macros.playing = false;
    match outcome {
        Ok(()) => {
            s.undo.truncate(mark.4);
            Ok(json!({"ran": true, "steps": loaded.source.matches("run(").count()}))
        }
        Err(e) => {
            s.doc = mark.0;
            s.active_sketch = mark.1;
            s.pending_moves = mark.2;
            s.redo = mark.3;
            s.undo.truncate(mark.4);
            s.refresh();
            Err(e)
        }
    }
}

fn eval_rhai(s: &mut Session, source: &str, bound: &[(String, Value)]) -> Result<()> {
    let shared = Rc::new(RefCell::new(std::mem::take(s)));
    let eval = {
        let mut engine = Engine::new();
        engine.set_max_operations(MAX_OPS);
        engine.set_max_expr_depths(64, 64);
        let sh = Rc::clone(&shared);
        engine.register_fn("run", move |cmd: ImmutableString, params: Dynamic| -> std::result::Result<Dynamic, Box<EvalAltResult>> {
            let json_params: Value = rhai::serde::from_dynamic(&params).unwrap_or(Value::Null);
            let v = sh.borrow_mut().execute(cmd.as_str(), &json_params).map_err(|e| Box::new(EvalAltResult::from(e.to_string())))?;
            rhai::serde::to_dynamic(&v).map_err(|e| Box::new(EvalAltResult::from(e.to_string())))
        });
        let sh = Rc::clone(&shared);
        engine.register_fn("run", move |cmd: ImmutableString| -> std::result::Result<Dynamic, Box<EvalAltResult>> {
            let v = sh.borrow_mut().execute(cmd.as_str(), &Value::Null).map_err(|e| Box::new(EvalAltResult::from(e.to_string())))?;
            rhai::serde::to_dynamic(&v).map_err(|e| Box::new(EvalAltResult::from(e.to_string())))
        });
        engine.register_fn("list_dir", list_dir_host);
        engine.register_fn("write_file", write_file_host);
        engine.register_fn("file_stem", file_stem_host);
        let mut scope = Scope::new();
        for (name, v) in bound {
            match rhai::serde::to_dynamic(v) {
                Ok(d) => {
                    scope.push_dynamic(name.clone(), d);
                }
                Err(e) => return Err(EngineError::Other(e.to_string())),
            }
        }
        engine.eval_with_scope::<Dynamic>(&mut scope, source).map(|_| ()).map_err(|e| EngineError::Other(format!("macro: {e}")))
    };
    match Rc::try_unwrap(shared) {
        Ok(cell) => *s = cell.into_inner(),
        Err(_) => return Err(EngineError::Other("macro still holds the session".into())),
    }
    eval
}

fn list_dir_host(path: ImmutableString) -> std::result::Result<Dynamic, Box<EvalAltResult>> {
    #[cfg(target_arch = "wasm32")]
    {
        let _ = path;
        return Err(Box::new(EvalAltResult::from("list_dir is not supported in the browser")));
    }
    #[cfg(not(target_arch = "wasm32"))]
    {
        if path.is_empty() || path.len() > 4096 || path.contains("..") {
            return Err(Box::new(EvalAltResult::from("list_dir: bad path")));
        }
        let mut names: Vec<Dynamic> = Vec::new();
        let entries = std::fs::read_dir(path.as_str()).map_err(|e| Box::new(EvalAltResult::from(e.to_string())))?;
        for e in entries.flatten().take(1000) {
            let p = e.path();
            if p.is_file() {
                names.push(Dynamic::from(p.to_string_lossy().into_owned()));
            }
        }
        names.sort_by_key(ToString::to_string);
        Ok(Dynamic::from_array(names))
    }
}

fn file_stem_host(path: ImmutableString) -> ImmutableString {
    Path::new(path.as_str()).file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default().into()
}

fn write_file_host(path: ImmutableString, text: ImmutableString) -> std::result::Result<Dynamic, Box<EvalAltResult>> {
    #[cfg(target_arch = "wasm32")]
    {
        let _ = (path, text);
        return Err(Box::new(EvalAltResult::from("write_file is not supported in the browser")));
    }
    #[cfg(not(target_arch = "wasm32"))]
    {
        if path.is_empty() || path.len() > 4096 || path.contains("..") {
            return Err(Box::new(EvalAltResult::from("write_file: bad path")));
        }
        if text.len() > MAX_MACRO_BYTES {
            return Err(Box::new(EvalAltResult::from("write_file: too large")));
        }
        if let Some(parent) = Path::new(path.as_str()).parent().filter(|p| !p.as_os_str().is_empty())
            && let Err(e) = solidcraft_io::vfs::create_dir_all(&parent.display().to_string())
        {
            return Err(Box::new(EvalAltResult::from(e.to_string())));
        }
        solidcraft_io::vfs::write(path.as_str(), text.as_bytes()).map_err(|e| Box::new(EvalAltResult::from(e.to_string())))?;
        Ok(Dynamic::UNIT)
    }
}

fn save(s: &mut Session, p: &Value) -> Result<Value> {
    let cmd = "macro.save";
    let name = p.get("name").filter(|v| !v.is_null());
    let path = p.get("path").filter(|v| !v.is_null());
    if name.is_some() && path.is_some() {
        return Err(bad(cmd, "give a name or a path, not both"));
    }
    let m = if run_count(&s.macros.captured.source) > 0 {
        s.macros.captured.clone()
    } else if let Some(m) = s.macros.loaded.clone().filter(|m| run_count(&m.source) > 0) {
        m
    } else {
        return Err(bad(cmd, "nothing has been recorded yet"));
    };
    let path = match (name, path) {
        (Some(name), None) => {
            let name = name.as_str().ok_or_else(|| bad(cmd, "`name` must be text"))?;
            named(cmd, name)?.display().to_string()
        }
        (None, Some(path)) => path.as_str().ok_or_else(|| bad(cmd, "`path` must be a file path"))?.to_string(),
        _ => {
            let stem = m.name.as_deref().filter(|n| !n.is_empty()).unwrap_or("macro");
            named(cmd, &format!("{stem}.rhai"))?.display().to_string()
        }
    };
    if !path.rsplit('.').next().is_some_and(|e| e.eq_ignore_ascii_case("rhai")) {
        return Err(bad(cmd, "`path` must end in .rhai"));
    }
    if let Some(parent) = Path::new(&path).parent().filter(|p| !p.as_os_str().is_empty())
        && let Err(e) = solidcraft_io::vfs::create_dir_all(&parent.display().to_string())
    {
        return Err(EngineError::Other(format!("{}: {e}", parent.display())));
    }
    solidcraft_io::vfs::write(&path, m.source.as_bytes()).map_err(|e| EngineError::Other(format!("{path}: {e}")))?;
    Ok(json!({"path": path, "steps": m.source.matches("run(").count()}))
}

fn saved_macros() -> Vec<(String, u64)> {
    let mut out = Vec::new();
    let Some(dir) = macros_dir() else { return out };
    let Ok(entries) = std::fs::read_dir(&dir) else { return out };
    for entry in entries.flatten() {
        let name = entry.file_name().to_string_lossy().to_string();
        if name.ends_with(".rhai")
            && entry.path().is_file()
            && let Ok(len) = entry.path().metadata().map(|m| m.len())
        {
            out.push((name, len));
        }
    }
    out.sort();
    out.truncate(MAX_LISTED);
    out
}

fn list(s: &mut Session, _p: &Value) -> Result<Value> {
    let dir = macros_dir().map(|d| d.display().to_string());
    let macros: Vec<Value> = saved_macros().into_iter().map(|(name, bytes)| json!({"name": name, "bytes": bytes})).collect();
    Ok(json!({"recording": s.macros.recording, "paused": s.macros.paused, "steps": s.macros.steps(), "dir": dir, "macros": macros}))
}

fn delete(_: &mut Session, p: &Value) -> Result<Value> {
    let cmd = "macro.delete";
    let name = str_(p, "name").ok_or_else(|| bad(cmd, "`name` is required (see macro.list)"))?;
    let path = named(cmd, name)?;
    solidcraft_io::vfs::remove(&path.display().to_string()).map_err(|e| EngineError::Other(format!("{}: {e}", path.display())))?;
    Ok(json!({"deleted": name}))
}

fn run_count(src: &str) -> usize {
    src.matches("run(").count()
}

fn append_run(source: &mut String, id: &str, params: &Value) {
    source.push_str("run(\"");
    source.push_str(id);
    if params.is_null() || params.as_object().is_some_and(|o| o.is_empty()) {
        source.push_str("\");\n");
    } else {
        source.push_str("\", ");
        source.push_str(&json_to_rhai(params));
        source.push_str(");\n");
    }
}

fn json_to_rhai(v: &Value) -> String {
    match v {
        Value::Null => "#{ }".into(),
        Value::Bool(b) => b.to_string(),
        Value::Number(n) => n.to_string(),
        Value::String(s) => format!("\"{}\"", s.replace('\\', "\\\\").replace('"', "\\\"")),
        Value::Array(a) => {
            let parts: Vec<String> = a.iter().map(json_to_rhai).collect();
            format!("[{}]", parts.join(", "))
        }
        Value::Object(o) => {
            let parts: Vec<String> = o
                .iter()
                .map(|(k, val)| {
                    let key = if k.chars().all(|c| c.is_ascii_alphanumeric() || c == '_') { k.clone() } else { format!("\"{k}\"") };
                    format!("{key}: {}", json_to_rhai(val))
                })
                .collect();
            format!("#{{{}}}", parts.join(", "))
        }
    }
}

impl Session {
    pub fn macro_state(&self) -> Value {
        json!({"recording": self.macros.recording, "paused": self.macros.paused, "steps": self.macros.steps()})
    }
}
