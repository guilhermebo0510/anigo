//! Núcleo XPBD (Macklin, Müller, Chentanez 2016; "Small Steps in Physics Simulation", 2019).
//!
//! * Restrições têm **compliance** `alpha` (m/N, inverso de rigidez em unidade física), não
//!   "stiffness" por iteração: `alpha~ = alpha / h^2`,
//!   `dlambda = (-C - alpha~ * lambda) / (grad C . M^-1 . grad C^T + alpha~)`.
//!   Com sub-passos (1 iteração por sub-passo, `h = dt / substeps`) o resultado estático deixa de
//!   depender de quantas iterações/sub-passos se usa: um peso `m` pendurado alonga `alpha * m * g`.
//! * Partículas fixas (`inv_mass == 0`) seguem o alvo animado (`targets`) interpolado dentro do passo.
//! * Restrições: distância (estrutural/flexão), direção ("angle restoration": o vetor pai->filho
//!   volta ao vetor animado), anexo (shape restoration), tether (distância máx./mín. a uma
//!   âncora), limites (max-distance + backstop, estilo Magica Cloth 2) e colisão com [`ColliderSet`].
//! * Determinístico: ordem fixa de restrições, sem hash, sem threads, passo fixo.
//! * Robusto: `dt` inválido é ignorado; deslocamentos e velocidades são limitados; estado
//!   não finito restaura o último quadro bom.

use glam::Vec3;
use serde::{Deserialize, Serialize};

use crate::collider::ColliderSet;
use crate::math::{safe_normalize, valid_dt};

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct XpbdConfig {
    /// Sub-passos por chamada de `step` (Small Steps).
    pub substeps: u32,
    /// Iterações de Gauss-Seidel por sub-passo (1 basta com sub-passos).
    pub iterations: u32,
    pub gravity: Vec3,
    /// Aceleração uniforme extra (vento), m/s^2.
    pub wind: Vec3,
    /// Amortecimento de velocidade global (1/s): `v *= exp(-damping * h)`.
    pub damping: f32,
    /// Velocidade máxima de uma partícula (m/s).
    pub max_speed: f32,
    /// Maior sub-passo `h` aceito (s). Se `dt / substeps` passar disso, o número de sub-passos
    /// aumenta (até 64); o tempo que ainda não couber é descartado (câmera lenta estável em vez de explosão).
    pub max_substep_dt: f32,
    /// Salto de uma partícula fixa (m, por passo) a partir do qual o sistema considera teleporte
    /// e se reinicia no alvo em vez de arrastar a roupa/cabelo.
    pub teleport_threshold: f32,
}

impl Default for XpbdConfig {
    fn default() -> Self {
        Self {
            substeps: 8,
            iterations: 1,
            gravity: Vec3::new(0.0, -9.81, 0.0),
            wind: Vec3::ZERO,
            damping: 1.0,
            max_speed: 30.0,
            max_substep_dt: 1.0 / 120.0,
            teleport_threshold: 1.5,
        }
    }
}

/// Distância bilateral entre duas partículas (estrutural, cisalhamento ou flexão).
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct DistanceConstraint {
    pub a: u32,
    pub b: u32,
    pub rest: f32,
    pub compliance: f32,
    #[serde(skip)]
    pub lambda: f32,
}

/// `x_filho - x_pai = alvo_filho - alvo_pai` (restauração de ângulo; independe da translação do corpo).
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct DirectionConstraint {
    pub parent: u32,
    pub child: u32,
    pub compliance: f32,
    #[serde(skip)]
    pub lambda: Vec3,
}

/// `x = alvo` (restauração de forma/posição).
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct AttachConstraint {
    pub p: u32,
    pub compliance: f32,
    #[serde(skip)]
    pub lambda: Vec3,
}

/// Limita a distância de `p` até a posição da partícula `anchor` a `[min_len, max_len]`.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct TetherConstraint {
    pub p: u32,
    pub anchor: u32,
    pub min_len: f32,
    pub max_len: f32,
    pub compliance: f32,
    #[serde(skip)]
    pub lambda: f32,
}

/// Limites por partícula em relação ao alvo animado (Magica Cloth 2: Max Distance + Backstop).
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct ParticleLimit {
    /// A partícula não se afasta mais que isto do alvo (m). `INFINITY` = sem limite.
    #[serde(with = "crate::math::serde_limit")]
    pub max_distance: f32,
    /// A partícula não entra além de `-backstop` na direção da normal do alvo (m): `dot(x - alvo,
    /// n) >= -backstop`. `INFINITY` = sem backstop.
    #[serde(with = "crate::math::serde_limit")]
    pub backstop: f32,
}

impl Default for ParticleLimit {
    fn default() -> Self {
        Self { max_distance: f32::INFINITY, backstop: f32::INFINITY }
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct XpbdStats {
    /// Maior erro absoluto de distância (m) após o passo.
    pub max_distance_error: f32,
    /// Colisões resolvidas no último passo.
    pub contacts: u32,
    /// Vezes que o estado não finito foi restaurado (acumulado).
    pub nan_restores: u32,
    /// Teleportes detectados (acumulado).
    pub teleports: u32,
}

#[derive(Debug, Clone, Default)]
pub struct XpbdSystem {
    pub config: XpbdConfig,
    // partículas (SoA)
    pub pos: Vec<Vec3>,
    pub vel: Vec<Vec3>,
    pub inv_mass: Vec<f32>,
    pub radius: Vec<f32>,
    /// Fator do amortecimento global por partícula (1 = padrão).
    pub damping_scale: Vec<f32>,
    pub limits: Vec<ParticleLimit>,
    /// Alvo animado (mundo) de cada partícula; partículas fixas seguem exatamente isto.
    pub targets: Vec<Vec3>,
    pub target_normals: Vec<Vec3>,
    prev_targets: Vec<Vec3>,
    // restrições
    pub distance: Vec<DistanceConstraint>,
    pub direction: Vec<DirectionConstraint>,
    pub attach: Vec<AttachConstraint>,
    pub tether: Vec<TetherConstraint>,
    /// Índices de colliders considerados (None = todos).
    pub collider_filter: Option<Vec<usize>>,
    stats: XpbdStats,
    /// Quadros bons para restaurar em caso de NaN.
    snapshot: Vec<Vec3>,
    scratch: Vec<Vec3>,
    old_buf: Vec<Vec3>,
    initialized_targets: bool,
}

impl XpbdSystem {
    pub fn new(config: XpbdConfig) -> Self {
        Self { config, ..Default::default() }
    }

    pub fn len(&self) -> usize {
        self.pos.len()
    }

    pub fn is_empty(&self) -> bool {
        self.pos.is_empty()
    }

    pub fn stats(&self) -> XpbdStats {
        self.stats
    }

    /// Adiciona uma partícula. `mass <= 0` ou não finita = fixa (segue o alvo). Devolve o índice,
    /// ou `None` se a posição não é finita.
    pub fn add_particle(&mut self, position: Vec3, mass: f32, radius: f32) -> Option<usize> {
        if !position.is_finite() {
            return None;
        }
        let inv_mass = if mass.is_finite() && mass > 0.0 { 1.0 / mass } else { 0.0 };
        self.pos.push(position);
        self.vel.push(Vec3::ZERO);
        self.inv_mass.push(inv_mass);
        self.radius.push(if radius.is_finite() { radius.max(0.0) } else { 0.0 });
        self.damping_scale.push(1.0);
        self.limits.push(ParticleLimit::default());
        self.targets.push(position);
        self.target_normals.push(Vec3::Y);
        self.prev_targets.push(position);
        self.snapshot.push(position);
        self.scratch.push(position);
        Some(self.pos.len() - 1)
    }

    pub fn is_pinned(&self, i: usize) -> bool {
        self.inv_mass.get(i).map(|w| *w == 0.0).unwrap_or(false)
    }

    pub fn pin(&mut self, i: usize) {
        if let Some(w) = self.inv_mass.get_mut(i) {
            *w = 0.0;
        }
    }

    pub fn add_distance(&mut self, a: usize, b: usize, rest: f32, compliance: f32) -> bool {
        if a >= self.len() || b >= self.len() || a == b || !(rest.is_finite() && rest >= 0.0) || !compliance.is_finite() || compliance < 0.0 {
            return false;
        }
        self.distance.push(DistanceConstraint { a: a as u32, b: b as u32, rest, compliance, lambda: 0.0 });
        true
    }

    /// Distância com o comprimento de repouso medido nas posições atuais.
    pub fn add_distance_auto(&mut self, a: usize, b: usize, compliance: f32) -> bool {
        match (self.pos.get(a), self.pos.get(b)) {
            (Some(pa), Some(pb)) => {
                let rest = (*pa - *pb).length();
                self.add_distance(a, b, rest, compliance)
            }
            _ => false,
        }
    }

    pub fn add_direction(&mut self, parent: usize, child: usize, compliance: f32) -> bool {
        if parent >= self.len() || child >= self.len() || parent == child || !compliance.is_finite() || compliance < 0.0 {
            return false;
        }
        self.direction.push(DirectionConstraint { parent: parent as u32, child: child as u32, compliance, lambda: Vec3::ZERO });
        true
    }

    pub fn add_attach(&mut self, p: usize, compliance: f32) -> bool {
        if p >= self.len() || !compliance.is_finite() || compliance < 0.0 {
            return false;
        }
        self.attach.push(AttachConstraint { p: p as u32, compliance, lambda: Vec3::ZERO });
        true
    }

    pub fn add_tether(&mut self, p: usize, anchor: usize, min_len: f32, max_len: f32, compliance: f32) -> bool {
        if p >= self.len()
            || anchor >= self.len()
            || p == anchor
            || !(min_len.is_finite() && max_len.is_finite() && min_len >= 0.0 && max_len >= min_len)
            || !compliance.is_finite()
            || compliance < 0.0
        {
            return false;
        }
        self.tether.push(TetherConstraint { p: p as u32, anchor: anchor as u32, min_len, max_len, compliance, lambda: 0.0 });
        true
    }

    /// Define os alvos animados (e normais). Entradas não finitas mantêm o valor anterior; o
    /// comprimento diferente de `len()` ignora o excedente/falta.
    pub fn set_targets(&mut self, targets: &[Vec3], normals: Option<&[Vec3]>) {
        if !self.initialized_targets {
            // primeiro alvo: sem interpolação a partir do ponto de criação
            self.initialized_targets = true;
        }
        for (i, t) in targets.iter().enumerate().take(self.len()) {
            if t.is_finite() {
                self.targets[i] = *t;
            }
        }
        if let Some(normals) = normals {
            for (i, n) in normals.iter().enumerate().take(self.len()) {
                if n.is_finite() && n.length_squared() > 1e-12 {
                    self.target_normals[i] = n.normalize();
                }
            }
        }
    }

    /// Reinicia no alvo (teleporte): posições = alvos, velocidades zero, λ zero.
    pub fn reset_to_targets(&mut self) {
        for i in 0..self.len() {
            self.pos[i] = self.targets[i];
            self.prev_targets[i] = self.targets[i];
            self.snapshot[i] = self.targets[i];
            self.vel[i] = Vec3::ZERO;
        }
        self.zero_lambdas();
    }

    /// Reinicia nas posições atuais (zera velocidades): útil após construir.
    pub fn settle_here(&mut self) {
        for i in 0..self.len() {
            self.targets[i] = self.pos[i];
            self.prev_targets[i] = self.pos[i];
            self.snapshot[i] = self.pos[i];
            self.vel[i] = Vec3::ZERO;
        }
        self.zero_lambdas();
    }

    fn zero_lambdas(&mut self) {
        self.distance.iter_mut().for_each(|c| c.lambda = 0.0);
        self.direction.iter_mut().for_each(|c| c.lambda = Vec3::ZERO);
        self.attach.iter_mut().for_each(|c| c.lambda = Vec3::ZERO);
        self.tether.iter_mut().for_each(|c| c.lambda = 0.0);
    }

    /// Avança `dt` segundos (use [`crate::FixedStepper`] para passo fixo). `dt` inválido é ignorado.
    pub fn step(&mut self, dt: f32, colliders: &ColliderSet) {
        let Some(dt) = valid_dt(dt) else { return };
        if self.is_empty() {
            return;
        }
        let max_h = if self.config.max_substep_dt.is_finite() && self.config.max_substep_dt > 1e-4 {
            self.config.max_substep_dt
        } else {
            1.0 / 120.0
        };
        let needed = (dt / max_h).ceil().min(64.0) as u32;
        let substeps = self.config.substeps.clamp(1, 64).max(needed).min(64);
        let dt = dt.min(substeps as f32 * max_h); // excedente descartado
        let iterations = self.config.iterations.clamp(1, 16);
        let h = dt / substeps as f32;

        // Teleporte: partícula fixa que salta longe demais.
        let threshold = self.config.teleport_threshold;
        if threshold.is_finite() && threshold > 0.0 {
            let jumped = (0..self.len()).any(|i| self.inv_mass[i] == 0.0 && (self.targets[i] - self.prev_targets[i]).length() > threshold);
            if jumped {
                self.stats.teleports += 1;
                self.reset_to_targets();
                return;
            }
        }

        let g = self.config.gravity + self.config.wind;
        let max_speed = if self.config.max_speed.is_finite() && self.config.max_speed > 0.0 { self.config.max_speed } else { 30.0 };
        let n = self.len();
        let mut contacts_total = 0u32;

        for s in 0..substeps {
            let frac = (s + 1) as f32 / substeps as f32;
            for i in 0..n {
                self.scratch[i] = self.prev_targets[i].lerp(self.targets[i], frac);
            }
            // 1) predição
            let mut old = std::mem::take(&mut self.old_buf);
            old.clear();
            old.extend_from_slice(&self.pos);
            for i in 0..n {
                if self.inv_mass[i] == 0.0 {
                    self.pos[i] = self.scratch[i];
                    self.vel[i] = Vec3::ZERO;
                    continue;
                }
                let damp = (-(self.config.damping.max(0.0) * self.damping_scale[i].max(0.0)) * h).exp();
                let mut v = (self.vel[i] + g * h) * damp;
                let sp = v.length();
                if sp > max_speed {
                    v *= max_speed / sp;
                }
                self.pos[i] += v * h;
                self.vel[i] = v;
            }
            self.zero_lambdas();
            // 2) restrições + colisão
            for it in 0..iterations {
                self.solve_constraints(h);
                // limites e colisão só depois da última iteração: continuam sendo a última palavra
                // do sub-passo (nada atravessa), mas custam 1x em vez de `iterations`x.
                if it + 1 == iterations {
                    contacts_total += self.solve_limits_and_collisions(colliders, &old);
                }
            }
            // 3) velocidades
            for i in 0..n {
                if self.inv_mass[i] == 0.0 {
                    continue;
                }
                let mut v = (self.pos[i] - old[i]) / h;
                let sp = v.length();
                if sp > max_speed {
                    v *= max_speed / sp;
                }
                self.vel[i] = v;
            }
            self.old_buf = old;
        }
        for i in 0..n {
            self.prev_targets[i] = self.targets[i];
        }
        self.stats.contacts = contacts_total;

        // Sanidade: NaN/infinito restaura o último quadro bom.
        if self.pos.iter().any(|p| !p.is_finite()) || self.vel.iter().any(|v| !v.is_finite()) {
            self.stats.nan_restores += 1;
            for i in 0..n {
                self.pos[i] = if self.snapshot[i].is_finite() { self.snapshot[i] } else { self.targets[i] };
                self.vel[i] = Vec3::ZERO;
            }
            self.zero_lambdas();
        } else {
            self.snapshot.copy_from_slice(&self.pos);
        }
        self.stats.max_distance_error = self.max_distance_error();
    }

    fn solve_constraints(&mut self, h: f32) {
        let inv_h2 = 1.0 / (h * h);
        // anexo (shape restoration)
        for k in 0..self.attach.len() {
            let c = self.attach[k];
            let i = c.p as usize;
            let w = self.inv_mass[i];
            if w == 0.0 {
                continue;
            }
            let at = c.compliance * inv_h2;
            let cv = self.pos[i] - self.scratch[i];
            let dl = (-cv - at * c.lambda) / (w + at);
            self.pos[i] += dl * w;
            self.attach[k].lambda += dl;
        }
        // direção (angle restoration)
        for k in 0..self.direction.len() {
            let c = self.direction[k];
            let (p, ch) = (c.parent as usize, c.child as usize);
            let (wp, wc) = (self.inv_mass[p], self.inv_mass[ch]);
            let wsum = wp + wc;
            if wsum == 0.0 {
                continue;
            }
            let at = c.compliance * inv_h2;
            let cv = (self.pos[ch] - self.pos[p]) - (self.scratch[ch] - self.scratch[p]);
            let dl = (-cv - at * c.lambda) / (wsum + at);
            self.pos[ch] += dl * wc;
            self.pos[p] -= dl * wp;
            self.direction[k].lambda += dl;
        }
        // distâncias (estrutural, cisalhamento e flexão) por último: as mais rígidas
        for k in 0..self.distance.len() {
            let c = self.distance[k];
            let (a, b) = (c.a as usize, c.b as usize);
            let (wa, wb) = (self.inv_mass[a], self.inv_mass[b]);
            let wsum = wa + wb;
            if wsum == 0.0 {
                continue;
            }
            let d = self.pos[a] - self.pos[b];
            let len = d.length();
            if !(len > 1e-9) {
                continue;
            }
            let n = d / len;
            let at = c.compliance * inv_h2;
            let cv = len - c.rest;
            let dl = (-cv - at * c.lambda) / (wsum + at);
            self.pos[a] += n * (dl * wa);
            self.pos[b] -= n * (dl * wb);
            self.distance[k].lambda += dl;
        }
        // tethers (desigualdade)
        for k in 0..self.tether.len() {
            let c = self.tether[k];
            let (p, a) = (c.p as usize, c.anchor as usize);
            let (wp, wa) = (self.inv_mass[p], self.inv_mass[a]);
            let wsum = wp + wa;
            if wsum == 0.0 {
                continue;
            }
            let d = self.pos[p] - self.pos[a];
            let len = d.length();
            if !(len > 1e-9) {
                continue;
            }
            let cv = if len > c.max_len {
                len - c.max_len
            } else if len < c.min_len {
                len - c.min_len
            } else {
                continue;
            };
            let n = d / len;
            let at = c.compliance * inv_h2;
            let dl = (-cv - at * c.lambda) / (wsum + at);
            self.pos[p] += n * (dl * wp);
            self.pos[a] -= n * (dl * wa);
            self.tether[k].lambda += dl;
        }
    }

    /// Max-distance, backstop e colisão. Devolve o número de contatos.
    fn solve_limits_and_collisions(&mut self, colliders: &ColliderSet, old: &[Vec3]) -> u32 {
        let mut contacts = 0u32;
        for i in 0..self.len() {
            if self.inv_mass[i] == 0.0 {
                continue;
            }
            let lim = self.limits[i];
            let t = self.scratch[i];
            if lim.max_distance.is_finite() {
                let d = self.pos[i] - t;
                let len = d.length();
                if len > lim.max_distance && len > 1e-9 {
                    self.pos[i] = t + d * (lim.max_distance / len);
                }
            }
            if lim.backstop.is_finite() {
                let n = self.target_normals[i];
                let s = (self.pos[i] - t).dot(n) + lim.backstop;
                if s < 0.0 {
                    self.pos[i] -= n * s;
                }
            }
            if !colliders.is_empty() {
                let before = self.pos[i];
                let (after, hit, friction) = match &self.collider_filter {
                    Some(f) => colliders.push_out(before, self.radius[i], Some(f)),
                    None => colliders.push_out(before, self.radius[i], None),
                };
                if hit {
                    contacts += 1;
                    let push = after - before;
                    let mut p = after;
                    // atrito de Coulomb simplificado: reduz o deslocamento tangencial do sub-passo
                    let normal = safe_normalize(push, Vec3::Y);
                    let disp = p - old[i];
                    let tangential = disp - normal * disp.dot(normal);
                    p -= tangential * friction.clamp(0.0, 1.0);
                    self.pos[i] = p;
                }
            }
        }
        contacts
    }

    pub fn max_distance_error(&self) -> f32 {
        self.distance
            .iter()
            .map(|c| ((self.pos[c.a as usize] - self.pos[c.b as usize]).length() - c.rest).abs())
            .fold(0.0, f32::max)
    }

    /// Maior alongamento relativo `(len - rest) / rest` entre as restrições de distância com rest > 0.
    pub fn max_stretch_ratio(&self) -> f32 {
        self.distance
            .iter()
            .filter(|c| c.rest > 1e-9)
            .map(|c| ((self.pos[c.a as usize] - self.pos[c.b as usize]).length() - c.rest) / c.rest)
            .fold(0.0, f32::max)
    }

    pub fn kinetic_energy(&self) -> f32 {
        (0..self.len())
            .filter(|&i| self.inv_mass[i] > 0.0)
            .map(|i| 0.5 * self.vel[i].length_squared() / self.inv_mass[i])
            .sum()
    }

    /// Energia potencial gravitacional `-m g . x` (zero em x = 0).
    pub fn potential_energy(&self) -> f32 {
        (0..self.len())
            .filter(|&i| self.inv_mass[i] > 0.0)
            .map(|i| -self.config.gravity.dot(self.pos[i]) / self.inv_mass[i])
            .sum()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::collider::{Collider, ColliderShape};
    use crate::math::state_hash;

    const DT: f32 = 1.0 / 60.0;

    fn run(sys: &mut XpbdSystem, seconds: f32, colliders: &ColliderSet) {
        for _ in 0..(seconds / DT).round() as usize {
            sys.step(DT, colliders);
        }
    }

    fn no_colliders() -> ColliderSet {
        ColliderSet::new()
    }

    /// Cadeia vertical pendurada: partícula 0 fixa em y=0, N livres de massa 1, segmento `seg`.
    fn hanging_chain(n: usize, seg: f32, compliance: f32, cfg: XpbdConfig) -> XpbdSystem {
        let mut s = XpbdSystem::new(cfg);
        s.add_particle(Vec3::ZERO, 0.0, 0.0);
        for k in 1..=n {
            s.add_particle(Vec3::new(0.0, -(k as f32) * seg, 0.0), 1.0, 0.0);
            s.add_distance(k - 1, k, seg, compliance);
        }
        s
    }

    #[test]
    fn queda_livre_segue_meio_g_t_quadrado() {
        let mut s = XpbdSystem::new(XpbdConfig { damping: 0.0, ..Default::default() });
        // Perto da origem: a velocidade vem de diferenças de posição f32 (ulp pequeno).
        s.add_particle(Vec3::new(0.0, 0.5, 0.0), 1.0, 0.0);
        run(&mut s, 1.0, &no_colliders());
        let drop = 0.5 - s.pos[0].y;
        eprintln!("queda livre 1 s: {drop:.4} m (teorico 4.905)");
        assert!((drop - 4.905).abs() / 4.905 < 0.02);
        assert!((s.vel[0].y + 9.81).abs() < 0.2);
    }

    #[test]
    fn pendulo_rigido_conserva_energia() {
        // Haste rígida de 1 m solta na horizontal. Sem amortecimento.
        let mut s = XpbdSystem::new(XpbdConfig { damping: 0.0, substeps: 20, ..Default::default() });
        s.add_particle(Vec3::ZERO, 0.0, 0.0);
        s.add_particle(Vec3::new(1.0, 0.0, 0.0), 1.0, 0.0);
        s.add_distance(0, 1, 1.0, 0.0);
        let e0 = s.kinetic_energy() + s.potential_energy();
        let mut worst = 0.0f32;
        let mut max_gain = 0.0f32;
        let mut min_y = 0.0f32;
        for _ in 0..240 {
            s.step(DT, &no_colliders());
            let e = s.kinetic_energy() + s.potential_energy();
            worst = worst.max((e - e0).abs());
            max_gain = max_gain.max(e - e0);
            min_y = min_y.min(s.pos[1].y);
            assert!((s.pos[1].length() - 1.0).abs() < 1e-3);
        }
        eprintln!(
            "pendulo: |dE|max = {worst:.4} J, ganho max = {max_gain:.4} J sobre m*g*L = 9.81 J ({:.2}% perda), y minimo {min_y:.4}",
            worst / 9.81 * 100.0
        );
        // XPBD e dissipativo (projecao de posicao): nunca ganha energia e perde pouco.
        assert!(max_gain / 9.81 < 0.01, "ganho de energia {max_gain}");
        assert!(worst / 9.81 < 0.10);
        assert!(min_y < -0.95, "chega perto do ponto mais baixo (-1)");
    }

    #[test]
    fn alongamento_estatico_e_alpha_m_g_independente_de_iteracoes_e_substeps() {
        // Teoria XPBD: segmento k (do topo) carrega (N-k+1) massas: stretch_k = alpha * (N-k+1) * g.
        let n = 6usize;
        let alpha = 1e-3f32;
        let seg = 0.2f32;
        let mut results = Vec::new();
        for (substeps, iterations) in [(10u32, 1u32), (10, 4), (20, 1), (40, 1)] {
            let cfg = XpbdConfig { substeps, iterations, damping: 3.0, ..Default::default() };
            let mut s = hanging_chain(n, seg, alpha, cfg);
            run(&mut s, 15.0, &no_colliders());
            let mut max_rel = 0.0f32;
            let mut stretches = Vec::new();
            for k in 1..=n {
                let len = (s.pos[k] - s.pos[k - 1]).length();
                let stretch = len - seg;
                let theory = alpha * (n - k + 1) as f32 * 9.81;
                stretches.push(stretch);
                max_rel = max_rel.max((stretch - theory).abs() / theory);
            }
            eprintln!("substeps={substeps} it={iterations}: alongamentos {stretches:?}, erro relativo max vs alpha*m*g = {:.2}%", max_rel * 100.0);
            assert!(max_rel < 0.05, "substeps={substeps} it={iterations}: {max_rel}");
            results.push(stretches);
        }
        // independencia: todas as configuracoes concordam entre si
        for r in &results[1..] {
            for (a, b) in results[0].iter().zip(r) {
                assert!((a - b).abs() / a.abs() < 0.04, "{a} vs {b}");
            }
        }
    }

    #[test]
    fn compliance_zero_e_praticamente_rigido() {
        let mut s = hanging_chain(10, 0.1, 0.0, XpbdConfig { substeps: 10, damping: 1.0, ..Default::default() });
        run(&mut s, 5.0, &no_colliders());
        eprintln!("corda rigida: erro de distancia max {:.2e} m, stretch {:.2e}", s.max_distance_error(), s.max_stretch_ratio());
        assert!(s.max_distance_error() < 2e-3);
        // pendurada: chega na vertical
        assert!(s.pos[10].x.abs() < 1e-3 && (s.pos[10].y + 1.0).abs() < 2e-3);
    }

    #[test]
    fn estavel_com_dt_grande_e_entradas_extremas() {
        for dt in [0.1f32, 0.5, 2.0, 100.0] {
            let mut s = hanging_chain(8, 0.1, 1e-4, XpbdConfig::default());
            s.pos[8] += Vec3::new(0.7, 0.0, 0.3); // perturba
            for _ in 0..60 {
                s.step(dt, &no_colliders());
                assert!(s.pos.iter().all(|p| p.is_finite() && p.length() < 10.0), "dt={dt}");
            }
        }
        // dt invalido: nada muda
        let mut s = hanging_chain(4, 0.1, 1e-4, XpbdConfig::default());
        let before = s.pos.clone();
        for dt in [0.0, -1.0, f32::NAN, f32::INFINITY] {
            s.step(dt, &no_colliders());
        }
        assert_eq!(before, s.pos);
    }

    #[test]
    fn determinismo_bit_a_bit() {
        let make = || {
            let mut s = hanging_chain(12, 0.05, 1e-5, XpbdConfig::default());
            for i in 0..200 {
                let t = i as f32 * DT;
                let targets: Vec<Vec3> = (0..s.len()).map(|k| if k == 0 { Vec3::new(t.sin() * 0.3, 0.0, 0.0) } else { s.targets[k] }).collect();
                s.set_targets(&targets, None);
                s.step(DT, &no_colliders());
            }
            state_hash(&s.pos)
        };
        assert_eq!(make(), make());
    }

    #[test]
    fn colisao_esfera_e_plano_sem_atravessar() {
        let mut set = ColliderSet::new();
        set.add(Collider::fixed(ColliderShape::Sphere { center: Vec3::ZERO, radius: 0.5 }));
        set.add(Collider::fixed(ColliderShape::Plane { point: Vec3::new(0.0, -2.0, 0.0), normal: Vec3::Y }));
        let mut s = XpbdSystem::new(XpbdConfig::default());
        // partículas caindo sobre a esfera (raio 0.5) em varias posicoes laterais
        for k in 0..9 {
            let x = (k as f32 - 4.0) * 0.1;
            s.add_particle(Vec3::new(x, 1.5, 0.05 * k as f32), 1.0, 0.02);
        }
        let mut worst = 0.0f32;
        for _ in 0..600 {
            s.step(DT, &set);
            for p in &s.pos {
                let pen = 0.5 + 0.02 - p.length();
                if p.y > -1.0 {
                    worst = worst.max(pen);
                }
                assert!(p.y > -2.0 + 0.02 - 1e-3, "atravessou o plano: {p:?}");
            }
        }
        eprintln!("penetracao maxima na esfera: {worst:.5} m");
        assert!(worst < 1e-3);
        assert!(s.stats().contacts > 0 || s.pos.iter().all(|p| p.y <= -1.9));
    }

    #[test]
    fn tether_limita_distancia_a_ancora() {
        let mut s = XpbdSystem::new(XpbdConfig { damping: 0.0, ..Default::default() });
        s.add_particle(Vec3::ZERO, 0.0, 0.0);
        s.add_particle(Vec3::new(0.0, -0.1, 0.0), 1.0, 0.0);
        s.add_tether(1, 0, 0.0, 0.5, 0.0);
        run(&mut s, 3.0, &no_colliders());
        let d = (s.pos[1] - s.pos[0]).length();
        assert!((d - 0.5).abs() < 2e-3, "caiu ate o limite do tether: {d}");
        // min_len: nao colapsa abaixo do minimo
        let mut s = XpbdSystem::new(XpbdConfig { gravity: Vec3::ZERO, ..Default::default() });
        s.add_particle(Vec3::ZERO, 0.0, 0.0);
        s.add_particle(Vec3::new(0.0, -0.5, 0.0), 1.0, 0.0);
        s.add_tether(1, 0, 0.4, 1.0, 0.0);
        s.vel[1] = Vec3::new(0.0, 5.0, 0.0);
        run(&mut s, 1.0, &no_colliders());
        assert!((s.pos[1] - s.pos[0]).length() >= 0.4 - 2e-3);
    }

    #[test]
    fn max_distance_e_backstop_em_relacao_ao_alvo() {
        let cfg = XpbdConfig { gravity: Vec3::new(0.0, 0.0, -9.81), damping: 0.0, ..Default::default() };
        let mut s = XpbdSystem::new(cfg);
        // normal do alvo +Z; gravidade empurra para -Z (para "dentro" do corpo)
        s.add_particle(Vec3::ZERO, 1.0, 0.0);
        s.target_normals[0] = Vec3::Z;
        s.limits[0] = ParticleLimit { max_distance: 0.3, backstop: 0.02 };
        run(&mut s, 2.0, &no_colliders());
        assert!(s.pos[0].z >= -0.02 - 1e-4, "backstop segura: {:?}", s.pos[0]);
        // sem backstop, so max_distance
        let mut s2 = XpbdSystem::new(cfg);
        s2.add_particle(Vec3::ZERO, 1.0, 0.0);
        s2.limits[0] = ParticleLimit { max_distance: 0.3, backstop: f32::INFINITY };
        run(&mut s2, 2.0, &no_colliders());
        assert!((s2.pos[0].length() - 0.3).abs() < 1e-3);
    }

    #[test]
    fn restauracao_de_angulo_segue_o_alvo_animado() {
        // Cadeia horizontal presa na origem; alvo gira 90 graus; sem gravidade.
        let cfg = XpbdConfig { gravity: Vec3::ZERO, damping: 4.0, substeps: 10, ..Default::default() };
        let mut s = XpbdSystem::new(cfg);
        s.add_particle(Vec3::ZERO, 0.0, 0.0);
        for k in 1..=4 {
            s.add_particle(Vec3::new(k as f32 * 0.1, 0.0, 0.0), 1.0, 0.0);
            s.add_distance(k - 1, k, 0.1, 0.0);
            s.add_direction(k - 1, k, 1e-5);
        }
        let rotated: Vec<Vec3> = (0..5).map(|k| Vec3::new(0.0, k as f32 * 0.1, 0.0)).collect();
        s.set_targets(&rotated, None);
        // salto grande (0.4 m) < teleport_threshold (1.5)
        run(&mut s, 3.0, &no_colliders());
        let err = (s.pos[4] - rotated[4]).length();
        eprintln!("angle restoration: erro da ponta ao alvo {err:.4} m");
        assert!(err < 0.02);
    }

    #[test]
    fn teleporte_reinicia_em_vez_de_arrastar() {
        let mut s = hanging_chain(6, 0.1, 1e-4, XpbdConfig::default());
        run(&mut s, 1.0, &no_colliders());
        let mut targets = s.targets.clone();
        targets[0] = Vec3::new(500.0, 0.0, 0.0);
        s.set_targets(&targets, None);
        s.step(DT, &no_colliders());
        assert_eq!(s.stats().teleports, 1);
        assert!(s.pos.iter().all(|p| p.is_finite()));
        assert!((s.pos[0].x - 500.0).abs() < 1e-3);
        // restante vem junto (alvos das livres seguem onde estavam, mas a ancora esta em 500: o
        // reset coloca TUDO nos alvos; a cadeia e reconstruida pelo chamador via set_targets)
    }

    #[test]
    fn nan_injetado_e_recuperado() {
        let mut s = hanging_chain(5, 0.1, 1e-4, XpbdConfig::default());
        run(&mut s, 1.0, &no_colliders());
        s.pos[3] = Vec3::splat(f32::NAN);
        s.step(DT, &no_colliders());
        assert!(s.pos.iter().all(|p| p.is_finite()));
        assert_eq!(s.stats().nan_restores, 1);
        run(&mut s, 1.0, &no_colliders());
        assert!(s.pos.iter().all(|p| p.is_finite()));
    }

    #[test]
    fn construcao_rejeita_entradas_invalidas_sem_panic() {
        let mut s = XpbdSystem::new(XpbdConfig::default());
        assert!(s.add_particle(Vec3::splat(f32::NAN), 1.0, 0.0).is_none());
        let a = s.add_particle(Vec3::ZERO, 1.0, 0.0).unwrap();
        let b = s.add_particle(Vec3::X, 1.0, 0.0).unwrap();
        assert!(!s.add_distance(a, a, 1.0, 0.0));
        assert!(!s.add_distance(a, 99, 1.0, 0.0));
        assert!(!s.add_distance(a, b, f32::NAN, 0.0));
        assert!(!s.add_distance(a, b, 1.0, -1.0));
        assert!(!s.add_direction(a, a, 0.0));
        assert!(!s.add_attach(99, 0.0));
        assert!(!s.add_tether(a, b, 2.0, 1.0, 0.0));
        s.set_targets(&[Vec3::splat(f32::NAN), Vec3::ONE, Vec3::ZERO, Vec3::ZERO], Some(&[Vec3::ZERO]));
        assert_eq!(s.targets[0], Vec3::ZERO, "NaN ignorado");
        assert_eq!(s.targets[1], Vec3::ONE);
        // massa invalida = fixa
        let c = s.add_particle(Vec3::Y, f32::NAN, 0.0).unwrap();
        assert!(s.is_pinned(c));
        s.step(DT, &no_colliders());
        // sistema vazio
        let mut e = XpbdSystem::new(XpbdConfig::default());
        e.step(DT, &no_colliders());
    }
}
