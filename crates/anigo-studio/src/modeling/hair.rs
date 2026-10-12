//! Ferramenta de cabelo (Modo Cabelo) do workspace Modelagem: estado da ferramenta, ações com undo
//! (checkpoint por traço/ação) e regeneração das malhas dos grupos como objetos da sessão.
//!
//! O dado vive em `Document::hair` ([`anigo_modeling::hair::HairSet`], espaço local da cabeça-alvo);
//! cada grupo vira UM objeto da sessão (`HairGroup::object_id`) com material próprio, regenerado a cada
//! edição. Como o `Document` inteiro entra no snapshot do undo, desfazer/refazer restaura mechas E objetos.
//! Sem egui: o `app::hair_ui` só traduz entrada e desenha.

use super::sculpt::{ray_to_local, ViewInfo};
use super::session::{Mode, ModelObject, ModelingSession, ObjectMaterial, DEFAULT_AUTO_SMOOTH};
use crate::viewport::camera::intersect_plane;
use anigo_core::math::Transform;
use anigo_modeling::bvh::TriBvh;
use anigo_modeling::hair::build::build_group;
use anigo_modeling::hair::surface::{strand_curve_from_stroke, StrokeAccumulator, StrokeParams};
use anigo_modeling::hair::{StrandParams, StrandRef};
use glam::{Vec2, Vec3};

/// Ferramentas do Modo Cabelo (ids do shelf: `hair.draw`, `hair.edit`, `hair.comb`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum HairTool {
    /// Desenhar mechas sobre a cabeça (freehand projetado na superfície).
    #[default]
    Draw,
    /// Selecionar mechas e editar pontos de controle.
    Edit,
    /// Pentear: pincel que reorienta mechas próximas.
    Comb,
}

impl HairTool {
    pub const ALL: [HairTool; 3] = [HairTool::Draw, HairTool::Edit, HairTool::Comb];

    pub fn tool_id(self) -> &'static str {
        match self {
            HairTool::Draw => "hair.draw",
            HairTool::Edit => "hair.edit",
            HairTool::Comb => "hair.comb",
        }
    }

    pub fn index(self) -> usize {
        Self::ALL.iter().position(|t| *t == self).unwrap_or(0)
    }

    pub fn from_index(i: usize) -> HairTool {
        Self::ALL.get(i).copied().unwrap_or_default()
    }
}

/// Alvo de um clique na ferramenta Editar.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum HairPick {
    /// Ponto de controle `idx` da mecha.
    Point(StrandRef, usize),
    /// Segmento `after → after+1` da mecha (posição do clique no segmento, espaço local).
    Segment(StrandRef, usize, Vec3),
}

/// Passo de um traço roteirizado (cenários de screenshot e testes): pontos em espaço LOCAL da cabeça,
/// projetados pela câmera real e enviados pelo mesmo caminho do mouse.
#[derive(Debug, Clone)]
pub enum ScriptStep {
    NewGroup(&'static str),
    Symmetry(bool),
    /// Traço de desenho (cada ponto vira um raio de câmera).
    Draw(Vec<Vec3>),
    /// Passada do pente (pontos em espaço local → pixels).
    Comb(Vec<Vec3>),
    /// Seleciona a última mecha criada e o ponto indicado.
    SelectLast(Option<usize>),
}

struct PointDrag {
    strand: StrandRef,
    idx: usize,
    plane_point: Vec3,
    checkpointed: bool,
}

struct CombRun {
    checkpointed: bool,
    last: Vec2,
    changed: bool,
}

struct BvhCache {
    obj_id: u32,
    revision: u64,
    hash: u64,
    bvh: TriBvh,
}

/// Estado da ferramenta de cabelo.
pub struct HairState {
    pub tool: HairTool,
    /// Grupo ativo (recebe mechas novas).
    pub group: Option<u32>,
    pub selected: Option<StrandRef>,
    pub point: Option<usize>,
    pub stroke: StrokeParams,
    /// Parâmetros copiados para as mechas novas.
    pub template: StrandParams,
    pub comb_radius_px: f32,
    pub comb_strength: f32,
    pub comb_all_groups: bool,
    pub bounce_groups: usize,
    pub bounce_bones: u32,
    pub error: Option<String>,
    pub script: Option<Vec<ScriptStep>>,
    /// Último ponto de superfície sob o cursor (local), para o anel do pincel.
    pub hover: Option<Vec3>,
    /// Traço de desenho em andamento (visível no overlay).
    draw: Option<StrokeAccumulator>,
    drag: Option<PointDrag>,
    comb: Option<CombRun>,
    bvh: Option<BvhCache>,
    /// Gesto contínuo (slider) já checkpointado.
    gesture: Option<String>,
}

impl Default for HairState {
    fn default() -> Self {
        Self::new()
    }
}

impl HairState {
    pub fn new() -> Self {
        Self {
            tool: HairTool::Draw,
            group: None,
            selected: None,
            point: None,
            stroke: StrokeParams::default(),
            template: StrandParams::default(),
            comb_radius_px: 70.0,
            comb_strength: 0.35,
            comb_all_groups: true,
            bounce_groups: 6,
            bounce_bones: 4,
            error: None,
            script: None,
            hover: None,
            draw: None,
            drag: None,
            comb: None,
            bvh: None,
            gesture: None,
        }
    }

    pub fn is_drawing(&self) -> bool {
        self.draw.is_some()
    }

    pub fn is_busy(&self) -> bool {
        self.draw.is_some() || self.drag.is_some() || self.comb.is_some()
    }

    /// Amostras do traço em andamento (espaço local) para o overlay.
    pub fn draw_preview(&self) -> Option<&[anigo_modeling::hair::surface::SurfaceSample]> {
        self.draw.as_ref().map(|d| d.samples())
    }

    /// Marca o fim de um gesto contínuo (soltou o ponteiro).
    pub fn end_gesture(&mut self) {
        self.gesture = None;
    }

    /// Grupo ativo válido (existente) ou o primeiro.
    pub fn active_group(&mut self, session: &ModelingSession) -> Option<u32> {
        let hair = &session.doc.hair;
        if let Some(g) = self.group.filter(|g| hair.group(*g).is_some()) {
            return Some(g);
        }
        self.group = hair.groups.first().map(|g| g.id);
        self.group
    }

    /// Descarta seleções que apontam para algo que já não existe (após undo/redo, remoções).
    pub fn prune(&mut self, session: &ModelingSession) {
        let hair = &session.doc.hair;
        if self.group.is_some_and(|g| hair.group(g).is_none()) {
            self.group = hair.groups.first().map(|g| g.id);
        }
        match self.selected {
            Some(r) if hair.strand(r).is_none() => {
                self.selected = None;
                self.point = None;
            }
            Some(r) => {
                let n = hair.strand(r).map(|s| s.guide.points.len()).unwrap_or(0);
                if self.point.is_some_and(|p| p >= n) {
                    self.point = None;
                }
            }
            None => self.point = None,
        }
    }

    // ------------------------------------------------------------ vínculo com o alvo

    /// Reconstrói o BVH do alvo quando a malha dele mudou (a regeneração do cabelo não o invalida).
    pub fn sync(&mut self, session: &ModelingSession) {
        let Some(id) = session.doc.hair.scalp_object else {
            self.bvh = None;
            return;
        };
        let Some(obj) = session.doc.objects.iter().find(|o| o.id == id) else {
            self.bvh = None;
            return;
        };
        if let Some(c) = &self.bvh {
            if c.obj_id == id && c.revision == session.revision {
                return;
            }
        }
        let hash = obj.mesh.content_hash();
        if let Some(c) = self.bvh.as_mut() {
            if c.obj_id == id && c.hash == hash {
                c.revision = session.revision;
                return;
            }
        }
        self.bvh = Some(BvhCache {
            obj_id: id,
            revision: session.revision,
            hash,
            bvh: TriBvh::build(&obj.mesh),
        });
    }

    pub fn bvh(&self) -> Option<&TriBvh> {
        self.bvh.as_ref().map(|c| &c.bvh)
    }

    /// Raio de MUNDO → local do alvo.
    fn local_ray(session: &ModelingSession, ray: (Vec3, Vec3)) -> Option<(Vec3, Vec3)> {
        let obj = session.hair_target()?;
        let (o, d) = ray_to_local(obj.model_matrix(), ray.0, ray.1);
        (d != Vec3::ZERO).then_some((o, d))
    }

    /// Atualiza o ponto de hover (anel do pincel) a partir do raio de mundo do cursor.
    pub fn update_hover(&mut self, session: &ModelingSession, ray: (Vec3, Vec3)) {
        self.hover = None;
        let (Some((o, d)), Some(bvh)) = (Self::local_ray(session, ray), self.bvh()) else {
            return;
        };
        self.hover = bvh.raycast_ex(o, d, f32::INFINITY, true).map(|h| h.point);
    }

    // ------------------------------------------------------------ desenhar

    /// Inicia um traço com o raio de mundo do cursor. `Ok(false)` se o raio não acerta a cabeça.
    pub fn begin_draw(&mut self, session: &mut ModelingSession, ray: (Vec3, Vec3)) -> Result<bool, String> {
        self.sync(session);
        if session.hair_target().is_none() {
            return Err("Selecione a cabeça (objeto-alvo) antes de desenhar".into());
        }
        let Some((o, d)) = Self::local_ray(session, ray) else {
            return Ok(false);
        };
        let Some(bvh) = self.bvh() else {
            return Err("A cabeça-alvo não tem malha".into());
        };
        let mut acc = StrokeAccumulator::new((self.stroke.resample_step * 0.4).max(1e-4));
        if !acc.push_ray(bvh, o, d) {
            return Ok(false);
        }
        self.draw = Some(acc);
        self.error = None;
        Ok(true)
    }

    pub fn continue_draw(&mut self, session: &ModelingSession, ray: (Vec3, Vec3)) {
        let (Some(acc), Some((o, d))) = (self.draw.as_mut(), Self::local_ray(session, ray)) else {
            return;
        };
        if let Some(c) = self.bvh.as_ref() {
            acc.push_ray(&c.bvh, o, d);
        }
    }

    pub fn cancel_draw(&mut self) {
        self.draw = None;
    }

    /// Fecha o traço: gera a guia, cria a mecha (e o espelho) e regenera a malha, com um checkpoint.
    /// `Ok(None)` se o traço foi curto demais (nada muda, nem o histórico).
    pub fn end_draw(&mut self, session: &mut ModelingSession) -> Result<Option<StrandRef>, String> {
        let Some(acc) = self.draw.take() else {
            return Ok(None);
        };
        if acc.len() < 2 {
            return Ok(None);
        }
        let from_stroke = strand_curve_from_stroke(acc.samples(), &self.stroke, self.bvh()).map_err(|e| e.to_string());
        let from_stroke = match from_stroke {
            Ok(s) => s,
            Err(e) if e.contains("curto") => return Ok(None),
            Err(e) => {
                self.error = Some(e.clone());
                return Err(e);
            }
        };
        session.checkpoint("Desenhar mecha");
        let gid = match self.active_group(session) {
            Some(g) => g,
            None => match session.doc.hair.add_group("Grupo 1") {
                Ok(g) => {
                    self.group = Some(g);
                    g
                }
                Err(e) => {
                    session.discard_checkpoint();
                    return Err(e.to_string());
                }
            },
        };
        let ids = match session.doc.hair.add_strand(gid, from_stroke.guide, self.template, from_stroke.root_normal) {
            Ok(ids) => ids,
            Err(e) => {
                session.discard_checkpoint();
                let msg = e.to_string();
                self.error = Some(msg.clone());
                return Err(msg);
            }
        };
        if let Err(e) = session.hair_regenerate(Some(gid)) {
            // a malha não pôde ser gerada: volta ao estado anterior sem sujar o histórico
            session.undo();
            session.drop_redo_tip();
            self.error = Some(e.clone());
            return Err(e);
        }
        let r = StrandRef { group: gid, strand: ids[0] };
        self.selected = Some(r);
        self.point = None;
        self.error = None;
        session.status = format!("Mecha desenhada ({} pontos de controle)", session.doc.hair.strand(r).map(|s| s.guide.points.len()).unwrap_or(0));
        Ok(Some(r))
    }

    // ------------------------------------------------------------ editar pontos

    /// Procura o ponto de controle (prioridade) ou segmento mais próximo do cursor, em pixels.
    pub fn pick(&self, session: &ModelingSession, view: &ViewInfo, mouse: Vec2) -> Option<HairPick> {
        let obj = session.hair_target()?;
        let model = obj.model_matrix();
        let to_px = |p: Vec3| view.camera.project(model.transform_point3(p), view.size).map(|(px, _)| px);
        let mut best_pt: Option<(f32, StrandRef, usize)> = None;
        let mut best_seg: Option<(f32, StrandRef, usize, Vec3)> = None;
        for g in session.doc.hair.groups.iter().filter(|g| g.visible) {
            for s in &g.strands {
                let r = StrandRef { group: g.id, strand: s.id };
                for (i, p) in s.guide.points.iter().enumerate() {
                    if let Some(px) = to_px(p.pos) {
                        let d = px.distance(mouse);
                        if d < 10.0 && best_pt.is_none_or(|b| d < b.0) {
                            best_pt = Some((d, r, i));
                        }
                    }
                }
                let n = s.guide.points.len();
                let samples = s.guide.sample_uniform((n * 8).max(16)).unwrap_or_default();
                for w in samples.windows(2) {
                    let (Some(a), Some(b)) = (to_px(w[0].pos), to_px(w[1].pos)) else {
                        continue;
                    };
                    let ab = b - a;
                    let len2 = ab.length_squared().max(1e-9);
                    let t = ((mouse - a).dot(ab) / len2).clamp(0.0, 1.0);
                    let d = (a + ab * t).distance(mouse);
                    if d < 7.0 && best_seg.is_none_or(|bs| d < bs.0) {
                        // segmento de controle que contém este trecho: pela fração de comprimento
                        let frac = 0.5 * (w[0].t + w[1].t);
                        let after = ((frac * (n - 1) as f32).floor() as usize).min(n - 2);
                        best_seg = Some((d, r, after, w[0].pos.lerp(w[1].pos, t)));
                    }
                }
            }
        }
        if let Some((_, r, i)) = best_pt {
            return Some(HairPick::Point(r, i));
        }
        best_seg.map(|(_, r, i, p)| HairPick::Segment(r, i, p))
    }

    pub fn select(&mut self, r: Option<StrandRef>, point: Option<usize>) {
        self.selected = r;
        self.point = point;
        if let Some(r) = r {
            self.group = Some(r.group);
        }
    }

    /// Começa a arrastar o ponto `idx` (o checkpoint só é gravado no primeiro movimento).
    pub fn begin_point_drag(&mut self, session: &ModelingSession, r: StrandRef, idx: usize) {
        let Some(p) = session.doc.hair.strand(r).and_then(|s| s.guide.points.get(idx)).map(|p| p.pos) else {
            return;
        };
        let world = session.hair_target().map(|o| o.model_matrix().transform_point3(p)).unwrap_or(p);
        self.drag = Some(PointDrag {
            strand: r,
            idx,
            plane_point: world,
            checkpointed: false,
        });
        self.select(Some(r), Some(idx));
    }

    pub fn is_dragging_point(&self) -> bool {
        self.drag.is_some()
    }

    /// Move o ponto arrastado para onde o raio do cursor cruza o plano da tela pelo ponto. A raiz
    /// (índice 0) é recolada no couro cabeludo.
    pub fn drag_point(&mut self, session: &mut ModelingSession, view: &ViewInfo, mouse: Vec2) -> Result<(), String> {
        let Some(d) = self.drag.as_ref() else {
            return Ok(());
        };
        let (r, idx, plane_point, checkpointed) = (d.strand, d.idx, d.plane_point, d.checkpointed);
        let (o, dir) = view.ray(mouse);
        let Some(world) = intersect_plane(o, dir, plane_point, view.camera.forward()) else {
            return Ok(());
        };
        let Some(model) = session.hair_target().map(|t| t.model_matrix()) else {
            return Ok(());
        };
        let mut local = model.inverse().transform_point3(world);
        if idx == 0 {
            self.sync(session);
            if let Some(c) = self.bvh() {
                if let Some(h) = c.closest_point(local) {
                    local = h.point;
                }
            }
        }
        let current = session.doc.hair.strand(r).and_then(|s| s.guide.points.get(idx)).map(|p| p.pos);
        if current.is_some_and(|c| c.distance(local) < 1e-6) {
            return Ok(());
        }
        if !checkpointed {
            session.checkpoint("Mover ponto da mecha");
            if let Some(d) = self.drag.as_mut() {
                d.checkpointed = true;
            }
        }
        session.doc.hair.set_point_pos(r, idx, local).map_err(|e| e.to_string())?;
        session.hair_regenerate(Some(r.group))
    }

    pub fn end_point_drag(&mut self) {
        self.drag = None;
    }

    /// Insere um ponto no segmento clicado.
    pub fn insert_point(&mut self, session: &mut ModelingSession, r: StrandRef, after: usize) -> Result<(), String> {
        session.checkpoint("Inserir ponto");
        match session.doc.hair.insert_point(r, after) {
            Ok(i) => {
                session.hair_regenerate(Some(r.group))?;
                self.select(Some(r), Some(i));
                Ok(())
            }
            Err(e) => {
                session.discard_checkpoint();
                Err(e.to_string())
            }
        }
    }

    /// Remove o ponto selecionado (ou a mecha toda se `whole` ou se não há ponto selecionado).
    pub fn delete_selected(&mut self, session: &mut ModelingSession, whole: bool) -> Result<(), String> {
        let Some(r) = self.selected else {
            return Err("Nenhuma mecha selecionada".into());
        };
        if let (Some(idx), false) = (self.point, whole) {
            session.checkpoint("Remover ponto");
            if let Err(e) = session.doc.hair.remove_point(r, idx) {
                session.discard_checkpoint();
                return Err(e.to_string());
            }
            self.point = None;
        } else {
            session.checkpoint("Remover mecha");
            session.doc.hair.remove_strand(r);
            self.selected = None;
            self.point = None;
        }
        session.hair_regenerate(Some(r.group))
    }

    /// Duplica a mecha selecionada (deslocada) e a seleciona.
    pub fn duplicate_selected(&mut self, session: &mut ModelingSession) -> Result<(), String> {
        let Some(r) = self.selected else {
            return Err("Nenhuma mecha selecionada".into());
        };
        let offset = Vec3::new(0.03, 0.0, 0.03);
        session.checkpoint("Duplicar mecha");
        match session.doc.hair.duplicate_strand(r, offset) {
            Ok(id) => {
                session.hair_regenerate(Some(r.group))?;
                self.select(Some(StrandRef { group: r.group, strand: id }), None);
                Ok(())
            }
            Err(e) => {
                session.discard_checkpoint();
                Err(e.to_string())
            }
        }
    }

    // ------------------------------------------------------------ pentear

    pub fn begin_comb(&mut self, mouse: Vec2) {
        self.comb = Some(CombRun {
            checkpointed: false,
            last: mouse,
            changed: false,
        });
    }

    pub fn is_combing(&self) -> bool {
        self.comb.is_some()
    }

    /// Um passo do pente: o movimento do cursor (em pixels) vira uma direção de mundo; os segmentos
    /// com ponto médio a menos de `comb_radius_px` do cursor giram para ela.
    pub fn comb_move(&mut self, session: &mut ModelingSession, view: &ViewInfo, mouse: Vec2) -> Result<usize, String> {
        let Some(run) = self.comb.as_mut() else {
            return Ok(0);
        };
        let delta = mouse - run.last;
        if delta.length() < 2.0 {
            return Ok(0);
        }
        run.last = mouse;
        let Some(model) = session.hair_target().map(|t| t.model_matrix()) else {
            return Ok(0);
        };
        let world_dir = view.camera.right() * delta.x - view.camera.up() * delta.y;
        let local_dir = model.inverse().transform_vector3(world_dir);
        let (radius, strength) = (self.comb_radius_px, self.comb_strength.clamp(0.0, 1.0));
        let camera = view.camera;
        let size = view.size;
        let weight = move |_s: &anigo_modeling::hair::HairStrand, mid: Vec3, _i: usize| -> f32 {
            let Some((px, _)) = camera.project(model.transform_point3(mid), size) else {
                return 0.0;
            };
            let d = px.distance(mouse) / radius;
            if d >= 1.0 {
                0.0
            } else {
                strength * (1.0 - d) * (1.0 - d)
            }
        };
        // checkpoint só se algo realmente mudar: testa numa cópia barata do conjunto de grupos
        let groups: Vec<u32> = session
            .doc
            .hair
            .groups
            .iter()
            .filter(|g| g.visible && (self.comb_all_groups || self.group == Some(g.id)))
            .map(|g| g.id)
            .collect();
        let mut touched = 0;
        let before = session.doc.hair.clone();
        for gid in &groups {
            touched += session.doc.hair.comb(*gid, local_dir, &weight).len();
        }
        if touched == 0 {
            return Ok(0);
        }
        let run = self.comb.as_mut().ok_or("pente inativo")?;
        if !run.checkpointed {
            // grava o estado ANTERIOR ao primeiro passo
            let after = std::mem::replace(&mut session.doc.hair, before);
            session.checkpoint("Pentear mechas");
            session.doc.hair = after;
            run.checkpointed = true;
        }
        run.changed = true;
        for gid in groups {
            session.hair_regenerate(Some(gid))?;
        }
        Ok(touched)
    }

    pub fn end_comb(&mut self) {
        self.comb = None;
    }

    // ------------------------------------------------------------ gestos contínuos

    /// Checkpoint para um gesto contínuo (slider/arrasto): só o primeiro passo grava; as mudanças
    /// seguintes com a mesma `key` reaproveitam o mesmo ponto de undo até [`Self::end_gesture`].
    pub fn gesture_checkpoint(&mut self, session: &mut ModelingSession, key: &str, label: &str) {
        if self.gesture.as_deref() != Some(key) {
            session.checkpoint(label);
            self.gesture = Some(key.to_string());
        }
    }

    // ------------------------------------------------------------ roteiro (cenários)

    /// Executa um roteiro pelo mesmo caminho do mouse: pontos locais → pixels da câmera real → raios.
    pub fn run_script(&mut self, session: &mut ModelingSession, view: &ViewInfo, steps: &[ScriptStep]) {
        let Some(model) = session.hair_target().map(|t| t.model_matrix()) else {
            return;
        };
        let ray_for = |p: Vec3| -> Option<(Vec3, Vec3)> {
            let (px, _) = view.camera.project(model.transform_point3(p), view.size)?;
            Some(view.ray(px))
        };
        for step in steps {
            match step {
                ScriptStep::NewGroup(name) => {
                    session.checkpoint("Novo grupo");
                    match session.doc.hair.add_group(*name) {
                        Ok(id) => self.group = Some(id),
                        Err(_) => session.discard_checkpoint(),
                    }
                }
                ScriptStep::Symmetry(on) => {
                    if let Some(g) = self.active_group(session) {
                        let _ = session.doc.hair.set_symmetry(g, *on);
                    }
                }
                ScriptStep::Draw(points) => {
                    let mut rays = points.iter().filter_map(|p| ray_for(*p));
                    let Some(first) = rays.next() else {
                        continue;
                    };
                    if matches!(self.begin_draw(session, first), Ok(true)) {
                        for r in rays {
                            self.continue_draw(session, r);
                        }
                        if let Err(e) = self.end_draw(session) {
                            self.error = Some(e);
                        }
                    }
                }
                ScriptStep::Comb(points) => {
                    let px: Vec<Vec2> = points
                        .iter()
                        .filter_map(|p| view.camera.project(model.transform_point3(*p), view.size).map(|(px, _)| px))
                        .collect();
                    if let Some(first) = px.first() {
                        self.begin_comb(*first);
                        for p in &px[1..] {
                            let _ = self.comb_move(session, view, *p);
                        }
                        self.end_comb();
                    }
                }
                ScriptStep::SelectLast(point) => {
                    if let Some(g) = self.active_group(session).and_then(|g| session.doc.hair.group(g)) {
                        if let Some(s) = g.strands.last() {
                            self.select(Some(StrandRef { group: g.id, strand: s.id }), *point);
                        }
                    }
                }
            }
        }
    }
}

// ============================================================================ sessão

impl ModelingSession {
    /// Objeto-alvo (a cabeça) do penteado, se existir.
    pub fn hair_target(&self) -> Option<&ModelObject> {
        let id = self.doc.hair.scalp_object?;
        self.doc.objects.iter().find(|o| o.id == id)
    }

    /// Define o objeto ativo como cabeça-alvo (centro = centro dos limites da malha, espaço local).
    pub fn hair_set_target_from_active(&mut self) -> bool {
        let Some(obj) = self.active_object() else {
            return false;
        };
        let (id, center) = (obj.id, obj.mesh.bounds().map(|(lo, hi)| 0.5 * (lo + hi)).unwrap_or(Vec3::ZERO));
        self.doc.hair.scalp_object = Some(id);
        self.doc.hair.scalp_center = center;
        true
    }

    /// Chamado ao entrar no Modo Cabelo: escolhe o alvo. Com penteado existente mantém o alvo dele
    /// (e o torna ativo); senão o objeto ativo vira a cabeça.
    pub(crate) fn hair_enter(&mut self) {
        let existing = self.hair_target().map(|o| o.id);
        let active_id = self.active_object().map(|o| o.id);
        let active_is_hair = active_id.is_some_and(|id| self.doc.hair.groups.iter().any(|g| g.object_id == Some(id)));
        match existing {
            Some(id) if !self.doc.hair.is_empty() || active_is_hair || active_id == Some(id) => {
                if let Some(i) = self.doc.objects.iter().position(|o| o.id == id) {
                    self.doc.active = Some(i);
                }
            }
            _ => {
                if !active_is_hair {
                    self.hair_set_target_from_active();
                }
            }
        }
        self.revision += 1;
    }

    /// Descarta a entrada de redo mais recente (usado quando uma ação é desfeita por falha interna).
    pub(crate) fn drop_redo_tip(&mut self) {
        self.history_drop_redo();
    }

    /// Regenera a malha de um grupo (ou de todos) como objeto da sessão. Mantém o objeto ativo.
    /// Grupos sem mechas não têm objeto. Não grava undo (o chamador já gravou o checkpoint).
    pub fn hair_regenerate(&mut self, only: Option<u32>) -> Result<(), String> {
        let scalp_tf: Transform = self.hair_target().map(|o| o.transform).unwrap_or_default();
        let center = self.doc.hair.scalp_center;
        let active_id = self.active_object().map(|o| o.id);
        let ids: Vec<u32> = self.doc.hair.groups.iter().map(|g| g.id).filter(|g| only.is_none_or(|o| o == *g)).collect();
        for gid in ids {
            let Some(group) = self.doc.hair.group(gid).cloned() else {
                continue;
            };
            let existing = group.object_id.and_then(|oid| self.doc.objects.iter().position(|o| o.id == oid));
            if group.strands.is_empty() {
                if let Some(i) = existing {
                    self.doc.objects.remove(i);
                }
                if let Some(g) = self.doc.hair.group_mut(gid) {
                    g.object_id = None;
                }
                continue;
            }
            let built = build_group(&group, center).map_err(|e| format!("Cabelo: {e}"))?;
            let name = format!("Cabelo - {}", group.name);
            let mut mesh = built.mesh;
            mesh.name = name.clone();
            let material = Some(ObjectMaterial {
                name: group.material.name.clone(),
                base_color: group.material.base_color,
                shade_color: group.material.shade_color,
                highlight_color: group.material.highlight_color,
            });
            match existing {
                Some(i) => {
                    let o = &mut self.doc.objects[i];
                    o.name = name;
                    o.mesh = mesh;
                    o.visible = group.visible;
                    o.transform = scalp_tf;
                    o.material = material;
                }
                None => {
                    let id = self.doc.alloc_id();
                    self.doc.objects.push(ModelObject {
                        id,
                        name,
                        mesh,
                        transform: scalp_tf,
                        visible: group.visible,
                        modifiers: Default::default(),
                        auto_smooth_angle: DEFAULT_AUTO_SMOOTH,
                        material,
                    });
                    if let Some(g) = self.doc.hair.group_mut(gid) {
                        g.object_id = Some(id);
                    }
                }
            }
        }
        // índices podem ter mudado (objetos removidos/adicionados): reencontra o ativo pelo id
        self.doc.active = active_id.and_then(|id| self.doc.objects.iter().position(|o| o.id == id)).or(self.doc.active.filter(|i| *i < self.doc.objects.len()));
        self.revision += 1;
        Ok(())
    }

    /// Remove o grupo `gid` e o objeto dele (com checkpoint).
    pub fn hair_remove_group(&mut self, gid: u32) -> Result<(), String> {
        if self.doc.hair.group(gid).is_none() {
            return Err("Grupo inexistente".into());
        }
        self.checkpoint("Excluir grupo de cabelo");
        let removed = self.doc.hair.remove_group(gid);
        if let Some(oid) = removed.and_then(|g| g.object_id) {
            let active_id = self.active_object().map(|o| o.id);
            self.doc.objects.retain(|o| o.id != oid);
            self.doc.active = active_id.and_then(|id| self.doc.objects.iter().position(|o| o.id == id));
        }
        self.revision += 1;
        Ok(())
    }

    /// Nome dos objetos de cabelo, para o status/inspector.
    pub fn hair_object_count(&self) -> usize {
        self.doc.hair.groups.iter().filter(|g| g.object_id.is_some()).count()
    }

    /// Entra/sai do Modo Cabelo pelo mesmo caminho do combo do cabeçalho.
    pub fn is_hair_mode(&self) -> bool {
        self.mode == Mode::Hair
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::modeling::PrimitiveKind;
    use crate::viewport::OrbitCamera;

    fn session_with_head() -> ModelingSession {
        let mut s = ModelingSession::new();
        s.add_primitive(PrimitiveKind::UvSphere, Vec3::new(0.0, 1.0, 0.0));
        s.clear_history();
        s.set_mode(Mode::Hair);
        s
    }

    fn view() -> ViewInfo {
        let mut camera = OrbitCamera::default();
        camera.target = Vec3::new(0.0, 1.0, 0.0);
        camera.distance = 4.0;
        camera.yaw = 0.0;
        camera.pitch = 0.2;
        ViewInfo {
            camera,
            size: Vec2::new(1000.0, 800.0),
        }
    }

    /// Pontos locais (cabeça = esfera de raio 1) numa linha de meridiano visível.
    fn meridian(lon: f32, lat0: f32, lat1: f32) -> Vec<Vec3> {
        (0..=14)
            .map(|k| {
                let lat = lat0 + (lat1 - lat0) * k as f32 / 14.0;
                Vec3::new(lat.cos() * lon.sin(), lat.sin(), lat.cos() * lon.cos())
            })
            .collect()
    }

    #[test]
    fn entering_hair_mode_targets_the_active_object() {
        let s = session_with_head();
        assert_eq!(s.mode, Mode::Hair);
        assert_eq!(s.hair_target().map(|o| o.id), s.active_object().map(|o| o.id));
        assert!((s.doc.hair.scalp_center - Vec3::ZERO).length() < 0.05, "centro local da esfera");
    }

    #[test]
    fn script_stroke_creates_group_object_with_material_and_undo_redo_restores() {
        let mut s = session_with_head();
        let mut hs = HairState::new();
        let v = view();
        let objects_before = s.doc.objects.len();
        hs.run_script(&mut s, &v, &[ScriptStep::NewGroup("Franja"), ScriptStep::Draw(meridian(0.35, 1.0, 0.1))]);
        assert!(hs.error.is_none(), "{:?}", hs.error);
        assert_eq!(s.doc.hair.groups.len(), 1);
        assert_eq!(s.doc.hair.strand_count(), 1);
        assert_eq!(s.doc.objects.len(), objects_before + 1, "um objeto por grupo");
        let g = &s.doc.hair.groups[0];
        let obj = s.doc.objects.iter().find(|o| Some(o.id) == g.object_id).expect("objeto do grupo");
        assert!(obj.mesh.face_count() > 0 && obj.material.is_some());
        assert_eq!(obj.material.as_ref().unwrap().base_color, g.material.base_color);
        assert_eq!(s.active_object().unwrap().id, s.hair_target().unwrap().id, "o alvo continua ativo");
        let hash_after = s.doc.hair.content_hash();
        // desfazer volta ao estado sem mecha NEM objeto; refazer traz de volta
        s.undo();
        assert_eq!(s.doc.hair.strand_count(), 0);
        assert_eq!(s.doc.objects.len(), objects_before);
        s.redo();
        assert_eq!(s.doc.hair.content_hash(), hash_after);
        assert_eq!(s.doc.objects.len(), objects_before + 1);
        // a raiz está no couro cabeludo (esfera de raio 1)
        let root = s.doc.hair.groups[0].strands[0].root();
        assert!((root.length() - 1.0).abs() < 0.05, "raiz sobre a esfera, |r|={}", root.length());
    }

    #[test]
    fn symmetry_stroke_creates_mirrored_pair_and_one_undo_step() {
        let mut s = session_with_head();
        let mut hs = HairState::new();
        hs.run_script(&mut s, &view(), &[ScriptStep::NewGroup("A"), ScriptStep::Symmetry(true), ScriptStep::Draw(meridian(0.4, 1.0, 0.0))]);
        assert_eq!(s.doc.hair.strand_count(), 2);
        let labels = s.history_labels().iter().filter(|l| **l == "Desenhar mecha").count();
        assert_eq!(labels, 1);
        s.doc.hair.validate().unwrap();
    }

    #[test]
    fn point_drag_checkpoints_once_and_snaps_root_to_scalp() {
        let mut s = session_with_head();
        let mut hs = HairState::new();
        let v = view();
        hs.run_script(&mut s, &v, &[ScriptStep::NewGroup("A"), ScriptStep::Draw(meridian(0.3, 1.0, 0.1)), ScriptStep::SelectLast(None)]);
        let r = hs.selected.unwrap();
        let history_before = s.history_labels().len();
        // arrasta o ponto do meio
        hs.begin_point_drag(&s, r, 1);
        let p = s.doc.hair.strand(r).unwrap().guide.points[1].pos;
        let (px, _) = v.camera.project(s.hair_target().unwrap().model_matrix().transform_point3(p), v.size).unwrap();
        hs.drag_point(&mut s, &v, px + Vec2::new(25.0, 10.0)).unwrap();
        hs.drag_point(&mut s, &v, px + Vec2::new(40.0, 20.0)).unwrap();
        hs.end_point_drag();
        assert_eq!(s.history_labels().len(), history_before + 1, "um checkpoint por arrasto");
        let moved = s.doc.hair.strand(r).unwrap().guide.points[1].pos;
        assert!(moved.distance(p) > 0.01);
        // arrastar a raiz para longe: ela é recolada na esfera
        hs.begin_point_drag(&s, r, 0);
        let root = s.doc.hair.strand(r).unwrap().guide.points[0].pos;
        let (rpx, _) = v.camera.project(s.hair_target().unwrap().model_matrix().transform_point3(root), v.size).unwrap();
        hs.drag_point(&mut s, &v, rpx + Vec2::new(30.0, 0.0)).unwrap();
        hs.end_point_drag();
        let new_root = s.doc.hair.strand(r).unwrap().guide.points[0].pos;
        assert!((new_root.length() - 1.0).abs() < 0.03, "raiz recolada: {}", new_root.length());
        // sem movimento real não há checkpoint
        let n = s.history_labels().len();
        hs.begin_point_drag(&s, r, 2);
        hs.end_point_drag();
        assert_eq!(s.history_labels().len(), n);
    }

    #[test]
    fn delete_duplicate_and_group_removal_keep_objects_consistent() {
        let mut s = session_with_head();
        let mut hs = HairState::new();
        let v = view();
        hs.run_script(&mut s, &v, &[ScriptStep::NewGroup("A"), ScriptStep::Draw(meridian(0.3, 1.0, 0.1)), ScriptStep::NewGroup("B"), ScriptStep::Draw(meridian(-0.4, 1.0, 0.1))]);
        assert_eq!(s.doc.hair.groups.len(), 2);
        let objs = s.doc.objects.len();
        let gb = s.doc.hair.groups[1].id;
        hs.select(Some(StrandRef { group: gb, strand: s.doc.hair.groups[1].strands[0].id }), None);
        hs.duplicate_selected(&mut s).unwrap();
        assert_eq!(s.doc.hair.group(gb).unwrap().strands.len(), 2);
        // remove ponto, depois a mecha
        hs.select(hs.selected, Some(1));
        let before_pts = s.doc.hair.strand(hs.selected.unwrap()).unwrap().guide.points.len();
        hs.delete_selected(&mut s, false).unwrap();
        assert_eq!(s.doc.hair.strand(hs.selected.unwrap()).unwrap().guide.points.len(), before_pts - 1);
        hs.delete_selected(&mut s, true).unwrap();
        assert_eq!(s.doc.hair.group(gb).unwrap().strands.len(), 1);
        // excluir o grupo remove o objeto e preserva o alvo ativo
        s.hair_remove_group(gb).unwrap();
        assert_eq!(s.doc.objects.len(), objs - 1);
        assert_eq!(s.active_object().unwrap().id, s.hair_target().unwrap().id);
        s.undo();
        assert_eq!(s.doc.objects.len(), objs);
        s.doc.hair.validate().unwrap();
    }

    #[test]
    fn comb_pass_reorients_strands_with_one_undo_step_and_a_miss_changes_nothing() {
        let mut s = session_with_head();
        let mut hs = HairState::new();
        let v = view();
        hs.run_script(&mut s, &v, &[ScriptStep::NewGroup("A"), ScriptStep::Draw(meridian(0.1, 1.2, 0.2))]);
        let r = StrandRef { group: s.doc.hair.groups[0].id, strand: s.doc.hair.groups[0].strands[0].id };
        let tip_before = s.doc.hair.strand(r).unwrap().tip();
        let len_before = s.doc.hair.strand(r).unwrap().length();
        let n = s.history_labels().len();
        // passada longe das mechas: nada muda e não suja o histórico
        hs.begin_comb(Vec2::new(30.0, 30.0));
        assert_eq!(hs.comb_move(&mut s, &v, Vec2::new(90.0, 30.0)).unwrap(), 0);
        hs.end_comb();
        assert_eq!(s.history_labels().len(), n);
        // passada sobre a mecha, para a direita
        let target = s.doc.hair.strand(r).unwrap().guide.points[1].pos;
        let (px, _) = v.camera.project(s.hair_target().unwrap().model_matrix().transform_point3(target), v.size).unwrap();
        hs.comb_radius_px = 120.0;
        hs.comb_strength = 0.8;
        hs.begin_comb(px);
        for k in 1..=6 {
            hs.comb_move(&mut s, &v, px + Vec2::new(14.0 * k as f32, 0.0)).unwrap();
        }
        hs.end_comb();
        let tip_after = s.doc.hair.strand(r).unwrap().tip();
        assert!(tip_after.distance(tip_before) > 0.02, "a ponta se moveu");
        assert!(tip_after.x > tip_before.x, "empurrada para a direita da tela (+X)");
        let len_after = s.doc.hair.strand(r).unwrap().length();
        assert!((len_after - len_before).abs() / len_before < 0.15, "comprimento ~preservado");
        assert_eq!(s.history_labels().len(), n + 1, "uma passada = um undo");
        s.undo();
        assert_eq!(s.doc.hair.strand(r).unwrap().tip(), tip_before);
    }

    #[test]
    fn pick_finds_points_then_segments() {
        let mut s = session_with_head();
        let mut hs = HairState::new();
        let v = view();
        hs.run_script(&mut s, &v, &[ScriptStep::NewGroup("A"), ScriptStep::Draw(meridian(0.3, 1.0, 0.1))]);
        let g = &s.doc.hair.groups[0];
        let r = StrandRef { group: g.id, strand: g.strands[0].id };
        let model = s.hair_target().unwrap().model_matrix();
        let p1 = g.strands[0].guide.points[1].pos;
        let (px, _) = v.camera.project(model.transform_point3(p1), v.size).unwrap();
        assert_eq!(hs.pick(&s, &v, px + Vec2::new(2.0, 1.0)), Some(HairPick::Point(r, 1)));
        // ponto médio entre dois pontos de controle: acerta o segmento
        let (a, b) = (g.strands[0].guide.points[1].pos, g.strands[0].guide.points[2].pos);
        let mid = g.strands[0].guide.sample(0.5 * (1.0 + 2.0) / (g.strands[0].guide.points.len() - 1) as f32).pos;
        let _ = (a, b);
        let (mpx, _) = v.camera.project(model.transform_point3(mid), v.size).unwrap();
        match hs.pick(&s, &v, mpx) {
            Some(HairPick::Segment(rr, after, _)) => {
                assert_eq!(rr, r);
                assert!(after >= 1 && after + 1 < g.strands[0].guide.points.len());
            }
            other => panic!("esperava segmento, veio {other:?}"),
        }
        assert_eq!(hs.pick(&s, &v, Vec2::new(5.0, 5.0)), None);
    }

    #[test]
    fn regenerate_follows_target_transform_and_survives_serde() {
        let mut s = session_with_head();
        let mut hs = HairState::new();
        hs.run_script(&mut s, &view(), &[ScriptStep::NewGroup("A"), ScriptStep::Draw(meridian(0.3, 1.0, 0.1))]);
        let tid = s.hair_target().unwrap().id;
        let json = serde_json::to_string(&s.doc).unwrap();
        let back: super::super::Document = serde_json::from_str(&json).unwrap();
        assert_eq!(back.hair.strand_count(), 1);
        assert_eq!(back.hair.scalp_object, Some(tid));
        // mover o alvo e regenerar: o objeto de cabelo acompanha
        let i = s.doc.objects.iter().position(|o| o.id == tid).unwrap();
        s.doc.objects[i].transform.translation += Vec3::new(2.0, 0.0, 0.0);
        s.hair_regenerate(None).unwrap();
        let g = &s.doc.hair.groups[0];
        let hair_obj = s.doc.objects.iter().find(|o| Some(o.id) == g.object_id).unwrap();
        assert_eq!(hair_obj.transform.translation, s.doc.objects[i].transform.translation);
        // documento antigo (sem o campo hair) ainda carrega
        let mut v: serde_json::Value = serde_json::from_str(&json).unwrap();
        v.as_object_mut().unwrap().remove("hair");
        let old: super::super::Document = serde_json::from_value(v).unwrap();
        assert!(old.hair.is_empty());
    }
}
