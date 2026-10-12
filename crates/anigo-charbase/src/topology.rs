//! Topologia estática do Skin (idêntica para qualquer `Params`): faces, regiões, laços,
//! grupos de vértices, UV, simetria e pesos de skin. É construída uma vez a partir de uma
//! geração de referência e é o que o contrato de topologia (`contract`) descreve.

use crate::cage::{build_cage, Cage, LoopDef, Region};
use crate::params::{smoothstep, Params};
use crate::qmesh::{edge_key, QuadMesh};
use crate::subdiv::{catmull_clark, Refined};
use anigo_core::morph_catalog::AnatomicalZone;
use glam::{Vec2, Vec3};
use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::sync::OnceLock;

pub const ZONE_COUNT: usize = 18;

/// As 18 zonas anatômicas do `AnatomicalZone` do core, na ordem do enum, com o nome do grupo.
pub const ZONES: [(AnatomicalZone, &str); ZONE_COUNT] = [
    (AnatomicalZone::GlobalSilhouette, "zone_global_silhouette"),
    (AnatomicalZone::Craniofacial, "zone_craniofacial"),
    (AnatomicalZone::Eyes, "zone_eyes"),
    (AnatomicalZone::Eyebrows, "zone_eyebrows"),
    (AnatomicalZone::Nose, "zone_nose"),
    (AnatomicalZone::MouthLips, "zone_mouth_lips"),
    (AnatomicalZone::JawChin, "zone_jaw_chin"),
    (AnatomicalZone::Ears, "zone_ears"),
    (AnatomicalZone::NeckTrapezius, "zone_neck_trapezius"),
    (AnatomicalZone::ShouldersClavicles, "zone_shoulders_clavicles"),
    (AnatomicalZone::ChestPectorals, "zone_chest_pectorals"),
    (AnatomicalZone::BustFemale, "zone_bust_female"),
    (AnatomicalZone::AbdomenWaist, "zone_abdomen_waist"),
    (AnatomicalZone::PelvisHips, "zone_pelvis_hips"),
    (AnatomicalZone::Gluteus, "zone_gluteus"),
    (AnatomicalZone::UpperLimbs, "zone_upper_limbs"),
    (AnatomicalZone::HandsFingers, "zone_hands_fingers"),
    (AnatomicalZone::LowerLimbs, "zone_lower_limbs"),
];

pub fn zone_index(z: AnatomicalZone) -> usize {
    ZONES.iter().position(|(zz, _)| *zz == z).unwrap_or(0)
}

/// Combinação afim fixa de vértices (soma dos pesos = 1): âncora que sobrevive a qualquer morph.
#[derive(Debug, Clone, PartialEq)]
pub struct Locator(pub Vec<(u32, f32)>);

impl Locator {
    pub fn vertex(i: u32) -> Self {
        Locator(vec![(i, 1.0)])
    }

    pub fn centroid(ids: &[u32]) -> Self {
        let w = 1.0 / ids.len().max(1) as f32;
        Locator(ids.iter().map(|&i| (i, w)).collect())
    }

    pub fn eval(&self, pos: &[Vec3]) -> Vec3 {
        let mut p = Vec3::ZERO;
        for &(i, w) in &self.0 {
            if let Some(v) = pos.get(i as usize) {
                p += *v * w;
            }
        }
        p
    }

    /// `a * (1 - t) + b * t`
    pub fn lerp(a: &Locator, b: &Locator, t: f32) -> Locator {
        Self::combine(&[(a, 1.0 - t), (b, t)])
    }

    /// Combinação linear de localizadores (os pesos finais somam a soma dos coeficientes).
    pub fn combine(parts: &[(&Locator, f32)]) -> Locator {
        let mut acc: BTreeMap<u32, f32> = BTreeMap::new();
        for (l, k) in parts {
            for &(i, w) in &l.0 {
                *acc.entry(i).or_insert(0.0) += w * k;
            }
        }
        Locator(acc.into_iter().filter(|(_, w)| w.abs() > 1e-9).collect())
    }

    pub fn weight_sum(&self) -> f32 {
        self.0.iter().map(|(_, w)| *w).sum()
    }
}

pub struct Topology {
    pub faces: Vec<[u32; 4]>,
    pub n_verts: usize,
    pub n_cage: usize,
    pub region: Vec<Region>,
    /// [linha do tronco, sin(theta), cos(theta), anel do dedo], propagado para o Skin final.
    pub attr: Vec<[f32; 4]>,
    /// Laços nomeados já expandidos para o Skin final (intercalando os pontos de aresta).
    pub loops: Vec<LoopDef>,
    /// Conjuntos nomeados de vértices (ids da gaiola; continuam válidos no Skin final).
    pub marks: BTreeMap<String, Vec<u32>>,
    pub zones: Vec<[f32; ZONE_COUNT]>,
    pub masks: BTreeMap<String, Vec<f32>>,
    pub landmarks: Vec<(String, Locator)>,
    /// UV por canto de face (4 por face), em [0, 1].
    pub uv: Vec<[[f32; 2]; 4]>,
    pub chart_of_face: Vec<u8>,
    pub mirror: Vec<u32>,
    pub adjacency: Vec<Vec<u32>>,
    /// Pesos de skin estáticos (até 4 influências por vértice): (osso, peso).
    pub weights: Vec<[(u16, f32); 4]>,
    pub refined: RefinedInfo,
    pub cage_faces: Vec<[u32; 4]>,
}

/// Informações da subdivisão necessárias para expandir laços e propagar atributos.
pub struct RefinedInfo {
    pub edge_point: BTreeMap<(u32, u32), u32>,
    pub face_point: Vec<u32>,
}

pub fn build_adjacency(n: usize, faces: &[[u32; 4]]) -> Vec<Vec<u32>> {
    let mut sets: Vec<BTreeSet<u32>> = vec![BTreeSet::new(); n];
    for f in faces {
        for k in 0..4 {
            let (a, b) = (f[k], f[(k + 1) % 4]);
            sets[a as usize].insert(b);
            sets[b as usize].insert(a);
        }
    }
    sets.into_iter().map(|s| s.into_iter().collect()).collect()
}

/// Distância em arestas a partir de um conjunto de fontes (u32::MAX = inalcançável).
pub fn bfs_dist(adj: &[Vec<u32>], sources: &[u32]) -> Vec<u32> {
    let mut d = vec![u32::MAX; adj.len()];
    let mut q = VecDeque::new();
    for &s in sources {
        if (s as usize) < d.len() && d[s as usize] == u32::MAX {
            d[s as usize] = 0;
            q.push_back(s);
        }
    }
    while let Some(v) = q.pop_front() {
        for &n in &adj[v as usize] {
            if d[n as usize] == u32::MAX {
                d[n as usize] = d[v as usize] + 1;
                q.push_back(n);
            }
        }
    }
    d
}

fn fall(d: u32, r: f32) -> f32 {
    if d == u32::MAX {
        return 0.0;
    }
    let t = 1.0 - d as f32 / r;
    let t = t.clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

pub fn hash_faces(faces: &[[u32; 4]], n_verts: usize) -> u64 {
    let mut h: u64 = 0xcbf29ce484222325;
    let mut feed = |x: u32| {
        for b in x.to_le_bytes() {
            h ^= b as u64;
            h = h.wrapping_mul(0x100000001b3);
        }
    };
    feed(n_verts as u32);
    feed(faces.len() as u32);
    for f in faces {
        for &v in f {
            feed(v);
        }
    }
    h
}

impl Topology {
    pub fn connectivity_hash(&self) -> u64 {
        hash_faces(&self.faces, self.n_verts)
    }

    pub fn loop_by_name(&self, name: &str) -> Option<&LoopDef> {
        self.loops.iter().find(|l| l.name == name)
    }

    pub fn mark(&self, name: &str) -> &[u32] {
        self.marks.get(name).map(|v| v.as_slice()).unwrap_or(&[])
    }

    pub fn landmark(&self, name: &str) -> Option<&Locator> {
        self.landmarks.iter().find(|(n, _)| n == name).map(|(_, l)| l)
    }

    /// Constrói a topologia a partir de uma gaiola e do seu refinamento (posições de referência).
    pub fn build(cage: &Cage, refined: &Refined) -> Result<Topology, String> {
        let n_verts = refined.mesh.pos.len();
        let ref_pos = &refined.mesh.pos;
        let faces = refined.mesh.faces.clone();
        let adj = build_adjacency(n_verts, &faces);

        // regiões e atributos propagados
        let region = refined.propagate(&cage.mesh, &cage.region, Region::merge);
        let attr = refined.propagate_avg4(&cage.mesh, &cage.attr);

        // laços expandidos
        let loops: Vec<LoopDef> = cage
            .loops
            .iter()
            .map(|l| LoopDef {
                name: l.name.clone(),
                verts: refined.expand_loop(&l.verts, l.closed),
                closed: l.closed,
            })
            .collect();
        let marks = cage.marks.clone();

        let mut t = Topology {
            faces,
            n_verts,
            n_cage: cage.mesh.pos.len(),
            region,
            attr,
            loops,
            marks,
            zones: Vec::new(),
            masks: BTreeMap::new(),
            landmarks: Vec::new(),
            uv: Vec::new(),
            chart_of_face: Vec::new(),
            mirror: Vec::new(),
            adjacency: adj,
            weights: Vec::new(),
            refined: RefinedInfo {
                edge_point: refined.edge_point.clone(),
                face_point: refined.face_point.clone(),
            },
            cage_faces: cage.mesh.faces.clone(),
        };
        t.mirror = compute_mirror(ref_pos)?;
        t.compute_zones_and_masks(cage, refined);
        t.landmarks = t.compute_landmarks();
        let (uv, charts) = crate::uv::compute_uvs(&t, ref_pos);
        t.uv = uv;
        t.chart_of_face = charts;
        Ok(t)
    }

    fn loop_ids(&self, name: &str) -> Vec<u32> {
        self.loop_by_name(name).map(|l| l.verts.clone()).unwrap_or_default()
    }

    fn compute_zones_and_masks(&mut self, cage: &Cage, refined: &Refined) {
        let n = self.n_verts;
        let adj_owned = self.adjacency.clone();
        let adj = &adj_owned;
        let mut zones = vec![[0.0f32; ZONE_COUNT]; n];
        let z = |a: AnatomicalZone| zone_index(a);

        let mut eye_src = self.loop_ids("eye_L_ring3");
        eye_src.extend(self.loop_ids("eye_R_ring3"));
        let d_eye = bfs_dist(adj, &eye_src);
        let mut brow_src = self.loop_ids("brow_line");
        let d_brow = bfs_dist(adj, &brow_src);
        let mut nose_src = self.loop_ids("nose_tip_ring");
        nose_src.extend(self.loop_ids("nose_alar_ring"));
        let d_nose = bfs_dist(adj, &nose_src);
        let mouth_src = self.loop_ids("mouth_ring3");
        let d_mouth = bfs_dist(adj, &mouth_src);
        let chin_src = self.loop_ids("chin_line");
        let d_chin = bfs_dist(adj, &chin_src);
        let mut ear_src = Vec::new();
        for s in ["L", "R"] {
            ear_src.extend(self.loop_ids(&format!("ear_{s}_ring3")));
            ear_src.extend(self.mark(&format!("ear_{s}_center")).to_vec());
        }
        let d_ear = bfs_dist(adj, &ear_src);
        let mut sh_src = self.loop_ids("shoulder_hole_L");
        sh_src.extend(self.loop_ids("shoulder_hole_R"));
        let d_sh = bfs_dist(adj, &sh_src);
        let mut bust_src = self.mark("bust_L").to_vec();
        bust_src.extend(self.mark("bust_R").to_vec());
        let d_bust = bfs_dist(adj, &bust_src);
        let crotch_src = self.loop_ids("crotch_loop");
        let d_crotch = bfs_dist(adj, &crotch_src);
        let _ = &mut brow_src;

        let tri = |x: f32, a: f32, b: f32, c: f32| -> f32 {
            // 0 em a, 1 em b, 0 em c
            if x <= a || x >= c {
                0.0
            } else if x < b {
                smoothstep(a, b, x)
            } else {
                1.0 - smoothstep(b, c, x)
            }
        };

        for v in 0..n {
            let r = self.region[v];
            let a = self.attr[v];
            let (row, sin_t, cos_t) = (a[0], a[1], a[2]);
            let zv = &mut zones[v];
            zv[z(AnatomicalZone::GlobalSilhouette)] = 1.0;
            let head = matches!(r, Region::Head);
            if head {
                zv[z(AnatomicalZone::Craniofacial)] = 1.0;
            } else if matches!(r, Region::Neck) {
                zv[z(AnatomicalZone::Craniofacial)] = 0.25;
            }
            zv[z(AnatomicalZone::Eyes)] = fall(d_eye[v], 9.0);
            zv[z(AnatomicalZone::Eyebrows)] = fall(d_brow[v], 5.0);
            zv[z(AnatomicalZone::Nose)] = fall(d_nose[v], 5.0);
            zv[z(AnatomicalZone::MouthLips)] = fall(d_mouth[v], 9.0);
            zv[z(AnatomicalZone::JawChin)] = if head { fall(d_chin[v], 8.0) } else { 0.0 };
            zv[z(AnatomicalZone::Ears)] = fall(d_ear[v], 5.0);
            let trunk_like = matches!(r, Region::Trunk | Region::Neck);
            let neck = if matches!(r, Region::Neck) {
                1.0
            } else if trunk_like {
                (1.0 - (row - 4.0) / 2.5).clamp(0.0, 1.0)
            } else {
                0.0
            };
            zv[z(AnatomicalZone::NeckTrapezius)] = neck;
            zv[z(AnatomicalZone::ShouldersClavicles)] = fall(d_sh[v], 7.0).max(if trunk_like && row >= 3.0 && row <= 5.0 { 0.6 } else { 0.0 });
            if trunk_like {
                let front = smoothstep(-0.1, 0.5, cos_t);
                let side_ok = 1.0 - smoothstep(0.78, 0.98, sin_t.abs());
                zv[z(AnatomicalZone::ChestPectorals)] = tri(row, 4.5, 8.0, 11.0) * front * side_ok;
                zv[z(AnatomicalZone::AbdomenWaist)] = tri(row, 8.5, 12.5, 16.0);
                zv[z(AnatomicalZone::PelvisHips)] = tri(row, 13.5, 16.5, 19.5).max(0.0);
                zv[z(AnatomicalZone::Gluteus)] = tri(row, 13.5, 16.5, 19.5) * (1.0 - smoothstep(-0.55, 0.05, cos_t));
            }
            zv[z(AnatomicalZone::BustFemale)] = fall(d_bust[v], 6.0) * if trunk_like { 1.0 } else { 0.0 };
            if matches!(r, Region::Leg) {
                let k = fall(d_crotch[v], 6.0);
                zv[z(AnatomicalZone::PelvisHips)] = zv[z(AnatomicalZone::PelvisHips)].max(k);
            }
            if matches!(r, Region::Arm) {
                zv[z(AnatomicalZone::UpperLimbs)] = 1.0;
            }
            if matches!(r, Region::Hand | Region::Thumb | Region::Index | Region::Middle | Region::Ring | Region::Little) {
                zv[z(AnatomicalZone::HandsFingers)] = 1.0;
            }
            if matches!(r, Region::Leg | Region::Foot | Region::Toes) {
                zv[z(AnatomicalZone::LowerLimbs)] = 1.0;
            }
        }
        self.zones = zones;

        // máscaras
        let mut masks: BTreeMap<String, Vec<f32>> = BTreeMap::new();
        let reg = |f: &dyn Fn(Region) -> bool| -> Vec<f32> { self.region.iter().map(|&r| if f(r) { 1.0 } else { 0.0 }).collect() };
        masks.insert("mask_hide_under_cloth_torso".into(), reg(&|r| matches!(r, Region::Trunk)));
        masks.insert("mask_hide_under_cloth_arms".into(), reg(&|r| matches!(r, Region::Arm)));
        masks.insert("mask_hide_under_cloth_legs".into(), reg(&|r| matches!(r, Region::Leg)));
        masks.insert("mask_hide_under_cloth_feet".into(), reg(&|r| matches!(r, Region::Foot | Region::Toes)));
        masks.insert(
            "mask_hide_under_cloth_hands".into(),
            reg(&|r| matches!(r, Region::Hand | Region::Thumb | Region::Index | Region::Middle | Region::Ring | Region::Little)),
        );
        let face_prox: Vec<f32> = (0..n)
            .map(|v| {
                if !matches!(self.region[v], Region::Head) {
                    return 0.0;
                }
                let d = d_eye[v].min(d_mouth[v]);
                fall(d, 18.0)
            })
            .collect();
        masks.insert("mask_face_normal_proxy".into(), face_prox);
        let outline: Vec<f32> = (0..n).map(|v| 1.0 - 0.7 * fall(d_eye[v].min(d_mouth[v]), 2.0)).collect();
        masks.insert("mask_outline_weight".into(), outline);
        masks.insert("mask_shadow_threshold_offset".into(), vec![0.0; n]);
        let sss: Vec<f32> = (0..n)
            .map(|v| match self.region[v] {
                Region::Head => 1.0,
                Region::Neck => 0.7,
                Region::Hand | Region::Thumb | Region::Index | Region::Middle | Region::Ring | Region::Little => 0.6,
                _ => 0.3,
            })
            .collect();
        masks.insert("mask_skin_sss".into(), sss);
        // cílios: metades superior/inferior da borda interna da pálpebra (anel 3 da gaiola)
        let mut upper = vec![0.0f32; cage.mesh.pos.len()];
        let mut lower = vec![0.0f32; cage.mesh.pos.len()];
        for s in ["L", "R"] {
            if let Some(l) = cage.loops.iter().find(|l| l.name == format!("eye_{s}_ring3")) {
                for (k, &v) in l.verts.iter().enumerate() {
                    if (5..=10).contains(&k) {
                        upper[v as usize] = 1.0;
                    } else {
                        lower[v as usize] = 1.0;
                    }
                }
            }
        }
        masks.insert("lash_upper".into(), refined.propagate_avg(&cage.mesh, &upper));
        masks.insert("lash_lower".into(), refined.propagate_avg(&cage.mesh, &lower));
        let mut scalp = vec![0.0f32; cage.mesh.pos.len()];
        if let Some(m) = cage.marks.get("hair_scalp") {
            for &v in m {
                scalp[v as usize] = 1.0;
            }
        }
        masks.insert("hair_scalp_region".into(), refined.propagate_avg(&cage.mesh, &scalp));
        let loop_mask = |names: &[&str]| -> Vec<f32> {
            let mut src = Vec::new();
            for nme in names {
                src.extend(self.loop_ids(nme));
            }
            let d = bfs_dist(adj, &src);
            d.iter().map(|&x| fall(x, 3.0)).collect()
        };
        masks.insert("cloth_attach_waist".into(), loop_mask(&["waist_loop"]));
        masks.insert("cloth_attach_shoulder".into(), loop_mask(&["shoulder_hole_L", "shoulder_hole_R"]));
        masks.insert("cloth_attach_neck".into(), loop_mask(&["neck_ring1"]));
        masks.insert("cloth_attach_wrist".into(), loop_mask(&["wrist_L_ring1", "wrist_R_ring1"]));
        masks.insert("cloth_attach_ankle".into(), loop_mask(&["ankle_L_ring1", "ankle_R_ring1"]));
        self.masks = masks;
    }

    fn compute_landmarks(&self) -> Vec<(String, Locator)> {
        let mut out: Vec<(String, Locator)> = Vec::new();
        let ring = |name: &str| -> Locator { Locator::centroid(self.mark(name)) };
        let c = |ids: &[u32]| -> Locator { Locator::centroid(ids) };
        for s in ["L", "R"] {
            out.push((format!("lm_eye_center_{s}"), c(&self.loop_ids(&format!("eye_{s}_ring3")))));
            let r3 = &self.marks.get(&format!("eye_{s}_center")).cloned().unwrap_or_default();
            // cantos: índices 4,5 (lado +u do contorno) e 10,11 (lado -u) do anel 3 da gaiola
            let cage_ring3: Vec<u32> = self
                .cage_loop_ring3(&format!("eye_{s}_ring3"))
                .unwrap_or_else(|| r3.clone());
            if cage_ring3.len() == 12 {
                let pick = |ix: &[usize]| -> Locator { c(&ix.iter().map(|&i| cage_ring3[i]).collect::<Vec<_>>()) };
                let (outer, inner) = if s == "L" { (pick(&[4, 5]), pick(&[10, 11])) } else { (pick(&[10, 11]), pick(&[4, 5])) };
                out.push((format!("lm_eye_outer_{s}"), outer));
                out.push((format!("lm_eye_inner_{s}"), inner));
                out.push((format!("lm_eye_upper_{s}"), pick(&[7, 8])));
                out.push((format!("lm_eye_lower_{s}"), pick(&[1, 2])));
            }
            out.push((format!("lm_ear_top_{s}"), c(&self.loop_ids(&format!("ear_{s}_ring2")))));
            out.push((format!("lm_ear_center_{s}"), c(self.mark(&format!("ear_{s}_center")))));
            out.push((format!("lm_shoulder_{s}"), ring(&format!("arm_{s}_hole"))));
            out.push((format!("lm_elbow_{s}"), ring(&format!("elbow_{s}"))));
            out.push((format!("lm_wrist_{s}"), ring(&format!("wrist_{s}"))));
            out.push((format!("lm_knee_{s}"), ring(&format!("knee_{s}"))));
            out.push((format!("lm_ankle_{s}"), ring(&format!("ankle_{s}"))));
            out.push((format!("lm_hip_{s}"), ring(&format!("thigh_top_{s}"))));
            out.push((format!("lm_bust_{s}"), ring(&format!("bust_{s}"))));
            out.push((format!("lm_heel_{s}"), ring(&format!("foot_{s}_heel"))));
            out.push((format!("lm_toe_{s}"), ring(&format!("foot_{s}_toetip"))));
        }
        if let Some(cr) = self.cage_loop_ring3("mouth_ring3") {
            if cr.len() == 14 {
                out.push(("lm_mouth_corner_R".into(), Locator::vertex(cr[13])));
                out.push(("lm_mouth_corner_L".into(), Locator::vertex(cr[6])));
                out.push(("lm_lip_upper".into(), c(&[cr[8], cr[9], cr[10]])));
                out.push(("lm_lip_lower".into(), c(&[cr[1], cr[2], cr[3]])));
            }
        }
        out.push(("lm_mouth_center".into(), ring("mouth_center")));
        out.push(("lm_nose_tip".into(), ring("nose_tip")));
        out.push(("lm_chin".into(), ring("chin")));
        out.push(("lm_head_top".into(), ring("head_top")));
        out.push(("lm_neck_base".into(), ring("neck_base_ring")));
        out.push(("lm_neck_top".into(), ring("neck_top_ring")));
        out.push(("lm_waist".into(), ring("spine_ring")));
        out.push(("lm_navel".into(), ring("navel")));
        out.push(("lm_hip_center".into(), ring("hip_ring")));
        out.push(("lm_chest".into(), ring("chest_ring")));
        out
    }

    /// Anel da gaiola (ids originais) com o nome dado, antes da expansão: derivado do laço final
    /// tomando só os vértices originais (índice < n_cage), na mesma ordem.
    fn cage_loop_ring3(&self, name: &str) -> Option<Vec<u32>> {
        let l = self.loop_by_name(name)?;
        Some(l.verts.iter().copied().filter(|&v| (v as usize) < self.n_cage).collect())
    }
}

fn compute_mirror(pos: &[Vec3]) -> Result<Vec<u32>, String> {
    // grade espacial com célula de 2 mm
    let cell = 0.002f32;
    let key = |p: Vec3| -> (i32, i32, i32) { ((p.x / cell).floor() as i32, (p.y / cell).floor() as i32, (p.z / cell).floor() as i32) };
    let mut grid: BTreeMap<(i32, i32, i32), Vec<u32>> = BTreeMap::new();
    for (i, p) in pos.iter().enumerate() {
        grid.entry(key(*p)).or_default().push(i as u32);
    }
    let mut mirror = vec![u32::MAX; pos.len()];
    for (i, p) in pos.iter().enumerate() {
        let m = Vec3::new(-p.x, p.y, p.z);
        let k = key(m);
        let mut best = (f32::MAX, u32::MAX);
        for dx in -1..=1 {
            for dy in -1..=1 {
                for dz in -1..=1 {
                    if let Some(c) = grid.get(&(k.0 + dx, k.1 + dy, k.2 + dz)) {
                        for &j in c {
                            let d = (pos[j as usize] - m).length();
                            if d < best.0 {
                                best = (d, j);
                            }
                        }
                    }
                }
            }
        }
        if best.1 == u32::MAX || best.0 > 1.5e-3 {
            return Err(format!("vértice {i} sem par de simetria (distância {})", best.0));
        }
        mirror[i] = best.1;
    }
    for i in 0..mirror.len() {
        if mirror[mirror[i] as usize] != i as u32 {
            return Err(format!("mapa de simetria não é involução em {i}"));
        }
    }
    Ok(mirror)
}

/// Força simetria exata em X: pares espelhados ficam com coordenadas exatamente opostas.
pub fn symmetrize(pos: &mut [Vec3], mirror: &[u32]) {
    for i in 0..pos.len() {
        let j = mirror[i] as usize;
        if j == i {
            pos[i].x = 0.0;
        } else if i < j {
            let a = pos[i];
            let b = pos[j];
            let x = 0.5 * (a.x - b.x);
            let y = 0.5 * (a.y + b.y);
            let z = 0.5 * (a.z + b.z);
            pos[i] = Vec3::new(x, y, z);
            pos[j] = Vec3::new(-x, y, z);
        }
    }
}

static REFERENCE: OnceLock<Topology> = OnceLock::new();

/// Parâmetros da geração de referência da topologia (neutro, gênero 0.5).
pub fn reference_params() -> Params {
    Params {
        gender: 0.5,
        ..Params::default()
    }
}

/// Topologia estática (construída uma vez por processo).
pub fn reference() -> &'static Topology {
    REFERENCE.get_or_init(|| {
        let cage = build_cage(&reference_params());
        let refined = catmull_clark(&cage.mesh);
        let mut pos = refined.mesh.pos.clone();
        // simetriza a referência antes de calcular o espelho
        let mirror = compute_mirror(&pos).expect("referência assimétrica");
        symmetrize(&mut pos, &mirror);
        let mut refined = refined;
        refined.mesh.pos = pos;
        let mut t = Topology::build(&cage, &refined).expect("topologia de referência");
        let (skel, ref_pos) = crate::skeleton::reference_skeleton(&t, &refined.mesh.pos);
        t.weights = crate::weights::compute_weights(&t, &ref_pos, &skel);
        t
    })
}

/// Aresta -> faces, útil para validação.
pub fn edge_face_map(faces: &[[u32; 4]]) -> BTreeMap<(u32, u32), Vec<u32>> {
    let mut m: BTreeMap<(u32, u32), Vec<u32>> = BTreeMap::new();
    for (fi, f) in faces.iter().enumerate() {
        for k in 0..4 {
            m.entry(edge_key(f[k], f[(k + 1) % 4])).or_default().push(fi as u32);
        }
    }
    m
}

pub fn quad_mesh_of(topo: &Topology, pos: &[Vec3]) -> QuadMesh {
    QuadMesh {
        pos: pos.to_vec(),
        faces: topo.faces.clone(),
    }
}

#[allow(dead_code)]
fn _unused(_: Vec2) {}
