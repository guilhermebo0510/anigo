//! Esqueleto VRM humanoid completo (55 ossos, hierarquia exata da spec VRM 1.0) mais ossos de
//! twist (4 por lado) e de busto (2), em T-pose com rotações de repouso identidade. Cada osso
//! é posicionado por um *localizador de junta* (combinação afim fixa de vértices do Skin), então
//! o esqueleto é re-derivado de qualquer malha morfada ou esculpida com a mesma topologia.

use crate::topology::{Locator, Topology};
use glam::{Mat4, Vec3};
use serde::{Deserialize, Serialize};

pub const HUMANOID_BONE_COUNT: usize = 55;
pub const TWIST_BONES: [&str; 8] = [
    "tw_leftUpperArm",
    "tw_leftLowerArm",
    "tw_rightUpperArm",
    "tw_rightLowerArm",
    "tw_leftUpperLeg",
    "tw_leftLowerLeg",
    "tw_rightUpperLeg",
    "tw_rightLowerLeg",
];
pub const BUST_BONES: [&str; 2] = ["bust_L", "bust_R"];

#[derive(Debug, Clone)]
pub struct BoneDef {
    pub name: String,
    pub parent: Option<usize>,
    /// Faz parte dos 55 ossos humanoid da spec VRM.
    pub humanoid: bool,
    pub locator: Locator,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Bone {
    pub name: String,
    pub parent: Option<usize>,
    pub humanoid: bool,
    /// Posição de repouso no mundo (T-pose, rotação identidade).
    pub world: [f32; 3],
    /// Posição local relativa ao pai.
    pub local: [f32; 3],
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Skeleton {
    pub bones: Vec<Bone>,
}

impl Skeleton {
    pub fn index(&self, name: &str) -> Option<usize> {
        self.bones.iter().position(|b| b.name == name)
    }

    pub fn pos(&self, i: usize) -> Vec3 {
        Vec3::from_array(self.bones[i].world)
    }

    pub fn len(&self) -> usize {
        self.bones.len()
    }

    pub fn is_empty(&self) -> bool {
        self.bones.is_empty()
    }

    /// Matrizes inversas de bind (translação pura: T-pose com rotação identidade).
    pub fn inverse_bind(&self) -> Vec<Mat4> {
        self.bones.iter().map(|b| Mat4::from_translation(-Vec3::from_array(b.world))).collect()
    }

    pub fn children(&self, i: usize) -> Vec<usize> {
        (0..self.bones.len()).filter(|&k| self.bones[k].parent == Some(i)).collect()
    }
}

/// Definição (nome, pai, localizador) dos ossos, na ordem de exportação.
pub fn bone_defs(t: &Topology) -> Vec<BoneDef> {
    let ring = |name: &str| -> Locator { Locator::centroid(t.mark(name)) };
    let lp = |name: &str| -> Locator { Locator::centroid(&t.loop_by_name(name).map(|l| l.verts.clone()).unwrap_or_default()) };
    let mut defs: Vec<BoneDef> = Vec::new();
    let mut add = |defs: &mut Vec<BoneDef>, name: &str, parent: Option<&str>, humanoid: bool, loc: Locator| {
        let p = parent.map(|pn| defs.iter().position(|d| d.name == pn).expect("pai deve vir antes"));
        defs.push(BoneDef {
            name: name.to_string(),
            parent: p,
            humanoid,
            locator: loc,
        });
    };
    add(&mut defs, "hips", None, true, ring("hip_ring"));
    add(&mut defs, "spine", Some("hips"), true, ring("spine_ring"));
    add(&mut defs, "chest", Some("spine"), true, ring("chest_ring"));
    add(&mut defs, "upperChest", Some("chest"), true, ring("upperchest_ring"));
    add(&mut defs, "neck", Some("upperChest"), true, ring("neck_base_ring"));
    add(&mut defs, "head", Some("neck"), true, ring("neck_top_ring"));

    // olhos: centro do globo = anel interno da pálpebra recuado ao longo da normal do contorno
    for (s, side) in [("L", "left"), ("R", "right")] {
        add(&mut defs, &format!("{side}Eye"), Some("head"), true, t.eye_center_locator(s));
    }
    let ear_mid = Locator::combine(&[(&ring("ear_L_center"), 0.5), (&ring("ear_R_center"), 0.5)]);
    let jaw = Locator::combine(&[(&ear_mid, 0.8), (&ring("neck_top_ring"), 0.2)]);
    add(&mut defs, "jaw", Some("head"), true, jaw);

    let fingers = ["Index", "Middle", "Ring", "Little"];
    for (s, side) in [("L", "left"), ("R", "right")] {
        let clav = Locator::combine(&[(&ring("upperchest_ring"), 0.5), (&ring("neck_base_ring"), 0.5)]);
        add(&mut defs, &format!("{side}Shoulder"), Some("upperChest"), true, clav);
        let hole = ring(&format!("arm_{s}_hole"));
        let r1 = ring(&format!("arm_{s}_ring1"));
        let sj = Locator::combine(&[(&hole, 1.7), (&r1, -0.7)]);
        add(&mut defs, &format!("{side}UpperArm"), Some(&format!("{side}Shoulder")), true, sj);
        add(&mut defs, &format!("{side}LowerArm"), Some(&format!("{side}UpperArm")), true, ring(&format!("elbow_{s}")));
        add(&mut defs, &format!("{side}Hand"), Some(&format!("{side}LowerArm")), true, ring(&format!("wrist_{s}")));
        let hand = format!("{side}Hand");
        // polegar: metacarpo, proximal, distal
        let th = |k: usize| ring(&format!("finger_{s}_thumb_ring{k}"));
        add(&mut defs, &format!("{side}ThumbMetacarpal"), Some(&hand), true, th(0));
        add(&mut defs, &format!("{side}ThumbProximal"), Some(&format!("{side}ThumbMetacarpal")), true, th(2));
        add(&mut defs, &format!("{side}ThumbDistal"), Some(&format!("{side}ThumbProximal")), true, th(4));
        for f in fingers {
            let lf = f.to_lowercase();
            let fr = |k: usize| ring(&format!("finger_{s}_{lf}_ring{k}"));
            add(&mut defs, &format!("{side}{f}Proximal"), Some(&hand), true, fr(0));
            add(&mut defs, &format!("{side}{f}Intermediate"), Some(&format!("{side}{f}Proximal")), true, fr(2));
            add(&mut defs, &format!("{side}{f}Distal"), Some(&format!("{side}{f}Intermediate")), true, fr(4));
        }
    }
    for (s, side) in [("L", "left"), ("R", "right")] {
        let top = ring(&format!("thigh_top_{s}"));
        let r1 = ring(&format!("leg_{s}_ring1"));
        let hip = Locator::combine(&[(&top, 1.9), (&r1, -0.9)]);
        add(&mut defs, &format!("{side}UpperLeg"), Some("hips"), true, hip);
        add(&mut defs, &format!("{side}LowerLeg"), Some(&format!("{side}UpperLeg")), true, ring(&format!("knee_{s}")));
        add(&mut defs, &format!("{side}Foot"), Some(&format!("{side}LowerLeg")), true, ring(&format!("ankle_{s}")));
        add(&mut defs, &format!("{side}Toes"), Some(&format!("{side}Foot")), true, ring(&format!("foot_{s}_ball")));
    }
    // twist (pontos médios do segmento)
    let mid = |defs: &Vec<BoneDef>, a: &str, b: &str| -> Locator {
        let la = defs.iter().find(|d| d.name == a).unwrap().locator.clone();
        let lb = defs.iter().find(|d| d.name == b).unwrap().locator.clone();
        Locator::lerp(&la, &lb, 0.5)
    };
    for side in ["left", "right"] {
        let m = mid(&defs, &format!("{side}UpperArm"), &format!("{side}LowerArm"));
        add(&mut defs, &format!("tw_{side}UpperArm"), Some(&format!("{side}UpperArm")), false, m);
        let m = mid(&defs, &format!("{side}LowerArm"), &format!("{side}Hand"));
        add(&mut defs, &format!("tw_{side}LowerArm"), Some(&format!("{side}LowerArm")), false, m);
    }
    for side in ["left", "right"] {
        let m = mid(&defs, &format!("{side}UpperLeg"), &format!("{side}LowerLeg"));
        add(&mut defs, &format!("tw_{side}UpperLeg"), Some(&format!("{side}UpperLeg")), false, m);
        let m = mid(&defs, &format!("{side}LowerLeg"), &format!("{side}Foot"));
        add(&mut defs, &format!("tw_{side}LowerLeg"), Some(&format!("{side}LowerLeg")), false, m);
    }
    for s in ["L", "R"] {
        add(&mut defs, &format!("bust_{s}"), Some("chest"), false, ring(&format!("bust_{s}")));
    }
    defs
}

impl Skeleton {
    /// Re-deriva o esqueleto de um conjunto de posições do Skin (mesma topologia).
    pub fn derive(defs: &[BoneDef], pos: &[Vec3]) -> Skeleton {
        let world: Vec<Vec3> = defs.iter().map(|d| d.locator.eval(pos)).collect();
        let bones = defs
            .iter()
            .enumerate()
            .map(|(i, d)| {
                let local = match d.parent {
                    Some(p) => world[i] - world[p],
                    None => world[i],
                };
                Bone {
                    name: d.name.clone(),
                    parent: d.parent,
                    humanoid: d.humanoid,
                    world: world[i].to_array(),
                    local: local.to_array(),
                }
            })
            .collect();
        Skeleton { bones }
    }
}

pub fn reference_skeleton(t: &Topology, pos: &[Vec3]) -> (Skeleton, Vec<Vec3>) {
    let defs = bone_defs(t);
    (Skeleton::derive(&defs, pos), pos.to_vec())
}

/// Nomes VRM humanoid em ordem (os 55).
pub fn humanoid_names() -> Vec<String> {
    let mut v = vec!["hips", "spine", "chest", "upperChest", "neck", "head"].into_iter().map(String::from).collect::<Vec<_>>();
    v.push("leftEye".into());
    v.push("rightEye".into());
    v.push("jaw".into());
    for side in ["left", "right"] {
        for b in ["Shoulder", "UpperArm", "LowerArm", "Hand", "ThumbMetacarpal", "ThumbProximal", "ThumbDistal"] {
            v.push(format!("{side}{b}"));
        }
        for f in ["Index", "Middle", "Ring", "Little"] {
            for p in ["Proximal", "Intermediate", "Distal"] {
                v.push(format!("{side}{f}{p}"));
            }
        }
    }
    for side in ["left", "right"] {
        for b in ["UpperLeg", "LowerLeg", "Foot", "Toes"] {
            v.push(format!("{side}{b}"));
        }
    }
    v
}
