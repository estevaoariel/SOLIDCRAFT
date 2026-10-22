// Universal gear and sprocket
// @param kind Kind {gear, sprocket} | gear
// @param hand Teeth {external, internal} | external @if kind=gear
// @param sizing Size {module, DP} | module @if kind=gear
// @param pa Pressure angle {14.5, 20, 25} | 20 @if kind=gear
// @param depth Depth {full, stub} | full @if kind=gear
// @param gear_module Module | 2 @if sizing=module
// @param dp Diametral pitch | 16 @if sizing=DP
// @param helix Helix angle (deg) | 0 @if kind=gear
// @param rim Internal rim OD | 80 @if hand=internal
// @param chain ANSI {25, 35, 40, 50, 60, 80, 100, 120} | 40 @if kind=sprocket
// @param teeth Teeth | 12
// @param thickness Thickness | 8 @if kind=gear
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
fn inv_of(cos_a) {
    let a = acos(clamp_unit(cos_a));
    tan(a) - a
}
fn atan2(y, x) {
    let pi = 3.141592653589793;
    if x > 0.0 { return atan(y / x); }
    if x < 0.0 && y >= 0.0 { return atan(y / x) + pi; }
    if x < 0.0 && y < 0.0 { return atan(y / x) - pi; }
    if x == 0.0 && y > 0.0 { return pi / 2.0; }
    if x == 0.0 && y < 0.0 { return 0.0 - pi / 2.0; }
    0.0
}
fn spin(x, y, th) {
    [x * cos(th) - y * sin(th), x * sin(th) + y * cos(th)]
}
fn ansi_pitch(id) {
    if id == "25" { return 6.35; }
    if id == "35" { return 9.525; }
    if id == "40" { return 12.7; }
    if id == "50" { return 15.875; }
    if id == "60" { return 19.05; }
    if id == "80" { return 25.4; }
    if id == "100" { return 31.75; }
    if id == "120" { return 38.1; }
    throw "unknown ANSI chain";
}
fn ansi_roller(id) {
    if id == "25" { return 3.302; }
    if id == "35" { return 5.08; }
    if id == "40" { return 7.925; }
    if id == "50" { return 10.16; }
    if id == "60" { return 11.91; }
    if id == "80" { return 15.875; }
    if id == "100" { return 19.05; }
    if id == "120" { return 22.225; }
    throw "unknown ANSI chain";
}
fn ansi_width(id) {
    if id == "25" { return 3.175; }
    if id == "35" { return 4.775; }
    if id == "40" { return 7.925; }
    if id == "50" { return 9.525; }
    if id == "60" { return 12.7; }
    if id == "80" { return 15.875; }
    if id == "100" { return 19.05; }
    if id == "120" { return 25.4; }
    throw "unknown ANSI chain";
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
fn involute_tooth(z, m, alpha, ha, hf, r_pitch) {
    let pi = 3.141592653589793;
    let r = r_pitch;
    let rb = r * cos(alpha);
    let ra = r + ha;
    let rf = r - hf;
    let r_inner = if rb > rf { rb } else { rf };
    let inv_p = inv_of(cos(alpha));
    let half_w = pi / (2.0 * z);
    let n_flank = 16;
    let n_tip = 4;
    let g_l = 0.0 - half_w;
    let g_r = half_w;
    let pts = [];
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
    let first = pts[0];
    let last = pts[pts.len() - 1];
    let a0 = atan2(last[1], last[0]);
    let a1 = atan2(first[1], first[0]);
    let k = 1;
    while k < 6 {
        let u = (k * 1.0) / 6.0;
        let ang = a0 + (a1 - a0) * u;
        pts.push([rf * cos(ang), rf * sin(ang)]);
        k = k + 1;
    }
    pts
}
fn sprocket_pts(z, P, Dr) {
    let pi = 3.141592653589793;
    let R = P / (2.0 * sin(pi / z));
    let rs = 0.5025 * Dr + 0.0381;
    let ra = P * (0.6 + 1.0 / tan(pi / z)) / 2.0;
    let a = (ra * ra - rs * rs + R * R) / (2.0 * R);
    let h2 = ra * ra - a * a;
    let n_s = 16;
    let n_o = 8;
    let pts = [];
    if h2 <= 0.0 {
        throw "ANSI sprocket: outside diameter does not meet the seating curve";
    }
    let h = sqrt(h2);
    let phi0 = atan2(h, a - R);
    let phi1 = atan2(0.0 - h, a - R);
    let od0 = atan2(h, a);
    let od1 = atan2(0.0 - h, a);
    let span = phi1 + 2.0 * pi - phi0;
    let tooth = 0;
    while tooth < z {
        let th = 2.0 * pi * tooth / z;
        let k = 0;
        while k <= n_s {
            let phi = phi0 + span * (k * 1.0) / (n_s * 1.0);
            let p = spin(R + rs * cos(phi), rs * sin(phi), th);
            pts.push(p);
            k = k + 1;
        }
        let thn = 2.0 * pi * (tooth + 1.0) / z;
        let k = 1;
        while k < n_o {
            let ang = od1 + th + (od0 + thn - od1 - th) * (k * 1.0) / (n_o * 1.0);
            pts.push([ra * cos(ang), ra * sin(ang)]);
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
let bore_d = as_n(bore);
let helix_n = as_n(helix);
let rim_n = as_n(rim);
let hub_od_n = as_n(hub_od);
let hub_len = as_n(hub_length);
let pa_n = as_n(pa);
let pi = 3.141592653589793;
if z < 6 {
    throw "need at least 6 teeth";
}

let pts = [];
let r_pitch = 0.0;
let rf = 0.0;
let t = as_n(thickness);
let internal = false;
if kind_s == "sprocket" {
    let P = ansi_pitch(chain_s);
    let Dr = ansi_roller(chain_s);
    t = ansi_width(chain_s);
    r_pitch = P / (2.0 * sin(pi / z));
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
    rf = r_pitch - hf;
    internal = hand_s == "internal";
    if helix_n != 0.0 && !internal {
        pts = involute_tooth(z, m, alpha, ha, hf, r_pitch);
    } else {
        pts = involute_pts(z, m, alpha, ha, hf, internal, r_pitch);
    }
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
} else if kind_s == "gear" && helix_n != 0.0 {
    run("solid.cylinder", #{ radius: rf, height: t, base: [0.0, 0.0, 0.0], axis: "Z", name: "Core" });
    run("sketch.create", #{ plane: "XY", name: "Tooth" });
    run("sketch.line", #{ points: pts, closed: true, infer: false });
    run("sketch.finish");
    run("sketch.create", #{ plane: "XY", name: "SweepPath" });
    run("sketch.options", #{ sketch_3d: true });
    let path_r = run("sketch.line3d", #{ points: [[0.0, 0.0, 0.0], [0.0, 0.0, t]] });
    run("sketch.finish");
    let twist = t * tan(helix_n * pi / 180.0) / r_pitch * 180.0 / pi;
    run("solid.sweep", #{ sketch: "Tooth", path_sketch: "SweepPath", path: [path_r.curves[0]], twist: twist, operation: "new", name: "Tooth" });
    run("solid.pattern.circular", #{ features: ["Tooth"], axis: "Z", count: z });
} else {
    run("sketch.create", #{ plane: "XY", name: "Gear" });
    run("sketch.line", #{ points: pts, closed: true, infer: false });
    run("sketch.finish");
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
