//! Edição de materiais com desfazer, sem UI: leitura do material efetivo de cada fonte, comandos do
//! núcleo (`SetMaterialParams`, `SetNodeProps`), pré-visualização de arrasto (fora do histórico) e commit
//! como UM passo. A UI (`app::shading_ui::sources`) só delega para cá, o que permite testar o contrato de
//! undo/redo de ponta a ponta.

use crate::modeling::{ModelingSession, ObjectMaterial, SnapshotPub};
use anigo_core::command::Command;
use anigo_core::ids::{MaterialId, NodeId};
use anigo_core::level::{NodeOverrides, NodePatch};
use anigo_core::{MaterialPatch, NodeKind, NodeSlot, ProjectState, StylizedMaterial};
use anigo_session::CoreSession;

/// O que está sendo editado.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Target {
    /// Material do projeto (o do personagem): `SetMaterialParams`.
    Project(MaterialId),
    /// Instância de um nó do Cenário: `SetNodeProps { overrides.material }`.
    Node(NodeId),
    /// Objeto da Modelagem (histórico próprio da `ModelingSession`).
    Object(u32),
}

/// Gesto de edição em andamento (arrasto de slider, seletor de cor…): o último material pré-visualizado.
pub struct Gesture {
    pub target: Target,
    pub start: StylizedMaterial,
    pub last: StylizedMaterial,
    /// Estado da Modelagem antes do gesto (desfazer de objetos).
    pub model_before: Option<SnapshotPub>,
}

fn no_effect(e: &str) -> bool {
    e.contains("no effect") || e.contains("already")
}

/// Material que a Modelagem mostra no preview Cel para um objeto sem material completo.
pub fn modeling_default(o: Option<&ObjectMaterial>) -> StylizedMaterial {
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

/// Material de um nó do Cenário: o do personagem para a raiz do personagem; para os demais, o padrão
/// de cenário (cinza-azulado) com as substituições do nó (campos básicos, depois o patch completo).
pub fn node_material(project: &ProjectState, slot: &NodeSlot) -> StylizedMaterial {
    if slot.kind == NodeKind::CharacterRoot {
        return project.character_material().map(|m| m.material.clone()).unwrap_or_default();
    }
    let mut m = StylizedMaterial {
        base_color: [0.70, 0.73, 0.82, 1.0],
        shade_color: [0.36, 0.38, 0.56, 1.0],
        outline_color: [0.12, 0.10, 0.16, 1.0],
        ..Default::default()
    };
    slot.overrides.apply_to_material(&mut m);
    m
}

/// Material base de um nó (sem NENHUMA substituição).
pub fn node_base(project: &ProjectState, id: &NodeId) -> Option<StylizedMaterial> {
    let slot = project.scene.node(id)?;
    let mut clean = slot.clone();
    clean.overrides = NodeOverrides::default();
    Some(node_material(project, &clean))
}

/// O material EFETIVO do alvo (o que o renderer desenha).
pub fn read(core: &CoreSession, modeling: &ModelingSession, t: &Target) -> Option<StylizedMaterial> {
    match t {
        Target::Project(id) => core.project().materials.get(id).map(|e| e.material.clone()),
        Target::Node(id) => {
            let slot = core.project().scene.node(id)?;
            Some(node_material(core.project(), slot))
        }
        Target::Object(id) => {
            let o = modeling.doc.objects.iter().find(|o| o.id == *id)?;
            Some(modeling.object_stylized(*id).cloned().unwrap_or_else(|| modeling_default(o.material.as_ref())))
        }
    }
}

/// Comando que leva o projeto do estado ATUAL a `to` (`None` = nada a fazer, ou alvo da Modelagem).
pub fn command_to(core: &CoreSession, t: &Target, to: &StylizedMaterial) -> Option<Command> {
    let project = core.project();
    match t {
        Target::Project(id) => {
            let cur = project.materials.get(id)?.material.clone();
            let patch = MaterialPatch::diff(&cur, to);
            (!patch.is_empty()).then(|| Command::SetMaterialParams { material_id: Some(id.clone()), patch })
        }
        Target::Node(id) => {
            let base = node_base(project, id)?;
            let patch = MaterialPatch::diff(&base, to);
            let ov = NodeOverrides { material: (!patch.is_empty()).then_some(patch), ..Default::default() };
            let cur = &project.scene.node(id)?.overrides;
            (*cur != ov).then(|| Command::SetNodeProps { node_id: id.clone(), patch: NodePatch { overrides: Some(ov), ..Default::default() } })
        }
        Target::Object(_) => None,
    }
}

/// Passo de um gesto contínuo: aplica ao vivo (sem histórico). Devolve uma mensagem de erro real (nunca "sem efeito").
pub fn change(core: &mut CoreSession, modeling: &mut ModelingSession, gesture: &mut Option<Gesture>, t: &Target, new: StylizedMaterial) -> Result<(), String> {
    if gesture.as_ref().is_some_and(|g| g.target != *t) {
        commit(core, modeling, gesture)?;
    }
    if gesture.is_none() {
        let start = read(core, modeling, t).ok_or("alvo inexistente")?;
        let model_before = matches!(t, Target::Object(_)).then(|| modeling.capture());
        *gesture = Some(Gesture { target: t.clone(), start, last: new.clone(), model_before });
    }
    let mut result = Ok(());
    match t {
        Target::Object(id) => modeling.set_object_stylized_live(*id, Some(new.clone())),
        _ => {
            if let Some(cmd) = command_to(core, t, &new) {
                if let Err(e) = core.preview(cmd) {
                    if !no_effect(&e) {
                        result = Err(e);
                    }
                }
            }
        }
    }
    if let Some(g) = gesture.as_mut() {
        g.last = new;
    }
    result
}

/// Grava o gesto em andamento (se houver) como um único passo. Devolve a descrição do comando aplicado.
pub fn commit(core: &mut CoreSession, modeling: &mut ModelingSession, gesture: &mut Option<Gesture>) -> Result<Option<String>, String> {
    let Some(g) = gesture.take() else { return Ok(None) };
    match &g.target {
        Target::Object(_) => {
            if g.last == g.start {
                return Ok(None);
            }
            if let Some(before) = g.model_before {
                modeling.commit_snapshot("Material", before);
            }
            Ok(Some("Material".to_string()))
        }
        t => {
            // volta ao estado de partida e aplica o comando final como um passo só
            core.cancel_preview();
            match command_to(core, t, &g.last) {
                Some(cmd) => match core.apply(cmd) {
                    Ok(o) => Ok(Some(o.description)),
                    Err(e) if no_effect(&e) => Ok(None),
                    Err(e) => Err(e),
                },
                None => Ok(None),
            }
        }
    }
}

/// Edição discreta (preset, colar, reset de grupo, trocar A/B): um passo de histórico agora.
pub fn apply(core: &mut CoreSession, modeling: &mut ModelingSession, gesture: &mut Option<Gesture>, t: &Target, new: StylizedMaterial) -> Result<Option<String>, String> {
    commit(core, modeling, gesture)?;
    match t {
        Target::Object(id) => {
            if read(core, modeling, t).as_ref() == Some(&new) {
                return Ok(None);
            }
            modeling.set_object_stylized("Material", *id, Some(new));
            Ok(Some("Material".to_string()))
        }
        _ => match command_to(core, t, &new) {
            Some(cmd) => match core.apply(cmd) {
                Ok(o) => Ok(Some(o.description)),
                Err(e) if no_effect(&e) => Ok(None),
                Err(e) => Err(e),
            },
            None => Ok(None),
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::modeling::PrimitiveKind;
    use anigo_core::level::{plan_place_primitive, PrimitiveShape};
    use anigo_core::Transform;
    use glam::Vec3;
    use std::collections::BTreeSet;

    fn sessions() -> (CoreSession, ModelingSession) {
        (CoreSession::new(), ModelingSession::with_default_cube())
    }

    fn character(core: &CoreSession) -> Target {
        Target::Project(core.project().character_material().unwrap().material_id.clone())
    }

    fn place_node(core: &mut CoreSession) -> Target {
        let (cmd, id) = plan_place_primitive(core.project(), PrimitiveShape::Cube, "Caixa", Transform { translation: Vec3::new(1.0, 0.5, 0.0), ..Default::default() }, &BTreeSet::new());
        core.apply(cmd).unwrap();
        Target::Node(id)
    }

    fn depth(core: &CoreSession) -> usize {
        core.history().undo_depth()
    }

    /// Simula um arrasto: muitos passos ao vivo e um commit.
    fn drag(core: &mut CoreSession, m: &mut ModelingSession, g: &mut Option<Gesture>, t: &Target, f: impl Fn(&mut StylizedMaterial, f32)) {
        let start = read(core, m, t).unwrap();
        for i in 1..=20 {
            let mut nm = start.clone();
            f(&mut nm, i as f32 / 20.0);
            change(core, m, g, t, nm).unwrap();
        }
    }

    #[test]
    fn dragging_a_project_material_is_one_undo_step_and_previews_do_not_touch_history() {
        let (mut core, mut m) = sessions();
        let t = character(&core);
        let mut g = None;
        let before = read(&core, &m, &t).unwrap();
        let d0 = depth(&core);
        drag(&mut core, &mut m, &mut g, &t, |mat, k| mat.rim_intensity = 0.8 + 2.0 * k);
        // ao vivo: o projeto já mostra o valor, o histórico não mudou
        assert!((read(&core, &m, &t).unwrap().rim_intensity - 2.8).abs() < 1e-5);
        assert_eq!(depth(&core), d0);
        assert!(core.is_previewing());
        let desc = commit(&mut core, &mut m, &mut g).unwrap();
        assert_eq!(desc.as_deref(), Some("Material"));
        assert_eq!(depth(&core), d0 + 1, "20 passos de arrasto = 1 passo de desfazer");
        assert!(!core.is_previewing());
        assert!((read(&core, &m, &t).unwrap().rim_intensity - 2.8).abs() < 1e-5);
        core.undo().unwrap();
        assert_eq!(read(&core, &m, &t).unwrap(), before);
        core.redo().unwrap();
        assert!((read(&core, &m, &t).unwrap().rim_intensity - 2.8).abs() < 1e-5);
    }

    #[test]
    fn dragging_back_to_the_start_value_leaves_no_history_and_no_residue() {
        let (mut core, mut m) = sessions();
        let t = character(&core);
        let mut g = None;
        let before = read(&core, &m, &t).unwrap();
        let d0 = depth(&core);
        let mut up = before.clone();
        up.outline_width = 0.01;
        change(&mut core, &mut m, &mut g, &t, up).unwrap();
        change(&mut core, &mut m, &mut g, &t, before.clone()).unwrap(); // voltou: o diff é vazio, mas o ao vivo precisa voltar
        assert_eq!(read(&core, &m, &t).unwrap().outline_width, before.outline_width, "o ao vivo volta ao valor inicial");
        assert_eq!(commit(&mut core, &mut m, &mut g).unwrap(), None);
        assert_eq!(depth(&core), d0);
        assert_eq!(read(&core, &m, &t).unwrap(), before);
    }

    #[test]
    fn discrete_edits_push_one_step_each_and_apply_the_whole_material() {
        let (mut core, mut m) = sessions();
        let t = character(&core);
        let mut g = None;
        let d0 = depth(&core);
        let mut preset = (crate::shading::presets::BUILT_IN[6].build)();
        preset.name = read(&core, &m, &t).unwrap().name;
        assert!(apply(&mut core, &mut m, &mut g, &t, preset.clone()).unwrap().is_some());
        assert_eq!(depth(&core), d0 + 1);
        assert_eq!(read(&core, &m, &t).unwrap(), preset, "o preset inteiro (npr, rampa) foi aplicado");
        // aplicar o mesmo de novo não polui o histórico
        assert_eq!(apply(&mut core, &mut m, &mut g, &t, preset.clone()).unwrap(), None);
        assert_eq!(depth(&core), d0 + 1);
        core.undo().unwrap();
        assert_eq!(read(&core, &m, &t).unwrap(), StylizedMaterial::default().tap_name(&preset.name));
    }

    trait TapName {
        fn tap_name(self, n: &str) -> Self;
    }
    impl TapName for StylizedMaterial {
        fn tap_name(mut self, n: &str) -> Self {
            self.name = n.to_string();
            self
        }
    }

    #[test]
    fn scene_node_materials_are_full_overrides_with_exact_undo() {
        let (mut core, mut m) = sessions();
        let t = place_node(&mut core);
        let Target::Node(id) = &t else { unreachable!() };
        let mut g = None;
        let base = node_base(core.project(), id).unwrap();
        assert_eq!(read(&core, &m, &t).unwrap(), base);
        let d0 = depth(&core);
        drag(&mut core, &mut m, &mut g, &t, |mat, k| {
            mat.npr.ramp_blend = 1.0;
            mat.npr.ramp_custom.set_stops(2, true, &[(0.0, 0.0), (0.3 + 0.4 * k, 1.0)]);
            mat.spec_power = 90.0;
        });
        commit(&mut core, &mut m, &mut g).unwrap();
        assert_eq!(depth(&core), d0 + 1);
        let now = read(&core, &m, &t).unwrap();
        assert_eq!(now.npr.ramp_blend, 1.0);
        assert_eq!(now.spec_power, 90.0);
        assert!(now.npr.ramp_custom.is_custom(2));
        // o patch fica no JSON do projeto (aditivo) e nada além do que mudou
        let slot = core.project().scene.node(id).unwrap();
        let patch = slot.overrides.material.as_ref().unwrap();
        assert!(patch.spec_power.is_some() && patch.npr.is_some() && patch.base_color.is_none());
        let doc = core.document().unwrap();
        assert!(doc.contains("ramp_custom"));
        core.undo().unwrap();
        assert_eq!(read(&core, &m, &t).unwrap(), base);
        assert!(core.project().scene.node(id).unwrap().overrides.is_default());
        core.redo().unwrap();
        assert_eq!(read(&core, &m, &t).unwrap(), now);
        // salvar e reabrir preserva
        let json = core.document().unwrap();
        let mut other = CoreSession::new();
        other.load_document(&json).unwrap();
        assert_eq!(read(&other, &m, &t).unwrap(), now);
    }

    #[test]
    fn a_project_without_shading_edits_serializes_exactly_as_before() {
        let (core, _) = sessions();
        let doc = core.document().unwrap();
        assert!(!doc.contains("ramp_custom") && !doc.contains("\"material\":{") || doc.contains("\"materials\""));
        assert!(!doc.contains("overrides"));
    }

    #[test]
    fn modeling_objects_use_the_session_history_not_the_core() {
        let (mut core, mut m) = sessions();
        let id = m.doc.objects[0].id;
        let t = Target::Object(id);
        let mut g = None;
        let d_core = depth(&core);
        let before = read(&core, &m, &t).unwrap();
        assert_eq!(before, StylizedMaterial::default(), "objeto sem material usa o padrão do preview");
        assert!(!m.can_undo());
        drag(&mut core, &mut m, &mut g, &t, |mat, k| {
            mat.outline_width = 0.004 + 0.01 * k;
            mat.base_color = [0.2, 0.5, 0.9, 1.0];
        });
        assert!(!m.can_undo(), "ao vivo não registra undo");
        assert_eq!(commit(&mut core, &mut m, &mut g).unwrap().as_deref(), Some("Material"));
        assert!(m.can_undo());
        assert_eq!(depth(&core), d_core, "o núcleo não foi tocado");
        assert_eq!(read(&core, &m, &t).unwrap().base_color, [0.2, 0.5, 0.9, 1.0]);
        m.undo();
        assert_eq!(read(&core, &m, &t).unwrap(), before);
        m.redo();
        assert_eq!(read(&core, &m, &t).unwrap().base_color, [0.2, 0.5, 0.9, 1.0]);
        // edição discreta + undo
        let mut preset = (crate::shading::presets::BUILT_IN[1].build)();
        preset.name = "x".into();
        apply(&mut core, &mut m, &mut g, &t, preset.clone()).unwrap();
        assert_eq!(read(&core, &m, &t).unwrap(), preset);
        m.undo();
        assert_eq!(read(&core, &m, &t).unwrap().base_color, [0.2, 0.5, 0.9, 1.0]);
        // novo objeto parte do padrão
        m.add_primitive(PrimitiveKind::Torus, Vec3::ZERO);
        let t2 = Target::Object(m.doc.objects.last().unwrap().id);
        assert_eq!(read(&core, &m, &t2).unwrap(), StylizedMaterial::default());
    }

    #[test]
    fn changing_target_mid_gesture_commits_the_previous_one() {
        let (mut core, mut m) = sessions();
        let t1 = character(&core);
        let t2 = place_node(&mut core);
        let mut g = None;
        let d0 = depth(&core);
        let mut a = read(&core, &m, &t1).unwrap();
        a.rim_intensity = 2.0;
        change(&mut core, &mut m, &mut g, &t1, a).unwrap();
        let mut b = read(&core, &m, &t2).unwrap();
        b.rim_intensity = 1.5;
        change(&mut core, &mut m, &mut g, &t2, b).unwrap(); // troca de alvo: o gesto anterior vira passo
        assert_eq!(depth(&core), d0 + 1);
        commit(&mut core, &mut m, &mut g).unwrap();
        assert_eq!(depth(&core), d0 + 2);
        assert_eq!(read(&core, &m, &t1).unwrap().rim_intensity, 2.0);
        assert_eq!(read(&core, &m, &t2).unwrap().rim_intensity, 1.5);
    }

    #[test]
    fn non_finite_values_never_reach_the_project() {
        let (mut core, mut m) = sessions();
        let t = character(&core);
        let mut g = None;
        let mut bad = read(&core, &m, &t).unwrap();
        bad.spec_power = f32::NAN;
        let r = change(&mut core, &mut m, &mut g, &t, bad.clone());
        assert!(r.is_err(), "valor não finito é recusado pelo núcleo");
        assert!(read(&core, &m, &t).unwrap().spec_power.is_finite());
        assert!(apply(&mut core, &mut m, &mut g, &t, bad).is_err());
    }

    #[test]
    fn character_root_node_reads_the_project_material_and_stale_targets_read_none() {
        let (core, m) = sessions();
        let root = core.project().scene.nodes.iter().find(|n| n.kind == NodeKind::CharacterRoot).unwrap().clone();
        assert_eq!(node_material(core.project(), &root), core.project().character_material().unwrap().material);
        assert!(read(&core, &m, &Target::Node(NodeId::from_slug("nao_existe"))).is_none());
        assert!(read(&core, &m, &Target::Object(99999)).is_none());
    }
}
