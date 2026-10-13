//! Fontes de material do Shading: delegam a `crate::shading::edit` (testado sem UI) e cuidam da lista de
//! alvos, da seleção e das mensagens de status.
//!
//! * `Target::Project(id)` — `ProjectState::materials[id]` (o material do personagem): `SetMaterialParams`;
//! * `Target::Node(id)` — instância de um nó do Cenário: `SetNodeProps { overrides.material }` (um patch
//!   completo sobre o material base do nó, aditivo e omitido do JSON enquanto vazio);
//! * `Target::Object(id)` — objeto da Modelagem: `Document::stylized` com o histórico da `ModelingSession`.
//!
//! Edição contínua (arrasto): `shading_change` pré-visualiza ao vivo SEM tocar o histórico e
//! `shading_finish_gesture` (ponteiro solto) grava UM passo. Edição discreta: `shading_apply_now`.

use super::{describe_shading, Source, StudioApp, Target};
use crate::icons;
use crate::shading::edit;
use anigo_core::hierarchy::NodeKind;
use anigo_core::{NodeSource, StylizedMaterial};

/// Linha da lista de alvos.
pub(crate) struct TargetRow {
    pub target: Target,
    pub label: String,
    pub icon: &'static str,
    pub swatch: ([f32; 4], [f32; 4]),
    pub depth: usize,
}

impl StudioApp {
    /// O material EFETIVO do alvo (o que o renderer desenha).
    pub(crate) fn shading_read(&self, t: &Target) -> Option<StylizedMaterial> {
        edit::read(&self.core, &self.modeling, t)
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

    fn shading_report(&mut self, r: Result<Option<String>, String>) {
        match r {
            Ok(Some(desc)) => self.shading_ui.message = describe_shading(&desc),
            Ok(None) => {}
            Err(e) => self.shading_ui.message = format!("{}: {e}", crate::i18n::tr("char.error")),
        }
    }

    /// Passo de um gesto contínuo: aplica ao vivo (sem histórico).
    pub(crate) fn shading_change(&mut self, new: StylizedMaterial) {
        let Some(t) = self.shading_ui.target.clone() else { return };
        if let Err(e) = edit::change(&mut self.core, &mut self.modeling, &mut self.shading_ui.gesture, &t, new) {
            self.shading_ui.message = format!("{}: {e}", crate::i18n::tr("char.error"));
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
        let r = edit::commit(&mut self.core, &mut self.modeling, &mut self.shading_ui.gesture);
        self.shading_report(r);
    }

    /// Edição discreta (preset, colar, reset de grupo, trocar A/B): um passo de histórico agora.
    pub(crate) fn shading_apply_now(&mut self, new: StylizedMaterial) {
        let Some(t) = self.shading_ui.target.clone() else { return };
        let r = edit::apply(&mut self.core, &mut self.modeling, &mut self.shading_ui.gesture, &t, new);
        self.shading_report(r);
    }

    // ------------------------------------------------------------------ copiar / colar

    pub(crate) fn shading_copy(&mut self) {
        if let Some(m) = self.shading_ui.target.as_ref().and_then(|t| self.shading_read(t)) {
            self.shading_ui.clipboard = Some(m);
            self.shading_ui.message = crate::i18n::tr("shading.copied").to_string();
        }
    }

    pub(crate) fn shading_paste(&mut self) {
        let Some(mut m) = self.shading_ui.clipboard.clone() else { return };
        // o nome pertence ao alvo; todo o resto vem do que foi copiado
        if let Some(cur) = self.shading_ui.target.as_ref().and_then(|t| self.shading_read(t)) {
            m.name = cur.name;
        }
        self.shading_apply_now(m);
        self.shading_ui.message = crate::i18n::tr("shading.pasted").to_string();
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

    /// Material do projeto do personagem (para cenários e atalhos).
    pub(crate) fn shading_character_target(&self) -> Option<Target> {
        self.core.project().character_material().map(|e| Target::Project(e.material_id.clone()))
    }
}
