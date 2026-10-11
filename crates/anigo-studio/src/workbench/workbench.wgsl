// ANIGO Workbench — shaders do viewport de edição (grid, sólido, linhas, pontos, fundo).
// Cores saem em espaço sRGB (gamma) direto: o alvo é Rgba8Unorm e o egui exibe sem conversão.

struct Globals {
    view: mat4x4<f32>,
    proj: mat4x4<f32>,
    view_proj: mat4x4<f32>,
    inv_view_proj: mat4x4<f32>,
    eye: vec4<f32>,
    // x = largura px, y = altura px, z = 1/largura, w = 1/altura
    viewport: vec4<f32>,
    // x = xray (0/1), y = bias de profundidade do overlay (clip), z = escala do grid, w = ortográfica (0/1)
    params: vec4<f32>,
    bg_top: vec4<f32>,
    bg_bottom: vec4<f32>,
};

struct Object {
    model: mat4x4<f32>,
    // normal matrix (inversa transposta do 3x3 do model) em colunas vec4
    normal0: vec4<f32>,
    normal1: vec4<f32>,
    normal2: vec4<f32>,
    tint: vec4<f32>,
};

@group(0) @binding(0) var<uniform> g: Globals;
@group(1) @binding(0) var<uniform> obj: Object;

// ---------------------------------------------------------------- fundo (gradiente)

struct BgOut {
    @builtin(position) pos: vec4<f32>,
    @location(0) t: f32,
};

@vertex
fn vs_bg(@builtin(vertex_index) vi: u32) -> BgOut {
    var p = array<vec2<f32>, 3>(vec2(-1.0, -1.0), vec2(3.0, -1.0), vec2(-1.0, 3.0));
    var o: BgOut;
    o.pos = vec4(p[vi], 1.0, 1.0);
    o.t = p[vi].y * 0.5 + 0.5;
    return o;
}

@fragment
fn fs_bg(i: BgOut) -> @location(0) vec4<f32> {
    return mix(g.bg_bottom, g.bg_top, clamp(i.t, 0.0, 1.0));
}

// ---------------------------------------------------------------- sólido

struct SolidIn {
    @location(0) pos: vec3<f32>,
    @location(1) normal: vec3<f32>,
    @location(2) color: vec4<f32>,
};

struct SolidOut {
    @builtin(position) pos: vec4<f32>,
    @location(0) normal_view: vec3<f32>,
    @location(1) color: vec4<f32>,
};

@vertex
fn vs_solid(i: SolidIn) -> SolidOut {
    var o: SolidOut;
    let world = obj.model * vec4(i.pos, 1.0);
    o.pos = g.view_proj * world;
    let n = vec3(
        dot(obj.normal0.xyz, i.normal),
        dot(obj.normal1.xyz, i.normal),
        dot(obj.normal2.xyz, i.normal),
    );
    o.normal_view = (g.view * vec4(n, 0.0)).xyz;
    o.color = i.color * obj.tint;
    return o;
}

@fragment
fn fs_solid(i: SolidOut, @builtin(front_facing) front: bool) -> @location(0) vec4<f32> {
    var n = normalize(i.normal_view);
    if (!front) { n = -n; }
    // luz de estúdio (espaço de vista): chave à esquerda/acima, preenchimento à direita, rim
    let key = normalize(vec3(-0.45, 0.65, 0.62));
    let fill = normalize(vec3(0.7, -0.15, 0.55));
    let diff = max(dot(n, key), 0.0) * 0.62 + max(dot(n, fill), 0.0) * 0.18;
    let hemi = mix(0.30, 0.46, n.y * 0.5 + 0.5);
    let rim = pow(1.0 - clamp(n.z, 0.0, 1.0), 3.0) * 0.12;
    let shade = hemi + diff + rim;
    return vec4(i.color.rgb * shade, i.color.a);
}

// ---------------------------------------------------------------- linhas (quads em espaço de tela)

struct LineIn {
    @location(0) a: vec3<f32>,
    @location(1) b: vec3<f32>,
    @location(2) color: vec4<f32>,
    @location(3) width: f32,
};

struct LineOut {
    @builtin(position) pos: vec4<f32>,
    @location(0) color: vec4<f32>,
};

fn overlay_depth(clip: vec4<f32>) -> vec4<f32> {
    var c = clip;
    if (g.params.x > 0.5) {
        c.z = 0.0; // raio-X: sempre à frente
    } else {
        c.z = c.z - g.params.y * c.w;
    }
    return c;
}

@vertex
fn vs_line(@builtin(vertex_index) vi: u32, i: LineIn) -> LineOut {
    var ca = g.view_proj * (obj.model * vec4(i.a, 1.0));
    var cb = g.view_proj * (obj.model * vec4(i.b, 1.0));
    let near_w = 1e-4;
    var o: LineOut;
    o.color = i.color;
    // recorta contra o plano w = near_w
    if (ca.w < near_w && cb.w < near_w) {
        o.pos = vec4(2.0, 2.0, 2.0, 1.0);
        return o;
    }
    if (ca.w < near_w) {
        let t = (near_w - ca.w) / (cb.w - ca.w);
        ca = mix(ca, cb, t);
    } else if (cb.w < near_w) {
        let t = (near_w - cb.w) / (ca.w - cb.w);
        cb = mix(cb, ca, t);
    }
    let sa = ca.xy / ca.w * g.viewport.xy * 0.5;
    let sb = cb.xy / cb.w * g.viewport.xy * 0.5;
    var dir = sb - sa;
    let len = length(dir);
    if (len < 1e-4) { dir = vec2(1.0, 0.0); } else { dir = dir / len; }
    let nrm = vec2(-dir.y, dir.x);
    // 6 vértices: dois triângulos de um quad
    var corner = array<vec2<f32>, 6>(
        vec2(0.0, -1.0), vec2(1.0, -1.0), vec2(1.0, 1.0),
        vec2(0.0, -1.0), vec2(1.0, 1.0), vec2(0.0, 1.0),
    );
    let c = corner[vi];
    let base = select(ca, cb, c.x > 0.5);
    let s = select(sa, sb, c.x > 0.5);
    let offset_px = nrm * c.y * i.width * 0.5;
    let ndc_off = offset_px * 2.0 * g.viewport.zw;
    var clip = vec4(base.xy + ndc_off * base.w, base.z, base.w);
    o.pos = overlay_depth(clip);
    return o;
}

@fragment
fn fs_line(i: LineOut) -> @location(0) vec4<f32> {
    return i.color;
}

// ---------------------------------------------------------------- pontos

struct PointIn {
    @location(0) pos: vec3<f32>,
    @location(1) state: f32,
};

struct PointOut {
    @builtin(position) pos: vec4<f32>,
    @location(0) color: vec4<f32>,
    @location(1) uv: vec2<f32>,
};

@vertex
fn vs_point(@builtin(vertex_index) vi: u32, i: PointIn) -> PointOut {
    var corner = array<vec2<f32>, 6>(
        vec2(-1.0, -1.0), vec2(1.0, -1.0), vec2(1.0, 1.0),
        vec2(-1.0, -1.0), vec2(1.0, 1.0), vec2(-1.0, 1.0),
    );
    let c = corner[vi];
    let clip = g.view_proj * (obj.model * vec4(i.pos, 1.0));
    var o: PointOut;
    var size = 4.5;
    var col = vec4(0.04, 0.04, 0.05, 1.0);
    if (i.state > 1.5) {
        col = vec4(1.0, 1.0, 1.0, 1.0);
        size = 6.0;
    } else if (i.state > 0.5) {
        col = vec4(1.0, 0.60, 0.18, 1.0);
        size = 5.5;
    }
    o.color = col;
    o.uv = c;
    if (clip.w < 1e-4) {
        o.pos = vec4(2.0, 2.0, 2.0, 1.0);
        return o;
    }
    let ndc_off = c * size * g.viewport.zw * 2.0 * 0.5;
    let moved = vec4(clip.xy + ndc_off * clip.w, clip.z, clip.w);
    o.pos = overlay_depth(moved);
    return o;
}

@fragment
fn fs_point(i: PointOut) -> @location(0) vec4<f32> {
    let d = length(i.uv);
    if (d > 1.0) { discard; }
    return i.color;
}

// ---------------------------------------------------------------- grid infinito

struct GridOut {
    @builtin(position) pos: vec4<f32>,
    @location(0) ndc: vec2<f32>,
};

@vertex
fn vs_grid(@builtin(vertex_index) vi: u32) -> GridOut {
    var p = array<vec2<f32>, 3>(vec2(-1.0, -1.0), vec2(3.0, -1.0), vec2(-1.0, 3.0));
    var o: GridOut;
    o.pos = vec4(p[vi], 0.0, 1.0);
    o.ndc = p[vi];
    return o;
}

struct GridFrag {
    @location(0) color: vec4<f32>,
    @builtin(frag_depth) depth: f32,
};

fn grid_line(coord: vec2<f32>, scale: f32) -> f32 {
    let c = coord / scale;
    let w = fwidth(c);
    let a = abs(fract(c - 0.5) - 0.5) / max(w, vec2(1e-6));
    return 1.0 - min(min(a.x, a.y), 1.0);
}

@fragment
fn fs_grid(i: GridOut) -> GridFrag {
    let n4 = g.inv_view_proj * vec4(i.ndc, 0.0, 1.0);
    let f4 = g.inv_view_proj * vec4(i.ndc, 1.0, 1.0);
    let n = n4.xyz / n4.w;
    let f = f4.xyz / f4.w;
    let dir = f - n;
    if (abs(dir.y) < 1e-7) { discard; }
    let t = -n.y / dir.y;
    if (t < 0.0) { discard; }
    let hit = n + dir * t;
    let clip = g.view_proj * vec4(hit, 1.0);
    var out: GridFrag;
    out.depth = clip.z / clip.w;
    if (out.depth < 0.0 || out.depth > 1.0) { discard; }

    let eye_dist = length(g.eye.xyz - hit);
    let ortho_extent = max(length(dir), 1.0);
    let fade_dist = select(60.0 * g.params.z, 400.0, g.params.w > 0.5);
    let fade = 1.0 - clamp(eye_dist / fade_dist, 0.0, 1.0);

    let minor = grid_line(hit.xz, 0.1 * g.params.z) * 0.10;
    let mid = grid_line(hit.xz, 1.0 * g.params.z) * 0.22;
    let major = grid_line(hit.xz, 10.0 * g.params.z) * 0.38;
    var a = max(max(minor, mid), major);

    var rgb = vec3(0.62, 0.62, 0.66);
    let aw = fwidth(hit.xz);
    let ax = 1.0 - clamp(abs(hit.z) / max(aw.y, 1e-6), 0.0, 1.0); // eixo X (z = 0)
    let az = 1.0 - clamp(abs(hit.x) / max(aw.x, 1e-6), 0.0, 1.0); // eixo Z (x = 0)
    if (ax > 0.0) { rgb = mix(rgb, vec3(0.90, 0.25, 0.28), ax); a = max(a, ax * 0.85); }
    if (az > 0.0) { rgb = mix(rgb, vec3(0.30, 0.52, 0.95), az); a = max(a, az * 0.85); }

    out.color = vec4(rgb, a * fade);
    return out;
}
