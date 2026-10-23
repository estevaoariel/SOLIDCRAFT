//! The Airflow solver on the GPU: the lattice Boltzmann step of [`solidcraft_sim::Lattice`] in
//! a compute shader (`flow.wgsl`), in f32. The CPU lattice is the reference it is checked
//! against: [`FlowGpu::upload`] takes a lattice's state (grid, relaxation, force, openings,
//! populations), [`GpuLattice::run`] steps it on the GPU, and [`GpuLattice::download`] puts the
//! populations back into a lattice, where its moments read as usual.
//!
//! Each step is three dispatches: collide and stream every fluid cell into the other population
//! buffer, interpolate the wall links to where the walls are, then rewrite the opening cells. Populations are stored as offsets from rest, so a
//! slow flow keeps its digits in f32.

use solidcraft_sim::lbm::W;
use solidcraft_sim::{Lattice, Opening, WallLink};
use wgpu::util::DeviceExt;

use crate::{GpuError, Result, popped};

/// The flow compute shader (WGSL).
pub const SHADER: &str = include_str!("flow.wgsl");

const Q: usize = 19;
const WORKGROUP: u32 = 64;
/// Bytes of one opening as the shader reads it (`Opening` in flow.wgsl).
const OPENING_BYTES: usize = 32;
/// Bytes of one wall link as the shader reads it (after the cells in `solid`, flow.wgsl).
const WALL_BYTES: usize = 16;
/// Steps recorded into one submission: keeps each submission short, so a large grid's batch
/// stays well under the driver's watchdog.
#[cfg(not(target_arch = "wasm32"))]
const STEPS_PER_SUBMIT: u32 = 32;

/// The bytes one population buffer needs for `cells` cells, if that fits a u32 index.
pub fn population_bytes(cells: usize) -> Option<u64> {
    let words = cells.checked_mul(Q)?;
    u32::try_from(words).ok()?;
    u64::try_from(words).ok()?.checked_mul(4)
}

/// The openings as the shader reads them (at least one slot, so the buffer is never empty).
/// An opening whose inside cell is another opening is refused: the GPU rewrites all openings
/// at once, so one may not read another.
fn pack_openings(lattice: &Lattice) -> Result<Vec<u8>> {
    let openings = lattice.openings();
    let opening_cells: std::collections::HashSet<usize> = openings.iter().map(|o| o.0).collect();
    if openings.iter().any(|o| opening_cells.contains(&o.1)) {
        return Err(GpuError::Unsupported("an opening whose inside cell is another opening".into()));
    }
    let mut packed = Vec::with_capacity(openings.len().max(1) * OPENING_BYTES);
    for &(cell, inside, kind) in openings {
        let (kind, value) = match kind {
            Opening::Velocity(u) => (0u32, [u[0] as f32, u[1] as f32, u[2] as f32, 0.0]),
            Opening::Density(rho) => (1u32, [(rho - 1.0) as f32, 0.0, 0.0, 0.0]),
            Opening::Inflow { velocity: u, density } => (2u32, [u[0] as f32, u[1] as f32, u[2] as f32, (density - 1.0) as f32]),
        };
        for word in [cell as u32, inside as u32, kind, 0] {
            packed.extend_from_slice(&word.to_le_bytes());
        }
        for v in value {
            packed.extend_from_slice(&v.to_le_bytes());
        }
    }
    if packed.is_empty() {
        packed.resize(OPENING_BYTES, 0);
    }
    Ok(packed)
}

/// The wall links as the shader reads them, with the cell each reads upstream. A link with no
/// fluid upstream stays half-way, as on the CPU, so it is left out.
fn pack_walls(lattice: &Lattice) -> (Vec<u8>, u32) {
    let mut packed = Vec::with_capacity(lattice.walls().len().max(1) * WALL_BYTES);
    let mut count = 0u32;
    for l in lattice.walls() {
        let WallLink { cell, q, frac } = *l;
        let Some(up) = lattice.upstream(l) else { continue };
        let (Ok(cell), Ok(q), Ok(up)) = (u32::try_from(cell), u32::try_from(q), u32::try_from(up)) else { continue };
        packed.extend_from_slice(&cell.to_le_bytes());
        packed.extend_from_slice(&q.to_le_bytes());
        packed.extend_from_slice(&(frac as f32).to_le_bytes());
        packed.extend_from_slice(&up.to_le_bytes());
        count += 1;
    }
    (packed, count)
}

/// The compute pipelines, ready to step lattices on one device.
pub struct FlowGpu {
    adapter: String,
    device: wgpu::Device,
    queue: wgpu::Queue,
    layout: wgpu::BindGroupLayout,
    collide: wgpu::ComputePipeline,
    walls: wgpu::ComputePipeline,
    openings: wgpu::ComputePipeline,
}

impl FlowGpu {
    /// The pipelines on an existing device (`adapter`: its name, for reports). An error, never
    /// a panic, when the device cannot build them.
    pub fn new(adapter: impl Into<String>, device: wgpu::Device, queue: wgpu::Queue) -> Result<Self> {
        Self::with_shader(adapter, device, queue, SHADER)
    }

    /// [`FlowGpu::new`] with another shader source (tests make pipeline creation fail with it).
    #[doc(hidden)]
    pub fn with_shader(adapter: impl Into<String>, device: wgpu::Device, queue: wgpu::Queue, shader: &str) -> Result<Self> {
        let scopes = [wgpu::ErrorFilter::OutOfMemory, wgpu::ErrorFilter::Internal, wgpu::ErrorFilter::Validation].map(|f| device.push_error_scope(f));
        let built = Self::build(&device, shader);
        let mut failed = None;
        for scope in scopes.into_iter().rev() {
            if let Some(e) = popped(scope.pop()) {
                failed.get_or_insert(e.to_string());
            }
        }
        if let Some(e) = failed {
            return Err(GpuError::Pipeline(e));
        }
        let (layout, collide, walls, openings) = built;
        Ok(FlowGpu { adapter: adapter.into(), device, queue, layout, collide, walls, openings })
    }

    fn build(device: &wgpu::Device, shader: &str) -> (wgpu::BindGroupLayout, wgpu::ComputePipeline, wgpu::ComputePipeline, wgpu::ComputePipeline) {
        let module = device
            .create_shader_module(wgpu::ShaderModuleDescriptor { label: Some("solidcraft flow"), source: wgpu::ShaderSource::Wgsl(shader.into()) });
        let entry = |binding: u32, ty: wgpu::BufferBindingType| wgpu::BindGroupLayoutEntry {
            binding,
            visibility: wgpu::ShaderStages::COMPUTE,
            ty: wgpu::BindingType::Buffer { ty, has_dynamic_offset: false, min_binding_size: None },
            count: None,
        };
        let storage = |read_only| wgpu::BufferBindingType::Storage { read_only };
        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("solidcraft flow"),
            entries: &[
                entry(0, wgpu::BufferBindingType::Uniform),
                entry(1, storage(true)),
                entry(2, storage(true)),
                entry(3, storage(false)),
                entry(4, storage(true)),
            ],
        });
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("solidcraft flow"),
            bind_group_layouts: &[Some(&layout)],
            immediate_size: 0,
        });
        let pipeline = |entry: &str| {
            device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
                label: Some("solidcraft flow"),
                layout: Some(&pipeline_layout),
                module: &module,
                entry_point: Some(entry),
                compilation_options: Default::default(),
                cache: None,
            })
        };
        let collide = pipeline("collide_stream");
        let walls = pipeline("apply_walls");
        let openings = pipeline("apply_openings");
        (layout, collide, walls, openings)
    }

    /// A device of its own, without a window (native only), opened with the adapter's largest
    /// storage buffers so big grids fit.
    #[cfg(not(target_arch = "wasm32"))]
    pub fn headless() -> Result<Self> {
        let (adapter, device, queue) = crate::open_headless_with(|a| wgpu::Limits {
            max_storage_buffer_binding_size: a.max_storage_buffer_binding_size,
            max_buffer_size: a.max_buffer_size,
            ..wgpu::Limits::downlevel_defaults().using_resolution(a.clone())
        })?;
        Self::new(adapter.get_info().name, device, queue)
    }

    /// The adapter's name, for reports.
    pub fn adapter_name(&self) -> &str {
        &self.adapter
    }

    /// The largest population buffer this device takes, in bytes.
    pub fn max_population_bytes(&self) -> u64 {
        let l = self.device.limits();
        l.max_buffer_size.min(l.max_storage_buffer_binding_size)
    }

    /// Most cells whose lattice fits this device: what its buffers take and, below the largest
    /// count it may hold at all (`cap`), the largest the device would allocate now, found by
    /// allocating its buffers and letting them go, with an eighth spare for the rest.
    pub fn max_cells(&self, cap: u64) -> u64 {
        let per_buffer = self.max_population_bytes() / (Q as u64 * 4);
        let mut cells = cap.min(per_buffer).min(u64::from(u32::MAX) / Q as u64);
        let storage = wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_SRC | wgpu::BufferUsages::COPY_DST;
        while cells > 100_000 {
            let scope = self.device.push_error_scope(wgpu::ErrorFilter::OutOfMemory);
            let buffer = |size: u64, usage| {
                self.device.create_buffer(&wgpu::BufferDescriptor { label: Some("flow size probe"), size, usage, mapped_at_creation: false })
            };
            let pops = cells * Q as u64 * 4;
            let held = [buffer(pops, storage), buffer(pops, storage), buffer(cells * 4, wgpu::BufferUsages::STORAGE)];
            let failed = popped(scope.pop()).is_some();
            for b in held {
                b.destroy();
            }
            if !failed {
                return cells - cells / 8;
            }
            cells -= cells / 5;
        }
        cells
    }

    /// A lattice's state on the GPU. Errors (never panics) for a grid larger than the device's
    /// buffers, and for an opening whose inside cell is an opening too (the GPU rewrites all
    /// openings at once, so one may not read another).
    pub fn upload(&self, lattice: &Lattice) -> Result<GpuLattice> {
        let solid = lattice.solid();
        let cells = solid.len();
        let max = self.max_population_bytes();
        let bytes = population_bytes(cells).filter(|b| *b <= max).ok_or(GpuError::FlowTooLarge { cells, max })?;
        let n = cells as u32;
        let d = &self.device;
        let limits = d.limits();

        let openings = lattice.openings();
        let packed = pack_openings(lattice)?;
        let opening_count = u32::try_from(openings.len()).map_err(|_| GpuError::FlowTooLarge { cells, max })?;
        let (walls, wall_count) = pack_walls(lattice);

        // Dispatch rows: as many workgroups across as the device allows, then further rows.
        let groups = n.div_ceil(WORKGROUP).max(1);
        let across = groups.min(limits.max_compute_workgroups_per_dimension.max(1));
        let rows = groups.div_ceil(across);
        if rows > limits.max_compute_workgroups_per_dimension {
            return Err(GpuError::FlowTooLarge { cells, max });
        }
        let opening_groups = opening_count.div_ceil(WORKGROUP);
        let opening_dispatch = (opening_groups.min(across), opening_groups.div_ceil(across));
        let wall_groups = wall_count.div_ceil(WORKGROUP);
        let wall_dispatch = (wall_groups.min(across), wall_groups.div_ceil(across));
        if wall_dispatch.1 > limits.max_compute_workgroups_per_dimension {
            return Err(GpuError::FlowTooLarge { cells, max });
        }

        let mut params = Vec::with_capacity(80);
        let [nx, ny, nz] = lattice.dims;
        let periodic = lattice.periodic.map(u32::from);
        for w in [nx as u32, ny as u32, nz as u32, n, periodic[0], periodic[1], periodic[2], opening_count] {
            params.extend_from_slice(&w.to_le_bytes());
        }
        let g = lattice.force;
        for v in [g[0] as f32, g[1] as f32, g[2] as f32, lattice.tau as f32, lattice.smagorinsky() as f32] {
            params.extend_from_slice(&v.to_le_bytes());
        }
        let (sponge, sponge_tau) = lattice.sponge().map_or((0, 0.0), |(w, t)| (u32::try_from(w).unwrap_or(u32::MAX), t as f32));
        for w in [across * WORKGROUP, u32::from(lattice.is_regularized()), sponge] {
            params.extend_from_slice(&w.to_le_bytes());
        }
        params.extend_from_slice(&sponge_tau.to_le_bytes());
        params.extend_from_slice(&(lattice.bulk().unwrap_or(0.0) as f32).to_le_bytes());
        for w in [wall_count, 0] {
            params.extend_from_slice(&w.to_le_bytes());
        }

        // Populations, direction by direction, as offsets from rest.
        let f = lattice.populations();
        let mut start = vec![0u8; usize::try_from(bytes).map_err(|_| GpuError::FlowTooLarge { cells, max })?];
        for (cell, is_solid) in solid.iter().enumerate() {
            if *is_solid {
                continue;
            }
            for (q, w) in W.iter().enumerate() {
                let v = f.get(cell * Q + q).copied().unwrap_or(*w) - w;
                if let Some(slot) = start.get_mut((q * cells + cell) * 4..(q * cells + cell) * 4 + 4) {
                    slot.copy_from_slice(&(v as f32).to_le_bytes());
                }
            }
        }
        let damping = lattice.damping();
        let mut solid_words: Vec<u8> = solid
            .iter()
            .enumerate()
            .flat_map(|(i, s)| {
                let g = damping.and_then(|d| d.get(i)).copied().unwrap_or(u16::MAX);
                (u32::from(*s) | (u32::from(g) << 16)).to_le_bytes()
            })
            .collect();
        solid_words.extend_from_slice(&walls);
        if u64::try_from(solid_words.len()).map_or(true, |b| b > max) {
            return Err(GpuError::FlowTooLarge { cells, max });
        }

        let scope = d.push_error_scope(wgpu::ErrorFilter::OutOfMemory);
        let storage = wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_SRC | wgpu::BufferUsages::COPY_DST;
        let init = |label, contents: &[u8], usage| d.create_buffer_init(&wgpu::util::BufferInitDescriptor { label: Some(label), contents, usage });
        let params = init("flow params", &params, wgpu::BufferUsages::UNIFORM);
        let solid_buf = init("flow solid", &solid_words, wgpu::BufferUsages::STORAGE);
        let opening_buf = init("flow openings", &packed, wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST);
        let pops = [init("flow populations a", &start, storage), init("flow populations b", &start, storage)];
        if let Some(e) = popped(scope.pop()) {
            return Err(GpuError::Failed(e.to_string()));
        }
        let bind = |from: &wgpu::Buffer, to: &wgpu::Buffer| {
            d.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("flow"),
                layout: &self.layout,
                entries: &[
                    wgpu::BindGroupEntry { binding: 0, resource: params.as_entire_binding() },
                    wgpu::BindGroupEntry { binding: 1, resource: solid_buf.as_entire_binding() },
                    wgpu::BindGroupEntry { binding: 2, resource: from.as_entire_binding() },
                    wgpu::BindGroupEntry { binding: 3, resource: to.as_entire_binding() },
                    wgpu::BindGroupEntry { binding: 4, resource: opening_buf.as_entire_binding() },
                ],
            })
        };
        let binds = [bind(&pops[0], &pops[1]), bind(&pops[1], &pops[0])];
        Ok(GpuLattice {
            device: self.device.clone(),
            queue: self.queue.clone(),
            collide: self.collide.clone(),
            walls: self.walls.clone(),
            openings: self.openings.clone(),
            cells,
            opening_count: openings.len(),
            bytes,
            dispatch: (across, rows),
            wall_dispatch,
            opening_dispatch,
            pops,
            binds,
            current: 0,
            steps: 0,
            _inputs: [params, solid_buf, opening_buf],
        })
    }
}

/// A lattice's state on the GPU, stepped by [`GpuLattice::record`] or [`GpuLattice::run`].
pub struct GpuLattice {
    device: wgpu::Device,
    queue: wgpu::Queue,
    collide: wgpu::ComputePipeline,
    walls: wgpu::ComputePipeline,
    openings: wgpu::ComputePipeline,
    cells: usize,
    opening_count: usize,
    bytes: u64,
    dispatch: (u32, u32),
    wall_dispatch: (u32, u32),
    opening_dispatch: (u32, u32),
    /// The two population buffers; `current` holds the latest step.
    pops: [wgpu::Buffer; 2],
    /// Bind groups stepping from buffer 0 into 1, and from 1 into 0.
    binds: [wgpu::BindGroup; 2],
    current: usize,
    steps: u64,
    _inputs: [wgpu::Buffer; 3],
}

impl GpuLattice {
    /// Add `steps` time steps to `enc`; they run when it is submitted.
    pub fn record(&mut self, enc: &mut wgpu::CommandEncoder, steps: u32) {
        for _ in 0..steps {
            let mut pass = enc.begin_compute_pass(&wgpu::ComputePassDescriptor { label: Some("flow step"), timestamp_writes: None });
            pass.set_bind_group(0, &self.binds[self.current], &[]);
            pass.set_pipeline(&self.collide);
            pass.dispatch_workgroups(self.dispatch.0, self.dispatch.1, 1);
            if self.wall_dispatch.0 > 0 {
                pass.set_pipeline(&self.walls);
                pass.dispatch_workgroups(self.wall_dispatch.0, self.wall_dispatch.1, 1);
            }
            if self.opening_dispatch.0 > 0 {
                pass.set_pipeline(&self.openings);
                pass.dispatch_workgroups(self.opening_dispatch.0, self.opening_dispatch.1, 1);
            }
            drop(pass);
            self.current = 1 - self.current;
            self.steps += 1;
        }
    }

    /// Take over `lattice`'s openings (the same ones as uploaded, with new speeds or
    /// densities, for example a fan inlet following its curve).
    pub fn set_openings(&mut self, lattice: &Lattice) -> Result<()> {
        if lattice.openings().len() != self.opening_count || lattice.solid().len() != self.cells {
            return Err(GpuError::Failed("the openings are not the ones uploaded".into()));
        }
        let packed = pack_openings(lattice)?;
        let [_, _, openings] = &self._inputs;
        self.queue.write_buffer(openings, 0, &packed);
        Ok(())
    }

    /// Time steps taken since the upload.
    pub fn steps(&self) -> u64 {
        self.steps
    }

    /// Run `steps` time steps and wait for the GPU (native only).
    #[cfg(not(target_arch = "wasm32"))]
    pub fn run(&mut self, steps: u32) -> Result<()> {
        let scope = self.device.push_error_scope(wgpu::ErrorFilter::Validation);
        let mut left = steps;
        while left > 0 {
            let batch = left.min(STEPS_PER_SUBMIT);
            let mut enc = self.device.create_command_encoder(&wgpu::CommandEncoderDescriptor { label: Some("flow") });
            self.record(&mut enc, batch);
            self.queue.submit([enc.finish()]);
            left -= batch;
        }
        self.device.poll(wgpu::PollType::wait_indefinitely()).map_err(|e| GpuError::Failed(e.to_string()))?;
        match popped(scope.pop()) {
            Some(e) => Err(GpuError::Failed(e.to_string())),
            None => Ok(()),
        }
    }

    /// The latest populations, put into `lattice` (the one uploaded, or one like it) as its CPU
    /// populations (native only).
    #[cfg(not(target_arch = "wasm32"))]
    pub fn download(&self, lattice: &mut Lattice) -> Result<()> {
        if lattice.solid().len() != self.cells {
            return Err(GpuError::Failed("the lattice is not the one uploaded".into()));
        }
        let read = self.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("flow read"),
            size: self.bytes,
            usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let mut enc = self.device.create_command_encoder(&wgpu::CommandEncoderDescriptor { label: Some("flow read") });
        enc.copy_buffer_to_buffer(&self.pops[self.current], 0, &read, 0, self.bytes);
        self.queue.submit([enc.finish()]);
        let slice = read.slice(..);
        let (tx, rx) = std::sync::mpsc::channel();
        slice.map_async(wgpu::MapMode::Read, move |r| {
            let _ = tx.send(r);
        });
        self.device.poll(wgpu::PollType::wait_indefinitely()).map_err(|e| GpuError::Failed(e.to_string()))?;
        rx.recv().map_err(|e| GpuError::Failed(e.to_string()))?.map_err(|e| GpuError::Failed(e.to_string()))?;
        let bytes = slice.get_mapped_range().map_err(|e| GpuError::Failed(e.to_string()))?;
        let words = bytes.as_chunks::<4>().0;
        let mut f = vec![0.0; self.cells * Q];
        for (cell, is_solid) in lattice.solid().iter().enumerate() {
            if *is_solid {
                continue;
            }
            for (q, w) in W.iter().enumerate() {
                let v = words.get(q * self.cells + cell).map(|b| f64::from(f32::from_le_bytes(*b))).unwrap_or(0.0);
                if let Some(slot) = f.get_mut(cell * Q + q) {
                    *slot = v + w;
                }
            }
        }
        drop(bytes);
        read.unmap();
        lattice.set_populations(&f).map_err(|e| GpuError::Failed(e.to_string()))
    }
}

/// The GPU as a study's [`solidcraft_sim::Backend`] (native only: runs block on the device).
#[cfg(not(target_arch = "wasm32"))]
pub struct Backend(pub std::sync::Arc<FlowGpu>, std::sync::OnceLock<u64>);

#[cfg(not(target_arch = "wasm32"))]
impl Backend {
    pub fn new(gpu: std::sync::Arc<FlowGpu>) -> Self {
        Self(gpu, std::sync::OnceLock::new())
    }
}

/// The most cells a GPU study may have, on a device with memory to spare.
pub const MAX_CELLS: u64 = 40_000_000;

#[cfg(not(target_arch = "wasm32"))]
impl solidcraft_sim::Backend for Backend {
    fn name(&self) -> String {
        self.0.adapter_name().to_string()
    }

    fn stepper(&self, lattice: &Lattice) -> solidcraft_sim::Result<Box<dyn solidcraft_sim::Stepper>> {
        Ok(Box::new(self.0.upload(lattice).map_err(sim_error)?))
    }

    fn max_cells(&self) -> Option<u64> {
        Some(*self.1.get_or_init(|| self.0.max_cells(MAX_CELLS)))
    }
}

/// A study backend that opens its own GPU device the first time a study runs (native only), and
/// runs studies on the CPU reference when no GPU opens: what the app and the CLI install, so
/// starting them costs nothing until a study runs.
#[cfg(not(target_arch = "wasm32"))]
#[derive(Default)]
pub struct Lazy {
    gpu: std::sync::OnceLock<std::result::Result<Backend, String>>,
}

#[cfg(not(target_arch = "wasm32"))]
impl Lazy {
    fn gpu(&self) -> &std::result::Result<Backend, String> {
        self.gpu.get_or_init(|| match FlowGpu::headless() {
            Ok(g) => Ok(Backend::new(std::sync::Arc::new(g))),
            Err(e) => {
                log::warn!("Airflow studies run on the CPU: {e}");
                Err(e.to_string())
            }
        })
    }
}

#[cfg(not(target_arch = "wasm32"))]
impl solidcraft_sim::Backend for Lazy {
    fn name(&self) -> String {
        match self.gpu.get() {
            Some(Ok(g)) => solidcraft_sim::Backend::name(g),
            Some(Err(_)) => "CPU (no GPU)".into(),
            None => "GPU".into(),
        }
    }

    fn stepper(&self, lattice: &Lattice) -> solidcraft_sim::Result<Box<dyn solidcraft_sim::Stepper>> {
        match self.gpu() {
            Ok(g) => g.stepper(lattice),
            Err(_) => solidcraft_sim::Cpu.stepper(lattice),
        }
    }

    fn max_cells(&self) -> Option<u64> {
        self.gpu().as_ref().ok().and_then(solidcraft_sim::Backend::max_cells)
    }
}

#[cfg(not(target_arch = "wasm32"))]
fn sim_error(e: GpuError) -> solidcraft_sim::SimError {
    solidcraft_sim::SimError::Backend(e.to_string())
}

#[cfg(not(target_arch = "wasm32"))]
impl solidcraft_sim::Stepper for GpuLattice {
    fn run(&mut self, steps: u32) -> solidcraft_sim::Result<()> {
        GpuLattice::run(self, steps).map_err(sim_error)
    }

    fn read(&mut self, lattice: &mut Lattice) -> solidcraft_sim::Result<()> {
        self.download(lattice).map_err(sim_error)
    }

    fn openings(&mut self, lattice: &Lattice) -> solidcraft_sim::Result<()> {
        self.set_openings(lattice).map_err(sim_error)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn population_buffers_are_sized_without_overflow() {
        assert_eq!(population_bytes(1), Some(76));
        assert_eq!(population_bytes(1_000_000), Some(76_000_000));
        // Past a u32 index (19 Ã— cells words) or usize: none.
        assert_eq!(population_bytes(usize::MAX), None);
        assert_eq!(population_bytes((u32::MAX as usize) / 19 + 1), None);
    }
}
