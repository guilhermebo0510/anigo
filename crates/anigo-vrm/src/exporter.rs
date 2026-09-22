//! ANIGO — exportador determinístico glTF 2.0 / GLB / VRM 1.0 (lado Rust,
//! espelho de `src/services/vrm/exporter.ts`).
//!
//! Determinístico = a MESMA cena produz BYTES idênticos em exportações
//! sucessivas (critério de aceitação #25.3). O JSON sai com `serde_json`
//! (chaves ordenadas alfabeticamente — ordem estável entre runs); os números
//! são quantizados para f32; o binário usa packing canônico (uma bufferView
//! por accessor, padding 4 B) e o GLB usa padding fixo (JSON 0x20, BIN 0x00).

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

use crate::gltf::build_glb;
use crate::vrm1::Vrm1Meta;

// ---------------------------------------------------------------------------
// Modelo de exportação (cena normalizada do ANIGO)
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ExportVertex {
    pub position: [f32; 3],
    pub normal: [f32; 3],
    pub uv: [f32; 2],
    pub color: [f32; 4],
    pub joints: [u16; 4],
    pub weights: [f32; 4],
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ExportMorphDelta {
    pub position: [f32; 3],
    pub normal: [f32; 3],
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ExportPrimitive {
    pub vertices: Vec<ExportVertex>,
    pub indices: Vec<u32>,
    pub material_index: i64,
    pub morph_targets: Vec<Vec<ExportMorphDelta>>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ExportMesh {
    pub name: Option<String>,
    pub primitives: Vec<ExportPrimitive>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ExportMToon {
    pub shadow_color: Option<[f32; 4]>,
    pub shade_shift: Option<f32>,
    pub shade_toony: Option<f32>,
    pub rim_color: Option<[f32; 4]>,
    pub rim_power: Option<f32>,
    pub rim_light: Option<bool>,
    pub specular_color: Option<[f32; 4]>,
    pub specular_power: Option<f32>,
    pub sub_emission: Option<[f32; 4]>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ExportMaterial {
    pub name: Option<String>,
    pub base_color_factor: [f32; 4],
    pub metallic_factor: Option<f32>,
    pub roughness_factor: Option<f32>,
    pub emissive_factor: Option<[f32; 3]>,
    pub alpha_mode: Option<String>,
    pub double_sided: Option<bool>,
    pub mtoon: Option<ExportMToon>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ExportNode {
    pub name: Option<String>,
    pub translation: Option<[f32; 3]>,
    pub rotation: Option<[f32; 4]>,
    pub scale: Option<[f32; 3]>,
    pub children: Vec<u64>,
    pub mesh_index: Option<u64>,
    pub skin_index: Option<u64>,
    pub morph_weights: Option<Vec<f32>>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ExportSkin {
    pub name: Option<String>,
    pub joints: Vec<u64>,
    /// 16 floats column-major por osso.
    pub inverse_bind_matrices: Vec<[f32; 16]>,
    pub skeleton: Option<u64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExportAnimationTrack {
    pub node: u64,
    pub path: String,
    pub times: Vec<f32>,
    pub values: Vec<f32>,
    pub step: bool,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ExportAnimation {
    pub name: Option<String>,
    pub tracks: Vec<ExportAnimationTrack>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ExportScene {
    pub name: Option<String>,
    pub root_nodes: Vec<u64>,
    pub nodes: Vec<ExportNode>,
    pub meshes: Vec<ExportMesh>,
    pub skins: Vec<ExportSkin>,
    pub materials: Vec<ExportMaterial>,
    pub animations: Vec<ExportAnimation>,
}

/// Camada VRM 1.0 do export (meta + humanoid + expressões + molas).
pub struct VrmExportData {
    pub meta: Vrm1Meta,
    /// bone name → índice do nó (None = ausente).
    pub humanoid_bones: Vec<(String, Option<u64>)>,
    /// Os 16 presets (name → índice de morph target).
    pub expression_presets: Vec<(String, u64)>,
    pub expression_custom: Vec<(String, u64)>,
    pub spring_bone: Option<crate::vrm1::Vrm1SpringBone>,
}

// ---------------------------------------------------------------------------
// Packing canônico do buffer binário
// ---------------------------------------------------------------------------

struct BinWriter {
    bytes: Vec<u8>,
}

impl BinWriter {
    fn new() -> Self {
        Self { bytes: Vec::new() }
    }

    fn offset(&self) -> usize {
        self.bytes.len()
    }

    fn push(&mut self, chunk: &[u8]) {
        self.bytes.extend_from_slice(chunk);
        let pad = (4 - (self.bytes.len() % 4)) % 4;
        self.bytes.extend(std::iter::repeat(0u8).take(pad));
    }

    fn write_f32(&mut self, values: &[f32]) -> usize {
        let offset = self.offset();
        let mut packed = Vec::with_capacity(values.len() * 4);
        for v in values {
            packed.extend_from_slice(&v.to_le_bytes());
        }
        self.push(&packed);
        offset
    }

    fn write_u16(&mut self, values: &[u16]) -> usize {
        let offset = self.offset();
        let mut packed = Vec::with_capacity(values.len() * 2);
        for v in values {
            packed.extend_from_slice(&v.to_le_bytes());
        }
        self.push(&packed);
        offset
    }

    fn write_u32(&mut self, values: &[u32]) -> usize {
        let offset = self.offset();
        let mut packed = Vec::with_capacity(values.len() * 4);
        for v in values {
            packed.extend_from_slice(&v.to_le_bytes());
        }
        self.push(&packed);
        offset
    }
}

// ---------------------------------------------------------------------------
// Exportador
// ---------------------------------------------------------------------------

/// Exporta a cena normalizada para (JSON glTF, bin). Com `vrm`, acrescenta as
/// extensões VRM 1.0.
pub fn export_gltf(scene: &ExportScene, vrm: Option<&VrmExportData>) -> (Value, Vec<u8>) {
    let mut bin = BinWriter::new();
    let mut buffer_views: Vec<Value> = Vec::new();
    let mut accessors: Vec<Value> = Vec::new();

    let push_view = |_bin: &mut BinWriter, buffer_views: &mut Vec<Value>, offset: usize, byte_length: usize, target: Option<u64>| -> u64 {
        let mut entry = Map::new();
        entry.insert("buffer".into(), Value::from(0));
        entry.insert("byteOffset".into(), Value::from(offset));
        entry.insert("byteLength".into(), Value::from(byte_length));
        if let Some(target) = target {
            entry.insert("target".into(), Value::from(target));
        }
        buffer_views.push(Value::Object(entry));
        (buffer_views.len() - 1) as u64
    };

    let push_accessor = |accessors: &mut Vec<Value>, buffer_view: u64, component_type: u64, count: u64, tpe: &str| -> u64 {
        let mut entry = Map::new();
        entry.insert("bufferView".into(), Value::from(buffer_view));
        entry.insert("componentType".into(), Value::from(component_type));
        entry.insert("count".into(), Value::from(count));
        entry.insert("type".into(), Value::from(tpe));
        accessors.push(Value::Object(entry));
        (accessors.len() - 1) as u64
    };

    let mut max_index = 0u32;
    for mesh in &scene.meshes {
        for prim in &mesh.primitives {
            for index in &prim.indices {
                max_index = max_index.max(*index);
            }
        }
    }
    let use_u16_indices = max_index < 65536;

    let mut meshes_json: Vec<Value> = Vec::new();
    for (mesh_index, mesh) in scene.meshes.iter().enumerate() {
        let mut primitives_json: Vec<Value> = Vec::new();
        for prim in &mesh.primitives {
            let count = prim.vertices.len() as u64;
            let mut attributes = Map::new();

            let position_values: Vec<f32> = prim.vertices.iter().flat_map(|v| v.position).collect();
            let __offset_1 = bin.write_f32(&position_values);
            let view = push_view(&mut bin, &mut buffer_views, __offset_1, position_values.len() * 4, Some(34962));
            attributes.insert("POSITION".into(), Value::from(push_accessor(&mut accessors, view, 5126, count, "VEC3")));

            let normal_values: Vec<f32> = prim.vertices.iter().flat_map(|v| v.normal).collect();
            let __offset_2 = bin.write_f32(&normal_values);
            let view = push_view(&mut bin, &mut buffer_views, __offset_2, normal_values.len() * 4, Some(34962));
            attributes.insert("NORMAL".into(), Value::from(push_accessor(&mut accessors, view, 5126, count, "VEC3")));

            let uv_values: Vec<f32> = prim.vertices.iter().flat_map(|v| v.uv).collect();
            let __offset_3 = bin.write_f32(&uv_values);
            let view = push_view(&mut bin, &mut buffer_views, __offset_3, uv_values.len() * 4, Some(34962));
            attributes.insert("TEXCOORD_0".into(), Value::from(push_accessor(&mut accessors, view, 5126, count, "VEC2")));

            let color_values: Vec<f32> = prim.vertices.iter().flat_map(|v| v.color).collect();
            let __offset_4 = bin.write_f32(&color_values);
            let view = push_view(&mut bin, &mut buffer_views, __offset_4, color_values.len() * 4, Some(34962));
            attributes.insert("COLOR_0".into(), Value::from(push_accessor(&mut accessors, view, 5126, count, "VEC4")));

            let joints_values: Vec<u16> = prim.vertices.iter().flat_map(|v| v.joints).collect();
            let __offset_5 = bin.write_u16(&joints_values);
            let view = push_view(&mut bin, &mut buffer_views, __offset_5, joints_values.len() * 2, Some(34962));
            attributes.insert("JOINTS_0".into(), Value::from(push_accessor(&mut accessors, view, 5122, count, "VEC4")));

            let weights_values: Vec<f32> = prim.vertices.iter().flat_map(|v| v.weights).collect();
            let __offset_6 = bin.write_f32(&weights_values);
            let view = push_view(&mut bin, &mut buffer_views, __offset_6, weights_values.len() * 4, Some(34962));
            attributes.insert("WEIGHTS_0".into(), Value::from(push_accessor(&mut accessors, view, 5126, count, "VEC4")));

            let (indices_view, indices_component) = if use_u16_indices {
                let indices_u16: Vec<u16> = prim.indices.iter().map(|i| *i as u16).collect();
                let offset = bin.write_u16(&indices_u16);
                (
                    push_view(&mut bin, &mut buffer_views, offset, indices_u16.len() * 2, Some(34963)),
                    5122,
                )
            } else {
                let offset = bin.write_u32(&prim.indices);
                (
                    push_view(&mut bin, &mut buffer_views, offset, prim.indices.len() * 4, Some(34963)),
                    5123,
                )
            };
            let indices_accessor = push_accessor(&mut accessors, indices_view, indices_component, prim.indices.len() as u64, "SCALAR");

            let mut primitive_json = Map::new();
            primitive_json.insert("attributes".into(), Value::Object(attributes));
            primitive_json.insert("indices".into(), Value::from(indices_accessor));
            primitive_json.insert("mode".into(), Value::from(4));
            if prim.material_index >= 0 {
                primitive_json.insert("material".into(), Value::from(prim.material_index as u64));
            }
            if !prim.morph_targets.is_empty() {
                let targets: Vec<Value> = prim
                    .morph_targets
                    .iter()
                    .map(|target| {
                        let position_values: Vec<f32> = target.iter().flat_map(|d| d.position).collect();
                        let normal_values: Vec<f32> = target.iter().flat_map(|d| d.normal).collect();
                        let __offset_7 = bin.write_f32(&position_values);
                        let pos_view = push_view(&mut bin, &mut buffer_views, __offset_7, position_values.len() * 4, Some(34962));
                        let pos_accessor = push_accessor(&mut accessors, pos_view, 5126, target.len() as u64, "VEC3");
                        let __offset_8 = bin.write_f32(&normal_values);
                        let norm_view = push_view(&mut bin, &mut buffer_views, __offset_8, normal_values.len() * 4, Some(34962));
                        let norm_accessor = push_accessor(&mut accessors, norm_view, 5126, target.len() as u64, "VEC3");
                        let mut entry = Map::new();
                        entry.insert("POSITION".into(), Value::from(pos_accessor));
                        entry.insert("NORMAL".into(), Value::from(norm_accessor));
                        Value::Object(entry)
                    })
                    .collect();
                primitive_json.insert("targets".into(), Value::Array(targets));
            }
            primitives_json.push(Value::Object(primitive_json));
        }
        let mut mesh_json = Map::new();
        mesh_json.insert("name".into(), Value::from(mesh.name.clone().unwrap_or_else(|| format!("mesh_{mesh_index}"))));
        mesh_json.insert("primitives".into(), Value::Array(primitives_json));
        meshes_json.push(Value::Object(mesh_json));
    }

    let mut skins_json: Vec<Value> = Vec::new();
    for (skin_index, skin) in scene.skins.iter().enumerate() {
        let ibm_values: Vec<f32> = skin.inverse_bind_matrices.iter().flat_map(|m| m.iter().copied()).collect();
        let __offset_9 = bin.write_f32(&ibm_values);
        let view = push_view(&mut bin, &mut buffer_views, __offset_9, ibm_values.len() * 4, None);
        let mut entry = Map::new();
        entry.insert("name".into(), Value::from(skin.name.clone().unwrap_or_else(|| format!("skin_{skin_index}"))));
        entry.insert("joints".into(), Value::Array(skin.joints.iter().map(|v| Value::from(*v)).collect()));
        entry.insert("inverseBindMatrices".into(), Value::from(push_accessor(&mut accessors, view, 5126, skin.inverse_bind_matrices.len() as u64, "MAT4")));
        if let Some(skeleton) = skin.skeleton {
            entry.insert("skeleton".into(), Value::from(skeleton));
        }
        skins_json.push(Value::Object(entry));
    }

    let mut animations_json: Vec<Value> = Vec::new();
    for (anim_index, animation) in scene.animations.iter().enumerate() {
        let mut samplers: Vec<Value> = Vec::new();
        let mut channels: Vec<Value> = Vec::new();
        for (track_index, track) in animation.tracks.iter().enumerate() {
            let __offset_10 = bin.write_f32(&track.times);
            let times_view = push_view(&mut bin, &mut buffer_views, __offset_10, track.times.len() * 4, None);
            let times_accessor = push_accessor(&mut accessors, times_view, 5126, track.times.len() as u64, "SCALAR");
            let value_components: u64 = match track.path.as_str() {
                "rotation" => 4,
                "weights" => 1,
                _ => 3,
            };
            let value_tpe = match value_components {
                4 => "VEC4",
                1 => "SCALAR",
                _ => "VEC3",
            };
            let __offset_11 = bin.write_f32(&track.values);
            let values_view = push_view(&mut bin, &mut buffer_views, __offset_11, track.values.len() * 4, None);
            let values_accessor = push_accessor(&mut accessors, values_view, 5126, (track.values.len() as u64) / value_components, value_tpe);
            let mut sampler = Map::new();
            sampler.insert("input".into(), Value::from(times_accessor));
            sampler.insert("output".into(), Value::from(values_accessor));
            if track.step {
                sampler.insert("interpolation".into(), Value::from("STEP"));
            }
            samplers.push(Value::Object(sampler));
            let mut target = Map::new();
            target.insert("node".into(), Value::from(track.node));
            target.insert("path".into(), Value::from(track.path.clone()));
            let mut channel = Map::new();
            channel.insert("sampler".into(), Value::from(track_index as u64));
            channel.insert("target".into(), Value::Object(target));
            channels.push(Value::Object(channel));
        }
        let mut entry = Map::new();
        entry.insert("name".into(), Value::from(animation.name.clone().unwrap_or_else(|| format!("anim_{anim_index}"))));
        entry.insert("channels".into(), Value::Array(channels));
        entry.insert("samplers".into(), Value::Array(samplers));
        animations_json.push(Value::Object(entry));
    }

    let mut materials_json: Vec<Value> = Vec::new();
    for (index, material) in scene.materials.iter().enumerate() {
        let mut pbr = Map::new();
        pbr.insert(
            "baseColorFactor".into(),
            Value::Array(material.base_color_factor.iter().map(|v| Value::from(*v as f64)).collect()),
        );
        if let Some(m) = material.metallic_factor {
            pbr.insert("metallicFactor".into(), Value::from(m));
        }
        if let Some(r) = material.roughness_factor {
            pbr.insert("roughnessFactor".into(), Value::from(r));
        }
        let mut entry = Map::new();
        entry.insert("name".into(), Value::from(material.name.clone().unwrap_or_else(|| format!("material_{index}"))));
        entry.insert("pbrMetallicRoughness".into(), Value::Object(pbr));
        if let Some(e) = material.emissive_factor {
            entry.insert("emissiveFactor".into(), Value::Array(e.iter().map(|v| Value::from(*v as f64)).collect()));
        }
        if let Some(mode) = &material.alpha_mode {
            entry.insert("alphaMode".into(), Value::from(mode.clone()));
        }
        if let Some(ds) = material.double_sided {
            entry.insert("doubleSided".into(), Value::from(ds));
        }
        if vrm.is_some() {
            if let Some(mtoon) = &material.mtoon {
                let mut mtoon_json = Map::new();
                mtoon_json.insert("mainTex".into(), Value::Null);
                mtoon_json.insert(
                    "subEmission".into(),
                    Value::Array(mtoon.sub_emission.unwrap_or([1.0, 1.0, 1.0, 1.0]).iter().map(|v| Value::from(*v as f64)).collect()),
                );
                mtoon_json.insert("multiply".into(), Value::Array(vec![1.0, 1.0, 1.0, 1.0].iter().map(|v| Value::from(*v as f64)).collect()));
                mtoon_json.insert(
                    "shadowColor".into(),
                    Value::Array(mtoon.shadow_color.unwrap_or([0.718, 0.831, 1.0, 1.0]).iter().map(|v| Value::from(*v as f64)).collect()),
                );
                mtoon_json.insert("shadeShift".into(), Value::from(mtoon.shade_shift.unwrap_or(0.0) as f64));
                mtoon_json.insert("shadeToony".into(), Value::from(mtoon.shade_toony.unwrap_or(1.0) as f64));
                mtoon_json.insert("lightColor".into(), Value::Array(vec![1.0, 1.0, 1.0].iter().map(|v| Value::from(*v as f64)).collect()));
                mtoon_json.insert(
                    "rimColor".into(),
                    Value::Array(mtoon.rim_color.unwrap_or([1.0, 1.0, 1.0, 0.0]).iter().map(|v| Value::from(*v as f64)).collect()),
                );
                mtoon_json.insert("rimPower".into(), Value::from(mtoon.rim_power.unwrap_or(1.0) as f64));
                mtoon_json.insert("rimLight".into(), Value::from(mtoon.rim_light.unwrap_or(false)));
                mtoon_json.insert(
                    "rimLightColor".into(),
                    Value::Array(vec![1.0, 1.0, 1.0, 1.0].iter().map(|v| Value::from(*v as f64)).collect()),
                );
                mtoon_json.insert(
                    "lightDirection".into(),
                    Value::Array(vec![0.0, 0.0, 1.0].iter().map(|v| Value::from(*v as f64)).collect()),
                );
                mtoon_json.insert(
                    "specularColor".into(),
                    Value::Array(mtoon.specular_color.unwrap_or([1.0, 1.0, 1.0, 1.0]).iter().map(|v| Value::from(*v as f64)).collect()),
                );
                mtoon_json.insert("specularPower".into(), Value::from(mtoon.specular_power.unwrap_or(1.0) as f64));
                mtoon_json.insert("useSmooth".into(), Value::from(false));
                mtoon_json.insert("smoothColor".into(), Value::Array(vec![0.7, 0.7, 0.7, 0.5].iter().map(|v| Value::from(*v as f64)).collect()));
                mtoon_json.insert("useSphere".into(), Value::from(false));
                mtoon_json.insert("sphereMode".into(), Value::from("normal"));
                mtoon_json.insert("useMatCap".into(), Value::from(false));
                let mut extensions = Map::new();
                extensions.insert("VRMC_materials_mtoon".into(), Value::Object(mtoon_json));
                entry.insert("extensions".into(), Value::Object(extensions));
            }
        }
        materials_json.push(Value::Object(entry));
    }

    let mut nodes_json: Vec<Value> = Vec::new();
    for (index, node) in scene.nodes.iter().enumerate() {
        let mut entry = Map::new();
        entry.insert("name".into(), Value::from(node.name.clone().unwrap_or_else(|| format!("node_{index}"))));
        if let Some(t) = node.translation {
            entry.insert("translation".into(), Value::Array(t.iter().map(|v| Value::from(*v as f64)).collect()));
        }
        if let Some(r) = node.rotation {
            entry.insert("rotation".into(), Value::Array(r.iter().map(|v| Value::from(*v as f64)).collect()));
        }
        if let Some(s) = node.scale {
            entry.insert("scale".into(), Value::Array(s.iter().map(|v| Value::from(*v as f64)).collect()));
        }
        if let Some(mesh_index) = node.mesh_index {
            entry.insert("mesh".into(), Value::from(mesh_index));
        }
        if let Some(skin_index) = node.skin_index {
            entry.insert("skin".into(), Value::from(skin_index));
        }
        if let Some(weights) = &node.morph_weights {
            entry.insert("weights".into(), Value::Array(weights.iter().map(|v| Value::from(*v as f64)).collect()));
        }
        if !node.children.is_empty() {
            entry.insert("children".into(), Value::Array(node.children.iter().map(|v| Value::from(*v)).collect()));
        }
        nodes_json.push(Value::Object(entry));
    }

    let mut extensions_used: Vec<&str> = Vec::new();
    let mut extensions_required: Vec<&str> = Vec::new();
    let mut gltf_extensions = Map::new();
    if let Some(vrm) = vrm {
        extensions_used.extend(["VRMC_vrm", "VRMC_materials_mtoon"]);
        extensions_required.extend(["VRMC_vrm", "VRMC_materials_mtoon"]);
        let mut preset = Map::new();
        for preset_name in crate::vrm1::VRM1_EXPRESSION_PRESETS {
            let index = vrm
                .expression_presets
                .iter()
                .find(|(name, _)| name == preset_name)
                .map(|(_, i)| *i)
                .unwrap_or(0);
            let mut binding = Map::new();
            binding.insert("blendShape".into(), Value::from(index));
            binding.insert("isolated".into(), Value::from(false));
            preset.insert(preset_name.to_string(), Value::Object(binding));
        }
        let mut custom = Map::new();
        for (name, index) in &vrm.expression_custom {
            let mut binding = Map::new();
            binding.insert("blendShape".into(), Value::from(*index));
            binding.insert("isolated".into(), Value::from(false));
            custom.insert(name.clone(), Value::Object(binding));
        }
        let mut humanoid_bones = Map::new();
        for (bone, node) in &vrm.humanoid_bones {
            humanoid_bones.insert(bone.clone(), Value::from(node.map(Value::from).unwrap_or(Value::Null)));
        }
        let mut meta = Map::new();
        meta.insert("title".into(), Value::from(vrm.meta.title.clone()));
        meta.insert("author".into(), Value::from(vrm.meta.author.clone()));
        meta.insert("version".into(), Value::from(vrm.meta.version.clone()));
        meta.insert("year".into(), Value::from(vrm.meta.year));
        let mut license = Map::new();
        if let Some(name) = &vrm.meta.license_name {
            license.insert("name".into(), Value::from(name.clone()));
        }
        meta.insert("license".into(), Value::Object(license));
        meta.insert("contactInformation".into(), Value::from(vrm.meta.contact_information.clone()));
        meta.insert(
            "metadata".into(),
            Value::Array(
                vrm.meta
                    .metadata
                    .iter()
                    .map(|(t, v)| {
                        let mut entry = Map::new();
                        entry.insert("type".into(), Value::from(t.clone()));
                        entry.insert("value".into(), Value::from(v.clone()));
                        Value::Object(entry)
                    })
                    .collect(),
            ),
        );
        meta.insert("reference".into(), Value::from(vrm.meta.reference.clone()));

        let mut humanoid = Map::new();
        humanoid.insert("humanBones".into(), Value::Object(humanoid_bones));
        let mut expression = Map::new();
        expression.insert("preset".into(), Value::Object(preset));
        expression.insert("custom".into(), Value::Object(custom));
        let mut vrm_json = Map::new();
        vrm_json.insert("meta".into(), Value::Object(meta));
        vrm_json.insert("humanoid".into(), Value::Object(humanoid));
        vrm_json.insert("expression".into(), Value::Object(expression));
        gltf_extensions.insert("VRMC_vrm".into(), Value::Object(vrm_json));

        if let Some(spring) = &vrm.spring_bone {
            extensions_used.push("VRMC_springBone");
            extensions_required.push("VRMC_springBone");
            let spring_joint = |j: &crate::vrm1::Vrm1SpringJoint| -> Value {
                let mut entry = Map::new();
                entry.insert("node".into(), Value::from(j.node));
                entry.insert("distance".into(), Value::from(j.distance as f64));
                entry.insert("hitRadius".into(), Value::from(j.hit_radius as f64));
                entry.insert("gravityPower".into(), Value::from(j.gravity_power as f64));
                entry.insert("gravityDir".into(), Value::Array(j.gravity_dir.iter().map(|v| Value::from(*v as f64)).collect()));
                entry.insert("springStiffness".into(), Value::from(j.spring_stiffness as f64));
                entry.insert("springDamping".into(), Value::from(j.spring_damping as f64));
                Value::Object(entry)
            };
            let groups: Vec<Value> = spring
                .groups
                .iter()
                .map(|group| {
                    let mut entry = Map::new();
                    entry.insert("name".into(), Value::from(group.name.clone()));
                    entry.insert("center".into(), spring_joint(&group.center));
                    entry.insert(
                        "joints".into(),
                        Value::Array(group.joints.iter().map(spring_joint).collect()),
                    );
                    entry.insert(
                        "colliderGroups".into(),
                        Value::Array(group.collider_groups.iter().map(|v| Value::from(*v)).collect()),
                    );
                    Value::Object(entry)
                })
                .collect();
            let mut colliders: Vec<Value> = Vec::new();
            for sphere in &spring.colliders.spheres {
                let mut sphere_entry = Map::new();
                sphere_entry.insert("group".into(), Value::from(sphere.group));
                sphere_entry.insert("node".into(), Value::from(sphere.node));
                sphere_entry.insert("radius".into(), Value::from(sphere.radius as f64));
                sphere_entry.insert("offset".into(), Value::Array(sphere.offset.iter().map(|v| Value::from(*v as f64)).collect()));
                let mut entry = Map::new();
                entry.insert("spheres".into(), Value::Array(vec![Value::Object(sphere_entry)]));
                colliders.push(Value::Object(entry));
            }
            for capsule in &spring.colliders.capsules {
                let mut capsule_entry = Map::new();
                capsule_entry.insert("group".into(), Value::from(capsule.group));
                capsule_entry.insert("node".into(), Value::from(capsule.node));
                capsule_entry.insert("from".into(), Value::Array(capsule.from.iter().map(|v| Value::from(*v as f64)).collect()));
                capsule_entry.insert("to".into(), Value::Array(capsule.to.iter().map(|v| Value::from(*v as f64)).collect()));
                capsule_entry.insert("radius".into(), Value::from(capsule.radius as f64));
                let mut entry = Map::new();
                entry.insert("capsules".into(), Value::Array(vec![Value::Object(capsule_entry)]));
                colliders.push(Value::Object(entry));
            }
            let mut rig = Map::new();
            rig.insert("groups".into(), Value::Array(groups));
            rig.insert("colliders".into(), Value::Array(colliders));
            let mut spring_json = Map::new();
            spring_json.insert("secondaryRig".into(), Value::Object(rig));
            gltf_extensions.insert("VRMC_springBone".into(), Value::Object(spring_json));
        }
    }

    let bin_bytes = bin.bytes.clone();
    let mut json = Map::new();
    let mut asset = Map::new();
    asset.insert("version".into(), Value::from("2.0"));
    asset.insert("generator".into(), Value::from("anigo-core/vrm (fase2-exporter)"));
    if !extensions_used.is_empty() {
        asset.insert("extensionsUsed".into(), Value::Array(extensions_used.iter().map(|e| Value::from(*e)).collect()));
    }
    if !extensions_required.is_empty() {
        asset.insert("extensionsRequired".into(), Value::Array(extensions_required.iter().map(|e| Value::from(*e)).collect()));
    }
    json.insert("asset".into(), Value::Object(asset));
    json.insert("scene".into(), Value::from(0));
    let mut scene_json = Map::new();
    scene_json.insert("name".into(), Value::from(scene.name.clone().unwrap_or_else(|| "ANIGO Scene".to_string())));
    scene_json.insert("nodes".into(), Value::Array(scene.root_nodes.iter().map(|v| Value::from(*v)).collect()));
    json.insert("scenes".into(), Value::Array(vec![Value::Object(scene_json)]));
    json.insert("nodes".into(), Value::Array(nodes_json));
    json.insert("meshes".into(), Value::Array(meshes_json));
    if !skins_json.is_empty() {
        json.insert("skins".into(), Value::Array(skins_json));
    }
    if !materials_json.is_empty() {
        json.insert("materials".into(), Value::Array(materials_json));
    }
    if !animations_json.is_empty() {
        json.insert("animations".into(), Value::Array(animations_json));
    }
    let mut buffer_spec = Map::new();
    buffer_spec.insert("byteLength".into(), Value::from(bin_bytes.len() as u64));
    json.insert("buffers".into(), Value::Array(vec![Value::Object(buffer_spec)]));
    json.insert("bufferViews".into(), Value::Array(buffer_views));
    json.insert("accessors".into(), Value::Array(accessors));
    if !gltf_extensions.is_empty() {
        json.insert("extensions".into(), Value::Object(gltf_extensions));
    }
    (Value::Object(json), bin_bytes)
}

/// Bytes `.glb` (ou `.vrm` com `vrm`). Determinístico.
pub fn export_glb(scene: &ExportScene, vrm: Option<&VrmExportData>) -> Vec<u8> {
    let (json, bin) = export_gltf(scene, vrm);
    build_glb(&json, &bin)
}
