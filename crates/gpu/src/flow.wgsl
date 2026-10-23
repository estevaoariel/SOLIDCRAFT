// The Airflow solver's lattice Boltzmann step on the GPU: the f32 twin of
// solidcraft_sim::Lattice (D3Q19, BGK with the Smagorinsky model, Guo forcing, half-way
// bounce-back interpolated to the true wall where it is known, regularized collision, openings by
// non-equilibrium extrapolation). See flow.rs.
//
// Populations are stored as their offset from the rest state (f - w), direction by direction:
// direction q of cell c is at q * cells + c. The offsets keep a small flow's digits in f32.

struct Params {
    nx: u32,
    ny: u32,
    nz: u32,
    cells: u32,
    // Which axes wrap around (1) or end in walls (0).
    px: u32,
    py: u32,
    pz: u32,
    openings: u32,
    // Body force per unit volume.
    gx: f32,
    gy: f32,
    gz: f32,
    tau: f32,
    // The Smagorinsky constant (0: laminar).
    cs: f32,
    // Invocations per dispatch row: an index is gid.x + gid.y * row.
    row: u32,
    // Regularized collision (1): relax only the non-equilibrium stress.
    reg: u32,
    // The sponge along the walled faces: its width in cells (0: none) and the relaxation time
    // at the face.
    sponge: u32,
    sponge_tau: f32,
    // The bulk relaxation time (regularized collision; 0: the shear one).
    bulk: f32,
    // How many wall links follow the cells in `solid`.
    walls: u32,
    pad2: u32,
}

// An opening cell: kind 0 imposes the velocity value.xyz, kind 1 the density offset value.x
// (rho - 1), kind 2 the velocity value.xyz eased by the density's departure from the offset
// value.w (an inlet that lets sound out).
struct Opening {
    cell: u32,
    inside: u32,
    kind: u32,
    pad: u32,
    value: vec4<f32>,
}

@group(0) @binding(0) var<uniform> params: Params;
// Per cell: bit 0 solid; bits 16..31 the Smagorinsky length scale's share there (wall damping,
// 65535 = all of it). After the cells, four words per wall link: the fluid cell, the direction q
// into the wall, the fraction of the link where the wall is (f32 bits), and the fluid cell one
// step back from the wall, which the interpolation reads. (One buffer for both: the downlevel
// limit is four storage buffers per stage.)
@group(0) @binding(1) var<storage, read> solid: array<u32>;
@group(0) @binding(2) var<storage, read> src: array<f32>;
@group(0) @binding(3) var<storage, read_write> dst: array<f32>;
@group(0) @binding(4) var<storage, read> openings: array<Opening>;

const NONE: u32 = 0xffffffffu;

// Directions: rest, 6 faces, 12 edges, in solidcraft_sim::lbm::C's order.
var<private> CX: array<i32, 19> = array<i32, 19>(0, 1, -1, 0, 0, 0, 0, 1, -1, 1, -1, 1, -1, 1, -1, 0, 0, 0, 0);
var<private> CY: array<i32, 19> = array<i32, 19>(0, 0, 0, 1, -1, 0, 0, 1, -1, -1, 1, 0, 0, 0, 0, 1, -1, 1, -1);
var<private> CZ: array<i32, 19> = array<i32, 19>(0, 0, 0, 0, 0, 1, -1, 0, 0, 0, 0, 1, -1, -1, 1, 1, -1, -1, 1);
var<private> OPP: array<u32, 19> = array<u32, 19>(0u, 2u, 1u, 4u, 3u, 6u, 5u, 8u, 7u, 10u, 9u, 12u, 11u, 14u, 13u, 16u, 15u, 18u, 17u);
var<private> W: array<f32, 19> = array<f32, 19>(
    1.0 / 3.0,
    1.0 / 18.0, 1.0 / 18.0, 1.0 / 18.0, 1.0 / 18.0, 1.0 / 18.0, 1.0 / 18.0,
    1.0 / 36.0, 1.0 / 36.0, 1.0 / 36.0, 1.0 / 36.0, 1.0 / 36.0, 1.0 / 36.0,
    1.0 / 36.0, 1.0 / 36.0, 1.0 / 36.0, 1.0 / 36.0, 1.0 / 36.0, 1.0 / 36.0,
);

fn dir(q: u32) -> vec3<f32> {
    return vec3<f32>(f32(CX[q]), f32(CY[q]), f32(CZ[q]));
}

// The equilibrium's offset from rest for density 1 + drho and velocity u.
fn feq(q: u32, drho: f32, u: vec3<f32>) -> f32 {
    let cu = 3.0 * dot(dir(q), u);
    let uu = 1.5 * dot(u, u);
    return W[q] * (drho + (1.0 + drho) * (cu + 0.5 * cu * cu - uu));
}

// One coordinate moved by c: wrapped on a periodic axis; -1 past a wall face.
fn along(v: i32, c: i32, n: u32, periodic: u32) -> i32 {
    let p = v + c;
    let m = i32(n);
    if (p >= 0 && p < m) {
        return p;
    }
    if (periodic == 0u) {
        return -1;
    }
    return (p + m) % m;
}

// The cell along direction q from (i, j, k), or NONE past a wall face.
fn neighbour(i: i32, j: i32, k: i32, q: u32) -> u32 {
    let x = along(i, CX[q], params.nx, params.px);
    let y = along(j, CY[q], params.ny, params.py);
    let z = along(k, CZ[q], params.nz, params.pz);
    if (x < 0 || y < 0 || z < 0) {
        return NONE;
    }
    return u32(x) + params.nx * (u32(y) + params.ny * u32(z));
}

// Collide every fluid cell, then stream to its neighbours; a population heading into a wall
// comes back the opposite way. Every slot of a fluid cell is written by exactly one invocation.
@compute @workgroup_size(64)
fn collide_stream(@builtin(global_invocation_id) gid: vec3<u32>) {
    let cell = gid.x + gid.y * params.row;
    if (cell >= params.cells) {
        return;
    }
    let word = solid[cell];
    if ((word & 1u) != 0u) {
        return;
    }
    let n = params.cells;
    var f: array<f32, 19>;
    var drho = 0.0;
    var m = vec3<f32>(0.0);
    for (var q = 0u; q < 19u; q++) {
        let v = src[q * n + cell];
        f[q] = v;
        drho += v;
        m += v * dir(q);
    }
    let g = vec3<f32>(params.gx, params.gy, params.gz);
    let u = (m + 0.5 * g) / (1.0 + drho);
    var eq: array<f32, 19>;
    for (var q = 0u; q < 19u; q++) {
        eq[q] = feq(q, drho, u);
    }
    let i = i32(cell % params.nx);
    let j = i32((cell / params.nx) % params.ny);
    let k = i32(cell / (params.nx * params.ny));
    var tau = params.tau;
    if (params.sponge > 0u) {
        var d = 0x7fffffff;
        if (params.px == 0u) {
            d = min(d, min(i, i32(params.nx) - 1 - i));
        }
        if (params.py == 0u) {
            d = min(d, min(j, i32(params.ny) - 1 - j));
        }
        if (params.pz == 0u) {
            d = min(d, min(k, i32(params.nz) - 1 - k));
        }
        let w = i32(params.sponge);
        if (d < w) {
            let t = f32(w - d) / f32(w);
            tau = tau + (params.sponge_tau - tau) * t * t;
        }
    }
    // The non-equilibrium momentum flux, a symmetric tensor: diagonal d, off-diagonal o
    // (xy, xz, yz).
    var d = vec3<f32>(0.0);
    var o = vec3<f32>(0.0);
    if (params.cs > 0.0 || params.reg != 0u) {
        for (var q = 0u; q < 19u; q++) {
            let c = dir(q);
            let neq = f[q] - eq[q];
            d += c * c * neq;
            o += vec3<f32>(c.x * c.y, c.x * c.z, c.y * c.z) * neq;
        }
    }
    if (params.cs > 0.0) {
        let norm = sqrt(dot(d, d) + 2.0 * dot(o, o));
        let cs = params.cs * (f32(word >> 16u) / 65535.0);
        tau = 0.5 * (tau + sqrt(tau * tau + 18.0 * sqrt(2.0) * cs * cs * norm / (1.0 + drho)));
    }
    let keep = 1.0 - 0.5 / tau;
    let relax = 1.0 - 1.0 / tau;
    var relax_bulk = relax;
    if (params.bulk > 0.0) {
        relax_bulk = 1.0 - 1.0 / params.bulk;
    }
    let trace = (d.x + d.y + d.z) / 3.0;
    for (var q = 0u; q < 19u; q++) {
        let c = dir(q);
        let source = dot(3.0 * (c - u) + 9.0 * dot(c, u) * c, g);
        var relaxed = relax * (f[q] - eq[q]);
        if (params.reg != 0u) {
            // w (9/2) (c c - I/3) : Pi, as its deviatoric (shear) and isotropic (bulk) parts.
            let ccpi = dot(c * c, d) + 2.0 * dot(vec3<f32>(c.x * c.y, c.x * c.z, c.y * c.z), o);
            let c2 = dot(c, c);
            relaxed = relax * W[q] * 4.5 * (ccpi - c2 * trace) + relax_bulk * W[q] * 4.5 * (c2 - 1.0) * trace;
        }
        let post = eq[q] + relaxed + keep * W[q] * source;
        let to = neighbour(i, j, k, q);
        var slot = OPP[q] * n + cell;
        if (to != NONE) {
            if ((solid[to] & 1u) == 0u) {
                slot = q * n + to;
            }
        }
        dst[slot] = post;
    }
}

// Rewrite the opening cells after streaming: the imposed density or velocity, the other moment
// and the non-equilibrium part taken from the fluid cell just inside (never an opening itself).
@compute @workgroup_size(64)
fn apply_openings(@builtin(global_invocation_id) gid: vec3<u32>) {
    let index = gid.x + gid.y * params.row;
    if (index >= params.openings) {
        return;
    }
    let o = openings[index];
    let n = params.cells;
    var f: array<f32, 19>;
    var drho_in = 0.0;
    var m = vec3<f32>(0.0);
    for (var q = 0u; q < 19u; q++) {
        let v = dst[q * n + o.inside];
        f[q] = v;
        drho_in += v;
        m += v * dir(q);
    }
    let g = vec3<f32>(params.gx, params.gy, params.gz);
    let u_in = (m + 0.5 * g) / (1.0 + drho_in);
    var drho = drho_in;
    var u = o.value.xyz;
    if (o.kind == 1u) {
        drho = o.value.x;
        u = u_in;
    } else if (o.kind == 2u) {
        let speed = length(o.value.xyz);
        if (speed > 0.0) {
            u = o.value.xyz * (1.0 - 0.57735027 * (drho_in - o.value.w) / (1.0 + drho_in) / speed);
        }
    }
    for (var q = 0u; q < 19u; q++) {
        dst[q * n + o.cell] = feq(q, drho, u) + f[q] - feq(q, drho_in, u_in);
    }
}

// After streaming, before the openings: each wall link's bounced population interpolated to
// where the wall is (solidcraft_sim::Lattice::apply_walls). Every slot read here is one no link
// writes, so all links run at once.
@compute @workgroup_size(64)
fn apply_walls(@builtin(global_invocation_id) gid: vec3<u32>) {
    let index = gid.x + gid.y * params.row;
    if (index >= params.walls) {
        return;
    }
    let n = params.cells;
    let at = n + 4u * index;
    let cell = solid[at];
    let q = solid[at + 1u];
    let frac = bitcast<f32>(solid[at + 2u]);
    let up = solid[at + 3u];
    let opp = OPP[q];
    let a = dst[opp * n + cell];
    var value: f32;
    if (frac < 0.5) {
        value = 2.0 * frac * a + (1.0 - 2.0 * frac) * dst[q * n + cell];
    } else {
        value = a / (2.0 * frac) + (1.0 - 1.0 / (2.0 * frac)) * dst[opp * n + up];
    }
    dst[opp * n + cell] = value;
}