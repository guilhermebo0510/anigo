//! `EditMesh`: malha poligonal (n-gons) editável, inspirada no BMesh do Blender.
//!
//! Decisões de projeto:
//! * Faces guardam **corners** (vértice + UV + normal customizada opcional), então
//!   atributos por-corner viajam junto com a face em qualquer operador.
//! * Atributos de aresta (sharp/seam/crease/bevel) vivem num mapa ordenado por
//!   par de vértices — sobrevivem a edições de topologia que preservam os vértices
//!   e dão serialização determinística.
//! * A adjacência (aresta→faces, vértice→faces) é **derivada** ([`crate::topology::Topology`]);
//!   nunca é fonte de verdade, então não pode dessincronizar.
//! * Aceita não-manifold (arestas soltas, 3+ faces por aresta) — necessário para
//!   modelagem em andamento e cartões de cabelo.

use crate::error::{ModelError, ModelResult};
use glam::{Vec2, Vec3};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

/// Chave canônica (ordenada) de aresta.
pub type EdgeKey = (u32, u32);

#[inline]
pub fn edge_key(a: u32, b: u32) -> EdgeKey {
    if a <= b {
        (a, b)
    } else {
        (b, a)
    }
}

/// Vértice editável. Os 4 canais de `color` seguem o contrato NPR do ANIGO
/// (`anigo_core::Vertex::color`): R = AO estilizado, G = shift do limiar de
/// sombra, B = multiplicador de espessura do contorno, A = máscara spec/rim.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct EVert {
    pub co: Vec3,
    pub color: [f32; 4],
    pub joints: [u16; 4],
    pub weights: [f32; 4],
    /// Máscara de escultura (0 = livre, 1 = protegido).
    pub mask: f32,
    /// Vinco por vértice para subdivisão (0..1).
    pub crease: f32,
    pub hidden: bool,
}

impl EVert {
    pub fn new(co: Vec3) -> Self {
        Self {
            co,
            color: [1.0, 0.5, 1.0, 1.0],
            joints: [0; 4],
            weights: [1.0, 0.0, 0.0, 0.0],
            mask: 0.0,
            crease: 0.0,
            hidden: false,
        }
    }

    /// Cria um vértice novo herdando atributos de `src` e posição `co`.
    pub fn derived(src: &EVert, co: Vec3) -> Self {
        Self { co, ..src.clone() }
    }

    /// Mistura atributos (cor, pesos de skin, máscara) de dois vértices em `t`.
    pub fn lerp(a: &EVert, b: &EVert, t: f32) -> Self {
        let mut color = [0.0; 4];
        for (i, c) in color.iter_mut().enumerate() {
            *c = a.color[i] + (b.color[i] - a.color[i]) * t;
        }
        // Skin: o vértice mais próximo manda (mistura de joints não é bem definida).
        let src = if t < 0.5 { a } else { b };
        Self {
            co: a.co.lerp(b.co, t),
            color,
            joints: src.joints,
            weights: src.weights,
            mask: a.mask + (b.mask - a.mask) * t,
            crease: a.crease + (b.crease - a.crease) * t,
            hidden: a.hidden && b.hidden,
        }
    }
}

/// Canto de face: referência de vértice + atributos por-corner.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Corner {
    pub vert: u32,
    pub uv: Vec2,
    /// Normal customizada (split normal). `None` = calculada automaticamente.
    #[serde(default)]
    pub normal: Option<Vec3>,
}

impl Corner {
    pub fn new(vert: u32) -> Self {
        Self {
            vert,
            uv: Vec2::ZERO,
            normal: None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Face {
    pub corners: Vec<Corner>,
    pub material: u16,
    /// Sombreamento suave (normais interpoladas) vs. chapado.
    pub smooth: bool,
    /// Face set de escultura (agrupa regiões protegidas / máscaras).
    pub face_set: i32,
    pub hidden: bool,
}

impl Face {
    pub fn new(verts: &[u32]) -> Self {
        Self {
            corners: verts.iter().map(|&v| Corner::new(v)).collect(),
            material: 0,
            smooth: true,
            face_set: 0,
            hidden: false,
        }
    }

    pub fn verts(&self) -> impl Iterator<Item = u32> + '_ {
        self.corners.iter().map(|c| c.vert)
    }

    pub fn len(&self) -> usize {
        self.corners.len()
    }

    pub fn is_empty(&self) -> bool {
        self.corners.is_empty()
    }

    /// Arestas orientadas (a→b) da face, em ordem de corner.
    pub fn edges(&self) -> impl Iterator<Item = (u32, u32)> + '_ {
        let n = self.corners.len();
        (0..n).map(move |i| (self.corners[i].vert, self.corners[(i + 1) % n].vert))
    }

    /// Cópia com atributos de face herdados mas outros cantos.
    pub fn with_verts_from(&self, verts: &[u32]) -> Face {
        Face {
            corners: verts.iter().map(|&v| Corner::new(v)).collect(),
            material: self.material,
            smooth: self.smooth,
            face_set: self.face_set,
            hidden: self.hidden,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Default, Serialize, Deserialize)]
pub struct EdgeAttr {
    /// Aresta dura: quebra normais suaves.
    pub sharp: bool,
    /// Costura de UV.
    pub seam: bool,
    /// Vinco de subdivisão (0..1).
    pub crease: f32,
    /// Peso para o modificador/ferramenta Bevel (0..1).
    pub bevel_weight: f32,
}

impl EdgeAttr {
    pub fn is_default(&self) -> bool {
        *self == EdgeAttr::default()
    }
}

/// Resultado de operações que compactam índices.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Remap {
    pub vert: Vec<Option<u32>>,
    pub face: Vec<Option<u32>>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct EditMesh {
    pub name: String,
    pub verts: Vec<EVert>,
    pub faces: Vec<Face>,
    /// Atributos de aresta não-padrão, ordenados (serialização determinística).
    #[serde(with = "edge_attr_serde")]
    pub edge_attrs: BTreeMap<EdgeKey, EdgeAttr>,
}

mod edge_attr_serde {
    use super::{EdgeAttr, EdgeKey};
    use serde::{Deserialize, Deserializer, Serialize, Serializer};
    use std::collections::BTreeMap;

    pub fn serialize<S: Serializer>(
        map: &BTreeMap<EdgeKey, EdgeAttr>,
        s: S,
    ) -> Result<S::Ok, S::Error> {
        let list: Vec<(EdgeKey, EdgeAttr)> = map.iter().map(|(k, v)| (*k, *v)).collect();
        list.serialize(s)
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(
        d: D,
    ) -> Result<BTreeMap<EdgeKey, EdgeAttr>, D::Error> {
        let list: Vec<(EdgeKey, EdgeAttr)> = Vec::deserialize(d)?;
        Ok(list.into_iter().collect())
    }
}

impl Default for EditMesh {
    fn default() -> Self {
        Self::new("Mesh")
    }
}

impl EditMesh {
    pub fn new(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            verts: Vec::new(),
            faces: Vec::new(),
            edge_attrs: BTreeMap::new(),
        }
    }

    // ---------------------------------------------------------------- construção

    pub fn add_vert(&mut self, co: Vec3) -> u32 {
        self.verts.push(EVert::new(co));
        (self.verts.len() - 1) as u32
    }

    pub fn push_vert(&mut self, v: EVert) -> u32 {
        self.verts.push(v);
        (self.verts.len() - 1) as u32
    }

    /// Adiciona uma face. Exige ≥ 3 cantos, índices válidos e sem vértice
    /// repetido em cantos consecutivos.
    pub fn add_face(&mut self, verts: &[u32]) -> ModelResult<u32> {
        self.push_face(Face::new(verts))
    }

    pub fn push_face(&mut self, face: Face) -> ModelResult<u32> {
        self.check_face(&face)?;
        self.faces.push(face);
        Ok((self.faces.len() - 1) as u32)
    }

    fn check_face(&self, face: &Face) -> ModelResult<()> {
        if face.corners.len() < 3 {
            return Err(ModelError::InvalidFace(format!(
                "face precisa de ≥ 3 cantos, recebeu {}",
                face.corners.len()
            )));
        }
        for c in &face.corners {
            if c.vert as usize >= self.verts.len() {
                return Err(ModelError::VertexOutOfRange(c.vert, self.verts.len()));
            }
        }
        let n = face.corners.len();
        for i in 0..n {
            if face.corners[i].vert == face.corners[(i + 1) % n].vert {
                return Err(ModelError::InvalidFace(format!(
                    "vértice {} repetido em cantos consecutivos",
                    face.corners[i].vert
                )));
            }
        }
        Ok(())
    }

    // ------------------------------------------------------------------ consultas

    pub fn vert_count(&self) -> usize {
        self.verts.len()
    }

    pub fn face_count(&self) -> usize {
        self.faces.len()
    }

    pub fn is_empty(&self) -> bool {
        self.faces.is_empty() && self.verts.is_empty()
    }

    pub fn co(&self, v: u32) -> Vec3 {
        self.verts[v as usize].co
    }

    /// Triângulos após triangulação em leque: Σ (n − 2).
    pub fn triangle_count(&self) -> usize {
        self.faces.iter().map(|f| f.len().saturating_sub(2)).sum()
    }

    pub fn face_positions(&self, f: u32) -> Vec<Vec3> {
        self.faces[f as usize]
            .corners
            .iter()
            .map(|c| self.verts[c.vert as usize].co)
            .collect()
    }

    /// Normal da face pelo método de Newell (robusto a n-gons não planares).
    pub fn face_normal(&self, f: u32) -> Vec3 {
        let pts = self.face_positions(f);
        newell_normal(&pts)
    }

    pub fn face_center(&self, f: u32) -> Vec3 {
        let pts = self.face_positions(f);
        if pts.is_empty() {
            return Vec3::ZERO;
        }
        pts.iter().copied().sum::<Vec3>() / pts.len() as f32
    }

    pub fn face_area(&self, f: u32) -> f32 {
        let pts = self.face_positions(f);
        polygon_area(&pts)
    }

    pub fn bounds(&self) -> Option<(Vec3, Vec3)> {
        let first = self.verts.first()?.co;
        Some(self.verts.iter().fold((first, first), |(lo, hi), v| {
            (lo.min(v.co), hi.max(v.co))
        }))
    }

    pub fn centroid(&self) -> Vec3 {
        if self.verts.is_empty() {
            return Vec3::ZERO;
        }
        self.verts.iter().map(|v| v.co).sum::<Vec3>() / self.verts.len() as f32
    }

    // ------------------------------------------------------- atributos de aresta

    pub fn edge_attr(&self, a: u32, b: u32) -> EdgeAttr {
        self.edge_attrs
            .get(&edge_key(a, b))
            .copied()
            .unwrap_or_default()
    }

    /// Edita o atributo de uma aresta; remove a entrada se voltar ao padrão.
    pub fn edit_edge_attr(&mut self, a: u32, b: u32, f: impl FnOnce(&mut EdgeAttr)) {
        let key = edge_key(a, b);
        let mut attr = self.edge_attrs.get(&key).copied().unwrap_or_default();
        f(&mut attr);
        if attr.is_default() {
            self.edge_attrs.remove(&key);
        } else {
            self.edge_attrs.insert(key, attr);
        }
    }

    pub fn set_sharp(&mut self, a: u32, b: u32, sharp: bool) {
        self.edit_edge_attr(a, b, |e| e.sharp = sharp);
    }

    pub fn set_seam(&mut self, a: u32, b: u32, seam: bool) {
        self.edit_edge_attr(a, b, |e| e.seam = seam);
    }

    pub fn set_crease(&mut self, a: u32, b: u32, crease: f32) {
        self.edit_edge_attr(a, b, |e| e.crease = crease.clamp(0.0, 1.0));
    }

    pub fn set_bevel_weight(&mut self, a: u32, b: u32, w: f32) {
        self.edit_edge_attr(a, b, |e| e.bevel_weight = w.clamp(0.0, 1.0));
    }

    // --------------------------------------------------------------- manutenção

    /// Remove as faces indicadas (índices antigos) e compacta os índices de
    /// face. Vértices permanecem (use [`Self::remove_unused_verts`]).
    pub fn delete_faces(&mut self, faces: &[u32]) -> ModelResult<()> {
        let mut kill = vec![false; self.faces.len()];
        for &f in faces {
            if f as usize >= self.faces.len() {
                return Err(ModelError::FaceOutOfRange(f, self.faces.len()));
            }
            kill[f as usize] = true;
        }
        let mut i = 0;
        self.faces.retain(|_| {
            let keep = !kill[i];
            i += 1;
            keep
        });
        Ok(())
    }

    /// Vértices referenciados por alguma face.
    pub fn used_verts(&self) -> Vec<bool> {
        let mut used = vec![false; self.verts.len()];
        for f in &self.faces {
            for c in &f.corners {
                used[c.vert as usize] = true;
            }
        }
        used
    }

    /// Remove vértices sem face (e atributos de aresta órfãos) e renumera.
    /// Arestas soltas só existem via `edge_attrs`; vértices com atributo de
    /// aresta explícito não são removidos para não perder arestas soltas.
    pub fn remove_unused_verts(&mut self) -> Remap {
        let mut keep = self.used_verts();
        for (a, b) in self.edge_attrs.keys() {
            keep[*a as usize] = true;
            keep[*b as usize] = true;
        }
        self.compact_verts(&keep)
    }

    /// Mantém apenas os vértices marcados e renumera faces/atributos.
    /// Faces que referenciam vértice removido são descartadas.
    pub fn compact_verts(&mut self, keep: &[bool]) -> Remap {
        let mut vert_map: Vec<Option<u32>> = vec![None; self.verts.len()];
        let mut next = 0u32;
        for (i, k) in keep.iter().enumerate() {
            if *k {
                vert_map[i] = Some(next);
                next += 1;
            }
        }
        let old_verts = std::mem::take(&mut self.verts);
        self.verts = old_verts
            .into_iter()
            .enumerate()
            .filter(|(i, _)| keep[*i])
            .map(|(_, v)| v)
            .collect();

        let mut face_map: Vec<Option<u32>> = vec![None; self.faces.len()];
        let old_faces = std::mem::take(&mut self.faces);
        for (fi, mut face) in old_faces.into_iter().enumerate() {
            if face.corners.iter().any(|c| vert_map[c.vert as usize].is_none()) {
                continue;
            }
            for c in &mut face.corners {
                c.vert = vert_map[c.vert as usize].unwrap_or(c.vert);
            }
            face_map[fi] = Some(self.faces.len() as u32);
            self.faces.push(face);
        }
        let old_attrs = std::mem::take(&mut self.edge_attrs);
        for ((a, b), attr) in old_attrs {
            if let (Some(na), Some(nb)) = (vert_map[a as usize], vert_map[b as usize]) {
                self.edge_attrs.insert(edge_key(na, nb), attr);
            }
        }
        Remap {
            vert: vert_map,
            face: face_map,
        }
    }

    /// Inverte o enrolamento de uma face (normal para o lado oposto).
    pub fn flip_face(&mut self, f: u32) -> ModelResult<()> {
        let face = self
            .faces
            .get_mut(f as usize)
            .ok_or(ModelError::FaceOutOfRange(f, 0))?;
        face.corners.reverse();
        for c in &mut face.corners {
            if let Some(n) = c.normal.as_mut() {
                *n = -*n;
            }
        }
        Ok(())
    }

    /// Valida invariantes estruturais (índices, tamanho de face, repetição).
    pub fn validate(&self) -> ModelResult<()> {
        for f in &self.faces {
            self.check_face(f)?;
        }
        for (a, b) in self.edge_attrs.keys() {
            for v in [a, b] {
                if *v as usize >= self.verts.len() {
                    return Err(ModelError::VertexOutOfRange(*v, self.verts.len()));
                }
            }
        }
        for v in &self.verts {
            if !v.co.is_finite() {
                return Err(ModelError::Numeric("vértice com coordenada não finita".into()));
            }
        }
        Ok(())
    }

    /// Mescla outra malha (join). Devolve o deslocamento de vértices/faces.
    pub fn append(&mut self, other: &EditMesh) -> (u32, u32) {
        let v_off = self.verts.len() as u32;
        let f_off = self.faces.len() as u32;
        self.verts.extend(other.verts.iter().cloned());
        for f in &other.faces {
            let mut f = f.clone();
            for c in &mut f.corners {
                c.vert += v_off;
            }
            self.faces.push(f);
        }
        for ((a, b), attr) in &other.edge_attrs {
            self.edge_attrs.insert(edge_key(a + v_off, b + v_off), *attr);
        }
        (v_off, f_off)
    }

    /// Aplica uma função a todas as posições.
    pub fn transform_positions(&mut self, f: impl Fn(Vec3) -> Vec3) {
        for v in &mut self.verts {
            v.co = f(v.co);
        }
    }

    /// Hash FNV-1a determinístico do conteúdo (posições, topologia, atributos).
    /// Útil para testes de regressão e para o log de comandos.
    pub fn content_hash(&self) -> u64 {
        let mut h: u64 = 0xcbf29ce484222325;
        let mut mix = |bytes: &[u8]| {
            for b in bytes {
                h ^= *b as u64;
                h = h.wrapping_mul(0x100000001b3);
            }
        };
        mix(&(self.verts.len() as u64).to_le_bytes());
        for v in &self.verts {
            for c in v.co.to_array() {
                mix(&c.to_bits().to_le_bytes());
            }
        }
        for f in &self.faces {
            mix(&(f.corners.len() as u32).to_le_bytes());
            for c in &f.corners {
                mix(&c.vert.to_le_bytes());
            }
            mix(&[f.smooth as u8]);
            mix(&f.material.to_le_bytes());
        }
        for ((a, b), e) in &self.edge_attrs {
            mix(&a.to_le_bytes());
            mix(&b.to_le_bytes());
            mix(&[e.sharp as u8, e.seam as u8]);
            mix(&e.crease.to_bits().to_le_bytes());
        }
        h
    }
}

/// Normal de Newell para polígono arbitrário (não normalizada se degenerado → ZERO).
pub fn newell_normal(pts: &[Vec3]) -> Vec3 {
    let mut n = Vec3::ZERO;
    let len = pts.len();
    for i in 0..len {
        let a = pts[i];
        let b = pts[(i + 1) % len];
        n.x += (a.y - b.y) * (a.z + b.z);
        n.y += (a.z - b.z) * (a.x + b.x);
        n.z += (a.x - b.x) * (a.y + b.y);
    }
    n.try_normalize().unwrap_or(Vec3::ZERO)
}

/// Área de polígono 3D (norma do vetor de área de Newell / 2).
pub fn polygon_area(pts: &[Vec3]) -> f32 {
    let mut n = Vec3::ZERO;
    let len = pts.len();
    for i in 0..len {
        n += pts[i].cross(pts[(i + 1) % len]);
    }
    n.length() * 0.5
}

#[cfg(test)]
mod tests {
    use super::*;

    fn quad() -> EditMesh {
        let mut m = EditMesh::new("q");
        let a = m.add_vert(Vec3::new(0.0, 0.0, 0.0));
        let b = m.add_vert(Vec3::new(1.0, 0.0, 0.0));
        let c = m.add_vert(Vec3::new(1.0, 1.0, 0.0));
        let d = m.add_vert(Vec3::new(0.0, 1.0, 0.0));
        m.add_face(&[a, b, c, d]).unwrap();
        m
    }

    #[test]
    fn face_normal_and_area_of_unit_quad() {
        let m = quad();
        assert!((m.face_normal(0) - Vec3::Z).length() < 1e-6);
        assert!((m.face_area(0) - 1.0).abs() < 1e-6);
        assert_eq!(m.triangle_count(), 2);
    }

    #[test]
    fn add_face_rejects_bad_input() {
        let mut m = quad();
        assert!(matches!(m.add_face(&[0, 1]), Err(ModelError::InvalidFace(_))));
        assert!(matches!(
            m.add_face(&[0, 1, 9]),
            Err(ModelError::VertexOutOfRange(9, 4))
        ));
        assert!(matches!(m.add_face(&[0, 0, 1]), Err(ModelError::InvalidFace(_))));
    }

    #[test]
    fn edge_attr_defaults_are_pruned() {
        let mut m = quad();
        m.set_sharp(1, 0, true);
        assert!(m.edge_attr(0, 1).sharp);
        assert_eq!(m.edge_attrs.len(), 1);
        m.set_sharp(0, 1, false);
        assert!(m.edge_attrs.is_empty());
    }

    #[test]
    fn remove_unused_verts_remaps_faces_and_attrs() {
        let mut m = quad();
        let extra = m.add_vert(Vec3::splat(9.0));
        assert_eq!(extra, 4);
        m.set_seam(2, 3, true);
        let remap = m.remove_unused_verts();
        assert_eq!(m.vert_count(), 4);
        assert_eq!(remap.vert[4], None);
        assert!(m.edge_attr(2, 3).seam);
    }

    #[test]
    fn delete_faces_compacts() {
        let mut m = quad();
        m.add_face(&[0, 1, 2]).unwrap();
        m.delete_faces(&[0]).unwrap();
        assert_eq!(m.face_count(), 1);
        assert_eq!(m.faces[0].len(), 3);
        assert!(m.delete_faces(&[5]).is_err());
    }

    #[test]
    fn content_hash_is_stable_and_sensitive() {
        let a = quad();
        let mut b = quad();
        assert_eq!(a.content_hash(), b.content_hash());
        b.verts[0].co.x += 0.001;
        assert_ne!(a.content_hash(), b.content_hash());
    }

    #[test]
    fn serde_roundtrip_is_lossless() {
        let mut m = quad();
        m.set_crease(0, 1, 0.5);
        let json = serde_json::to_string(&m).unwrap();
        let back: EditMesh = serde_json::from_str(&json).unwrap();
        assert_eq!(m, back);
    }

    #[test]
    fn append_offsets_indices() {
        let mut a = quad();
        let b = quad();
        let (vo, fo) = a.append(&b);
        assert_eq!((vo, fo), (4, 1));
        assert_eq!(a.faces[1].corners[0].vert, 4);
        a.validate().unwrap();
    }
}
