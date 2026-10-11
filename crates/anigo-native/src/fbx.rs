//! Import de FBX com o **ufbx** (C, vendorizado pelo crate `ufbx`).
//!
//! [`import_fbx`] lê os bytes de um FBX (binário ou ASCII) e devolve um
//! [`FbxScene`] em tipos do ANIGO: malhas como [`anigo_modeling::EditMesh`]
//! (**n-gons preservados**), hierarquia de nós, materiais, skin, blend shapes e
//! clips de animação.
//!
//! # Conversão de espaço (documentada e testada)
//!
//! O ufbx faz a conversão ao carregar (`LoadOpts`); nada é refeito aqui. O modo é
//! `SpaceConversion::ModifyGeometry`: a **escala de unidade é embutida nos vértices,
//! normais, blend shapes e matrizes de skin**, e a **rotação do eixo "para cima"
//! vai para o TRS local dos nós** (pré-rotação dobrada em `FbxNode::rotation`) — o
//! mesmo desenho do glTF. Resultado no **espaço do mundo** (hierarquia × geometria):
//!
//! | aspecto        | resultado |
//! |----------------|-----------|
//! | eixos          | **Y para cima**, mão direita, +X direita, +Z para a frente (`axes_right_handed_y_up`, igual ao glTF). Arquivos Z-up ou mão esquerda são convertidos (`handedness_conversion_axis = X`). |
//! | unidade        | **metros** (`target_unit_meters = 1.0`): um FBX em cm vira ×0.01 (já nos vértices; nós sem escala 0.01 herdada). |
//! | onde           | geometria local: metros, orientação do arquivo; rotação de eixos no TRS dos nós. Para obter a malha já em Y-up use [`FbxScene::mesh_in_world_space`] (ou componha [`FbxScene::node_world_matrix`]). Arquivos já Y-up/metros não têm rotação nos nós. |
//! | enrolamento    | **anti-horário visto de fora** = normal para fora (igual a `EditMesh`/glTF). Ao espelhar a mão, o ufbx inverte o enrolamento das faces para manter a normal para fora. |
//! | transform de geometria / pivôs | embutidos nos vértices (`GeometryTransformHandling::ModifyGeometry`; nós auxiliares só quando uma malha instanciada exige). |
//! | herança de escala | compensada (`InheritModeHandling::Compensate`), então a hierarquia é TRS puro. |
//! | UV             | origem embaixo-esquerda, V para cima (convenção FBX/Blender), **sem flip**. |
//! | normais        | por corner → `Corner::normal` (só se o arquivo as tem; senão `None` = calculadas). |
//! | vértices       | um `EVert` por *control point* do FBX; faces preservadas com todos os corners. |
//! | skin           | até 4 influências por vértice (as maiores, renormalizadas) em `EVert::joints/weights`; o índice de joint é a posição em [`FbxSkin::joints`]. |
//! | cor de vértice | não importada (o canal `EVert::color` é o contrato NPR do ANIGO). |
//!
//! # Segurança
//!
//! * tamanho máximo de entrada, limite de memória e profundidade de nós passados ao ufbx;
//! * toda lista lida do ufbx é consultada com `get()` (nada de indexação cega);
//! * a chamada inteira roda sob `catch_unwind` (`NativeError::Panic`).

use crate::{guarded, NativeError, NativeResult};
use anigo_modeling::{Corner, EVert, EditMesh, Face};
use glam::{Mat4, Quat, Vec2, Vec3};

const LIB: &str = "ufbx";

/// Opções de [`import_fbx_with`].
#[derive(Debug, Clone, PartialEq)]
pub struct FbxImportOptions {
    /// Importa clips de animação (bake em amostras TRS por nó).
    pub import_animations: bool,
    /// Importa blend shapes (shape keys).
    pub import_blend_shapes: bool,
    /// Importa pesos de skin (top-4) e a lista de joints.
    pub import_skin: bool,
    /// Taxa de amostragem do bake de animação (quadros por segundo).
    pub animation_sample_rate: f64,
    /// Tamanho máximo do arquivo de entrada, em bytes.
    pub max_input_bytes: usize,
    /// Memória máxima que o ufbx pode alocar (temporária e de resultado), em bytes.
    pub max_memory_bytes: usize,
    /// Profundidade máxima da hierarquia de nós.
    pub max_node_depth: u32,
    /// Total máximo de chaves de animação baked (translação+rotação+escala).
    pub max_animation_keys: usize,
}

impl Default for FbxImportOptions {
    fn default() -> Self {
        Self {
            import_animations: true,
            import_blend_shapes: true,
            import_skin: true,
            animation_sample_rate: 30.0,
            max_input_bytes: 512 * 1024 * 1024,
            max_memory_bytes: 2 * 1024 * 1024 * 1024,
            max_node_depth: 256,
            max_animation_keys: 5_000_000,
        }
    }
}

/// Informações do arquivo de origem.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct FbxSourceInfo {
    /// Programa que gerou o arquivo (campo `Creator`), se houver.
    pub creator: String,
    /// Versão do formato (ex.: 7400).
    pub fbx_version: u32,
    pub ascii: bool,
    /// Eixo "para cima" do arquivo ANTES da conversão (ex.: `PositiveZ`).
    pub original_up_axis: String,
    /// Metros por unidade do arquivo ANTES da conversão (cm = 0.01).
    pub original_unit_meters: f64,
    /// Quadros por segundo da cena.
    pub frames_per_second: f64,
    /// Avisos do ufbx (dados ausentes, índices corrigidos…), no máximo 32.
    pub warnings: Vec<String>,
    /// Faces com menos de 3 corners (pontos/linhas) ou degeneradas descartadas.
    pub dropped_faces: usize,
}

/// Material do FBX (parâmetros básicos; o ANIGO mapeia para seu material NPR depois).
#[derive(Debug, Clone, PartialEq)]
pub struct FbxMaterial {
    pub name: String,
    /// Cor base RGBA linear-ish como no arquivo (`[1,1,1,1]` se ausente).
    pub base_color: [f32; 4],
    /// Arquivo da textura de cor base, como escrito no FBX (relativo se houver).
    pub base_color_texture: Option<String>,
}

/// Delta esparso de um vértice de um blend shape.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct FbxShapeDelta {
    /// Índice do vértice em `FbxMesh::mesh.verts`.
    pub vertex: u32,
    /// Deslocamento de posição (já convertido para Y-up, metros).
    pub position: Vec3,
    /// Deslocamento de normal (zero se o shape não tem normais).
    pub normal: Vec3,
}

/// Blend shape / shape key simples: nome + deltas esparsos.
#[derive(Debug, Clone, PartialEq)]
pub struct FbxBlendShape {
    pub name: String,
    /// Peso inicial do canal, em `[0, 1]` (FBX usa 0–100; o ufbx normaliza).
    pub default_weight: f32,
    pub deltas: Vec<FbxShapeDelta>,
}

/// Um joint de skin (cluster do FBX).
#[derive(Debug, Clone, PartialEq)]
pub struct FbxJoint {
    /// Índice em [`FbxScene::nodes`] do osso (`None` se o cluster não aponta para um nó).
    pub node: Option<usize>,
    pub name: String,
    /// Matriz espaço-da-geometria-da-malha → espaço-do-osso (bind pose inversa).
    pub inverse_bind: Mat4,
    /// Matriz espaço-do-osso (bind) → mundo, na pose de bind.
    pub bind_to_world: Mat4,
}

/// Skin de uma malha. `EVert::joints[k]` indexa `joints`.
#[derive(Debug, Clone, PartialEq)]
pub struct FbxSkin {
    /// Índice em [`FbxScene::meshes`].
    pub mesh: usize,
    pub joints: Vec<FbxJoint>,
}

/// Malha importada.
#[derive(Debug, Clone, PartialEq)]
pub struct FbxMesh {
    pub name: String,
    /// Geometria editável: n-gons, UV e normais por corner, material por face.
    pub mesh: EditMesh,
    /// Slot de material (`Face::material`) → índice em [`FbxScene::materials`].
    pub material_slots: Vec<usize>,
    /// Índice em [`FbxScene::skins`] se a malha tem skin.
    pub skin: Option<usize>,
    pub blend_shapes: Vec<FbxBlendShape>,
}

/// Nó da hierarquia (sem o nó raiz sintético do ufbx).
#[derive(Debug, Clone, PartialEq)]
pub struct FbxNode {
    pub name: String,
    pub parent: Option<usize>,
    pub children: Vec<usize>,
    /// Transform local (TRS) já em Y-up/metros.
    pub translation: Vec3,
    pub rotation: Quat,
    pub scale: Vec3,
    /// Índice em [`FbxScene::meshes`].
    pub mesh: Option<usize>,
    /// É um osso (atributo Skeleton do FBX ou alvo de skin).
    pub is_bone: bool,
    pub visible: bool,
}

impl FbxNode {
    /// Transform local como matriz.
    pub fn local_matrix(&self) -> Mat4 {
        Mat4::from_scale_rotation_translation(self.scale, self.rotation, self.translation)
    }
}

/// Amostras de animação de um nó.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct FbxTrack {
    /// Índice em [`FbxScene::nodes`].
    pub node: usize,
    /// `(tempo em s desde o início do clip, valor)`.
    pub translation: Vec<(f32, Vec3)>,
    pub rotation: Vec<(f32, Quat)>,
    pub scale: Vec<(f32, Vec3)>,
}

/// Clip de animação (um `AnimStack` do FBX), com bake TRS por nó.
#[derive(Debug, Clone, PartialEq)]
pub struct FbxClip {
    pub name: String,
    /// Duração em segundos.
    pub duration: f32,
    /// Taxa de amostragem usada no bake.
    pub sample_rate: f64,
    pub tracks: Vec<FbxTrack>,
}

/// Cena importada.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct FbxScene {
    pub info: FbxSourceInfo,
    pub nodes: Vec<FbxNode>,
    /// Nós sem pai.
    pub roots: Vec<usize>,
    pub meshes: Vec<FbxMesh>,
    pub materials: Vec<FbxMaterial>,
    pub skins: Vec<FbxSkin>,
    pub clips: Vec<FbxClip>,
}

impl FbxScene {
    /// Matriz local→mundo de um nó (produto dos TRS locais até a raiz).
    /// `None` se o índice é inválido ou a hierarquia tem ciclo.
    pub fn node_world_matrix(&self, node: usize) -> Option<Mat4> {
        let mut m = Mat4::IDENTITY;
        let mut cur = Some(node);
        let mut steps = 0usize;
        while let Some(i) = cur {
            let n = self.nodes.get(i)?;
            m = n.local_matrix() * m;
            cur = n.parent;
            steps += 1;
            if steps > self.nodes.len() {
                return None; // ciclo
            }
        }
        Some(m)
    }

    /// Malha do nó `node` transformada para o **espaço do mundo** (Y-up, metros):
    /// posições por `M`, normais por corner pela inversa-transposta, e enrolamento
    /// invertido se `M` espelha (det < 0), mantendo as normais para fora.
    /// `None` se o nó não tem malha ou a matriz não é invertível.
    pub fn mesh_in_world_space(&self, node: usize) -> Option<EditMesh> {
        let mesh_id = self.nodes.get(node)?.mesh?;
        let mut mesh = self.meshes.get(mesh_id)?.mesh.clone();
        let m = self.node_world_matrix(node)?;
        let det = m.determinant();
        if !det.is_finite() || det.abs() < 1e-12 {
            return None;
        }
        let normal_m = m.inverse().transpose();
        for v in &mut mesh.verts {
            v.co = m.transform_point3(v.co);
        }
        let mirrored = det < 0.0;
        for f in &mut mesh.faces {
            for c in &mut f.corners {
                if let Some(n) = c.normal {
                    c.normal = normal_m.transform_vector3(n).try_normalize();
                }
            }
            if mirrored {
                f.corners.reverse();
            }
        }
        Some(mesh)
    }
}

/// Importa um FBX (binário ou ASCII) com as opções padrão.
pub fn import_fbx(bytes: &[u8]) -> NativeResult<FbxScene> {
    import_fbx_with(bytes, &FbxImportOptions::default())
}

/// Importa um FBX. Bytes vazios, truncados ou que não são FBX devolvem `Err`.
pub fn import_fbx_with(bytes: &[u8], opts: &FbxImportOptions) -> NativeResult<FbxScene> {
    guarded(LIB, || import_inner(bytes, opts))
}

fn backend(message: impl Into<String>) -> NativeError {
    NativeError::Backend {
        lib: LIB,
        message: message.into(),
    }
}

fn v3(v: ufbx::Vec3) -> Vec3 {
    Vec3::new(v.x as f32, v.y as f32, v.z as f32)
}

fn q4(q: ufbx::Quat) -> Quat {
    let r = Quat::from_xyzw(q.x as f32, q.y as f32, q.z as f32, q.w as f32);
    if r.is_finite() && r.length_squared() > 0.0 {
        r.normalize()
    } else {
        Quat::IDENTITY
    }
}

/// Matriz 3×4 do ufbx (colunas = vetores-base + translação) → `Mat4`.
fn m4(m: &ufbx::Matrix) -> Mat4 {
    Mat4::from_cols_array(&[
        m.m00 as f32,
        m.m10 as f32,
        m.m20 as f32,
        0.0,
        m.m01 as f32,
        m.m11 as f32,
        m.m21 as f32,
        0.0,
        m.m02 as f32,
        m.m12 as f32,
        m.m22 as f32,
        0.0,
        m.m03 as f32,
        m.m13 as f32,
        m.m23 as f32,
        1.0,
    ])
}

fn import_inner(bytes: &[u8], opts: &FbxImportOptions) -> NativeResult<FbxScene> {
    if bytes.is_empty() {
        return Err(NativeError::EmptyInput("arquivo FBX vazio"));
    }
    if bytes.len() > opts.max_input_bytes {
        return Err(NativeError::LimitExceeded {
            what: "tamanho do FBX (bytes)",
            got: bytes.len() as u64,
            max: opts.max_input_bytes as u64,
        });
    }
    if !opts.animation_sample_rate.is_finite()
        || !(1.0..=480.0).contains(&opts.animation_sample_rate)
    {
        return Err(NativeError::InvalidInput(format!(
            "animation_sample_rate deve estar em [1, 480]; veio {}",
            opts.animation_sample_rate
        )));
    }

    let mut load = ufbx::LoadOpts {
        ignore_animation: !opts.import_animations,
        // Conversão de espaço: Y-up, mão direita, metros, embutida na geometria.
        target_axes: ufbx::CoordinateAxes::right_handed_y_up(),
        target_unit_meters: 1.0,
        space_conversion: ufbx::SpaceConversion::ModifyGeometry,
        handedness_conversion_axis: ufbx::MirrorAxis::X,
        geometry_transform_handling: ufbx::GeometryTransformHandling::ModifyGeometry,
        inherit_mode_handling: ufbx::InheritModeHandling::Compensate,
        clean_skin_weights: true,
        node_depth_limit: opts.max_node_depth,
        file_size_estimate: bytes.len() as u64,
        // Não segue caminhos externos (texturas/OBJ-MTL): só o que está nos bytes.
        load_external_files: false,
        ignore_missing_external_files: true,
        ..Default::default()
    };
    load.temp_allocator.memory_limit = opts.max_memory_bytes;
    load.result_allocator.memory_limit = opts.max_memory_bytes;
    load.temp_allocator.allocation_limit = 200_000_000;
    load.result_allocator.allocation_limit = 200_000_000;

    let root = ufbx::load_memory(bytes, load)
        .map_err(|e| backend(format!("{:?}: {}", e.type_, &*e.description)))?;
    let scene: &ufbx::Scene = &root;

    let mut out = FbxScene {
        info: FbxSourceInfo {
            creator: scene.metadata.creator.to_string(),
            fbx_version: scene.metadata.version,
            ascii: scene.metadata.ascii,
            original_up_axis: format!("{:?}", scene.settings.original_axis_up),
            original_unit_meters: scene.settings.original_unit_meters,
            frames_per_second: scene.settings.frames_per_second,
            warnings: scene
                .metadata
                .warnings
                .iter()
                .take(32)
                .map(|w| w.description.to_string())
                .collect(),
            dropped_faces: 0,
        },
        ..FbxScene::default()
    };

    // ---- nós (sem o nó raiz sintético)
    let mut node_index: Vec<Option<usize>> = vec![None; scene.nodes.len()];
    for node in scene.nodes.iter() {
        if node.is_root {
            continue;
        }
        let tid = node.element.typed_id as usize;
        if tid >= node_index.len() {
            return Err(backend("typed_id de nó fora de faixa"));
        }
        node_index[tid] = Some(out.nodes.len());
        let t = &node.local_transform;
        out.nodes.push(FbxNode {
            name: node.element.name.to_string(),
            parent: None,
            children: Vec::new(),
            translation: v3(t.translation),
            rotation: q4(t.rotation),
            scale: v3(t.scale),
            mesh: None,
            is_bone: node.bone.is_some(),
            visible: node.visible,
        });
    }
    let lookup_node = |n: &ufbx::Node| -> Option<usize> {
        node_index
            .get(n.element.typed_id as usize)
            .copied()
            .flatten()
    };
    for node in scene.nodes.iter() {
        let Some(me) = lookup_node(node) else {
            continue;
        };
        if let Some(parent) = node.parent.as_ref().and_then(|p| lookup_node(p)) {
            out.nodes[me].parent = Some(parent);
            out.nodes[parent].children.push(me);
        }
    }
    out.roots = out
        .nodes
        .iter()
        .enumerate()
        .filter(|(_, n)| n.parent.is_none())
        .map(|(i, _)| i)
        .collect();

    // ---- materiais
    for mat in scene.materials.iter() {
        let c = &mat.pbr.base_color;
        let base_color = if c.has_value {
            [
                c.value_vec4.x as f32,
                c.value_vec4.y as f32,
                c.value_vec4.z as f32,
                if c.value_components >= 4 {
                    c.value_vec4.w as f32
                } else {
                    1.0
                },
            ]
        } else {
            [1.0, 1.0, 1.0, 1.0]
        };
        let base_color_texture = c.texture.as_ref().map(|t| {
            let rel = t.relative_filename.to_string();
            if rel.is_empty() {
                t.filename.to_string()
            } else {
                rel
            }
        });
        out.materials.push(FbxMaterial {
            name: mat.element.name.to_string(),
            base_color,
            base_color_texture,
        });
    }

    // ---- malhas
    let mut mesh_index: Vec<Option<usize>> = vec![None; scene.meshes.len()];
    for (mi, m) in scene.meshes.iter().enumerate() {
        let (fbx_mesh, dropped) = convert_mesh(m, scene, opts)?;
        out.info.dropped_faces += dropped;
        mesh_index[mi] = Some(out.meshes.len());
        out.meshes.push(fbx_mesh);
    }
    for node in scene.nodes.iter() {
        let Some(me) = lookup_node(node) else {
            continue;
        };
        if let Some(m) = node.mesh.as_ref() {
            out.nodes[me].mesh = mesh_index
                .get(m.element.typed_id as usize)
                .copied()
                .flatten();
        }
    }

    // ---- skins (precisam dos índices de nó) e flags de osso
    if opts.import_skin {
        for (mi, m) in scene.meshes.iter().enumerate() {
            let Some(my_mesh) = mesh_index[mi] else {
                continue;
            };
            let Some(deformer) = m.skin_deformers.first() else {
                continue;
            };
            let mut joints = Vec::with_capacity(deformer.clusters.len());
            for cluster in deformer.clusters.iter() {
                let node = cluster.bone_node.as_ref().and_then(|n| lookup_node(n));
                if let Some(n) = node {
                    out.nodes[n].is_bone = true;
                }
                joints.push(FbxJoint {
                    node,
                    name: cluster.element.name.to_string(),
                    inverse_bind: m4(&cluster.geometry_to_bone),
                    bind_to_world: m4(&cluster.bind_to_world),
                });
            }
            if joints.len() > u16::MAX as usize {
                return Err(NativeError::LimitExceeded {
                    what: "joints por skin",
                    got: joints.len() as u64,
                    max: u16::MAX as u64,
                });
            }
            let skin_id = out.skins.len();
            apply_skin_weights(&mut out.meshes[my_mesh].mesh, deformer)?;
            out.meshes[my_mesh].skin = Some(skin_id);
            out.skins.push(FbxSkin {
                mesh: my_mesh,
                joints,
            });
        }
    }

    // ---- animação
    if opts.import_animations {
        for stack in scene.anim_stacks.iter() {
            let bake_opts = ufbx::BakeOpts {
                resample_rate: opts.animation_sample_rate,
                trim_start_time: false,
                ..Default::default()
            };
            let baked = ufbx::bake_anim(scene, &stack.anim, bake_opts)
                .map_err(|e| backend(format!("bake de animação: {:?}", e.type_)))?;
            let start = baked.playback_time_begin;
            let mut tracks = Vec::new();
            let mut total_keys = 0usize;
            for bn in baked.nodes.iter() {
                let Some(node) = node_index.get(bn.typed_id as usize).copied().flatten() else {
                    continue;
                };
                total_keys +=
                    bn.translation_keys.len() + bn.rotation_keys.len() + bn.scale_keys.len();
                if total_keys > opts.max_animation_keys {
                    return Err(NativeError::LimitExceeded {
                        what: "chaves de animação",
                        got: total_keys as u64,
                        max: opts.max_animation_keys as u64,
                    });
                }
                let rel = |t: f64| (t - start) as f32;
                tracks.push(FbxTrack {
                    node,
                    translation: bn
                        .translation_keys
                        .iter()
                        .map(|k| (rel(k.time), v3(k.value)))
                        .collect(),
                    rotation: bn
                        .rotation_keys
                        .iter()
                        .map(|k| (rel(k.time), q4(k.value)))
                        .collect(),
                    scale: bn
                        .scale_keys
                        .iter()
                        .map(|k| (rel(k.time), v3(k.value)))
                        .collect(),
                });
            }
            if tracks.is_empty() {
                continue;
            }
            out.clips.push(FbxClip {
                name: stack.element.name.to_string(),
                duration: baked.playback_duration.max(0.0) as f32,
                sample_rate: opts.animation_sample_rate,
                tracks,
            });
        }
    }

    Ok(out)
}

/// Lê atributo por corner com checagem de faixa (`indices[corner]` → `values[i]`).
fn corner_attr<T: Copy>(exists: bool, indices: &[u32], values: &[T], corner: usize) -> Option<T> {
    if !exists {
        return None;
    }
    let i = *indices.get(corner)? as usize;
    values.get(i).copied()
}

fn convert_mesh(
    m: &ufbx::Mesh,
    scene: &ufbx::Scene,
    opts: &FbxImportOptions,
) -> NativeResult<(FbxMesh, usize)> {
    let mut em = EditMesh::new(m.element.name.to_string());
    for p in m.vertices.iter() {
        let co = v3(*p);
        if !co.is_finite() {
            return Err(backend("vértice com coordenada não finita"));
        }
        em.verts.push(EVert::new(co));
    }
    let vcount = em.verts.len();
    let has_uv = m.vertex_uv.exists;
    let has_normal = m.vertex_normal.exists;
    let mut dropped = 0usize;

    for (fi, face) in m.faces.iter().enumerate() {
        let begin = face.index_begin as usize;
        let n = face.num_indices as usize;
        let Some(vis) = m.vertex_indices.get(begin..begin + n) else {
            return Err(backend("face referencia corners fora de faixa"));
        };
        let mut corners: Vec<Corner> = Vec::with_capacity(n);
        for (k, &vi) in vis.iter().enumerate() {
            if vi as usize >= vcount {
                return Err(backend("índice de vértice fora de faixa"));
            }
            let ci = begin + k;
            let uv = corner_attr(has_uv, &m.vertex_uv.indices, &m.vertex_uv.values, ci)
                .map(|u| Vec2::new(u.x as f32, u.y as f32))
                .unwrap_or(Vec2::ZERO);
            let normal = corner_attr(
                has_normal,
                &m.vertex_normal.indices,
                &m.vertex_normal.values,
                ci,
            )
            .map(v3)
            .filter(|v| v.is_finite())
            .and_then(|v| v.try_normalize());
            // descarta corner repetido consecutivo (polígono degenerado)
            if corners.last().is_some_and(|c| c.vert == vi) {
                continue;
            }
            corners.push(Corner {
                vert: vi,
                uv,
                normal,
            });
        }
        if corners.len() > 1 && corners.first().map(|c| c.vert) == corners.last().map(|c| c.vert) {
            corners.pop();
        }
        if corners.len() < 3 {
            dropped += 1;
            continue;
        }
        let material = m
            .face_material
            .get(fi)
            .copied()
            .unwrap_or(0)
            .min(u16::MAX as u32) as u16;
        let smooth = m.face_smoothing.get(fi).copied().unwrap_or(true);
        em.faces.push(Face {
            corners,
            material,
            smooth,
            face_set: 0,
            hidden: false,
        });
    }

    // arestas: vinco e dureza (índices de aresta do ufbx são índices de corner)
    for (ei, e) in m.edges.iter().enumerate() {
        let (Some(&a), Some(&b)) = (
            m.vertex_indices.get(e.a as usize),
            m.vertex_indices.get(e.b as usize),
        ) else {
            continue;
        };
        if a == b || a as usize >= vcount || b as usize >= vcount {
            continue;
        }
        if let Some(&c) = m.edge_crease.get(ei) {
            if c > 0.0 && c.is_finite() {
                em.set_crease(a, b, (c as f32).clamp(0.0, 1.0));
            }
        }
        if m.edge_smoothing.get(ei) == Some(&false) {
            em.set_sharp(a, b, true);
        }
    }

    // slots de material → índices na cena
    let material_slots: Vec<usize> = m
        .materials
        .iter()
        .map(|mat| mat.element.typed_id as usize)
        .filter(|&i| i < scene.materials.len())
        .collect();

    // blend shapes
    let mut blend_shapes = Vec::new();
    if opts.import_blend_shapes {
        for deformer in m.blend_deformers.iter() {
            for channel in deformer.channels.iter() {
                let shape = channel
                    .target_shape
                    .as_ref()
                    .or_else(|| channel.keyframes.last().map(|k| &k.shape));
                let Some(shape) = shape else { continue };
                let n = shape.num_offsets.min(shape.offset_vertices.len());
                let mut deltas = Vec::with_capacity(n);
                for i in 0..n {
                    let vertex = shape.offset_vertices[i];
                    if vertex as usize >= vcount {
                        return Err(backend("blend shape referencia vértice fora de faixa"));
                    }
                    let Some(pos) = shape.position_offsets.get(i) else {
                        break;
                    };
                    let normal = shape
                        .normal_offsets
                        .get(i)
                        .map(|n| v3(*n))
                        .unwrap_or(Vec3::ZERO);
                    deltas.push(FbxShapeDelta {
                        vertex,
                        position: v3(*pos),
                        normal,
                    });
                }
                let name = if channel.element.name.is_empty() {
                    shape.element.name.to_string()
                } else {
                    channel.element.name.to_string()
                };
                blend_shapes.push(FbxBlendShape {
                    name,
                    default_weight: channel.weight as f32,
                    deltas,
                });
            }
        }
    }

    Ok((
        FbxMesh {
            name: m.element.name.to_string(),
            mesh: em,
            material_slots,
            skin: None,
            blend_shapes,
        },
        dropped,
    ))
}

/// Top-4 influências por vértice (ordenadas por peso desc., renormalizadas).
fn apply_skin_weights(em: &mut EditMesh, deformer: &ufbx::SkinDeformer) -> NativeResult<()> {
    let ncl = deformer.clusters.len();
    for (vi, vert) in em.verts.iter_mut().enumerate() {
        let Some(sv) = deformer.vertices.get(vi) else {
            continue;
        };
        let begin = sv.weight_begin as usize;
        let count = sv.num_weights as usize;
        let Some(ws) = deformer.weights.get(begin..begin + count) else {
            return Err(backend("pesos de skin fora de faixa"));
        };
        // o ufbx já ordena por peso decrescente; reordena por garantia e desempata por índice
        let mut top: Vec<(u32, f32)> = ws
            .iter()
            .filter(|w| (w.cluster_index as usize) < ncl && w.weight.is_finite() && w.weight > 0.0)
            .map(|w| (w.cluster_index, w.weight as f32))
            .collect();
        top.sort_by(|a, b| b.1.total_cmp(&a.1).then(a.0.cmp(&b.0)));
        top.truncate(4);
        let sum: f32 = top.iter().map(|t| t.1).sum();
        if sum > 0.0 {
            let mut joints = [0u16; 4];
            let mut weights = [0.0f32; 4];
            for (k, (j, w)) in top.iter().enumerate() {
                joints[k] = *j as u16; // ncl <= u16::MAX conferido pelo chamador
                weights[k] = w / sum;
            }
            vert.joints = joints;
            vert.weights = weights;
        }
    }
    Ok(())
}

#[cfg(test)]
#[path = "fbx_tests.rs"]
mod tests;
