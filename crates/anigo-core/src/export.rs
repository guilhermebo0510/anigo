//! ANIGO — exportação canônica (P0 "Consolidar o renderer" + §8).
//!
//! Critérios do documento que este módulo fecha:
//!
//! * "viewport, exportação e headless usarem os mesmos contratos";
//! * "viewport e exportação passarem teste de paridade".
//!
//! A exportação **não** é um segundo caminho de deformação: ela monta o artefato
//! a partir da mesma malha base (`deformation::prepare_base_mesh`), do mesmo
//! conjunto canônico de morphs e dos mesmos pesos que o snapshot entrega ao
//! viewport. O que o módulo acrescenta é a **identidade verificável** desse
//! artefato: o `ExportManifest` carrega os checksums (FNV-1a-64 sobre os bytes
//! canônicos) da geometria base, da geometria deformada e da paleta de skinning,
//! de modo que o viewport possa comparar, campo a campo, o que ele está
//! desenhando com o que foi exportado.
//!
//! Regras que valem para o manifesto:
//!
//! * determinístico — nada de timestamp/host no documento (senão o artefato não
//!   seria reproduzível e o teste de determinismo não teria sentido);
//! * checksums de bytes, não de floats: `pack_vertices`/`pack_indices` são os
//!   mesmos bytes que o snapshot envia, então a comparação é exata entre Rust e
//!   TypeScript (os dois usam FNV-1a-64 sobre o mesmo buffer);
//! * o GLB sai de `Mesh::to_glb_bytes` — o mesmo escritor glTF 2.0 com skin.

use serde::{Deserialize, Serialize};

use crate::ids::{AssetId, CharacterId, ProjectId};
use crate::mesh::{BaseGender, GltfMeshError, Mesh};
use crate::morph::SparseMorphSet;
use crate::project::ProjectState;
use crate::snapshot::{
    catalog_fingerprint, mesh_topology_hash, pack_indices, pack_vertices, MorphWeight, SkinPayload,
    SNAPSHOT_FORMAT_VERSION, VERTEX_STRIDE_BYTES,
};

/// Versão do documento de exportação (o frontend valida este número).
pub const EXPORT_FORMAT_VERSION: u32 = 1;

/// Quem gerou o arquivo (rastro auditável no manifesto).
pub const EXPORT_GENERATOR: &str = "anigo-core/export";

/// FNV-1a-64 do buffer, em hex de 16 dígitos.
///
/// Mesmo algoritmo de `ids::fnv1a64`/`snapshot::mesh_topology_hash` e do
/// `fnv1a64` do TypeScript — é o que permite comparar o artefato exportado com o
/// que o viewport está desenhando sem trafegar a malha inteira.
pub fn buffer_checksum(bytes: &[u8]) -> String {
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    for byte in bytes {
        hash ^= *byte as u64;
        hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
    }
    format!("{hash:016x}")
}

/// Falhas possíveis da exportação (explícitas e recuperáveis — nada de silêncio).
#[derive(Debug, thiserror::Error)]
pub enum ExportError {
    #[error("malha vazia: nada a exportar ({vertices} vértices, {indices} índices)")]
    EmptyMesh { vertices: usize, indices: usize },
    #[error(
        "malha deformada com {deformed} vértices, mas a base tem {base} — exportar isto geraria um artefato inválido"
    )]
    VertexCountMismatch { deformed: usize, base: usize },
    #[error("falha ao serializar o GLB: {0}")]
    Glb(#[from] GltfMeshError),
}

/// Identidade da geometria base do artefato.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ExportGeometryInfo {
    pub vertex_count: u32,
    pub index_count: u32,
    pub triangle_count: u32,
    pub vertex_stride_bytes: u32,
    pub topology_hash: String,
    /// FNV-1a-64 dos bytes de `pack_vertices` da **base** (antes dos morphs).
    pub base_vertex_checksum: String,
    /// FNV-1a-64 dos bytes de `pack_indices`.
    pub base_index_checksum: String,
    pub catalog_fingerprint: String,
    pub mesh_uri: String,
    pub mesh_asset_id: String,
}

/// O que a deformação fez no artefato (autoria do núcleo).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ExportDeformationInfo {
    /// Autoridade da deformação (`core`).
    pub authority: String,
    /// Deltas empacotados no buffer canônico.
    pub morph_total_deltas: u32,
    /// Canais com peso não nulo.
    pub active_channels: u32,
    /// Pesos aplicados (esparsos: só os canais ativos).
    pub morph_weights: Vec<MorphWeight>,
    /// FNV-1a-64 dos 72 B por vértice da malha **deformada**.
    pub deformed_vertex_checksum: String,
    /// FNV-1a-64 das posições (12 B por vértice) da malha deformada.
    pub deformed_position_checksum: String,
}

/// Estado de skinning do artefato.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ExportSkinInfo {
    pub bone_count: u32,
    pub bind_pose: String,
    pub palette_is_identity: bool,
    pub palette_checksum: String,
    /// Vértices com soma de pesos ≈ 1 (deformáveis).
    pub skinned_vertices: u32,
    /// Vértices sem influência (a GPU devolve a identidade para eles).
    pub unskinned_vertices: u32,
}

/// Identidade do projeto exportado.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ExportProjectInfo {
    pub project_id: ProjectId,
    pub project_name: String,
    pub character_id: CharacterId,
    pub base_gender: BaseGender,
    pub static_revision: u64,
    pub dynamic_revision: u64,
    pub snapshot_version: u32,
}

/// Como o frame/arte-fim foi renderizado (preenchido pelo app shell, que é quem
/// conhece o renderer; o núcleo só monta o documento de geometria).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ExportRenderInfo {
    pub width: u32,
    pub height: u32,
    pub color_format: String,
    pub depth_format: String,
    pub msaa_samples: u32,
    pub render_passes: Vec<String>,
    pub clear_source: String,
    pub clear_color: [f32; 4],
    pub adapter_name: String,
    pub backend: String,
    pub render_time_ms: f64,
}

/// Documento que acompanha todo artefato exportado.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ExportManifest {
    pub format_version: u32,
    pub generator: String,
    pub project: ExportProjectInfo,
    pub geometry: ExportGeometryInfo,
    pub deformation: ExportDeformationInfo,
    pub skin: ExportSkinInfo,
    /// Presente só em exportações de frame (o shell preenche).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub render: Option<ExportRenderInfo>,
}

impl ExportManifest {
    /// Preenche a parte de render (usada pela exportação de frame/PNG).
    pub fn with_render(mut self, render: ExportRenderInfo) -> Self {
        self.render = Some(render);
        self
    }
}

/// Artefato pronto: bytes do GLB + manifesto.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ExportBundle {
    pub glb: Vec<u8>,
    pub manifest: ExportManifest,
}

impl ExportBundle {
    /// Manifesto serializado (JSON indentado, o mesmo formato do arquivo em disco).
    pub fn manifest_json(&self) -> Result<String, serde_json::Error> {
        serde_json::to_string_pretty(&self.manifest)
    }

    /// FNV-1a-64 dos bytes do GLB (o arquivo escrito precisa bater com isto).
    pub fn glb_checksum(&self) -> String {
        buffer_checksum(&self.glb)
    }
}

/// Entrada da exportação: exatamente o que o snapshot do viewport usa.
pub struct ExportRequest<'a> {
    pub project: &'a ProjectState,
    /// Malha base canônica (`prepare_base_mesh`) — a mesma do snapshot.
    pub base_mesh: &'a Mesh,
    /// Malha com os morphs já aplicados pelo núcleo (`apply_cpu`).
    pub deformed_mesh: &'a Mesh,
    pub morph_set: &'a SparseMorphSet,
    /// Pesos na ordem do catálogo (`deformation::catalog_weights`).
    pub weights: &'a [f32],
    pub static_revision: u64,
    pub dynamic_revision: u64,
    pub mesh_uri: &'a str,
    /// Skin **do snapshot**: o mesmo payload que o viewport recebeu, para que a
    /// paleta exportada e a desenhada não possam divergir.
    pub skin: &'a SkinPayload,
}

/// Monta o artefato completo (manifesto + GLB) a partir do estado canônico.
pub fn build_export_bundle(request: ExportRequest<'_>) -> Result<ExportBundle, ExportError> {
    let manifest = build_export_manifest(&request)?;
    // GLB: o **mesmo** escritor glTF do núcleo (com JOINTS_0/WEIGHTS_0).
    let glb = request.deformed_mesh.to_glb_bytes()?;
    Ok(ExportBundle { glb, manifest })
}

/// Monta só o manifesto — a exportação de frame descreve o render e não grava
/// GLB, mas precisa da mesma identidade de geometria/skin.
pub fn build_export_manifest(request: &ExportRequest<'_>) -> Result<ExportManifest, ExportError> {
    let base = request.base_mesh;
    let deformed = request.deformed_mesh;

    if base.vertices.is_empty() || base.indices.is_empty() {
        return Err(ExportError::EmptyMesh {
            vertices: base.vertices.len(),
            indices: base.indices.len(),
        });
    }
    if deformed.vertices.len() != base.vertices.len() {
        return Err(ExportError::VertexCountMismatch {
            deformed: deformed.vertices.len(),
            base: base.vertices.len(),
        });
    }

    // 1. Geometria base: checksums dos bytes canônicos (idênticos aos do snapshot).
    let base_vertex_bytes = pack_vertices(&base.vertices);
    let base_index_bytes = pack_indices(&base.indices);
    let geometry = ExportGeometryInfo {
        vertex_count: base.vertices.len() as u32,
        index_count: base.indices.len() as u32,
        triangle_count: (base.indices.len() / 3) as u32,
        vertex_stride_bytes: VERTEX_STRIDE_BYTES,
        topology_hash: mesh_topology_hash(base),
        base_vertex_checksum: buffer_checksum(&base_vertex_bytes),
        base_index_checksum: buffer_checksum(&base_index_bytes),
        catalog_fingerprint: catalog_fingerprint(),
        mesh_uri: request.mesh_uri.to_string(),
        mesh_asset_id: AssetId::for_uri(request.mesh_uri).to_string(),
    };

    // 2. Deformação: os mesmos canais/pesos do snapshot (esparso, |w| > 1e-6).
    let (_, _channels, deltas) =
        request
            .morph_set
            .pack_all_channels(request.weights, base.vertices.len() as u32);
    let mut morph_weights = Vec::new();
    let mut active_channels = 0u32;
    for (index, target) in request.morph_set.targets.iter().enumerate() {
        let weight = request.weights.get(index).copied().unwrap_or(0.0);
        if weight.abs() <= 1e-6 {
            continue;
        }
        active_channels += 1;
        // `value` é o valor autoral reconstruído (default do catálogo + peso),
        // exatamente como o snapshot dinâmico faz.
        let default_value = crate::morph_catalog::find_slider_def(&target.name)
            .map(|def| def.default_value)
            .unwrap_or(0.0);
        morph_weights.push(MorphWeight {
            target: crate::ids::MorphId::for_slider(&target.name),
            slider_id: target.name.clone(),
            value: default_value + weight,
            weight,
        });
    }

    let deformed_vertex_bytes = pack_vertices(&deformed.vertices);
    let mut positions = Vec::with_capacity(deformed.vertices.len() * 12);
    for vertex in &deformed.vertices {
        for component in vertex.position {
            positions.extend_from_slice(&component.to_le_bytes());
        }
    }
    let deformation = ExportDeformationInfo {
        authority: "core".to_string(),
        morph_total_deltas: deltas.len() as u32,
        active_channels,
        morph_weights,
        deformed_vertex_checksum: buffer_checksum(&deformed_vertex_bytes),
        deformed_position_checksum: buffer_checksum(&positions),
    };

    // 3. Skinning: a paleta e a cobertura real dos vértices.
    let mut skinned = 0u32;
    for vertex in &deformed.vertices {
        if (vertex.weights.iter().sum::<f32>() - 1.0).abs() <= 1e-5 {
            skinned += 1;
        }
    }
    let skin = ExportSkinInfo {
        bone_count: request.skin.bone_count,
        bind_pose: request.skin.bind_pose.clone(),
        palette_is_identity: request.skin.palette_is_identity
            && crate::skinning::flat_palette_is_identity(&request.skin.palette),
        palette_checksum: buffer_checksum(&floats_to_bytes(&request.skin.palette)),
        skinned_vertices: skinned,
        unskinned_vertices: deformed.vertices.len() as u32 - skinned,
    };

    Ok(ExportManifest {
        format_version: EXPORT_FORMAT_VERSION,
        generator: EXPORT_GENERATOR.to_string(),
        project: ExportProjectInfo {
            project_id: request.project.project_id.clone(),
            project_name: request.project.name.clone(),
            character_id: request.project.character.character_id.clone(),
            base_gender: request.project.character.base_gender,
            static_revision: request.static_revision,
            dynamic_revision: request.dynamic_revision,
            snapshot_version: SNAPSHOT_FORMAT_VERSION,
        },
        geometry,
        deformation,
        skin,
        render: None,
    })
}

/// Little-endian por componente (o mesmo layout que o WGSL/glTF usam).
fn floats_to_bytes(values: &[f32]) -> Vec<u8> {
    let mut bytes = Vec::with_capacity(values.len() * 4);
    for value in values {
        bytes.extend_from_slice(&value.to_le_bytes());
    }
    bytes
}

/// Reconstrói o manifesto a partir do JSON gravado (usado pelo app shell e pelos
/// testes de contrato).
pub fn manifest_from_json(raw: &str) -> Result<ExportManifest, serde_json::Error> {
    serde_json::from_str(raw)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::deformation::{catalog_weights, prepare_base_mesh, DeformationInputs};
    use crate::morph_catalog::build_canonical_sparse_morph_set;
    use crate::snapshot::{build_snapshot_for_project, decode_buffer, SkinPayload};

    struct Fixture {
        project: ProjectState,
        base_mesh: Mesh,
        deformed_mesh: Mesh,
        morph_set: SparseMorphSet,
        weights: Vec<f32>,
        skin: SkinPayload,
    }

    fn fixture() -> Fixture {
        let project = ProjectState::default();
        let base_mesh = prepare_base_mesh(&project).expect("base mesh");
        let morph_set = build_canonical_sparse_morph_set(&base_mesh);
        let weights = catalog_weights(&DeformationInputs::from_project(&project));
        let mut deformed_mesh = base_mesh.clone();
        let mut vertices = base_mesh.vertices.clone();
        morph_set.apply_cpu(&weights, &base_mesh.vertices, &mut vertices);
        deformed_mesh.vertices = vertices;
        Fixture {
            project,
            base_mesh,
            deformed_mesh,
            morph_set,
            weights,
            skin: SkinPayload::canonical_base(),
        }
    }

    fn bundle_of(fixture: &Fixture) -> ExportBundle {
        build_export_bundle(ExportRequest {
            project: &fixture.project,
            base_mesh: &fixture.base_mesh,
            deformed_mesh: &fixture.deformed_mesh,
            morph_set: &fixture.morph_set,
            weights: &fixture.weights,
            static_revision: 4,
            dynamic_revision: 9,
            mesh_uri: "anigo://canonical/base",
            skin: &fixture.skin,
        })
        .expect("export bundle")
    }

    fn fixture_json(relative: &str) -> serde_json::Value {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../contracts/fixtures")
            .join(relative);
        let raw = std::fs::read_to_string(&path)
            .unwrap_or_else(|e| panic!("fixture {} unreadable: {e}", path.display()));
        serde_json::from_str(&raw).expect("fixture must be valid JSON")
    }

    #[test]
    fn export_is_deterministic() {
        let fixture = fixture();
        let first = bundle_of(&fixture);
        let second = bundle_of(&fixture);
        assert_eq!(first.glb, second.glb, "o GLB precisa ser reproduzível byte a byte");
        assert_eq!(
            first.manifest_json().expect("manifest"),
            second.manifest_json().expect("manifest"),
            "o manifesto não pode carregar nada volátil (timestamp/host)"
        );
        assert_eq!(first.glb_checksum(), buffer_checksum(&second.glb));
        assert_eq!(first.glb_checksum().len(), 16);
    }

    #[test]
    fn manifest_geometry_is_the_snapshot_geometry() {
        // A paridade viewport ⇄ exportação começa aqui: os checksums do manifesto
        // saem dos **mesmos bytes** que o snapshot envia ao viewport.
        let fixture = fixture();
        let bundle = bundle_of(&fixture);
        let snapshot = build_snapshot_for_project(
            &fixture.project,
            &fixture.morph_set,
            9,
            4,
            true,
            "anigo://canonical/base",
        )
        .expect("snapshot");
        let payload = snapshot.static_payload.as_ref().expect("static payload");

        assert_eq!(bundle.manifest.geometry.vertex_count, payload.vertex_count);
        assert_eq!(bundle.manifest.geometry.index_count, payload.index_count);
        assert_eq!(
            bundle.manifest.geometry.vertex_stride_bytes,
            payload.vertex_stride_bytes
        );
        assert_eq!(bundle.manifest.geometry.topology_hash, payload.topology_hash);
        assert_eq!(
            bundle.manifest.geometry.catalog_fingerprint,
            payload.catalog_fingerprint
        );
        assert_eq!(bundle.manifest.geometry.mesh_uri, payload.mesh_uri);
        assert_eq!(bundle.manifest.geometry.mesh_asset_id, payload.mesh_asset_id.to_string());

        let vertex_bytes = decode_buffer(&payload.vertex_buffer_base64, VERTEX_STRIDE_BYTES)
            .expect("vertex buffer");
        let index_bytes = decode_buffer(&payload.index_buffer_base64, 4).expect("index buffer");
        assert_eq!(
            bundle.manifest.geometry.base_vertex_checksum,
            buffer_checksum(&vertex_bytes),
            "o checksum da base precisa ser o dos bytes do snapshot"
        );
        assert_eq!(
            bundle.manifest.geometry.base_index_checksum,
            buffer_checksum(&index_bytes)
        );

        // E os pesos do manifesto são os do snapshot (mesma regra de esparsidade).
        assert_eq!(
            bundle.manifest.deformation.morph_weights.len(),
            snapshot.dynamic.morph_weights.len()
        );
        for (exported, dynamic) in bundle
            .manifest
            .deformation
            .morph_weights
            .iter()
            .zip(snapshot.dynamic.morph_weights.iter())
        {
            assert_eq!(exported.slider_id, dynamic.slider_id);
            assert!((exported.weight - dynamic.weight).abs() < 1e-6);
        }
    }

    #[test]
    fn deformed_checksum_matches_the_core_apply_cpu() {
        let fixture = fixture();
        let bundle = bundle_of(&fixture);
        let mut vertices = fixture.base_mesh.vertices.clone();
        fixture
            .morph_set
            .apply_cpu(&fixture.weights, &fixture.base_mesh.vertices, &mut vertices);
        let mut replayed = fixture.base_mesh.clone();
        replayed.vertices = vertices;
        assert_eq!(
            bundle.manifest.deformation.deformed_vertex_checksum,
            buffer_checksum(&pack_vertices(&replayed.vertices))
        );
        assert_eq!(bundle.manifest.deformation.authority, "core");
    }

    #[test]
    fn glb_round_trips_with_skin_attributes() {
        let fixture = fixture();
        let bundle = bundle_of(&fixture);
        let parsed = Mesh::from_glb_bytes(&bundle.glb).expect("o GLB exportado precisa reabrir");
        assert_eq!(parsed.vertices.len(), fixture.deformed_mesh.vertices.len());
        assert_eq!(parsed.indices.len(), fixture.deformed_mesh.indices.len());
        for (exported, original) in parsed.vertices.iter().zip(fixture.deformed_mesh.vertices) {
            for axis in 0..3 {
                assert!(
                    (exported.position[axis] - original.position[axis]).abs() < 1e-6,
                    "posição divergiu no eixo {axis}"
                );
            }
            assert_eq!(exported.joints, original.joints, "JOINTS_0 precisa sobreviver");
            for slot in 0..4 {
                assert!((exported.weights[slot] - original.weights[slot]).abs() < 1e-6);
            }
        }
    }

    #[test]
    fn skin_block_reports_real_coverage() {
        let fixture = fixture();
        let bundle = bundle_of(&fixture);
        let skin = &bundle.manifest.skin;
        assert_eq!(skin.bone_count as usize, crate::bone_sync::CANONICAL_JOINT_COUNT);
        assert_eq!(skin.bind_pose, "canonical_rest");
        assert!(skin.palette_is_identity, "o núcleo entrega a paleta neutra hoje");
        assert_eq!(skin.palette_checksum.len(), 16);
        assert_eq!(skin.skinned_vertices, fixture.base_mesh.vertices.len() as u32);
        assert_eq!(skin.unskinned_vertices, 0, "nenhum vértice pode ficar sem osso");
    }

    #[test]
    fn empty_or_mismatched_meshes_are_refused_instead_of_written() {
        let fixture = fixture();
        let mut empty = fixture.base_mesh.clone();
        empty.vertices.clear();
        empty.indices.clear();
        let error = build_export_bundle(ExportRequest {
            project: &fixture.project,
            base_mesh: &empty,
            deformed_mesh: &fixture.deformed_mesh,
            morph_set: &fixture.morph_set,
            weights: &fixture.weights,
            static_revision: 0,
            dynamic_revision: 0,
            mesh_uri: "anigo://canonical/base",
            skin: &fixture.skin,
        })
        .expect_err("malha vazia precisa ser recusada");
        assert!(matches!(error, ExportError::EmptyMesh { .. }));

        let mut wrong = fixture.deformed_mesh.clone();
        wrong.vertices.pop();
        let error = build_export_bundle(ExportRequest {
            project: &fixture.project,
            base_mesh: &fixture.base_mesh,
            deformed_mesh: &wrong,
            morph_set: &fixture.morph_set,
            weights: &fixture.weights,
            static_revision: 0,
            dynamic_revision: 0,
            mesh_uri: "anigo://canonical/base",
            skin: &fixture.skin,
        })
        .expect_err("contagem divergente precisa ser recusada");
        assert!(matches!(error, ExportError::VertexCountMismatch { .. }));
    }

    #[test]
    fn fixture_manifest_agrees_with_the_snapshot_fixture() {
        // Os dois fixtures são gerados pelo mesmo script (TypeScript). Este teste
        // pina, do lado Rust, que os checksums do manifesto são os do snapshot —
        // é o que garante que a comparação de paridade no frontend tem sentido.
        let manifest_value = fixture_json("export_manifest_v1.json");
        let manifest: ExportManifest =
            serde_json::from_value(manifest_value).expect("manifesto do fixture precisa decodificar");
        assert_eq!(manifest.format_version, EXPORT_FORMAT_VERSION);
        assert_eq!(manifest.generator, EXPORT_GENERATOR);

        let snapshot = fixture_json("core_snapshot_v1.json");
        let payload = snapshot["static_payload"].clone();
        let vertex_bytes = decode_buffer(
            payload["vertex_buffer_base64"].as_str().expect("base64"),
            VERTEX_STRIDE_BYTES,
        )
        .expect("vertex buffer do fixture");
        let index_bytes =
            decode_buffer(payload["index_buffer_base64"].as_str().expect("base64"), 4)
                .expect("index buffer do fixture");

        assert_eq!(
            manifest.geometry.base_vertex_checksum,
            buffer_checksum(&vertex_bytes)
        );
        assert_eq!(
            manifest.geometry.base_index_checksum,
            buffer_checksum(&index_bytes)
        );
        assert_eq!(
            manifest.geometry.vertex_count,
            payload["vertex_count"].as_u64().unwrap() as u32
        );
        assert_eq!(
            manifest.geometry.topology_hash,
            payload["topology_hash"].as_str().unwrap()
        );
        assert_eq!(
            manifest.geometry.catalog_fingerprint,
            payload["catalog_fingerprint"].as_str().unwrap()
        );
        assert_eq!(manifest.skin.bone_count, payload["skin"]["bone_count"].as_u64().unwrap() as u32);
        assert!(manifest.skin.palette_is_identity);

        // O JSON do fixture precisa voltar pelo mesmo caminho de leitura do app.
        let raw = serde_json::to_string(&manifest).expect("re-serialize");
        assert_eq!(manifest_from_json(&raw).expect("from_json"), manifest);
    }

    #[test]
    fn checksum_is_the_fnv_1a_64_of_the_bytes() {
        // Valores congelados: o TypeScript usa o mesmo algoritmo (cross-language).
        assert_eq!(buffer_checksum(b""), "cbf29ce484222325");
        assert_eq!(buffer_checksum(b"anigo"), "badef0cd4c0ad193");
        assert_eq!(buffer_checksum(&[0x00, 0x01, 0x02, 0xff]), buffer_checksum(&[0x00, 0x01, 0x02, 0xff]));
    }
}
