//! Shape keys (blend shapes) do `EditMesh` e ponte com os morph targets
//! esparsos do núcleo (`anigo_core::morph`).
//!
//! ## Modelo
//!
//! Um [`ShapeKeySet`] guarda, para uma malha base ("basis"), uma lista de
//! [`ShapeKey`]. Cada chave guarda **deltas esparsos** de posição em relação à
//! base (`(vértice, Δ)`, ordenados e sem repetição). O valor avaliado é
//!
//! `p'(v) = p_basis(v) + Σ_k clamp(value_k, min_k, max_k) · m_k(v) · (Δ_k(v) − Δ_ref(k)(v))`
//!
//! onde `m_k(v)` é a máscara de vertex group (1 se não houver) e `ref(k)` é a
//! chave de referência `relative_to` (`None` = a base, `Δ_ref = 0`). Como no
//! Blender, a referência usa **só os deltas dela** (a própria `relative_to` da
//! referência é ignorada — não há recursão, logo não há ciclos).
//!
//! ## Ponte com o núcleo
//!
//! O renderer usa vértices divididos por (posição, UV, normal)
//! (`convert::to_core_mesh`), então um vértice de edição pode virar vários
//! vértices do núcleo. [`ShapeKeySet::to_core_sparse_morph_set`] replica essa
//! divisão para expandir cada delta a todos os vértices do núcleo e, opcionalmente,
//! calcula a `delta_normal` exata (normais por corner da forma deformada menos as
//! da base). Normais **customizadas** não acompanham a deformação (delta 0) — é
//! intencional: a normal de sombreamento editada à mão não deve girar sozinha.

use crate::attributes::MirrorAxis;
use crate::convert::ToCoreOptions;
use crate::error::{ModelError, ModelResult};
use crate::mesh::EditMesh;
use crate::normals::compute_corner_normals;
use crate::normals_tools::PointHash;
use anigo_core::morph::{SparseMorphDelta, SparseMorphSet};
use glam::Vec3;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, HashMap};

/// Uma shape key com deltas esparsos de posição.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ShapeKey {
    pub name: String,
    /// Valor atual (influência). Limitado a `[min, max]` na avaliação.
    pub value: f32,
    /// Deltas esparsos `(vértice, Δposição)`, ordenados por vértice e únicos.
    pub deltas: Vec<(u32, Vec3)>,
    /// Chave de referência (índice em `ShapeKeySet::keys`); `None` = base.
    pub relative_to: Option<usize>,
    /// Máscara esparsa (vertex group): vértices ausentes têm peso 0. `None` = sem máscara.
    pub vertex_group_mask: Option<Vec<(u32, f32)>>,
    pub min: f32,
    pub max: f32,
}

impl ShapeKey {
    /// Chave vazia, valor 0, faixa `[0, 1]`, relativa à base.
    pub fn new(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            value: 0.0,
            deltas: Vec::new(),
            relative_to: None,
            vertex_group_mask: None,
            min: 0.0,
            max: 1.0,
        }
    }

    /// Captura a diferença `shaped − basis` como chave esparsa: entram os
    /// vértices com `|Δ| > threshold`. As malhas devem ter o mesmo número de
    /// vértices; `threshold` finito e ≥ 0.
    pub fn capture(basis: &EditMesh, shaped: &EditMesh, name: impl Into<String>, threshold: f32) -> ModelResult<ShapeKey> {
        if basis.vert_count() != shaped.vert_count() {
            return Err(ModelError::UnsupportedTopology(format!(
                "contagem de vértices difere (base {}, forma {})",
                basis.vert_count(),
                shaped.vert_count()
            )));
        }
        if !threshold.is_finite() || threshold < 0.0 {
            return Err(ModelError::InvalidParameter("threshold inválido".into()));
        }
        let mut key = ShapeKey::new(name);
        for (i, (b, s)) in basis.verts.iter().zip(&shaped.verts).enumerate() {
            let d = s.co - b.co;
            if !d.is_finite() {
                return Err(ModelError::Numeric(format!("delta não finito no vértice {i}")));
            }
            if d.length() > threshold {
                key.deltas.push((i as u32, d));
            }
        }
        Ok(key)
    }

    /// Peso da máscara no vértice `v` (1 sem máscara, 0 se ausente).
    fn mask_weight(&self, v: u32) -> f32 {
        match &self.vertex_group_mask {
            None => 1.0,
            Some(m) => m
                .binary_search_by_key(&v, |x| x.0)
                .map(|i| m[i].1)
                .unwrap_or(0.0),
        }
    }
}

/// Conjunto de shape keys de uma malha.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ShapeKeySet {
    /// `content_hash` da base no momento da criação (detecta base editada).
    pub basis_hash: u64,
    /// Número de vértices da base (as chaves só valem para essa contagem).
    pub vert_count: u32,
    pub keys: Vec<ShapeKey>,
}

/// Opções da conversão para morphs do núcleo.
#[derive(Debug, Clone)]
pub struct MorphExportOptions {
    /// Opções usadas por `to_core_mesh` (define a divisão de vértices e as normais).
    pub core: ToCoreOptions,
    /// Preencher `delta_normal` (normais por corner deformado − base).
    pub with_normals: bool,
    /// Deltas de posição menores que isto (e sem delta de normal) são omitidos.
    pub min_delta: f32,
}

impl Default for MorphExportOptions {
    fn default() -> Self {
        Self {
            core: ToCoreOptions::default(),
            with_normals: true,
            min_delta: 1.0e-7,
        }
    }
}

/// Vértice do núcleo → (vértice de edição, face e corner de origem), na mesma
/// ordem de criação de `convert::to_core_mesh` (faces ocultas são puladas).
struct CoreVertex {
    edit_vert: u32,
    face: u32,
    corner: u32,
}

/// Replica a divisão de vértices de `convert::to_core_mesh`: chave
/// (vértice, UV, normal efetiva) por corner, na ordem face→corner.
fn core_vertex_map(mesh: &EditMesh, opts: &ToCoreOptions) -> ModelResult<Vec<CoreVertex>> {
    mesh.validate()?;
    let cn = compute_corner_normals(mesh, &opts.normals);
    let mut lookup: HashMap<(u32, [u32; 2], [u32; 3]), u32> = HashMap::new();
    let mut out: Vec<CoreVertex> = Vec::new();
    for (fi, face) in mesh.faces.iter().enumerate() {
        if face.hidden {
            continue;
        }
        for (ci, c) in face.corners.iter().enumerate() {
            let n = cn[fi][ci];
            let key = (
                c.vert,
                [c.uv.x.to_bits(), c.uv.y.to_bits()],
                [n.x.to_bits(), n.y.to_bits(), n.z.to_bits()],
            );
            lookup.entry(key).or_insert_with(|| {
                out.push(CoreVertex {
                    edit_vert: c.vert,
                    face: fi as u32,
                    corner: ci as u32,
                });
                (out.len() - 1) as u32
            });
        }
    }
    Ok(out)
}

impl ShapeKeySet {
    /// Conjunto vazio para a malha base `basis`.
    pub fn new(basis: &EditMesh) -> Self {
        Self {
            basis_hash: basis.content_hash(),
            vert_count: basis.vert_count() as u32,
            keys: Vec::new(),
        }
    }

    /// Conjunto com uma única chave capturada de `shaped` (ver [`ShapeKey::capture`]).
    pub fn capture(basis: &EditMesh, shaped: &EditMesh, name: impl Into<String>, threshold: f32) -> ModelResult<Self> {
        let mut set = Self::new(basis);
        set.add_capture(basis, shaped, name, threshold)?;
        Ok(set)
    }

    /// Captura `shaped − basis` e acrescenta como nova chave; devolve o índice.
    /// Nome repetido é erro.
    pub fn add_capture(
        &mut self,
        basis: &EditMesh,
        shaped: &EditMesh,
        name: impl Into<String>,
        threshold: f32,
    ) -> ModelResult<usize> {
        self.check_basis(basis)?;
        let key = ShapeKey::capture(basis, shaped, name, threshold)?;
        self.add_key(key)
    }

    /// Acrescenta uma chave já pronta (validada contra o conjunto).
    pub fn add_key(&mut self, key: ShapeKey) -> ModelResult<usize> {
        if self.keys.iter().any(|k| k.name == key.name) {
            return Err(ModelError::InvalidParameter(format!("shape key '{}' já existe", key.name)));
        }
        self.validate_key(&key, self.keys.len() + 1)?;
        self.keys.push(key);
        Ok(self.keys.len() - 1)
    }

    /// Índice da chave pelo nome.
    pub fn find(&self, name: &str) -> Option<usize> {
        self.keys.iter().position(|k| k.name == name)
    }

    /// Remove a chave `index`. Chaves que a tinham como `relative_to` voltam a
    /// ser relativas à base; as demais referências são reindexadas.
    pub fn remove_key(&mut self, index: usize) -> ModelResult<ShapeKey> {
        if index >= self.keys.len() {
            return Err(ModelError::InvalidParameter(format!(
                "shape key {index} fora do intervalo ({})",
                self.keys.len()
            )));
        }
        let removed = self.keys.remove(index);
        for k in &mut self.keys {
            k.relative_to = match k.relative_to {
                Some(r) if r == index => None,
                Some(r) if r > index => Some(r - 1),
                other => other,
            };
        }
        Ok(removed)
    }

    /// `true` se `basis` ainda tem o mesmo conteúdo da criação do conjunto.
    pub fn is_basis_current(&self, basis: &EditMesh) -> bool {
        basis.vert_count() as u32 == self.vert_count && basis.content_hash() == self.basis_hash
    }

    /// Reancora o conjunto na base atual (atualiza hash e contagem); só é
    /// permitido se a contagem de vértices não mudou.
    pub fn rebase(&mut self, basis: &EditMesh) -> ModelResult<()> {
        self.check_basis(basis)?;
        self.basis_hash = basis.content_hash();
        Ok(())
    }

    fn check_basis(&self, basis: &EditMesh) -> ModelResult<()> {
        if basis.vert_count() as u32 != self.vert_count {
            return Err(ModelError::UnsupportedTopology(format!(
                "a base tem {} vértices, o conjunto espera {}",
                basis.vert_count(),
                self.vert_count
            )));
        }
        Ok(())
    }

    fn validate_key(&self, key: &ShapeKey, key_count: usize) -> ModelResult<()> {
        if !key.min.is_finite() || !key.max.is_finite() || key.min > key.max {
            return Err(ModelError::InvalidParameter(format!(
                "'{}': faixa [{}, {}] inválida",
                key.name, key.min, key.max
            )));
        }
        if !key.value.is_finite() {
            return Err(ModelError::InvalidParameter(format!("'{}': valor não finito", key.name)));
        }
        if let Some(r) = key.relative_to {
            if r >= key_count {
                return Err(ModelError::InvalidParameter(format!(
                    "'{}': relative_to {r} fora do intervalo",
                    key.name
                )));
            }
        }
        let mut prev: Option<u32> = None;
        for &(v, d) in &key.deltas {
            if v >= self.vert_count {
                return Err(ModelError::VertexOutOfRange(v, self.vert_count as usize));
            }
            if prev.map(|p| v <= p).unwrap_or(false) {
                return Err(ModelError::InvalidParameter(format!(
                    "'{}': deltas devem estar ordenados e sem repetição",
                    key.name
                )));
            }
            if !d.is_finite() {
                return Err(ModelError::Numeric(format!("'{}': delta não finito", key.name)));
            }
            prev = Some(v);
        }
        if let Some(m) = &key.vertex_group_mask {
            let mut prev: Option<u32> = None;
            for &(v, w) in m {
                if v >= self.vert_count {
                    return Err(ModelError::VertexOutOfRange(v, self.vert_count as usize));
                }
                if prev.map(|p| v <= p).unwrap_or(false) || !w.is_finite() {
                    return Err(ModelError::InvalidParameter(format!(
                        "'{}': máscara inválida (ordem/NaN)",
                        key.name
                    )));
                }
                prev = Some(v);
            }
        }
        Ok(())
    }

    /// Valida o conjunto inteiro (índices, ordenação, finitude, referências,
    /// nomes únicos). A avaliação chama isto antes de tocar na malha.
    pub fn validate(&self) -> ModelResult<()> {
        for (i, k) in self.keys.iter().enumerate() {
            if self.keys[..i].iter().any(|o| o.name == k.name) {
                return Err(ModelError::InvalidParameter(format!("nome '{}' repetido", k.name)));
            }
            self.validate_key(k, self.keys.len())?;
            if k.relative_to == Some(i) {
                return Err(ModelError::InvalidParameter(format!(
                    "'{}' não pode ser relativa a si mesma",
                    k.name
                )));
            }
        }
        Ok(())
    }

    /// Deltas efetivos da chave `k` a valor 1: `m(v)·(Δ_k(v) − Δ_ref(v))`,
    /// esparsos e ordenados (zeros exatos omitidos).
    pub fn effective_deltas(&self, k: usize) -> ModelResult<Vec<(u32, Vec3)>> {
        let key = self
            .keys
            .get(k)
            .ok_or_else(|| ModelError::InvalidParameter(format!("shape key {k} inexistente")))?;
        let mut merged: BTreeMap<u32, Vec3> = BTreeMap::new();
        for &(v, d) in &key.deltas {
            merged.insert(v, d);
        }
        if let Some(r) = key.relative_to {
            let rk = self
                .keys
                .get(r)
                .ok_or_else(|| ModelError::InvalidParameter(format!("relative_to {r} inexistente")))?;
            for &(v, d) in &rk.deltas {
                *merged.entry(v).or_insert(Vec3::ZERO) -= d;
            }
        }
        Ok(merged
            .into_iter()
            .filter_map(|(v, d)| {
                let d = d * key.mask_weight(v);
                (d != Vec3::ZERO).then_some((v, d))
            })
            .collect())
    }

    /// Avalia o conjunto sobre `basis` usando os `value` das chaves.
    pub fn apply(&self, basis: &EditMesh) -> ModelResult<EditMesh> {
        let values: Vec<f32> = self.keys.iter().map(|k| k.value).collect();
        self.apply_values(basis, &values)
    }

    /// Como [`Self::apply`], mas com valores explícitos (um por chave), úteis
    /// para animação/pré-visualização sem mutar o conjunto. Cada valor é
    /// limitado ao `[min, max]` da chave. Devolve uma nova malha; `basis` não é tocada.
    pub fn apply_values(&self, basis: &EditMesh, values: &[f32]) -> ModelResult<EditMesh> {
        self.check_basis(basis)?;
        self.validate()?;
        if values.len() != self.keys.len() {
            return Err(ModelError::InvalidParameter(format!(
                "{} valores para {} chaves",
                values.len(),
                self.keys.len()
            )));
        }
        if values.iter().any(|v| !v.is_finite()) {
            return Err(ModelError::InvalidParameter("valor não finito".into()));
        }
        let mut out = basis.clone();
        for (k, key) in self.keys.iter().enumerate() {
            let w = values[k].clamp(key.min, key.max);
            if w == 0.0 {
                continue;
            }
            for (v, d) in self.effective_deltas(k)? {
                out.verts[v as usize].co += d * w;
            }
        }
        Ok(out)
    }

    /// Acha o vértice parceiro (posição refletida) de cada vértice da base.
    fn mirror_partners(basis: &EditMesh, axis: MirrorAxis, tolerance: f32) -> Vec<Option<u32>> {
        let hash = PointHash::new(
            basis.verts.iter().enumerate().map(|(i, v)| (i as u32, v.co)),
            tolerance * 2.0,
        );
        let ai = match axis {
            MirrorAxis::X => 0,
            MirrorAxis::Y => 1,
            MirrorAxis::Z => 2,
        };
        basis
            .verts
            .iter()
            .map(|v| {
                let mut target = v.co;
                target[ai] = -target[ai];
                let mut best: Option<(f32, u32)> = None;
                for j in hash.candidates(target) {
                    let d = basis.verts[j as usize].co.distance(target);
                    if d <= tolerance && best.map(|b| d < b.0 || (d == b.0 && j < b.1)).unwrap_or(true) {
                        best = Some((d, j));
                    }
                }
                best.map(|b| b.1)
            })
            .collect()
    }

    /// Cria o espelho L↔R da chave `index` e o acrescenta ao conjunto.
    ///
    /// Para cada vértice `v` com parceiro `m(v)` (posição da base refletida no
    /// `axis`, a distância ≤ `tolerance`; vértices no plano são parceiros de si
    /// mesmos), `Δ'(v) = refletir(Δ(m(v)))` (nega a componente do eixo). A máscara
    /// também é espelhada. Vértices sem parceiro ficam sem delta. O nome é
    /// trocado por [`mirror_name`] (se o resultado já existir, falha). Devolve
    /// `(índice da nova chave, vértices da base sem parceiro)`.
    pub fn mirror_key(
        &mut self,
        basis: &EditMesh,
        index: usize,
        axis: MirrorAxis,
        tolerance: f32,
    ) -> ModelResult<(usize, usize)> {
        self.check_basis(basis)?;
        if !tolerance.is_finite() || tolerance <= 0.0 {
            return Err(ModelError::InvalidParameter("tolerância deve ser > 0".into()));
        }
        let key = self
            .keys
            .get(index)
            .ok_or_else(|| ModelError::InvalidParameter(format!("shape key {index} inexistente")))?
            .clone();
        let new_name = mirror_name(&key.name);
        if self.find(&new_name).is_some() {
            return Err(ModelError::InvalidParameter(format!("shape key '{new_name}' já existe")));
        }
        let partners = Self::mirror_partners(basis, axis, tolerance);
        let unmatched = partners.iter().filter(|p| p.is_none()).count();
        let ai = match axis {
            MirrorAxis::X => 0,
            MirrorAxis::Y => 1,
            MirrorAxis::Z => 2,
        };
        let old: HashMap<u32, Vec3> = key.deltas.iter().copied().collect();
        let mut mirrored = ShapeKey::new(new_name);
        mirrored.relative_to = key.relative_to;
        mirrored.min = key.min;
        mirrored.max = key.max;
        for (v, p) in partners.iter().enumerate() {
            let Some(p) = p else { continue };
            if let Some(&d) = old.get(p) {
                let mut r = d;
                r[ai] = -r[ai];
                mirrored.deltas.push((v as u32, r));
            }
        }
        if let Some(mask) = &key.vertex_group_mask {
            let old_mask: HashMap<u32, f32> = mask.iter().copied().collect();
            let mut m = Vec::new();
            for (v, p) in partners.iter().enumerate() {
                if let Some(w) = p.and_then(|p| old_mask.get(&p)) {
                    m.push((v as u32, *w));
                }
            }
            mirrored.vertex_group_mask = Some(m);
        }
        let idx = self.add_key(mirrored)?;
        Ok((idx, unmatched))
    }

    /// Converte para `SparseMorphSet` do núcleo (uma `MorphTarget` por chave,
    /// com os deltas efetivos a valor 1; `value` não entra — é o peso do canal).
    ///
    /// Cada delta de vértice de edição é replicado para todos os vértices do
    /// núcleo gerados por ele (ver [`core_vertex_map`]). Com `with_normals`, a
    /// `delta_normal` de cada vértice do núcleo é `n_deformado − n_base` das
    /// normais por corner efetivas; vértices cujo normal muda mas que não se
    /// moveram (vizinhos) também entram, com delta de posição zero.
    pub fn to_core_sparse_morph_set(&self, basis: &EditMesh, opts: &MorphExportOptions) -> ModelResult<SparseMorphSet> {
        self.check_basis(basis)?;
        self.validate()?;
        if !opts.min_delta.is_finite() || opts.min_delta < 0.0 {
            return Err(ModelError::InvalidParameter("min_delta inválido".into()));
        }
        let map = core_vertex_map(basis, &opts.core)?;
        let base_cn = if opts.with_normals {
            Some(compute_corner_normals(basis, &opts.core.normals))
        } else {
            None
        };
        let mut set = SparseMorphSet::new();
        for k in 0..self.keys.len() {
            let eff = self.effective_deltas(k)?;
            let dpos: HashMap<u32, Vec3> = eff.iter().copied().collect();
            let shaped_cn = if let Some(_) = &base_cn {
                let mut shaped = basis.clone();
                for (&v, d) in &dpos {
                    shaped.verts[v as usize].co += *d;
                }
                Some(compute_corner_normals(&shaped, &opts.core.normals))
            } else {
                None
            };
            let mut deltas: Vec<SparseMorphDelta> = Vec::new();
            for (ci, cv) in map.iter().enumerate() {
                let dp = dpos.get(&cv.edit_vert).copied().unwrap_or(Vec3::ZERO);
                let dn = match (&base_cn, &shaped_cn) {
                    (Some(b), Some(s)) => {
                        s[cv.face as usize][cv.corner as usize] - b[cv.face as usize][cv.corner as usize]
                    }
                    _ => Vec3::ZERO,
                };
                if dp.length() <= opts.min_delta && dn.length() <= 1.0e-6 {
                    continue;
                }
                deltas.push(SparseMorphDelta::new(ci as u32, dp.to_array(), dn.to_array()));
            }
            set.add_target(self.keys[k].name.clone(), deltas);
        }
        Ok(set)
    }

    /// Cria um `ShapeKeySet` a partir de um `SparseMorphSet` do núcleo, usando
    /// `basis` para inverter a divisão de vértices (vértice do núcleo →
    /// vértice de edição). Deltas de vários vértices do núcleo sobre o mesmo
    /// vértice de edição são **promediados**. Deltas de normal são descartados
    /// (são recalculáveis). Índices fora do intervalo são erro.
    pub fn from_core_sparse_morph_set(
        set: &SparseMorphSet,
        basis: &EditMesh,
        opts: &MorphExportOptions,
    ) -> ModelResult<ShapeKeySet> {
        let map = core_vertex_map(basis, &opts.core)?;
        let mut out = ShapeKeySet::new(basis);
        for target in &set.targets {
            let mut acc: BTreeMap<u32, (Vec3, u32)> = BTreeMap::new();
            for d in &target.deltas {
                let cv = map.get(d.vertex_index as usize).ok_or_else(|| {
                    ModelError::InvalidParameter(format!(
                        "delta aponta para o vértice do núcleo {} (a malha gera {})",
                        d.vertex_index,
                        map.len()
                    ))
                })?;
                let p = Vec3::from_array(d.delta_position);
                if !p.is_finite() {
                    return Err(ModelError::Numeric(format!("alvo '{}': delta não finito", target.name)));
                }
                let e = acc.entry(cv.edit_vert).or_insert((Vec3::ZERO, 0));
                e.0 += p;
                e.1 += 1;
            }
            let mut key = ShapeKey::new(target.name.clone());
            for (v, (sum, n)) in acc {
                let d = sum / n as f32;
                if d != Vec3::ZERO {
                    key.deltas.push((v, d));
                }
            }
            out.add_key(key)?;
        }
        Ok(out)
    }
}

/// Nome do lado oposto: troca `.L/.R`, `_L/_R`, `-L/-R` (e minúsculas),
/// prefixos `L_/R_`/`L./R.` e `Left/Right`/`left/right`. Se não houver marca de
/// lado, devolve `"<nome>.mirror"`.
pub fn mirror_name(name: &str) -> String {
    // sufixos de uma letra: no fim ou seguidos de separador
    for (a, b) in [('L', 'R'), ('l', 'r'), ('R', 'L'), ('r', 'l')] {
        for sep in ['.', '_', '-'] {
            let bytes: Vec<char> = name.chars().collect();
            for i in 0..bytes.len().saturating_sub(1) {
                if bytes[i] == sep && bytes[i + 1] == a {
                    let after = bytes.get(i + 2).copied();
                    if after.map(|c| c == '.' || c == '_' || c == '-').unwrap_or(true) {
                        let mut out = bytes.clone();
                        out[i + 1] = b;
                        return out.into_iter().collect();
                    }
                }
            }
            // prefixo "L_" / "L."
            if bytes.len() >= 2 && bytes[0] == a && bytes[1] == sep {
                let mut out = bytes.clone();
                out[0] = b;
                return out.into_iter().collect();
            }
        }
    }
    for (a, b) in [("Left", "Right"), ("Right", "Left"), ("left", "right"), ("right", "left")] {
        if let Some(pos) = name.find(a) {
            let mut out = String::with_capacity(name.len() + 1);
            out.push_str(&name[..pos]);
            out.push_str(b);
            out.push_str(&name[pos + a.len()..]);
            return out;
        }
    }
    format!("{name}.mirror")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::convert::to_core_mesh;
    use crate::primitives;

    fn deformed(base: &EditMesh) -> EditMesh {
        let mut m = base.clone();
        for v in &mut m.verts {
            if v.co.y > 0.5 {
                v.co.x += 0.3;
                v.co.y += 0.1;
            }
        }
        m
    }

    #[test]
    fn capture_is_sparse_and_apply_reconstructs() {
        let base = primitives::quad_sphere(1.0, 6);
        let shaped = deformed(&base);
        let key = ShapeKey::capture(&base, &shaped, "smile", 1e-6).unwrap();
        let moved = base.verts.iter().filter(|v| v.co.y > 0.5).count();
        assert_eq!(key.deltas.len(), moved);
        assert!(key.deltas.len() < base.vert_count());
        assert!(key.deltas.windows(2).all(|w| w[0].0 < w[1].0));

        let mut set = ShapeKeySet::new(&base);
        set.add_key(key).unwrap();
        // valor 0: igual à base; 1: igual à forma; 0.5: meio
        assert_eq!(set.apply(&base).unwrap().verts, base.verts);
        set.keys[0].value = 1.0;
        let full = set.apply(&base).unwrap();
        for (a, b) in full.verts.iter().zip(&shaped.verts) {
            assert!((a.co - b.co).length() < 1e-6);
        }
        set.keys[0].value = 0.5;
        let half = set.apply(&base).unwrap();
        for ((h, b), s) in half.verts.iter().zip(&base.verts).zip(&shaped.verts) {
            assert!((h.co - (b.co + s.co) * 0.5).length() < 1e-6);
        }
        // base nunca é mutada; faces/atributos preservados
        assert_eq!(half.faces, base.faces);
        // threshold alto elimina deltas
        let none = ShapeKey::capture(&base, &shaped, "x", 10.0).unwrap();
        assert!(none.deltas.is_empty());
    }

    #[test]
    fn range_clamp_values_and_mask() {
        let base = primitives::quad_sphere(1.0, 4);
        let shaped = deformed(&base);
        let mut set = ShapeKeySet::capture(&base, &shaped, "k", 1e-6).unwrap();
        // fora da faixa é limitado
        set.keys[0].value = 5.0;
        let clamped = set.apply(&base).unwrap();
        for (a, b) in clamped.verts.iter().zip(&shaped.verts) {
            assert!((a.co - b.co).length() < 1e-6);
        }
        // faixa estendida
        set.keys[0].min = -1.0;
        set.keys[0].max = 2.0;
        set.keys[0].value = -1.0;
        let neg = set.apply(&base).unwrap();
        for ((a, b), s) in neg.verts.iter().zip(&base.verts).zip(&shaped.verts) {
            assert!((a.co - (b.co - (s.co - b.co))).length() < 1e-6);
        }
        // apply_values não muta o conjunto
        let v = set.apply_values(&base, &[2.0]).unwrap();
        assert!((v.verts[base.verts.iter().position(|x| x.co.y > 0.5).unwrap()].co.x
            - (base.verts[base.verts.iter().position(|x| x.co.y > 0.5).unwrap()].co.x + 0.6))
            .abs()
            < 1e-5);
        assert_eq!(set.keys[0].value, -1.0);
        // máscara esparsa: só o vértice com peso 0.5 se move pela metade
        set.keys[0].min = 0.0;
        set.keys[0].max = 1.0;
        set.keys[0].value = 1.0;
        let first = set.keys[0].deltas[0].0;
        let second = set.keys[0].deltas[1].0;
        set.keys[0].vertex_group_mask = Some(vec![(first, 0.5)]);
        let m = set.apply(&base).unwrap();
        let d0 = shaped.verts[first as usize].co - base.verts[first as usize].co;
        assert!((m.verts[first as usize].co - (base.verts[first as usize].co + d0 * 0.5)).length() < 1e-6);
        assert_eq!(m.verts[second as usize].co, base.verts[second as usize].co);
    }

    #[test]
    fn relative_keys_follow_blender_semantics() {
        let base = primitives::quad_sphere(1.0, 4);
        let mut a = base.clone();
        let mut b = base.clone();
        for v in &mut a.verts {
            if v.co.y > 0.5 {
                v.co.x += 1.0;
            }
        }
        for v in &mut b.verts {
            if v.co.y > 0.5 {
                v.co.x += 1.5;
            }
        }
        let mut set = ShapeKeySet::new(&base);
        let ia = set.add_capture(&base, &a, "A", 1e-6).unwrap();
        let ib = set.add_capture(&base, &b, "B", 1e-6).unwrap();
        set.keys[ib].relative_to = Some(ia);
        set.validate().unwrap();
        // B relativa a A, valor 1: desloca +0.5 (B − A)
        set.keys[ib].value = 1.0;
        let m = set.apply(&base).unwrap();
        let v = base.verts.iter().position(|x| x.co.y > 0.5).unwrap();
        assert!((m.verts[v].co.x - (base.verts[v].co.x + 0.5)).abs() < 1e-5);
        // vértice só na referência: delta = −ref
        let mut only_ref = ShapeKeySet::new(&base);
        only_ref.add_key(ShapeKey { deltas: vec![(0, Vec3::X)], ..ShapeKey::new("ref") }).unwrap();
        only_ref
            .add_key(ShapeKey { relative_to: Some(0), value: 1.0, ..ShapeKey::new("empty") })
            .unwrap();
        let out = only_ref.apply(&base).unwrap();
        assert!((out.verts[0].co - (base.verts[0].co - Vec3::X)).length() < 1e-6);
        // remover a referência religa à base e reindexa
        let removed = set.remove_key(ia).unwrap();
        assert_eq!(removed.name, "A");
        assert_eq!(set.keys[0].name, "B");
        assert_eq!(set.keys[0].relative_to, None);
        assert!(set.remove_key(9).is_err());
        let mut three = ShapeKeySet::new(&base);
        for n in ["x", "y", "z"] {
            three.add_key(ShapeKey::new(n)).unwrap();
        }
        three.keys[2].relative_to = Some(1);
        three.remove_key(0).unwrap();
        assert_eq!(three.keys[1].relative_to, Some(0));
    }

    #[test]
    fn validation_errors() {
        let base = primitives::quad_sphere(1.0, 3);
        let n = base.vert_count() as u32;
        let mut set = ShapeKeySet::new(&base);
        // contagem diferente
        let other = primitives::quad_sphere(1.0, 4);
        assert!(matches!(
            ShapeKey::capture(&base, &other, "x", 0.0),
            Err(ModelError::UnsupportedTopology(_))
        ));
        assert!(ShapeKey::capture(&base, &base, "x", -1.0).is_err());
        assert!(ShapeKey::capture(&base, &base, "x", f32::NAN).is_err());
        let mut nan = base.clone();
        nan.verts[0].co = Vec3::NAN;
        assert!(ShapeKey::capture(&base, &nan, "x", 0.0).is_err());
        assert!(set.add_capture(&other, &other, "x", 0.0).is_err());
        // chaves inválidas
        let bad = |k: ShapeKey| set.clone().add_key(k).is_err();
        assert!(bad(ShapeKey { deltas: vec![(n, Vec3::X)], ..ShapeKey::new("a") }));
        assert!(bad(ShapeKey { deltas: vec![(2, Vec3::X), (1, Vec3::X)], ..ShapeKey::new("a") }));
        assert!(bad(ShapeKey { deltas: vec![(1, Vec3::X), (1, Vec3::X)], ..ShapeKey::new("a") }));
        assert!(bad(ShapeKey { deltas: vec![(1, Vec3::NAN)], ..ShapeKey::new("a") }));
        assert!(bad(ShapeKey { min: 1.0, max: 0.0, ..ShapeKey::new("a") }));
        assert!(bad(ShapeKey { min: f32::NAN, ..ShapeKey::new("a") }));
        assert!(bad(ShapeKey { value: f32::NAN, ..ShapeKey::new("a") }));
        assert!(bad(ShapeKey { relative_to: Some(5), ..ShapeKey::new("a") }));
        assert!(bad(ShapeKey { vertex_group_mask: Some(vec![(n, 1.0)]), ..ShapeKey::new("a") }));
        assert!(bad(ShapeKey { vertex_group_mask: Some(vec![(2, 1.0), (1, 1.0)]), ..ShapeKey::new("a") }));
        assert!(bad(ShapeKey { vertex_group_mask: Some(vec![(1, f32::NAN)]), ..ShapeKey::new("a") }));
        set.add_key(ShapeKey::new("a")).unwrap();
        assert!(set.add_key(ShapeKey::new("a")).is_err()); // duplicado
        // auto-referência detectada em validate()
        set.keys[0].relative_to = Some(0);
        assert!(set.validate().is_err());
        assert!(set.apply(&base).is_err());
        set.keys[0].relative_to = None;
        // base com contagem errada / valores errados
        assert!(set.apply(&other).is_err());
        assert!(set.apply_values(&base, &[]).is_err());
        assert!(set.apply_values(&base, &[f32::NAN]).is_err());
        // nome repetido por edição direta
        set.keys.push(ShapeKey::new("a"));
        assert!(set.validate().is_err());
        set.keys.pop();
        assert!(set.effective_deltas(7).is_err());
        assert!(set.rebase(&other).is_err());
        assert!(set.is_basis_current(&base));
        let mut edited = base.clone();
        edited.verts[0].co.x += 1.0;
        assert!(!set.is_basis_current(&edited));
        set.rebase(&edited).unwrap();
        assert!(set.is_basis_current(&edited));
    }

    #[test]
    fn serde_roundtrip() {
        let base = primitives::quad_sphere(1.0, 3);
        let mut set = ShapeKeySet::capture(&base, &deformed(&base), "k", 1e-6).unwrap();
        set.keys[0].vertex_group_mask = Some(vec![(0, 0.25), (3, 1.0)]);
        set.keys[0].value = 0.3;
        let json = serde_json::to_string(&set).unwrap();
        let back: ShapeKeySet = serde_json::from_str(&json).unwrap();
        assert_eq!(set, back);
        assert_eq!(set.apply(&base).unwrap().content_hash(), back.apply(&base).unwrap().content_hash());
    }

    #[test]
    fn mirror_names() {
        assert_eq!(mirror_name("smile.L"), "smile.R");
        assert_eq!(mirror_name("smile.R"), "smile.L");
        assert_eq!(mirror_name("brow_L_up"), "brow_R_up");
        assert_eq!(mirror_name("eye-l"), "eye-r");
        assert_eq!(mirror_name("L_cheek"), "R_cheek");
        assert_eq!(mirror_name("mouthLeft"), "mouthRight");
        assert_eq!(mirror_name("right_blink"), "left_blink");
        assert_eq!(mirror_name("jaw_open"), "jaw_open.mirror");
        assert_eq!(mirror_name("smile.Lip"), "smile.Lip.mirror");
    }

    #[test]
    fn mirror_key_flips_deltas() {
        let base = primitives::quad_sphere(1.0, 4);
        // deforma só o lado +X puxando para fora e para cima
        let mut shaped = base.clone();
        for v in &mut shaped.verts {
            if v.co.x > 0.05 {
                v.co += Vec3::new(0.2, 0.1, 0.0);
            }
        }
        let mut set = ShapeKeySet::new(&base);
        set.add_capture(&base, &shaped, "smile.L", 1e-6).unwrap();
        let (idx, unmatched) = set.mirror_key(&base, 0, MirrorAxis::X, 1e-4).unwrap();
        assert_eq!(unmatched, 0);
        assert_eq!(set.keys[idx].name, "smile.R");
        set.keys[0].value = 0.0;
        set.keys[idx].value = 1.0;
        let m = set.apply(&base).unwrap();
        for (v, orig) in m.verts.iter().zip(&base.verts) {
            if orig.co.x < -0.05 {
                assert!((v.co - (orig.co + Vec3::new(-0.2, 0.1, 0.0))).length() < 1e-5);
            } else if orig.co.x > 0.05 {
                assert_eq!(v.co, orig.co);
            }
        }
        // espelhar de novo falha (nome já existe) e índice inválido também
        assert!(set.mirror_key(&base, 0, MirrorAxis::X, 1e-4).is_err());
        assert!(set.mirror_key(&base, 9, MirrorAxis::X, 1e-4).is_err());
        assert!(set.mirror_key(&base, 0, MirrorAxis::X, 0.0).is_err());
        // máscara espelhada e malha assimétrica
        let mut asym = base.clone();
        asym.verts[0].co.x += 0.3; // quebra a simetria para o vértice 0
        let mut s2 = ShapeKeySet::new(&asym);
        let mut k = ShapeKey::new("a.L");
        k.deltas = vec![(1, Vec3::X)];
        k.vertex_group_mask = Some(vec![(1, 0.5)]);
        s2.add_key(k).unwrap();
        let (i2, un) = s2.mirror_key(&asym, 0, MirrorAxis::X, 1e-4).unwrap();
        assert!(un >= 1);
        assert!(s2.keys[i2].vertex_group_mask.is_some());
    }

    #[test]
    fn core_conversion_matches_to_core_mesh() {
        // cubo com arestas duras: vértices do núcleo divididos por normal
        let mut base = primitives::cube_subdivided(2.0, 2);
        for f in &mut base.faces {
            f.smooth = true;
        }
        let shaped = {
            let mut m = base.clone();
            for v in &mut m.verts {
                if v.co.y > 0.5 {
                    v.co.y += 0.4;
                    v.co.x *= 0.8;
                }
            }
            m
        };
        let set = ShapeKeySet::capture(&base, &shaped, "tall", 1e-6).unwrap();
        let opts = MorphExportOptions::default();
        let morphs = set.to_core_sparse_morph_set(&base, &opts).unwrap();
        assert_eq!(morphs.len(), 1);
        assert_eq!(morphs.targets[0].name, "tall");
        assert!(morphs.targets[0].has_normal_deltas());

        let core_base = to_core_mesh(&base, &opts.core).unwrap();
        let core_shaped = to_core_mesh(&shaped, &opts.core).unwrap();
        assert_eq!(core_base.vertices.len(), core_shaped.vertices.len());
        // vértices do núcleo podem ser mais que os de edição (divisão por normal)
        assert!(core_base.vertices.len() >= base.vert_count());
        let mut out = core_base.vertices.clone();
        morphs.apply_cpu(&[1.0], &core_base.vertices, &mut out);
        for (o, s) in out.iter().zip(&core_shaped.vertices) {
            for k in 0..3 {
                assert!((o.position[k] - s.position[k]).abs() < 1e-5, "posição");
            }
            let dot: f32 = (0..3).map(|k| o.normal[k] * s.normal[k]).sum();
            assert!(dot > 0.999, "normal dot {dot}");
        }
        // sem normais: delta_normal zero
        let no_n = set
            .to_core_sparse_morph_set(&base, &MorphExportOptions { with_normals: false, ..Default::default() })
            .unwrap();
        assert!(!no_n.targets[0].has_normal_deltas());
        assert!(no_n.targets[0].deltas.len() <= morphs.targets[0].deltas.len());

        // volta: from_core recupera os mesmos deltas de posição
        let back = ShapeKeySet::from_core_sparse_morph_set(&morphs, &base, &opts).unwrap();
        assert_eq!(back.keys.len(), 1);
        assert_eq!(back.keys[0].name, "tall");
        let re = back.apply_values(&base, &[1.0]).unwrap();
        for (a, b) in re.verts.iter().zip(&shaped.verts) {
            assert!((a.co - b.co).length() < 1e-5);
        }
        // delta de índice inválido
        let mut bad = SparseMorphSet::new();
        bad.add_target("x", vec![SparseMorphDelta::new(1_000_000, [1.0; 3], [0.0; 3])]);
        assert!(ShapeKeySet::from_core_sparse_morph_set(&bad, &base, &opts).is_err());
        let mut nan = SparseMorphSet::new();
        nan.add_target("x", vec![SparseMorphDelta::new(0, [f32::NAN; 3], [0.0; 3])]);
        assert!(ShapeKeySet::from_core_sparse_morph_set(&nan, &base, &opts).is_err());
        // opções inválidas / base de tamanho errado
        assert!(set
            .to_core_sparse_morph_set(&base, &MorphExportOptions { min_delta: -1.0, ..Default::default() })
            .is_err());
        assert!(set.to_core_sparse_morph_set(&primitives::cube(1.0), &opts).is_err());
    }

    #[test]
    fn core_conversion_respects_relative_mask_and_hidden() {
        let base = primitives::quad_sphere(1.0, 3);
        let shaped = deformed(&base);
        let mut set = ShapeKeySet::capture(&base, &shaped, "k", 1e-6).unwrap();
        let first = set.keys[0].deltas[0].0;
        set.keys[0].vertex_group_mask = Some(vec![(first, 1.0)]);
        let opts = MorphExportOptions { with_normals: false, ..Default::default() };
        let morphs = set.to_core_sparse_morph_set(&base, &opts).unwrap();
        // só o vértice mascarado (e seus clones do núcleo) tem delta
        let map = core_vertex_map(&base, &opts.core).unwrap();
        for d in &morphs.targets[0].deltas {
            assert_eq!(map[d.vertex_index as usize].edit_vert, first);
        }
        assert!(!morphs.targets[0].deltas.is_empty());
        // faces ocultas não geram vértices do núcleo
        let mut hidden = base.clone();
        hidden.faces[0].hidden = true;
        let m_all = core_vertex_map(&base, &opts.core).unwrap().len();
        let m_hid = core_vertex_map(&hidden, &opts.core).unwrap().len();
        assert!(m_hid < m_all);
        let core = to_core_mesh(&hidden, &opts.core).unwrap();
        assert_eq!(core.vertices.len(), m_hid);
    }

    #[test]
    fn large_mesh_sanity_and_determinism() {
        let base = primitives::quad_sphere(1.0, 48);
        let shaped = deformed(&base);
        let t = std::time::Instant::now();
        let mut set = ShapeKeySet::capture(&base, &shaped, "k", 1e-6).unwrap();
        set.mirror_key(&base, 0, MirrorAxis::X, 1e-4).unwrap_or((0, 0));
        set.keys[0].value = 0.7;
        let a = set.apply(&base).unwrap();
        let b = set.apply(&base).unwrap();
        assert_eq!(a.content_hash(), b.content_hash());
        let opts = MorphExportOptions::default();
        let m1 = set.to_core_sparse_morph_set(&base, &opts).unwrap();
        let m2 = set.to_core_sparse_morph_set(&base, &opts).unwrap();
        assert_eq!(m1, m2);
        assert!(t.elapsed().as_secs() < 90);
    }
}
