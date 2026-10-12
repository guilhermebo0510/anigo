//! Fontes de material do Shading e escrita por comandos com desfazer.
//!
//! * `Target::Project(id)` — `ProjectState::materials[id]` (o material do personagem): `SetMaterialParams`;
//! * `Target::Node(id)` — instância de um nó do Cenário: `SetNodeProps { overrides.material }` (um patch
//!   completo sobre o material base do nó, aditivo e omitido do JSON enquanto vazio);
//! * `Target::Object(id)` — objeto da Modelagem: `Document::stylized` com o histórico da `ModelingSession`.
//!
//! Edição contínua (arrasto): `shading_change` pré-visualiza ao vivo SEM tocar o histórico e
//! `shading_finish_gesture` (ponteiro solto) grava UM passo. Edição discreta: `shading_apply_now`.

use super::{describe_shading, Gesture, Source, StudioApp, Target};
use crate::icons;
use crate::modeling::ObjectMaterial;
use anigo_core::command::Command;
use anigo_core::hierarchy::NodeKind;
use anigo_core::ids::{MaterialId, NodeId};
use anigo_core::level::{NodeOverrides, NodePatch};
use anigo_core::{MaterialPatch, NodeSource, StylizedMaterial};

/// Linha da lista de alvos.
pub(crate) struct TargetRow {
    pub target: Target,
    pub label: String,
    pub icon: &'static str,
    pub swatch: ([f32; 4], [f32; 4]),
    pub depth: usize,
}

fn no_effect(e: &str) -> bool {
    e.contains("no effect") || e.contains("already")
}

/// Material que a Modelagem mostra no preview Cel para um objeto sem material completo.
fn modeling_default(o: Option<&ObjectMaterial>) -> StylizedMaterial {
    match o {
        Some(m) => StylizedMaterial {
            name: m.name.clone(),
            base_color: [m.base_color[0], m.base_color[1], m.base_color[2], 1.0],
            shade_color: [m.shade_color[0], m.shade_color[1], m.shade_color[2], 1.0],
            specular_color: [m.highlight_color[0], m.highlight_color[1], m.highlight_color[2], 1.0],
            spec_intensity: 0.9,
            rim_intensity: 0.45,
            ..Default::default()
        },
        None => StylizedMaterial::default(),
    }
}

impl StudioApp {
    /// Material base de um nó do Cenário (sem as substituições do Shading).
    fn shading_node_base(&self, id: &NodeId) -> Option<StylizedMaterial> {
        let slot = self.core.project().scene.node(id)?;
        let mut clean = slot.clone();
        clean.overrides = NodeOverrides::default();
        Some(super::super::scene_ui::node_material(self.core.project(), &clean))
    }

    /// O material EFETIVO do alvo (o que o renderer desenha).
    pub(crate) fn shading_read(&self, t: &Target) -> Option<StylizedMaterial> {
        match t {
            Target::Project(id) => self.core.project().materials.get(id).map(|e| e.material.clone()),
            Target::Node(id) => {
                let slot = self.core.project().scene.node(id)?;
                Some(super::super::scene_ui::node_material(self.core.project(), slot))
            }
            Target::Object(id) => {
                let o = self.modeling.doc.objects.iter().find(|o| o.id == *id)?;
                Some(self.modeling.object_stylized(*id).cloned().unwrap_or_else(|| modeling_default(o.material.as_ref())))
            }
        }
    }

    pub(crate) fn shading_name(&self, t: &Target) -> String {
        match t {
            Target::Project(id) => self.core.project().materials.get(id).map(|e| e.name.clone()).unwrap_or_else(|| id.to_string()),
            Target::Node(id) => self.core.project().scene.node(id).map(|n| n.name.clone()).unwrap_or_else(|| id.to_string()),
            Target::Object(id) => self.modeling.doc.objects.iter().find(|o| o.id == *id).map(|o| o.name.clone()).unwrap_or_default(),
        }
    }

    /// Alvos de uma aba.
    pub(crate) fn shading_targets(&self, src: Source) -> Vec<TargetRow> {
        let colors = |m: &StylizedMaterial| (m.base_color, m.shade_color);
        match src {
            Source::Personagem => self
                .core
                .project()
                .materials
                .iter()
                .map(|(id, e)| TargetRow { target: Target::Project(id.clone()), label: e.name.clone(), icon: icons::USER, swatch: colors(&e.material), depth: 0 })
                .collect(),
            Source::Cenario => {
                let p = self.core.project();
                let order = p.scene.hierarchy_order().unwrap_or_default();
                let mut out = Vec::new();
                for i in order {
                    let n = &p.scene.nodes[i];
                    if matches!(n.kind, NodeKind::Group | NodeKind::Camera | NodeKind::Light | NodeKind::HumanoidBone) {
                        continue;
                    }
                    if matches!(n.source, Some(NodeSource::Camera { .. }) | Some(NodeSource::Light { .. }) | Some(NodeSource::Card { .. })) {
                        continue;
                    }
                    let (target, icon) = if n.kind == NodeKind::CharacterRoot {
                        match p.character_material() {
                            Some(e) => (Target::Project(e.material_id.clone()), icons::USER),
                            None => continue,
                        }
                    } else {
                        let icon = match &n.source {
                            Some(NodeSource::LibraryAsset { .. }) => icons::PACKAGE,
                            Some(NodeSource::Primitive { .. }) => icons::CUBE,
                            _ => icons::CUBE_FOCUS,
                        };
                        (Target::Node(n.node_id.clone()), icon)
                    };
                    let Some(mat) = self.shading_read(&target) else { continue };
                    out.push(TargetRow { target, label: n.name.clone(), icon, swatch: colors(&mat), depth: p.scene.depth_of(&n.node_id).unwrap_or(0) });
                }
                out
            }
            Source::Modelagem => self
                .modeling
                .doc
                .objects
                .iter()
                .filter_map(|o| {
                    let t = Target::Object(o.id);
                    let m = self.shading_read(&t)?;
                    Some(TargetRow { target: t, label: o.name.clone(), icon: icons::CUBE, swatch: colors(&m), depth: 0 })
                })
                .collect(),
        }
    }

    /// Garante um alvo existente (o primeiro da aba se o atual sumiu).
    pub(crate) fn shading_validate_target(&mut self) {
        let valid = self.shading_ui.target.as_ref().is_some_and(|t| self.shading_read(t).is_some());
        if valid {
            return;
        }
        let first = self.shading_targets(self.shading_ui.source).into_iter().next().map(|r| r.target);
        self.shading_ui.target = first;
        self.shading_ui.snapshot_a = None;
        self.shading_ui.framed = false;
    }

    pub(crate) fn shading_select(&mut self, t: Target) {
        self.shading_commit_now();
        if self.shading_ui.target.as_ref() != Some(&t) {
            self.shading_ui.target = Some(t);
            self.shading_ui.snapshot_a = None;
            self.shading_ui.framed = false;
        }
    }

    pub(crate) fn shading_set_source(&mut self, src: Source) {
        self.shading_commit_now();
        if self.shading_ui.source != src {
            self.shading_ui.source = src;
            self.shading_ui.target = None;
            self.shading_validate_target();
        }
    }

    /// Comando que leva o projeto do estado ATUAL a `to` para o alvo (`None` = nada a fazer / alvo de Modelagem).
    fn shading_command(&self, t: &Target, to: &StylizedMaterial) -> Option<Command> {
        match t {
            Target::Project(id) => {
                let cur = self.core.project().materials.get(id)?.material.clone();
                let patch = MaterialPatch::diff(&cur, to);
                (!patch.is_empty()).then(|| Command::SetMaterialParams { material_id: Some(id.clone()), patch })
            }
            Target::Node(id) => {
                let base = self.shading_node_base(id)?;
                let patch = MaterialPatch::diff(&base, to);
                let ov = NodeOverrides { material: (!patch.is_empty()).then_some(patch), ..Default::default() };
                let cur = &self.core.project().scene.node(id)?.overrides;
                (*cur != ov).then(|| Command::SetNodeProps { node_id: id.clone(), patch: NodePatch { overrides: Some(ov), ..Default::default() } })
            }
            Target::Object(_) => None,
        }
    }

    fn shading_say(&mut self, msg: String) {
        self.shading_ui.message = msg;
    }

    /// Passo de um gesto contínuo: aplica ao vivo (sem histórico).
    pub(crate) fn shading_change(&mut self, new: StylizedMaterial) {
        let Some(t) = self.shading_ui.target.clone() else { return };
        if self.shading_ui.gesture.as_ref().is_some_and(|g| g.target != t) {
            self.shading_commit_now();
        }
        if self.shading_ui.gesture.is_none() {
            let Some(start) = self.shading_read(&t) else { return };
            let model_before = matches!(t, Target::Object(_)).then(|| self.modeling.capture());
            self.shading_ui.gesture = Some(Gesture { target: t.clone(), start, last: new.clone(), model_before });
        }
        match &t {
            Target::Object(id) => self.modeling.set_object_stylized_live(*id, Some(new.clone())),
            _ => {
                if let Some(cmd) = self.shading_command(&t, &new) {
                    if let Err(e) = self.core.preview(cmd) {
                        if !no_effect(&e) {
                            self.shading_say(format!("{}: {e}", crate::i18n::tr("char.error")));
                        }
                    }
                }
            }
        }
        if let Some(g) = self.shading_ui.gesture.as_mut() {
            g.last = new;
        }
    }

    /// Ao soltar o ponteiro, o gesto vira UM passo de histórico.
    pub(crate) fn shading_finish_gesture(&mut self, ctx: &egui::Context) {
        if self.shading_ui.gesture.is_none() || ctx.input(|i| i.pointer.any_down()) {
            return;
        }
        self.shading_commit_now();
    }

    /// Grava o gesto em andamento (se houver) como um único passo.
    pub(crate) fn shading_commit_now(&mut self) {
        let Some(g) = self.shading_ui.gesture.take() else { return };
        match &g.target {
            Target::Object(id) => {
                if g.last == g.start {
                    return;
                }
                if let Some(before) = g.model_before {
                    self.modeling.commit_snapshot(crate::i18n::tr("shading.cmd.material"), before);
                }
                self.shading_say(crate::i18n::tr("shading.cmd.material").to_string());
                let _ = id;
            }
            t => {
                // volta ao estado de partida e aplica o comando final como um passo só
                self.core.cancel_preview();
                if let Some(cmd) = self.shading_command(t, &g.last) {
                    match self.core.apply(cmd) {
                        Ok(o) => self.shading_say(describe_shading(&o.description)),
                        Err(e) if no_effect(&e) => {}
                        Err(e) => self.shading_say(format!("{}: {e}", crate::i18n::tr("char.error"))),
                    }
                }
            }
        }
    }

    /// Edição discreta (preset, colar, reset de grupo, trocar A/B): um passo de histórico agora.
    pub(crate) fn shading_apply_now(&mut self, new: StylizedMaterial) {
        self.shading_commit_now();
        let Some(t) = self.shading_ui.target.clone() else { return };
        match &t {
            Target::Object(id) => {
                let cur = self.shading_read(&t);
                if cur.as_ref() != Some(&new) {
                    self.modeling.set_object_stylized(crate::i18n::tr("shading.cmd.material"), *id, Some(new));
                    self.shading_say(crate::i18n::tr("shading.cmd.material").to_string());
                }
            }
            _ => {
                if let Some(cmd) = self.shading_command(&t, &new) {
                    match self.core.apply(cmd) {
                        Ok(o) => self.shading_say(describe_shading(&o.description)),
                        Err(e) if no_effect(&e) => {}
                        Err(e) => self.shading_say(format!("{}: {e}", crate::i18n::tr("char.error"))),
                    }
                }
            }
        }
    }

    // ------------------------------------------------------------------ copiar / colar

    pub(crate) fn shading_copy(&mut self) {
        if let Some(m) = self.shading_ui.target.as_ref().and_then(|t| self.shading_read(t)) {
            self.shading_ui.clipboard = Some(m);
            self.shading_say(crate::i18n::tr("shading.copied").to_string());
        }
    }

    pub(crate) fn shading_paste(&mut self) {
        let Some(mut m) = self.shading_ui.clipboard.clone() else { return };
        // o nome pertence ao alvo; todo o resto vem do que foi copiado
        if let Some(cur) = self.shading_ui.target.as_ref().and_then(|t| self.shading_read(t)) {
            m.name = cur.name;
        }
        self.shading_apply_now(m);
        self.shading_say(crate::i18n::tr("shading.pasted").to_string());
    }

    // ------------------------------------------------------------------ desfazer / refazer

    /// Desfazer/refazer roteado pela aba: Modelagem fala com a `ModelingSession`, o resto com o núcleo.
    pub(crate) fn shading_undo_redo(&mut self, undo: bool) -> Option<String> {
        self.shading_commit_now();
        self.core.cancel_preview();
        if self.shading_ui.source == Source::Modelagem {
            if undo {
                self.modeling.undo();
            } else {
                self.modeling.redo();
            }
            return None;
        }
        let r = if undo { self.core.undo() } else { self.core.redo() };
        Some(match r {
            Ok(o) => describe_shading(&o.description),
            Err(e) => e,
        })
    }

    pub(crate) fn shading_can_undo_redo(&self) -> (bool, bool) {
        if self.shading_ui.source == Source::Modelagem {
            (self.modeling.can_undo(), self.modeling.can_redo())
        } else {
            (self.core.history().can_undo(), self.core.history().can_redo())
        }
    }

    /// Id do material do projeto do personagem (para cenários e atalhos).
    pub(crate) fn shading_character_target(&self) -> Option<Target> {
        self.core.project().character_material().map(|e| Target::Project(e.material_id.clone()))
    }

    #[allow(dead_code)]
    pub(crate) fn shading_material_id(&self) -> Option<MaterialId> {
        self.core.project().character_material().map(|e| e.material_id.clone())
    }
}
