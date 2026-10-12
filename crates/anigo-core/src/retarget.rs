//! Retargeting de movimento humanoide para o esqueleto canônico de 24 ossos do ANIGO (base da importação de
//! glTF/VRMA/BVH/FBX da Animação; ver `docs/native/pesquisa/animacao.md` §4).
//!
//! ## Método (por direção de osso, não FK cega)
//!
//! A spec do VRM Animation dá a fórmula FK entre duas T-poses: `Normalized = W·L⁻¹·localA·W⁻¹` e
//! `localB = L_B·W_B⁻¹·Normalized·W_B`. Ela supõe repousos iguais. O rig do ANIGO repousa com os **braços
//! pendurados** e todas as rotações de repouso são a identidade; um BVH/VRMA em T-pose aplicado por FK cega
//! aponta os braços para a direção errada. Aqui o movimento é transferido pela DIREÇÃO do osso no mundo:
//!
//! 1. converte o esqueleto fonte para os eixos do ANIGO (Y para cima, esquerda do personagem = +X, frente = +Z),
//!    deduzidos da pose de repouso (quadril→cabeça = cima; coxa E − coxa D = esquerda), então não depende da
//!    convenção do arquivo (Y-up/Z-up, VRM 0.x/1.0, Mixamo, CMU…);
//! 2. para cada osso mapeado, `D = W_anim · W_repouso⁻¹` (delta de mundo) e a direção animada
//!    `d = D · dir_repouso_fonte` (osso → próximo osso mapeado da cadeia);
//! 3. o delta de mundo do destino é o *swing* mínimo `dir_repouso_destino → d` mais a *torção* da fonte em torno do
//!    eixo do osso: `Dt = swing · twist`. Com repousos iguais isto reproduz `D` exatamente (teste);
//! 4. rotação local = `Dt(pai)⁻¹ · Dt(osso)`; ossos ausentes na fonte herdam o delta do pai;
//! 5. a translação do quadril é `Δp` do quadril escalada por `altura_destino / altura_fonte` (VRMA).

use std::collections::BTreeMap;

use glam::{Mat3, Quat, Vec3};

use crate::animation::{canonicalize_clip, Channel, ChannelTarget, Interp, Key, KeyValue, MAX_FRAME};
use crate::ids::AnimationClipId;
use crate::pose::{bone, bone_index, PoseRig, PoseState, BONE_COUNT, BONE_PARENTS};
use crate::project::AnimationClip;

// ---------------------------------------------------------------------------------------------
// Esqueleto e movimento fonte
// ---------------------------------------------------------------------------------------------

/// Junta do esqueleto fonte (repouso LOCAL).
#[derive(Debug, Clone, PartialEq)]
pub struct SrcJoint {
    pub name: String,
    pub parent: Option<usize>,
    pub rest_t: Vec3,
    pub rest_r: Quat,
}

/// Esqueleto fonte: pais sempre antes dos filhos.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct SrcSkeleton {
    pub joints: Vec<SrcJoint>,
}

impl SrcSkeleton {
    /// Valida a ordem topológica (pai antes do filho).
    pub fn validate(&self) -> Result<(), String> {
        for (i, j) in self.joints.iter().enumerate() {
            if let Some(p) = j.parent {
                if p >= i {
                    return Err(format!("junta '{}' vem antes do pai", j.name));
                }
            }
            if !j.rest_t.is_finite() || !j.rest_r.is_finite() {
                return Err(format!("repouso não finito em '{}'", j.name));
            }
        }
        Ok(())
    }

    /// Posição e rotação de mundo de cada junta no repouso.
    pub fn rest_world(&self) -> (Vec<Vec3>, Vec<Quat>) {
        self.world(&self.joints.iter().map(|j| j.rest_r).collect::<Vec<_>>(), &BTreeMap::new())
    }

    /// FK com rotações locais `rot` e translações locais substituídas em `pos` (índice → translação local).
    pub fn world(&self, rot: &[Quat], pos: &BTreeMap<usize, Vec3>) -> (Vec<Vec3>, Vec<Quat>) {
        let n = self.joints.len();
        let mut wp = vec![Vec3::ZERO; n];
        let mut wr = vec![Quat::IDENTITY; n];
        for (i, j) in self.joints.iter().enumerate() {
            let local_t = pos.get(&i).copied().unwrap_or(j.rest_t);
            let local_r = rot.get(i).copied().unwrap_or(j.rest_r);
            match j.parent {
                Some(p) => {
                    wp[i] = wp[p] + wr[p] * local_t;
                    wr[i] = (wr[p] * local_r).normalize();
                }
                None => {
                    wp[i] = local_t;
                    wr[i] = local_r.normalize();
                }
            }
        }
        (wp, wr)
    }
}

/// Um quadro do movimento fonte: rotação local de cada junta e translações locais que mudaram.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct SrcFrame {
    pub rot: Vec<Quat>,
    pub pos: BTreeMap<usize, Vec3>,
}

// ---------------------------------------------------------------------------------------------
// Mapeamento de nomes
// ---------------------------------------------------------------------------------------------

/// Osso canônico → junta fonte.
#[derive(Debug, Clone, PartialEq)]
pub struct BoneMap(pub [Option<usize>; BONE_COUNT]);

impl BoneMap {
    pub fn empty() -> BoneMap {
        BoneMap([None; BONE_COUNT])
    }

    pub fn mapped_count(&self) -> usize {
        self.0.iter().filter(|b| b.is_some()).count()
    }

    /// Mapeia as juntas pelo nome (VRM humanoid, Mixamo, VRoid, Rigify, CMU, 3ds Max Biped…). Em caso de duas
    /// juntas candidatas ao mesmo osso, vale a de MENOR profundidade na hierarquia.
    pub fn from_names(skel: &SrcSkeleton) -> BoneMap {
        let depth = |mut i: usize| {
            let mut d = 0;
            while let Some(p) = skel.joints[i].parent {
                d += 1;
                i = p;
            }
            d
        };
        let mut map = BoneMap::empty();
        // vocabulário canônico do ANIGO (ex.: arquivo exportado por nós): "LeftShoulder" é o braço, não a clavícula
        let exact = |n: &str| skel.joints.iter().any(|j| j.name == n);
        let canonical_vocab = exact("LeftElbow") && exact("LeftWrist") && exact("Hips");
        if canonical_vocab {
            for (j, joint) in skel.joints.iter().enumerate() {
                if let Some(b) = bone_index(&joint.name) {
                    if b != bone::ROOT {
                        map.0[b] = Some(j);
                    }
                }
            }
            return map;
        }
        let mut best_depth = [usize::MAX; BONE_COUNT];
        for (j, joint) in skel.joints.iter().enumerate() {
            if let Some(b) = canonical_bone_for_name(&joint.name) {
                if b == bone::ROOT {
                    continue;
                }
                let d = depth(j);
                if d < best_depth[b] {
                    best_depth[b] = d;
                    map.0[b] = Some(j);
                }
            }
        }
        map
    }
}

fn normalize_name(name: &str) -> String {
    let lower = name.to_lowercase();
    // prefixos de namespace comuns (mixamorig:Hips, Armature|Hips, bip01 Pelvis…)
    let tail = lower.rsplit([':', '|']).next().unwrap_or(&lower).to_string();
    let mut s = tail;
    for prefix in ["mixamorig", "bip001", "bip01", "bip", "cc_base_", "j_bip_", "j_adj_", "def-", "def_", "armature"] {
        if let Some(rest) = s.strip_prefix(prefix) {
            s = rest.trim_start_matches(['_', '-', ' ', '.']).to_string();
        }
    }
    s
}

/// Lado (`-1` direita, `1` esquerda, `0` centro) e o resto do nome sem a marca de lado.
fn split_side(name: &str) -> (i8, String) {
    let n = name.to_string();
    // marcadores de lado: prefixo, sufixo ou infixo
    let mut side = 0i8;
    // j_bip_l_upperarm -> "l_upperarm"; upper_arm.l; leftarm; lupleg; left_arm
    let tokens: Vec<&str> = n.split(['_', '.', ' ', '-']).filter(|t| !t.is_empty() && !matches!(*t, "c" | "center" | "mid")).collect();
    let mut kept: Vec<String> = Vec::new();
    let mut found = false;
    for (i, t) in tokens.iter().enumerate() {
        let is_l = matches!(*t, "l" | "left" | "lt");
        let is_r = matches!(*t, "r" | "right" | "rt");
        if (is_l || is_r) && !found {
            side = if is_l { 1 } else { -1 };
            found = true;
            let _ = i;
            continue;
        }
        kept.push((*t).to_string());
    }
    let rest = kept.join("");
    if found {
        // lado encontrado como token
    } else {
        // fundido: leftarm, rightupleg, lthigh, rfemur, lhumerus
        let compact = tokens.join("");
        for (prefix, s) in [("left", 1i8), ("right", -1)] {
            if let Some(r) = compact.strip_prefix(prefix) {
                return (s, r.to_string());
            }
            if let Some(r) = compact.strip_suffix(prefix) {
                return (s, r.to_string());
            }
        }
        const SHORT: [&str; 18] = [
            "upleg", "uparm", "thigh", "femur", "humerus", "clavicle", "shoulder", "arm", "forearm", "hand", "leg", "foot", "toe", "calf", "shin", "radius", "tibia", "wrist",
        ];
        for (prefix, s) in [("l", 1i8), ("r", -1)] {
            if let Some(r) = compact.strip_prefix(prefix) {
                if SHORT.iter().any(|k| r.starts_with(k)) {
                    return (s, r.to_string());
                }
            }
        }
    }
    (side, rest)
}

/// Osso canônico de um nome de junta (qualquer convenção comum), ou `None`.
pub fn canonical_bone_for_name(raw: &str) -> Option<usize> {
    use bone::*;
    let name = normalize_name(raw);
    // nomes VRM humanoid camelCase exatos têm prioridade (normalizados em minúsculas)
    let (side, core) = split_side(&name);
    let core = core.replace(['_', '.', ' ', '-'], "");
    let pick = |l: usize, r: usize, c: Option<usize>| -> Option<usize> {
        match side {
            1 => Some(l),
            -1 => Some(r),
            _ => c,
        }
    };
    match core.as_str() {
        "hips" | "hip" | "pelvis" if side == 0 => Some(HIPS),
        "spine" | "spine0" | "abdomen" | "waist" | "lowerback" => Some(SPINE),
        "spine1" | "chest" | "spine01" | "upperback" => Some(CHEST),
        "spine2" | "upperchest" | "chest2" | "chest1" | "spine02" | "thorax" => Some(UPPER_CHEST),
        "neck" | "neck1" | "neckbase" | "lowerneck" => Some(NECK),
        "head" => Some(HEAD),
        "shoulder" | "clavicle" | "collar" | "collarbone" => pick(L_CLAVICLE, R_CLAVICLE, None),
        "upperarm" | "arm" | "uparm" | "humerus" | "upperarm01" => pick(L_SHOULDER, R_SHOULDER, None),
        "lowerarm" | "forearm" | "radius" | "elbow" | "lowerarm01" => pick(L_ELBOW, R_ELBOW, None),
        "hand" | "wrist" => pick(L_WRIST, R_WRIST, None),
        "upperleg" | "upleg" | "thigh" | "femur" | "upperleg01" => pick(L_THIGH, R_THIGH, None),
        "lowerleg" | "leg" | "calf" | "shin" | "tibia" | "knee" | "lowerleg01" => pick(L_KNEE, R_KNEE, None),
        "foot" | "ankle" => pick(L_ANKLE, R_ANKLE, None),
        "toes" | "toe" | "toebase" | "toe0" | "ball" => pick(L_TOES, R_TOES, None),
        _ => None,
    }
}

// ---------------------------------------------------------------------------------------------
// Retargeter
// ---------------------------------------------------------------------------------------------

/// Próximo osso da cadeia principal (direção do osso).
fn chain_next(b: usize) -> Option<usize> {
    use bone::*;
    Some(match b {
        HIPS => SPINE,
        SPINE => CHEST,
        CHEST => UPPER_CHEST,
        UPPER_CHEST => NECK,
        NECK => HEAD,
        L_CLAVICLE => L_SHOULDER,
        L_SHOULDER => L_ELBOW,
        L_ELBOW => L_WRIST,
        R_CLAVICLE => R_SHOULDER,
        R_SHOULDER => R_ELBOW,
        R_ELBOW => R_WRIST,
        L_THIGH => L_KNEE,
        L_KNEE => L_ANKLE,
        L_ANKLE => L_TOES,
        R_THIGH => R_KNEE,
        R_KNEE => R_ANKLE,
        R_ANKLE => R_TOES,
        _ => return None,
    })
}

/// Osso anterior da cadeia (para folhas: cabeça, mãos, pés).
fn chain_prev(b: usize) -> Option<usize> {
    use bone::*;
    Some(match b {
        HEAD => NECK,
        L_WRIST => L_ELBOW,
        R_WRIST => R_ELBOW,
        L_TOES => L_ANKLE,
        R_TOES => R_ANKLE,
        _ => return None,
    })
}

/// Direção de repouso de um osso: vetor `(origem, ponta)` em duas tabelas de posições.
#[derive(Debug, Clone, Copy)]
struct BoneDir {
    src: Vec3,
    dst: Vec3,
}

/// Retargeter pré-calculado para um esqueleto fonte, um mapa de ossos e o rig de destino.
#[derive(Debug, Clone)]
pub struct Retargeter {
    skel: SrcSkeleton,
    map: BoneMap,
    /// Rotação fonte→ANIGO aplicada ao mundo fonte.
    fix: Quat,
    rest_wr: Vec<Quat>,
    rest_wp: Vec<Vec3>,
    dirs: [Option<BoneDir>; BONE_COUNT],
    hips_scale: f32,
    /// Avisos de importação (ossos ausentes, eixos…).
    pub warnings: Vec<String>,
}

impl Retargeter {
    pub fn new(skel: &SrcSkeleton, map: &BoneMap, rig: &PoseRig) -> Result<Retargeter, String> {
        skel.validate()?;
        let hips = map.0[bone::HIPS].ok_or("o esqueleto não tem quadril (Hips) reconhecível")?;
        let (rest_wp, rest_wr) = skel.rest_world();
        let mut warnings = Vec::new();
        let missing: Vec<&str> = [bone::SPINE, bone::NECK, bone::HEAD, bone::L_THIGH, bone::R_THIGH, bone::L_SHOULDER, bone::R_SHOULDER]
            .into_iter()
            .filter(|b| map.0[*b].is_none())
            .map(|b| crate::pose::BONE_NAMES[b])
            .collect();
        if !missing.is_empty() {
            warnings.push(format!("ossos principais não encontrados no arquivo: {}", missing.join(", ")));
        }
        let fix = frame_fix(map, &rest_wp);
        let rest_pos = |b: usize| map.0[b].map(|j| fix * rest_wp[j]);

        let mut dirs: [Option<BoneDir>; BONE_COUNT] = [None; BONE_COUNT];
        for b in 0..BONE_COUNT {
            let Some(_) = map.0[b] else { continue };
            // próximo osso mapeado na cadeia (pula os ausentes)
            let mut next = chain_next(b);
            while let Some(n) = next {
                if map.0[n].is_some() {
                    break;
                }
                next = chain_next(n);
            }
            let (a_s, b_s, a_d, b_d) = if let Some(n) = next {
                (rest_pos(b), rest_pos(n), rig.bind[b], rig.bind[n])
            } else if let Some(p) = chain_prev(b) {
                // folha: usa a direção do osso anterior até este
                if map.0[p].is_none() {
                    continue;
                }
                (rest_pos(p), rest_pos(b), rig.bind[p], rig.bind[b])
            } else {
                continue;
            };
            let (Some(a_s), Some(b_s)) = (a_s, b_s) else { continue };
            let (ds, dd) = ((b_s - a_s), (b_d - a_d));
            if ds.length() < 1e-5 || dd.length() < 1e-5 {
                continue;
            }
            dirs[b] = Some(BoneDir { src: ds.normalize(), dst: dd.normalize() });
        }
        // escala do quadril: altura do quadril acima do pé mais baixo
        let feet = [bone::L_TOES, bone::R_TOES, bone::L_ANKLE, bone::R_ANKLE];
        let foot_y = feet.into_iter().filter_map(rest_pos).map(|p| p.y).fold(f32::INFINITY, f32::min);
        let hips_y_src = rest_pos(bone::HIPS).map(|p| p.y).unwrap_or(0.0);
        let h_src = if foot_y.is_finite() { hips_y_src - foot_y } else { hips_y_src };
        // mesma definição nos dois lados (quadril acima do pé mais baixo), para repousos iguais darem escala 1
        let foot_y_dst = feet.into_iter().filter(|b| map.0[*b].is_some()).map(|b| rig.bind[b].y).fold(f32::INFINITY, f32::min);
        let h_dst = if foot_y_dst.is_finite() { rig.bind[bone::HIPS].y - foot_y_dst } else { rig.bind[bone::HIPS].y - rig.sole_y };
        let hips_scale = if h_src > 0.05 && h_dst > 0.05 { h_dst / h_src } else { 1.0 };
        let _ = hips;
        Ok(Retargeter { skel: skel.clone(), map: map.clone(), fix, rest_wr, rest_wp, dirs, hips_scale, warnings })
    }

    /// Escala aplicada à translação do quadril (`altura_destino / altura_fonte`).
    pub fn hips_scale(&self) -> f32 {
        self.hips_scale
    }

    /// Rotação fonte→ANIGO deduzida da pose de repouso.
    pub fn frame_fix(&self) -> Quat {
        self.fix
    }

    /// Pose canônica de um quadro do movimento fonte.
    pub fn pose(&self, frame: &SrcFrame) -> PoseState {
        let (wp, wr) = self.skel.world(&frame.rot, &frame.pos);
        let fix = self.fix;
        let mut dt: [Quat; BONE_COUNT] = [Quat::IDENTITY; BONE_COUNT];
        let mut pose = PoseState::default();
        for b in 0..BONE_COUNT {
            let parent_dt = BONE_PARENTS[b].map(|p| dt[p]).unwrap_or(Quat::IDENTITY);
            let (Some(j), Some(dir)) = (self.map.0[b], self.dirs[b]) else {
                // sem direção (ausente, Pelvis, Root…): acompanha o pai
                dt[b] = parent_dt;
                continue;
            };
            // delta de mundo da fonte, nos eixos do ANIGO
            let d_world = (wr[j] * self.rest_wr[j].inverse()).normalize();
            let d = (fix * d_world * fix.inverse()).normalize();
            let animated = d * dir.src;
            // torção em torno do eixo de repouso da fonte
            let twist_angle = twist_about(d, dir.src);
            let swing = Quat::from_rotation_arc(dir.dst, animated.normalize());
            let target = (swing * Quat::from_axis_angle(dir.dst, twist_angle)).normalize();
            dt[b] = target;
            let local = (parent_dt.inverse() * target).normalize();
            pose.set_rotation(b, local);
        }
        // translação do quadril
        if let Some(j) = self.map.0[bone::HIPS] {
            let rest = self.rest_wp[j];
            let delta = fix * (wp[j] - rest);
            let t = delta * self.hips_scale;
            if t.is_finite() && t.length() > 1e-6 {
                pose.hips_translation = t.to_array().map(|c| c.clamp(-crate::pose::MAX_TRANSLATION, crate::pose::MAX_TRANSLATION));
            }
        }
        pose
    }
}

/// Ângulo de torção (com sinal, em torno de `axis`) de uma rotação.
fn twist_about(q: Quat, axis: Vec3) -> f32 {
    let v = Vec3::new(q.x, q.y, q.z);
    let proj = axis * v.dot(axis);
    let twist = Quat::from_xyzw(proj.x, proj.y, proj.z, q.w);
    if twist.length_squared() < 1e-12 {
        return 0.0;
    }
    let twist = twist.normalize();
    // ângulo com sinal em torno de `axis` (w ≥ 0 -> ângulo em [−π, π])
    let s = Vec3::new(twist.x, twist.y, twist.z).dot(axis);
    2.0 * s.atan2(twist.w)
}

/// Rotação fonte→ANIGO a partir do repouso: cima = quadril→cabeça; esquerda = coxa E − coxa D.
fn frame_fix(map: &BoneMap, rest_wp: &[Vec3]) -> Quat {
    let pos = |b: usize| map.0[b].map(|j| rest_wp[j]);
    let hips = pos(bone::HIPS);
    let top = pos(bone::HEAD).or_else(|| pos(bone::NECK)).or_else(|| pos(bone::UPPER_CHEST)).or_else(|| pos(bone::CHEST)).or_else(|| pos(bone::SPINE));
    let (Some(h), Some(t)) = (hips, top) else { return Quat::IDENTITY };
    let up = (t - h).normalize_or_zero();
    let left_vec = match (pos(bone::L_THIGH), pos(bone::R_THIGH)) {
        (Some(l), Some(r)) => l - r,
        _ => match (pos(bone::L_SHOULDER), pos(bone::R_SHOULDER)) {
            (Some(l), Some(r)) => l - r,
            _ => return Quat::IDENTITY,
        },
    };
    let left = (left_vec - up * left_vec.dot(up)).normalize_or_zero();
    if up == Vec3::ZERO || left == Vec3::ZERO {
        return Quat::IDENTITY;
    }
    let forward = left.cross(up);
    // B leva coordenadas ANIGO -> fonte; a correção é a inversa (B transposta)
    let b = Mat3::from_cols(left, up, forward);
    let fix = Quat::from_mat3(&b.transpose()).normalize();
    // arquivos de convenções diferentes diferem por múltiplos de 90°: encaixa no mais próximo dos 24 se estiver a
    // menos de 6° (a inclinação natural da coluna não vira erro sistemático em todos os ossos)
    let snapped = nearest_axis_rotation(fix);
    if fix.angle_between(snapped).to_degrees() < 6.0 {
        snapped
    } else {
        fix
    }
}

/// A rotação do grupo das 24 rotações alinhadas aos eixos mais próxima de `q`.
fn nearest_axis_rotation(q: Quat) -> Quat {
    let axes = [Vec3::X, Vec3::Y, Vec3::Z];
    let mut best = (f32::NEG_INFINITY, Quat::IDENTITY);
    for perm in [[0usize, 1, 2], [0, 2, 1], [1, 0, 2], [1, 2, 0], [2, 0, 1], [2, 1, 0]] {
        for signs in 0..8u32 {
            let s = |i: u32| if signs & (1 << i) != 0 { -1.0f32 } else { 1.0 };
            let m = Mat3::from_cols(axes[perm[0]] * s(0), axes[perm[1]] * s(1), axes[perm[2]] * s(2));
            if m.determinant() < 0.0 {
                continue;
            }
            let cand = Quat::from_mat3(&m).normalize();
            let d = cand.dot(q).abs();
            if d > best.0 {
                best = (d, cand);
            }
        }
    }
    best.1
}

// ---------------------------------------------------------------------------------------------
// O próprio rig como fonte (exportação) e variantes de repouso
// ---------------------------------------------------------------------------------------------

/// Esqueleto fonte idêntico ao rig canônico (nomes canônicos, repouso sem rotação): serve de FONTE para exportar
/// um clip para outro repouso (ex.: a T-pose do VRM Animation) com o mesmo retargeter da importação.
pub fn canonical_source_skeleton(rig: &PoseRig) -> SrcSkeleton {
    SrcSkeleton {
        joints: (0..BONE_COUNT)
            .map(|b| SrcJoint { name: crate::pose::BONE_NAMES[b].to_string(), parent: BONE_PARENTS[b], rest_t: rig.offset(b), rest_r: Quat::IDENTITY })
            .collect(),
    }
}

/// Quadro fonte que reproduz uma pose canônica no esqueleto de [`canonical_source_skeleton`].
pub fn source_frame_of_pose(pose: &PoseState, rig: &PoseRig) -> SrcFrame {
    let mut f = SrcFrame { rot: (0..BONE_COUNT).map(|b| pose.rotation(b)).collect(), pos: BTreeMap::new() };
    f.pos.insert(bone::HIPS, rig.offset(bone::HIPS) + Vec3::from_array(pose.hips_translation));
    f
}

/// O mesmo rig em T-pose: braços esticados na horizontal (a malha do ANIGO repousa com os braços pendurados).
/// Comprimentos de osso preservados. É o repouso exigido pelo VRM Animation.
pub fn t_pose_rig(rig: &PoseRig) -> PoseRig {
    let mut out = rig.clone();
    for (shoulder, sign) in [(bone::L_SHOULDER, 1.0f32), (bone::R_SHOULDER, -1.0)] {
        let origin = rig.bind[shoulder];
        let upper = rig.bind[shoulder].distance(rig.bind[shoulder + 1]);
        let lower = rig.bind[shoulder + 1].distance(rig.bind[shoulder + 2]);
        out.bind[shoulder + 1] = origin + Vec3::new(sign * upper, 0.0, 0.0);
        out.bind[shoulder + 2] = origin + Vec3::new(sign * (upper + lower), 0.0, 0.0);
    }
    out
}

// ---------------------------------------------------------------------------------------------
// Poses -> clip
// ---------------------------------------------------------------------------------------------

/// Opções da conversão em clip.
#[derive(Debug, Clone, Copy)]
pub struct ClipBuild {
    /// Remove chaves redundantes (Douglas–Peucker por canal).
    pub reduce: bool,
    /// Tolerância de rotação, em graus.
    pub tol_deg: f32,
    /// Tolerância de translação, em metros.
    pub tol_m: f32,
    /// Interpolação das chaves criadas.
    pub interp: Interp,
}

impl Default for ClipBuild {
    fn default() -> Self {
        ClipBuild { reduce: true, tol_deg: 0.25, tol_m: 0.002, interp: Interp::Linear }
    }
}

/// Reamostra `poses` (a `src_fps`) para `target_fps` e monta um clip com chaves de osso e de quadril.
pub fn poses_to_clip(poses: &[PoseState], src_fps: f32, target_fps: f32, id: AnimationClipId, name: &str, opts: ClipBuild) -> AnimationClip {
    let mut clip = AnimationClip::new(id, name);
    if poses.is_empty() {
        return clip;
    }
    let src_fps = src_fps.max(1.0);
    let target_fps = target_fps.max(1.0);
    let duration = (poses.len() - 1) as f32 / src_fps;
    let frames = ((duration * target_fps).round() as i32).clamp(0, MAX_FRAME);
    let at = |f: i32| -> PoseState {
        let u = (f as f32 / target_fps * src_fps).clamp(0.0, (poses.len() - 1) as f32);
        let i = u.floor() as usize;
        let j = (i + 1).min(poses.len() - 1);
        let t = u - i as f32;
        if t < 1e-6 || i == j {
            poses[i].clone()
        } else {
            PoseState::blend(&poses[i], &poses[j], t)
        }
    };
    let sampled: Vec<PoseState> = (0..=frames).map(at).collect();
    // quais ossos se mexem em algum quadro
    let mut channels: Vec<Channel> = Vec::new();
    for b in 0..BONE_COUNT {
        if sampled.iter().all(|p| p.rotation(b).angle_between(Quat::IDENTITY) < 1e-5) {
            continue;
        }
        let keys: Vec<Key> = sampled
            .iter()
            .enumerate()
            .map(|(f, p)| Key { frame: f as i32, value: KeyValue::Quat(crate::animation::canonical_quat(p.rotation(b).to_array())), interp: opts.interp })
            .collect();
        channels.push(Channel { target: ChannelTarget::bone_index(b), keys });
    }
    if sampled.iter().any(|p| p.hips_translation.iter().any(|c| c.abs() > 1e-6)) {
        let keys: Vec<Key> = sampled.iter().enumerate().map(|(f, p)| Key { frame: f as i32, value: KeyValue::Vec3(p.hips_translation), interp: opts.interp }).collect();
        channels.push(Channel { target: ChannelTarget::Hips, keys });
    }
    if opts.reduce {
        for ch in &mut channels {
            ch.keys = if opts.interp == Interp::Step { dedupe_step_keys(&ch.keys) } else { reduce_keys(&ch.keys, opts.tol_deg, opts.tol_m) };
        }
    }
    clip.channels = channels;
    clip.range = Some([0, frames]);
    clip.duration_seconds = duration;
    canonicalize_clip(&mut clip);
    clip
}

fn key_error(a: &Key, b: &Key, k: &Key) -> (f32, bool) {
    let u = (k.frame - a.frame) as f32 / (b.frame - a.frame) as f32;
    let m = KeyValue::mix(&a.value, &b.value, u);
    match (m, k.value) {
        (KeyValue::Quat(x), KeyValue::Quat(y)) => (Quat::from_xyzw(x[0], x[1], x[2], x[3]).angle_between(Quat::from_xyzw(y[0], y[1], y[2], y[3])).to_degrees(), true),
        (KeyValue::Vec3(x), KeyValue::Vec3(y)) => (Vec3::from_array(x).distance(Vec3::from_array(y)), false),
        _ => (0.0, false),
    }
}

/// Interpolação em degrau: só as chaves em que o valor muda (mais a última, que fixa o fim da faixa).
pub fn dedupe_step_keys(keys: &[Key]) -> Vec<Key> {
    let mut out: Vec<Key> = Vec::new();
    for (i, k) in keys.iter().enumerate() {
        let last = i + 1 == keys.len();
        if last || out.last().is_none_or(|p| p.value != k.value) {
            out.push(*k);
        }
    }
    out
}

/// Douglas–Peucker sobre as chaves de um canal (erro máx. pela interpolação linear/slerp entre as pontas).
pub fn reduce_keys(keys: &[Key], tol_deg: f32, tol_m: f32) -> Vec<Key> {
    if keys.len() <= 2 {
        return keys.to_vec();
    }
    let mut keep = vec![false; keys.len()];
    keep[0] = true;
    keep[keys.len() - 1] = true;
    let mut stack = vec![(0usize, keys.len() - 1)];
    while let Some((lo, hi)) = stack.pop() {
        if hi <= lo + 1 {
            continue;
        }
        let (mut worst, mut worst_i, mut worst_over) = (0.0f32, lo, false);
        for i in lo + 1..hi {
            let (e, is_rot) = key_error(&keys[lo], &keys[hi], &keys[i]);
            let tol = if is_rot { tol_deg } else { tol_m };
            let over = e > tol;
            if over && (e / tol.max(1e-9)) > worst {
                worst = e / tol.max(1e-9);
                worst_i = i;
                worst_over = true;
            }
        }
        if worst_over {
            keep[worst_i] = true;
            stack.push((lo, worst_i));
            stack.push((worst_i, hi));
        }
    }
    keys.iter().zip(keep).filter(|(_, k)| *k).map(|(k, _)| *k).collect()
}

#[cfg(test)]
mod tests;
