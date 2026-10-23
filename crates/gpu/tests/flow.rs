//! The GPU lattice Boltzmann step against its CPU reference (`solidcraft_sim::Lattice`) and
//! against exact solutions. Skipped, with a note, on machines without a GPU.

use solidcraft_geom::{Aabb3, Mesh, Vec3};
use solidcraft_gpu::GpuError;
use solidcraft_gpu::flow::FlowGpu;
use solidcraft_sim::lbm::C;
use solidcraft_sim::{Cell, Grid, Lattice, Opening, WallLink};

fn gpu() -> Option<FlowGpu> {
    match FlowGpu::headless() {
        Ok(g) => {
            eprintln!("GPU: {}", g.adapter_name());
            Some(g)
        }
        Err(e @ GpuError::Pipeline(_)) => panic!("{e}"),
        Err(e) if std::env::var_os("SOLIDCRAFT_REQUIRE_GPU").is_some() => panic!("a GPU is required here: {e}"),
        Err(e) => {
            eprintln!("skipping: {e}");
            None
        }
    }
}

fn ok<T, E: std::fmt::Debug>(r: Result<T, E>) -> T {
    match r {
        Ok(v) => v,
        Err(e) => panic!("{e:?}"),
    }
}

fn add_box(mesh: &mut Mesh, min: [f64; 3], max: [f64; 3]) {
    let base = mesh.positions.len() as u32;
    for k in 0..8 {
        let pick = |bit: usize, axis: usize| if k & bit == 0 { min[axis] } else { max[axis] };
        mesh.positions.push(Vec3 { x: pick(1, 0), y: pick(2, 1), z: pick(4, 2) });
    }
    for [a, b, c, d] in [[0, 2, 3, 1], [4, 5, 7, 6], [0, 1, 5, 4], [2, 6, 7, 3], [0, 4, 6, 2], [1, 3, 7, 5]] {
        mesh.triangles.push([base + a, base + b, base + c]);
        mesh.triangles.push([base + a, base + c, base + d]);
    }
}

/// A square duct along x, `outer` wide outside with `wall`-thick walls, voxelized in unit cells.
fn duct(outer: f64, wall: f64, len: f64) -> Grid {
    let mut mesh = Mesh::default();
    add_box(&mut mesh, [0.0, 0.0, 0.0], [len, outer, wall]);
    add_box(&mut mesh, [0.0, 0.0, outer - wall], [len, outer, outer]);
    add_box(&mut mesh, [0.0, 0.0, wall], [len, wall, outer - wall]);
    add_box(&mut mesh, [0.0, outer - wall, wall], [len, outer, outer - wall]);
    let mut grid = ok(Grid::around(mesh.bounds(), 1.0, 0, 1_000_000));
    ok(solidcraft_sim::voxelize(&mesh, &mut grid));
    grid
}

/// The duct with a velocity inlet at x = 0 and a pressure outlet at the far end.
fn duct_with_openings(grid: &Grid, tau: f64, u0: f64, cs: f64) -> (Lattice, Vec<(usize, usize)>) {
    duct_with(grid, tau, u0, cs, false)
}

fn duct_with(grid: &Grid, tau: f64, u0: f64, cs: f64, regularized: bool) -> (Lattice, Vec<(usize, usize)>) {
    let [nx, ny, nz] = grid.dims;
    let bore: Vec<(usize, usize)> =
        (0..ny).flat_map(|j| (0..nz).map(move |k| (j, k))).filter(|&(j, k)| grid.get(0, j, k) == Some(Cell::Fluid)).collect();
    let mut lattice = ok(ok(Lattice::new(grid, tau, [0.0; 3], [false; 3])).with_smagorinsky(cs));
    if regularized {
        lattice = ok(lattice.regularized().with_bulk(0.9));
        // And a wall damping that varies from cell to cell.
        let n = lattice.solid().len();
        lattice = ok(lattice.with_damping((0..n).map(|i| ((i * 7919) % 65536) as u16).collect()));
    }
    let inlet: Vec<[usize; 3]> = bore.iter().map(|&(j, k)| [0, j, k]).collect();
    let outlet: Vec<[usize; 3]> = bore.iter().map(|&(j, k)| [nx - 1, j, k]).collect();
    ok(lattice.add_opening(&inlet, [1, 0, 0], Opening::Velocity([u0, 0.0, 0.0])));
    ok(lattice.add_opening(&outlet, [-1, 0, 0], Opening::Density(1.0)));
    (lattice, bore)
}

/// Every link from a fluid cell into a wall, with fractions spread over (0, 1]: both kinds of
/// interpolation, nearer and further than half-way.
fn some_walls(grid: &Grid) -> Vec<WallLink> {
    let [nx, ny, nz] = grid.dims;
    let mut links = Vec::new();
    for k in 0..nz {
        for j in 0..ny {
            for i in 0..nx {
                let Some(cell) = grid.index(i, j, k).filter(|c| grid.cells.get(*c) == Some(&Cell::Fluid)) else { continue };
                for (q, e) in C.iter().enumerate().skip(1) {
                    let at = [i as i64 + i64::from(e[0]), j as i64 + i64::from(e[1]), k as i64 + i64::from(e[2])];
                    if at.iter().any(|v| *v < 0) {
                        continue;
                    }
                    if grid.get(at[0] as usize, at[1] as usize, at[2] as usize) == Some(Cell::Solid) {
                        let frac = 0.1 + 0.9 * (((cell * 19 + q) * 7919 % 1000) as f64 / 1000.0);
                        links.push(WallLink { cell, q, frac });
                    }
                }
            }
        }
    }
    links
}

/// The largest difference in density and in velocity between two lattices on the same grid.
fn difference(a: &Lattice, b: &Lattice) -> (f64, f64) {
    let [nx, ny, nz] = a.dims;
    let (mut rho, mut u) = (0.0f64, 0.0f64);
    for k in 0..nz {
        for j in 0..ny {
            for i in 0..nx {
                if let (Some((ra, ua)), Some((rb, ub))) = (a.moments(i, j, k), b.moments(i, j, k)) {
                    rho = rho.max((ra - rb).abs());
                    u = u.max((0..3).map(|c| (ua[c] - ub[c]).abs()).fold(0.0, f64::max));
                }
            }
        }
    }
    (rho, u)
}

/// The same duct, openings, turbulence model, collision (plain or regularized) and walls (half-way
/// or interpolated) stepped on the CPU in f64 and on the GPU in f32
/// give the same flow, cell by cell, while it develops.
#[test]
fn the_gpu_step_follows_the_cpu_reference() {
    let Some(gpu) = gpu() else { return };
    let grid = duct(14.0, 2.0, 24.0);
    let u0 = 0.05;
    let walls = some_walls(&grid);
    assert!(walls.len() > 1000);
    for (cs, reg, curved) in [(0.0, false, false), (0.17, false, false), (0.17, true, false), (0.0, true, false), (0.17, true, true)] {
        let made = || {
            let l = duct_with(&grid, 0.56, u0, cs, reg).0;
            if curved { ok(l.with_walls(walls.clone())) } else { l }
        };
        let mut cpu = made();
        let mut on_gpu = ok(gpu.upload(&cpu));
        let mut back = made();
        for steps in [1u32, 49, 250] {
            for _ in 0..steps {
                cpu.step();
            }
            ok(on_gpu.run(steps));
            ok(on_gpu.download(&mut back));
            let (rho, u) = difference(&cpu, &back);
            eprintln!(
                "cs {cs}, regularized {reg}, curved walls {curved}, after {} steps: density within {rho:e}, velocity within {u:e}",
                on_gpu.steps()
            );
            assert!(rho < 1e-5 && u < 1e-4 * u0, "cs {cs}: density {rho:e}, velocity {u:e} apart after {} steps", on_gpu.steps());
        }
    }
}

/// A body force and periodic axes on the GPU: the channel between two plates develops the
/// exact parabola, as the CPU reference does.
#[test]
fn plane_poiseuille_flow_on_the_gpu_matches_the_parabola() {
    let Some(gpu) = gpu() else { return };
    let (nz, h) = (18usize, 16.0);
    let bounds = Aabb3 { min: Vec3 { x: 0.0, y: 0.0, z: 0.0 }, max: Vec3 { x: 4.0, y: 4.0, z: nz as f64 } };
    let mut grid = ok(Grid::around(bounds, 1.0, 0, 10_000));
    for j in 0..grid.dims[1] {
        for i in 0..grid.dims[0] {
            for k in [0, nz - 1] {
                if let Some(idx) = grid.index(i, j, k) {
                    grid.cells[idx] = Cell::Solid;
                }
            }
        }
    }
    let g = 1e-6;
    let mut lattice = ok(Lattice::new(&grid, 0.8, [g, 0.0, 0.0], [true, true, false]));
    let mut on_gpu = ok(gpu.upload(&lattice));
    ok(on_gpu.run(20_000));
    ok(on_gpu.download(&mut lattice));
    let nu = lattice.viscosity();
    let peak = g * h * h / (8.0 * nu);
    let mut worst = 0.0f64;
    for k in 1..nz - 1 {
        let z = k as f64 - 0.5;
        let exact = g / (2.0 * nu) * z * (h - z);
        let (rho, u) = ok(lattice.moments(1, 2, k).ok_or("fluid"));
        assert!((rho - 1.0).abs() < 1e-5, "mass is kept: {rho}");
        assert!(u[1].abs() < 1e-3 * peak && u[2].abs() < 1e-3 * peak, "{u:?}");
        worst = worst.max((u[0] - exact).abs() / peak);
    }
    eprintln!("worst error {worst:e} of the peak speed");
    assert!(worst < 1e-2, "worst error {worst} of the peak speed");
}

/// The duct with an inlet and an outlet run to a developed flow on the GPU: what goes in comes
/// out, and the pressure falls at the exact laminar rate (as in the CPU reference's test).
#[test]
fn a_duct_on_the_gpu_keeps_mass_and_the_laminar_pressure_drop() {
    let Some(gpu) = gpu() else { return };
    let (outer, wall, len) = (18.0, 3.0, 36.0);
    let grid = duct(outer, wall, len);
    let u0 = 0.02;
    let (mut lattice, bore) = duct_with_openings(&grid, 0.8, u0, 0.0);
    let mut on_gpu = ok(gpu.upload(&lattice));
    ok(on_gpu.run(8_000));
    ok(on_gpu.download(&mut lattice));
    let section = |i: usize| -> (f64, f64) {
        let (mut flow, mut rho) = (0.0, 0.0);
        for &(j, k) in &bore {
            let (r, u) = ok(lattice.moments(i, j, k).ok_or("fluid"));
            flow += r * u[0];
            rho += r;
        }
        (flow, rho / bore.len() as f64)
    };
    let a = outer - 2.0 * wall;
    let nominal = u0 * a * a;
    let (q_mid, rho_a) = section(12);
    let (q_out, rho_b) = section(28);
    assert!((q_mid - nominal).abs() / nominal < 0.06, "flow {q_mid} vs {nominal} nominal");
    assert!((q_out - q_mid).abs() / q_mid < 0.002, "mass kept: {q_out} out vs {q_mid}");
    let gradient = (rho_a - rho_b) / 3.0 / 16.0;
    let exact = lattice.viscosity() * q_mid / (0.035_144 * a.powi(4));
    let error = (gradient - exact).abs() / exact;
    assert!(error < 0.05, "dp/dx {gradient:e}, exact {exact:e}, error {error}");
}

/// What the GPU cannot run is an error, not a crash: openings stacked so that one reads another,
/// and a lattice that is not the one uploaded.
#[test]
fn what_the_gpu_cannot_run_is_an_error() {
    let Some(gpu) = gpu() else { return };
    let grid = duct(10.0, 2.0, 8.0);
    let (mut lattice, bore) = duct_with_openings(&grid, 0.8, 0.02, 0.0);
    let second: Vec<[usize; 3]> = bore.iter().map(|&(j, k)| [1, j, k]).collect();
    ok(lattice.add_opening(&second, [-1, 0, 0], Opening::Density(1.0)));
    assert!(matches!(gpu.upload(&lattice), Err(GpuError::Unsupported(_))), "an opening read from another opening");

    let (plain, _) = duct_with_openings(&grid, 0.8, 0.02, 0.0);
    let on_gpu = ok(gpu.upload(&plain));
    let mut other = ok(Lattice::new(&duct(10.0, 2.0, 12.0), 0.8, [0.0; 3], [false; 3]));
    assert!(on_gpu.download(&mut other).is_err(), "another grid");
}

/// A whole study (a duct in millimetres, an inlet at a set speed, a fan inlet on its curve)
/// run on the GPU gives the CPU run's flows, and settles the same way.
#[test]
fn a_study_runs_on_the_gpu_as_on_the_cpu() {
    use solidcraft_sim::{AIR, FanCurve, FlowSetup, Inlet, Limits, Outlet, Patch, Run, Source, State};
    let Some(gpu) = gpu() else { return };
    let backend = solidcraft_gpu::flow::Backend::new(std::sync::Arc::new(gpu));
    let (a, w, len) = (8.0, 2.0, 24.0);
    let mesh = {
        let mut m = Mesh::default();
        let o = a + 2.0 * w;
        add_box(&mut m, [0.0, 0.0, 0.0], [len, o, w]);
        add_box(&mut m, [0.0, 0.0, o - w], [len, o, o]);
        add_box(&mut m, [0.0, 0.0, w], [len, w, o - w]);
        add_box(&mut m, [0.0, o - w, w], [len, o, o - w]);
        m
    };
    let across = |x: f64| {
        let (lo, hi) = (w, w + a);
        let p = |y: f64, z: f64| Vec3::new(x, y, z);
        Patch { triangles: vec![[p(lo, lo), p(hi, lo), p(hi, hi)], [p(lo, lo), p(hi, hi), p(lo, hi)]], normal: Vec3::X }
    };
    for source in [Source::Speed(0.05), Source::Fan(ok(FanCurve::linear(4e-6, 0.05)))] {
        let setup = FlowSetup {
            cell: 1.0,
            pad: 4.0,
            air: AIR,
            inlets: vec![Inlet { name: "in".into(), patch: across(0.0), source: source.clone(), cap: true }],
            outlets: vec![Outlet { name: "out".into(), patch: across(len) }],
            turbulence: 0.0,
            roughness: 0.0,
            max_cells: 200_000,
        };
        let limits = Limits { max_steps: 30_000, check_every: 250, tolerance: 0.005, soft_start: true };
        let mut cpu = ok(Run::new(ok(solidcraft_sim::prepare(&[&mesh], &setup)), &solidcraft_sim::Cpu, limits));
        let mut on_gpu = ok(Run::new(ok(solidcraft_sim::prepare(&[&mesh], &setup)), &backend, limits));
        assert_eq!(ok(on_gpu.finish(|_| true)), State::Settled, "{source:?}");
        assert_eq!(ok(cpu.finish(|_| true)), State::Settled);
        let (c, g) = (cpu.last().unwrap(), on_gpu.last().unwrap());
        eprintln!(
            "{source:?}: CPU {} steps, {:e} m³/s at {} Pa; GPU {} steps, {:e} m³/s at {} Pa",
            cpu.steps, c.outlets[0].flow, c.inlets[0].pressure, on_gpu.steps, g.outlets[0].flow, g.inlets[0].pressure
        );
        assert!((c.outlets[0].flow - g.outlets[0].flow).abs() / c.outlets[0].flow < 0.01);
        assert!((c.inlets[0].pressure - g.inlets[0].pressure).abs() / c.inlets[0].pressure < 0.02);
        assert_eq!(on_gpu.backend, backend.0.adapter_name());
    }
}

/// The default Airflow case: a 50 mm bore pipe (2 mm wall), 300 mm long, air at 5 m/s, 1 mm
/// cells, turbulence on. Reports what it takes and the pressure drop next to the Blasius
/// friction factor's (fully developed; the short pipe's developing flow and exit add to it).
/// Heavy: run with `--ignored`.
#[test]
#[ignore]
fn the_default_pipe_case_runs_stably() {
    use solidcraft_sim::{AIR, FlowSetup, Inlet, Limits, Outlet, Patch, Run, Source};
    let Some(gpu) = gpu() else { return };
    let backend = solidcraft_gpu::flow::Backend::new(std::sync::Arc::new(gpu));
    let (r, wall, len, speed) = (25.0, 2.0, 300.0, 5.0);
    let outer = ok(solidcraft_kernel::cylinder(Vec3::ZERO, Vec3::Z, r + wall, len));
    let bore = ok(solidcraft_kernel::cylinder(Vec3::new(0.0, 0.0, -1.0), Vec3::Z, r, len + 2.0));
    let pipe = ok(solidcraft_kernel::boolean(&outer, &bore, solidcraft_kernel::BoolOp::Cut)).unwrap();
    let mesh = ok(pipe.display_plan(0.05)).evaluate();
    let disk = |z: f64| {
        let n = 96;
        let p = |i: usize| {
            let t = std::f64::consts::TAU * i as f64 / n as f64;
            Vec3::new(r * t.cos(), r * t.sin(), z)
        };
        Patch { triangles: (0..n).map(|i| [Vec3::new(0.0, 0.0, z), p(i), p(i + 1)]).collect(), normal: Vec3::Z }
    };
    let setup = FlowSetup {
        cell: 1.0,
        pad: 20.0,
        air: AIR,
        inlets: vec![Inlet { name: "inlet".into(), patch: disk(0.0), source: Source::Speed(speed), cap: true }],
        outlets: vec![Outlet { name: "outlet".into(), patch: disk(len) }],
        turbulence: 0.17,
        roughness: 0.0,
        max_cells: 20_000_000,
    };
    let t = std::time::Instant::now();
    let prepared = ok(solidcraft_sim::prepare(&[&mesh], &setup));
    let cells = prepared.grid.len();
    eprintln!(
        "prepared {:?} = {cells} cells in {:.1} s; tau {}, warnings {:?}",
        prepared.grid.dims,
        t.elapsed().as_secs_f64(),
        prepared.lattice.tau,
        prepared.warnings
    );
    let t = std::time::Instant::now();
    let mut run = ok(Run::new(prepared, &backend, Limits { max_steps: 60_000, check_every: 1_000, tolerance: 0.01, soft_start: true }));
    let state = ok(run.finish(|r| {
        if let Some(s) = r.last() {
            eprintln!(
                "  {} steps ({:.4} s simulated): in {:.3} Pa, {:.3e} m³/s; out {:.3e} m³/s",
                s.step, s.time, s.inlets[0].pressure, s.inlets[0].flow, s.outlets[0].flow
            );
        }
        true
    }));
    let secs = t.elapsed().as_secs_f64();
    let last = run.average().unwrap();
    let re = speed * 2.0 * r * 1e-3 / AIR.viscosity;
    let f = 0.316 * re.powf(-0.25);
    let blasius = f * len / (2.0 * r) * 0.5 * AIR.density * speed * speed;
    eprintln!(
        "{state:?} after {} steps in {secs:.1} s ({:.0} million cell updates a second); mean inlet {:.3} Pa vs Blasius {blasius:.3} Pa (Re {re:.0}); flow in {:.4e}, out {:.4e} m³/s (nominal {:.4e})",
        run.steps,
        cells as f64 * run.steps as f64 / secs / 1e6,
        last.inlets[0].pressure,
        last.inlets[0].flow,
        last.outlets[0].flow,
        speed * std::f64::consts::PI * (r * 1e-3).powi(2)
    );
    assert!(last.inlets[0].pressure > 0.5 * blasius && last.inlets[0].pressure < 10.0 * blasius);
}

/// A study is sized to what the device holds: no more cells than asked, enough for a real
/// study, and a lattice of that size uploads.
#[test]
fn the_cell_budget_fits_the_device() {
    let Some(gpu) = gpu() else { return };
    let cap = 40_000_000;
    let fits = gpu.max_cells(cap);
    eprintln!("{} holds {fits} cells", gpu.adapter_name());
    assert!(fits <= cap && fits > 1_000_000, "{fits}");
    assert!(fits as u128 * 19 * 4 <= u128::from(gpu.max_population_bytes()), "{fits}");
}
