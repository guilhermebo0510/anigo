//! Fonte de quadros da Animação para a fila de Render: cada quadro do intervalo vira uma `Scene` com o clip avaliado
//! (pose do personagem, dedos, transform dos nós, sol e câmera de cena). Liga o gancho `FrameSource`/`FrameRange` que o
//! workspace Render deixou pronto.

use crate::render_out::job::FrameSource;
use anigo_core::animation::{CameraProp, ChannelTarget, SunProp};
use anigo_core::pose::{finger_curl_mesh, PoseRig};
use anigo_core::project::{AnimationClip, ProjectState};
use anigo_core::{Mesh, Scene};

pub struct ClipFrameSource {
    /// Cena do projeto como o Render a monta (geometria, materiais, câmera da saída).
    pub base: Scene,
    pub project: ProjectState,
    pub rig: PoseRig,
    pub clip: AnimationClip,
    pub fps: f32,
    /// Id do nó do personagem na cena.
    pub character_node: String,
    /// Malha do personagem sem curvatura de dedos (a base da deformação por quadro).
    pub rest_mesh: Option<Mesh>,
}

impl ClipFrameSource {
    pub fn new(base: Scene, project: ProjectState, rig: PoseRig, clip: AnimationClip, fps: f32) -> ClipFrameSource {
        let character_node = anigo_core::NodeId::canonical_character().to_string();
        let rest_mesh = base.nodes.iter().find(|n| n.id == character_node).and_then(|n| n.mesh.clone());
        ClipFrameSource { base, project, rig, clip, fps, character_node, rest_mesh }
    }

    fn animates(&self, pred: impl Fn(&ChannelTarget) -> bool) -> bool {
        self.clip.channels.iter().any(|c| pred(&c.target))
    }

    /// Cena de um quadro de animação.
    pub fn scene_at(&self, frame: u32) -> Scene {
        let sample = self.clip.evaluate(self.fps, frame as f32);
        let mut project = self.project.clone();
        sample.apply_to_project(&mut project);
        let mut scene = self.base.clone();
        // transform dos nós
        for node in &mut scene.nodes {
            if let Ok(id) = anigo_core::NodeId::parse(&node.id) {
                if let Some(slot) = project.scene.node(&id) {
                    node.transform = slot.transform;
                }
            }
        }
        // pose: paleta de skinning e dedos
        let pose = project.character.pose.clone();
        if pose.is_rest() {
            scene.skin = anigo_core::snapshot::SkinPayload::canonical_base();
        } else {
            scene.skin.palette = self.rig.palette_floats(&pose);
            scene.skin.palette_is_identity = anigo_core::skinning::flat_palette_is_identity(&scene.skin.palette);
        }
        if let (false, Some(rest)) = (pose.hands.is_rest(), &self.rest_mesh) {
            let mut mesh = rest.clone();
            finger_curl_mesh(&mut mesh, &pose.hands);
            if let Some(n) = scene.nodes.iter_mut().find(|n| n.id == self.character_node) {
                n.mesh = Some(mesh);
            }
        }
        // sol e câmera: só o que o clip anima (o resto fica como o Render configurou)
        if self.animates(|t| matches!(t, ChannelTarget::Sun { .. })) {
            if let Some(slot) = project.scene.lights.first() {
                let l = &slot.light;
                if self.animates(|t| matches!(t, ChannelTarget::Sun { prop: SunProp::Direction })) {
                    scene.light.direction = l.direction;
                }
                if self.animates(|t| matches!(t, ChannelTarget::Sun { prop: SunProp::Intensity })) {
                    scene.light.intensity = l.intensity;
                }
                if self.animates(|t| matches!(t, ChannelTarget::Sun { prop: SunProp::Color })) {
                    scene.light.color = l.color;
                }
            }
        }
        let cam = &project.scene.camera.camera;
        if self.animates(|t| matches!(t, ChannelTarget::Camera { prop: CameraProp::Fov })) {
            scene.camera.fov_y = cam.fov_y;
        }
        if self.animates(|t| matches!(t, ChannelTarget::Camera { prop: CameraProp::Eye })) {
            scene.camera.eye = cam.eye;
        }
        if self.animates(|t| matches!(t, ChannelTarget::Camera { prop: CameraProp::Target })) {
            scene.camera.target = cam.target;
        }
        scene
    }
}

impl FrameSource for ClipFrameSource {
    fn scene(&self, frame: Option<u32>) -> Result<Scene, String> {
        Ok(self.scene_at(frame.unwrap_or(0)))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use anigo_core::animation::{canonicalize_clip, Channel, Key, KeyValue};
    use anigo_core::ids::AnimationClipId;
    use anigo_core::scene::SceneNode;
    use glam::Quat;

    fn base_scene(project: &ProjectState) -> Scene {
        let mut s = Scene::new_empty();
        let mut c = SceneNode::new(anigo_core::NodeId::canonical_character().to_string(), "Personagem");
        c.mesh = Some(anigo_core::deformation::canonical_base_mesh(anigo_core::BaseGender::Male));
        s.add_node(c);
        s.camera = project.scene.camera.camera.clone();
        s.light = project.scene.lights.first().map(|l| l.light.clone()).unwrap_or_default();
        s
    }

    fn clip() -> AnimationClip {
        let mut c = AnimationClip::new(AnimationClipId::from_slug("seq"), "seq");
        c.range = Some([0, 24]);
        let node = anigo_core::NodeId::canonical_character();
        c.channels = vec![
            Channel { target: ChannelTarget::bone("LeftElbow"), keys: vec![Key::new(0, KeyValue::Quat([0.0, 0.0, 0.0, 1.0])), Key::new(24, KeyValue::Quat(Quat::from_rotation_x(-1.5).to_array()))] },
            Channel { target: ChannelTarget::NodeTranslation { node_id: node }, keys: vec![Key::new(0, KeyValue::Vec3([0.0; 3])), Key::new(24, KeyValue::Vec3([2.0, 0.0, 0.0]))] },
            Channel { target: ChannelTarget::Camera { prop: CameraProp::Fov }, keys: vec![Key::new(0, KeyValue::Scalar(30.0)), Key::new(24, KeyValue::Scalar(60.0))] },
            Channel { target: ChannelTarget::Hand { side: anigo_core::animation::HandSide::Left }, keys: vec![Key::new(0, KeyValue::Curl([0.0; 5])), Key::new(24, KeyValue::Curl([1.0; 5]))] },
        ];
        canonicalize_clip(&mut c);
        c
    }

    #[test]
    fn each_frame_scene_carries_the_evaluated_clip() {
        let mut project = ProjectState::default();
        let c = clip();
        project.animation.clips.insert(c.clip_id.clone(), c.clone());
        let base = base_scene(&project);
        let src = ClipFrameSource::new(base.clone(), project.clone(), PoseRig::default(), c, 24.0);
        let (s0, s12, s24) = (src.scene(Some(0)).unwrap(), src.scene(Some(12)).unwrap(), src.scene(Some(24)).unwrap());
        // paleta: identidade no quadro 0; muda no meio e no fim
        assert!(s0.skin.palette_is_identity);
        assert!(!s12.skin.palette_is_identity && !s24.skin.palette_is_identity);
        assert_ne!(s12.skin.palette, s24.skin.palette);
        // o nó do personagem anda e a câmera abre
        assert_eq!(s0.nodes[0].transform.translation.x, 0.0);
        assert!((s12.nodes[0].transform.translation.x - 1.0).abs() < 1e-5);
        assert!((s24.camera.fov_y.to_degrees() - 60.0).abs() < 1e-3);
        assert!((s12.camera.fov_y.to_degrees() - 45.0).abs() < 1e-3);
        // dedos: a malha do personagem muda com a curvatura, e o quadro 0 é a malha de repouso
        let m = |s: &Scene| s.nodes[0].mesh.clone().unwrap();
        assert_eq!(m(&s0).vertices.iter().map(|v| v.position).collect::<Vec<_>>(), m(&base).vertices.iter().map(|v| v.position).collect::<Vec<_>>());
        assert_ne!(m(&s24).vertices.iter().map(|v| v.position).collect::<Vec<_>>(), m(&s0).vertices.iter().map(|v| v.position).collect::<Vec<_>>());
        // determinismo: o mesmo quadro duas vezes dá a mesma cena
        assert_eq!(src.scene_at(12).skin.palette, s12.skin.palette);
        // a câmera/sol que o clip NÃO anima ficam como o Render configurou
        assert_eq!(s24.camera.eye, base.camera.eye);
        assert_eq!(s24.light, base.light);
    }

    #[test]
    fn a_sun_animation_changes_only_the_animated_light_properties() {
        let mut project = ProjectState::default();
        let mut c = AnimationClip::new(AnimationClipId::from_slug("sol"), "sol");
        c.channels = vec![Channel { target: ChannelTarget::Sun { prop: SunProp::Intensity }, keys: vec![Key::new(0, KeyValue::Scalar(0.5)), Key::new(10, KeyValue::Scalar(2.0))] }];
        c.range = Some([0, 10]);
        project.animation.clips.insert(c.clip_id.clone(), c.clone());
        let base = base_scene(&project);
        let src = ClipFrameSource::new(base.clone(), project, PoseRig::default(), c, 24.0);
        let s = src.scene_at(10);
        assert_eq!(s.light.intensity, 2.0);
        assert_eq!(s.light.direction, base.light.direction);
        assert_eq!(s.light.color, base.light.color);
    }
}
