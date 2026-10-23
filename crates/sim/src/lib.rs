//! Flow simulation setup, without GPU code: a body's triangle mesh becomes a Cartesian voxel
//! grid of solid and fluid cells ([`voxelize`]), and a flood fill from a seed finds the fluid
//! region a study runs in ([`fluid_region`]), reporting whether it leaks to the grid's edge.
//! A study in physical units ([`study`]) becomes a lattice ([`lbm`], the CPU reference), and a
//! [`Run`] steps it until the flow settles. The GPU solver (wgpu compute) sits above this crate,
//! beside the app's GPU code, and plugs in as a [`Backend`].
#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]
#![forbid(unsafe_code)]

mod fluid;
mod grid;
pub mod lbm;
pub mod run;
pub mod study;
mod voxelize;

pub use fluid::{Region, fluid_region};
pub use grid::{Cell, Grid};
pub use lbm::{Lattice, Opening, WallLink};
pub use run::{Backend, Cpu, Fields, Limits, Quantity, Run, Sample, State, Stepper};
pub use study::{AIR, Air, FanCurve, FlowSetup, Inlet, Outlet, Patch, Prepared, Source, Through, Units, Warning, prepare, prepare_paced};
pub use voxelize::{voxelize, wall_links};

#[derive(Debug, thiserror::Error, PartialEq)]
pub enum SimError {
    #[error("the cell size must be a positive length")]
    InvalidSpacing,
    #[error("the grid would have {cells} cells, more than the limit of {max}; use larger cells")]
    TooManyCells { cells: u64, max: u64 },
    #[error("the body has no triangles to voxelize")]
    EmptyMesh,
    #[error("the body's mesh refers to a vertex it does not have")]
    BadMesh,
    #[error("the body's surface is not closed, so its inside cannot be found")]
    NotClosed,
    #[error("the relaxation time must be above ½ and the force finite")]
    InvalidRelaxation,
    #[error("an opening must be fluid cells with fluid just inside, a finite speed under 0.3 or a positive density")]
    BadOpening,
    #[error("the seed cell is outside the grid or not fluid")]
    BadSeed,
    #[error("the populations must be finite and one set per cell")]
    BadPopulations,
    #[error(
        "a fan curve needs 2 to 64 points (flow m³/s, pressure Pa) with the flow rising and the pressure falling, from a positive shut-off pressure to a positive free-air flow"
    )]
    BadFanCurve,
    #[error("the study needs at least one inlet")]
    NoInlet,
    #[error("the opening `{0}` covers no air cell; pick its face again or use smaller cells")]
    OpeningTooSmall(String),
    #[error("the flow went unstable after {steps} steps; use smaller cells, a lower speed or turn turbulence on")]
    Unstable { steps: u64 },
    #[error("the solver failed: {0}")]
    Backend(String),
    #[error("the study was cancelled")]
    Cancelled,
}

pub type Result<T> = std::result::Result<T, SimError>;
