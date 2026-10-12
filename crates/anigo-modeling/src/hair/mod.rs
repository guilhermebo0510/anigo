//! Ferramenta de cabelo (estilo VRoid) do workspace Modelagem: modelo de dados editável.
//!
//! Hierarquia (a mesma do VRoid): [`HairSet`] (penteado) → [`HairGroup`] (grupo) → [`HairStrand`]
//! (mecha). Cada mecha guarda uma **curva guia** do tipo [`crate::curve::Curve`] (raiz → ponta; pontos
//! com posição, `tilt` e `radius`) mais os parâmetros de seção/afinamento/resolução; a malha NÃO é
//! guardada: é regenerada por [`build`] a cada edição (as mechas continuam editáveis). Os tipos
//! convergem com `curve.rs`: a seção plana/arco vira [`crate::curve::hair_card`], a mecha com volume
//! (triângulo/tubo) é uma varredura própria com os mesmos frames de Bishop, e o "clump" usa
//! [`crate::curve::hair_clump`].
//!
//! Módulos:
//! * [`build`] — curva → malha (cards em fita e mechas com seção; normais para cel shading com viés
//!   para fora da cabeça; UV ao longo do comprimento; cor de vértice com gradiente de raiz).
//! * [`surface`] — traço freehand → raycast no couro cabeludo (BVH) → curva elevada.
//! * [`comb`] — pincel de "pentear" (reorienta mechas preservando o comprimento).
//! * [`chain`] — grupos de bounce (auto 1–30) e [`chain::HairChainSpec`] para a física.
//!
//! Espaço: todas as posições estão no espaço **local do objeto-alvo** (a cabeça). O plano de simetria
//! é `x = 0` desse espaço. Tudo é determinístico e serializável (`serde`).

pub mod build;
pub mod chain;
pub mod comb;
pub mod surface;

use crate::curve::{Curve, CurveKind, CurvePoint};
use crate::error::{ModelError, ModelResult};
use glam::Vec3;
use serde::{Deserialize, Serialize};

/// Versão do formato serializado de [`HairSet`].
pub const HAIR_SCHEMA_VERSION: u32 = 1;
/// Limites de sanidade (evitam malhas gigantes por entrada inválida).
pub const MAX_GROUPS: usize = 64;
pub const MAX_STRANDS_PER_GROUP: usize = 4096;
pub const MAX_RESOLUTION: u32 = 64;
/// Raiz mais perto que isto do plano `x = 0` não ganha mecha espelhada (ficaria sobreposta).
pub const MIRROR_EPS: f32 = 0.002;

/// Forma da seção transversal da mecha.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum SectionKind {
    /// Fita plana (card de dois lados).
    #[default]
    Flat,
    /// Fita curva (arco de `curl` radianos).
    Arc,
    /// Mecha sólida de 3 lados (elipse achatada; barata e com bico natural).
    Triangle,
    /// Mecha sólida de 6 lados (elipse).
    Tube,
}

impl SectionKind {
    pub const ALL: [SectionKind; 4] = [SectionKind::Flat, SectionKind::Arc, SectionKind::Triangle, SectionKind::Tube];

    pub fn key(self) -> &'static str {
        match self {
            SectionKind::Flat => "hair.section.flat",
            SectionKind::Arc => "hair.section.arc",
            SectionKind::Triangle => "hair.section.triangle",
            SectionKind::Tube => "hair.section.tube",
        }
    }

    /// Seção fechada (varredura própria) em vez de card aberto.
    pub fn is_solid(self) -> bool {
        matches!(self, SectionKind::Triangle | SectionKind::Tube)
    }

    /// Lados da seção sólida (0 para cards).
    pub fn sides(self) -> usize {
        match self {
            SectionKind::Triangle => 3,
            SectionKind::Tube => 6,
            _ => 0,
        }
    }
}

/// Seção transversal da mecha: largura (eixo B do frame), espessura (eixo N), torção e curvatura.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct HairSection {
    pub kind: SectionKind,
    /// Largura na raiz (> 0), em unidades do objeto-alvo.
    pub width: f32,
    /// Espessura (só seções sólidas; > 0).
    pub thickness: f32,
    /// Torção total ao longo do comprimento (radianos).
    pub twist: f32,
    /// Arco da seção `Arc` (radianos, |curl| < 2π).
    pub curl: f32,
}

impl Default for HairSection {
    fn default() -> Self {
        Self {
            kind: SectionKind::Flat,
            width: 0.08,
            thickness: 0.02,
            twist: 0.0,
            curl: 1.2,
        }
    }
}

/// Mecha composta: a guia vira `count` fios finos (`hair_clump`), cada um renderizado como um card/mecha
/// de largura `strand_width`.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct HairClump {
    pub count: u32,
    pub spread: f32,
    pub tip_spread: f32,
    pub noise: f32,
    pub twist: f32,
    pub points: u32,
    pub length_variation: f32,
    pub seed: u64,
    pub strand_width: f32,
}

impl Default for HairClump {
    fn default() -> Self {
        Self {
            count: 5,
            spread: 0.025,
            tip_spread: 0.006,
            noise: 0.15,
            twist: 0.0,
            points: 10,
            length_variation: 0.12,
            seed: 1,
            strand_width: 0.025,
        }
    }
}

/// Parâmetros da mecha que não são geometria da guia (copiados para mechas novas).
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct StrandParams {
    pub section: HairSection,
    /// Largura na ponta como fração da largura da raiz (0 = ponta em bico, 1 = sem afinamento).
    pub tip_taper: f32,
    /// Expoente do afinamento (> 0): maior concentra o afinamento perto da ponta.
    pub taper_power: f32,
    /// Segmentos ao longo do comprimento (1..=[`MAX_RESOLUTION`]).
    pub resolution: u32,
    /// Bojo: largura × `(1 + flare·sin(π t))`.
    pub flare: f32,
    pub clump: Option<HairClump>,
}

impl Default for StrandParams {
    fn default() -> Self {
        Self {
            section: HairSection::default(),
            tip_taper: 0.0,
            taper_power: 1.3,
            resolution: 12,
            flare: 0.18,
            clump: None,
        }
    }
}

impl StrandParams {
    pub fn validate(&self) -> ModelResult<()> {
        let s = &self.section;
        for (v, what) in [(s.width, "width"), (s.thickness, "thickness")] {
            if !v.is_finite() || v <= 0.0 {
                return Err(ModelError::InvalidParameter(format!("{what} deve ser finito e > 0")));
            }
        }
        if !s.twist.is_finite() {
            return Err(ModelError::InvalidParameter("twist não finito".into()));
        }
        if !s.curl.is_finite() || s.curl.abs() >= std::f32::consts::TAU - 1e-3 {
            return Err(ModelError::InvalidParameter("curl deve estar em (−2π, 2π)".into()));
        }
        if !self.tip_taper.is_finite() || !(0.0..=1.0).contains(&self.tip_taper) {
            return Err(ModelError::InvalidParameter("tip_taper fora de [0, 1]".into()));
        }
        if !self.taper_power.is_finite() || self.taper_power <= 0.0 {
            return Err(ModelError::InvalidParameter("taper_power deve ser > 0".into()));
        }
        if self.resolution == 0 || self.resolution > MAX_RESOLUTION {
            return Err(ModelError::InvalidParameter(format!("resolution fora de 1..={MAX_RESOLUTION}")));
        }
        if !self.flare.is_finite() || self.flare < -1.0 {
            return Err(ModelError::InvalidParameter("flare deve ser >= −1".into()));
        }
        if let Some(c) = &self.clump {
            if c.count == 0 || c.count > 64 {
                return Err(ModelError::InvalidParameter("clump.count fora de 1..=64".into()));
            }
            if c.points < 2 || c.points > 64 {
                return Err(ModelError::InvalidParameter("clump.points fora de 2..=64".into()));
            }
            for (v, what) in [(c.spread, "spread"), (c.tip_spread, "tip_spread"), (c.noise, "noise")] {
                if !v.is_finite() || v < 0.0 {
                    return Err(ModelError::InvalidParameter(format!("clump.{what} deve ser >= 0")));
                }
            }
            if !c.strand_width.is_finite() || c.strand_width <= 0.0 {
                return Err(ModelError::InvalidParameter("clump.strand_width deve ser > 0".into()));
            }
            if !c.twist.is_finite() || !(0.0..1.0).contains(&c.length_variation) {
                return Err(ModelError::InvalidParameter("clump.twist/length_variation inválidos".into()));
            }
        }
        Ok(())
    }
}

/// Mecha de cabelo: guia + parâmetros. Raiz = primeiro ponto (a direção do traço define a direção da
/// mecha e depois a da cadeia de física).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct HairStrand {
    pub id: u32,
    /// Curva guia (raiz → ponta). Sempre `CatmullRom` aberta nas mechas criadas pela ferramenta.
    pub guide: Curve,
    #[serde(flatten)]
    pub params: StrandParams,
    /// Normal do couro cabeludo na raiz: orienta a face do card para fora da cabeça.
    pub root_normal: Vec3,
    /// Mecha espelhada (X) ligada a esta, quando a simetria do grupo estava ativa ao criar.
    pub mirror_of: Option<u32>,
}

impl HairStrand {
    pub fn new(id: u32, guide: Curve, params: StrandParams, root_normal: Vec3) -> Self {
        Self {
            id,
            guide,
            params,
            root_normal,
            mirror_of: None,
        }
    }

    pub fn root(&self) -> Vec3 {
        self.guide.points.first().map(|p| p.pos).unwrap_or(Vec3::ZERO)
    }

    pub fn tip(&self) -> Vec3 {
        self.guide.points.last().map(|p| p.pos).unwrap_or(Vec3::ZERO)
    }

    /// Comprimento de arco da guia.
    pub fn length(&self) -> f32 {
        self.guide.length()
    }

    pub fn validate(&self) -> ModelResult<()> {
        self.guide.validate()?;
        if self.guide.cyclic {
            return Err(ModelError::UnsupportedTopology("a guia de uma mecha não pode ser cíclica".into()));
        }
        self.params.validate()?;
        if !self.root_normal.is_finite() {
            return Err(ModelError::Numeric("root_normal não finito".into()));
        }
        Ok(())
    }

    /// Cópia espelhada no plano `x = 0` (involução: `m.mirrored_x().mirrored_x() == m`, salvo `id`/`mirror_of`).
    /// A reflexão inverte a lateralidade: `tilt` e `twist` mudam de sinal.
    pub fn mirrored_x(&self) -> HairStrand {
        let mx = |v: Vec3| Vec3::new(-v.x, v.y, v.z);
        let mut s = self.clone();
        for p in &mut s.guide.points {
            p.pos = mx(p.pos);
            p.handle_in = p.handle_in.map(mx);
            p.handle_out = p.handle_out.map(mx);
            p.tilt = -p.tilt;
        }
        s.params.section.twist = -s.params.section.twist;
        if let Some(c) = &mut s.params.clump {
            c.twist = -c.twist;
        }
        s.root_normal = mx(s.root_normal);
        s
    }

    /// Insere um ponto no meio do segmento `after → after+1` (posição na curva avaliada, não na corda).
    pub fn insert_point(&mut self, after: usize) -> ModelResult<usize> {
        let n = self.guide.points.len();
        if after + 1 >= n {
            return Err(ModelError::InvalidParameter("segmento inexistente".into()));
        }
        let t0 = after as f32 / (n - 1) as f32;
        let t1 = (after + 1) as f32 / (n - 1) as f32;
        let s = self.guide.sample(0.5 * (t0 + t1));
        let (a, b) = (self.guide.points[after], self.guide.points[after + 1]);
        let mut p = CurvePoint::new(s.pos);
        p.radius = 0.5 * (a.radius + b.radius);
        p.tilt = 0.5 * (a.tilt + b.tilt);
        self.guide.points.insert(after + 1, p);
        Ok(after + 1)
    }

    /// Remove um ponto de controle (mínimo 2). A raiz (índice 0) pode ser removida: o próximo vira raiz.
    pub fn remove_point(&mut self, idx: usize) -> ModelResult<()> {
        if self.guide.points.len() <= 2 {
            return Err(ModelError::InvalidParameter("a mecha precisa de pelo menos 2 pontos".into()));
        }
        if idx >= self.guide.points.len() {
            return Err(ModelError::InvalidParameter("ponto inexistente".into()));
        }
        self.guide.points.remove(idx);
        Ok(())
    }

    /// Reduz pontos de controle (Ramer–Douglas–Peucker) com tolerância `eps`, mantendo as pontas
    /// ("Smoothing" do VRoid). Devolve quantos pontos foram removidos.
    pub fn simplify(&mut self, eps: f32) -> usize {
        let pos: Vec<Vec3> = self.guide.points.iter().map(|p| p.pos).collect();
        let keep = surface::rdp_indices(&pos, eps);
        let removed = pos.len() - keep.len();
        if removed > 0 && keep.len() >= 2 {
            self.guide.points = keep.iter().map(|&i| self.guide.points[i]).collect();
        }
        removed
    }

    /// Afina a ponta: muda a razão de largura na ponta e o expoente (equivalente ao controle
    /// "afinar ponta"). `tip` em `[0, 1]`.
    pub fn taper_tip(&mut self, tip: f32, power: f32) -> ModelResult<()> {
        let mut p = self.params;
        p.tip_taper = tip;
        p.taper_power = power;
        p.validate()?;
        self.params = p;
        Ok(())
    }
}

/// Material do grupo (cel shading + gradiente de raiz). Slot de material da malha = `material_slot`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct HairMaterial {
    pub name: String,
    pub base_color: [f32; 3],
    pub shade_color: [f32; 3],
    pub highlight_color: [f32; 3],
    /// AO na raiz (canal R da cor de vértice): 1 = sem escurecimento.
    pub root_ao: f32,
    /// Viés das normais para fora da cabeça (0 = normal do card, 1 = esfera de sombreamento).
    pub normal_bias: f32,
}

impl Default for HairMaterial {
    fn default() -> Self {
        Self {
            name: "Cabelo".into(),
            base_color: [0.40, 0.26, 0.17],
            shade_color: [0.20, 0.11, 0.09],
            highlight_color: [0.93, 0.80, 0.62],
            root_ao: 0.62,
            normal_bias: 0.65,
        }
    }
}

/// Célula de atlas de texturas de mecha (`cols × rows`, origem V embaixo).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct HairAtlas {
    pub cols: u32,
    pub rows: u32,
    pub col: u32,
    pub row: u32,
}

/// Orientação da UV: o comprimento da mecha em V (convenção de `curve.rs`) ou em U (a tangente derivada
/// da UV por MikkTSpace passa a ser a direção do fio, útil para o brilho anisotrópico).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum UvLayout {
    #[default]
    LengthV,
    LengthU,
}

/// Grupo de cabelo: vira UM objeto da sessão de Modelagem com material próprio.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct HairGroup {
    pub id: u32,
    pub name: String,
    /// Mechas do grupo.
    pub strands: Vec<HairStrand>,
    /// Slot de material gravado nas faces (`Face::material`).
    pub material_slot: u16,
    pub material: HairMaterial,
    /// Simetria em X: mechas novas ganham a mecha espelhada e as edições se propagam ao par.
    pub symmetry: bool,
    pub visible: bool,
    /// Objeto da sessão que guarda a malha gerada (preenchido pela sessão).
    pub object_id: Option<u32>,
    pub atlas: Option<HairAtlas>,
    /// Cada mecha usa a célula `(id mod células)` do atlas.
    pub atlas_cycle: bool,
    pub uv_layout: UvLayout,
}

impl HairGroup {
    pub fn new(id: u32, name: impl Into<String>) -> Self {
        Self {
            id,
            name: name.into(),
            strands: Vec::new(),
            material_slot: 0,
            material: HairMaterial::default(),
            symmetry: false,
            visible: true,
            object_id: None,
            atlas: None,
            atlas_cycle: false,
            uv_layout: UvLayout::LengthV,
        }
    }

    pub fn strand(&self, id: u32) -> Option<&HairStrand> {
        self.strands.iter().find(|s| s.id == id)
    }

    pub fn strand_mut(&mut self, id: u32) -> Option<&mut HairStrand> {
        self.strands.iter_mut().find(|s| s.id == id)
    }

    pub fn validate(&self) -> ModelResult<()> {
        if self.strands.len() > MAX_STRANDS_PER_GROUP {
            return Err(ModelError::InvalidParameter(format!("grupo '{}' excede {MAX_STRANDS_PER_GROUP} mechas", self.name)));
        }
        let mut ids = std::collections::HashSet::new();
        for s in &self.strands {
            if !ids.insert(s.id) {
                return Err(ModelError::InvalidParameter(format!("id de mecha {} repetido no grupo '{}'", s.id, self.name)));
            }
            s.validate().map_err(|e| ModelError::InvalidParameter(format!("mecha {} do grupo '{}': {e}", s.id, self.name)))?;
            if let Some(m) = s.mirror_of {
                match self.strand(m) {
                    Some(p) if p.mirror_of == Some(s.id) => {}
                    _ => return Err(ModelError::InvalidParameter(format!("mecha {} aponta para um espelho inválido ({m})", s.id))),
                }
            }
        }
        if let Some(a) = &self.atlas {
            if a.cols == 0 || a.rows == 0 || a.col >= a.cols || a.row >= a.rows {
                return Err(ModelError::InvalidParameter("célula de atlas inválida".into()));
            }
        }
        if !(0.0..=1.0).contains(&self.material.root_ao) || !(0.0..=1.0).contains(&self.material.normal_bias) {
            return Err(ModelError::InvalidParameter("root_ao/normal_bias fora de [0, 1]".into()));
        }
        Ok(())
    }
}

/// Referência a uma mecha dentro do penteado.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub struct StrandRef {
    pub group: u32,
    pub strand: u32,
}

/// Penteado completo: grupos, grupos de bounce e metadados do alvo.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct HairSet {
    pub version: u32,
    pub groups: Vec<HairGroup>,
    pub bounce: Vec<chain::BounceGroup>,
    /// Objeto-alvo (a cabeça) na sessão; as mechas estão no espaço local dele.
    pub scalp_object: Option<u32>,
    /// Centro da cabeça (espaço local do alvo): origem do viés de normais.
    pub scalp_center: Vec3,
    /// Nome do osso/nó em que as cadeias de física se prendem (dica, convertida depois).
    pub root_node_hint: String,
    next_group_id: u32,
    next_strand_id: u32,
    next_bounce_id: u32,
}

impl Default for HairSet {
    fn default() -> Self {
        Self {
            version: HAIR_SCHEMA_VERSION,
            groups: Vec::new(),
            bounce: Vec::new(),
            scalp_object: None,
            scalp_center: Vec3::ZERO,
            root_node_hint: "head".into(),
            next_group_id: 1,
            next_strand_id: 1,
            next_bounce_id: 1,
        }
    }
}

impl HairSet {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn is_empty(&self) -> bool {
        self.groups.iter().all(|g| g.strands.is_empty())
    }

    pub fn strand_count(&self) -> usize {
        self.groups.iter().map(|g| g.strands.len()).sum()
    }

    pub fn group(&self, id: u32) -> Option<&HairGroup> {
        self.groups.iter().find(|g| g.id == id)
    }

    pub fn group_mut(&mut self, id: u32) -> Option<&mut HairGroup> {
        self.groups.iter_mut().find(|g| g.id == id)
    }

    pub fn strand(&self, r: StrandRef) -> Option<&HairStrand> {
        self.group(r.group)?.strand(r.strand)
    }

    pub fn strand_mut(&mut self, r: StrandRef) -> Option<&mut HairStrand> {
        self.group_mut(r.group)?.strand_mut(r.strand)
    }

    pub(crate) fn alloc_bounce_id(&mut self) -> u32 {
        let id = self.next_bounce_id;
        self.next_bounce_id += 1;
        id
    }

    fn alloc_strand_id(&mut self) -> u32 {
        let id = self.next_strand_id;
        self.next_strand_id += 1;
        id
    }

    // ------------------------------------------------------------------ grupos

    /// Novo grupo com o material padrão rotacionado por uma paleta (cores distintas por grupo).
    pub fn add_group(&mut self, name: impl Into<String>) -> ModelResult<u32> {
        if self.groups.len() >= MAX_GROUPS {
            return Err(ModelError::InvalidParameter(format!("limite de {MAX_GROUPS} grupos")));
        }
        let id = self.next_group_id;
        self.next_group_id += 1;
        let mut g = HairGroup::new(id, name);
        let palette: [([f32; 3], [f32; 3]); 6] = [
            ([0.40, 0.26, 0.17], [0.20, 0.11, 0.09]),
            ([0.86, 0.66, 0.30], [0.52, 0.34, 0.16]),
            ([0.55, 0.30, 0.62], [0.27, 0.14, 0.36]),
            ([0.30, 0.52, 0.78], [0.14, 0.24, 0.46]),
            ([0.78, 0.30, 0.34], [0.42, 0.13, 0.18]),
            ([0.82, 0.82, 0.86], [0.46, 0.46, 0.56]),
        ];
        let (base, shade) = palette[(id as usize - 1) % palette.len()];
        g.material.base_color = base;
        g.material.shade_color = shade;
        g.material.name = format!("Cabelo {id}");
        g.material_slot = (self.groups.len() as u16).min(u16::MAX - 1);
        self.groups.push(g);
        Ok(id)
    }

    pub fn rename_group(&mut self, id: u32, name: impl Into<String>) -> ModelResult<()> {
        let name = name.into();
        if name.trim().is_empty() {
            return Err(ModelError::InvalidParameter("nome de grupo vazio".into()));
        }
        self.group_mut(id).ok_or_else(|| ModelError::InvalidParameter(format!("grupo {id} inexistente")))?.name = name;
        Ok(())
    }

    /// Remove o grupo (e as referências a ele nos grupos de bounce). Devolve o grupo removido.
    pub fn remove_group(&mut self, id: u32) -> Option<HairGroup> {
        let pos = self.groups.iter().position(|g| g.id == id)?;
        let g = self.groups.remove(pos);
        for b in &mut self.bounce {
            b.strands.retain(|r| r.group != id);
        }
        Some(g)
    }

    fn clone_group_with_new_ids(&mut self, src: &HairGroup, name: String, mirror: bool) -> ModelResult<u32> {
        if self.groups.len() >= MAX_GROUPS {
            return Err(ModelError::InvalidParameter(format!("limite de {MAX_GROUPS} grupos")));
        }
        let id = self.next_group_id;
        self.next_group_id += 1;
        let mut g = src.clone();
        g.id = id;
        g.name = name;
        g.object_id = None;
        let mut remap = std::collections::HashMap::new();
        for s in &mut g.strands {
            let new = self.alloc_strand_id();
            remap.insert(s.id, new);
            if mirror {
                let mut m = s.mirrored_x();
                m.id = s.id;
                m.mirror_of = s.mirror_of;
                *s = m;
            }
            s.id = new;
        }
        for s in &mut g.strands {
            s.mirror_of = s.mirror_of.and_then(|m| remap.get(&m).copied());
        }
        self.groups.push(g);
        Ok(id)
    }

    /// Duplica o grupo (novos ids; sem objeto).
    pub fn duplicate_group(&mut self, id: u32) -> ModelResult<u32> {
        let src = self.group(id).cloned().ok_or_else(|| ModelError::InvalidParameter(format!("grupo {id} inexistente")))?;
        let name = format!("{} (cópia)", src.name);
        self.clone_group_with_new_ids(&src, name, false)
    }

    /// "Flip" do VRoid: cria um grupo novo com as mechas espelhadas em X do grupo `id`.
    /// `flip(flip(g))` reproduz as posições de `g` (involução, ver testes).
    pub fn flip_group(&mut self, id: u32) -> ModelResult<u32> {
        let src = self.group(id).cloned().ok_or_else(|| ModelError::InvalidParameter(format!("grupo {id} inexistente")))?;
        let name = format!("{} (espelho)", src.name);
        self.clone_group_with_new_ids(&src, name, true)
    }

    // ------------------------------------------------------------------ mechas

    /// Adiciona uma mecha ao grupo. Com `symmetry` no grupo e raiz fora do plano de espelho, cria também
    /// a mecha espelhada (ligada por `mirror_of`). Devolve os ids criados (1 ou 2; o primeiro é o original).
    pub fn add_strand(&mut self, group: u32, guide: Curve, params: StrandParams, root_normal: Vec3) -> ModelResult<Vec<u32>> {
        let mut guide = guide;
        guide.kind = CurveKind::CatmullRom;
        guide.cyclic = false;
        let probe = HairStrand::new(0, guide, params, root_normal);
        probe.validate()?;
        let g = self.group(group).ok_or_else(|| ModelError::InvalidParameter(format!("grupo {group} inexistente")))?;
        let symmetric = g.symmetry && probe.root().x.abs() > MIRROR_EPS;
        let needed = if symmetric { 2 } else { 1 };
        if g.strands.len() + needed > MAX_STRANDS_PER_GROUP {
            return Err(ModelError::InvalidParameter(format!("grupo cheio ({MAX_STRANDS_PER_GROUP} mechas)")));
        }
        let id = self.alloc_strand_id();
        let mut a = probe;
        a.id = id;
        let mut ids = vec![id];
        let mut mirror = None;
        if symmetric {
            let mid = self.alloc_strand_id();
            let mut m = a.mirrored_x();
            m.id = mid;
            m.mirror_of = Some(id);
            a.mirror_of = Some(mid);
            ids.push(mid);
            mirror = Some(m);
        }
        let g = self.group_mut(group).ok_or_else(|| ModelError::InvalidParameter("grupo inexistente".into()))?;
        g.strands.push(a);
        g.strands.extend(mirror);
        Ok(ids)
    }

    /// Remove a mecha (e o par espelhado, se houver). Devolve quantas foram removidas.
    pub fn remove_strand(&mut self, r: StrandRef) -> usize {
        let Some(g) = self.group_mut(r.group) else {
            return 0;
        };
        let partner = g.strand(r.strand).and_then(|s| s.mirror_of);
        let before = g.strands.len();
        g.strands.retain(|s| s.id != r.strand && Some(s.id) != partner);
        let removed = before - g.strands.len();
        for b in &mut self.bounce {
            b.strands.retain(|x| !(x.group == r.group && (x.strand == r.strand || Some(x.strand) == partner)));
        }
        removed
    }

    /// Duplica a mecha deslocando `offset` (o par espelhado, se existir, é duplicado com o deslocamento
    /// espelhado e os dois novos ficam ligados). Devolve o id da cópia da mecha pedida.
    pub fn duplicate_strand(&mut self, r: StrandRef, offset: Vec3) -> ModelResult<u32> {
        let g = self.group(r.group).ok_or_else(|| ModelError::InvalidParameter("grupo inexistente".into()))?;
        let src = g.strand(r.strand).cloned().ok_or_else(|| ModelError::InvalidParameter("mecha inexistente".into()))?;
        let partner = src.mirror_of.and_then(|m| g.strand(m).cloned());
        let needed = 1 + partner.is_some() as usize;
        if g.strands.len() + needed > MAX_STRANDS_PER_GROUP {
            return Err(ModelError::InvalidParameter(format!("grupo cheio ({MAX_STRANDS_PER_GROUP} mechas)")));
        }
        let shift = |s: &mut HairStrand, d: Vec3| {
            for p in &mut s.guide.points {
                p.pos += d;
                p.handle_in = p.handle_in.map(|h| h + d);
                p.handle_out = p.handle_out.map(|h| h + d);
            }
        };
        let id_a = self.alloc_strand_id();
        let mut a = src;
        a.id = id_a;
        a.mirror_of = None;
        shift(&mut a, offset);
        let mut b = None;
        if let Some(mut p) = partner {
            let id_b = self.alloc_strand_id();
            p.id = id_b;
            p.mirror_of = Some(id_a);
            shift(&mut p, Vec3::new(-offset.x, offset.y, offset.z));
            a.mirror_of = Some(id_b);
            b = Some(p);
        }
        let g = self.group_mut(r.group).ok_or_else(|| ModelError::InvalidParameter("grupo inexistente".into()))?;
        g.strands.push(a);
        g.strands.extend(b);
        Ok(id_a)
    }

    /// Move a mecha (e o par espelhado) para outro grupo.
    pub fn move_strand(&mut self, r: StrandRef, to: u32) -> ModelResult<()> {
        if r.group == to {
            return Ok(());
        }
        let dest = self.group(to).ok_or_else(|| ModelError::InvalidParameter(format!("grupo {to} inexistente")))?;
        let from = self.group(r.group).ok_or_else(|| ModelError::InvalidParameter("grupo inexistente".into()))?;
        let src = from.strand(r.strand).ok_or_else(|| ModelError::InvalidParameter("mecha inexistente".into()))?;
        let partner = src.mirror_of;
        let moving = 1 + partner.is_some() as usize;
        if dest.strands.len() + moving > MAX_STRANDS_PER_GROUP {
            return Err(ModelError::InvalidParameter("grupo de destino cheio".into()));
        }
        let from = self.group_mut(r.group).ok_or_else(|| ModelError::InvalidParameter("grupo inexistente".into()))?;
        let (take, keep): (Vec<_>, Vec<_>) = std::mem::take(&mut from.strands).into_iter().partition(|s| s.id == r.strand || Some(s.id) == partner);
        from.strands = keep;
        let dest = self.group_mut(to).ok_or_else(|| ModelError::InvalidParameter("grupo inexistente".into()))?;
        dest.strands.extend(take);
        for b in &mut self.bounce {
            for x in &mut b.strands {
                if x.group == r.group && (x.strand == r.strand || Some(x.strand) == partner) {
                    x.group = to;
                }
            }
        }
        Ok(())
    }

    /// Recalcula o par espelhado de `r` a partir da mecha (guia + parâmetros). No-op sem par.
    pub fn sync_mirror(&mut self, r: StrandRef) {
        let Some(g) = self.group_mut(r.group) else {
            return;
        };
        let Some(src) = g.strand(r.strand).cloned() else {
            return;
        };
        let Some(pid) = src.mirror_of else {
            return;
        };
        if let Some(p) = g.strand_mut(pid) {
            let mut m = src.mirrored_x();
            m.id = pid;
            m.mirror_of = Some(src.id);
            *p = m;
        }
    }

    /// Move o ponto de controle `idx` da mecha (e o do par espelhado).
    pub fn set_point_pos(&mut self, r: StrandRef, idx: usize, pos: Vec3) -> ModelResult<()> {
        if !pos.is_finite() {
            return Err(ModelError::Numeric("posição não finita".into()));
        }
        let s = self.strand_mut(r).ok_or_else(|| ModelError::InvalidParameter("mecha inexistente".into()))?;
        let p = s.guide.points.get_mut(idx).ok_or_else(|| ModelError::InvalidParameter("ponto inexistente".into()))?;
        p.pos = pos;
        p.handle_in = None;
        p.handle_out = None;
        self.sync_mirror(r);
        Ok(())
    }

    /// Insere um ponto no segmento `after → after+1` (propaga ao par espelhado). Devolve o novo índice.
    pub fn insert_point(&mut self, r: StrandRef, after: usize) -> ModelResult<usize> {
        let s = self.strand_mut(r).ok_or_else(|| ModelError::InvalidParameter("mecha inexistente".into()))?;
        let i = s.insert_point(after)?;
        self.sync_mirror(r);
        Ok(i)
    }

    /// Remove um ponto de controle (propaga ao par espelhado).
    pub fn remove_point(&mut self, r: StrandRef, idx: usize) -> ModelResult<()> {
        let s = self.strand_mut(r).ok_or_else(|| ModelError::InvalidParameter("mecha inexistente".into()))?;
        s.remove_point(idx)?;
        self.sync_mirror(r);
        Ok(())
    }

    /// Altera os parâmetros da mecha (e do par espelhado).
    pub fn set_params(&mut self, r: StrandRef, params: StrandParams) -> ModelResult<()> {
        params.validate()?;
        let s = self.strand_mut(r).ok_or_else(|| ModelError::InvalidParameter("mecha inexistente".into()))?;
        s.params = params;
        self.sync_mirror(r);
        Ok(())
    }

    /// Aplica `params` a TODAS as mechas do grupo (mantendo as guias).
    pub fn set_group_params(&mut self, group: u32, params: StrandParams) -> ModelResult<()> {
        params.validate()?;
        let g = self.group_mut(group).ok_or_else(|| ModelError::InvalidParameter("grupo inexistente".into()))?;
        for s in &mut g.strands {
            s.params = params;
        }
        // pares espelhados: a torção do par é o oposto
        let ids: Vec<u32> = g.strands.iter().filter(|s| s.mirror_of.is_some()).map(|s| s.id).collect();
        for id in ids {
            self.sync_mirror(StrandRef { group, strand: id });
        }
        Ok(())
    }

    /// Simplifica (RDP) todas as mechas do grupo. Devolve o total de pontos removidos.
    pub fn smooth_group(&mut self, group: u32, eps: f32) -> ModelResult<usize> {
        let g = self.group_mut(group).ok_or_else(|| ModelError::InvalidParameter("grupo inexistente".into()))?;
        let mut removed = 0;
        for s in &mut g.strands {
            removed += s.simplify(eps);
        }
        let ids: Vec<u32> = g.strands.iter().filter(|s| s.mirror_of.is_some()).map(|s| s.id).collect();
        for id in ids {
            self.sync_mirror(StrandRef { group, strand: id });
        }
        Ok(removed)
    }

    // --------------------------------------------------------------- validação

    pub fn validate(&self) -> ModelResult<()> {
        if self.version != HAIR_SCHEMA_VERSION {
            return Err(ModelError::InvalidParameter(format!("versão de cabelo {} não suportada", self.version)));
        }
        if self.groups.len() > MAX_GROUPS {
            return Err(ModelError::InvalidParameter(format!("mais de {MAX_GROUPS} grupos")));
        }
        let mut gids = std::collections::HashSet::new();
        let mut sids = std::collections::HashSet::new();
        for g in &self.groups {
            if !gids.insert(g.id) {
                return Err(ModelError::InvalidParameter(format!("id de grupo {} repetido", g.id)));
            }
            g.validate()?;
            for s in &g.strands {
                if !sids.insert(s.id) {
                    return Err(ModelError::InvalidParameter(format!("id de mecha {} repetido entre grupos", s.id)));
                }
            }
        }
        if !self.scalp_center.is_finite() {
            return Err(ModelError::Numeric("scalp_center não finito".into()));
        }
        for b in &self.bounce {
            b.validate()?;
            for r in &b.strands {
                if self.strand(*r).is_none() {
                    return Err(ModelError::InvalidParameter(format!("grupo de bounce '{}' referencia uma mecha inexistente", b.name)));
                }
            }
        }
        Ok(())
    }

    /// JSON canônico (para hash/export). Determinístico: a ordem dos campos é a das structs.
    pub fn to_json(&self) -> ModelResult<String> {
        serde_json::to_string(self).map_err(|e| ModelError::Numeric(format!("serialização: {e}")))
    }

    /// Lê e VALIDA um penteado serializado.
    pub fn from_json(s: &str) -> ModelResult<HairSet> {
        let set: HairSet = serde_json::from_str(s).map_err(|e| ModelError::InvalidParameter(format!("JSON de cabelo inválido: {e}")))?;
        set.validate()?;
        Ok(set)
    }

    /// Hash FNV-1a do JSON canônico: muda sempre que qualquer parâmetro/guia muda.
    pub fn content_hash(&self) -> u64 {
        let json = self.to_json().unwrap_or_default();
        let mut h: u64 = 0xcbf2_9ce4_8422_2325;
        for b in json.as_bytes() {
            h ^= *b as u64;
            h = h.wrapping_mul(0x100_0000_01b3);
        }
        h
    }
}

#[cfg(test)]
pub(crate) mod test_util {
    use super::*;

    /// Guia reta descendo em Y a partir de `root` (n pontos).
    pub fn drop_guide(root: Vec3, len: f32, n: usize) -> Curve {
        let pts: Vec<Vec3> = (0..n).map(|i| root - Vec3::Y * (len * i as f32 / (n - 1) as f32)).collect();
        Curve::from_positions(CurveKind::CatmullRom, &pts, false)
    }
}

#[cfg(test)]
mod tests {
    use super::test_util::drop_guide;
    use super::*;

    fn set_with_group(symmetry: bool) -> (HairSet, u32) {
        let mut s = HairSet::new();
        let g = s.add_group("Franja").unwrap();
        s.group_mut(g).unwrap().symmetry = symmetry;
        (s, g)
    }

    #[test]
    fn symmetry_creates_linked_mirror_and_involution_holds() {
        let (mut s, g) = set_with_group(true);
        let guide = drop_guide(Vec3::new(0.3, 1.0, 0.1), 0.8, 5);
        let ids = s.add_strand(g, guide, StrandParams::default(), Vec3::new(0.3, 0.9, 0.1).normalize()).unwrap();
        assert_eq!(ids.len(), 2);
        s.validate().unwrap();
        let a = s.strand(StrandRef { group: g, strand: ids[0] }).unwrap().clone();
        let b = s.strand(StrandRef { group: g, strand: ids[1] }).unwrap().clone();
        assert_eq!(b.mirror_of, Some(a.id));
        for (pa, pb) in a.guide.points.iter().zip(&b.guide.points) {
            assert!((pa.pos.x + pb.pos.x).abs() < 1e-6 && (pa.pos.y - pb.pos.y).abs() < 1e-6);
        }
        // involução do espelhamento
        let mut back = a.mirrored_x().mirrored_x();
        back.id = a.id;
        assert_eq!(back, a);
    }

    #[test]
    fn root_on_mirror_plane_has_no_partner() {
        let (mut s, g) = set_with_group(true);
        let ids = s.add_strand(g, drop_guide(Vec3::new(0.0, 1.0, 0.2), 0.5, 4), StrandParams::default(), Vec3::Y).unwrap();
        assert_eq!(ids.len(), 1);
    }

    #[test]
    fn editing_a_point_propagates_to_the_mirror_and_removal_removes_both() {
        let (mut s, g) = set_with_group(true);
        let ids = s.add_strand(g, drop_guide(Vec3::new(0.4, 1.0, 0.0), 0.8, 5), StrandParams::default(), Vec3::X).unwrap();
        let r = StrandRef { group: g, strand: ids[0] };
        s.set_point_pos(r, 2, Vec3::new(0.7, 0.4, 0.1)).unwrap();
        let partner = s.strand(StrandRef { group: g, strand: ids[1] }).unwrap();
        assert!((partner.guide.points[2].pos - Vec3::new(-0.7, 0.4, 0.1)).length() < 1e-6);
        s.insert_point(r, 1).unwrap();
        assert_eq!(s.strand(r).unwrap().guide.points.len(), 6);
        assert_eq!(s.strand(StrandRef { group: g, strand: ids[1] }).unwrap().guide.points.len(), 6);
        s.validate().unwrap();
        assert_eq!(s.remove_strand(r), 2);
        assert!(s.group(g).unwrap().strands.is_empty());
    }

    #[test]
    fn flip_group_is_an_involution_on_positions() {
        let (mut s, g) = set_with_group(true);
        s.add_strand(g, drop_guide(Vec3::new(0.3, 1.0, 0.1), 0.8, 5), StrandParams::default(), Vec3::X).unwrap();
        s.add_strand(g, drop_guide(Vec3::new(0.2, 1.1, -0.2), 0.6, 4), StrandParams::default(), Vec3::X).unwrap();
        let f1 = s.flip_group(g).unwrap();
        let f2 = s.flip_group(f1).unwrap();
        let (orig, back) = (s.group(g).unwrap(), s.group(f2).unwrap());
        assert_eq!(orig.strands.len(), back.strands.len());
        for (a, b) in orig.strands.iter().zip(&back.strands) {
            for (pa, pb) in a.guide.points.iter().zip(&b.guide.points) {
                assert!((pa.pos - pb.pos).length() < 1e-6);
            }
        }
        s.validate().unwrap();
    }

    #[test]
    fn groups_rename_move_duplicate() {
        let (mut s, g1) = set_with_group(false);
        let g2 = s.add_group("Lateral").unwrap();
        let ids = s.add_strand(g1, drop_guide(Vec3::new(0.3, 1.0, 0.1), 0.8, 5), StrandParams::default(), Vec3::X).unwrap();
        let r = StrandRef { group: g1, strand: ids[0] };
        s.move_strand(r, g2).unwrap();
        assert!(s.group(g1).unwrap().strands.is_empty());
        assert_eq!(s.group(g2).unwrap().strands.len(), 1);
        s.rename_group(g2, "Lateral E").unwrap();
        assert!(s.rename_group(g2, "  ").is_err());
        let copy = s.duplicate_strand(StrandRef { group: g2, strand: ids[0] }, Vec3::new(0.05, 0.0, 0.0)).unwrap();
        assert_ne!(copy, ids[0]);
        let dup = s.duplicate_group(g2).unwrap();
        assert_eq!(s.group(dup).unwrap().strands.len(), 2);
        s.validate().unwrap();
        assert!(s.remove_group(g2).is_some());
        assert!(s.group(g2).is_none());
    }

    #[test]
    fn invalid_input_is_rejected_without_mutation() {
        let (mut s, g) = set_with_group(false);
        let before = s.clone();
        let one_point = Curve::from_positions(CurveKind::CatmullRom, &[Vec3::ZERO], false);
        assert!(s.add_strand(g, one_point, StrandParams::default(), Vec3::Y).is_err());
        let mut bad = StrandParams::default();
        bad.section.width = -1.0;
        assert!(s.add_strand(g, drop_guide(Vec3::Y, 1.0, 3), bad, Vec3::Y).is_err());
        bad = StrandParams::default();
        bad.resolution = 0;
        assert!(bad.validate().is_err());
        bad = StrandParams::default();
        bad.tip_taper = 1.5;
        assert!(bad.validate().is_err());
        let nan = Curve::from_positions(CurveKind::CatmullRom, &[Vec3::ZERO, Vec3::new(f32::NAN, 0.0, 0.0)], false);
        assert!(s.add_strand(g, nan, StrandParams::default(), Vec3::Y).is_err());
        assert!(s.add_strand(99, drop_guide(Vec3::Y, 1.0, 3), StrandParams::default(), Vec3::Y).is_err());
        assert_eq!(s, before, "uma operação que falha não pode alterar o penteado");
    }

    #[test]
    fn serde_round_trip_is_deterministic_and_validated() {
        let (mut s, g) = set_with_group(true);
        let mut p = StrandParams::default();
        p.clump = Some(HairClump::default());
        p.section.kind = SectionKind::Tube;
        s.add_strand(g, drop_guide(Vec3::new(0.3, 1.0, 0.1), 0.8, 5), p, Vec3::X).unwrap();
        let j1 = s.to_json().unwrap();
        let back = HairSet::from_json(&j1).unwrap();
        assert_eq!(back, s);
        assert_eq!(back.to_json().unwrap(), j1);
        assert_eq!(back.content_hash(), s.content_hash());
        let mut s2 = s.clone();
        s2.set_point_pos(StrandRef { group: g, strand: 1 }, 1, Vec3::new(0.31, 0.8, 0.1)).unwrap();
        assert_ne!(s2.content_hash(), s.content_hash());
        // JSON adulterado (id de mecha repetido) é recusado
        let tampered = j1.replacen("\"id\":2", "\"id\":1", 1);
        assert!(HairSet::from_json(&tampered).is_err());
        assert!(HairSet::from_json("{}").is_err());
    }

    #[test]
    fn simplify_and_taper_tip() {
        let mut st = HairStrand::new(1, drop_guide(Vec3::Y, 1.0, 20), StrandParams::default(), Vec3::Y);
        let removed = st.simplify(0.001);
        assert!(removed >= 15, "uma reta de 20 pontos deve virar ~2 pontos, removeu {removed}");
        assert!(st.guide.points.len() >= 2);
        st.taper_tip(0.0, 2.0).unwrap();
        assert_eq!(st.params.tip_taper, 0.0);
        assert!(st.taper_tip(2.0, 1.0).is_err());
        assert_eq!(st.params.taper_power, 2.0, "falha não altera");
    }
}
