//! Linked parameters: a design uses the user parameters of another design file by name (Link
//! Parameters). The other design is read and its parameters are evaluated on their own; their
//! values come in read-only and are kept in this design, so it opens and evaluates without the
//! file. The link remembers the file, relative to this design's folder once it is saved; Update
//! Parameter Links reads the files again, and Break Parameter Link keeps the values as ordinary
//! user parameters and forgets the file.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

use serde_json::{Value, json};
use solidcraft_doc::{Document, LinkedParam, Param, ParamScope, ParamSource, expr};

use super::CommandSpec;
use super::component_link::{absolute, resolve, stored};
use super::parameters::failing;
use crate::params::{bad, str_};
use crate::{EngineError, Result, Session};

#[cfg(test)]
#[path = "parameters_link_tests.rs"]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests;

pub static COMMANDS: &[CommandSpec] = &[
    CommandSpec::new("parameters.link_design", "Link Parameters", link_design).icon("chain").params(
        "path: a .solidcraft design → its user parameters come in read-only and can be used by name in this design; \
         scope?: favorites|all (default favorites: its favourite user parameters) → source, file, scope, parameters, recomputed, errors",
    ),
    CommandSpec::new("parameters.update_links", "Update Parameter Links", update_links).icon("update").params(
        "source?: a linked design's path or file name (default: every linked design) — reads the linked designs again → \
         updated [{source, changed [{name, from, to}], added, removed}], unchanged, recomputed, errors",
    ),
    CommandSpec::new("parameters.break_link", "Break Parameter Link", break_link).icon("break").params(
        "source?: a linked design's path or file name (needed when there are several) — its parameters become ordinary user \
         parameters with the values last read, and the file is forgotten",
    ),
    CommandSpec::new("parameters.links", "List Parameter Links", list_links).icon("chain").noundo().params(
        "→ sources: path, file (resolved), scope, parameters, found, status: up_to_date|out_of_date|missing|unreadable, \
         changed (names whose value differs in the file), error?",
    ),
];

/// Most designs one design links parameters from.
const MAX_SOURCES: usize = 64;
/// How far, and through how many files, a loop of parameter links is looked for.
const MAX_DEPTH: usize = 16;
const MAX_VISITS: usize = 256;

/// The design at `file`, read as it is saved.
fn read_design(file: &Path) -> Result<Document> {
    let shown = file.display().to_string();
    let bytes = solidcraft_io::vfs::read(&shown, solidcraft_io::MAX_DESIGN_BYTES).map_err(|e| EngineError::Other(format!("{shown}: {e}")))?;
    solidcraft_io::read_design(&bytes).map_err(|e| EngineError::Other(format!("{shown}: {e}")))
}

/// Whether two paths name one file: the same path, or (on native hosts) the same file through
/// a symlink or a path with different case.
fn same_file(a: &Path, b: &Path) -> bool {
    absolute(a) == absolute(b) || matches!((a.canonicalize(), b.canonicalize()), (Ok(x), Ok(y)) if x == y)
}

/// A value as a number in `unit` with the unit written in (`8.2 mm`), rounded to 10 decimals so
/// a round trip through another unit doesn't show as a change.
fn literal(v: expr::Value, unit: &str) -> Option<String> {
    let x = expr::value_in_unit(v, unit);
    if !x.is_finite() {
        return None;
    }
    let mut n = format!("{x:.10}");
    if n.contains('.') {
        n = n.trim_end_matches('0').trim_end_matches('.').to_string();
    }
    if n == "-0" {
        n = "0".into();
    }
    let unit = unit.trim();
    Some(if unit.is_empty() { n } else { format!("{n} {unit}") })
}

/// The user parameters `doc` shares under `scope`, as values in their own units.
fn shared(doc: &Document, scope: ParamScope, shown: &str) -> Result<Vec<LinkedParam>> {
    let (vals, errs) = doc.param_values();
    let mut out: Vec<LinkedParam> = Vec::new();
    let mut names = BTreeSet::new();
    for p in doc.params.iter().filter(|p| !p.model) {
        if scope == ParamScope::Favorites && !doc.favorites.contains(&p.name) {
            continue;
        }
        if let Some(e) = errs.get(&p.name) {
            return Err(EngineError::Other(format!("{shown}: parameter `{}` has an error ({e}); fix it there first", p.name)));
        }
        let Some(e) = vals.get(&p.name).and_then(|v| literal(*v, &p.unit)) else {
            return Err(EngineError::Other(format!("{shown}: parameter `{}` has no value", p.name)));
        };
        if !names.insert(p.name.clone()) {
            return Err(EngineError::Other(format!("{shown}: the design has two parameters named `{}`", p.name)));
        }
        out.push(LinkedParam { name: p.name.clone(), expr: e, unit: p.unit.trim().to_string(), comment: p.comment.clone() });
    }
    if out.is_empty() {
        return Err(EngineError::Other(match scope {
            ParamScope::Favorites => {
                format!("{shown}: the design has no favourite user parameters (mark the ones to share as favourites there, or link with scope `all`)")
            }
            ParamScope::All => format!("{shown}: the design has no user parameters"),
        }));
    }
    Ok(out)
}

/// Whether `doc` (saved at `file`) uses the parameters of the design at `own` through its own
/// parameter links, directly or further along.
fn links_back(doc: &Document, file: &Path, own: &Path) -> bool {
    let at = file.to_string_lossy().into_owned();
    let mut stack: Vec<(PathBuf, usize)> = doc.param_sources.iter().map(|x| (resolve(Some(&at), &x.path), 1)).collect();
    let mut seen = BTreeSet::new();
    while let Some((f, depth)) = stack.pop() {
        if same_file(&f, own) {
            return true;
        }
        if depth >= MAX_DEPTH || seen.len() >= MAX_VISITS || !seen.insert(absolute(&f)) {
            continue;
        }
        let Ok(next) = read_design(&f) else { continue };
        let at = f.to_string_lossy().into_owned();
        stack.extend(next.param_sources.iter().map(|x| (resolve(Some(&at), &x.path), depth + 1)));
    }
    false
}

/// Names of `params` this design already uses for something else: its own parameters, feature
/// inputs, or parameters linked from another design than source `skip`.
fn clashes(doc: &Document, sources: &[ParamSource], params: &[LinkedParam], skip: Option<usize>) -> Vec<String> {
    params
        .iter()
        .filter(|p| {
            doc.param(&p.name).is_some()
                || doc.features.iter().any(|f| f.param_names.contains(&p.name))
                || sources.iter().enumerate().any(|(i, x)| Some(i) != skip && x.params.iter().any(|q| q.name == p.name))
        })
        .map(|p| p.name.clone())
        .collect()
}

fn scope_arg(p: &Value, cmd: &str) -> Result<ParamScope> {
    match str_(p, "scope").map(str::to_ascii_lowercase).as_deref() {
        None | Some("favorites" | "favourites" | "favorite" | "favourite") => Ok(ParamScope::Favorites),
        Some("all") => Ok(ParamScope::All),
        Some(other) => Err(bad(cmd, format!("unknown scope `{other}` (favorites or all)"))),
    }
}

/// The linked design `source` names: its path as stored, the file, or the file's name.
fn source_index(s: &Session, p: &Value, cmd: &str) -> Result<usize> {
    let sources = &s.doc.param_sources;
    let Some(key) = str_(p, "source").map(str::trim).filter(|k| !k.is_empty()) else {
        return match sources.len() {
            0 => Err(bad(cmd, "this design links no parameters")),
            1 => Ok(0),
            _ => Err(bad(cmd, "`source` is required: this design links parameters from several designs")),
        };
    };
    if let Some(i) = sources.iter().position(|x| x.path == key) {
        return Ok(i);
    }
    let file = absolute(Path::new(key));
    let hits: Vec<usize> = sources
        .iter()
        .enumerate()
        .filter(|(_, x)| {
            let f = resolve(s.path.as_deref(), &x.path);
            absolute(&f) == file || [f.file_name(), f.file_stem()].iter().flatten().any(|n| n.to_string_lossy() == key)
        })
        .map(|(i, _)| i)
        .collect();
    match hits.as_slice() {
        [i] => Ok(*i),
        [] => Err(bad(cmd, format!("no linked design `{key}`"))),
        _ => Err(bad(cmd, format!("several linked designs match `{key}`; give its path"))),
    }
}

fn link_design(s: &mut Session, p: &Value) -> Result<Value> {
    let cmd = "parameters.link_design";
    let given = str_(p, "path").filter(|x| !x.trim().is_empty() && x.len() < 4096).ok_or_else(|| bad(cmd, "`path` is required"))?;
    let scope = scope_arg(p, cmd)?;
    let file = absolute(Path::new(given));
    let own = s.path.clone();
    if own.as_deref().is_some_and(|o| same_file(Path::new(o), &file)) {
        return Err(bad(cmd, "a design can't link its own parameters"));
    }
    if s.doc.param_sources.iter().any(|x| same_file(&resolve(own.as_deref(), &x.path), &file)) {
        return Err(bad(cmd, "this design already links that design's parameters; update the parameter links instead"));
    }
    if s.doc.param_sources.len() >= MAX_SOURCES {
        return Err(bad(cmd, format!("a design can link parameters from at most {MAX_SOURCES} designs")));
    }
    let shown = file.display().to_string();
    let doc = read_design(&file).map_err(|e| bad(cmd, e.to_string()))?;
    if own.as_deref().is_some_and(|o| links_back(&doc, &file, Path::new(o))) {
        return Err(bad(cmd, format!("{shown} uses this design's parameters through its own parameter links, so linking it would make a loop")));
    }
    let params = shared(&doc, scope, &shown).map_err(|e| bad(cmd, e.to_string()))?;
    let clash = clashes(&s.doc, &s.doc.param_sources, &params, None);
    if !clash.is_empty() {
        return Err(bad(cmd, format!("this design already has parameters named {}; rename them here or in {shown} first", clash.join(", "))));
    }
    let linked = s.doc.linked_params().count();
    if linked.saturating_add(params.len()) > solidcraft_doc::MAX_PARAMS {
        return Err(bad(cmd, "too many linked parameters"));
    }
    let names: Vec<String> = params.iter().map(|x| x.name.clone()).collect();
    let path = stored(own.as_deref(), &file);
    let solved = s.solved_sketches();
    s.doc_mut().param_sources.push(ParamSource { path: path.clone(), scope, params });
    s.settle_value_edit(&solved)?;
    Ok(json!({"source": path, "file": shown, "scope": scope, "parameters": names, "recomputed": s.model.last_recomputed, "errors": failing(s)}))
}

fn update_links(s: &mut Session, p: &Value) -> Result<Value> {
    let cmd = "parameters.update_links";
    let which: Vec<usize> = if p.get("source").is_some() {
        vec![source_index(s, p, cmd)?]
    } else if s.doc.param_sources.is_empty() {
        return Err(bad(cmd, "this design links no parameters"));
    } else {
        (0..s.doc.param_sources.len()).collect()
    };
    let own = s.path.clone();
    let mut next = s.doc.param_sources.clone();
    let (mut updated, mut unchanged) = (Vec::new(), Vec::new());
    for i in which {
        let Some(src) = next.get(i).cloned() else { continue };
        let file = resolve(own.as_deref(), &src.path);
        let shown = file.display().to_string();
        let doc = read_design(&file).map_err(|e| bad(cmd, e.to_string()))?;
        if own.as_deref().is_some_and(|o| links_back(&doc, &file, Path::new(o))) {
            return Err(bad(cmd, format!("{shown} now uses this design's parameters through its own parameter links, which makes a loop")));
        }
        let params = shared(&doc, src.scope, &shown).map_err(|e| bad(cmd, e.to_string()))?;
        if params == src.params {
            unchanged.push(src.path.clone());
            continue;
        }
        let old = |n: &str| src.params.iter().find(|x| x.name == n);
        let changed: Vec<Value> = params
            .iter()
            .filter_map(|x| old(&x.name).filter(|o| o.expr != x.expr).map(|o| json!({"name": x.name, "from": o.expr, "to": x.expr})))
            .collect();
        let added: Vec<LinkedParam> = params.iter().filter(|x| old(&x.name).is_none()).cloned().collect();
        let removed: Vec<String> = src.params.iter().filter(|o| !params.iter().any(|x| x.name == o.name)).map(|o| o.name.clone()).collect();
        let clash = clashes(&s.doc, &next, &added, Some(i));
        if !clash.is_empty() {
            return Err(bad(
                cmd,
                format!("{shown} now has parameters named {}, which this design already uses; rename them first", clash.join(", ")),
            ));
        }
        for name in &removed {
            let users = s.doc.param_users(name);
            if !users.is_empty() {
                let list: Vec<String> = users.iter().map(solidcraft_doc::ParamUser::describe).collect();
                return Err(bad(cmd, format!("`{name}` is no longer shared by {shown}, but {} use it", list.join(", "))));
            }
        }
        let added: Vec<String> = added.into_iter().map(|x| x.name).collect();
        updated.push(json!({"source": src.path, "changed": changed, "added": added, "removed": removed}));
        if let Some(slot) = next.get_mut(i) {
            slot.params = params;
        }
    }
    if !updated.is_empty() {
        if next.iter().fold(0usize, |n, x| n.saturating_add(x.params.len())) > solidcraft_doc::MAX_PARAMS {
            return Err(bad(cmd, "too many linked parameters"));
        }
        let solved = s.solved_sketches();
        s.doc_mut().param_sources = next;
        s.settle_value_edit(&solved)?;
    }
    Ok(json!({"updated": updated, "unchanged": unchanged, "recomputed": s.model.last_recomputed, "errors": failing(s)}))
}

fn break_link(s: &mut Session, p: &Value) -> Result<Value> {
    let cmd = "parameters.break_link";
    let i = source_index(s, p, cmd)?;
    let src = s.doc.param_sources.get(i).cloned().ok_or_else(|| bad(cmd, "no such linked design"))?;
    if s.doc.params.len().saturating_add(src.params.len()) > solidcraft_doc::MAX_PARAMS {
        return Err(bad(cmd, "too many parameters"));
    }
    let names: Vec<String> = src.params.iter().map(|x| x.name.clone()).collect();
    let solved = s.solved_sketches();
    let d = s.doc_mut();
    if i < d.param_sources.len() {
        d.param_sources.remove(i);
    }
    d.params.extend(src.params.into_iter().map(|x| Param { name: x.name, expr: x.expr, unit: x.unit, comment: x.comment, model: false }));
    s.settle_value_edit(&solved)?;
    Ok(json!({"source": src.path, "parameters": names}))
}

/// Where a linked design's parameters stand against its file.
fn status(own: Option<&str>, src: &ParamSource) -> Value {
    let file = resolve(own, &src.path);
    let shown = file.display().to_string();
    let mut out =
        json!({"path": src.path, "file": shown, "scope": src.scope, "parameters": src.params.len(), "found": file.is_file(), "changed": []});
    if !file.is_file() {
        out["status"] = json!("missing");
        return out;
    }
    match read_design(&file).and_then(|d| shared(&d, src.scope, &shown)) {
        Ok(now) => {
            let changed: Vec<&str> = src
                .params
                .iter()
                .filter(|o| now.iter().find(|x| x.name == o.name).is_none_or(|x| x != *o))
                .map(|o| o.name.as_str())
                .chain(now.iter().filter(|x| !src.params.iter().any(|o| o.name == x.name)).map(|x| x.name.as_str()))
                .collect();
            out["status"] = json!(if changed.is_empty() && now.len() == src.params.len() { "up_to_date" } else { "out_of_date" });
            out["changed"] = json!(changed);
        }
        Err(e) => {
            out["status"] = json!("unreadable");
            out["error"] = json!(e.to_string());
        }
    }
    out
}

fn list_links(s: &mut Session, _p: &Value) -> Result<Value> {
    let sources: Vec<Value> = s.doc.param_sources.iter().map(|x| status(s.path.as_deref(), x)).collect();
    Ok(json!({"sources": sources}))
}

/// What to tell someone opening the design about its linked parameters: designs that are
/// missing, can't be read or have changed since they were last read.
pub(super) fn open_warnings(s: &Session) -> Vec<String> {
    let mut out = Vec::new();
    for x in &s.doc.param_sources {
        let st = status(s.path.as_deref(), x);
        match st["status"].as_str() {
            Some("missing") => out.push(format!("linked parameters: {} is missing; the values last read are used", x.path)),
            Some("unreadable") => out.push(format!("linked parameters: {} can't be read ({}); the values last read are used", x.path, st["error"])),
            Some("out_of_date") => out.push(format!("linked parameters: {} has changed; update the parameter links to use its new values", x.path)),
            _ => {}
        }
    }
    out
}

/// The parameter links of `doc` stored for a design saved at `to` instead of at `from`.
pub(super) fn rebased(doc: &Document, from: Option<&str>, to: &str) -> Vec<ParamSource> {
    doc.param_sources.iter().map(|x| ParamSource { path: stored(Some(to), &resolve(from, &x.path)), ..x.clone() }).collect()
}
