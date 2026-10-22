// Universal gear and sprocket
// Sprocket layout: roller-chain drawing guide (Dp, Do, Df, Ds) plus ISO 606 seating/flank radii.
// Not a certified ASME B29.1 cutter profile.
// @param kind Kind {gear, sprocket} | gear
// @param hand Teeth {external, internal} | external @if kind=gear
// @param sizing Size {module, DP} | module @if kind=gear
// @param pa Pressure angle {14.5, 20, 25} | 20 @if kind=gear
// @param depth Depth {full, stub} | full @if kind=gear
// @param form Form {spur, helical, herringbone} | spur @if kind=gear
// @param helix Helix angle (deg) | 15 @if form=helical,herringbone
// @param gear_module Module | 2 @if sizing=module
// @param dp Diametral pitch | 16 @if sizing=DP
// @param rim Internal rim OD | 80 @if hand=internal
// @param chain ANSI {25, 35, 40, 41, 50, 60, 80, 100, 120, 140, 160} | 40 @if kind=sprocket
// @param teeth Teeth | 12
// @param thickness Thickness | 8 @if kind=gear
// @param edge_chamfer Edge chamfer {yes, no} | yes @if kind=sprocket
// @param bore Bore diameter | 8 @if hand=external
// @param hub Hub {none, one, both} | none @if hand=external
// @param hub_od Hub OD | 20 @if hub=one,both
// @param hub_length Hub length | 8 @if hub=one,both
fn as_n(v) {
    if type_of(v) == "string" { parse_float(v) } else { v }
}
fn as_s(v) {
    if type_of(v) == "string" { v } else { "" + v }
}
fn clamp_unit(x) {
    if x > 1.0 { return 1.0; }
    if x < -1.0 { return -1.0; }
    x
}
fn absf(x) {
    if x < 0.0 { 0.0 - x } else { x }
}
fn atan2f(y, x) {
    let r = sqrt(x * x + y * y);
    if r < 0.000000000001 { return 0.0; }
    let a = acos(clamp_unit(x / r));
    if y < 0.0 { 0.0 - a } else { a }
}
fn inv_of(cos_a) {
    let a = acos(clamp_unit(cos_a));
    tan(a) - a
}
fn ansi_chain(id) {
    let IN = 25.4;
    if id == "25" { return [0.250 * IN, 0.130 * IN, 0.110 * IN]; }
    if id == "35" { return [0.375 * IN, 0.200 * IN, 0.168 * IN]; }
    if id == "40" { return [0.500 * IN, 0.312 * IN, 0.284 * IN]; }
    if id == "41" { return [0.500 * IN, 0.306 * IN, 0.227 * IN]; }
    if id == "50" { return [0.625 * IN, 0.400 * IN, 0.343 * IN]; }
    if id == "60" { return [0.750 * IN, 0.469 * IN, 0.459 * IN]; }
    if id == "80" { return [1.000 * IN, 0.625 * IN, 0.575 * IN]; }
    if id == "100" { return [1.250 * IN, 0.750 * IN, 0.692 * IN]; }
    if id == "120" { return [1.500 * IN, 0.875 * IN, 0.924 * IN]; }
    if id == "140" { return [1.750 * IN, 1.000 * IN, 1.075 * IN]; }
    if id == "160" { return [2.000 * IN, 1.125 * IN, 1.225 * IN]; }
    throw "unknown ANSI chain";
}
fn circ_hits(c1, r1, c2, r2) {
    let dx = c2[0] - c1[0];
    let dy = c2[1] - c1[1];
    let d = sqrt(dx * dx + dy * dy);
    let out = [];
    if d < 0.000000000001 { return out; }
    if d > r1 + r2 + 0.000000001 { return out; }
    if d < absf(r1 - r2) - 0.000000001 { return out; }
    let a = (r1 * r1 - r2 * r2 + d * d) / (2.0 * d);
    let h2 = r1 * r1 - a * a;
    let px = c1[0] + a * dx / d;
    let py = c1[1] + a * dy / d;
    if h2 <= 0.0000000000000001 {
        out.push([px, py]);
        return out;
    }
    let h = sqrt(h2);
    out.push([px + h * dy / d, py - h * dx / d]);
    out.push([px - h * dy / d, py + h * dx / d]);
    out
}
fn ang_diff(a, b) {
    let pi = 3.141592653589793;
    let d = a - b;
    while d > pi { d = d - 2.0 * pi; }
    while d < 0.0 - pi { d = d + 2.0 * pi; }
    absf(d)
}
fn lerp_ang(a0, a1, u) {
    let pi = 3.141592653589793;
    let d = a1 - a0;
    while d > pi { d = d - 2.0 * pi; }
    while d < 0.0 - pi { d = d + 2.0 * pi; }
    a0 + d * u
}
fn pick_hit(hits, tip_ang, fallback) {
    if hits.len() == 0 { return fallback; }
    let best = hits[0];
    let best_d = ang_diff(atan2f(best[1], best[0]), tip_ang);
    let i = 1;
    while i < hits.len() {
        let p = hits[i];
        let d = ang_diff(atan2f(p[1], p[0]), tip_ang);
        if d < best_d {
            best = p;
            best_d = d;
        }
        i = i + 1;
    }
    best
}
fn involute_pts(z, m, alpha, ha, hf, invert, r_pitch) {
    let pi = 3.141592653589793;
    let r = r_pitch;
    let rb = r * cos(alpha);
    let ra = r + ha;
    let rf = r - hf;
    let r_inner = if rb > rf { rb } else { rf };
    if rf < m * 0.2 {
        throw "root diameter is too small for this teeth/module";
    }
    let inv_p = inv_of(cos(alpha));
    let half_w = pi / (2.0 * z);
    let n_flank = 24;
    let n_tip = 5;
    let n_root = 8;
    let pts = [];
    let tooth = 0;
    while tooth < z {
        let phi = 2.0 * pi * tooth / z;
        let g_l = phi - half_w;
        let g_r = phi + half_w;
        let k = 0;
        while k <= n_flank {
            let rr = r_inner + (ra - r_inner) * (k * 1.0) / (n_flank * 1.0);
            let inv_r = inv_of(rb / rr);
            let ang = g_l + (inv_r - inv_p);
            pts.push([rr * cos(ang), rr * sin(ang)]);
            k = k + 1;
        }
        let inv_t = inv_of(rb / ra);
        let tip_l = g_l + (inv_t - inv_p);
        let tip_r = g_r - (inv_t - inv_p);
        let k = 1;
        while k < n_tip {
            let ang = tip_l + (tip_r - tip_l) * (k * 1.0) / (n_tip * 1.0);
            pts.push([ra * cos(ang), ra * sin(ang)]);
            k = k + 1;
        }
        let k = n_flank;
        while k >= 0 {
            let rr = r_inner + (ra - r_inner) * (k * 1.0) / (n_flank * 1.0);
            let inv_r = inv_of(rb / rr);
            let ang = g_r - (inv_r - inv_p);
            pts.push([rr * cos(ang), rr * sin(ang)]);
            k = k - 1;
        }
        let phi_n = 2.0 * pi * (tooth + 1.0) / z;
        let g_l_n = phi_n - half_w;
        let inv_i = inv_of(rb / r_inner);
        let a_r = g_r - (inv_i - inv_p);
        let a_l = g_l_n + (inv_i - inv_p);
        if r_inner > rf + m * 0.001 {
            pts.push([rf * cos(a_r), rf * sin(a_r)]);
        }
        let k = 1;
        while k < n_root {
            let ang = a_r + (a_l - a_r) * (k * 1.0) / (n_root * 1.0);
            pts.push([rf * cos(ang), rf * sin(ang)]);
            k = k + 1;
        }
        if r_inner > rf + m * 0.001 {
            pts.push([rf * cos(a_l), rf * sin(a_l)]);
        }
        tooth = tooth + 1;
    }
    if invert {
        let out = [];
        let i = pts.len() - 1;
        while i >= 0 {
            let p = pts[i];
            let rr = sqrt(p[0] * p[0] + p[1] * p[1]);
            if rr > 0.001 {
                let s = (2.0 * r - rr) / rr;
                out.push([p[0] * s, p[1] * s]);
            }
            i = i - 1;
        }
        return out;
    }
    pts
}
fn sprocket_pts(z, P, Dr) {
    let pi = 3.141592653589793;
    let R = P / (2.0 * sin(pi / z));
    let ra = P * (0.6 + 1.0 / tan(pi / z)) / 2.0;
    let rs = 0.5025 * Dr + 0.003 * 25.4 / 2.0;
    let alpha = (35.0 + 60.0 / z) * pi / 180.0;
    let r2 = 0.8 * Dr;
    let n_s = 12;
    let n_f = 10;
    let n_t = 8;
    let pts = [];
    let k = 0;
    while k < z {
        let th = 2.0 * pi * k / z;
        let cx = R * cos(th);
        let cy = R * sin(th);
        let i = 0;
        while i <= n_s {
            let phi = pi + alpha - 2.0 * alpha * (i * 1.0) / (n_s * 1.0);
            pts.push([cx + rs * cos(th + phi), cy + rs * sin(th + phi)]);
            i = i + 1;
        }
        let ux = cos(th + pi - alpha);
        let uy = sin(th + pi - alpha);
        let t_out = [cx + rs * ux, cy + rs * uy];
        let c2 = [t_out[0] - r2 * ux, t_out[1] - r2 * uy];
        let tip_ang = th + pi / z;
        let fb = [ra * cos(tip_ang), ra * sin(tip_ang)];
        let u_hit = pick_hit(circ_hits(c2, r2, [0.0, 0.0], ra), tip_ang, fb);
        let a0 = atan2f(t_out[1] - c2[1], t_out[0] - c2[0]);
        let a1 = atan2f(u_hit[1] - c2[1], u_hit[0] - c2[0]);
        let j = 1;
        while j <= n_f {
            let a = lerp_ang(a0, a1, (j * 1.0) / (n_f * 1.0));
            pts.push([c2[0] + r2 * cos(a), c2[1] + r2 * sin(a)]);
            j = j + 1;
        }
        let thn = 2.0 * pi * (k + 1.0) / z;
        let cxn = R * cos(thn);
        let cyn = R * sin(thn);
        let unx = cos(thn + pi + alpha);
        let uny = sin(thn + pi + alpha);
        let t_in = [cxn + rs * unx, cyn + rs * uny];
        let c2n = [t_in[0] - r2 * unx, t_in[1] - r2 * uny];
        let u_in = pick_hit(circ_hits(c2n, r2, [0.0, 0.0], ra), tip_ang, fb);
        let ta0 = atan2f(u_hit[1], u_hit[0]);
        let ta1 = atan2f(u_in[1], u_in[0]);
        let i = 1;
        while i < n_t {
            let a = lerp_ang(ta0, ta1, (i * 1.0) / (n_t * 1.0));
            pts.push([ra * cos(a), ra * sin(a)]);
            i = i + 1;
        }
        let b0 = atan2f(u_in[1] - c2n[1], u_in[0] - c2n[0]);
        let b1 = atan2f(t_in[1] - c2n[1], t_in[0] - c2n[0]);
        let j = 1;
        while j < n_f {
            let a = lerp_ang(b0, b1, (j * 1.0) / (n_f * 1.0));
            pts.push([c2n[0] + r2 * cos(a), c2n[1] + r2 * sin(a)]);
            j = j + 1;
        }
        k = k + 1;
    }
    pts
}
fn chamfer_sprocket_edges(ra, t, h, g) {
    let r = ra + 0.05;
    run("solid.cylinder", #{ radius: r, height: t, base: [0.0, 0.0, 0.0], axis: "Z", operation: "new", name: "Blank", body_name: "Blank" });
    run("solid.chamfer", #{ edges: [[r, 0.0, t]], distance: h, distance2: g, body: "Blank" });
    run("solid.chamfer", #{ edges: [[r, 0.0, 0.0]], distance: h, distance2: g, flip: true, body: "Blank" });
    run("solid.combine", #{ target: "Gear", tools: ["Blank"], operation: "intersect" });
}

let kind_s = as_s(kind);
let hand_s = as_s(hand);
let sizing_s = as_s(sizing);
let depth_s = as_s(depth);
let form_s = as_s(form);
let hub_s = as_s(hub);
let chain_s = as_s(chain);
let z = as_n(teeth);
let bore_d = as_n(bore);
let rim_n = as_n(rim);
let hub_od_n = as_n(hub_od);
let hub_len = as_n(hub_length);
let pa_n = as_n(pa);
let pi = 3.141592653589793;
let helix_n = 0.0;
if form_s != "spur" && kind_s != "sprocket" {
    helix_n = as_n(helix);
}
if kind_s == "sprocket" {
    if z < 9 { throw "need at least 9 teeth"; }
} else if z < 6 {
    throw "need at least 6 teeth";
}
if z > 120 {
    throw "at most 120 teeth";
}

let pts = [];
let r_pitch = 0.0;
let t = as_n(thickness);
let internal = false;
let sprocket_P = 0.0;
let sprocket_ra = 0.0;
let sprocket_rf = 0.0;
if kind_s == "sprocket" {
    let dim = ansi_chain(chain_s);
    let P = dim[0];
    let Dr = dim[1];
    t = dim[2];
    r_pitch = P / (2.0 * sin(pi / z));
    sprocket_P = P;
    sprocket_ra = P * (0.6 + 1.0 / tan(pi / z)) / 2.0;
    sprocket_rf = r_pitch - Dr / 2.0;
    if bore_d > 0.0 && bore_d >= 2.0 * sprocket_rf - 1.0 {
        throw "bore is too large for this root diameter";
    }
    if hub_s != "none" && hub_od_n >= 2.0 * sprocket_rf {
        throw "hub OD must stay inside the root diameter";
    }
    pts = sprocket_pts(z, P, Dr);
} else {
    let m = if sizing_s == "DP" { 25.4 / as_n(dp) } else { as_n(gear_module) };
    if m <= 0.0 {
        throw "module / DP must be positive";
    }
    if t <= 0.0 {
        throw "thickness must be positive";
    }
    let alpha = pa_n * pi / 180.0;
    let ha = if depth_s == "stub" { 0.8 * m } else { 1.0 * m };
    let hf = if depth_s == "stub" { 1.0 * m } else { 1.25 * m };
    r_pitch = m * z / 2.0;
    internal = hand_s == "internal";
    pts = involute_pts(z, m, alpha, ha, hf, internal, r_pitch);
    if internal {
        let inner_max = 0.0;
        let i = 0;
        while i < pts.len() {
            let p = pts[i];
            let rr = sqrt(p[0] * p[0] + p[1] * p[1]);
            if rr > inner_max { inner_max = rr; }
            i = i + 1;
        }
        if rim_n / 2.0 <= inner_max + m * 0.5 {
            throw "internal rim OD must clear the teeth";
        }
    }
}

if internal {
    run("sketch.create", #{ plane: "XY", name: "Rim" });
    run("sketch.circle.center", #{ center: [0.0, 0.0], radius: rim_n / 2.0 });
    run("sketch.finish");
    run("solid.extrude", #{ sketch: "Rim", distance: t, name: "Gear" });
    run("sketch.create", #{ plane: "XY", name: "ToothCut" });
    run("sketch.line", #{ points: pts, closed: true, infer: false });
    run("sketch.finish");
    run("solid.extrude", #{ sketch: "ToothCut", distance: t, operation: "cut", name: "ToothCut" });
} else if helix_n != 0.0 {
    let h = t;
    if form_s == "herringbone" {
        h = t / 2.0;
    }
    run("sketch.create", #{ plane: "XY", name: "Gear" });
    run("sketch.line", #{ points: pts, closed: true, infer: false });
    run("sketch.finish");
    run("sketch.create", #{ plane: "XY", name: "SweepPath" });
    run("sketch.options", #{ sketch_3d: true });
    let path_r = run("sketch.line3d", #{ points: [[0.0, 0.0, 0.0], [0.0, 0.0, h]] });
    run("sketch.finish");
    let twist = h * tan(helix_n * pi / 180.0) / r_pitch * 180.0 / pi;
    run("solid.sweep", #{ sketch: "Gear", path_sketch: "SweepPath", path: [path_r.curves[0]], twist: twist, operation: "new", name: "Gear", body_name: "Gear" });
    if form_s == "herringbone" {
        run("solid.mirror", #{ bodies: ["Gear"], plane: #{ origin: [0.0, 0.0, h], normal: [0.0, 0.0, 1.0] }, combine: true });
    }
} else {
    run("sketch.create", #{ plane: "XY", name: "Gear" });
    run("sketch.line", #{ points: pts, closed: true, infer: false });
    run("sketch.finish");
    run("solid.extrude", #{ sketch: "Gear", distance: t, name: "Gear", body_name: "Gear" });
}

if kind_s == "sprocket" && as_s(edge_chamfer) != "no" {
    let h = sprocket_P / 2.0;
    let g = sprocket_P / 8.0;
    if 2.0 * g > t - 0.2 {
        g = (t - 0.2) / 2.0;
    }
    if sprocket_ra - h <= sprocket_rf + 0.2 {
        h = sprocket_ra - sprocket_rf - 0.2;
    }
    if g > 0.05 && h > 0.05 {
        chamfer_sprocket_edges(sprocket_ra, t, h, g);
    }
}

if hub_s != "none" && !internal {
    if hub_od_n <= bore_d {
        throw "hub OD must be larger than the bore";
    }
    run("solid.cylinder", #{ radius: hub_od_n / 2.0, height: hub_len, base: [0.0, 0.0, t], axis: "Z", operation: "join", name: "Hub" });
    if hub_s == "both" {
        run("solid.cylinder", #{ radius: hub_od_n / 2.0, height: hub_len, base: [0.0, 0.0, 0.0 - hub_len], axis: "Z", operation: "join", name: "HubBack" });
    }
}

if !internal && bore_d > 0.0 {
    let z_hole = t;
    if hub_s == "one" || hub_s == "both" {
        z_hole = t + hub_len;
    }
    run("solid.hole", #{ position: [0.0, 0.0, z_hole], diameter: bore_d });
}
