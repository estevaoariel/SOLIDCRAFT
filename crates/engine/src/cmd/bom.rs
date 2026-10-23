//! Part properties and the bill of materials. `component.metadata` and
//! `component.metadata.set` read and change a component definition's part number, description,
//! revision and free properties (shared by every occurrence, saved with the component);
//! `assembly.bom` counts the placed components of the design, or of one component, and lists
//! them as rows, optionally written to a CSV or JSON file. With `group_children`, a part with a
//! part number keeps the parts placed inside it (a bolt's washers and nut) in its own row.
//!
//! A component's bill of materials type (`bom_type`, a part property) decides how it counts: a
//! phantom has no row and what it holds counts in its parent; a reference counts nowhere, and
//! nothing inside it does; a purchased or inseparable component is one row, and of what an
//! inseparable one holds only its purchased parts reach the parts-only list. One occurrence can
//! count as another type (`occurrence.bom_type`), in its own rows.
//!
//! A row names the physical materials of its parts' bodies, and with `mass` what one of its parts
//! weighs as the row stands for it: with what is placed inside it, except what the list shows as
//! rows of their own beside it, so every list adds up to the assembly once (measured once per
//! definition; a reference placement adds nothing).

use std::collections::{BTreeMap, HashSet};
use std::sync::Arc;

use serde_json::{Map, Value, json};
use solidcraft_doc::BomType;
use solidcraft_doc::metadata::{BOM_TYPE, DESCRIPTION, MAX_KEYS, MAX_TEXT_BYTES, PART_NUMBER, REVISION, WELL_KNOWN};

use super::CommandSpec;
use crate::params::{bad, str_};
use crate::{EngineError, Result, Session};

pub static COMMANDS: &[CommandSpec] = &[
    CommandSpec::new("component.metadata", "Part Properties", read).icon("settings").noundo().doc_only().params(
        "component: id or name (0 or \"root\": the design's own properties) → component, name, part_number, description, revision, \
         bom_type (components only), metadata (as saved, other tools' keys too), properties: {name: text}",
    ),
    CommandSpec::new("component.metadata.set", "Change Part Properties", set).icon("settings").doc_only().params(
        "component: id or name (not the root: use document.metadata.set); part_number?, description?, revision?: text (empty or null \
         removes); bom_type?: normal|phantom|reference|purchased|inseparable (empty or null: normal); values?: {key: value} \
         (SolidCraft's keys project, part_number, revision, author, organization, description, license, bom_type, or a reverse-DNS \
         key holding an object); properties?: {name: text, …} (free properties; empty or null removes). All or nothing, one undo \
         step → component, metadata, properties, changed: [keys]",
    ),
    CommandSpec::new("occurrence.bom_type", "Bill of Materials Type", occurrence_type).icon("bom").doc_only().params(
        "occurrence: id or name; path?: [occurrence ids] (checked when given); bom_type: normal|phantom|reference|purchased|inseparable \
         (this occurrence counts as that type in every bill of materials, in place of its component's; an occurrence inside a \
         subassembly belongs to its definition, so the type holds wherever the subassembly is placed) | same_as_component or null \
         (as its component again). One undo step → occurrence, bom_type (its own, or same_as_component), effective (the type it \
         counts as), changed",
    ),
    CommandSpec::new("assembly.bom", "Bill of Materials", bom).at("SOLID", "INSPECT").icon("bom").noundo().params(
        "component?: id or name (default the design); levels?: flat (default: every part at any depth, counted through its \
         subassemblies) | top (the component's direct children, subassemblies as one row each); group_by?: [\"part_number\"] \
         (default; components without a part number get a row per definition) | [\"definition\"]; group_children?: true (a component \
         with a part number is one row, the parts placed inside it listed under it as children: [{item, part_number, description, \
         revision, each, count, definition, material, mass_g?}], not as rows of their own); mass?: true (measures each part: \
         mass_g, what one of the row's parts weighs in grams with what is placed inside it, except what the list shows as rows of \
         their own beside it (in parts only a part's contents are rows of their own, but not a folded part's or a purchased one's, \
         and of an inseparable one's only its purchased parts are), so the rows add up to the assembly; a reference placement \
         adds nothing; null when nothing in it has a body, a body can't be measured or the row's definitions differ); path?: write \
         a .csv or .json file (children as items 1.1, 1.2…); format?: csv|json (default from the path). Each placement counts as \
         its occurrence's bom_type, else its component's: phantom has no row (what it holds counts here), reference counts \
         nowhere, purchased and inseparable are one row (in flat, an inseparable one's purchased parts are listed too); placements \
         of one component that count as different types are different rows → rows: [{item, part_number, description, revision, \
         bom_type, count, definition, material (the physical materials of the definitions' own bodies, \"; \" between them), \
         mass_g?, components, properties, children?}] sorted by part number, total_occurrences (children included)",
    ),
];

/// Longest free property name, in bytes.
const MAX_PROPERTY_NAME_BYTES: usize = 128;
/// How deep, and over how many placements, the bill of materials walks.
const MAX_DEPTH: usize = 64;
const MAX_PLACEMENTS: usize = 100_000;

fn key(v: Option<&Value>) -> Option<String> {
    match v {
        Some(Value::Number(n)) => Some(n.to_string()),
        Some(Value::String(s)) => Some(s.clone()),
        _ => None,
    }
}

fn component_id(s: &Session, p: &Value, cmd: &str) -> Result<u64> {
    let k = key(p.get("component")).ok_or_else(|| bad(cmd, "`component` must be a component id or name"))?;
    s.doc.find_component(&k).ok_or_else(|| bad(cmd, format!("no component `{k}`")))
}

fn read(s: &mut Session, p: &Value) -> Result<Value> {
    let cmd = "component.metadata";
    let id = component_id(s, p, cmd)?;
    if id == 0 {
        let m = &s.doc.metadata;
        return Ok(json!({"component": 0, "name": s.doc.name, "part_number": m.part_number(), "description": m.description(),
            "revision": m.revision(), "metadata": m, "properties": {}}));
    }
    let c = s.doc.components.iter().find(|c| c.id == id).ok_or_else(|| bad(cmd, format!("no component `{id}`")))?;
    let m = &c.metadata;
    Ok(json!({"component": id, "name": c.name, "part_number": m.part_number(), "description": m.description(),
        "revision": m.revision(), "bom_type": m.bom_type().as_str(), "metadata": m, "properties": c.properties}))
}

/// A free property as it may be saved (`None`: remove it), or why not.
fn property(name: &str, value: &Value, cmd: &str) -> Result<Option<String>> {
    let shown: String = name.chars().take(64).collect();
    if name.trim().is_empty() || name.len() > MAX_PROPERTY_NAME_BYTES || name.chars().any(char::is_control) {
        return Err(bad(cmd, format!("property name `{shown}` must be 1…{MAX_PROPERTY_NAME_BYTES} bytes of text without control characters")));
    }
    let text = match value {
        Value::Null => return Ok(None),
        Value::String(t) if t.is_empty() => return Ok(None),
        Value::String(t) => t.clone(),
        Value::Number(n) => n.to_string(),
        Value::Bool(b) => b.to_string(),
        _ => return Err(bad(cmd, format!("property `{shown}` takes text"))),
    };
    if text.len() > MAX_TEXT_BYTES {
        return Err(bad(cmd, format!("property `{shown}` is longer than {MAX_TEXT_BYTES} bytes")));
    }
    if text.chars().any(|c| c.is_control() && !matches!(c, '\n' | '\t' | '\r')) {
        return Err(bad(cmd, format!("property `{shown}` has a control character")));
    }
    Ok(Some(text))
}

fn set(s: &mut Session, p: &Value) -> Result<Value> {
    let cmd = "component.metadata.set";
    let id = component_id(s, p, cmd)?;
    if id == 0 {
        return Err(bad(cmd, "the root is the design: its properties are document.metadata.set's"));
    }
    let c = s.doc.components.iter().find(|c| c.id == id).ok_or_else(|| bad(cmd, format!("no component `{id}`")))?;
    let (mut metadata, mut properties) = (c.metadata.clone(), c.properties.clone());
    let mut entries: Vec<(String, Value)> = Vec::new();
    match p.get("values") {
        None => {}
        Some(Value::Object(v)) if v.len() <= MAX_KEYS => entries.extend(v.iter().map(|(k, v)| (k.clone(), v.clone()))),
        Some(Value::Object(_)) => return Err(bad(cmd, format!("`values` can hold at most {MAX_KEYS} keys"))),
        Some(_) => return Err(bad(cmd, "`values` must be an object: {key: value, …}")),
    }
    for k in [PART_NUMBER, DESCRIPTION, REVISION, BOM_TYPE] {
        if let Some(v) = p.get(k) {
            entries.push((k.to_string(), v.clone()));
        }
    }
    let props: Vec<(&String, &Value)> = match p.get("properties") {
        None => Vec::new(),
        Some(Value::Object(v)) if v.len() <= MAX_KEYS => v.iter().collect(),
        Some(Value::Object(_)) => return Err(bad(cmd, format!("`properties` can hold at most {MAX_KEYS} names"))),
        Some(_) => return Err(bad(cmd, "`properties` must be an object: {name: text, …}")),
    };
    if entries.is_empty() && props.is_empty() {
        return Err(bad(
            cmd,
            format!("give part_number, description, revision, bom_type, values or properties ({}, {BOM_TYPE})", WELL_KNOWN.join(", ")),
        ));
    }
    let mut changed: Vec<String> = Vec::new();
    for (k, v) in entries {
        if metadata.set(&k, v).map_err(|e| bad(cmd, e.to_string()))? && !changed.contains(&k) {
            changed.push(k);
        }
    }
    for (name, value) in props {
        let was = properties.get(name).cloned();
        match property(name, value, cmd)? {
            Some(text) => {
                properties.insert(name.clone(), text);
            }
            None => {
                properties.remove(name);
            }
        }
        if properties.get(name) != was.as_ref() {
            changed.push(format!("properties.{name}"));
        }
    }
    if properties.len() > MAX_KEYS {
        return Err(bad(cmd, format!("a component holds at most {MAX_KEYS} properties")));
    }
    // Only a real change touches the design (an undo step, unsaved changes).
    if !changed.is_empty()
        && let Some(c) = s.doc_mut().components.iter_mut().find(|c| c.id == id)
    {
        c.metadata = metadata;
        c.properties = properties;
    }
    let c = s.doc.components.iter().find(|c| c.id == id);
    Ok(json!({"component": id, "metadata": c.map(|c| &c.metadata), "properties": c.map(|c| &c.properties), "changed": changed}))
}

/// The word that clears an occurrence's own bill of materials type.
const SAME_AS_COMPONENT: &str = "same_as_component";

fn occurrence_type(s: &mut Session, p: &Value) -> Result<Value> {
    let cmd = "occurrence.bom_type";
    let id = super::component::occurrence(s, p, cmd)?;
    if p.get("path").is_some() {
        super::component::occurrence_path(s, p, id, cmd)?;
    }
    let words = || format!("{}, or {SAME_AS_COMPONENT}", BomType::ALL.map(BomType::as_str).join(", "));
    let want = match p.get("bom_type") {
        Some(Value::Null) => None,
        Some(Value::String(w)) if w == SAME_AS_COMPONENT => None,
        Some(Value::String(w)) => Some(BomType::parse(w).ok_or_else(|| bad(cmd, format!("`bom_type` is one of {}", words())))?),
        _ => return Err(bad(cmd, format!("give `bom_type`: {}", words()))),
    };
    let changed = s.doc.occurrences.iter().find(|o| o.id == id).ok_or_else(|| bad(cmd, "no occurrence"))?.bom_type != want;
    // Only a real change touches the design (an undo step, unsaved changes).
    if changed && let Some(o) = s.doc_mut().occurrences.iter_mut().find(|o| o.id == id) {
        o.bom_type = want;
    }
    let effective = s.doc.occurrences.iter().zip(s.doc.occurrence_bom_types()).find(|(o, _)| o.id == id).map(|(_, t)| t).unwrap_or_default();
    Ok(json!({"occurrence": id, "bom_type": want.map_or(SAME_AS_COMPONENT, BomType::as_str), "effective": effective.as_str(),
        "changed": changed}))
}

#[derive(Clone, Copy, PartialEq)]
enum Levels {
    Flat,
    Top,
}

#[derive(Clone, Copy, PartialEq)]
enum Group {
    PartNumber,
    Definition,
}

/// One row being gathered.
struct Row {
    part_number: String,
    description: String,
    revision: String,
    bom_type: BomType,
    count: u64,
    components: Vec<u64>,
    names: Vec<String>,
    properties: BTreeMap<String, String>,
    /// With `group_children`: what is placed inside one of this row's parts, and how often.
    kit: BTreeMap<Placed, u64>,
}

/// What the bill of materials lists and how.
#[derive(Clone, Copy)]
struct Options {
    levels: Levels,
    group: Group,
    /// A part with a part number keeps its sub-parts in its row.
    fold: bool,
    /// Each row's mass (measuring every part listed).
    mass: bool,
}

fn options(p: &Value, cmd: &str) -> Result<Options> {
    let levels = match p.get("levels") {
        None | Some(Value::Null) => Levels::Flat,
        Some(Value::String(l)) if l == "flat" => Levels::Flat,
        Some(Value::String(l)) if l == "top" => Levels::Top,
        Some(_) => return Err(bad(cmd, "`levels` is flat or top")),
    };
    let group = match p.get("group_by") {
        None | Some(Value::Null) => Group::PartNumber,
        Some(Value::String(g)) => group_key(g, cmd)?,
        Some(Value::Array(a)) => match a.as_slice() {
            [Value::String(g)] => group_key(g, cmd)?,
            _ => return Err(bad(cmd, "`group_by` is [\"part_number\"] or [\"definition\"]")),
        },
        Some(_) => return Err(bad(cmd, "`group_by` is [\"part_number\"] or [\"definition\"]")),
    };
    let fold = match p.get("group_children") {
        None | Some(Value::Null) => false,
        Some(Value::Bool(b)) => *b,
        Some(_) => return Err(bad(cmd, "`group_children` is true or false")),
    };
    let mass = match p.get("mass") {
        None | Some(Value::Null) => false,
        Some(Value::Bool(b)) => *b,
        Some(_) => return Err(bad(cmd, "`mass` is true or false")),
    };
    Ok(Options { levels, group, fold, mass })
}

/// A component as it is placed: its id, and the bill of materials type it counts as there.
type Placed = (u64, BomType);

/// The design's structure, looked up once per bill of materials.
struct Index {
    /// What each component places (by its id; 0 is the root), each with the type it counts as
    /// there: the occurrence's own, else its component's.
    children: BTreeMap<u64, Vec<Placed>>,
    /// The bodies each component owns (positions in `state.bodies`); a component that owns none
    /// isn't here.
    bodies: BTreeMap<u64, Vec<usize>>,
    /// Components with a part number: with `group_children`, one row however much is inside.
    numbered: HashSet<u64>,
    state: Arc<solidcraft_doc::ModelState>,
}

impl Index {
    fn new(s: &Session) -> Index {
        let mut children: BTreeMap<u64, Vec<Placed>> = BTreeMap::new();
        for (o, bom_type) in s.doc.occurrences.iter().zip(s.doc.occurrence_bom_types()) {
            children.entry(o.parent).or_default().push((o.component, bom_type));
        }
        let numbered = s.doc.components.iter().filter(|c| c.metadata.part_number().is_some_and(|n| !n.is_empty())).map(|c| c.id).collect();
        let state = s.model.state();
        let mut bodies: BTreeMap<u64, Vec<usize>> = BTreeMap::new();
        for (i, owner) in s.doc.body_owners(&state).into_iter().enumerate() {
            bodies.entry(owner).or_default().push(i);
        }
        Index { children, bodies, numbered, state }
    }

    /// The bodies a component owns.
    fn own_bodies(&self, id: u64) -> impl Iterator<Item = &solidcraft_doc::ModelBody> {
        self.bodies.get(&id).into_iter().flatten().filter_map(|&i| self.state.bodies.get(i))
    }

    /// The physical materials of the components' own bodies, each once, in body order, joined as
    /// a row's definitions are.
    fn material(&self, s: &Session, components: &[u64]) -> String {
        let mut names: Vec<String> = Vec::new();
        for body in components.iter().flat_map(|&c| self.own_bodies(c)) {
            let name = s.doc.body_material(&body.name);
            if !names.contains(&name) {
                names.push(name);
            }
        }
        names.join("; ")
    }
}

/// What one of a component weighs: grams, and how many bodies that is (none: nothing to weigh).
/// `None` when a body in it can't be measured, or measures inside out.
type Weighed = Option<(f64, usize)>;

/// What a row's part stands for when it is weighed, so that the rows of any list add up to the
/// assembly once.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum Holds {
    /// Everything placed inside it.
    All,
    /// Its own bodies alone: what it holds is listed beside it.
    Nothing,
    /// Everything but its purchased parts, which are listed beside it.
    AllButPurchased,
}

/// What a row of `bom_type` stands for in a list of `levels`. At the top level a row is the
/// whole of what it holds. In parts only, what a part holds is listed beside it, except in a
/// folded part (a kit's row), in a purchased part, and in an inseparable one, of which only the
/// purchased parts are listed.
fn holds(levels: Levels, bom_type: BomType, folded: bool) -> Holds {
    match (levels, bom_type) {
        (Levels::Top, _) | (_, BomType::Purchased) => Holds::All,
        (Levels::Flat, BomType::Inseparable) => Holds::AllButPurchased,
        (Levels::Flat, _) if folded => Holds::All,
        (Levels::Flat, _) => Holds::Nothing,
    }
}

/// One of `id`, as `holds` says: its own bodies, and what is placed inside it of any type but
/// reference, which adds nothing (nor does anything inside it). Placements are rigid, so where a
/// part is placed doesn't change what it weighs: each definition is weighed once for each kind
/// of row (`done`), and each body is measured once.
fn weigh(s: &Session, ix: &Index, id: u64, holds: Holds, depth: usize, done: &mut BTreeMap<(u64, Holds), Weighed>, cmd: &str) -> Result<Weighed> {
    if let Some(&weighed) = done.get(&(id, holds)) {
        return Ok(weighed);
    }
    if depth > MAX_DEPTH {
        return Err(bad(cmd, format!("the assembly is deeper than {MAX_DEPTH} levels")));
    }
    let mut total: Weighed = Some((0.0, 0));
    if holds == Holds::Nothing {
        for body in ix.own_bodies(id) {
            // Only the volume is needed: `measure_mass` skips the topology counts' tessellation.
            let measured = match s.doc.model_body_offset(body) {
                Some(offset) => solidcraft_kernel::transform_matrix(&body.body, *offset).and_then(|b| solidcraft_kernel::measure_mass(&b)),
                None => solidcraft_kernel::measure_mass(&body.body),
            };
            // A negative volume (a body inside out) is unknown, never a negative mass.
            add(&mut total, measured.ok().filter(|m| m.volume >= 0.0).map(|m| (m.volume * s.doc.density(&body.name) / 1000.0, 1)));
        }
    } else {
        add(&mut total, weigh(s, ix, id, Holds::Nothing, depth, done, cmd)?);
        for &(child, bom_type) in ix.children.get(&id).into_iter().flatten() {
            let listed_beside = holds == Holds::AllButPurchased && bom_type == BomType::Purchased;
            if bom_type != BomType::Reference && !listed_beside {
                add(&mut total, weigh(s, ix, child, holds, depth + 1, done, cmd)?);
            }
        }
    }
    let weighed = total.filter(|(grams, _)| grams.is_finite());
    done.insert((id, holds), weighed);
    Ok(weighed)
}

/// `more` added to `total`; unknown when either is.
fn add(total: &mut Weighed, more: Weighed) {
    *total = total.zip(more).map(|((grams, bodies), (g, n))| (grams + g, bodies + n));
}

/// One of a row's parts weighed as the row stands for them: its mass in grams, or null.
fn mass(s: &Session, ix: &Index, components: &[u64], holds: Holds, done: &mut BTreeMap<(u64, Holds), Weighed>, cmd: &str) -> Result<Value> {
    for &c in components {
        weigh(s, ix, c, holds, 0, done, cmd)?;
    }
    Ok(json!(row_mass(components, holds, done)))
}

/// One of a row's parts in grams, or `None` when that isn't known: a definition can't be
/// measured or has nothing to weigh, or the row's definitions (one part number) weigh differently.
fn row_mass(components: &[u64], holds: Holds, weighed: &BTreeMap<(u64, Holds), Weighed>) -> Option<f64> {
    let mut mass = None;
    for c in components {
        let (grams, bodies) = weighed.get(&(*c, holds)).copied().flatten()?;
        if bodies == 0 {
            return None;
        }
        match mass {
            None => mass = Some(grams),
            Some(first) if (grams - first).abs() > 1e-9 * first.abs() => return None,
            Some(_) => {}
        }
    }
    mass
}

/// What a walk below a component lists.
#[derive(Clone, Copy, PartialEq)]
enum Walk {
    /// What the bill of materials lists.
    Listed,
    /// Inside an inseparable component, in parts only: just its purchased parts.
    Purchased,
}

fn group_key(g: &str, cmd: &str) -> Result<Group> {
    match g {
        "part_number" => Ok(Group::PartNumber),
        "definition" => Ok(Group::Definition),
        o => Err(bad(cmd, format!("can't group by `{o}` (part_number or definition)"))),
    }
}

/// How many times each component is listed below `root`, by the bill of materials type it
/// counts as where it is placed: in `Top`, the direct children; in `Flat`, the parts at any depth
/// (a part in a subassembly placed twice counts twice), not below a purchased part, nor below a
/// numbered part when `fold`, and of what an inseparable part holds only its purchased parts. A
/// phantom's contents count where the phantom is placed; a reference counts nowhere, and nothing
/// inside it does.
fn counts(ix: &Index, root: u64, levels: Levels, fold: bool, cmd: &str) -> Result<BTreeMap<Placed, u64>> {
    let mut out: BTreeMap<Placed, u64> = BTreeMap::new();
    let mut placed = 0usize;
    let mut stack: Vec<(u64, usize, Walk)> = vec![(root, 0, Walk::Listed)];
    while let Some((comp, depth, walk)) = stack.pop() {
        for &(child, bom_type) in ix.children.get(&comp).into_iter().flatten() {
            placed += 1;
            if depth >= MAX_DEPTH || placed > MAX_PLACEMENTS {
                return Err(bad(cmd, format!("the assembly is deeper than {MAX_DEPTH} levels or has more than {MAX_PLACEMENTS} placements")));
            }
            let inside = ix.children.contains_key(&child);
            match (bom_type, walk) {
                (BomType::Reference, _) => {}
                (BomType::Phantom, _) => stack.push((child, depth + 1, walk)),
                (BomType::Purchased, _) => *out.entry((child, bom_type)).or_default() += 1,
                (_, Walk::Purchased) => {
                    if inside {
                        stack.push((child, depth + 1, Walk::Purchased));
                    }
                }
                (BomType::Inseparable, Walk::Listed) => {
                    *out.entry((child, bom_type)).or_default() += 1;
                    if levels == Levels::Flat && inside {
                        stack.push((child, depth + 1, Walk::Purchased));
                    }
                }
                (BomType::Normal, Walk::Listed) => {
                    // Parts only: a subassembly is listed through its parts, unless it has bodies
                    // of its own (or, folding, a part number).
                    let folded = fold && ix.numbered.contains(&child);
                    if levels == Levels::Top || !inside || folded || ix.bodies.contains_key(&child) {
                        *out.entry((child, bom_type)).or_default() += 1;
                    }
                    if levels == Levels::Flat && inside && !folded {
                        stack.push((child, depth + 1, Walk::Listed));
                    }
                }
            }
        }
    }
    Ok(out)
}

/// How a kit reads when rows are compared: its parts by part number (else definition), type
/// (when not normal) and count.
fn kit_key(s: &Session, kit: &BTreeMap<Placed, u64>) -> String {
    let mut parts: Vec<String> = kit
        .iter()
        .map(|((id, bom_type), n)| {
            let pn = s.doc.components.iter().find(|c| c.id == *id).and_then(|c| c.metadata.part_number()).unwrap_or_default();
            let part = if pn.is_empty() { format!("#{id}") } else { pn.to_string() };
            if *bom_type == BomType::Normal { format!("{part}×{n}") } else { format!("{part} ({})×{n}", bom_type.as_str()) }
        })
        .collect();
    parts.sort();
    parts.join("\n")
}

fn rows(s: &Session, counts: &BTreeMap<Placed, u64>, group: Group, kits: &BTreeMap<u64, BTreeMap<Placed, u64>>) -> Vec<Row> {
    let mut grouped: BTreeMap<(String, String, u64, BomType), Row> = BTreeMap::new();
    for (&(id, bom_type), n) in counts {
        let Some(c) = s.doc.components.iter().find(|c| c.id == id) else { continue };
        let m = &c.metadata;
        let part_number = m.part_number().unwrap_or_default().to_string();
        // Only a normal part is listed with what it holds.
        let kit = if bom_type == BomType::Normal { kits.get(&id).cloned().unwrap_or_default() } else { BTreeMap::new() };
        // Without a part number a definition is its own row; parts with different contents
        // (a bolt with or without its nut) or different bill of materials types are different rows.
        let k = match group {
            Group::PartNumber if !part_number.is_empty() => (part_number.clone(), kit_key(s, &kit), 0, bom_type),
            _ => (String::new(), String::new(), id, bom_type),
        };
        let row = grouped.entry(k).or_insert_with(|| Row {
            part_number: part_number.clone(),
            description: String::new(),
            revision: String::new(),
            bom_type,
            count: 0,
            components: Vec::new(),
            names: Vec::new(),
            properties: BTreeMap::new(),
            kit,
        });
        row.count += n;
        row.components.push(id);
        if !row.names.contains(&c.name) {
            row.names.push(c.name.clone());
        }
        if row.description.is_empty() {
            row.description = m.description().unwrap_or_default().to_string();
        }
        if row.revision.is_empty() {
            row.revision = m.revision().unwrap_or_default().to_string();
        }
        for (k, v) in &c.properties {
            row.properties.entry(k.clone()).or_insert_with(|| v.clone());
        }
    }
    let mut out: Vec<Row> = grouped.into_values().collect();
    // By part number; rows without one after them, by name; one part's rows by type.
    out.sort_by(|a, b| {
        (a.part_number.is_empty(), &a.part_number, &a.names, &a.components, a.bom_type).cmp(&(
            b.part_number.is_empty(),
            &b.part_number,
            &b.names,
            &b.components,
            b.bom_type,
        ))
    });
    out
}

/// A CSV field, always quoted.
fn quote(s: &str) -> String {
    format!("\"{}\"", s.replace('"', "\"\""))
}

/// The bill of materials as CSV: fixed columns, every field quoted.
fn csv(rows: &[Value]) -> String {
    let mut out = ["item", "part_number", "description", "revision", "count", "definition"].map(quote).join(",");
    out.push('\n');
    let lines = rows.iter().flat_map(|r| std::iter::once(r).chain(r["children"].as_array().into_iter().flatten()));
    for r in lines {
        let text = |k: &str| match &r[k] {
            Value::String(t) => t.clone(),
            Value::Null => String::new(),
            v => v.to_string(),
        };
        let fields = ["item", "part_number", "description", "revision", "count", "definition"].map(|k| quote(&text(k)));
        out.push_str(&fields.join(","));
        out.push('\n');
    }
    out
}

fn bom(s: &mut Session, p: &Value) -> Result<Value> {
    let cmd = "assembly.bom";
    let root = if p.get("component").is_some_and(|v| !v.is_null()) { component_id(s, p, cmd)? } else { 0 };
    let o = options(p, cmd)?;
    let ix = Index::new(s);
    let counts = counts(&ix, root, o.levels, o.fold, cmd)?;
    // Folding: what one of each numbered part holds, as parts at any depth. A purchased or
    // inseparable part already keeps what it holds out of the list.
    let mut kits: BTreeMap<u64, BTreeMap<Placed, u64>> = BTreeMap::new();
    if o.fold {
        for &(id, _) in counts.keys().filter(|(id, bom_type)| *bom_type == BomType::Normal && ix.numbered.contains(id)) {
            let kit = self::counts(&ix, id, Levels::Flat, false, cmd)?;
            if !kit.is_empty() {
                kits.insert(id, kit);
            }
        }
    }
    let mut weighed: BTreeMap<(u64, Holds), Weighed> = BTreeMap::new();
    let mut total: u64 = 0;
    let mut rows = Vec::new();
    for (i, r) in self::rows(s, &counts, o.group, &kits).into_iter().enumerate() {
        total += r.count;
        let mut v = json!({"item": i + 1, "part_number": r.part_number, "description": r.description, "revision": r.revision,
            "bom_type": r.bom_type.as_str(), "count": r.count, "definition": r.names.join("; "),
            "material": ix.material(s, &r.components), "components": r.components, "properties": r.properties});
        if o.mass {
            let folded = !r.kit.is_empty();
            v["mass_g"] = mass(s, &ix, &r.components, holds(o.levels, r.bom_type, folded), &mut weighed, cmd)?;
        }
        if !r.kit.is_empty() {
            let mut children = Vec::new();
            // What a folded part holds, as parts only lists it.
            for (j, c) in self::rows(s, &r.kit, Group::PartNumber, &BTreeMap::new()).into_iter().enumerate() {
                let count = c.count.saturating_mul(r.count);
                total += count;
                let mut child = json!({"item": format!("{}.{}", i + 1, j + 1), "part_number": c.part_number,
                    "description": c.description, "revision": c.revision, "bom_type": c.bom_type.as_str(), "each": c.count,
                    "count": count, "definition": c.names.join("; "), "material": ix.material(s, &c.components)});
                if o.mass {
                    child["mass_g"] = mass(s, &ix, &c.components, holds(Levels::Flat, c.bom_type, false), &mut weighed, cmd)?;
                }
                children.push(child);
            }
            v["children"] = Value::Array(children);
        }
        rows.push(v);
    }
    let mut out = Map::new();
    out.insert("component".into(), json!(root));
    out.insert("levels".into(), json!(if o.levels == Levels::Top { "top" } else { "flat" }));
    out.insert("group_by".into(), json!([if o.group == Group::Definition { "definition" } else { "part_number" }]));
    out.insert("group_children".into(), json!(o.fold));
    out.insert("mass".into(), json!(o.mass));
    if let Some(path) = str_(p, "path") {
        let format = match str_(p, "format").map(str::to_ascii_lowercase) {
            Some(f) => f,
            None if path.to_ascii_lowercase().ends_with(".json") => "json".into(),
            None if path.to_ascii_lowercase().ends_with(".csv") => "csv".into(),
            None => return Err(bad(cmd, "`path` must end in .csv or .json, or give `format`")),
        };
        let text = match format.as_str() {
            "csv" => csv(&rows),
            "json" => serde_json::to_string_pretty(&json!({"rows": rows, "total_occurrences": total})).map_err(|e| bad(cmd, e.to_string()))?,
            other => return Err(bad(cmd, format!("unknown format `{other}` (csv or json)"))),
        };
        solidcraft_io::vfs::write(path, text.as_bytes()).map_err(|e| EngineError::Other(format!("{path}: {e}")))?;
        out.insert("path".into(), json!(path));
        out.insert("format".into(), json!(format));
    }
    out.insert("rows".into(), Value::Array(rows));
    out.insert("total_occurrences".into(), json!(total));
    Ok(Value::Object(out))
}

#[cfg(test)]
#[path = "bom_tests.rs"]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests;
