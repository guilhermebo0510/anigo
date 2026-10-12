//! Roupa por malha-proxy simulada em XPBD (estilo Magica Cloth 2):
//!
//! * a proxy é uma malha pequena (centenas de vértices) com vértices FIXOS (seguem o corpo por
//!   skinning) e MÓVEIS; cada vértice recebe um `depth` 0..1 = distância geodésica ao fixo mais
//!   próximo / maior distância;
//! * restrições: distância (estrutural) + flexão (vértices opostos de triângulos vizinhos),
//!   **angle restoration** (pai->filho volta ao vetor animado, rigidez decrescente com o depth),
//!   **tether** (limite de alongamento/compressão em relação ao fixo mais próximo), **max distance**
//!   e **backstop** (não entra além do plano da normal animada) por depth, e colisão com os
//!   colliders do corpo (cápsulas ao longo dos ossos);
//! * o resultado é transferido à malha de render por [`ProxyBinding`] (3 vértices + baricêntricas
//!   + offset).

use glam::{Mat4, Vec3};
use serde::{Deserialize, Serialize};

use crate::binding::{apply_proxy_bindings, bind_to_proxy, ProxyBinding};
use crate::collider::ColliderSet;
use crate::error::{check_index, PhysicsError, PhysicsResult};
use crate::hair::compliance_from_stiffness;
use crate::math::rotation_of;
use crate::xpbd::{ParticleLimit, XpbdConfig, XpbdSystem};

/// Skinning de um vértice da proxy (4 influências, como o renderer).
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct VertexSkin {
    pub joints: [u16; 4],
    pub weights: [f32; 4],
}

impl VertexSkin {
    pub fn rigid(bone: u16) -> Self {
        Self { joints: [bone, 0, 0, 0], weights: [1.0, 0.0, 0.0, 0.0] }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct ClothParams {
    pub stretch_compliance: f32,
    pub bending_compliance: f32,
    /// Rigidez do angle restoration (0..1) no depth 0 e no depth 1; curva `depth^power`.
    pub angle_stiffness_root: f32,
    pub angle_stiffness_tip: f32,
    pub angle_power: f32,
    /// Alongamento máximo permitido em relação à distância geodésica ao fixo (fração). < 0 desliga o tether.
    pub tether_stretch: f32,
    /// Compressão mínima: distância >= `tether_compress * distância de repouso` ao fixo.
    pub tether_compress: f32,
    /// Max distance ao alvo animado (m) no depth 0 e no depth 1. `INFINITY` desliga.
    #[serde(with = "crate::math::serde_limit")]
    pub max_distance_root: f32,
    #[serde(with = "crate::math::serde_limit")]
    pub max_distance_tip: f32,
    /// Backstop (m): a roupa não entra além disto atrás do plano da normal animada. `INFINITY` desliga.
    #[serde(with = "crate::math::serde_limit")]
    pub backstop: f32,
    pub mass: f32,
    pub tip_mass_scale: f32,
    pub damping_root: f32,
    pub damping_tip: f32,
    pub gravity_scale: f32,
    pub particle_radius: f32,
}

impl Default for ClothParams {
    fn default() -> Self {
        Self {
            stretch_compliance: 1e-7,
            bending_compliance: 5e-5,
            angle_stiffness_root: 0.95,
            angle_stiffness_tip: 0.35,
            angle_power: 1.0,
            tether_stretch: 0.05,
            tether_compress: 0.8,
            max_distance_root: 0.03,
            max_distance_tip: 0.35,
            backstop: 0.01,
            mass: 1.0,
            tip_mass_scale: 1.0,
            damping_root: 6.0,
            damping_tip: 2.0,
            gravity_scale: 0.8,
            particle_radius: 0.01,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct ClothProxyDef {
    pub name: String,
    /// Posições de repouso (mundo, na pose de bind).
    pub positions: Vec<Vec3>,
    /// Triângulos CCW vistos de fora (a normal aponta para fora do corpo).
    pub triangles: Vec<[u32; 3]>,
    /// Vértices fixos (acompanham o corpo). Pelo menos um.
    pub fixed: Vec<bool>,
    /// Skinning por vértice; vazio = proxy parada no mundo.
    pub skin: Vec<VertexSkin>,
    pub params: ClothParams,
    pub xpbd: XpbdConfig,
    pub collider_filter: Option<Vec<usize>>,
}

impl Default for ClothProxyDef {
    fn default() -> Self {
        Self {
            name: "cloth".into(),
            positions: Vec::new(),
            triangles: Vec::new(),
            fixed: Vec::new(),
            skin: Vec::new(),
            params: ClothParams::default(),
            xpbd: XpbdConfig { substeps: 6, damping: 1.0, ..Default::default() },
            collider_filter: None,
        }
    }
}

#[derive(Debug, Clone)]
pub struct ClothSim {
    sys: XpbdSystem,
    rest: Vec<Vec3>,
    rest_normals: Vec<Vec3>,
    triangles: Vec<[u32; 3]>,
    skin: Vec<VertexSkin>,
    inv_bind: Vec<Mat4>,
    last_good_bones: Vec<Mat4>,
    depth: Vec<f32>,
    geodesic: Vec<f32>,
    edges: Vec<(u32, u32, f32)>,
    bindings: Vec<ProxyBinding>,
}

/// Normais de vértice (média ponderada por área) de uma malha de triângulos.
pub fn vertex_normals(positions: &[Vec3], triangles: &[[u32; 3]]) -> Vec<Vec3> {
    let mut n = vec![Vec3::ZERO; positions.len()];
    for t in triangles {
        let (a, b, c) = (t[0] as usize, t[1] as usize, t[2] as usize);
        if a >= positions.len() || b >= positions.len() || c >= positions.len() {
            continue;
        }
        let fn_ = (positions[b] - positions[a]).cross(positions[c] - positions[a]);
        if fn_.is_finite() {
            n[a] += fn_;
            n[b] += fn_;
            n[c] += fn_;
        }
    }
    n.into_iter().map(|v| if v.length_squared() > 1e-18 { v.normalize() } else { Vec3::Y }).collect()
}

impl ClothSim {
    pub fn new(def: &ClothProxyDef, rest_bone_world: &[Mat4]) -> PhysicsResult<Self> {
        let p = def.params;
        let n = def.positions.len();
        if n < 3 || def.triangles.is_empty() {
            return Err(PhysicsError::BadMesh("proxy precisa de >= 3 vértices e >= 1 triângulo".into()));
        }
        if def.positions.iter().any(|v| !v.is_finite()) {
            return Err(PhysicsError::BadValue { what: "ClothProxyDef.positions".into() });
        }
        if def.fixed.len() != n {
            return Err(PhysicsError::BadMesh(format!("fixed tem {} entradas, esperado {n}", def.fixed.len())));
        }
        if !def.fixed.iter().any(|f| *f) {
            return Err(PhysicsError::BadMesh("proxy sem vértice fixo (nada a que se prender)".into()));
        }
        if !def.skin.is_empty() && def.skin.len() != n {
            return Err(PhysicsError::BadMesh(format!("skin tem {} entradas, esperado {n}", def.skin.len())));
        }
        for (what, v) in [
            ("stretch_compliance", p.stretch_compliance),
            ("bending_compliance", p.bending_compliance),
            ("angle_stiffness_root", p.angle_stiffness_root),
            ("angle_stiffness_tip", p.angle_stiffness_tip),
            ("angle_power", p.angle_power),
            ("tether_stretch", p.tether_stretch),
            ("tether_compress", p.tether_compress),
            ("mass", p.mass),
            ("tip_mass_scale", p.tip_mass_scale),
            ("damping_root", p.damping_root),
            ("damping_tip", p.damping_tip),
            ("gravity_scale", p.gravity_scale),
            ("particle_radius", p.particle_radius),
        ] {
            if !v.is_finite() {
                return Err(PhysicsError::BadValue { what: format!("ClothParams.{what}") });
            }
        }
        // max_distance/backstop aceitam INFINITY (= sem limite), mas não NaN
        for (what, v) in [("max_distance_root", p.max_distance_root), ("max_distance_tip", p.max_distance_tip), ("backstop", p.backstop)] {
            if v.is_nan() {
                return Err(PhysicsError::BadValue { what: format!("ClothParams.{what}") });
            }
        }
        if p.mass <= 0.0 || p.angle_power <= 0.0 || p.stretch_compliance < 0.0 || p.bending_compliance < 0.0 {
            return Err(PhysicsError::BadConfig("ClothParams: mass/angle_power > 0 e compliances >= 0".into()));
        }
        for t in &def.triangles {
            for &i in t {
                check_index("cloth.triangle", i as usize, n)?;
            }
        }
        for s in &def.skin {
            for (&j, &w) in s.joints.iter().zip(&s.weights) {
                if !w.is_finite() || w < 0.0 {
                    return Err(PhysicsError::BadValue { what: "VertexSkin.weights".into() });
                }
                if w > 0.0 {
                    check_index("cloth.skin.joint", j as usize, rest_bone_world.len())?;
                }
            }
        }
        if rest_bone_world.iter().any(|m| !m.is_finite()) {
            return Err(PhysicsError::BadValue { what: "rest_bone_world".into() });
        }

        // Arestas únicas + adjacência de triângulos (para flexão).
        let mut edge_map: std::collections::BTreeMap<(u32, u32), Vec<u32>> = std::collections::BTreeMap::new();
        for t in &def.triangles {
            if t[0] == t[1] || t[1] == t[2] || t[0] == t[2] {
                return Err(PhysicsError::BadMesh("triângulo com vértices repetidos".into()));
            }
            for k in 0..3 {
                let (a, b) = (t[k], t[(k + 1) % 3]);
                let opp = t[(k + 2) % 3];
                edge_map.entry((a.min(b), a.max(b))).or_default().push(opp);
            }
        }
        let mut edges = Vec::with_capacity(edge_map.len());
        let mut adjacency: Vec<Vec<(u32, f32)>> = vec![Vec::new(); n];
        for (&(a, b), _) in &edge_map {
            let len = (def.positions[a as usize] - def.positions[b as usize]).length();
            edges.push((a, b, len));
            adjacency[a as usize].push((b, len));
            adjacency[b as usize].push((a, len));
        }

        // Dijkstra multi-fonte a partir dos fixos: distância geodésica, âncora e pai na árvore.
        let mut geodesic = vec![f32::INFINITY; n];
        let mut anchor = vec![u32::MAX; n];
        let mut parent = vec![u32::MAX; n];
        let mut done = vec![false; n];
        for i in 0..n {
            if def.fixed[i] {
                geodesic[i] = 0.0;
                anchor[i] = i as u32;
            }
        }
        for _ in 0..n {
            let mut best = None;
            for i in 0..n {
                if !done[i] && geodesic[i].is_finite() && best.map(|b: usize| geodesic[i] < geodesic[b]).unwrap_or(true) {
                    best = Some(i);
                }
            }
            let Some(u) = best else { break };
            done[u] = true;
            for &(v, w) in &adjacency[u] {
                let v = v as usize;
                if geodesic[u] + w < geodesic[v] - 1e-9 {
                    geodesic[v] = geodesic[u] + w;
                    anchor[v] = anchor[u];
                    parent[v] = u as u32;
                }
            }
        }
        if geodesic.iter().any(|g| !g.is_finite()) {
            return Err(PhysicsError::BadMesh("há vértices desconectados de qualquer vértice fixo".into()));
        }
        let max_geo = geodesic.iter().copied().fold(0.0f32, f32::max).max(1e-9);
        let depth: Vec<f32> = geodesic.iter().map(|g| (g / max_geo).clamp(0.0, 1.0)).collect();

        let rest_normals = vertex_normals(&def.positions, &def.triangles);
        let mut sys = XpbdSystem::new(def.xpbd);
        sys.config.gravity *= p.gravity_scale;
        sys.collider_filter = def.collider_filter.clone();
        for i in 0..n {
            let t = depth[i];
            let mass = if def.fixed[i] { 0.0 } else { p.mass * (1.0 + (p.tip_mass_scale - 1.0) * t) };
            let idx = sys
                .add_particle(def.positions[i], mass, p.particle_radius)
                .ok_or_else(|| PhysicsError::BadValue { what: "posição da proxy".into() })?;
            let damping = p.damping_root + (p.damping_tip - p.damping_root) * t;
            sys.damping_scale[idx] = (damping / sys.config.damping.max(1e-6)).max(0.0);
            let max_d = p.max_distance_root + (p.max_distance_tip - p.max_distance_root) * t;
            sys.limits[idx] = ParticleLimit {
                max_distance: if max_d.is_nan() { f32::INFINITY } else { max_d.max(0.0) },
                backstop: if p.backstop.is_finite() { p.backstop.max(0.0) } else { f32::INFINITY },
            };
            sys.target_normals[idx] = rest_normals[i];
        }
        for &(a, b, len) in &edges {
            sys.add_distance(a as usize, b as usize, len, p.stretch_compliance);
        }
        for (&(a, b), opposite) in &edge_map {
            if opposite.len() == 2 && opposite[0] != opposite[1] {
                let _ = (a, b);
                sys.add_distance_auto(opposite[0] as usize, opposite[1] as usize, p.bending_compliance);
            }
        }
        for v in 0..n {
            if def.fixed[v] {
                continue;
            }
            let s = p.angle_stiffness_root + (p.angle_stiffness_tip - p.angle_stiffness_root) * depth[v].powf(p.angle_power);
            sys.add_direction(parent[v] as usize, v, compliance_from_stiffness(s));
            if p.tether_stretch >= 0.0 {
                let a = anchor[v] as usize;
                let euclid = (def.positions[v] - def.positions[a]).length();
                let max_len = geodesic[v] * (1.0 + p.tether_stretch);
                let min_len = (p.tether_compress.clamp(0.0, 1.0) * euclid).min(max_len);
                sys.add_tether(v, a, min_len, max_len, 0.0);
            }
        }
        sys.settle_here();

        let inv_bind: Vec<Mat4> = rest_bone_world.iter().map(|m| m.inverse()).collect();
        let sim = Self {
            sys,
            rest: def.positions.clone(),
            rest_normals,
            triangles: def.triangles.clone(),
            skin: def.skin.clone(),
            inv_bind,
            last_good_bones: rest_bone_world.to_vec(),
            depth,
            geodesic,
            edges,
            bindings: Vec::new(),
        };
        Ok(sim)
    }

    pub fn system(&self) -> &XpbdSystem {
        &self.sys
    }

    pub fn system_mut(&mut self) -> &mut XpbdSystem {
        &mut self.sys
    }

    pub fn vertex_count(&self) -> usize {
        self.rest.len()
    }

    pub fn positions(&self) -> &[Vec3] {
        &self.sys.pos
    }

    pub fn rest_positions(&self) -> &[Vec3] {
        &self.rest
    }

    pub fn triangles(&self) -> &[[u32; 3]] {
        &self.triangles
    }

    pub fn depth(&self) -> &[f32] {
        &self.depth
    }

    pub fn geodesic(&self) -> &[f32] {
        &self.geodesic
    }

    /// Normais atuais dos vértices da proxy simulada.
    pub fn normals(&self) -> Vec<Vec3> {
        vertex_normals(&self.sys.pos, &self.triangles)
    }

    /// Define a pose atual do esqueleto (matrizes de mundo dos ossos). Matriz não finita mantém a última boa.
    pub fn set_pose(&mut self, bone_world: &[Mat4]) {
        for (i, m) in bone_world.iter().enumerate() {
            if m.is_finite() {
                if let Some(slot) = self.last_good_bones.get_mut(i) {
                    *slot = *m;
                }
            }
        }
        let skin_mats: Vec<Mat4> = self.last_good_bones.iter().zip(&self.inv_bind).map(|(w, ib)| *w * *ib).collect();
        let n = self.rest.len();
        let mut targets = Vec::with_capacity(n);
        let mut normals = Vec::with_capacity(n);
        for i in 0..n {
            if self.skin.is_empty() {
                targets.push(self.rest[i]);
                normals.push(self.rest_normals[i]);
                continue;
            }
            let s = self.skin[i];
            let mut p = Vec3::ZERO;
            let mut nn = Vec3::ZERO;
            let mut wsum = 0.0f32;
            for k in 0..4 {
                let w = s.weights[k];
                if w <= 0.0 {
                    continue;
                }
                if let Some(m) = skin_mats.get(s.joints[k] as usize) {
                    p += m.transform_point3(self.rest[i]) * w;
                    nn += (rotation_of(m) * self.rest_normals[i]) * w;
                    wsum += w;
                }
            }
            if wsum > 1e-6 {
                targets.push(p / wsum);
                normals.push(nn / wsum);
            } else {
                targets.push(self.rest[i]);
                normals.push(self.rest_normals[i]);
            }
        }
        self.sys.set_targets(&targets, Some(&normals));
    }

    pub fn step(&mut self, dt: f32, colliders: &ColliderSet) {
        self.sys.step(dt, colliders);
    }

    pub fn reset(&mut self) {
        self.sys.reset_to_targets();
    }

    /// Prende uma malha de render (posições de repouso, no espaço da proxy em repouso) à proxy.
    pub fn bind_render_mesh(&mut self, vertices: &[Vec3]) -> PhysicsResult<()> {
        self.bindings = bind_to_proxy(vertices, &self.rest, &self.triangles)?;
        Ok(())
    }

    pub fn bindings(&self) -> &[ProxyBinding] {
        &self.bindings
    }

    /// Posições atuais da malha de render. Vértice com vínculo inválido volta ao repouso.
    pub fn render_positions(&self, rest_render: &[Vec3]) -> Vec<Vec3> {
        apply_proxy_bindings(&self.bindings, &self.sys.pos)
            .into_iter()
            .enumerate()
            .map(|(i, p)| p.unwrap_or_else(|| rest_render.get(i).copied().unwrap_or(Vec3::ZERO)))
            .collect()
    }

    /// Maior alongamento relativo das arestas estruturais da proxy.
    pub fn max_edge_stretch(&self) -> f32 {
        self.edges
            .iter()
            .map(|&(a, b, rest)| if rest > 1e-9 { ((self.sys.pos[a as usize] - self.sys.pos[b as usize]).length() - rest) / rest } else { 0.0 })
            .fold(0.0, f32::max)
    }
}

// ---------------------------------------------------------------------------
// Geradores de proxy (saia e capa) — também usados nos testes, exemplo e presets
// ---------------------------------------------------------------------------

/// Saia cônica: `segments` colunas x (`rings` + 1) anéis; o anel 0 (cintura) é fixo. Triângulos
/// com a normal para FORA. `waist`: centro do anel da cintura.
pub fn skirt_proxy(waist: Vec3, top_radius: f32, bottom_radius: f32, height: f32, segments: usize, rings: usize, bone: u16) -> ClothProxyDef {
    let segments = segments.max(3);
    let rings = rings.max(1);
    let mut positions = Vec::new();
    let mut fixed = Vec::new();
    for r in 0..=rings {
        let t = r as f32 / rings as f32;
        let radius = top_radius + (bottom_radius - top_radius) * t;
        for s in 0..segments {
            let a = std::f32::consts::TAU * s as f32 / segments as f32;
            positions.push(waist + Vec3::new(radius * a.cos(), -height * t, radius * a.sin()));
            fixed.push(r == 0);
        }
    }
    let mut triangles = Vec::new();
    let idx = |r: usize, s: usize| (r * segments + (s % segments)) as u32;
    for r in 0..rings {
        for s in 0..segments {
            // Ângulo cresce de +X para +Z (anti-horário visto de cima); esta ordem dá a normal para fora.
            triangles.push([idx(r, s), idx(r, s + 1), idx(r + 1, s)]);
            triangles.push([idx(r, s + 1), idx(r + 1, s + 1), idx(r + 1, s)]);
        }
    }
    let n = positions.len();
    ClothProxyDef {
        name: "skirt".into(),
        positions,
        triangles,
        fixed,
        skin: vec![VertexSkin::rigid(bone); n],
        ..Default::default()
    }
}

/// Capa: grade `cols` x (`rows` + 1) pendurada nos ombros; a fileira de cima é fixa. Fica no plano
/// XY em z = `z`, com leve curvatura para trás; normal para +Z (fora, para trás do corpo).
pub fn cape_proxy(top_left: Vec3, top_right: Vec3, height: f32, cols: usize, rows: usize, bone: u16) -> ClothProxyDef {
    let cols = cols.max(2);
    let rows = rows.max(1);
    let mut positions = Vec::new();
    let mut fixed = Vec::new();
    for r in 0..=rows {
        let v = r as f32 / rows as f32;
        for c in 0..cols {
            let u = c as f32 / (cols - 1) as f32;
            let top = top_left.lerp(top_right, u);
            let bulge = 0.06 * (std::f32::consts::PI * u).sin() * v; // abre um pouco para trás
            positions.push(top + Vec3::new(0.0, -height * v, bulge));
            fixed.push(r == 0);
        }
    }
    let mut triangles = Vec::new();
    let idx = |r: usize, c: usize| (r * cols + c) as u32;
    for r in 0..rows {
        for c in 0..cols - 1 {
            // top_left -> top_right é +X e v cresce para baixo (-Y): a ordem abaixo dá normal +Z.
            triangles.push([idx(r, c), idx(r + 1, c), idx(r, c + 1)]);
            triangles.push([idx(r, c + 1), idx(r + 1, c), idx(r + 1, c + 1)]);
        }
    }
    let n = positions.len();
    ClothProxyDef {
        name: "cape".into(),
        positions,
        triangles,
        fixed,
        skin: vec![VertexSkin::rigid(bone); n],
        params: ClothParams { max_distance_root: 0.02, max_distance_tip: 0.6, backstop: f32::INFINITY, ..Default::default() },
        ..Default::default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::collider::{Collider, ColliderShape};
    use glam::Quat;

    const DT: f32 = 1.0 / 60.0;

    fn hip_bone(pos: Vec3, rot: Quat) -> Mat4 {
        Mat4::from_rotation_translation(rot, pos)
    }

    fn skirt_sim() -> (ClothSim, ClothProxyDef) {
        let waist = Vec3::new(0.0, 0.9, 0.0);
        let def = skirt_proxy(waist, 0.14, 0.30, 0.4, 16, 6, 0);
        (ClothSim::new(&def, &[hip_bone(waist, Quat::IDENTITY)]).unwrap(), def)
    }

    #[test]
    fn depth_e_geodesica_crescem_da_cintura_para_a_barra() {
        let (sim, def) = skirt_sim();
        for (i, f) in def.fixed.iter().enumerate() {
            if *f {
                assert_eq!(sim.depth()[i], 0.0);
            }
        }
        let last_ring = def.positions.len() - 1;
        assert!((sim.depth()[last_ring] - 1.0).abs() < 1e-6);
        // monotonia por coluna
        let seg = 16;
        for s in 0..seg {
            for r in 0..6 {
                assert!(sim.depth()[(r + 1) * seg + s] > sim.depth()[r * seg + s]);
            }
        }
    }

    #[test]
    fn normais_da_saia_apontam_para_fora_e_da_capa_para_tras() {
        let (_, def) = skirt_sim();
        let normals = vertex_normals(&def.positions, &def.triangles);
        for (p, n) in def.positions.iter().zip(&normals) {
            let radial = Vec3::new(p.x, 0.0, p.z).normalize();
            assert!(n.dot(radial) > 0.5, "normal da saia para dentro em {p:?}: {n:?}");
        }
        let cape = cape_proxy(Vec3::new(-0.2, 1.4, -0.1), Vec3::new(0.2, 1.4, -0.1), 0.8, 6, 4, 0);
        for n in vertex_normals(&cape.positions, &cape.triangles) {
            assert!(n.z > 0.5, "{n:?}");
        }
    }

    #[test]
    fn saia_em_repouso_fica_em_repouso_e_nao_estica() {
        let (mut sim, def) = skirt_sim();
        let set = ColliderSet::new();
        let waist_pose = [hip_bone(Vec3::new(0.0, 0.9, 0.0), Quat::IDENTITY)];
        for _ in 0..600 {
            sim.set_pose(&waist_pose);
            sim.step(DT, &set);
        }
        // sem corpo a saia cede um pouco por gravidade mas respeita max_distance e tether
        let worst = def.positions.iter().zip(sim.positions()).map(|(a, b)| (*a - *b).length()).fold(0.0, f32::max);
        eprintln!("saia parada: desvio max {worst:.4} m, estiramento max {:.4}", sim.max_edge_stretch());
        assert!(worst <= 0.35 + 1e-3, "max_distance_tip respeitado");
        assert!(sim.max_edge_stretch() < 0.05);
    }

    #[test]
    fn saia_segue_a_cintura_girando_sem_estirar_mais_que_x_por_cento() {
        let (mut sim, _) = skirt_sim();
        let set = ColliderSet::new();
        let mut worst_stretch = 0.0f32;
        for f in 0..480 {
            let t = f as f32 * DT;
            let rot = Quat::from_rotation_y((t * 3.0).sin() * 1.2);
            let pos = Vec3::new((t * 2.0).sin() * 0.3, 0.9, 0.0);
            sim.set_pose(&[hip_bone(pos, rot)]);
            sim.step(DT, &set);
            worst_stretch = worst_stretch.max(sim.max_edge_stretch());
            assert!(sim.positions().iter().all(|p| p.is_finite()));
        }
        eprintln!("saia girando: estiramento max das arestas {:.4}", worst_stretch);
        assert!(worst_stretch < 0.06, "estiramento {worst_stretch}");
    }

    #[test]
    fn saia_nao_atravessa_as_pernas_chutando() {
        // Ossos: 0 = quadril, 1 = coxa esquerda, 2 = coxa direita. As pernas são cápsulas presas às
        // coxas; a perna esquerda chuta para frente (+Z) até 30 graus, empurrando a saia.
        let waist = Vec3::new(0.0, 0.9, 0.0);
        let hip_l = Vec3::new(-0.09, 0.9, 0.0);
        let hip_r = Vec3::new(0.09, 0.9, 0.0);
        let rest_bones = [hip_bone(waist, Quat::IDENTITY), hip_bone(hip_l, Quat::IDENTITY), hip_bone(hip_r, Quat::IDENTITY)];
        let mut def = skirt_proxy(waist, 0.19, 0.30, 0.45, 20, 7, 0);
        def.params.max_distance_tip = 0.6;
        def.params.gravity_scale = 1.0;
        def.params.backstop = f32::INFINITY;
        let mut set = ColliderSet::new();
        for bone in [1usize, 2] {
            set.add(Collider::on_bone(bone, ColliderShape::Capsule { a: Vec3::ZERO, b: Vec3::new(0.0, -0.7, 0.0), radius: 0.075 }));
        }
        // Executa com colisao e sem (controle): o controle PRECISA penetrar, senao o teste nao prova nada.
        let mut results = Vec::new();
        for with_colliders in [true, false] {
            let mut sim = ClothSim::new(&def, &rest_bones).unwrap();
            let mut worst = 0.0f32;
            let mut worst_stretch = 0.0f32;
            let mut contacts = 0u32;
            for f in 0..480 {
                let t = f as f32 * DT;
                let kick = ((t * 3.0).sin().max(0.0)) * 30f32.to_radians(); // so chuta para frente
                let bones = [
                    hip_bone(waist, Quat::IDENTITY),
                    hip_bone(hip_l, Quat::from_rotation_x(-kick)), // -X gira -Y para +Z
                    hip_bone(hip_r, Quat::IDENTITY),
                ];
                set.update(&bones);
                sim.set_pose(&bones);
                if with_colliders {
                    sim.step(DT, &set);
                } else {
                    sim.step(DT, &ColliderSet::new());
                }
                contacts += sim.system().stats().contacts;
                worst_stretch = worst_stretch.max(sim.max_edge_stretch());
                let (a, b) = (hip_l, hip_l + Quat::from_rotation_x(-kick) * Vec3::new(0.0, -0.7, 0.0));
                for (i, p) in sim.positions().iter().enumerate() {
                    if def.fixed[i] {
                        continue;
                    }
                    let (c, _) = crate::math::closest_point_on_segment(*p, a, b);
                    worst = worst.max(0.075 + def.params.particle_radius - (*p - c).length());
                }
            }
            results.push((worst, worst_stretch, contacts));
        }
        let (with, without) = (results[0], results[1]);
        eprintln!(
            "saia x perna chutando: COM colisao pen {:.5} m / estiramento {:.4} / {} contatos; SEM colisao (controle) pen {:.5} m",
            with.0, with.1, with.2, without.0
        );
        assert!(without.0 > 0.02, "o controle precisa penetrar: {}", without.0);
        assert!(with.2 > 0, "houve contatos");
        assert!(with.0 < 5e-3, "penetracao {}", with.0);
        assert!(with.1 < 0.10, "estiramento {}", with.1);
    }

    #[test]
    fn capa_oscila_e_estabiliza_em_dez_segundos() {
        let def = cape_proxy(Vec3::new(-0.2, 1.4, -0.1), Vec3::new(0.2, 1.4, -0.1), 0.8, 8, 8, 0);
        let mut sim = ClothSim::new(&def, &[hip_bone(Vec3::ZERO, Quat::IDENTITY)]).unwrap();
        let set = ColliderSet::new();
        // personagem anda 2 s e para; depois mede-se a velocidade residual
        for f in 0..1200 {
            let t = f as f32 * DT;
            let x = if t < 2.0 { t * 1.5 } else { 3.0 };
            sim.set_pose(&[hip_bone(Vec3::new(x, 0.0, 0.0), Quat::IDENTITY)]);
            sim.step(DT, &set);
            assert!(sim.positions().iter().all(|p| p.is_finite() && p.y > 0.0 && p.y < 2.0), "frame {f}");
        }
        let ke = sim.system().kinetic_energy();
        eprintln!("capa: energia cinetica apos 20 s = {ke:.3e}; estiramento {:.4}", sim.max_edge_stretch());
        assert!(ke < 1e-3);
        assert!(sim.max_edge_stretch() < 0.06);
    }

    #[test]
    fn transferencia_proxy_para_malha_de_render() {
        let (mut sim, def) = skirt_sim();
        // malha de render: pontos deslocados 1 cm para fora da proxy + pontos entre vertices
        let normals = vertex_normals(&def.positions, &def.triangles);
        let mut render: Vec<Vec3> = Vec::new();
        for (i, p) in def.positions.iter().enumerate() {
            render.push(*p + normals[i] * 0.01);
            render.push((*p + def.positions[(i + 1) % def.positions.len()]) * 0.5 + normals[i] * 0.005);
        }
        sim.bind_render_mesh(&render).unwrap();
        let out = sim.render_positions(&render);
        let err = render.iter().zip(&out).map(|(a, b)| (*a - *b).length()).fold(0.0, f32::max);
        eprintln!("transferencia no repouso: erro max {err:.2e}");
        assert!(err < 1e-4);
        // com movimento do corpo (rigido): a malha de render acompanha
        let rot = Quat::from_rotation_y(0.7);
        let pos = Vec3::new(0.4, 0.9, -0.2);
        let set = ColliderSet::new();
        sim.set_pose(&[hip_bone(pos, rot)]);
        sim.reset();
        sim.step(DT, &set);
        let moved = sim.render_positions(&render);
        assert!(moved.iter().all(|p| p.is_finite()));
        let rigid = |p: Vec3| rot * (p - Vec3::new(0.0, 0.9, 0.0)) + pos;
        // vertices vinculados a triangulos de alto-depth defasam; os do anel fixo coincidem
        let fixed_matches = (0..16).filter(|&i| (moved[2 * i] - rigid(render[2 * i])).length() < 0.02).count();
        assert!(fixed_matches >= 14, "anel da cintura acompanha o corpo: {fixed_matches}/16");
    }

    #[test]
    fn construcao_invalida_e_rejeitada() {
        let waist = Vec3::new(0.0, 0.9, 0.0);
        let bones = [hip_bone(waist, Quat::IDENTITY)];
        let base = skirt_proxy(waist, 0.14, 0.30, 0.4, 8, 3, 0);
        let mut d = base.clone();
        d.fixed = vec![false; d.positions.len()];
        assert!(ClothSim::new(&d, &bones).is_err(), "sem fixo");
        let mut d = base.clone();
        d.fixed.pop();
        assert!(ClothSim::new(&d, &bones).is_err());
        let mut d = base.clone();
        d.triangles.push([0, 1, 9999]);
        assert!(ClothSim::new(&d, &bones).is_err());
        let mut d = base.clone();
        d.triangles.push([0, 0, 1]);
        assert!(ClothSim::new(&d, &bones).is_err());
        let mut d = base.clone();
        d.positions[3] = Vec3::splat(f32::NAN);
        assert!(ClothSim::new(&d, &bones).is_err());
        let mut d = base.clone();
        d.skin[2] = VertexSkin::rigid(7);
        assert!(ClothSim::new(&d, &bones).is_err(), "osso fora da faixa");
        let mut d = base.clone();
        d.skin[2].weights[0] = f32::NAN;
        assert!(ClothSim::new(&d, &bones).is_err());
        let mut d = base.clone();
        d.params.mass = 0.0;
        assert!(ClothSim::new(&d, &bones).is_err());
        let mut d = base.clone();
        d.params.stretch_compliance = f32::NAN;
        assert!(ClothSim::new(&d, &bones).is_err());
        // ilha desconectada dos fixos
        let mut d = base.clone();
        let n = d.positions.len() as u32;
        d.positions.extend([Vec3::new(5.0, 0.0, 0.0), Vec3::new(5.1, 0.0, 0.0), Vec3::new(5.0, 0.1, 0.0)]);
        d.fixed.extend([false; 3]);
        d.skin.extend([VertexSkin::rigid(0); 3]);
        d.triangles.push([n, n + 1, n + 2]);
        assert!(ClothSim::new(&d, &bones).is_err());
        // pose com NaN e ignorada
        let mut sim = ClothSim::new(&base, &bones).unwrap();
        sim.set_pose(&[Mat4::from_translation(Vec3::splat(f32::NAN))]);
        sim.step(DT, &ColliderSet::new());
        assert!(sim.positions().iter().all(|p| p.is_finite()));
        assert!(ClothSim::new(&ClothProxyDef::default(), &bones).is_err());
    }
}
