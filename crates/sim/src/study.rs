//! An Airflow study in physical units: the solid bodies (millimetres), where the air comes in
//! (an opening at a set speed, or a fan's face on its fan curve), where it is expected to leave,
//! and the air. [`prepare`] turns it into a lattice: the bodies voxelized into a box that
//! extends `pad` past them on every side, so jets leaving the outlets keep going in open air,
//! the box's outer faces held at ambient pressure, and each inlet a layer of cells on the fluid
//! side of its patch.
//!
//! Units: a cell is `cell` mm; the time step is chosen so the fastest inlet moves
//! [`LATTICE_SPEED`] cells per step (low Mach number), and the air's viscosity then sets the
//! relaxation time. [`Units`] converts lattice speeds and densities back to m/s and pascals
//! (gauge, against the ambient pressure at the box's faces).

use solidcraft_geom::{Aabb3, Mesh, Vec3};

use crate::{Cell, Grid, Lattice, Opening, Result, SimError};

/// The fastest inlet's speed in lattice units (cells per step): a Mach number of about 0.14,
/// where the lattice's compressibility error stays around 2%.
pub const LATTICE_SPEED: f64 = 0.08;

/// The relaxation time at the box's faces, where the sponge is most viscous.
const SPONGE_TAU: f64 = 0.9;

/// The least share of the Smagorinsky length scale wall damping leaves a cell.
const DAMPING_FLOOR: f64 = 0.25;

/// Van Driest's damping constant. The textbook 26 is for a wall resolved to y⁺ ≈ 1; on cells a
/// millimetre across, the first air cell sits at y⁺ of 10 or more and 26 leaves it too much eddy
/// viscosity. 80 brings a 50 mm pipe's pressure gradient to within 3% of a wall-resolved OpenFOAM
/// solution and of Shah and London's developing-flow correlation (26 read 50% high).
const VAN_DRIEST_A: f64 = 80.0;

/// The bulk relaxation time: sound waves die out within a few passes instead of ringing.
const BULK_TAU: f64 = 1.0;

/// The fluid: density (kg/m³) and kinematic viscosity (m²/s).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Air {
    pub density: f64,
    pub viscosity: f64,
}

/// Dry air at 20 °C and 1 atm.
pub const AIR: Air = Air { density: 1.204, viscosity: 1.516e-5 };

/// One cubic metre per second in cubic feet per minute.
pub const CFM_PER_M3S: f64 = 2118.88;

/// How lattice units map to physical ones.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Units {
    /// Cell size, metres.
    pub dx: f64,
    /// Time step, seconds.
    pub dt: f64,
    /// The air's density, kg/m³.
    pub density: f64,
}

impl Units {
    /// A lattice speed in m/s.
    pub fn speed(&self, lattice: f64) -> f64 {
        lattice * self.dx / self.dt
    }

    /// A speed in m/s in lattice units.
    pub fn lattice_speed(&self, speed: f64) -> f64 {
        speed * self.dt / self.dx
    }

    /// A lattice density as a gauge pressure in pascals (p = c² ρ, c² = ⅓).
    pub fn pressure(&self, rho: f64) -> f64 {
        (rho - 1.0) / 3.0 * self.density * (self.dx / self.dt).powi(2)
    }

    /// A gauge pressure in pascals as a lattice density.
    pub fn lattice_density(&self, pressure: f64) -> f64 {
        1.0 + 3.0 * pressure / (self.density * (self.dx / self.dt).powi(2))
    }

    /// The area of `cells` cell faces, m².
    pub fn area(&self, cells: usize) -> f64 {
        cells as f64 * self.dx * self.dx
    }

    /// `steps` time steps in seconds.
    pub fn time(&self, steps: u64) -> f64 {
        steps as f64 * self.dt
    }
}

/// A fan's pressure rise against its flow: points (flow m³/s, pressure Pa) with the flow rising
/// and the pressure never rising, from the shut-off pressure to the free-air flow.
#[derive(Clone, Debug, PartialEq)]
pub struct FanCurve {
    points: Vec<(f64, f64)>,
}

/// A typical axial fan's curve, as fractions of its free-air flow and its shut-off pressure:
/// steep near shut-off, a flat stretch where the blades stall, then falling to free air.
const AXIAL: [(f64, f64); 8] = [(0.0, 1.0), (0.15, 0.80), (0.3, 0.64), (0.42, 0.60), (0.55, 0.52), (0.7, 0.38), (0.85, 0.21), (1.0, 0.0)];
/// A typical radial (centrifugal) blower: high pressure held over most of its flow range.
const RADIAL: [(f64, f64); 7] = [(0.0, 1.0), (0.2, 0.98), (0.4, 0.93), (0.6, 0.80), (0.75, 0.62), (0.9, 0.32), (1.0, 0.0)];

impl FanCurve {
    pub fn new(points: Vec<(f64, f64)>) -> Result<FanCurve> {
        let ok = points.len() >= 2
            && points.len() <= 64
            && points.iter().all(|(q, p)| q.is_finite() && p.is_finite() && *q >= 0.0 && *p >= 0.0)
            && points.windows(2).all(|w| matches!(w, [a, b] if b.0 > a.0 && b.1 <= a.1))
            && points.first().is_some_and(|p| p.1 > 0.0)
            && points.last().is_some_and(|p| p.0 > 0.0);
        if !ok {
            return Err(SimError::BadFanCurve);
        }
        Ok(FanCurve { points })
    }

    fn scaled(shape: &[(f64, f64)], free_flow: f64, max_pressure: f64) -> Result<FanCurve> {
        if !(free_flow.is_finite() && max_pressure.is_finite() && free_flow > 0.0 && max_pressure > 0.0) {
            return Err(SimError::BadFanCurve);
        }
        FanCurve::new(shape.iter().map(|(q, p)| (q * free_flow, p * max_pressure)).collect())
    }

    /// An axial fan with this free-air flow (m³/s) and shut-off pressure (Pa).
    pub fn axial(free_flow: f64, max_pressure: f64) -> Result<FanCurve> {
        Self::scaled(&AXIAL, free_flow, max_pressure)
    }

    /// A radial blower with this free-air flow (m³/s) and shut-off pressure (Pa).
    pub fn radial(free_flow: f64, max_pressure: f64) -> Result<FanCurve> {
        Self::scaled(&RADIAL, free_flow, max_pressure)
    }

    /// A straight line from the shut-off pressure to the free-air flow.
    pub fn linear(free_flow: f64, max_pressure: f64) -> Result<FanCurve> {
        Self::scaled(&[(0.0, 1.0), (1.0, 0.0)], free_flow, max_pressure)
    }

    pub fn points(&self) -> &[(f64, f64)] {
        &self.points
    }

    /// The flow with no pressure against the fan, m³/s.
    pub fn free_flow(&self) -> f64 {
        self.points.last().map_or(0.0, |p| p.0)
    }

    /// The pressure rise at `flow`: the shut-off pressure below the curve's first flow, 0 past
    /// free air.
    pub fn pressure_at(&self, flow: f64) -> f64 {
        let Some(first) = self.points.first() else { return 0.0 };
        if flow <= first.0 {
            return first.1;
        }
        for w in self.points.windows(2) {
            if let [a, b] = w
                && flow <= b.0
            {
                return a.1 + (b.1 - a.1) * (flow - a.0) / (b.0 - a.0);
            }
        }
        0.0
    }

    /// The flow the fan delivers against `pressure` (Pa): free air at or below 0, none at or
    /// above the shut-off pressure; on a flat stretch, its largest flow.
    pub fn flow_at(&self, pressure: f64) -> f64 {
        if pressure <= 0.0 {
            return self.free_flow();
        }
        for w in self.points.windows(2).rev() {
            if let [a, b] = w
                && pressure <= a.1
            {
                if a.1 - b.1 <= f64::EPSILON * a.1.max(1.0) {
                    return b.0;
                }
                let t = ((a.1 - pressure) / (a.1 - b.1)).clamp(0.0, 1.0);
                return a.0 + t * (b.0 - a.0);
            }
        }
        0.0
    }
}

/// What drives the air through an inlet.
#[derive(Clone, Debug, PartialEq)]
pub enum Source {
    /// A set speed, m/s, along the inlet's normal.
    Speed(f64),
    /// A fan on its curve: the speed follows the pressure the flow builds up at the inlet.
    Fan(FanCurve),
}

/// A flat patch on a body: triangles (millimetres) and the normal pointing into the air the
/// patch faces (for an inlet, the way the air goes in).
#[derive(Clone, Debug, PartialEq)]
pub struct Patch {
    pub triangles: Vec<[Vec3; 3]>,
    pub normal: Vec3,
}

impl Patch {
    /// Its area, mm².
    pub fn area(&self) -> f64 {
        self.triangles.iter().map(|[a, b, c]| (*b - *a).cross(*c - *a).len() * 0.5).sum()
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct Inlet {
    pub name: String,
    pub patch: Patch,
    pub source: Source,
    /// Close the cells just behind the patch: on for a hole in a duct (the open end of a pipe),
    /// off for a fan's face, whose body is solid behind it anyway.
    pub cap: bool,
}

/// Where the air is expected to leave: measured, not imposed (the air flows on past it).
#[derive(Clone, Debug, PartialEq)]
pub struct Outlet {
    pub name: String,
    /// Normal pointing out of the duct, the way the air leaves.
    pub patch: Patch,
}

#[derive(Clone, Debug, PartialEq)]
pub struct FlowSetup {
    /// Cell size, mm.
    pub cell: f64,
    /// How far the box of air extends past the bodies on every side, mm.
    pub pad: f64,
    pub air: Air,
    pub inlets: Vec<Inlet>,
    pub outlets: Vec<Outlet>,
    /// The Smagorinsky constant (0: laminar).
    pub turbulence: f64,
    /// The walls' equivalent sand-grain roughness, mm (0: smooth). It acts through the turbulence
    /// model's wall damping, so it does nothing in a laminar study.
    pub roughness: f64,
    /// Most cells the grid may have.
    pub max_cells: u64,
}

/// One opening's cells in a prepared lattice.
#[derive(Clone, Debug, PartialEq)]
pub struct Layer {
    pub name: String,
    /// Grid cells (fluid), in grid order.
    pub cells: Vec<usize>,
    /// Where its flow is measured: for an inlet the air cells just inside its cells (the
    /// imposed cells themselves carry the set speed, not what crosses into the duct); for an
    /// outlet its own cells.
    pub probe: Vec<usize>,
    /// Unit normal (the way the air goes through).
    pub normal: Vec3,
    /// For an inlet: its openings in [`Lattice::openings`].
    pub openings: std::ops::Range<usize>,
}

/// Something about the setup the results depend on, worth showing next to them.
#[derive(Clone, Debug, PartialEq)]
pub enum Warning {
    /// With the outlets closed, the air still reaches the box's edge from the first inlet: a gap
    /// in the walls, or a wall thinner than a cell.
    Leak,
    /// Walls thinner than a cell (they may let air through or vanish): sample points, mm.
    ThinWalls(Vec<Vec3>),
    /// Inlet or outlet cells dropped because they lay in a wall or against another opening.
    DroppedCells { opening: String, cells: usize },
    /// The air's viscosity on this grid is so low the lattice relies on its turbulence model to
    /// stay stable (relaxation time just over ½).
    LowViscosity { tau: f64 },
    /// The flow went unstable at the usual time step (jets faster than the openings let the
    /// setup foresee), so it ran with steps `pace` times shorter.
    ShortSteps { pace: f64 },
}

pub struct Prepared {
    pub grid: Grid,
    pub lattice: Lattice,
    pub units: Units,
    pub inlets: Vec<Layer>,
    pub outlets: Vec<Layer>,
    /// Per inlet: what drives it (as set up).
    pub sources: Vec<Source>,
    /// Per inlet: its current speed, m/s (a fan's follows its curve while the flow develops).
    pub speeds: Vec<f64>,
    /// Per inlet: the lattice density its speed eases around (its mean, tracked as the flow
    /// develops), so pressure waves leave through it instead of ringing.
    pub densities: Vec<f64>,
    /// Per inlet: its share of its speed now (the soft start ramps it from 0 to 1).
    pub scales: Vec<f64>,
    pub warnings: Vec<Warning>,
}

fn union(a: Aabb3, b: Aabb3) -> Aabb3 {
    Aabb3 { min: a.min.min(b.min), max: a.max.max(b.max) }
}

/// The lattice direction (one of the six face directions) closest to `n`.
fn axis(n: Vec3) -> [i32; 3] {
    let a = [n.x, n.y, n.z];
    let (mut best, mut at) = (0.0f64, 0usize);
    for (i, v) in a.iter().enumerate() {
        if v.abs() > best.abs() {
            best = *v;
            at = i;
        }
    }
    let mut d = [0; 3];
    if let Some(slot) = d.get_mut(at) {
        *slot = if best < 0.0 { -1 } else { 1 };
    }
    d
}

/// The grid cells whose centres lie within `from..to` (mm) along the patch's normal and inside
/// one of its triangles seen along the normal.
fn cells_on(grid: &Grid, patch: &Patch, n: Vec3, from: f64, to: f64) -> Vec<usize> {
    let s = grid.spacing;
    let mut out = Vec::new();
    let [nx, ny, nz] = grid.dims;
    let range = |lo: f64, hi: f64, o: f64, n: usize| -> std::ops::Range<usize> {
        let a = (((lo - o) / s) - 1.0).floor().max(0.0) as usize;
        let b = ((((hi - o) / s) + 1.0).ceil().max(0.0) as usize).min(n);
        a..b
    };
    for t in &patch.triangles {
        let [a, b, c] = *t;
        let lo = a.min(b).min(c) - n.map_abs() * (to.abs().max(from.abs()) + s);
        let hi = a.max(b).max(c) + n.map_abs() * (to.abs().max(from.abs()) + s);
        let (e1, e2) = (b - a, c - a);
        let normal = e1.cross(e2);
        let nn = normal.len2();
        // Skip slivers and triangles standing edge-on to the patch's normal.
        if !nn.is_finite() || nn < 1e-18 || normal.dot(n).abs() < 1e-9 * nn.sqrt() {
            continue;
        }
        for k in range(lo.z, hi.z, grid.origin.z, nz) {
            for j in range(lo.y, hi.y, grid.origin.y, ny) {
                for i in range(lo.x, hi.x, grid.origin.x, nx) {
                    let p = grid.center(i, j, k);
                    let d = (p - a).dot(n);
                    if d < from || d >= to {
                        continue;
                    }
                    // Inside the triangle seen along n (with a hair of slack for shared edges).
                    let q = p - n * d;
                    let w1 = (q - a).cross(e2).dot(normal) / nn;
                    let w2 = e1.cross(q - a).dot(normal) / nn;
                    let eps = 1e-9;
                    if w1 >= -eps
                        && w2 >= -eps
                        && w1 + w2 <= 1.0 + eps
                        && let Some(idx) = grid.index(i, j, k)
                    {
                        out.push(idx);
                    }
                }
            }
        }
    }
    out.sort_unstable();
    out.dedup();
    out
}

trait MapAbs {
    fn map_abs(self) -> Vec3;
}

impl MapAbs for Vec3 {
    fn map_abs(self) -> Vec3 {
        Vec3::new(self.x.abs(), self.y.abs(), self.z.abs())
    }
}

/// The speed a source starts at and the fastest it may reach, m/s, for an inlet of `area` m².
fn source_speeds(source: &Source, area: f64) -> Result<(f64, f64)> {
    match source {
        Source::Speed(v) if v.is_finite() && *v > 0.0 => Ok((*v, *v)),
        Source::Fan(curve) if area > 0.0 => {
            let free = curve.free_flow() / area;
            // Start at half the free-air speed; the curve settles it.
            Ok((0.5 * free, free))
        }
        _ => Err(SimError::BadOpening),
    }
}

/// Points on the bodies where the wall is thinner than a cell: no solid cell lies under a
/// triangle's centre within a cell and a half inward. At most `max` points.
/// About how thick the bodies' thinner walls are (mm): from up to 400 of their triangles, spread
/// over the mesh, a ray straight into the material to where it comes out again; the thickness
/// that a tenth of that area (by triangle area) is thinner than. None for no closed walls.
pub fn wall_thickness(bodies: &[&Mesh]) -> Option<f64> {
    let tris: Vec<[Vec3; 3]> = bodies.iter().flat_map(|m| m.triangles.iter().filter_map(|t| m.tri(t))).collect();
    let stride = tris.len().div_ceil(400).max(1);
    let mut found: Vec<(f64, f64)> = Vec::new();
    for [a, b, c] in tris.iter().step_by(stride) {
        let normal = (*b - *a).cross(*c - *a);
        let area = normal.len() * 0.5;
        let Some(n) = normal.normalized() else { continue };
        let from = (*a + *b + *c) / 3.0;
        let inward = n * -1.0;
        let out = tris
            .iter()
            .filter_map(|[p, q, r]| {
                let e1 = *q - *p;
                let e2 = *r - *p;
                let h = inward.cross(e2);
                let det = e1.dot(h);
                if det.abs() < 1e-18 {
                    return None;
                }
                let to = from - *p;
                let u = to.dot(h) / det;
                let qv = to.cross(e1);
                let v = inward.dot(qv) / det;
                let t = e2.dot(qv) / det;
                ((0.0..=1.0).contains(&u) && v >= 0.0 && u + v <= 1.0 && t > 1e-6).then_some(t)
            })
            .fold(f64::INFINITY, f64::min);
        if out.is_finite() && area > 0.0 {
            found.push((out, area));
        }
    }
    found.sort_by(|x, y| x.0.total_cmp(&y.0));
    let total: f64 = found.iter().map(|f| f.1).sum();
    let mut run = 0.0;
    found
        .iter()
        .find(|(_, area)| {
            run += area;
            run >= 0.1 * total
        })
        .map(|f| f.0)
}

fn thin_walls(grid: &Grid, bodies: &[&Mesh], max: usize) -> Vec<Vec3> {
    let mut out = Vec::new();
    let s = grid.spacing;
    for m in bodies {
        for t in &m.triangles {
            let Some([a, b, c]) = m.tri(t) else { continue };
            let Some(n) = (b - a).cross(c - a).normalized() else { continue };
            // Only triangles at least a cell across say something about the cells.
            if (b - a).cross(c - a).len() * 0.5 < 0.25 * s * s {
                continue;
            }
            let centre = (a + b + c) / 3.0;
            // Probe into the wall up to a cell and a half: a wall at least a cell thick puts a
            // solid cell under one of the probes, staircase and all.
            let solid_under = (1..=6).any(|step| {
                let at = centre - n * (0.25 * f64::from(step) * s) - grid.origin;
                let (i, j, k) = ((at.x / s).floor(), (at.y / s).floor(), (at.z / s).floor());
                i < 0.0 || j < 0.0 || k < 0.0 || grid.get(i as usize, j as usize, k as usize) != Some(Cell::Fluid)
            });
            if !solid_under {
                out.push(centre);
                if out.len() >= max {
                    return out;
                }
            }
        }
    }
    out
}

/// Van Driest's damping of the Smagorinsky length scale, 1 − exp(−y⁺/A), per cell (65535 = 1):
/// y is the distance to the nearest wall (counted in cells through the air, up to 16), and the
/// friction velocity comes from the Blasius friction factor for air at `speed` (m/s) through a
/// duct of diameter `diameter` (m). Rough walls (equivalent sand-grain roughness `roughness`, m)
/// damp less: Cebeci and Chang's shift of y⁺ by Δy⁺ = 0.9 (√k⁺ − k⁺ e^(−k⁺/6)) (AIAA J. 16(7),
/// 1978), which applies from k⁺ of about 4.5 (below it the wall is hydraulically smooth) and grows
/// with k⁺.
fn wall_damping(grid: &Grid, speed: f64, diameter: f64, viscosity: f64, roughness: f64) -> Vec<u16> {
    const FAR: u8 = 16;
    let n = grid.len();
    let [nx, ny, nz] = grid.dims;
    let mut dist = vec![FAR; n];
    let mut queue = std::collections::VecDeque::new();
    for (idx, cell) in grid.cells.iter().enumerate() {
        if *cell == Cell::Solid {
            if let Some(d) = dist.get_mut(idx) {
                *d = 0;
            }
            queue.push_back(idx);
        }
    }
    while let Some(at) = queue.pop_front() {
        let here = dist.get(at).copied().unwrap_or(FAR);
        if here + 1 >= FAR {
            continue;
        }
        let [i, j, k] = grid.coords(at);
        let near = [
            (i > 0).then(|| at - 1),
            (i + 1 < nx).then(|| at + 1),
            (j > 0).then(|| at - nx),
            (j + 1 < ny).then(|| at + nx),
            (k > 0).then(|| at - nx * ny),
            (k + 1 < nz).then(|| at + nx * ny),
        ];
        for m in near.into_iter().flatten() {
            if let Some(d) = dist.get_mut(m)
                && *d > here + 1
            {
                *d = here + 1;
                queue.push_back(m);
            }
        }
    }
    let re = (speed * diameter / viscosity).max(2300.0);
    let u_tau = speed * (0.316 * re.powf(-0.25) / 8.0).sqrt();
    let per_cell = grid.spacing * 1e-3 * u_tau / viscosity;
    let shift = rough_shift(roughness * u_tau / viscosity);
    dist.iter()
        .map(|d| {
            if *d == 0 {
                return 0;
            }
            let y_plus = (f64::from(*d) - 0.5) * per_cell + shift;
            // A floor keeps some eddy viscosity in the first cells, where the lattice is least
            // stable at a relaxation time this close to ½.
            ((1.0 - (-y_plus / VAN_DRIEST_A).exp()).clamp(DAMPING_FLOOR, 1.0) * 65535.0).round() as u16
        })
        .collect()
}

/// Cebeci and Chang's roughness shift Δy⁺ for a roughness of `k_plus` wall units.
fn rough_shift(k_plus: f64) -> f64 {
    // Below this the roughness sits inside the viscous sublayer.
    const SMOOTH: f64 = 4.535;
    if !(k_plus.is_finite() && k_plus >= SMOOTH) {
        return 0.0;
    }
    (0.9 * (k_plus.sqrt() - k_plus * (-k_plus / 6.0).exp())).max(0.0)
}

/// The lattice for `setup` around `bodies` (closed triangle meshes, mm).
pub fn prepare(bodies: &[&Mesh], setup: &FlowSetup) -> Result<Prepared> {
    prepare_paced(bodies, setup, 1.0)
}

/// [`prepare`] with a time step `pace` times shorter than usual (1 or more): for a flow whose jets
/// run faster than the openings let `prepare` foresee (exits that aren't named as outlets).
pub fn prepare_paced(bodies: &[&Mesh], setup: &FlowSetup, pace: f64) -> Result<Prepared> {
    if !(pace.is_finite() && (1.0..=64.0).contains(&pace)) {
        return Err(SimError::InvalidSpacing);
    }
    if bodies.is_empty() {
        return Err(SimError::EmptyMesh);
    }
    if setup.inlets.is_empty() {
        return Err(SimError::NoInlet);
    }
    if !(setup.pad.is_finite()
        && setup.pad >= 0.0
        && setup.air.density.is_finite()
        && setup.air.density > 0.0
        && setup.air.viscosity.is_finite()
        && setup.air.viscosity > 0.0
        && setup.roughness.is_finite()
        && setup.roughness >= 0.0)
    {
        return Err(SimError::InvalidSpacing);
    }
    let mut bounds = bodies.iter().map(|m| m.bounds()).reduce(union).ok_or(SimError::EmptyMesh)?;
    let pad = Vec3::new(setup.pad, setup.pad, setup.pad);
    bounds = Aabb3 { min: bounds.min - pad, max: bounds.max + pad };
    // Past each outlet, room for its jet: one outlet width more than the padding. Closer, the
    // sponge at the box's edge slows the jet and pushes pressure back up the duct (a 50 mm pipe
    // read 4 Pa at its exit with 20 mm of air past it, 1.4 Pa with 40 and with 80).
    for outlet in &setup.outlets {
        let (area, n) = (outlet.patch.area(), outlet.patch.normal.normalized());
        let Some(n) = n.filter(|_| area > 0.0) else { continue };
        let centre = outlet
            .patch
            .triangles
            .iter()
            .map(|[a, b, c]| (*a + *b + *c) * ((*b - *a).cross(*c - *a).len() * 0.5 / 3.0))
            .fold(Vec3::new(0.0, 0.0, 0.0), |s, p| s + p)
            * (1.0 / area);
        let reach = centre + n * (setup.pad + (4.0 * area / std::f64::consts::PI).sqrt());
        bounds = union(bounds, Aabb3 { min: reach, max: reach });
    }
    // One more cell all round: the box's faces are the ambient openings and its edges walls.
    let mut grid = Grid::around(bounds, setup.cell, 1, setup.max_cells)?;
    for m in bodies {
        crate::voxelize(m, &mut grid)?;
    }
    let mut warnings = Vec::new();
    let s = grid.spacing;
    let units_dx = s / 1000.0;

    // Inlet layers (first cell on the air side), their caps, and the outlets' layers.
    let mut inlet_cells = Vec::new();
    for inlet in &setup.inlets {
        let n = inlet.patch.normal.normalized().ok_or(SimError::BadOpening)?;
        let cells = cells_on(&grid, &inlet.patch, n, 0.0, s);
        if inlet.cap {
            for c in cells_on(&grid, &inlet.patch, n, -s, 0.0) {
                if let Some(cell) = grid.cells.get_mut(c) {
                    *cell = Cell::Solid;
                }
            }
        }
        inlet_cells.push((n, cells));
    }
    let mut outlet_layers = Vec::new();
    for outlet in &setup.outlets {
        let n = outlet.patch.normal.normalized().ok_or(SimError::BadOpening)?;
        // The last cell inside the duct, just before the patch.
        let cells: Vec<usize> = cells_on(&grid, &outlet.patch, n, -s, 0.0).into_iter().filter(|c| grid.cells.get(*c) == Some(&Cell::Fluid)).collect();
        if cells.is_empty() {
            return Err(SimError::OpeningTooSmall(outlet.name.clone()));
        }
        outlet_layers.push(Layer { name: outlet.name.clone(), probe: cells.clone(), cells, normal: n, openings: 0..0 });
    }

    let [nx, ny, nz] = grid.dims;
    let on_face =
        |i: usize, j: usize, k: usize| usize::from(i == 0 || i + 1 == nx) + usize::from(j == 0 || j + 1 == ny) + usize::from(k == 0 || k + 1 == nz);
    // The box's edges and corners are walls, so each face opening has an inside cell.
    for k in 0..nz {
        for j in 0..ny {
            for i in 0..nx {
                if on_face(i, j, k) >= 2
                    && let Some(cell) = grid.index(i, j, k).and_then(|x| grid.cells.get_mut(x))
                {
                    *cell = Cell::Solid;
                }
            }
        }
    }

    // Leak check: with the outlets closed, the air from the first inlet must not reach the box.
    let first = inlet_cells.first().and_then(|(_, c)| c.iter().find(|c| grid.cells.get(**c) == Some(&Cell::Fluid)).copied());
    let Some(seed) = first else { return Err(SimError::OpeningTooSmall(setup.inlets.first().map(|i| i.name.clone()).unwrap_or_default())) };
    let closed: std::collections::HashSet<usize> = outlet_layers.iter().flat_map(|l| l.cells.iter().copied()).collect();
    if !closed.is_empty() {
        let region = crate::fluid_region(&grid, grid.coords(seed), |i, j, k| grid.index(i, j, k).is_some_and(|c| closed.contains(&c)))?;
        if region.touches_edge {
            warnings.push(Warning::Leak);
        }
    }
    // Air that the inlets cannot reach plays no part: make it solid.
    let reach = crate::fluid_region(&grid, grid.coords(seed), |_, _, _| false)?;
    for (cell, reached) in grid.cells.iter_mut().zip(&reach.cells) {
        if !reached {
            *cell = Cell::Solid;
        }
    }
    let thin = thin_walls(&grid, bodies, 64);
    if !thin.is_empty() {
        warnings.push(Warning::ThinWalls(thin));
    }

    // Units from the fastest inlet.
    let mut speeds = Vec::new();
    let mut fastest = 0.0f64;
    for (inlet, (_, cells)) in setup.inlets.iter().zip(&inlet_cells) {
        let area = cells.len() as f64 * units_dx * units_dx;
        let (start, max) = source_speeds(&inlet.source, area)?;
        speeds.push(start);
        fastest = fastest.max(max);
    }
    // Air leaving through outlets smaller than the inlets goes faster than it came in: by
    // continuity at least the inlets' flow over the outlets' area, and a small outlet of several
    // takes more than its share. Time the lattice on that too, or the jets outrun it.
    let inflow: f64 = setup
        .inlets
        .iter()
        .zip(&inlet_cells)
        .zip(&speeds)
        .map(|((inlet, (_, cells)), start)| {
            let area = cells.len() as f64 * units_dx * units_dx;
            match &inlet.source {
                Source::Fan(curve) => curve.free_flow(),
                Source::Speed(_) => start * area,
            }
        })
        .sum();
    let outlet_area: f64 = outlet_layers.iter().map(|l| l.cells.len() as f64 * units_dx * units_dx).sum();
    if outlet_area > 0.0 {
        fastest = fastest.max(1.5 * inflow / outlet_area);
    }
    let dt = LATTICE_SPEED * units_dx / (fastest * pace);
    let units = Units { dx: units_dx, dt, density: setup.air.density };
    let tau = 0.5 + 3.0 * setup.air.viscosity * dt / (units_dx * units_dx);
    if tau < 0.51 {
        warnings.push(Warning::LowViscosity { tau });
    }
    let mut lattice = Lattice::new(&grid, tau, [0.0; 3], [false; 3])?
        .with_smagorinsky(setup.turbulence)?
        .regularized()
        .with_bulk(tau.max(BULK_TAU))?
        .with_walls(crate::wall_links(bodies, &grid)?)?;
    if setup.turbulence > 0.0 {
        let largest = inlet_cells.iter().map(|(_, c)| c.len()).max().unwrap_or(0) as f64 * units_dx * units_dx;
        lattice = lattice.with_damping(wall_damping(
            &grid,
            fastest,
            (4.0 * largest / std::f64::consts::PI).sqrt(),
            setup.air.viscosity,
            setup.roughness * 1e-3,
        ))?;
    }
    // A sponge over the outer part of the padding (never reaching the bodies) damps what would
    // otherwise reflect off the box's faces.
    let pad_cells = (setup.pad / s).floor() as usize;
    if pad_cells >= 4 {
        lattice = lattice.with_sponge((pad_cells / 2).min(12), tau.max(SPONGE_TAU))?;
    }

    // Inlet openings.
    let mut taken: std::collections::HashSet<usize> = std::collections::HashSet::new();
    for (_, cells) in &inlet_cells {
        taken.extend(cells.iter().copied());
    }
    let mut inlets = Vec::new();
    for ((inlet, (n, cells)), speed) in setup.inlets.iter().zip(&inlet_cells).zip(&speeds) {
        let inward = axis(*n);
        let (mut kept, mut probe) = (Vec::new(), Vec::new());
        for c in cells {
            let [i, j, k] = grid.coords(*c);
            let inside = [i as i64 + i64::from(inward[0]), j as i64 + i64::from(inward[1]), k as i64 + i64::from(inward[2])];
            let inside = (inside.iter().all(|v| *v >= 0)).then(|| grid.index(inside[0] as usize, inside[1] as usize, inside[2] as usize)).flatten();
            let ok = grid.cells.get(*c) == Some(&Cell::Fluid)
                && on_face(i, j, k) == 0
                && inside.is_some_and(|x| grid.cells.get(x) == Some(&Cell::Fluid) && !taken.contains(&x));
            if ok && let Some(inside) = inside {
                kept.push(*c);
                probe.push(inside);
            }
        }
        // A few rim cells against the wall are expected; more says the patch is off.
        if (cells.len() - kept.len()) * 20 > cells.len() {
            warnings.push(Warning::DroppedCells { opening: inlet.name.clone(), cells: cells.len() - kept.len() });
        }
        if kept.is_empty() {
            return Err(SimError::OpeningTooSmall(inlet.name.clone()));
        }
        let start = lattice.openings().len();
        let u = units.lattice_speed(*speed);
        let at: Vec<[usize; 3]> = kept.iter().map(|c| grid.coords(*c)).collect();
        lattice.add_opening(&at, inward, Opening::Inflow { velocity: [n.x * u, n.y * u, n.z * u], density: 1.0 })?;
        inlets.push(Layer { name: inlet.name.clone(), cells: kept, probe, normal: *n, openings: start..lattice.openings().len() });
    }
    // The box's faces: ambient pressure.
    for (inward, face) in [([1, 0, 0], 0usize), ([-1, 0, 0], 1), ([0, 1, 0], 2), ([0, -1, 0], 3), ([0, 0, 1], 4), ([0, 0, -1], 5)] {
        let mut at = Vec::new();
        for k in 0..nz {
            for j in 0..ny {
                for i in 0..nx {
                    let here = match face {
                        0 => i == 0,
                        1 => i + 1 == nx,
                        2 => j == 0,
                        3 => j + 1 == ny,
                        4 => k == 0,
                        _ => k + 1 == nz,
                    };
                    if !here || on_face(i, j, k) != 1 {
                        continue;
                    }
                    let inside = [i as i64 + inward[0], j as i64 + inward[1], k as i64 + inward[2]].map(|v| v.max(0) as usize);
                    if grid.get(i, j, k) == Some(Cell::Fluid) && grid.get(inside[0], inside[1], inside[2]) == Some(Cell::Fluid) {
                        at.push([i, j, k]);
                    }
                }
            }
        }
        if !at.is_empty() {
            lattice.add_opening(&at, [inward[0] as i32, inward[1] as i32, inward[2] as i32], Opening::Density(1.0))?;
        }
    }
    let sources = setup.inlets.iter().map(|i| i.source.clone()).collect();
    let densities = vec![1.0; speeds.len()];
    let scales = vec![1.0; speeds.len()];
    Ok(Prepared { grid, lattice, units, inlets, outlets: outlet_layers, sources, speeds, densities, scales, warnings })
}

/// The flow through one opening's layer.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Through {
    /// Volume flow along the normal, m³/s.
    pub flow: f64,
    /// Mean gauge pressure, Pa.
    pub pressure: f64,
    /// Mean speed along the normal, m/s.
    pub speed: f64,
    /// Fastest cell, m/s.
    pub max_speed: f64,
    /// m².
    pub area: f64,
}

impl Prepared {
    /// The flow through `layer` in `lattice` (this study's).
    pub fn through(&self, lattice: &Lattice, layer: &Layer) -> Through {
        let (mut flow, mut pressure, mut max_speed) = (0.0, 0.0, 0.0f64);
        let mut count = 0usize;
        let n = layer.normal;
        for c in &layer.probe {
            let [i, j, k] = self.grid.coords(*c);
            let Some((rho, u)) = lattice.moments(i, j, k) else { continue };
            let u = Vec3::new(u[0], u[1], u[2]);
            flow += rho * u.dot(n);
            pressure += self.units.pressure(rho);
            max_speed = max_speed.max(u.len());
            count += 1;
        }
        let area = self.units.area(count);
        let flow = self.units.speed(flow) * self.units.dx * self.units.dx;
        let mean = |v: f64| if count == 0 { 0.0 } else { v / count as f64 };
        Through { flow, pressure: mean(pressure), speed: if area > 0.0 { flow / area } else { 0.0 }, max_speed: self.units.speed(max_speed), area }
    }

    /// Start from an earlier result instead of still air: each air cell takes the flow found at its
    /// centre in `from` (another grid's, in physical units), as its equilibrium. Cells `from` has
    /// no air at keep still air. Returns how many cells took a value.
    pub fn warm_from(&mut self, from: &crate::Fields) -> usize {
        let mut n = 0;
        for idx in 0..self.grid.len() {
            if self.lattice.solid().get(idx).copied().unwrap_or(true) {
                continue;
            }
            let [i, j, k] = self.grid.coords(idx);
            let Some((u, p)) = from.sample(self.grid.center(i, j, k)) else { continue };
            let lu = [self.units.lattice_speed(u.x), self.units.lattice_speed(u.y), self.units.lattice_speed(u.z)];
            self.lattice.set_equilibrium(idx, self.units.lattice_density(p), lu);
            n += 1;
        }
        n
    }

    /// Write inlet `i`'s opening from its speed, share and reference density.
    fn write_inlet(&mut self, i: usize) -> Result<()> {
        let (Some(layer), Some(speed), Some(scale), Some(density)) =
            (self.inlets.get(i), self.speeds.get(i), self.scales.get(i), self.densities.get(i))
        else {
            return Ok(());
        };
        let u = self.units.lattice_speed(speed * scale);
        let n = layer.normal;
        self.lattice.set_openings(layer.openings.clone(), Opening::Inflow { velocity: [n.x * u, n.y * u, n.z * u], density: *density })
    }

    /// Run every inlet at `scale` (0..1) of its speed: the soft start ramps the inlets up so
    /// the air is not hit by a pressure wave.
    pub fn scale_inlets(&mut self, scale: f64) -> Result<()> {
        for i in 0..self.inlets.len() {
            if let Some(s) = self.scales.get_mut(i) {
                *s = scale.clamp(0.0, 1.0);
            }
            self.write_inlet(i)?;
        }
        Ok(())
    }

    /// Move each inlet's reference density toward the mean density just inside it (by `relax`,
    /// 0..1).
    pub fn track_inlets(&mut self, relax: f64) -> Result<()> {
        for i in 0..self.inlets.len() {
            let Some(layer) = self.inlets.get(i) else { continue };
            let seen = self.units.lattice_density(self.through(&self.lattice, layer).pressure);
            if let Some(d) = self.densities.get_mut(i)
                && seen.is_finite()
                && seen > 0.0
            {
                *d += relax.clamp(0.0, 1.0) * (seen - *d);
            }
            self.write_inlet(i)?;
        }
        Ok(())
    }

    /// Move each fan inlet's speed toward its curve's flow at the pressure the inlet sees now in
    /// [`Prepared::lattice`] (by `relax`, 0..1). Returns how far the largest one moved, as a
    /// fraction of its speed.
    pub fn follow_fans(&mut self, relax: f64) -> Result<f64> {
        let mut moved = 0.0f64;
        for i in 0..self.inlets.len() {
            let (Some(Source::Fan(curve)), Some(layer)) = (self.sources.get(i), self.inlets.get(i)) else { continue };
            let seen = self.through(&self.lattice, layer);
            if seen.area <= 0.0 {
                continue;
            }
            let ceiling = self.units.speed(2.0 * LATTICE_SPEED);
            let Some(speed) = self.speeds.get_mut(i) else { continue };
            // The fan's curve is about the air that gets into the duct, the flow one cell in, which
            // can fall short of what the opening pushes (a duct whose fan face is partly in a wall
            // took 24% less): scale the opening's speed until that flow is on the curve.
            let want = curve.flow_at(seen.pressure);
            // Early on the flow lags the opening, so one step moves the speed by half again at most.
            let target = if seen.flow > 1e-12 && *speed > 0.0 { *speed * (want / seen.flow).clamp(0.5, 1.5) } else { want / seen.area };
            // Never past what the lattice can carry.
            let next = (*speed + relax.clamp(0.0, 1.0) * (target - *speed)).clamp(0.0, ceiling);
            moved = moved.max((next - *speed).abs() / speed.abs().max(1e-12));
            *speed = next;
            self.write_inlet(i)?;
        }
        Ok(moved)
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    #[test]
    fn fan_curves_are_checked_and_read_both_ways() {
        let fan = FanCurve::linear(0.01, 300.0).unwrap();
        assert_eq!(fan.free_flow(), 0.01);
        assert!((fan.pressure_at(0.005) - 150.0).abs() < 1e-9);
        assert!((fan.flow_at(150.0) - 0.005).abs() < 1e-12);
        assert_eq!(fan.flow_at(-5.0), 0.01, "free air past zero pressure");
        assert_eq!(fan.flow_at(400.0), 0.0, "shut off past its pressure");
        for (q, p) in [(0.0, 1.0), (0.2, 0.98), (0.6, 0.8)] {
            let radial = FanCurve::radial(1.0, 1.0).unwrap();
            assert!((radial.pressure_at(q) - p).abs() < 1e-12);
        }
        // The axial curve's flat stretch reads as its largest flow.
        let axial = FanCurve::axial(1.0, 100.0).unwrap();
        let q = axial.flow_at(60.0);
        assert!((axial.pressure_at(q) - 60.0).abs() < 1e-9, "{q}");
        for bad in [
            vec![(0.0, 1.0)],
            vec![(0.0, 1.0), (0.0, 0.0)],
            vec![(0.0, 1.0), (1.0, 2.0)],
            vec![(0.0, f64::NAN), (1.0, 0.0)],
            vec![(0.0, 0.0), (1.0, 0.0)],
        ] {
            assert!(FanCurve::new(bad.clone()).is_err(), "{bad:?}");
        }
        assert!(FanCurve::axial(0.0, 10.0).is_err() && FanCurve::radial(1.0, f64::INFINITY).is_err());
    }

    /// A square duct along x (bore `a`, walls `w`, length `len`, mm), optionally with a square
    /// hole of side `hole` through its top wall halfway along.
    pub(crate) fn duct(a: f64, w: f64, len: f64, hole: f64) -> Mesh {
        let add = crate::voxelize::tests::add_box;
        let mut m = Mesh::default();
        let o = a + 2.0 * w;
        add(&mut m, [0.0, 0.0, 0.0], [len, o, w]);
        if hole > 0.0 {
            let (h0, h1) = (0.5 * (len - hole), 0.5 * (len + hole));
            add(&mut m, [0.0, 0.0, o - w], [h0, o, o]);
            add(&mut m, [h1, 0.0, o - w], [len, o, o]);
            add(&mut m, [h0, 0.0, o - w], [h1, w + 0.5 * (a - hole), o]);
            add(&mut m, [h0, w + 0.5 * (a + hole), o - w], [h1, o, o]);
        } else {
            add(&mut m, [0.0, 0.0, o - w], [len, o, o]);
        }
        add(&mut m, [0.0, 0.0, w], [len, w, o - w]);
        add(&mut m, [0.0, o - w, w], [len, o, o - w]);
        m
    }

    /// The square across the bore at `x`, facing +x.
    fn across(a: f64, w: f64, x: f64) -> Patch {
        let (lo, hi) = (w, w + a);
        let p = |y: f64, z: f64| Vec3::new(x, y, z);
        Patch { triangles: vec![[p(lo, lo), p(hi, lo), p(hi, hi)], [p(lo, lo), p(hi, hi), p(lo, hi)]], normal: Vec3::X }
    }

    pub(crate) fn setup(a: f64, w: f64, len: f64, source: Source) -> FlowSetup {
        FlowSetup {
            cell: 1.0,
            pad: 4.0,
            air: AIR,
            inlets: vec![Inlet { name: "in".into(), patch: across(a, w, 0.0), source, cap: true }],
            outlets: vec![Outlet { name: "out".into(), patch: across(a, w, len) }],
            turbulence: 0.0,
            roughness: 0.0,
            max_cells: 200_000,
        }
    }

    /// Slow air blown into a square duct's open end leaves through the other: the flow is what
    /// the inlet speed times its area gives, it is kept along the duct, and the pressure the
    /// inlet needs is close to the laminar duct's (dp = 28.45 μ Q L / a⁴ for a square bore, plus
    /// a little for the developing flow and the exit). The duct is sealed: no leak.
    #[test]
    fn a_ducts_wall_thickness_is_measured() {
        for w in [0.6, 2.0] {
            let t = wall_thickness(&[&duct(8.0, w, 24.0, 0.0)]).unwrap();
            assert!((t - w).abs() < 1e-6, "{t} for {w}");
        }
        assert_eq!(wall_thickness(&[&Mesh::default()]), None);
    }

    #[test]
    fn a_duct_study_carries_its_inlet_flow_to_the_outlet() {
        let (a, w, len) = (8.0, 2.0, 24.0);
        let mesh = duct(a, w, len, 0.0);
        let speed = 0.05;
        let mut p = prepare(&[&mesh], &setup(a, w, len, Source::Speed(speed))).unwrap();
        assert!(!p.warnings.iter().any(|w| matches!(w, Warning::Leak | Warning::ThinWalls(_))), "{:?}", p.warnings);
        assert_eq!(p.inlets[0].cells.len(), 64);
        // As a run does: the inlet's reference density follows its mean between stretches.
        for _ in 0..20 {
            for _ in 0..200 {
                p.lattice.step();
            }
            p.track_inlets(0.3).unwrap();
        }
        let (inlet, outlet) = (p.through(&p.lattice, &p.inlets[0]), p.through(&p.lattice, &p.outlets[0]));
        let nominal = speed * (a * 1e-3) * (a * 1e-3);
        assert!((inlet.flow - nominal).abs() / nominal < 0.08, "inlet {:e} vs {nominal:e}", inlet.flow);
        assert!((outlet.flow - inlet.flow).abs() / inlet.flow < 0.03, "outlet {:e} vs inlet {:e}", outlet.flow, inlet.flow);
        let mu = AIR.density * AIR.viscosity;
        let laminar = 28.45 * mu * inlet.flow * (len * 1e-3) / (a * 1e-3).powi(4);
        assert!(inlet.pressure > 0.8 * laminar && inlet.pressure < 2.5 * laminar, "inlet {} Pa vs laminar {laminar} Pa", inlet.pressure);
    }

    /// A hole in the duct's wall: with the outlet closed the air still reaches the box, which
    /// the setup reports.
    #[test]
    fn a_hole_in_the_wall_is_reported_as_a_leak() {
        let (a, w, len) = (8.0, 2.0, 24.0);
        let p = prepare(&[&duct(a, w, len, 4.0)], &setup(a, w, len, Source::Speed(0.05))).unwrap();
        assert!(p.warnings.contains(&Warning::Leak), "{:?}", p.warnings);
        let sealed = prepare(&[&duct(a, w, len, 0.0)], &setup(a, w, len, Source::Speed(0.05))).unwrap();
        assert!(!sealed.warnings.contains(&Warning::Leak));
    }

    /// A fan on a linear curve settles where its curve meets the duct: the flow it delivers is
    /// the curve's flow at the pressure the inlet sees.
    #[test]
    fn a_fan_inlet_settles_on_its_curve() {
        let (a, w, len) = (8.0, 2.0, 24.0);
        // Free air 4e-6 m³/s (0.06 m/s across the bore), shut off at 0.05 Pa: the duct's
        // resistance puts the operating point well inside the curve.
        let fan = FanCurve::linear(4e-6, 0.05).unwrap();
        let mut p = prepare(&[&duct(a, w, len, 0.0)], &setup(a, w, len, Source::Fan(fan.clone()))).unwrap();
        let mut moved = 1.0;
        for _ in 0..20 {
            for _ in 0..300 {
                p.lattice.step();
            }
            p.track_inlets(0.3).unwrap();
            moved = p.follow_fans(0.3).unwrap();
        }
        let seen = p.through(&p.lattice, &p.inlets[0]);
        let on_curve = fan.flow_at(seen.pressure);
        assert!(moved < 0.02, "still moving {moved}");
        assert!((seen.flow - on_curve).abs() / on_curve < 0.05, "{:e} m³/s at {} Pa; the curve gives {on_curve:e}", seen.flow, seen.pressure);
        assert!(seen.flow < 0.95 * fan.free_flow() && seen.pressure > 0.0, "{seen:?}");
    }

    /// Cebeci and Chang's roughness shift: none on smooth walls or below k⁺ ≈ 4.5, then it grows
    /// toward 0.9 √k⁺ as the roughness takes over from the viscous layer.
    #[test]
    fn rough_walls_shift_the_damping() {
        assert_eq!(rough_shift(0.0), 0.0);
        assert_eq!(rough_shift(f64::NAN), 0.0);
        assert_eq!(rough_shift(4.0), 0.0);
        assert!(rough_shift(5.0) > 0.0, "{}", rough_shift(5.0));
        assert!((rough_shift(60.0) - 0.9 * 60f64.sqrt()).abs() < 0.01, "{}", rough_shift(60.0));
        assert!(rough_shift(20.0) > rough_shift(10.0) && rough_shift(10.0) > rough_shift(5.0));
    }

    #[test]
    fn units_round_trip() {
        let u = Units { dx: 1e-3, dt: 1e-5, density: AIR.density };
        assert!((u.speed(u.lattice_speed(5.0)) - 5.0).abs() < 1e-12);
        assert!((u.pressure(u.lattice_density(12.5)) - 12.5).abs() < 1e-9);
        assert_eq!(u.pressure(1.0), 0.0);
    }
}
