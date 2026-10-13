//! Bevel de arestas e de vértices (equivalente ao `Ctrl+B` do Blender).
//!
//! # Modelo
//! Cada vértice tocado é visto como um "leque" (*fan*) de arestas e faces ao
//! redor dele. Para cada face do leque calcula-se o ponto onde as retas
//! deslocadas das duas arestas vizinhas se cruzam (`P = V + (w_ant·a + w_pos·b)/sin θ`);
//! arestas chanfradas viram uma faixa de `segments` quads entre os pontos das
//! duas faces vizinhas (perfil superelíptico, `profile = 0.5` é um arco
//! circular), arestas não chanfradas apenas encurtam até um ponto único, e o
//! vértice original é substituído por um *patch* (polígono, ou quads em torno
//! de um vértice central quando tem mais de 4 lados).
//!
//! # Suporte e limitações (documentado, falha explícita em vez de lixo)
//! * Arestas devem ter exatamente 2 faces. Arestas de fronteira, soltas ou
//!   não-manifold → `UnsupportedTopology`.
//! * Todo vértice tocado precisa ter um leque manifold simples (aberto, se está
//!   na fronteira da malha, ou fechado), enrolamento consistente, sem arestas
//!   soltas e sem faces que visitem o vértice duas vezes; caso contrário
//!   `UnsupportedTopology`.
//! * Larguras são medidas sobre as faces; o `clamp_overlap` reduz **todas** as
//!   larguras por um fator comum (não localmente como o Blender) para que os
//!   deslocamentos de uma mesma aresta não se cruzem.
//! * O patch de vértice com todas as arestas chanfradas e `segments >= 2` usa um
//!   vértice central (quads, e 1 triângulo quando o contorno tem nº ímpar de
//!   lados); não é o grid regular de cantos do Blender.
//! * Atributos das arestas chanfradas (sharp/seam/crease/bevel_weight) são
//!   descartados; os das demais arestas acompanham o encurtamento.
//! * Faixas e patches usam `smooth` da face vizinha, ou `true` se `segments > 1`.

use super::extrude::{
    face_pts_uvs, prune_orphan_edge_attrs, unique_corner_index, uv_at_shifted, wire_edge_keys,
};
use crate::error::{ModelError, ModelResult};
use crate::mesh::{edge_key, Corner, EVert, EdgeAttr, EdgeKey, EditMesh, Face};
use crate::ops::OpOutput;
use crate::select::Selection;
use crate::topology::Topology;
use glam::{Vec2, Vec3};
use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::f32::consts::{FRAC_1_SQRT_2, FRAC_PI_2, SQRT_2};

/// Máximo de segmentos por bevel.
pub const MAX_SEGMENTS: u32 = 64;

// ---------------------------------------------------------------- estruturas

#[derive(Clone, Copy, Debug)]
struct Pt {
    id: u32,
    pos: Vec3,
}

/// Leque de arestas/faces ao redor de um vértice.
#[derive(Clone, Debug)]
struct Fan {
    v: u32,
    closed: bool,
    /// vértice remoto de cada aresta
    far: Vec<u32>,
    /// `faces[i]` fica entre `far[i]` (anterior) e `far[i+1]` (posterior)
    faces: Vec<u32>,
}

impl Fan {
    fn k(&self) -> usize {
        self.far.len()
    }
    fn edge_index(&self, far: u32) -> Option<usize> {
        self.far.iter().position(|&f| f == far)
    }
    /// face antes da aresta `j` (a face onde `j` é a aresta posterior)
    fn face_before(&self, j: usize) -> Option<usize> {
        if j > 0 {
            Some(j - 1)
        } else if self.closed {
            Some(self.k() - 1)
        } else {
            None
        }
    }
    /// face depois da aresta `j` (onde `j` é a aresta anterior)
    fn face_after(&self, j: usize) -> Option<usize> {
        if j < self.faces.len() {
            Some(j)
        } else {
            None
        }
    }
}

struct FanGeom {
    u: Vec<Vec3>,
    len: Vec<f32>,
    s: Vec<f32>,
    nrm: Vec<Vec3>,
}

struct Plan {
    fan: Fan,
    /// pontos que substituem o corner do vértice em cada face do leque
    corner: Vec<Vec<Pt>>,
    /// arco do perfil de cada aresta chanfrada (de Lp a Rp, `segments + 1` pontos)
    arcs: Vec<Option<Vec<Pt>>>,
    /// contorno do patch na ordem do leque (vazio se aberto ou degenerado)
    patch: Vec<Pt>,
    center: Option<Pt>,
    /// anéis internos do bevel de vértice com `segments > 1`
    rings: Vec<Vec<Pt>>,
    /// vértice onde termina cada aresta não chanfrada (`None` nas chanfradas)
    edge_end: Vec<Option<u32>>,
}

struct Builder {
    old_n: usize,
    new_verts: Vec<EVert>,
    overwrite: Vec<(u32, EVert)>,
}

impl Builder {
    fn alloc(&mut self, v: u32, slot_used: &mut bool, ev: EVert) -> u32 {
        if !*slot_used {
            *slot_used = true;
            self.overwrite.push((v, ev));
            v
        } else {
            let id = (self.old_n + self.new_verts.len()) as u32;
            self.new_verts.push(ev);
            id
        }
    }
}

fn unsupported(v: u32, msg: &str) -> ModelError {
    ModelError::UnsupportedTopology(format!("bevel: vértice {v}: {msg}"))
}

fn prev_next(mesh: &EditMesh, f: u32, v: u32) -> Option<(u32, u32)> {
    let face = &mesh.faces[f as usize];
    let i = unique_corner_index(face, v)?;
    let n = face.corners.len();
    Some((
        face.corners[(i + n - 1) % n].vert,
        face.corners[(i + 1) % n].vert,
    ))
}

fn build_fan(mesh: &EditMesh, topo: &Topology, v: u32) -> ModelResult<Fan> {
    let inc = &topo.vert_edges[v as usize];
    if inc.len() < 2 {
        return Err(unsupported(v, "grau menor que 2"));
    }
    let mut border: Vec<u32> = Vec::new();
    let mut fars: Vec<u32> = Vec::new();
    for &ei in inc {
        let e = &topo.edges[ei as usize];
        let far = if e.verts.0 == v { e.verts.1 } else { e.verts.0 };
        match e.faces.len() {
            1 => border.push(far),
            2 => {}
            0 => return Err(unsupported(v, "aresta solta incidente")),
            _ => return Err(unsupported(v, "aresta não-manifold incidente")),
        }
        fars.push(far);
    }
    border.sort_unstable();
    let closed = border.is_empty();
    if !closed && border.len() != 2 {
        return Err(unsupported(
            v,
            "fronteira em pinça (vértice com mais de uma borda)",
        ));
    }
    let (start_far, start_face) = if closed {
        let far0 = fars
            .iter()
            .copied()
            .min()
            .ok_or_else(|| unsupported(v, "sem arestas"))?;
        let cands: Vec<u32> = topo
            .edge_faces(v, far0)
            .iter()
            .copied()
            .filter(|&f| prev_next(mesh, f, v).is_some_and(|(p, _)| p == far0))
            .collect();
        if cands.len() != 1 {
            return Err(unsupported(
                v,
                "enrolamento inconsistente das faces (use Recalculate Outside) ou face que visita o vértice duas vezes",
            ));
        }
        (far0, cands[0])
    } else {
        let mut found = None;
        for &b in &border {
            if let Some(&f) = topo.edge_faces(v, b).first() {
                if prev_next(mesh, f, v).is_some_and(|(p, _)| p == b) {
                    found = Some((b, f));
                    break;
                }
            }
        }
        found.ok_or_else(|| unsupported(v, "enrolamento inconsistente na borda"))?
    };
    let mut far: Vec<u32> = Vec::new();
    let mut faces: Vec<u32> = Vec::new();
    let (mut cur_far, mut cur_face) = (start_far, start_face);
    loop {
        far.push(cur_far);
        faces.push(cur_face);
        if far.len() > inc.len() {
            return Err(unsupported(v, "leque não fecha (topologia irregular)"));
        }
        let (p, n) = prev_next(mesh, cur_face, v)
            .ok_or_else(|| unsupported(v, "face visita o vértice mais de uma vez"))?;
        if p != cur_far {
            return Err(unsupported(
                v,
                "enrolamento inconsistente das faces vizinhas",
            ));
        }
        if closed && n == far[0] {
            break;
        }
        let ef = topo.edge_faces(v, n);
        if ef.len() == 1 {
            far.push(n);
            break;
        }
        if ef.len() != 2 {
            return Err(unsupported(v, "aresta não-manifold no leque"));
        }
        let other = if ef[0] == cur_face { ef[1] } else { ef[0] };
        if other == cur_face {
            return Err(unsupported(v, "aresta com a mesma face dos dois lados"));
        }
        cur_far = n;
        cur_face = other;
    }
    if far.len() != inc.len() {
        return Err(unsupported(
            v,
            "faces do vértice não formam um leque simples (pinça)",
        ));
    }
    let uniq: BTreeSet<u32> = far.iter().copied().collect();
    if uniq.len() != far.len() {
        return Err(unsupported(v, "arestas repetidas no leque"));
    }
    Ok(Fan {
        v,
        closed,
        far,
        faces,
    })
}

fn fan_geom(mesh: &EditMesh, pos: &[Vec3], fan: &Fan) -> ModelResult<FanGeom> {
    let k = fan.k();
    let v = fan.v;
    let mut u = Vec::with_capacity(k);
    let mut len = Vec::with_capacity(k);
    for &f in &fan.far {
        let d = pos[f as usize] - pos[v as usize];
        let l = d.length();
        if !(l > 1e-9) || !l.is_finite() {
            return Err(unsupported(v, "aresta de comprimento zero ou não finito"));
        }
        u.push(d / l);
        len.push(l);
    }
    let mut s = Vec::new();
    let mut nrm = Vec::new();
    for (i, &f) in fan.faces.iter().enumerate() {
        let n = mesh.face_normal(f);
        if n == Vec3::ZERO {
            return Err(unsupported(v, "face degenerada (normal nula)"));
        }
        let j = (i + 1) % k;
        s.push(u[j].cross(u[i]).dot(n));
        nrm.push(n);
    }
    Ok(FanGeom { u, len, s, nrm })
}

/// Deslocamento `P - V` para a face `i` do leque, dadas as larguras das arestas.
fn face_offset(fan: &Fan, g: &FanGeom, w: &[f32], i: usize) -> Vec3 {
    let k = fan.k();
    let (jp, jn) = (i, (i + 1) % k);
    let (wp, wn) = (w[jp], w[jn]);
    let wmax = wp.max(wn);
    if wmax <= 0.0 {
        return Vec3::ZERO;
    }
    let (a, b) = (g.u[jn], g.u[jp]);
    let raw = if g.s[i].abs() < 1e-3 {
        g.nrm[i].cross(a) * (wp + wn) * 0.5
    } else {
        (a * wp + b * wn) / g.s[i]
    };
    let max_len = 20.0 * wmax;
    if raw.length() > max_len {
        raw.normalize_or_zero() * max_len
    } else {
        raw
    }
}

/// Expoente da superelipse e fator de "bojo" a partir do parâmetro de perfil.
fn profile_params(profile: f32) -> (f32, f32) {
    let p = profile.clamp(0.0, 1.0);
    let q = if p <= 0.5 {
        SQRT_2 * p
    } else {
        FRAC_1_SQRT_2 + (p - 0.5) * 2.0 * (1.0 - FRAC_1_SQRT_2)
    };
    let q = q.clamp(0.05, 0.9999);
    let n = (-(2.0f32).ln() / q.ln()).clamp(0.2, 40.0);
    let kappa = (2.0 * q - 1.0).clamp(0.0, 1.0);
    (n, kappa)
}

/// Pontos do perfil de `lp` a `rp` em torno do ápice `m` (`segs + 1` pontos).
fn arc_points(lp: Vec3, rp: Vec3, m: Vec3, segs: usize, profile: f32) -> Vec<Vec3> {
    if segs <= 1 {
        return vec![lp, rp];
    }
    let (n, _) = profile_params(profile);
    let e = 2.0 / n;
    let mut out = Vec::with_capacity(segs + 1);
    for k in 0..=segs {
        if k == 0 {
            out.push(lp);
            continue;
        }
        if k == segs {
            out.push(rp);
            continue;
        }
        let phi = FRAC_PI_2 * k as f32 / segs as f32;
        let x = 1.0 - phi.sin().powf(e);
        let y = 1.0 - phi.cos().powf(e);
        out.push(m + (lp - m) * x + (rp - m) * y);
    }
    out
}

fn dedup_loop(mut v: Vec<Pt>) -> Vec<Pt> {
    v.dedup_by(|a, b| a.id == b.id);
    while v.len() > 1 && v.first().map(|p| p.id) == v.last().map(|p| p.id) {
        v.pop();
    }
    v
}

fn mean_pos(pts: &[Pt]) -> Vec3 {
    if pts.is_empty() {
        return Vec3::ZERO;
    }
    pts.iter().map(|p| p.pos).sum::<Vec3>() / pts.len() as f32
}

// ------------------------------------------------------------ plano (arestas)

#[allow(clippy::too_many_arguments)]
fn plan_edge_vertex(
    mesh: &EditMesh,
    pos: &[Vec3],
    fan: Fan,
    g: &FanGeom,
    w: &[f32],
    segments: usize,
    profile: f32,
    b: &mut Builder,
) -> ModelResult<Plan> {
    let k = fan.k();
    let v = fan.v;
    let vp = pos[v as usize];
    let vev = &mesh.verts[v as usize];
    let bev = |j: usize| w[j] > 0.0;
    let nf = fan.faces.len();
    let offsets: Vec<Vec3> = (0..nf).map(|i| face_offset(&fan, g, w, i)).collect();
    let mut slot_used = false;

    // pontos das arestas não chanfradas
    let mut q: Vec<Option<Pt>> = vec![None; k];
    // arestas sem nenhum vizinho chanfrado: ficam no vértice original; cada
    // sequência contígua compartilha um ponto, e sequências separadas por
    // arestas chanfradas recebem pontos distintos (senão o vértice "belisca" a malha)
    let candidates = |j: usize| -> Vec<f32> {
        let mut ts: Vec<f32> = Vec::new();
        if let Some(i) = fan.face_before(j) {
            if bev((j + k - 1) % k) {
                ts.push(offsets[i].dot(g.u[j]));
            }
        }
        if let Some(i) = fan.face_after(j) {
            if bev((j + 1) % k) {
                ts.push(offsets[i].dot(g.u[j]));
            }
        }
        ts
    };
    let none: Vec<bool> = (0..k)
        .map(|j| !bev(j) && candidates(j).is_empty())
        .collect();
    let mut run: Vec<usize> = vec![usize::MAX; k];
    let mut n_runs = 0;
    for j in 0..k {
        if !none[j] {
            continue;
        }
        if j > 0 && none[j - 1] {
            run[j] = run[j - 1];
        } else {
            run[j] = n_runs;
            n_runs += 1;
        }
    }
    if fan.closed && k > 1 && none[0] && none[k - 1] && run[0] != run[k - 1] {
        let (from, to) = (run[k - 1], run[0]);
        for r in run.iter_mut() {
            if *r == from {
                *r = to;
            }
        }
    }
    if fan.closed && n_runs >= 2 {
        return Err(unsupported(
            v,
            "arestas chanfradas não adjacentes separam grupos de arestas não chanfradas (ex.: polo de valência >= 6); o vértice ficaria \"beliscado\" — chanfre também as arestas intermediárias",
        ));
    }
    let mut run_pt: BTreeMap<usize, Pt> = BTreeMap::new();
    for j in 0..k {
        if !none[j] {
            continue;
        }
        let pt = *run_pt.entry(run[j]).or_insert_with(|| {
            let id = b.alloc(v, &mut slot_used, EVert::derived(vev, vp));
            Pt { id, pos: vp }
        });
        q[j] = Some(pt);
    }
    for j in 0..k {
        if bev(j) || q[j].is_some() {
            continue;
        }
        let mut ts: Vec<f32> = Vec::new();
        if let Some(i) = fan.face_before(j) {
            if bev((j + k - 1) % k) {
                ts.push(offsets[i].dot(g.u[j]));
            }
        }
        if let Some(i) = fan.face_after(j) {
            if bev((j + 1) % k) {
                ts.push(offsets[i].dot(g.u[j]));
            }
        }
        let t = (ts.iter().sum::<f32>() / ts.len() as f32).clamp(0.0, g.len[j]);
        let far_ev = &mesh.verts[fan.far[j] as usize];
        let ev = EVert::lerp(vev, far_ev, t / g.len[j]);
        let p = vp + g.u[j] * t;
        let id = b.alloc(v, &mut slot_used, ev);
        q[j] = Some(Pt { id, pos: p });
    }
    // pontos por face
    let mut corner: Vec<Vec<Pt>> = Vec::with_capacity(nf);
    for i in 0..nf {
        let (ep, en) = (i, (i + 1) % k);
        let pts = match (bev(ep), bev(en)) {
            (true, true) => {
                let p = vp + offsets[i];
                let ev = EVert::derived(vev, p);
                let id = b.alloc(v, &mut slot_used, ev);
                vec![Pt { id, pos: p }]
            }
            (true, false) => vec![q[en].unwrap_or(Pt { id: v, pos: vp })],
            (false, true) => vec![q[ep].unwrap_or(Pt { id: v, pos: vp })],
            (false, false) => dedup_loop(vec![
                q[ep].unwrap_or(Pt { id: v, pos: vp }),
                q[en].unwrap_or(Pt { id: v, pos: vp }),
            ]),
        };
        corner.push(pts);
    }
    // arcos
    let mut arcs: Vec<Option<Vec<Pt>>> = vec![None; k];
    for j in 0..k {
        if !bev(j) {
            continue;
        }
        let (Some(fb), Some(fa)) = (fan.face_before(j), fan.face_after(j)) else {
            continue; // arestas de fronteira nunca são chanfradas (validado antes)
        };
        let lp = corner[fb][0];
        let rp = corner[fa][0];
        let t_l = (lp.pos - vp).dot(g.u[j]);
        let t_r = (rp.pos - vp).dot(g.u[j]);
        let m = vp + g.u[j] * (0.5 * (t_l + t_r));
        let positions = arc_points(lp.pos, rp.pos, m, segments, profile);
        let mut arc = Vec::with_capacity(segments + 1);
        for (idx, p) in positions.iter().enumerate() {
            if idx == 0 {
                arc.push(lp);
            } else if idx == segments {
                arc.push(rp);
            } else {
                let ev = EVert::derived(vev, *p);
                let id = b.alloc(v, &mut slot_used, ev);
                arc.push(Pt { id, pos: *p });
            }
        }
        arcs[j] = Some(arc);
    }
    // contorno do patch
    let mut patch: Vec<Pt> = Vec::new();
    let mut center = None;
    if fan.closed {
        let mut lp: Vec<Pt> = Vec::new();
        for j in 0..k {
            if bev(j) {
                if let Some(arc) = &arcs[j] {
                    lp.extend(arc.iter().copied());
                }
            } else if let Some(p) = q[j] {
                lp.push(p);
            }
        }
        let lp = dedup_loop(lp);
        if lp.len() >= 3 {
            if lp.len() > 4 {
                let gpos = mean_pos(&lp);
                // cantos totalmente chanfrados abaulam em direção ao vértice original;
                // os demais patches (faixa entre arcos e uma aresta reta) ficam planos
                let all_bev = (0..k).all(bev);
                let kappa = if all_bev {
                    profile_params(profile).1
                } else {
                    0.0
                };
                let c = gpos + (vp - gpos) * kappa;
                let ev = EVert::derived(vev, c);
                let id = b.alloc(v, &mut slot_used, ev);
                center = Some(Pt { id, pos: c });
            }
            patch = lp;
        }
    }
    let edge_end = q.iter().map(|o| o.map(|p| p.id)).collect();
    Ok(Plan {
        fan,
        corner,
        arcs,
        patch,
        center,
        rings: Vec::new(),
        edge_end,
    })
}

// ------------------------------------------------------- reconstrução comum

struct Assembled {
    replaced: Vec<(u32, Face)>,
    uv_of: HashMap<u32, Vec2>,
}

/// Reescreve as faces tocadas pelos planos (corners dos vértices chanfrados são
/// substituídos pelos pontos do plano, com UV deslocada afim).
fn rewrite_faces(mesh: &EditMesh, pos: &[Vec3], plans: &BTreeMap<u32, Plan>) -> Assembled {
    let mut touched: BTreeSet<u32> = BTreeSet::new();
    for p in plans.values() {
        touched.extend(p.fan.faces.iter().copied());
    }
    let mut replaced = Vec::with_capacity(touched.len());
    let mut uv_of: HashMap<u32, Vec2> = HashMap::new();
    for &f in &touched {
        let old = &mesh.faces[f as usize];
        let (pts, uvs) = face_pts_uvs(pos, old);
        let mut corners: Vec<Corner> = Vec::with_capacity(old.corners.len() + 2);
        for (ci, c) in old.corners.iter().enumerate() {
            match plans.get(&c.vert) {
                Some(plan) => {
                    let Some(i) = plan.fan.faces.iter().position(|&g| g == f) else {
                        corners.push(*c);
                        continue;
                    };
                    for pt in &plan.corner[i] {
                        let uv = uv_at_shifted(&pts, &uvs, ci, pt.pos);
                        uv_of.entry(pt.id).or_insert(uv);
                        corners.push(Corner {
                            vert: pt.id,
                            uv,
                            normal: c.normal,
                        });
                    }
                }
                None => corners.push(*c),
            }
        }
        let mut nf = old.clone();
        nf.corners = corners;
        replaced.push((f, nf));
    }
    Assembled { replaced, uv_of }
}

/// UV que o ponto `pos` teria dentro da face `f`, no corner de `x`.
fn shifted_uv(mesh: &EditMesh, pos: &[Vec3], f: u32, x: u32, p: Vec3) -> Vec2 {
    let face = &mesh.faces[f as usize];
    let (pts, uvs) = face_pts_uvs(pos, face);
    match unique_corner_index(face, x) {
        Some(ci) => uv_at_shifted(&pts, &uvs, ci, p),
        None => Vec2::ZERO,
    }
}

fn make_face(template: &Face, verts: &[u32], uvs: &[Vec2], smooth: bool) -> Face {
    let mut f = template.with_verts_from(verts);
    f.smooth = smooth;
    for (c, uv) in f.corners.iter_mut().zip(uvs) {
        c.uv = *uv;
    }
    f
}

/// Gera as faces do patch (polígono único ou quads em torno do centro).
/// `uv_of` fornece a UV de cada vértice do contorno.
fn patch_faces(
    plan: &Plan,
    template: &Face,
    smooth: bool,
    uv_of: &mut HashMap<u32, Vec2>,
) -> Vec<Face> {
    let mut out = Vec::new();
    // ordem reversa do leque para oposição às faixas/faces vizinhas
    let b: Vec<Pt> = plan.patch.iter().rev().copied().collect();
    let n = b.len();
    if n < 3 {
        return out;
    }
    let uv = |m: &HashMap<u32, Vec2>, p: &Pt| m.get(&p.id).copied().unwrap_or(Vec2::ZERO);
    match plan.center {
        None => {
            let verts: Vec<u32> = b.iter().map(|p| p.id).collect();
            let uvs: Vec<Vec2> = b.iter().map(|p| uv(uv_of, p)).collect();
            out.push(make_face(template, &verts, &uvs, smooth));
        }
        Some(c) => {
            let cuv = b.iter().map(|p| uv(uv_of, p)).sum::<Vec2>() / n as f32;
            uv_of.entry(c.id).or_insert(cuv);
            let quads = n / 2;
            for j in 0..quads {
                let ids = [c, b[2 * j], b[2 * j + 1], b[(2 * j + 2) % n]];
                let verts: Vec<u32> = ids.iter().map(|p| p.id).collect();
                let uvs: Vec<Vec2> = ids.iter().map(|p| uv(uv_of, p)).collect();
                out.push(make_face(template, &verts, &uvs, smooth));
            }
            if n % 2 == 1 {
                let ids = [c, b[n - 1], b[0]];
                let verts: Vec<u32> = ids.iter().map(|p| p.id).collect();
                let uvs: Vec<Vec2> = ids.iter().map(|p| uv(uv_of, p)).collect();
                out.push(make_face(template, &verts, &uvs, smooth));
            }
        }
    }
    out
}

fn finite_check(b: &Builder, plans: &BTreeMap<u32, Plan>) -> ModelResult<()> {
    let bad = b.new_verts.iter().any(|v| !v.co.is_finite())
        || b.overwrite.iter().any(|(_, v)| !v.co.is_finite())
        || plans
            .values()
            .any(|p| p.corner.iter().flatten().any(|pt| !pt.pos.is_finite()));
    if bad {
        return Err(ModelError::Numeric(
            "bevel produziu coordenadas não finitas (geometria degenerada)".into(),
        ));
    }
    Ok(())
}

// ------------------------------------------------------------- bevel arestas

fn bevel_impl(
    mesh: &mut EditMesh,
    widths: BTreeMap<EdgeKey, f32>,
    segments: u32,
    profile: f32,
    clamp_overlap: bool,
) -> ModelResult<OpOutput> {
    let topo = Topology::build(mesh);
    let wires = wire_edge_keys(mesh);
    // ---- validação das arestas
    for &(a, b) in widths.keys() {
        for v in [a, b] {
            if v as usize >= mesh.verts.len() {
                return Err(ModelError::VertexOutOfRange(v, mesh.verts.len()));
            }
        }
        let Some(e) = topo.edge(a, b) else {
            return Err(ModelError::MissingEdge(a, b));
        };
        match e.faces.len() {
            2 => {}
            0 => {
                return Err(ModelError::UnsupportedTopology(format!(
                    "bevel: aresta ({a}, {b}) é solta (sem faces)"
                )))
            }
            1 => {
                return Err(ModelError::UnsupportedTopology(format!(
                    "bevel: aresta ({a}, {b}) é de fronteira (uma só face); apenas arestas internas podem ser chanfradas"
                )))
            }
            n => {
                return Err(ModelError::UnsupportedTopology(format!(
                    "bevel: aresta ({a}, {b}) é não-manifold ({n} faces)"
                )))
            }
        }
    }
    let pos: Vec<Vec3> = mesh.verts.iter().map(|v| v.co).collect();
    let bev_verts: BTreeSet<u32> = widths.keys().flat_map(|&(a, b)| [a, b]).collect();
    let mut fans: BTreeMap<u32, (Fan, FanGeom)> = BTreeMap::new();
    for &v in &bev_verts {
        let fan = build_fan(mesh, &topo, v)?;
        let g = fan_geom(mesh, &pos, &fan)?;
        fans.insert(v, (fan, g));
    }
    let width_of =
        |v: u32, far: u32| -> f32 { widths.get(&edge_key(v, far)).copied().unwrap_or(0.0) };

    // ---- clamp global
    let mut scale = 1.0f32;
    if clamp_overlap {
        let mut along: HashMap<EdgeKey, f32> = HashMap::new();
        for (&v, (fan, g)) in &fans {
            let k = fan.k();
            let w: Vec<f32> = fan.far.iter().map(|&f| width_of(v, f)).collect();
            let offs: Vec<Vec3> = (0..fan.faces.len())
                .map(|i| face_offset(fan, g, &w, i))
                .collect();
            for j in 0..k {
                let mut ts = Vec::new();
                if let Some(i) = fan.face_before(j) {
                    ts.push(offs[i].dot(g.u[j]));
                }
                if let Some(i) = fan.face_after(j) {
                    ts.push(offs[i].dot(g.u[j]));
                }
                let t = (ts.iter().sum::<f32>() / ts.len().max(1) as f32).max(0.0);
                *along.entry(edge_key(v, fan.far[j])).or_insert(0.0) += t;
            }
        }
        for (&(a, b), &sum) in &along {
            let l = pos[a as usize].distance(pos[b as usize]);
            if sum > 0.98 * l && sum > 0.0 {
                scale = scale.min(0.98 * l / sum);
            }
        }
    }

    // ---- planos
    let mut builder = Builder {
        old_n: mesh.verts.len(),
        new_verts: Vec::new(),
        overwrite: Vec::new(),
    };
    let mut plans: BTreeMap<u32, Plan> = BTreeMap::new();
    for (v, (fan, g)) in fans {
        let w: Vec<f32> = fan.far.iter().map(|&f| width_of(v, f) * scale).collect();
        let plan = plan_edge_vertex(
            mesh,
            &pos,
            fan,
            &g,
            &w,
            segments as usize,
            profile,
            &mut builder,
        )?;
        plans.insert(v, plan);
    }
    finite_check(&builder, &plans)?;

    // ---- faces reescritas
    let Assembled {
        replaced,
        mut uv_of,
    } = rewrite_faces(mesh, &pos, &plans);

    // ---- faixas
    let s = segments as usize;
    let mut new_faces: Vec<Face> = Vec::new();
    for &(a, b) in widths.keys() {
        let (pa, pb) = (&plans[&a], &plans[&b]);
        let (Some(ja), Some(jb)) = (pa.fan.edge_index(b), pb.fan.edge_index(a)) else {
            return Err(ModelError::UnsupportedTopology(format!(
                "bevel: aresta ({a}, {b}) ausente do leque"
            )));
        };
        let (Some(arc_a), Some(arc_b)) = (pa.arcs[ja].as_ref(), pb.arcs[jb].as_ref()) else {
            return Err(ModelError::UnsupportedTopology(format!(
                "bevel: aresta ({a}, {b}) sem perfil"
            )));
        };
        let (Some(fl), Some(fr)) = (pa.fan.face_before(ja), pa.fan.face_after(ja)) else {
            return Err(ModelError::UnsupportedTopology(format!(
                "bevel: aresta ({a}, {b}) sem duas faces"
            )));
        };
        let (face_l, face_r) = (pa.fan.faces[fl], pa.fan.faces[fr]);
        let uv_la = shifted_uv(mesh, &pos, face_l, a, arc_a[0].pos);
        let uv_ra = shifted_uv(mesh, &pos, face_r, a, arc_a[s].pos);
        let uv_lb = shifted_uv(mesh, &pos, face_l, b, arc_b[s].pos);
        let uv_rb = shifted_uv(mesh, &pos, face_r, b, arc_b[0].pos);
        let template = mesh.faces[face_l as usize].clone();
        let smooth = template.smooth || s > 1;
        let pa_t = |t: usize| arc_a[t];
        let pb_t = |t: usize| arc_b[s - t];
        let uva = |t: usize| uv_la.lerp(uv_ra, t as f32 / s as f32);
        let uvb = |t: usize| uv_lb.lerp(uv_rb, t as f32 / s as f32);
        for t in 0..s {
            let ids = [pb_t(t).id, pa_t(t).id, pa_t(t + 1).id, pb_t(t + 1).id];
            let uvs = [uvb(t), uva(t), uva(t + 1), uvb(t + 1)];
            new_faces.push(make_face(&template, &ids, &uvs, smooth));
        }
        for t in 1..s {
            uv_of.entry(pa_t(t).id).or_insert(uva(t));
            uv_of.entry(pb_t(t).id).or_insert(uvb(t));
        }
    }
    // ---- patches
    for plan in plans.values() {
        if plan.patch.len() < 3 {
            continue;
        }
        let f0 = plan.fan.faces[0];
        let template = mesh.faces[f0 as usize].clone();
        let smooth = template.smooth || s > 1;
        let faces = patch_faces(plan, &template, smooth, &mut uv_of);
        new_faces.extend(faces);
    }

    // ---- atributos de aresta das não-chanfradas que encurtaram
    let end_of = |plans: &BTreeMap<u32, Plan>, v: u32, far: u32| -> u32 {
        match plans.get(&v) {
            Some(p) => p
                .fan
                .edge_index(far)
                .and_then(|j| p.edge_end[j])
                .unwrap_or(v),
            None => v,
        }
    };
    let mut attr_moves: Vec<(EdgeKey, EdgeAttr)> = Vec::new();
    for (&(a, b), &attr) in mesh.edge_attrs.iter() {
        if widths.contains_key(&(a, b)) {
            continue;
        }
        if !plans.contains_key(&a) && !plans.contains_key(&b) {
            continue;
        }
        let (na, nb) = (end_of(&plans, a, b), end_of(&plans, b, a));
        if (na, nb) != (a, b) && na != nb {
            attr_moves.push((edge_key(na, nb), attr));
        }
    }

    // ---- commit
    for (id, ev) in builder.overwrite {
        mesh.verts[id as usize] = ev;
    }
    let created_verts: Vec<u32> =
        (builder.old_n as u32..(builder.old_n + builder.new_verts.len()) as u32).collect();
    mesh.verts.extend(builder.new_verts);
    for (f, face) in replaced {
        mesh.faces[f as usize] = face;
    }
    let first_new = mesh.faces.len() as u32;
    mesh.faces.extend(new_faces);
    let created_faces: Vec<u32> = (first_new..mesh.faces.len() as u32).collect();
    // remove atributos das arestas chanfradas e move os das encurtadas
    for k in widths.keys() {
        mesh.edge_attrs.remove(k);
    }
    for (k, a) in attr_moves {
        mesh.edge_attrs.insert(k, a);
    }
    prune_orphan_edge_attrs(mesh, &wires);
    Ok(OpOutput {
        selection: Selection::from_faces(mesh, created_faces.iter().copied()),
        created_verts,
        created_faces,
        remap: None,
    })
}

fn check_bevel_params(width: f32, segments: u32, profile: f32) -> ModelResult<()> {
    if !width.is_finite() || width <= 0.0 {
        return Err(ModelError::InvalidParameter(format!(
            "largura do bevel deve ser finita e > 0 (recebeu {width})"
        )));
    }
    if segments == 0 || segments > MAX_SEGMENTS {
        return Err(ModelError::InvalidParameter(format!(
            "segments deve estar em 1..={MAX_SEGMENTS} (recebeu {segments})"
        )));
    }
    if !profile.is_finite() || !(0.0..=1.0).contains(&profile) {
        return Err(ModelError::InvalidParameter(format!(
            "profile deve estar em [0, 1] (recebeu {profile})"
        )));
    }
    Ok(())
}

/// Bevel de arestas.
///
/// * `edges`: arestas internas (2 faces) a chanfrar; duplicatas são ignoradas.
/// * `width`: largura em unidades, medida sobre as faces adjacentes a partir
///   da aresta original.
/// * `segments`: divisões do perfil (>= 1); `1` = chanfro reto.
/// * `profile`: `0..=1`, `0.5` = arco circular, maior = mais convexo (tende a
///   um canto vivo em `1.0`), menor = mais plano/côncavo.
/// * `clamp_overlap`: reduz a largura para evitar que os deslocamentos de uma
///   mesma aresta se cruzem.
///
/// Vértices de qualquer valência são suportados quando o leque é manifold
/// (cantos de cubo: patch triangular; cilindros com tampa, esferas, toros e
/// bordas de malhas abertas — ver limitações no cabeçalho do módulo). A
/// topologia resultante preserva a característica de Euler e a orientação das
/// normais.
///
/// Seleção devolvida: as faces do chanfro (faixas e patches de canto).
pub fn bevel_edges(
    mesh: &mut EditMesh,
    edges: &[EdgeKey],
    width: f32,
    segments: u32,
    profile: f32,
    clamp_overlap: bool,
) -> ModelResult<OpOutput> {
    check_bevel_params(width, segments, profile)?;
    if edges.is_empty() {
        return Err(ModelError::EmptySelection(
            "bevel precisa de ao menos uma aresta",
        ));
    }
    let mut widths = BTreeMap::new();
    for &(a, b) in edges {
        widths.insert(edge_key(a, b), width);
    }
    bevel_impl(mesh, widths, segments, profile, clamp_overlap)
}

/// Bevel das arestas cujo `bevel_weight >= min_weight`, com largura
/// `width * bevel_weight` (como o modificador Bevel do Blender em modo *Weight*).
///
/// `min_weight` deve estar em `(0, 1]`. Arestas marcadas que não podem ser
/// chanfradas (fronteira ou soltas) são ignoradas; arestas não-manifold
/// retornam `UnsupportedTopology`. Sem nenhuma aresta elegível → `EmptySelection`.
pub fn bevel_edges_by_weight(
    mesh: &mut EditMesh,
    min_weight: f32,
    width: f32,
    segments: u32,
    profile: f32,
    clamp_overlap: bool,
) -> ModelResult<OpOutput> {
    check_bevel_params(width, segments, profile)?;
    if !min_weight.is_finite() || min_weight <= 0.0 || min_weight > 1.0 {
        return Err(ModelError::InvalidParameter(format!(
            "min_weight deve estar em (0, 1] (recebeu {min_weight})"
        )));
    }
    let topo = Topology::build(mesh);
    let mut widths = BTreeMap::new();
    for (&k, attr) in &mesh.edge_attrs {
        if attr.bevel_weight >= min_weight {
            match topo.edge(k.0, k.1).map(|e| e.faces.len()) {
                Some(2) => {
                    widths.insert(k, width * attr.bevel_weight.min(1.0));
                }
                Some(n) if n > 2 => {
                    return Err(ModelError::UnsupportedTopology(format!(
                        "bevel: aresta ({}, {}) com peso é não-manifold ({n} faces)",
                        k.0, k.1
                    )))
                }
                _ => {}
            }
        }
    }
    if widths.is_empty() {
        return Err(ModelError::EmptySelection(
            "nenhuma aresta interna com bevel_weight suficiente",
        ));
    }
    bevel_impl(mesh, widths, segments, profile, clamp_overlap)
}

// ------------------------------------------------------------- bevel vértices

/// Bevel de vértices: cada vértice é cortado por um polígono cujos vértices
/// ficam a `width` de distância ao longo de cada aresta incidente (limitado a
/// 49 % do comprimento da aresta).
///
/// Com `segments > 1` o polígono ganha anéis concêntricos que abaulam em
/// direção ao vértice original (aproximação cônica, não uma esfera perfeita).
/// Vértices de fronteira não recebem polígono (apenas o corte do canto).
/// Falha com `UnsupportedTopology` nos mesmos casos que [`bevel_edges`].
///
/// Seleção devolvida: os polígonos criados.
pub fn bevel_verts(
    mesh: &mut EditMesh,
    verts: &[u32],
    width: f32,
    segments: u32,
) -> ModelResult<OpOutput> {
    check_bevel_params(width, segments, 0.5)?;
    let verts = super::extrude::validated_verts(
        mesh,
        verts,
        "bevel de vértices precisa de ao menos um vértice",
    )?;
    let topo = Topology::build(mesh);
    let wires = wire_edge_keys(mesh);
    let pos: Vec<Vec3> = mesh.verts.iter().map(|v| v.co).collect();
    let mut builder = Builder {
        old_n: mesh.verts.len(),
        new_verts: Vec::new(),
        overwrite: Vec::new(),
    };
    let mut plans: BTreeMap<u32, Plan> = BTreeMap::new();
    let s = segments as usize;
    for &v in &verts {
        let fan = build_fan(mesh, &topo, v)?;
        let g = fan_geom(mesh, &pos, &fan)?;
        let k = fan.k();
        let vp = pos[v as usize];
        let vev = &mesh.verts[v as usize];
        let mut slot_used = false;
        let mut q: Vec<Pt> = Vec::with_capacity(k);
        for j in 0..k {
            let d = width.min(0.49 * g.len[j]);
            let far_ev = &mesh.verts[fan.far[j] as usize];
            let ev = EVert::lerp(vev, far_ev, d / g.len[j]);
            let p = vp + g.u[j] * d;
            let id = builder.alloc(v, &mut slot_used, ev);
            q.push(Pt { id, pos: p });
        }
        let nf = fan.faces.len();
        let corner: Vec<Vec<Pt>> = (0..nf).map(|i| vec![q[i], q[(i + 1) % k]]).collect();
        let mut patch = Vec::new();
        let mut rings = Vec::new();
        if fan.closed && k >= 3 {
            patch = q.clone();
            if s > 1 {
                let gpos = mean_pos(&q);
                let (_, kappa) = profile_params(0.5);
                let c = gpos + (vp - gpos) * kappa;
                for m in 1..s {
                    let t = m as f32 / s as f32;
                    let mut ring = Vec::with_capacity(k);
                    for p in &q {
                        let pp = p.pos + (c - p.pos) * t;
                        let id = builder.alloc(v, &mut slot_used, EVert::derived(vev, pp));
                        ring.push(Pt { id, pos: pp });
                    }
                    rings.push(ring);
                }
            }
        }
        plans.insert(
            v,
            Plan {
                fan,
                corner,
                arcs: Vec::new(),
                patch,
                center: None,
                rings,
                edge_end: q.iter().map(|p| Some(p.id)).collect(),
            },
        );
    }
    finite_check(&builder, &plans)?;
    let Assembled {
        replaced,
        mut uv_of,
    } = rewrite_faces(mesh, &pos, &plans);
    let mut new_faces: Vec<Face> = Vec::new();
    for plan in plans.values() {
        if plan.patch.len() < 3 {
            continue;
        }
        let template = mesh.faces[plan.fan.faces[0] as usize].clone();
        let smooth = template.smooth || s > 1;
        // anéis: loop externo = patch; internos = plan.rings
        let mut levels: Vec<Vec<Pt>> = vec![plan.patch.clone()];
        levels.extend(plan.rings.iter().cloned());
        let k = plan.patch.len();
        for (m, ring) in levels.iter().enumerate().skip(1) {
            for (i, p) in ring.iter().enumerate() {
                let base = uv_of.get(&levels[0][i].id).copied().unwrap_or(Vec2::ZERO);
                let _ = m;
                uv_of.entry(p.id).or_insert(base);
            }
        }
        let uv = |m: &HashMap<u32, Vec2>, p: &Pt| m.get(&p.id).copied().unwrap_or(Vec2::ZERO);
        for m in 0..levels.len() - 1 {
            for i in 0..k {
                let j = (i + 1) % k;
                let ids = [
                    levels[m][j],
                    levels[m][i],
                    levels[m + 1][i],
                    levels[m + 1][j],
                ];
                let verts: Vec<u32> = ids.iter().map(|p| p.id).collect();
                let uvs: Vec<Vec2> = ids.iter().map(|p| uv(&uv_of, p)).collect();
                new_faces.push(make_face(&template, &verts, &uvs, smooth));
            }
        }
        // tampa: último anel, em ordem reversa
        let last = &levels[levels.len() - 1];
        let ids: Vec<Pt> = last.iter().rev().copied().collect();
        let verts: Vec<u32> = ids.iter().map(|p| p.id).collect();
        let uvs: Vec<Vec2> = ids.iter().map(|p| uv(&uv_of, p)).collect();
        new_faces.push(make_face(&template, &verts, &uvs, smooth));
    }
    // atributos de aresta: arestas encurtadas acompanham o novo extremo
    let mut attr_moves: Vec<(EdgeKey, EdgeAttr)> = Vec::new();
    let end_of = |v: u32, far: u32| -> u32 {
        match plans.get(&v) {
            Some(p) => p
                .fan
                .edge_index(far)
                .and_then(|j| p.edge_end[j])
                .unwrap_or(v),
            None => v,
        }
    };
    for (&(a, b), &attr) in mesh.edge_attrs.iter() {
        if !plans.contains_key(&a) && !plans.contains_key(&b) {
            continue;
        }
        let (na, nb) = (end_of(a, b), end_of(b, a));
        if (na, nb) != (a, b) && na != nb {
            attr_moves.push((edge_key(na, nb), attr));
        }
    }
    for (id, ev) in builder.overwrite {
        mesh.verts[id as usize] = ev;
    }
    let created_verts: Vec<u32> =
        (builder.old_n as u32..(builder.old_n + builder.new_verts.len()) as u32).collect();
    mesh.verts.extend(builder.new_verts);
    for (f, face) in replaced {
        mesh.faces[f as usize] = face;
    }
    let first_new = mesh.faces.len() as u32;
    mesh.faces.extend(new_faces);
    let created_faces: Vec<u32> = (first_new..mesh.faces.len() as u32).collect();
    for (k, a) in attr_moves {
        mesh.edge_attrs.insert(k, a);
    }
    prune_orphan_edge_attrs(mesh, &wires);
    Ok(OpOutput {
        selection: Selection::from_faces(mesh, created_faces.iter().copied()),
        created_verts,
        created_faces,
        remap: None,
    })
}

// -------------------------------------------------------------------- testes

#[cfg(test)]
mod tests {
    use super::super::bisect::bisect;
    use super::super::extrude::test_util::*;
    use super::*;
    use crate::primitives;
    use std::collections::HashSet;

    fn assert_consistent_orientation(m: &EditMesh) {
        let mut seen = HashSet::new();
        for f in &m.faces {
            for (a, b) in f.edges() {
                assert!(
                    seen.insert((a, b)),
                    "enrolamento inconsistente em ({a},{b})"
                );
            }
        }
    }

    fn all_edges(m: &EditMesh) -> Vec<EdgeKey> {
        Topology::build(m).edges.iter().map(|e| e.verts).collect()
    }

    fn polytope_volume_cube_chamfer(a: f32, w: f32) -> f32 {
        // cubo [-a/2, a/2]^3 recortado por 12 planos de aresta e 8 planos de canto (via bisect)
        let mut m = primitives::cube(a);
        let h = a / 2.0;
        let mut planes: Vec<(Vec3, f32)> = Vec::new();
        for sx in [-1.0f32, 1.0] {
            for sy in [-1.0f32, 1.0] {
                planes.push((Vec3::new(sx, sy, 0.0), a - w)); // arestas ao longo de z
                planes.push((Vec3::new(sx, 0.0, sy), a - w));
                planes.push((Vec3::new(0.0, sx, sy), a - w));
            }
        }
        // x + y = h + h - w  => normal (1,1,0)/sqrt2 com distância (a - w)/sqrt2
        let mut vol_planes: Vec<(Vec3, Vec3)> = Vec::new();
        for (n, c) in &planes {
            let nn = n.normalize();
            vol_planes.push((nn, nn * (c / n.length())));
        }
        for sx in [-1.0f32, 1.0] {
            for sy in [-1.0f32, 1.0] {
                for sz in [-1.0f32, 1.0] {
                    let n = Vec3::new(sx, sy, sz);
                    let corner_pt = Vec3::new(sx * (h - w), sy * (h - w), sz * h);
                    let nn = n.normalize();
                    vol_planes.push((nn, nn * nn.dot(corner_pt)));
                }
            }
        }
        for (n, p) in vol_planes {
            bisect(&mut m, p, n, false, true, true, 1e-7).unwrap();
        }
        signed_volume(&m)
    }

    #[test]
    fn cube_all_edges_chamfer_is_exact_closed_and_outward() {
        let mut m = primitives::cube(2.0);
        let edges = all_edges(&m);
        let out = bevel_edges(&mut m, &edges, 0.3, 1, 0.5, true).unwrap();
        assert_eq!(m.vert_count(), 24);
        assert_eq!(m.face_count(), 6 + 12 + 8);
        assert_eq!(out.created_faces.len(), 20);
        assert_closed(&m, Some(2));
        assert_consistent_orientation(&m);
        assert_outward_star(&m);
        let v = signed_volume(&m);
        assert!(v < 8.0 && v > 6.0, "{v}");
        let reference = polytope_volume_cube_chamfer(2.0, 0.3);
        assert!(
            (v - reference).abs() < 1e-3,
            "bevel {v} vs poliedro {reference}"
        );
        for f in 0..m.face_count() as u32 {
            assert!(m.face_area(f) > 1e-4, "face {f} degenerada");
        }
    }

    #[test]
    fn cube_all_edges_round_bevel_with_segments() {
        for (segs, prof) in [
            (2u32, 0.5f32),
            (3, 0.5),
            (4, 0.5),
            (5, 0.7),
            (3, 0.25),
            (3, 1.0),
            (4, 0.0),
        ] {
            let mut m = primitives::cube(2.0);
            let edges = all_edges(&m);
            bevel_edges(&mut m, &edges, 0.3, segs, prof, true).unwrap();
            assert_closed(&m, Some(2));
            assert_consistent_orientation(&m);
            assert_outward_star(&m);
            let v = signed_volume(&m);
            assert!(v > 6.0 && v < 8.0, "segs={segs} prof={prof} vol={v}");
            m.validate().unwrap();
        }
        // perfil circular: volume maior que o chanfro reto, menor que o cubo
        let vol = |segs: u32, prof: f32| {
            let mut m = primitives::cube(2.0);
            let edges = all_edges(&m);
            bevel_edges(&mut m, &edges, 0.3, segs, prof, true).unwrap();
            signed_volume(&m)
        };
        let flat = vol(1, 0.5);
        let round = vol(8, 0.5);
        assert!(round > flat && round < 8.0, "{flat} < {round} < 8");
        // mais convexo => mais volume
        assert!(vol(8, 0.8) > round);
        assert!(vol(8, 0.3) < round);
    }

    #[test]
    fn round_profile_points_lie_on_a_circle_of_radius_width() {
        let mut m = primitives::cube(2.0);
        // uma só aresta: topo-frente (y = 1, z = 1), ao longo de x
        let e = all_edges(&m)
            .into_iter()
            .find(|&(a, b)| {
                let (pa, pb) = (m.co(a), m.co(b));
                (pa.y - 1.0).abs() < 1e-6
                    && (pb.y - 1.0).abs() < 1e-6
                    && (pa.z - 1.0).abs() < 1e-6
                    && (pb.z - 1.0).abs() < 1e-6
            })
            .unwrap();
        let w = 0.4;
        let out = bevel_edges(&mut m, &[e], w, 6, 0.5, true).unwrap();
        assert_closed(&m, Some(2));
        // centro do arco: (y, z) = (1 - w, 1 - w)
        let mut on_circle = 0;
        for &v in &out.created_verts {
            let p = m.co(v);
            let r = ((p.y - (1.0 - w)).powi(2) + (p.z - (1.0 - w)).powi(2)).sqrt();
            if (r - w).abs() < 1e-4 {
                on_circle += 1;
            }
        }
        assert!(
            on_circle >= out.created_verts.len() / 2,
            "{on_circle}/{}",
            out.created_verts.len()
        );
        // área total do arco: w * pi/2 por unidade de comprimento
        assert_outward_star(&m);
    }

    #[test]
    fn single_edge_and_edge_loops_keep_euler() {
        // 1 aresta do cubo
        let mut m = primitives::cube(2.0);
        let e = all_edges(&m)[0];
        bevel_edges(&mut m, &[e], 0.2, 1, 0.5, true).unwrap();
        assert_eq!(m.vert_count(), 10);
        assert_eq!(m.face_count(), 7);
        assert_closed(&m, Some(2));
        assert_consistent_orientation(&m);
        // duas arestas adjacentes (compartilham um vértice)
        let mut m = primitives::cube(2.0);
        let t = Topology::build(&m);
        let v0 = 0u32;
        let inc: Vec<EdgeKey> = t.vert_edges[v0 as usize]
            .iter()
            .map(|&e| t.edges[e as usize].verts)
            .take(2)
            .collect();
        bevel_edges(&mut m, &inc, 0.2, 2, 0.5, true).unwrap();
        assert_closed(&m, Some(2));
        assert_consistent_orientation(&m);
        assert_outward_star(&m);
        // anel de arestas do topo do cubo
        let mut m = primitives::cube(2.0);
        let ring: Vec<EdgeKey> = all_edges(&m)
            .into_iter()
            .filter(|&(a, b)| m.co(a).y > 0.9 && m.co(b).y > 0.9)
            .collect();
        assert_eq!(ring.len(), 4);
        bevel_edges(&mut m, &ring, 0.3, 3, 0.5, true).unwrap();
        assert_closed(&m, Some(2));
        assert_outward_star(&m);
    }

    #[test]
    fn cylinder_cap_edges_and_quad_sphere_loops() {
        let mut m = primitives::cylinder(1.0, 2.0, 12, 1, true);
        let t = Topology::build(&m);
        let cap_edges: Vec<EdgeKey> = t
            .edges
            .iter()
            .filter(|e| e.faces.iter().any(|&f| m.faces[f as usize].len() == 12))
            .map(|e| e.verts)
            .collect();
        assert_eq!(cap_edges.len(), 24);
        let n0 = m.face_count();
        bevel_edges(&mut m, &cap_edges, 0.2, 3, 0.5, true).unwrap();
        assert_closed(&m, Some(2));
        assert_consistent_orientation(&m);
        assert_outward_star(&m);
        assert_eq!(
            m.face_count(),
            n0 + 24 * 3 + 24 * 3,
            "faixas + patches de canto (3 quads cada)"
        );
        // loop da quad sphere (vértices de valência 4 com duas arestas chanfradas opostas)
        let mut s = primitives::quad_sphere(1.0, 6);
        let ts = Topology::build(&s);
        let e = ts
            .edges
            .iter()
            .map(|e| e.verts)
            .find(|&(a, b)| s.co(a).y.abs() < 1e-5 && s.co(b).y.abs() < 1e-5)
            .unwrap();
        let lp = super::super::loops::edge_loop(&s, &ts, e).unwrap();
        assert!(lp.closed);
        for segs in [1u32, 2, 3] {
            let mut sc = s.clone();
            bevel_edges(&mut sc, &lp.edges, 0.05, segs, 0.5, true).unwrap();
            assert_closed(&sc, Some(2));
            assert_consistent_orientation(&sc);
            assert_outward_star(&sc);
        }
        // todas as arestas de uma esfera pequena: valência 3 e 4 misturadas
        let edges = all_edges(&s);
        bevel_edges(&mut s, &edges, 0.04, 2, 0.5, true).unwrap();
        assert_closed(&s, Some(2));
        assert_consistent_orientation(&s);
    }

    #[test]
    fn boundary_vertices_and_open_meshes() {
        // caixa aberta (sem a face de cima): chanfra uma aresta vertical interna ao canto
        let mut g = primitives::grid(3, 3, 3.0, 3.0);
        let t = Topology::build(&g);
        let e = t
            .edges
            .iter()
            .filter(|e| e.faces.len() == 2)
            .map(|e| e.verts)
            .find(|&(a, b)| g.co(a).x.abs() > 1.4 || g.co(b).x.abs() > 1.4)
            .unwrap();
        // aresta interna que termina na fronteira
        bevel_edges(&mut g, &[e], 0.2, 2, 0.5, true).unwrap();
        g.validate().unwrap();
        let tg = Topology::build(&g);
        assert_eq!(tg.non_manifold_edge_count(), 0);
        assert_consistent_orientation(&g);
        // todas as arestas internas da grade
        let mut g2 = primitives::grid(4, 4, 4.0, 4.0);
        let interior: Vec<EdgeKey> = Topology::build(&g2)
            .edges
            .iter()
            .filter(|e| e.faces.len() == 2)
            .map(|e| e.verts)
            .collect();
        bevel_edges(&mut g2, &interior, 0.15, 2, 0.5, true).unwrap();
        let t2 = Topology::build(&g2);
        assert_eq!(t2.non_manifold_edge_count(), 0);
        assert_consistent_orientation(&g2);
        g2.validate().unwrap();
        // cilindro aberto (lateral) com a aresta vertical
        let mut c = primitives::cylinder(1.0, 2.0, 8, 2, false);
        let tc = Topology::build(&c);
        let verticals: Vec<EdgeKey> = tc
            .edges
            .iter()
            .filter(|e| e.faces.len() == 2)
            .map(|e| e.verts)
            .filter(|&(a, b)| (c.co(a).y - c.co(b).y).abs() > 0.5)
            .collect();
        bevel_edges(&mut c, &verticals, 0.1, 2, 0.5, true).unwrap();
        assert_eq!(Topology::build(&c).non_manifold_edge_count(), 0);
        assert_consistent_orientation(&c);
    }

    #[test]
    fn property_random_edge_subsets_stay_closed_manifold() {
        let bases = [
            primitives::cube(2.0),
            primitives::cube_subdivided(2.0, 3),
            primitives::quad_sphere(1.0, 4),
            primitives::cylinder(1.0, 2.0, 8, 2, true),
            primitives::torus(2.0, 0.6, 12, 8),
            primitives::capsule(0.5, 1.0, 8, 3),
            primitives::uv_sphere(1.0, 6, 8),
        ];
        let mut state = 0x9E3779B97F4A7C15u64;
        let mut rnd = move || {
            state ^= state << 13;
            state ^= state >> 7;
            state ^= state << 17;
            state
        };
        let mut ok_count = 0;
        for base in &bases {
            let edges = all_edges(base);
            let t = Topology::build(base);
            let euler = t.euler_characteristic(base);
            for trial in 0..40 {
                let frac = [0.05f64, 0.2, 0.5, 1.0][trial % 4];
                let sel: Vec<EdgeKey> = edges
                    .iter()
                    .copied()
                    .filter(|_| (rnd() % 1000) as f64 / 1000.0 < frac)
                    .collect();
                if sel.is_empty() {
                    continue;
                }
                let segs = 1 + (rnd() % 4) as u32;
                let mut m = base.clone();
                let w = [0.02f32, 0.08, 0.25][(rnd() % 3) as usize];
                let prof = [0.2f32, 0.5, 0.5, 0.9][(rnd() % 4) as usize];
                match bevel_edges(&mut m, &sel, w, segs, prof, true) {
                    Ok(_) => {
                        ok_count += 1;
                        assert_closed(&m, Some(euler));
                        assert_consistent_orientation(&m);
                        let v0 = signed_volume(base);
                        let v1 = signed_volume(&m);
                        assert!(
                            v1 <= v0 * 1.002 && v1 > v0 * 0.6,
                            "{} {v1} vs {v0}",
                            base.name
                        );
                    }
                    Err(ModelError::UnsupportedTopology(_)) => {}
                    Err(e) => panic!("erro inesperado {e:?} em {}", base.name),
                }
            }
        }
        assert!(ok_count >= 70, "poucos casos suportados: {ok_count}");
    }

    #[test]
    fn pole_with_separated_beveled_edges_does_not_pinch_the_vertex() {
        // polo da uv sphere (valência 8): duas arestas chanfradas não adjacentes
        // deixam dois grupos de arestas sem vizinho chanfrado; o vértice não pode ficar belisco
        let base = primitives::uv_sphere(1.0, 6, 8);
        let t = Topology::build(&base);
        let inc: Vec<EdgeKey> = t.vert_edges[0]
            .iter()
            .map(|&i| t.edges[i as usize].verts)
            .collect();
        assert_eq!(inc.len(), 8);
        for (a, b) in [(0usize, 2usize), (0, 1), (0, 3), (2, 7)] {
            for segs in [1u32, 2, 4] {
                let mut m = base.clone();
                bevel_edges(&mut m, &[inc[a], inc[b]], 0.08, segs, 0.5, true).unwrap();
                assert_closed(&m, Some(2));
                assert_consistent_orientation(&m);
                assert_outward_star(&m);
            }
        }
        // opostas: dois grupos de arestas livres -> recusado com mensagem clara, malha intacta
        for (a, b) in [(0usize, 4usize), (1, 5)] {
            let mut m = base.clone();
            let h = m.content_hash();
            match bevel_edges(&mut m, &[inc[a], inc[b]], 0.08, 2, 0.5, true) {
                Err(ModelError::UnsupportedTopology(msg)) => {
                    assert!(msg.contains("beliscado"), "{msg}")
                }
                other => panic!("esperava UnsupportedTopology, veio {other:?}"),
            }
            assert_eq!(h, m.content_hash());
        }
        // todas as arestas do polo
        let mut m = base.clone();
        bevel_edges(&mut m, &inc, 0.05, 2, 0.5, true).unwrap();
        assert_closed(&m, Some(2));
        assert_outward_star(&m);
    }

    #[test]
    fn by_weight_selects_marked_edges_and_scales_width() {
        let mut m = primitives::cube(2.0);
        let ring: Vec<EdgeKey> = all_edges(&m)
            .into_iter()
            .filter(|&(a, b)| m.co(a).y > 0.9 && m.co(b).y > 0.9)
            .collect();
        for e in &ring {
            m.set_bevel_weight(e.0, e.1, 0.5);
        }
        // uma aresta inferior com peso fraco, abaixo do limiar
        let low = all_edges(&m)
            .into_iter()
            .find(|&(a, b)| m.co(a).y < -0.9 && m.co(b).y < -0.9)
            .unwrap();
        m.set_bevel_weight(low.0, low.1, 0.1);
        let mut by_weight = m.clone();
        bevel_edges_by_weight(&mut by_weight, 0.3, 0.4, 1, 0.5, true).unwrap();
        let mut manual = primitives::cube(2.0);
        bevel_edges(&mut manual, &ring, 0.2, 1, 0.5, true).unwrap();
        assert_closed(&by_weight, Some(2));
        assert!((signed_volume(&by_weight) - signed_volume(&manual)).abs() < 1e-4);
        // os pesos das arestas consumidas somem; o da aresta fraca permanece
        assert!(by_weight.edge_attr(low.0, low.1).bevel_weight > 0.05);
        // nenhuma elegível
        let mut none = primitives::cube(2.0);
        assert!(matches!(
            bevel_edges_by_weight(&mut none, 0.5, 0.2, 1, 0.5, true),
            Err(ModelError::EmptySelection(_))
        ));
        assert!(matches!(
            bevel_edges_by_weight(&mut none, 0.0, 0.2, 1, 0.5, true),
            Err(ModelError::InvalidParameter(_))
        ));
        assert!(matches!(
            bevel_edges_by_weight(&mut none, 1.5, 0.2, 1, 0.5, true),
            Err(ModelError::InvalidParameter(_))
        ));
    }

    #[test]
    fn vertex_bevel_cuts_corners() {
        let mut m = primitives::cube(2.0);
        let all: Vec<u32> = (0..8).collect();
        let out = bevel_verts(&mut m, &all, 0.4, 1).unwrap();
        assert_eq!(m.vert_count(), 24);
        assert_eq!(m.face_count(), 6 + 8);
        assert_eq!(out.created_faces.len(), 8);
        assert_closed(&m, Some(2));
        assert_consistent_orientation(&m);
        assert_outward_star(&m);
        // 8 tetraedros de aresta 0.4 removidos
        let expected = 8.0 - 8.0 * (0.4f32.powi(3) / 6.0);
        assert!((signed_volume(&m) - expected).abs() < 1e-4);
        // com segmentos
        for segs in [2u32, 3] {
            let mut m2 = primitives::cube(2.0);
            bevel_verts(&mut m2, &all, 0.4, segs).unwrap();
            assert_closed(&m2, Some(2));
            assert_consistent_orientation(&m2);
            assert_outward_star(&m2);
            assert!(signed_volume(&m2) > expected - 1e-3 && signed_volume(&m2) < 8.0);
        }
        // um só vértice
        let mut m3 = primitives::cube(2.0);
        bevel_verts(&mut m3, &[3], 0.5, 1).unwrap();
        assert_closed(&m3, Some(2));
        // vértices da quad sphere (valência 3 e 4 misturadas) e vértice de fronteira
        let mut s = primitives::quad_sphere(1.0, 4);
        let some: Vec<u32> = (0..s.vert_count() as u32).step_by(5).collect();
        bevel_verts(&mut s, &some, 0.05, 2).unwrap();
        assert_closed(&s, Some(2));
        let mut g = primitives::grid(2, 2, 2.0, 2.0);
        let corner = (0..g.vert_count() as u32)
            .find(|&v| (g.co(v) - Vec3::new(-1.0, 0.0, -1.0)).length() < 1e-5)
            .unwrap();
        let center = (0..g.vert_count() as u32)
            .find(|&v| g.co(v).length() < 1e-5)
            .unwrap();
        bevel_verts(&mut g, &[corner, center], 0.2, 1).unwrap();
        g.validate().unwrap();
        assert_eq!(Topology::build(&g).non_manifold_edge_count(), 0);
        assert_consistent_orientation(&g);
    }

    #[test]
    fn uv_material_and_edge_attrs_are_handled() {
        let mut m = primitives::cube(2.0);
        let t = Topology::build(&m);
        let e = t.edges[0].verts;
        // aresta vizinha (compartilha vértice) com seam, que não é chanfrada
        let other = t.vert_edges[e.0 as usize]
            .iter()
            .map(|&i| t.edges[i as usize].verts)
            .find(|&k| k != e)
            .unwrap();
        m.set_seam(other.0, other.1, true);
        m.set_sharp(e.0, e.1, true);
        for f in &mut m.faces {
            f.material = 2;
        }
        let out = bevel_edges(&mut m, &[e], 0.25, 2, 0.5, true).unwrap();
        // seam acompanhou o encurtamento
        assert_eq!(m.edge_attrs.values().filter(|a| a.seam).count(), 1);
        // sharp da aresta chanfrada sumiu e não há wire edges órfãs
        assert!(m.edge_attrs.values().all(|a| !a.sharp));
        let tt = Topology::build(&m);
        assert!(tt.edges.iter().all(|e| !e.faces.is_empty()));
        for &f in &out.created_faces {
            assert_eq!(m.faces[f as usize].material, 2);
            assert!(m.faces[f as usize].smooth);
            for c in &m.faces[f as usize].corners {
                assert!(c.uv.is_finite());
            }
        }
        for f in &m.faces {
            assert!(f.corners.iter().all(|c| c.uv.is_finite()));
        }
        // UV deslocada: cubo de lado 2, UV 0..1; largura 0.25 => 0.125
        let mut cube = primitives::cube(2.0);
        let ring_edge = Topology::build(&cube).edges[0].verts;
        bevel_edges(&mut cube, &[ring_edge], 0.25, 1, 0.5, true).unwrap();
        for f in &cube.faces {
            for c in &f.corners {
                assert!(c.uv.x > -0.2 && c.uv.x < 1.2 && c.uv.y > -0.2 && c.uv.y < 1.2);
            }
        }
    }

    #[test]
    fn clamp_overlap_prevents_inversion_on_short_edges() {
        let mut m = primitives::cube(2.0);
        let edges = all_edges(&m);
        // largura maior que metade da aresta: com clamp continua válido
        bevel_edges(&mut m, &edges, 5.0, 2, 0.5, true).unwrap();
        assert_closed(&m, Some(2));
        assert_consistent_orientation(&m);
        assert!(signed_volume(&m) > 0.0);
        for v in &m.verts {
            assert!(v.co.is_finite());
            assert!(v.co.abs().max_element() <= 1.0 + 1e-4);
        }
        // sem clamp não pode dar pânico nem NaN
        let mut m2 = primitives::cube(2.0);
        let r = bevel_edges(&mut m2, &edges, 5.0, 2, 0.5, false);
        assert!(r.is_ok() || matches!(r, Err(ModelError::Numeric(_))));
        for v in &m2.verts {
            assert!(v.co.is_finite());
        }
    }

    #[test]
    fn invalid_inputs_leave_mesh_untouched() {
        let mut m = primitives::cube(2.0);
        let h = m.content_hash();
        let e = all_edges(&m)[0];
        assert!(matches!(
            bevel_edges(&mut m, &[], 0.1, 1, 0.5, true),
            Err(ModelError::EmptySelection(_))
        ));
        for w in [0.0f32, -1.0, f32::NAN, f32::INFINITY] {
            assert!(matches!(
                bevel_edges(&mut m, &[e], w, 1, 0.5, true),
                Err(ModelError::InvalidParameter(_))
            ));
        }
        assert!(matches!(
            bevel_edges(&mut m, &[e], 0.1, 0, 0.5, true),
            Err(ModelError::InvalidParameter(_))
        ));
        assert!(matches!(
            bevel_edges(&mut m, &[e], 0.1, 1000, 0.5, true),
            Err(ModelError::InvalidParameter(_))
        ));
        for p in [-0.1f32, 1.1, f32::NAN] {
            assert!(matches!(
                bevel_edges(&mut m, &[e], 0.1, 1, p, true),
                Err(ModelError::InvalidParameter(_))
            ));
        }
        assert!(matches!(
            bevel_edges(&mut m, &[(0, 99)], 0.1, 1, 0.5, true),
            Err(ModelError::VertexOutOfRange(99, 8))
        ));
        let t = Topology::build(&m);
        let missing = (0..8u32)
            .flat_map(|a| (a + 1..8).map(move |b| (a, b)))
            .find(|&(a, b)| t.edge(a, b).is_none())
            .unwrap();
        assert!(matches!(
            bevel_edges(&mut m, &[missing], 0.1, 1, 0.5, true),
            Err(ModelError::MissingEdge(_, _))
        ));
        assert!(matches!(
            bevel_verts(&mut m, &[], 0.1, 1),
            Err(ModelError::EmptySelection(_))
        ));
        assert!(matches!(
            bevel_verts(&mut m, &[99], 0.1, 1),
            Err(ModelError::VertexOutOfRange(99, 8))
        ));
        assert!(matches!(
            bevel_verts(&mut m, &[0], 0.0, 1),
            Err(ModelError::InvalidParameter(_))
        ));
        assert_eq!(h, m.content_hash());
    }

    #[test]
    fn unsupported_topologies_report_clear_errors() {
        // borda
        let mut g = primitives::grid(2, 2, 2.0, 2.0);
        let hg = g.content_hash();
        let border = Topology::build(&g)
            .edges
            .iter()
            .find(|e| e.faces.len() == 1)
            .unwrap()
            .verts;
        let err = bevel_edges(&mut g, &[border], 0.1, 1, 0.5, true).unwrap_err();
        match err {
            ModelError::UnsupportedTopology(msg) => assert!(msg.contains("fronteira"), "{msg}"),
            e => panic!("{e:?}"),
        }
        assert_eq!(hg, g.content_hash());
        // aresta solta
        let mut w = primitives::cube(1.0);
        crate::ops::extrude::extrude_verts(&mut w, &[0]).unwrap();
        let wire = *w.edge_attrs.keys().next().unwrap();
        let hw = w.content_hash();
        assert!(matches!(
            bevel_edges(&mut w, &[wire], 0.1, 1, 0.5, true),
            Err(ModelError::UnsupportedTopology(_))
        ));
        // aresta interna cujo vértice tem uma aresta solta incidente
        let t = Topology::build(&w);
        let around = t.vert_edges[0]
            .iter()
            .map(|&i| t.edges[i as usize].verts)
            .find(|k| !t.is_wire_edge(k.0, k.1))
            .unwrap();
        assert!(matches!(
            bevel_edges(&mut w, &[around], 0.1, 1, 0.5, true),
            Err(ModelError::UnsupportedTopology(_))
        ));
        assert_eq!(hw, w.content_hash());
        // não-manifold: aba de 3 faces
        let mut f = primitives::grid(2, 1, 2.0, 1.0);
        let interior = Topology::build(&f)
            .edges
            .iter()
            .find(|e| e.faces.len() == 2)
            .unwrap()
            .verts;
        crate::ops::extrude::extrude_edges(&mut f, &[interior]).unwrap();
        let hf = f.content_hash();
        assert!(matches!(
            bevel_edges(&mut f, &[interior], 0.1, 1, 0.5, true),
            Err(ModelError::UnsupportedTopology(_))
        ));
        assert_eq!(hf, f.content_hash());
        // normais invertidas numa face: enrolamento inconsistente
        let mut c = primitives::cube(2.0);
        c.flip_face(0).unwrap();
        let hc = c.content_hash();
        let ce = all_edges(&c);
        let r = bevel_edges(&mut c, &ce, 0.1, 1, 0.5, true);
        assert!(
            matches!(r, Err(ModelError::UnsupportedTopology(_))),
            "{r:?}"
        );
        assert_eq!(hc, c.content_hash());
        // vértice de fronteira com bevel de vértice numa pinça
        let mut bow = EditMesh::new("bowtie");
        for p in [
            Vec3::ZERO,
            Vec3::X,
            Vec3::new(1.0, 0.0, -1.0),
            Vec3::NEG_X,
            Vec3::new(-1.0, 0.0, 1.0),
        ] {
            bow.add_vert(p);
        }
        bow.add_face(&[0, 1, 2]).unwrap();
        bow.add_face(&[0, 3, 4]).unwrap();
        let hb = bow.content_hash();
        assert!(matches!(
            bevel_verts(&mut bow, &[0], 0.1, 1),
            Err(ModelError::UnsupportedTopology(_))
        ));
        assert_eq!(hb, bow.content_hash());
    }

    #[test]
    fn bevel_is_deterministic_and_fast_on_large_mesh() {
        let run = || {
            let mut m = primitives::quad_sphere(1.0, 64);
            let t = Topology::build(&m);
            // arestas espalhadas (não adjacentes na maioria)
            let sel: Vec<EdgeKey> = t
                .edges
                .iter()
                .enumerate()
                .filter(|(i, _)| i % 41 == 0)
                .map(|(_, e)| e.verts)
                .collect();
            let t0 = std::time::Instant::now();
            bevel_edges(&mut m, &sel, 0.004, 2, 0.5, true).unwrap();
            assert!(t0.elapsed().as_secs() < 60);
            m
        };
        let a = run();
        assert_closed(&a, Some(2));
        assert_eq!(a.content_hash(), run().content_hash());
    }
}
