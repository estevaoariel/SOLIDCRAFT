//! Goal Seek: an Airflow study run again and again with one to three design parameters changed,
//! to find the values that best meet the goals. A sweep runs every combination of the values
//! asked for; an optimization (Nelder–Mead in the parameters' ranges) spends a budget of cycles
//! moving toward the best. Each cycle changes the parameters on a scratch copy of the design (the
//! design itself is untouched until a cycle is applied, as one undo step), sets the study up on the
//! rebuilt model and starts from the previous cycle's flow, so it settles sooner.
//!
//! What a cycle is judged on: the study's openings (inlets and outlets by name), the pressure drop
//! (`pressure_drop`), and regions of its own: boxes (mean and top speed, mean pressure in them)
//! and discs (the flow through them). Goals maximize, minimize, hit a target, even out several
//! measures, or set limits that rule a design out; each counts by its weight.

use std::collections::BTreeMap;
use std::sync::Arc;

use serde_json::{Value, json};
use solidcraft_geom::Vec3;

use crate::flow::{FlowResult, Job};

/// Most cycles one seek runs.
pub const MAX_CYCLES: usize = 500;

#[derive(Clone, Debug, PartialEq)]
pub struct Vary {
    pub parameter: String,
    pub min: f64,
    pub max: f64,
    /// Values a sweep takes across the range (2 or more), ends included.
    pub count: usize,
    /// The parameter's unit, as values are written back (`mm`, `deg`, or none).
    pub unit: String,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mode {
    Sweep,
    Optimize { budget: usize },
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Shape {
    Box { min: Vec3, max: Vec3 },
    Disc { centre: Vec3, normal: Vec3, radius: f64 },
}

#[derive(Clone, Debug, PartialEq)]
pub struct Region {
    pub name: String,
    pub shape: Shape,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Quantity {
    /// m³/s through an opening or a disc.
    Flow,
    /// Mean gauge pressure, Pa.
    Pressure,
    /// Mean speed, m/s.
    Speed,
    /// Top speed, m/s.
    MaxSpeed,
}

impl Quantity {
    pub fn parse(s: &str) -> Option<Quantity> {
        Some(match s {
            "flow" => Quantity::Flow,
            "pressure" => Quantity::Pressure,
            "speed" => Quantity::Speed,
            "max_speed" => Quantity::MaxSpeed,
            _ => return None,
        })
    }

    pub fn id(self) -> &'static str {
        match self {
            Quantity::Flow => "flow",
            Quantity::Pressure => "pressure",
            Quantity::Speed => "speed",
            Quantity::MaxSpeed => "max_speed",
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub enum Goal {
    Maximize {
        measure: String,
        quantity: Quantity,
        weight: f64,
    },
    Minimize {
        measure: String,
        quantity: Quantity,
        weight: f64,
    },
    Target {
        measure: String,
        quantity: Quantity,
        value: f64,
        weight: f64,
    },
    /// The measures as even as possible (their spread over their mean).
    Equalize {
        measures: Vec<String>,
        quantity: Quantity,
        weight: f64,
    },
    /// A design outside min..max is ruled out.
    Limit {
        measure: String,
        quantity: Quantity,
        min: Option<f64>,
        max: Option<f64>,
    },
}

#[derive(Clone, Debug, PartialEq)]
pub struct SeekSpec {
    pub study: String,
    pub vary: Vec<Vary>,
    pub mode: Mode,
    pub goals: Vec<Goal>,
    pub regions: Vec<Region>,
}

/// One measure of a cycle.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Measured {
    pub flow: f64,
    pub pressure: f64,
    pub speed: f64,
    pub max_speed: f64,
}

impl Measured {
    pub fn get(&self, q: Quantity) -> f64 {
        match q {
            Quantity::Flow => self.flow,
            Quantity::Pressure => self.pressure,
            Quantity::Speed => self.speed,
            Quantity::MaxSpeed => self.max_speed,
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct Cycle {
    /// The parameters' values, in `SeekSpec::vary` order.
    pub values: Vec<f64>,
    pub measures: BTreeMap<String, Measured>,
    /// Lower is better.
    pub score: f64,
    pub feasible: bool,
    pub error: Option<String>,
    pub steps: u64,
    pub seconds: f64,
}

#[derive(Clone, Debug, PartialEq)]
pub enum SeekState {
    Ready,
    Running,
    Done,
    Cancelled,
}

/// Nelder–Mead in the unit cube, asked for points and told their scores one at a time.
#[derive(Clone, Debug)]
pub struct NelderMead {
    n: usize,
    simplex: Vec<(Vec<f64>, f64)>,
    queue: Vec<Vec<f64>>,
    stage: Stage,
}

#[derive(Clone, Debug)]
enum Stage {
    /// Building the first simplex.
    Init,
    Reflect {
        xr: Vec<f64>,
    },
    Expand {
        xr: Vec<f64>,
        fr: f64,
        xe: Vec<f64>,
    },
    Contract {
        xc: Vec<f64>,
        beat: f64,
    },
    Shrink {
        left: usize,
    },
}

fn clamp01(x: Vec<f64>) -> Vec<f64> {
    x.into_iter().map(|v| if v.is_finite() { v.clamp(0.0, 1.0) } else { 0.5 }).collect()
}

impl NelderMead {
    pub fn new(n: usize) -> NelderMead {
        let start = vec![0.5; n];
        let mut queue = vec![start.clone()];
        for i in 0..n {
            let mut v = start.clone();
            if let Some(x) = v.get_mut(i) {
                *x += 0.3;
            }
            queue.push(clamp01(v));
        }
        NelderMead { n, simplex: Vec::new(), queue, stage: Stage::Init }
    }

    /// The next point to evaluate (none once the simplex has shrunk to nothing).
    pub fn ask(&mut self) -> Option<Vec<f64>> {
        if let Some(x) = self.queue.first() {
            return Some(x.clone());
        }
        if self.simplex.len() < self.n + 1 {
            return None;
        }
        self.simplex.sort_by(|a, b| a.1.total_cmp(&b.1));
        let best = self.simplex.first().map(|s| s.0.clone())?;
        let size = self.simplex.iter().map(|(x, _)| x.iter().zip(&best).map(|(a, b)| (a - b).abs()).fold(0.0, f64::max)).fold(0.0, f64::max);
        if size < 1e-3 {
            return None;
        }
        let worst = self.simplex.last().map(|s| s.0.clone())?;
        let c = self.centroid();
        let xr = clamp01(c.iter().zip(&worst).map(|(c, w)| c + (c - w)).collect());
        self.stage = Stage::Reflect { xr: xr.clone() };
        self.queue.push(xr.clone());
        Some(xr)
    }

    fn centroid(&self) -> Vec<f64> {
        let mut c = vec![0.0; self.n];
        let k = self.simplex.len().saturating_sub(1).max(1) as f64;
        for (x, _) in self.simplex.iter().take(self.simplex.len().saturating_sub(1)) {
            for (ci, xi) in c.iter_mut().zip(x) {
                *ci += xi / k;
            }
        }
        c
    }

    fn replace_worst(&mut self, x: Vec<f64>, f: f64) {
        if let Some(last) = self.simplex.last_mut() {
            *last = (x, f);
        }
        self.stage = Stage::Init;
    }

    fn shrink(&mut self) {
        let Some(best) = self.simplex.first().map(|s| s.0.clone()) else { return };
        let moved: Vec<Vec<f64>> =
            self.simplex.iter().skip(1).map(|(x, _)| clamp01(x.iter().zip(&best).map(|(x, b)| b + 0.5 * (x - b)).collect())).collect();
        self.simplex.truncate(1);
        self.queue = moved;
        self.stage = Stage::Shrink { left: self.queue.len() };
    }

    /// The score of the point last asked for.
    pub fn tell(&mut self, f: f64) {
        let Some(x) = (!self.queue.is_empty()).then(|| self.queue.remove(0)) else { return };
        let f = if f.is_finite() { f } else { 1e12 };
        match std::mem::replace(&mut self.stage, Stage::Init) {
            Stage::Init => self.simplex.push((x, f)),
            Stage::Shrink { left } => {
                self.simplex.push((x, f));
                if left > 1 {
                    self.stage = Stage::Shrink { left: left - 1 };
                }
            }
            Stage::Reflect { xr } => {
                let fb = self.simplex.first().map_or(f64::INFINITY, |s| s.1);
                let fsw = self.simplex.iter().rev().nth(1).map_or(f64::INFINITY, |s| s.1);
                let fw = self.simplex.last().map_or(f64::INFINITY, |s| s.1);
                let c = self.centroid();
                if f < fb {
                    let xe = clamp01(c.iter().zip(&xr).map(|(c, r)| c + 2.0 * (r - c)).collect());
                    self.queue.push(xe.clone());
                    self.stage = Stage::Expand { xr, fr: f, xe };
                } else if f < fsw {
                    self.replace_worst(xr, f);
                } else {
                    let worst = self.simplex.last().map(|s| s.0.clone()).unwrap_or_else(|| c.clone());
                    let (from, beat) = if f < fw { (xr, f) } else { (worst, fw) };
                    let xc = clamp01(c.iter().zip(&from).map(|(c, p)| c + 0.5 * (p - c)).collect());
                    self.queue.push(xc.clone());
                    self.stage = Stage::Contract { xc, beat };
                }
            }
            Stage::Expand { xr, fr, xe } => {
                if f < fr {
                    self.replace_worst(xe, f);
                } else {
                    self.replace_worst(xr, fr);
                }
            }
            Stage::Contract { xc, beat } => {
                if f < beat {
                    self.replace_worst(xc, f);
                } else {
                    self.shrink();
                }
            }
        }
    }
}

#[derive(Clone, Debug)]
pub enum Plan {
    Sweep { points: Vec<Vec<f64>>, next: usize },
    Optimize { nm: NelderMead, budget: usize },
}

impl Plan {
    pub fn new(spec: &SeekSpec) -> Plan {
        match spec.mode {
            Mode::Sweep => {
                let mut points = vec![Vec::new()];
                for v in &spec.vary {
                    let n = v.count.max(2);
                    let mut next = Vec::new();
                    for p in &points {
                        for i in 0..n {
                            let mut q = p.clone();
                            q.push(v.min + (v.max - v.min) * i as f64 / (n - 1) as f64);
                            next.push(q);
                        }
                    }
                    points = next;
                }
                Plan::Sweep { points, next: 0 }
            }
            Mode::Optimize { budget } => Plan::Optimize { nm: NelderMead::new(spec.vary.len()), budget },
        }
    }

    /// How many cycles the plan runs at most.
    pub fn total(&self) -> usize {
        match self {
            Plan::Sweep { points, .. } => points.len(),
            Plan::Optimize { budget, .. } => *budget,
        }
    }

    /// The next parameter values to run (`done` cycles so far), in the parameters' own units.
    pub fn ask(&mut self, spec: &SeekSpec, done: usize) -> Option<Vec<f64>> {
        match self {
            Plan::Sweep { points, next } => points.get(*next).cloned(),
            Plan::Optimize { nm, budget } => {
                if done >= *budget {
                    return None;
                }
                let t = nm.ask()?;
                Some(spec.vary.iter().zip(&t).map(|(v, t)| v.min + (v.max - v.min) * t).collect())
            }
        }
    }

    pub fn tell(&mut self, score: f64) {
        match self {
            Plan::Sweep { next, .. } => *next += 1,
            Plan::Optimize { nm, .. } => nm.tell(score),
        }
    }
}

pub struct Seek {
    pub name: String,
    pub spec: SeekSpec,
    pub cycles: Vec<Cycle>,
    pub plan: Plan,
    pub state: SeekState,
    /// The cycle being run: its values and its flow job.
    pub running: Option<(Vec<f64>, Job)>,
    /// The last cycle's result (the next one starts from it) and the best one's.
    pub last: Option<Arc<FlowResult>>,
    pub best: Option<(usize, Arc<FlowResult>)>,
    /// Each goal's scale, from the first cycle that measured it (the goals' terms are fractions
    /// of it, so goals in different units weigh alike).
    pub scales: Vec<f64>,
    pub cancel: bool,
    /// Run cycles in the calling thread (scripts' `wait`, or where no thread can start).
    pub wait: bool,
    pub revision: u64,
}

impl Seek {
    pub fn new(name: String, spec: SeekSpec) -> Seek {
        let plan = Plan::new(&spec);
        Seek {
            name,
            spec,
            cycles: Vec::new(),
            plan,
            state: SeekState::Ready,
            running: None,
            last: None,
            best: None,
            scales: Vec::new(),
            cancel: false,
            wait: false,
            revision: 0,
        }
    }

    /// The best feasible cycle so far.
    pub fn best_cycle(&self) -> Option<(usize, &Cycle)> {
        self.cycles.iter().enumerate().filter(|(_, c)| c.feasible && c.error.is_none()).min_by(|a, b| a.1.score.total_cmp(&b.1.score))
    }
}

/// The measures of a finished run: its openings, the pressure drop, and the seek's regions.
pub fn measures(r: &FlowResult, regions: &[Region]) -> BTreeMap<String, Measured> {
    let mut out = BTreeMap::new();
    let a = &r.average;
    for (t, n) in a.inlets.iter().zip(&r.inlets).chain(a.outlets.iter().zip(&r.outlets)) {
        out.insert(n.clone(), Measured { flow: t.flow, pressure: t.pressure, speed: t.speed, max_speed: t.max_speed });
    }
    let mean = |ts: &[solidcraft_sim::Through]| {
        let area: f64 = ts.iter().map(|t| t.area).sum();
        if area > 0.0 { ts.iter().map(|t| t.pressure * t.area).sum::<f64>() / area } else { 0.0 }
    };
    out.insert("pressure_drop".into(), Measured { pressure: mean(&a.inlets) - mean(&a.outlets), ..Default::default() });
    for region in regions {
        let m = match region.shape {
            Shape::Box { min, max } => {
                r.fields.box_stats(min, max).map(|(speed, top, pressure, _)| Measured { flow: 0.0, pressure, speed, max_speed: top })
            }
            Shape::Disc { centre, normal, radius } => r.fields.disc_flow(centre, normal, radius).map(|(flow, pressure, area)| Measured {
                flow,
                pressure,
                speed: if area > 0.0 { flow / area } else { 0.0 },
                max_speed: 0.0,
            }),
        };
        out.insert(region.name.clone(), m.unwrap_or_default());
    }
    out
}

/// A cycle's score (lower is better) and whether it keeps to the limits. `scales` is filled from
/// the first cycle (one per goal).
pub fn score(goals: &[Goal], m: &BTreeMap<String, Measured>, scales: &mut Vec<f64>) -> (f64, bool) {
    let get = |name: &str, q: Quantity| m.get(name).map(|x| x.get(q));
    let mut total = 0.0;
    let mut feasible = true;
    for (i, g) in goals.iter().enumerate() {
        let raw = match g {
            Goal::Maximize { measure, quantity, .. } | Goal::Minimize { measure, quantity, .. } => get(measure, *quantity),
            Goal::Target { measure, quantity, .. } => get(measure, *quantity),
            Goal::Equalize { measures, quantity, .. } => {
                let v: Vec<f64> = measures.iter().filter_map(|n| get(n, *quantity)).collect();
                (!v.is_empty()).then(|| {
                    let mean = v.iter().sum::<f64>() / v.len() as f64;
                    let var = v.iter().map(|x| (x - mean) * (x - mean)).sum::<f64>() / v.len() as f64;
                    if mean.abs() > 0.0 { var.sqrt() / mean.abs() } else { var.sqrt() }
                })
            }
            Goal::Limit { measure, quantity, .. } => get(measure, *quantity),
        };
        let Some(v) = raw else { continue };
        if scales.len() <= i {
            scales.resize(i + 1, 0.0);
        }
        if let Some(s) = scales.get_mut(i)
            && *s == 0.0
        {
            *s = v.abs().max(1e-12);
        }
        let scale = scales.get(i).copied().unwrap_or(1.0).max(1e-12);
        total += match g {
            Goal::Maximize { weight, .. } => -weight * v / scale,
            Goal::Minimize { weight, .. } => weight * v / scale,
            Goal::Target { value, weight, .. } => weight * (v - value).abs() / value.abs().max(scale),
            Goal::Equalize { weight, .. } => weight * v,
            Goal::Limit { min, max, .. } => {
                if min.is_some_and(|lo| v < lo) || max.is_some_and(|hi| v > hi) {
                    feasible = false;
                }
                0.0
            }
        };
    }
    (total, feasible)
}

pub fn spec_json(s: &SeekSpec) -> Value {
    json!({
        "study": s.study,
        "vary": s.vary.iter().map(|v| json!({"parameter": v.parameter, "min": v.min, "max": v.max, "count": v.count, "unit": v.unit})).collect::<Vec<_>>(),
        "mode": match s.mode { Mode::Sweep => "sweep", Mode::Optimize { .. } => "optimize" },
        "budget": match s.mode { Mode::Sweep => None, Mode::Optimize { budget } => Some(budget) },
        "goals": s.goals.iter().map(goal_json).collect::<Vec<_>>(),
        "regions": s.regions.iter().map(|r| match r.shape {
            Shape::Box { min, max } => json!({"name": r.name, "box": {"min": [min.x, min.y, min.z], "max": [max.x, max.y, max.z]}}),
            Shape::Disc { centre, normal, radius } => json!({"name": r.name, "disc": {"center": [centre.x, centre.y, centre.z], "normal": [normal.x, normal.y, normal.z], "radius": radius}}),
        }).collect::<Vec<_>>(),
    })
}

pub fn goal_json(g: &Goal) -> Value {
    match g {
        Goal::Maximize { measure, quantity, weight } => json!({"goal": "maximize", "measure": measure, "quantity": quantity.id(), "weight": weight}),
        Goal::Minimize { measure, quantity, weight } => json!({"goal": "minimize", "measure": measure, "quantity": quantity.id(), "weight": weight}),
        Goal::Target { measure, quantity, value, weight } => {
            json!({"goal": "target", "measure": measure, "quantity": quantity.id(), "value": value, "weight": weight})
        }
        Goal::Equalize { measures, quantity, weight } => {
            json!({"goal": "equalize", "measures": measures, "quantity": quantity.id(), "weight": weight})
        }
        Goal::Limit { measure, quantity, min, max } => {
            json!({"goal": "limit", "measure": measure, "quantity": quantity.id(), "min": min, "max": max})
        }
    }
}

pub fn cycle_json(spec: &SeekSpec, i: usize, c: &Cycle) -> Value {
    let values: serde_json::Map<String, Value> = spec.vary.iter().zip(&c.values).map(|(v, x)| (v.parameter.clone(), json!(x))).collect();
    let measures: serde_json::Map<String, Value> = c
        .measures
        .iter()
        .map(|(k, m)| (k.clone(), json!({"flow_m3s": m.flow, "flow_cfm": m.flow * solidcraft_sim::study::CFM_PER_M3S, "pressure_pa": m.pressure, "speed_ms": m.speed, "max_speed_ms": m.max_speed})))
        .collect();
    json!({"cycle": i, "values": values, "measures": measures, "score": c.score, "feasible": c.feasible, "error": c.error, "steps": c.steps, "seconds": c.seconds})
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Nelder–Mead finds the bottom of a bowl in the unit square within its budget.
    #[test]
    fn nelder_mead_finds_a_bowls_bottom() {
        let mut nm = NelderMead::new(2);
        let f = |x: &[f64]| (x[0] - 0.2).powi(2) + 2.0 * (x[1] - 0.7).powi(2);
        let mut best = f64::INFINITY;
        let mut at = vec![];
        for _ in 0..80 {
            let Some(x) = nm.ask() else { break };
            let v = f(&x);
            if v < best {
                best = v;
                at = x.clone();
            }
            nm.tell(v);
        }
        assert!(best < 1e-4, "{best} at {at:?}");
    }

    #[test]
    fn a_sweep_takes_every_combination_and_scores_by_goal() {
        let spec = SeekSpec {
            study: "s".into(),
            vary: vec![
                Vary { parameter: "a".into(), min: 0.0, max: 10.0, count: 3, unit: "mm".into() },
                Vary { parameter: "b".into(), min: 1.0, max: 2.0, count: 2, unit: String::new() },
            ],
            mode: Mode::Sweep,
            goals: vec![],
            regions: vec![],
        };
        let mut plan = Plan::new(&spec);
        assert_eq!(plan.total(), 6);
        let mut seen = Vec::new();
        while let Some(v) = plan.ask(&spec, seen.len()) {
            seen.push(v);
            plan.tell(0.0);
        }
        assert_eq!(seen.len(), 6);
        assert!(seen.contains(&vec![5.0, 2.0]));
        let m: BTreeMap<String, Measured> = [
            ("Out A".to_string(), Measured { flow: 2.0, ..Default::default() }),
            ("Out B".to_string(), Measured { flow: 1.0, ..Default::default() }),
        ]
        .into();
        let goals = vec![
            Goal::Maximize { measure: "Out A".into(), quantity: Quantity::Flow, weight: 1.0 },
            Goal::Equalize { measures: vec!["Out A".into(), "Out B".into()], quantity: Quantity::Flow, weight: 1.0 },
            Goal::Limit { measure: "Out B".into(), quantity: Quantity::Flow, min: Some(1.5), max: None },
        ];
        let mut scales = Vec::new();
        let (s, feasible) = score(&goals, &m, &mut scales);
        assert!(!feasible, "Out B under its limit rules the design out");
        assert!((s - (-1.0 + 1.0 / 3.0)).abs() < 1e-9, "{s}");
    }
}
