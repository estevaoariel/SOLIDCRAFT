# Airflow studies

An Airflow study is steady, low-speed air moving through a design: a part-cooling duct, a fan
shroud, an enclosure's vents. Set the openings and a fan (or a speed), run it, and read the flow
(CFM), the pressures and the speed of the air where it lands. Goal Seek varies the design's
parameters to push a result toward a goal.

It runs on the GPU when there is one, and on the CPU for small grids when there is not. Nothing
in the design changes; studies are kept beside the design in `<design>.sim.json`.

## Using it

1. **Simulation workspace → Airflow.** Pick the duct's body.
2. **Inlets.** Click the face the air comes in through (a fan's face). For an open end with no
   face, give it as a section: a point in the opening and the direction the air goes in.
   - **Fan:** the type (axial, radial blower, straight line) with its free-air flow (CFM) and
     max pressure (Pa) from the datasheet, or the datasheet's curve as `[cfm, pa]` points. The
     study finds where the fan's curve meets the duct's resistance: the operating point.
   - **Set speed:** a fixed speed in m/s.
3. **Outlets** (optional, recommended). Where the air is expected to leave. They are measured
   (each outlet's share of the flow) and the air carries on past them into open air. Naming them
   also lets the solver time its steps for the jets they make; without them a fast jet can make
   the run unstable.
4. **Bodies in the air's way.** By default every visible body: the bed, the nozzle, the hotend.
5. **Surface finish.** Smooth (machined, moulded or resin printed) or rough (an FDM print),
   or from a script a roughness of your own (`roughness_mm`, the equivalent sand-grain roughness). See
   [Surface finish](#surface-finish) for what it changes.
6. **Cell size.** `auto` puts about eight cells across the narrowest opening and keeps cells no
   thicker than the thinnest walls. Smaller is more accurate and slower (cells are cubes, so
   half the size is eight times the cells). **Open air** is how far the box of air reaches past
   the bodies on every side; it also reaches one outlet width past each outlet.
7. **Run.** It runs until flows and pressures change by less than 0.5% between checks
   (**Settled**) or until the step limit. Then **Results**: flow, pressure and speed at each
   opening, the pressure drop, and **Show in the view**: speed or pressure on a section,
   streamlines, see-through bodies.

From a script or the MCP server, the same steps are `sim.flow.new`, `sim.flow.run`,
`sim.flow.results` and `sim.flow.probe` (the velocity and pressure at a point or over a grid);
`solidcraft-cli commands` lists their parameters. A complete example:

```json
[
  {"command": "sim.flow.new", "params": {"body": "Duct", "name": "Part cooling", "bodies": ["Duct", "Nozzle", "Bed"], "pad": 10, "cell": 0.8,
    "inlets": [{"name": "Fan 4010", "section": {"point": [22, 22, 13], "normal": [0, 0, -1]}, "fan": {"type": "axial", "free_flow_cfm": 6, "max_pressure_pa": 70}}],
    "outlets": [{"name": "Slot", "section": {"point": [83.5, 22, 5], "normal": [1, 0, 0]}}]}},
  {"command": "sim.flow.run", "params": {"study": "Part cooling", "wait": true}},
  {"command": "sim.flow.results", "params": {"study": "Part cooling"}}
]
```

### Sizing to the GPU

The first study in a session measures what the GPU can hold: it allocates a study's buffers at
decreasing sizes until the card takes them, keeps an eighth spare, and caps the grid there
(40 million cells at most). With `cell` on `auto`, a study that would not fit gets larger cells
instead of failing, so a 4 GB card runs the same study as an 8 GB one, more coarsely. A cell
takes about 156 bytes on the GPU, so 4 GB holds roughly 20 million cells less what other programs
are using. Without a GPU, studies run on the CPU up to 2 million cells.

### Warnings worth reading

- **The walls leak:** with the outlets closed the air still reaches open air. There is a gap,
  or a wall thinner than a cell. Use smaller cells or close the gap.
- **Walls thinner than a cell:** they are kept closed, but the flow beside them is coarse.
- **Cells of an opening lie in a wall:** the opening's face is partly in the duct's wall at
  this cell size; the fan's flow is measured one cell in, where it really enters.
- **Went unstable:** the air moved faster than the time step allows. Name the outlets, use
  smaller cells or a lower speed.

### Surface finish

A rough wall slows the air near it more than a smooth one. The study models this the way
Cebeci and Chang did for mixing-length models: roughness shifts the wall distance y⁺ used in
the near-wall damping by Δy⁺ = 0.9 (√k⁺ − k⁺ e^(−k⁺/6)), where k⁺ is the roughness in wall
units. The two presets are equivalent sand-grain roughness:

| finish | roughness | for |
|---|---|---|
| smooth | 0 | machined, moulded or resin-printed walls, whose roughness sits far inside the viscous layer |
| rough | 0.06 mm | an FDM print's walls: about three times the mean roughness Ra usually reported for them |

Below k⁺ of about 4.5 the roughness sits inside the wall's viscous layer and changes nothing:
the wall is *hydraulically smooth*. At part-cooling speeds that covers most prints. At 5 m/s
in a 20 mm duct, roughness starts to count at about 0.2 mm, rougher than a typical FDM wall.
So on most printer ducts the finish makes little difference, and the presets say so rather
than inventing one. It matters for fast jets and coarse prints (large nozzles, thick layers,
unsanded supports), where you can give your own `roughness_mm`. It does nothing with the
turbulence model off.

## How it works

The solver is a lattice Boltzmann method on a uniform grid of cubic cells (D3Q19: each cell
carries 19 populations of air moving to its neighbours). Each step every cell relaxes its
populations toward equilibrium (collision) and passes them to its neighbours (streaming). That
is local and the same everywhere, so it maps onto the GPU directly.

- **Collision:** regularized BGK, which keeps the populations' non-hydrodynamic parts from
  growing on coarse grids, with a separate bulk relaxation that damps sound waves.
- **Turbulence:** a Smagorinsky eddy viscosity (C = 0.14), damped near walls with Van Driest's
  function. Its constant is 80 rather than the textbook 26: the textbook value assumes the wall
  is resolved to y⁺ ≈ 1, and on millimetre cells the first cell is at y⁺ of 10 or more.
- **Walls:** interpolated bounce-back (Bouzidi), so a curved or slanted wall sits where it is
  rather than on the cell faces. Walls thinner than a cell are sealed so air cannot leak through.
- **Inlets:** a velocity on the inlet's cells. A fan's speed is moved, every few hundred steps,
  until the flow measured one cell in is on its curve at the pressure measured there.
- **Open air:** the box's faces hold ambient pressure. A sponge over the outer half of the
  padding raises the viscosity toward the faces so what leaves does not reflect back.
- **Units:** the time step is set so the fastest expected air moves 0.08 cells a step (Mach
  0.14 on the lattice), where the lattice's compressibility error stays around 2%.

## How accurate it is

Checked against a wall-resolved OpenFOAM solution (simpleFoam, laminar, structured O-grid with
1.04 million cells graded to 0.1 mm at the wall) and Shah and London's correlation for
developing flow, which agree with each other to 0.5%. The case: a 50 mm pipe, 300 mm long, air
in at 5 m/s (Re 16 700), 1 mm cells.

| | reference | SolidCraft | |
|---|---|---|---|
| pressure gradient mid-pipe, Pa per 50 mm | 0.50 | 0.48 | -3% |
| inlet pressure, Pa | 3.93 | 4.25 | +8% |

Before these checks the gradient read 1.5 times too high and the exit read 4 Pa of
back-pressure. An earlier OpenFOAM run on a 1 mm mesh with no wall layers read 0.14 Pa per
50 mm, which made SolidCraft look five times off; that mesh could not resolve a boundary layer
2 to 4 mm thick.

Two 3D-printer part-cooling ducts against OpenFOAM (k-omega SST, 500 iterations):

| duct | OpenFOAM | SolidCraft |
|---|---|---|
| shroud on a 5020 radial blower | 4.93 CFM | 4.78 CFM (-3%) |
| duct on a 4028 axial fan | 9.96 CFM | 7.16 CFM (-28%) |

The second is still open: the two set the duct up a little differently (which bodies are in the
air's way), and the fan's face sits partly in the duct's wall at 0.8 mm cells.

For ducts, trust differences between designs more than the absolute numbers: a change that
gives 20% more flow at the nozzle in SolidCraft will in OpenFOAM too.

## Limits

- Steady, low-speed air only (below about 30% of the speed of sound). Not compressible flow,
  shocks or external aerodynamics yet (`sim.gas.new` and `sim.aero.new` say so).
- No heat: temperatures and heat transfer are not simulated.
- Roughness acts only through the near-wall damping. Ridges larger than a cell are geometry,
  and are better drawn.
- One uniform cell size over the whole box. Thin passages need small cells everywhere.
