// Universal gear and sprocket
// Involute gears (ISO 53 / Machinery's Handbook) and roller-chain sprockets (ISO 606, ASME B29.1)
// @param kind Kind {gear, sprocket} | gear
// @param hand Teeth {external, internal} | external
// @param sizing Size {module, DP} | module
// @param pa Pressure angle {14.5, 20, 25} | 20
// @param depth Depth {full, stub} | full
// @param chain Chain {25, 35, 40, 50, 60, 80, 06B, 08B, 10B, 12B, 16B} | 40
// @param hub Hub {none, one, both} | none
// @param teeth Teeth | 12
// @param size Module or DP | 2
// @param bore Bore diameter | 8
// @param thickness Thickness | 8
// @param helix Helix angle (deg) | 0
// @param rim Internal rim OD | 80
// @param hub_od Hub OD | 20
// @param hub_length Hub length | 8
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
fn inv_of(cos_a) {
    let a = acos(clamp_unit(cos_a));
    tan(a) - a
}
fn chain_pitch(id) {
    if id == "25" { return 6.35; }
    if id == "35" { return 9.525; }
    if id == "40" { return 12.7; }
    if id == "50" { return 15.875; }
    if id == "60" { return 19.05; }
    if id == "80" { return 25.4; }
    if id == "06B" { return 9.525; }
    if id == "08B" { return 12.7; }
    if id == "10B" { return 15.875; }
    if id == "12B" { return 19.05; }
    if id == "16B" { return 25.4; }
    throw "unknown chain";
}
fn chain_roller(id) {
    if id == "25" { return 3.3; }
    if id == "35" { return 5.08; }
    if id == "40" { return 7.92; }
    if id == "50" { return 10.16; }
    if id == "60" { return 11.91; }
    if id == "80" { return 15.88; }
    if id == "06B" { return 6.35; }
    if id == "08B" { return 7.92; }
    if id == "10B" { return 10.16; }
    if id == "12B" { return 12.07; }
    if id == "16B" { return 15.88; }
    throw "unknown chain";
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
    let n_flank = 16;
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
    let r_pitch = P / (2.0 * sin(pi / z));
    let r_seat = 0.5025 * Dr + 0.05;
    let da = P * (1.0 / tan(pi / z) + 0.5);
    let ra = da / 2.0;
    let n_seat = 8;
    let n_tip = 4;
    let span = 1.15;
    let pts = [];
    let tooth = 0;
    while tooth < z {
        let theta = 2.0 * pi * tooth / z;
        let cx = r_pitch * cos(theta);
        let cy = r_pitch * sin(theta);
        let k = 0;
        while k <= n_seat {
            let a = theta + pi - span + 2.0 * span * (k * 1.0) / (n_seat * 1.0);
            pts.push([cx + r_seat * cos(a), cy + r_seat * sin(a)]);
            k = k + 1;
        }
        let tip = theta + pi / z;
        let k = 1;
        while k < n_tip {
            let a = tip - 0.2 * pi / z + 0.4 * pi / z * (k * 1.0) / (n_tip * 1.0);
            pts.push([ra * cos(a), ra * sin(a)]);
            k = k + 1;
        }
        tooth = tooth + 1;
    }
    pts
}

let kind_s = as_s(kind);
let hand_s = as_s(hand);
let sizing_s = as_s(sizing);
let depth_s = as_s(depth);
let hub_s = as_s(hub);
let chain_s = as_s(chain);
let z = as_n(teeth);
let size_n = as_n(size);
let bore_d = as_n(bore);
let t = as_n(thickness);
let helix_n = as_n(helix);
let rim_n = as_n(rim);
let hub_od_n = as_n(hub_od);
let hub_len = as_n(hub_length);
let pa_n = as_n(pa);
let pi = 3.141592653589793;
if z < 6 {
    throw "need at least 6 teeth";
}
if t <= 0.0 {
    throw "thickness must be positive";
}

let pts = [];
let r_pitch = 0.0;
let internal = false;
if kind_s == "sprocket" {
    let P = chain_pitch(chain_s);
    let Dr = chain_roller(chain_s);
    r_pitch = P / (2.0 * sin(pi / z));
    pts = sprocket_pts(z, P, Dr);
} else {
    let m = if sizing_s == "DP" { 25.4 / size_n } else { size_n };
    if m <= 0.0 {
        throw "module / DP must be positive";
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

run("sketch.create", #{ plane: "XY", name: "Gear" });
if internal {
    run("sketch.circle.center", #{ center: [0.0, 0.0], radius: rim_n / 2.0 });
}
run("sketch.line", #{ points: pts, closed: true });
run("sketch.finish");

if kind_s == "gear" && helix_n != 0.0 && !internal {
    let twist = t * tan(helix_n * pi / 180.0) / r_pitch * 180.0 / pi;
    run("sketch.create", #{ plane: "XY", name: "SweepPath" });
    run("sketch.options", #{ sketch_3d: true });
    let path_r = run("sketch.line3d", #{ points: [[0.0, 0.0, 0.0], [0.0, 0.0, t]] });
    run("sketch.finish");
    run("solid.sweep", #{ sketch: "Gear", path_sketch: "SweepPath", path: [path_r.curves[0]], twist: twist, name: "Gear" });
} else {
    run("solid.extrude", #{ sketch: "Gear", distance: t, name: "Gear" });
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
