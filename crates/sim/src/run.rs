//! Running a prepared study: a [`Stepper`] advances its lattice (the CPU reference here; the app
//! supplies the GPU one through a [`Backend`]), and [`Run`] steps it a stretch at a time,
//! measures the openings, moves fan inlets along their curves, and stops once the flow has
//! settled. [`Fields`] holds the result in physical units (m/s, Pa) for colour maps, section
//! planes and streamlines.

use solidcraft_geom::Vec3;

use crate::study::{LATTICE_SPEED, Prepared, Through};
use crate::{Lattice, Result, SimError};

/// Advances one lattice.
pub trait Stepper: Send {
    /// Take `steps` time steps.
    fn run(&mut self, steps: u32) -> Result<()>;
    /// Copy the latest populations into `lattice` (the one the stepper was made from).
    fn read(&mut self, lattice: &mut Lattice) -> Result<()>;
    /// `lattice`'s openings changed (a fan inlet's speed): take them over.
    fn openings(&mut self, lattice: &Lattice) -> Result<()>;
}

/// Makes steppers: [`Cpu`], or the app's GPU.
pub trait Backend: Send + Sync {
    /// What runs the study, for reports ("CPU", the GPU's name).
    fn name(&self) -> String;
    fn stepper(&self, lattice: &Lattice) -> Result<Box<dyn Stepper>>;
    /// Most cells a lattice may have to run here, where the backend knows (a GPU's memory).
    fn max_cells(&self) -> Option<u64> {
        None
    }
}

/// The CPU reference (f64; slow, for tests and machines without a GPU).
pub struct Cpu;

struct CpuStepper(Lattice);

impl Stepper for CpuStepper {
    fn run(&mut self, steps: u32) -> Result<()> {
        for _ in 0..steps {
            self.0.step();
        }
        Ok(())
    }

    fn read(&mut self, lattice: &mut Lattice) -> Result<()> {
        lattice.set_populations(self.0.populations())
    }

    fn openings(&mut self, lattice: &Lattice) -> Result<()> {
        let count = lattice.openings().len();
        if count != self.0.openings().len() {
            return Err(SimError::BadOpening);
        }
        for (i, o) in lattice.openings().iter().enumerate() {
            self.0.set_openings(i..i + 1, o.2)?;
        }
        Ok(())
    }
}

impl Backend for Cpu {
    fn name(&self) -> String {
        "CPU".into()
    }

    fn stepper(&self, lattice: &Lattice) -> Result<Box<dyn Stepper>> {
        Ok(Box::new(CpuStepper(lattice.clone())))
    }
}

/// When a run stops.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Limits {
    pub max_steps: u64,
    /// Steps between measurements.
    pub check_every: u32,
    /// Settled once the openings' flows and pressures change by less than this fraction over
    /// the last few measurements.
    pub tolerance: f64,
    /// Ramp the inlets up from rest (off for a warm start from an earlier flow).
    pub soft_start: bool,
}

impl Default for Limits {
    fn default() -> Self {
        Limits { max_steps: 200_000, check_every: 500, tolerance: 0.005, soft_start: true }
    }
}

/// One measurement while the flow develops.
#[derive(Clone, Debug, PartialEq)]
pub struct Sample {
    pub step: u64,
    /// Simulated time, s.
    pub time: f64,
    pub inlets: Vec<Through>,
    pub outlets: Vec<Through>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum State {
    Running,
    /// The flow stopped changing.
    Settled,
    /// It ran its most steps without settling; the result is the last state.
    StepLimit,
}

pub struct Run {
    pub prepared: Prepared,
    stepper: Box<dyn Stepper>,
    pub backend: String,
    pub steps: u64,
    pub history: Vec<Sample>,
    pub limits: Limits,
    pub state: State,
    /// Steps for the air to cross the box once at the fastest inlet speed: no run settles
    /// sooner than one crossing.
    pub crossing: u64,
    /// The soft start: the inlets ramp up from rest over this many steps (one crossing), so
    /// the air in a long duct is not slammed into motion.
    pub ramp: u64,
    /// The inlets' share of their speed now (1 once the ramp is over).
    scale: f64,
}

/// Steps between the soft start's speed changes.
const RAMP_STRETCH: u64 = 50;

/// How many measurements in a row must agree to call the flow settled.
const SETTLED_SAMPLES: usize = 4;

impl Run {
    pub fn new(prepared: Prepared, backend: &dyn Backend, limits: Limits) -> Result<Run> {
        if limits.check_every == 0 || limits.max_steps == 0 || !(limits.tolerance.is_finite() && limits.tolerance > 0.0) {
            return Err(SimError::Backend("the run needs a step limit, a check interval and a positive tolerance".into()));
        }
        let stepper = backend.stepper(&prepared.lattice)?;
        let longest = prepared.grid.dims.iter().copied().max().unwrap_or(1) as f64;
        let crossing = (longest / LATTICE_SPEED).ceil() as u64;
        let ramp = if limits.soft_start { crossing } else { 0 };
        let mut run = Run {
            prepared,
            stepper,
            backend: backend.name(),
            steps: 0,
            history: Vec::new(),
            limits,
            state: State::Running,
            crossing,
            ramp,
            scale: 1.0,
        };
        if ramp > 0 {
            run.set_scale(0.0)?;
        }
        Ok(run)
    }

    fn set_scale(&mut self, scale: f64) -> Result<()> {
        self.prepared.scale_inlets(scale)?;
        self.stepper.openings(&self.prepared.lattice)?;
        self.scale = scale;
        Ok(())
    }

    /// The latest measurement.
    pub fn last(&self) -> Option<&Sample> {
        self.history.last()
    }

    /// Run one stretch of steps (or what is left of the limit), read the result back and
    /// measure it. Errors when the flow goes unstable.
    pub fn advance(&mut self) -> Result<State> {
        if self.state != State::Running {
            return Ok(self.state);
        }
        let left = self.limits.max_steps.saturating_sub(self.steps);
        let n = left.min(u64::from(self.limits.check_every));
        let mut done = 0;
        while done < n {
            let chunk = if self.steps < self.ramp { RAMP_STRETCH.min(n - done) } else { n - done };
            if self.steps < self.ramp {
                self.set_scale(((self.steps + chunk) as f64 / self.ramp as f64).min(1.0))?;
            } else if self.scale < 1.0 {
                self.set_scale(1.0)?;
            }
            self.stepper.run(u32::try_from(chunk).unwrap_or(u32::MAX))?;
            self.steps += chunk;
            done += chunk;
        }
        // Populations that are no longer finite (refused on the way back) mean the flow blew up.
        match self.stepper.read(&mut self.prepared.lattice) {
            Err(SimError::BadPopulations) => return Err(SimError::Unstable { steps: self.steps }),
            Err(SimError::Backend(e)) if e.contains("finite") => return Err(SimError::Unstable { steps: self.steps }),
            r => r?,
        }
        if !self.stable() {
            return Err(SimError::Unstable { steps: self.steps });
        }
        let p = &self.prepared;
        let sample = Sample {
            step: self.steps,
            time: p.units.time(self.steps),
            inlets: p.inlets.iter().map(|l| p.through(&p.lattice, l)).collect(),
            outlets: p.outlets.iter().map(|l| p.through(&p.lattice, l)).collect(),
        };
        self.history.push(sample);
        // The inlets ease around their mean density; fans follow their curves once the soft
        // start is over.
        self.prepared.track_inlets(0.3)?;
        let moved = if self.scale < 1.0 { 1.0 } else { self.prepared.follow_fans(0.3)? };
        self.stepper.openings(&self.prepared.lattice)?;
        if moved < self.limits.tolerance && self.steps >= self.crossing && self.settled() {
            self.state = State::Settled;
        } else if self.steps >= self.limits.max_steps {
            self.state = State::StepLimit;
        }
        Ok(self.state)
    }

    /// Run until settled or out of steps; `progress` is told after every stretch and may return
    /// false to cancel.
    pub fn finish(&mut self, mut progress: impl FnMut(&Run) -> bool) -> Result<State> {
        while self.advance()? == State::Running {
            if !progress(self) {
                return Err(SimError::Cancelled);
            }
        }
        Ok(self.state)
    }

    fn stable(&self) -> bool {
        let l = &self.prepared.lattice;
        let [nx, ny, nz] = l.dims;
        // Every fluid cell finite, and the density within reason (pressure waves stay small).
        for k in 0..nz {
            for j in 0..ny {
                for i in 0..nx {
                    if let Some((rho, u)) = l.moments(i, j, k)
                        && (!rho.is_finite() || !(0.5..2.0).contains(&rho) || u.iter().any(|v| !v.is_finite() || v.abs() > 0.5))
                    {
                        return false;
                    }
                }
            }
        }
        true
    }

    /// The mean of the measurements in `samples`.
    fn mean(samples: &[Sample]) -> Option<Sample> {
        let last = samples.last()?;
        let n = samples.len() as f64;
        let avg = |pick: fn(&Sample) -> &Vec<Through>| -> Vec<Through> {
            (0..pick(last).len())
                .map(|i| {
                    let mut t = Through::default();
                    for x in samples.iter().filter_map(|s| pick(s).get(i)) {
                        t.flow += x.flow / n;
                        t.pressure += x.pressure / n;
                        t.speed += x.speed / n;
                        t.max_speed = t.max_speed.max(x.max_speed);
                        t.area = x.area;
                    }
                    t
                })
                .collect()
        };
        Some(Sample { step: last.step, time: last.time, inlets: avg(|s| &s.inlets), outlets: avg(|s| &s.outlets) })
    }

    /// The result's measurements: the mean over the last quarter of the run (at least the last
    /// few measurements), as unsteady flow keeps moving about its mean.
    pub fn average(&self) -> Option<Sample> {
        let n = (self.history.len() / 4).max(SETTLED_SAMPLES).min(self.history.len());
        Self::mean(self.history.get(self.history.len() - n..)?)
    }

    /// The mean of the last few measurements agrees with the mean of the few before: flows
    /// within the tolerance of each other, pressures within the tolerance of the dynamic
    /// pressure.
    fn settled(&self) -> bool {
        let len = self.history.len();
        let recent = len.checked_sub(SETTLED_SAMPLES).and_then(|from| self.history.get(from..)).and_then(Self::mean);
        let before = len.checked_sub(2 * SETTLED_SAMPLES).and_then(|from| self.history.get(from..len - SETTLED_SAMPLES)).and_then(Self::mean);
        let (Some(recent), Some(before)) = (recent, before) else { return false };
        let p = &self.prepared;
        let fastest = p.speeds.iter().copied().fold(0.0, f64::max);
        let dynamic = (0.5 * p.units.density * fastest * fastest).max(1e-9);
        let tol = self.limits.tolerance;
        let flows = |s: &Sample| -> Vec<(f64, f64)> { s.inlets.iter().chain(&s.outlets).map(|t| (t.flow, t.pressure)).collect() };
        let (a, b) = (flows(&recent), flows(&before));
        let scale = a.iter().map(|(q, _)| q.abs()).fold(0.0, f64::max).max(1e-15);
        a.iter().zip(&b).all(|((q, pr), (q0, p0))| (q - q0).abs() <= tol * scale && (pr - p0).abs() <= tol * dynamic)
    }

    /// The result in physical units.
    pub fn fields(&self) -> Fields {
        Fields::of(&self.prepared)
    }
}

/// A flow field in physical units on the study's grid.
#[derive(Clone, Debug, PartialEq)]
pub struct Fields {
    pub dims: [usize; 3],
    /// The corner of cell (0, 0, 0), mm.
    pub origin: Vec3,
    /// Cell size, mm.
    pub cell: f64,
    pub fluid: Vec<bool>,
    /// m/s.
    pub velocity: Vec<[f32; 3]>,
    /// Gauge pressure, Pa.
    pub pressure: Vec<f32>,
}

/// What a colour map shows.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Quantity {
    Speed,
    Pressure,
    /// One velocity component (0 = x, 1 = y, 2 = z).
    Component(u8),
}

impl Fields {
    pub fn of(p: &Prepared) -> Fields {
        let l = &p.lattice;
        let n = p.grid.len();
        let (mut fluid, mut velocity, mut pressure) = (vec![false; n], vec![[0.0f32; 3]; n], vec![0.0f32; n]);
        for idx in 0..n {
            let [i, j, k] = p.grid.coords(idx);
            if let Some((rho, u)) = l.moments(i, j, k)
                && let (Some(f), Some(v), Some(pr)) = (fluid.get_mut(idx), velocity.get_mut(idx), pressure.get_mut(idx))
            {
                *f = true;
                *v = u.map(|c| p.units.speed(c) as f32);
                *pr = p.units.pressure(rho) as f32;
            }
        }
        Fields { dims: p.grid.dims, origin: p.grid.origin, cell: p.grid.spacing, fluid, velocity, pressure }
    }

    fn index(&self, i: usize, j: usize, k: usize) -> Option<usize> {
        let [nx, ny, nz] = self.dims;
        (i < nx && j < ny && k < nz).then_some(i + nx * (j + ny * k))
    }

    /// A cell's value of `q` (None for a solid cell).
    pub fn value(&self, idx: usize, q: Quantity) -> Option<f64> {
        if !self.fluid.get(idx).copied().unwrap_or(false) {
            return None;
        }
        let v = self.velocity.get(idx)?;
        Some(match q {
            Quantity::Speed => f64::from(v[0]).hypot(f64::from(v[1])).hypot(f64::from(v[2])),
            Quantity::Pressure => f64::from(*self.pressure.get(idx)?),
            Quantity::Component(c) => f64::from(*v.get(usize::from(c))?),
        })
    }

    /// The smallest and largest value of `q` over the fluid cells.
    pub fn range(&self, q: Quantity) -> Option<(f64, f64)> {
        (0..self.fluid.len())
            .filter_map(|i| self.value(i, q))
            .fold(None, |r, v| Some(r.map_or((v, v), |(lo, hi): (f64, f64)| (lo.min(v), hi.max(v)))))
    }

    /// Velocity (m/s) and pressure (Pa) at `p` (mm), interpolated between the fluid cell
    /// centres around it; None away from the air.
    pub fn sample(&self, p: Vec3) -> Option<(Vec3, f64)> {
        let s = self.cell;
        let at = (p - self.origin) / s - Vec3::new(0.5, 0.5, 0.5);
        if !at.is_finite() {
            return None;
        }
        let base = [at.x.floor(), at.y.floor(), at.z.floor()];
        let frac = [at.x - base[0], at.y - base[1], at.z - base[2]];
        let (mut u, mut pr, mut weight) = (Vec3::ZERO, 0.0, 0.0);
        for corner in 0..8usize {
            let off = [corner & 1, (corner >> 1) & 1, (corner >> 2) & 1];
            let c: Vec<f64> = (0..3).map(|a| base[a] + off[a] as f64).collect();
            if c.iter().any(|v| *v < 0.0) {
                continue;
            }
            let Some(idx) = self.index(c[0] as usize, c[1] as usize, c[2] as usize) else { continue };
            if !self.fluid.get(idx).copied().unwrap_or(false) {
                continue;
            }
            let w: f64 = (0..3).map(|a| if off[a] == 1 { frac[a] } else { 1.0 - frac[a] }).product();
            let (Some(v), Some(q)) = (self.velocity.get(idx), self.pressure.get(idx)) else { continue };
            u += Vec3::new(f64::from(v[0]), f64::from(v[1]), f64::from(v[2])) * w;
            pr += f64::from(*q) * w;
            weight += w;
        }
        (weight > 1e-6).then(|| (u / weight, pr / weight))
    }

    /// Over the air cells whose centres lie in the box `min`..`max` (mm): mean speed, top speed
    /// (m/s), mean pressure (Pa) and how many cells; None when the box holds no air.
    pub fn box_stats(&self, min: Vec3, max: Vec3) -> Option<(f64, f64, f64, usize)> {
        let s = self.cell;
        let lo = |v: f64, o: f64| (((v - o) / s) - 0.5).ceil().max(0.0) as usize;
        let hi = |v: f64, o: f64, n: usize| ((((v - o) / s) - 0.5).floor().max(-1.0) + 1.0).min(n as f64) as usize;
        let (mut speed, mut top, mut pressure, mut n) = (0.0, 0.0f64, 0.0, 0usize);
        for k in lo(min.z, self.origin.z)..hi(max.z, self.origin.z, self.dims[2]) {
            for j in lo(min.y, self.origin.y)..hi(max.y, self.origin.y, self.dims[1]) {
                for i in lo(min.x, self.origin.x)..hi(max.x, self.origin.x, self.dims[0]) {
                    let Some(idx) = self.index(i, j, k) else { continue };
                    let (Some(v), Some(p)) = (self.value(idx, Quantity::Speed), self.value(idx, Quantity::Pressure)) else { continue };
                    speed += v;
                    top = top.max(v);
                    pressure += p;
                    n += 1;
                }
            }
        }
        (n > 0).then(|| (speed / n as f64, top, pressure / n as f64, n))
    }

    /// Through the disc at `centre` (mm) facing `normal`, of `radius` (mm), sampled about every
    /// half cell: flow along the normal (m³/s), mean pressure (Pa) and the area found in the air
    /// (m²); None when none of it is in the air.
    pub fn disc_flow(&self, centre: Vec3, normal: Vec3, radius: f64) -> Option<(f64, f64, f64)> {
        let n = normal.normalized()?;
        if !(radius.is_finite() && radius > 0.0) {
            return None;
        }
        let u = n.any_perp();
        let v = n.cross(u);
        let h = (0.5 * self.cell).max(radius / 400.0);
        let steps = (radius / h).ceil() as i64;
        let da = (h * 1e-3) * (h * 1e-3);
        let (mut flow, mut pressure, mut area) = (0.0, 0.0, 0.0);
        for a in -steps..=steps {
            for b in -steps..=steps {
                let (x, y) = (a as f64 * h, b as f64 * h);
                if x * x + y * y > radius * radius {
                    continue;
                }
                let Some((vel, p)) = self.sample(centre + u * x + v * y) else { continue };
                flow += vel.dot(n) * da;
                pressure += p * da;
                area += da;
            }
        }
        (area > 0.0).then(|| (flow, pressure / area, area))
    }

    /// A streamline from `seed` (mm) along the flow: points and the speed there (m/s), stepping
    /// about half a cell at a time (midpoint rule), until it leaves the air, stalls, or has
    /// `max_points`.
    pub fn streamline(&self, seed: Vec3, max_points: usize) -> Vec<(Vec3, f64)> {
        let h = 0.5 * self.cell;
        let mut out = Vec::new();
        let mut p = seed;
        let floor = self.range(Quantity::Speed).map_or(0.0, |(_, hi)| hi * 1e-3);
        for _ in 0..max_points {
            let Some((u, _)) = self.sample(p) else { break };
            let speed = u.len();
            out.push((p, speed));
            if speed <= floor {
                break;
            }
            let mid = p + u * (0.5 * h / speed);
            let Some((um, _)) = self.sample(mid) else { break };
            let Some(dir) = um.normalized() else { break };
            p += dir * h;
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::study::tests::{duct, setup};
    use crate::{Source, prepare};

    /// A run on the CPU settles by itself, its history tracks the flow developing, and a
    /// streamline started in the inlet comes out past the duct's far end.
    #[test]
    fn a_run_settles_and_its_streamlines_follow_the_duct() {
        let (a, w, len) = (8.0, 2.0, 24.0);
        let mesh = duct(a, w, len, 0.0);
        let prepared = prepare(&[&mesh], &setup(a, w, len, Source::Speed(0.05))).unwrap();
        let mut run = Run::new(prepared, &Cpu, Limits { max_steps: 30_000, check_every: 250, tolerance: 0.005, soft_start: true }).unwrap();
        let mut calls = 0;
        assert_eq!(
            run.finish(|_| {
                calls += 1;
                true
            })
            .unwrap(),
            State::Settled
        );
        assert!(run.steps >= run.crossing && run.steps < 30_000, "{} steps", run.steps);
        assert_eq!(calls + 1, run.history.len());
        let first = run.history.first().unwrap().outlets[0].flow;
        let last = run.last().unwrap().outlets[0].flow;
        assert!(last > first, "the outlet flow builds up: {first:e} → {last:e}");
        let fields = run.fields();
        let (lo, hi) = fields.range(Quantity::Speed).unwrap();
        assert!(lo >= 0.0 && hi > 0.05 && hi < 0.2, "{lo}..{hi} m/s");
        let mid = w + 0.5 * a;
        let line = fields.streamline(Vec3::new(1.5, mid, mid), 4_000);
        let end = line.last().unwrap().0;
        assert!(end.x > len, "the streamline reaches {end:?}");
        assert!(line.iter().all(|(p, _)| p.y > w && p.y < w + a || p.x > len), "it stays inside the bore");
        // A cancelled run stops with an error.
        let prepared = prepare(&[&mesh], &setup(a, w, len, Source::Speed(0.05))).unwrap();
        let mut run = Run::new(prepared, &Cpu, Limits::default()).unwrap();
        assert_eq!(run.finish(|_| false).unwrap_err(), SimError::Cancelled);
    }
}
