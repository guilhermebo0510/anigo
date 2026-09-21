//! ANIGO — sessão canônica do núcleo (P0 §7, itens 4–5: "remover deformação de
//! produção do TypeScript" + "fazer o viewport consumir snapshots do núcleo").
//!
//! Este módulo é a única porta pela qual o app shell fala com a autoridade dos
//! dados. Ele possui:
//!
//! * `ProjectState` — o documento canônico (`crates/anigo-core`);
//! * `CommandHistory` — undo/redo com inversos armazenados (o UI nunca decide o
//!   que é um comando válido, nem o que é reversível);
//! * a geometria base preparada pelo núcleo (`deformation::prepare_base_mesh`) e
//!   o conjunto canônico de morphs esparsos, em cache por chave de geometria
//!   (gênero + proporções + dimorfismo): mexer num slider não reconstrói a malha;
//! * `CoreSnapshot` — o payload que o viewport (TypeScript) consome. É a única
//!   forma de o frontend obter geometria: o TS não deforma nada.
//!
//! O snapshot separa a parte **estática** (malha + canais de morph, com revisão
//! própria) da **dinâmica** (pesos, câmera, luzes, materiais). O cliente manda
//! `client_static_revision` e só recebe a geometria quando está desatualizado.

use anigo_core::command::{restore_from_log, Command, CommandHistory, CommandLog, CommandOutcome};
use anigo_core::export::{
    build_export_bundle, build_export_manifest, ExportBundle, ExportManifest, ExportRenderInfo,
    ExportRequest,
};
use anigo_core::deformation::{catalog_weights, prepare_base_mesh, DeformationInputs};
use anigo_core::ids::fnv1a64;
use anigo_core::mesh::Mesh;
use anigo_core::morph::SparseMorphSet;
use anigo_core::morph_catalog::build_canonical_sparse_morph_set;
use anigo_core::project::{ProjectState, URI_BASE_FEMALE, URI_BASE_MALE};
use anigo_core::scene::{Scene, SceneNode};
use anigo_core::snapshot::{build_snapshot, CoreSnapshot, StaticGeometryPayload};

/// Geometria canônica em cache (parte estática do snapshot).
///
/// `Clone` é intencional: a exportação clona a geometria em cache para montar o
/// artefato sem manter o `CoreSession` emprestado (ver `export_parts`).
#[derive(Clone)]
pub struct CachedGeometry {
    /// Chave de geometria (gênero + proporções + dimorfismo) que a gerou.
    pub key: u64,
    /// Malha base já preparada pelo núcleo (gênero + proporções + normais).
    pub base_mesh: Mesh,
    /// Conjunto canônico de morphs esparsos (157 sliders; somatotype entra como
    /// macro sliders do próprio catálogo).
    pub morph_set: SparseMorphSet,
    /// Revisão da parte estática (o cliente usa como cache key).
    pub static_revision: u64,
    /// URI canônica da malha base.
    pub mesh_uri: String,
}

/// Sessão canônica: projeto + histórico + geometria em cache.
pub struct CoreSession {
    project: ProjectState,
    history: CommandHistory,
    geometry: Option<CachedGeometry>,
    static_revision: u64,
    dynamic_revision: u64,
}

impl Default for CoreSession {
    fn default() -> Self {
        Self::new()
    }
}

/// Chave de cache da geometria base: gênero, proporções e dimorfismo.
///
/// Deliberadamente **não** inclui pesos de morph: os deltas do conjunto canônico
/// são calculados sobre a malha base e valem para qualquer peso (é isso que
/// permite mover um slider sem reconstruir geometria — exatamente o item 5).
fn geometry_key(project: &ProjectState) -> u64 {
    let gender = match project.character.base_gender {
        anigo_core::mesh::BaseGender::Male => "male",
        anigo_core::mesh::BaseGender::Female => "female",
    };
    let raw = format!(
        "{}|{}|{}|{:?}|{}",
        project.character.character_id.as_str(),
        gender,
        project.character.gender_dimorphism,
        project.character.proportions,
        project.project_id.as_str(),
    );
    fnv1a64(&raw)
}

impl CoreSession {
    /// Cria uma sessão com o projeto default do editor.
    pub fn new() -> Self {
        let project = ProjectState::default();
        let limit = project.settings.history_limit as usize;
        Self {
            project,
            history: CommandHistory::new(limit),
            geometry: None,
            // Revisão inicial não-nula: `0` é "sem geometria" para o cliente,
            // então o primeiro snapshot sempre traz a parte estática.
            static_revision: 1,
            dynamic_revision: 0,
        }
    }

    /// Projeto canônico (somente leitura).
    pub fn project(&self) -> &ProjectState {
        &self.project
    }

    /// Histórico de comandos (somente leitura).
    pub fn history(&self) -> &CommandHistory {
        &self.history
    }

    /// Revisão dinâmica atual (muda a cada comando aceito).
    pub fn dynamic_revision(&self) -> u64 {
        self.dynamic_revision
    }

    /// Revisão estática atual (muda quando a malha base é reconstruída).
    pub fn static_revision(&self) -> u64 {
        self.static_revision
    }

    /// Substitui o projeto pelo documento canônico persistido.
    ///
    /// O documento passa por `ProjectState::from_json` (migração + validação),
    /// que é o mesmo caminho do carregamento de arquivo — schema inválido vira
    /// erro explícito e recuperável, nunca estado parcial.
    pub fn load_document(&mut self, raw: &str) -> Result<(), String> {
        let project = ProjectState::from_json(raw).map_err(|error| error.to_string())?;
        project.validate().map_err(|error| error.to_string())?;
        let limit = project.settings.history_limit as usize;
        self.project = project;
        self.history = CommandHistory::new(limit);
        self.geometry = None;
        self.static_revision += 1;
        self.dynamic_revision = 0;
        Ok(())
    }

    /// Replay de um log de comandos persistido (autosave/restauração).
    ///
    /// O prefixo válido é preservado quando um comando do log é inválido — a
    /// recuperação parcial exigida por §3.1 — e o erro nomeia o índice.
    pub fn restore_log(&mut self, base_document: &str, log_json: &str) -> Result<usize, String> {
        self.load_document(base_document)?;
        let log = CommandLog::from_json(log_json).map_err(|error| error.to_string())?;
        let limit = self.project.settings.history_limit as usize;
        match restore_from_log(&self.project, &log, limit) {
            Ok(replayed) => {
                let applied = replayed.outcomes.len();
                self.project = replayed.state;
                self.history = replayed.history;
                self.dynamic_revision = applied as u64;
                self.invalidate_geometry();
                Ok(applied)
            }
            Err((index, error)) => {
                // Prefixo aplicado é mantido; o resto do log é descartado.
                if index > 0 {
                    let prefix = CommandLog {
                        version: log.version,
                        entries: log.entries[..index].to_vec(),
                    };
                    if let Ok(replayed) = restore_from_log(&self.project, &prefix, limit) {
                        self.project = replayed.state;
                        self.history = replayed.history;
                        self.dynamic_revision = index as u64;
                        self.invalidate_geometry();
                    }
                }
                Err(format!("comando {} do log é inválido: {}", index, error))
            }
        }
    }

    /// Documento canônico serializado (fonte da verdade para persistir).
    pub fn document(&self) -> Result<String, String> {
        self.project.to_canonical_json().map_err(|error| error.to_string())
    }

    /// Valida + aplica + empilha um comando. Estado intacto quando inválido.
    pub fn apply(&mut self, command: Command) -> Result<CommandOutcome, String> {
        let outcome = self
            .history
            .execute(&mut self.project, command)
            .map_err(|error| error.to_string())?;
        self.dynamic_revision += 1;
        if outcome.scope.requires_static_rebuild() {
            self.invalidate_geometry();
        }
        Ok(outcome)
    }

    /// Aplica um comando serializado (mesma representação do contrato TS).
    pub fn apply_json(&mut self, raw: &str) -> Result<CommandOutcome, String> {
        let command = Command::from_json(raw).map_err(|error| error.to_string())?;
        self.apply(command)
    }

    /// Desfaz o último comando aceito.
    pub fn undo(&mut self) -> Result<CommandOutcome, String> {
        let outcome = self.history.undo(&mut self.project).map_err(|error| error.to_string())?;
        self.dynamic_revision += 1;
        if outcome.scope.requires_static_rebuild() {
            self.invalidate_geometry();
        }
        Ok(outcome)
    }

    /// Refaz o último comando desfeito.
    pub fn redo(&mut self) -> Result<CommandOutcome, String> {
        let outcome = self.history.redo(&mut self.project).map_err(|error| error.to_string())?;
        self.dynamic_revision += 1;
        if outcome.scope.requires_static_rebuild() {
            self.invalidate_geometry();
        }
        Ok(outcome)
    }

    /// Descarta a geometria em cache (reconstruída no próximo snapshot, que já
    /// devolve a nova revisão estática ao cliente).
    fn invalidate_geometry(&mut self) {
        if self.geometry.is_some() {
            self.geometry = None;
            self.static_revision += 1;
        }
    }

    /// Snapshot canônico para o viewport.
    ///
    /// `client_static_revision == Some(atual)` faz o núcleo enviar só a parte
    /// dinâmica (pesos/câmera/luz/materiais) — é o caminho de um slider vivo.
    pub fn snapshot(
        &mut self,
        include_static: bool,
        client_static_revision: Option<u64>,
    ) -> Result<CoreSnapshot, String> {
        let geometry_key = geometry_key(&self.project);
        let stale = match &self.geometry {
            Some(cached) => cached.key != geometry_key,
            None => true,
        };
        if stale {
            let base_mesh =
                prepare_base_mesh(&self.project).map_err(|error| error.to_string())?;
            let morph_set = build_canonical_sparse_morph_set(&base_mesh);
            let mesh_uri = match self.project.character.base_gender {
                anigo_core::mesh::BaseGender::Male => URI_BASE_MALE,
                anigo_core::mesh::BaseGender::Female => URI_BASE_FEMALE,
            };
            tracing::info!(
                target: "anigo::core_session",
                vertices = base_mesh.vertices.len(),
                indices = base_mesh.indices.len(),
                targets = morph_set.targets.len(),
                mesh_uri = %mesh_uri,
                "canonical base geometry rebuilt by the core"
            );
            if self.geometry.is_some() {
                self.static_revision += 1;
            }
            self.geometry = Some(CachedGeometry {
                key: geometry_key,
                base_mesh,
                morph_set,
                static_revision: self.static_revision,
                mesh_uri: mesh_uri.to_string(),
            });
        }

        let geometry = self
            .geometry
            .as_ref()
            .ok_or_else(|| "canonical geometry unavailable".to_string())?;
        let fresh_client = client_static_revision == Some(geometry.static_revision);
        // Geometria vai sempre que o cliente está desatualizado — `include_static`
        // é só o pedido explícito de reenvio (recuperação de contexto GPU). O
        // caminho rápido (só dinâmico) é `include_static == false` + revisão atual.
        let wants_static = include_static || !fresh_client;
        Ok(build_snapshot(
            &self.project,
            &geometry.base_mesh,
            &geometry.morph_set,
            self.dynamic_revision,
            geometry.static_revision,
            wants_static,
            &geometry.mesh_uri,
        ))
    }

    /// Payload estático isolado (mesmo caminho do snapshot, sem o resto).
    pub fn static_payload(&mut self) -> Result<StaticGeometryPayload, String> {
        let snapshot = self.snapshot(true, None)?;
        snapshot
            .static_payload
            .ok_or_else(|| "static payload unavailable".to_string())
    }

    /// Pesos de canal de todos os sliders (ordem do catálogo), como o compute
    /// canônico usa. Serve para diagnóstico e para os testes de contrato.
    pub fn channel_weights(&self) -> Vec<f32> {
        catalog_weights(&DeformationInputs::from_project(&self.project))
    }

    /// Malha totalmente deformada pelo núcleo (base + morphs ativos).
    pub fn deformed_mesh(&mut self) -> Result<Mesh, String> {
        let _ = self.snapshot(true, None)?;
        let geometry = self
            .geometry
            .as_ref()
            .ok_or_else(|| "canonical geometry unavailable".to_string())?;
        let weights = self.channel_weights();
        let mut out = geometry.base_mesh.clone();
        let mut vertices = geometry.base_mesh.vertices.clone();
        geometry.morph_set.apply_cpu(&weights, &geometry.base_mesh.vertices, &mut vertices);
        out.vertices = vertices;
        Ok(out)
    }

    /// Espelha o estado canônico na `Scene` que o renderer headless desenha.
    ///
    /// O viewport do webview consome o snapshot; esta sincronização existe para
    /// que o frame PNG do núcleo (exportação/headless) use exatamente a mesma
    /// geometria e o mesmo material do projeto.
    pub fn sync_scene(&mut self, scene: &mut Scene) -> Result<(), String> {
        let mesh = self.deformed_mesh()?;
        let material = self.project.character_material().map(|entry| entry.material.clone());
        let light = self.project.scene.lights.first().map(|slot| slot.light.clone());
        let camera = self.project.scene.camera.camera.clone();
        let background = self.project.render.background_color;

        let mut node = SceneNode::new("primary_mesh", "Canonical Character").with_mesh(mesh);
        node.material = material;
        scene.nodes = vec![node];
        if let Some(light) = light {
            scene.light = light;
        }
        scene.camera = camera;
        scene.background_color = background;
        Ok(())
    }

    /// Artefato de exportação (P0 §7 / §8): GLB da malha **deformada pelo
    /// núcleo** + o manifesto que identifica o que foi exportado.
    ///
    /// O GLB e o manifesto saem do mesmo caminho que o snapshot: a malha base em
    /// cache (`prepare_base_mesh`), o conjunto canônico de morphs e os pesos
    /// atuais. A paleta de skinning vem do **payload estático do snapshot**, e
    /// não de uma cópia local, então o que o viewport recebeu e o que foi
    /// exportado não podem divergir.
    pub fn export_bundle(&mut self) -> Result<ExportBundle, String> {
        let (geometry, weights, skin, deformed) = self.export_parts()?;
        build_export_bundle(ExportRequest {
            project: &self.project,
            base_mesh: &geometry.base_mesh,
            deformed_mesh: &deformed,
            morph_set: &geometry.morph_set,
            weights: &weights,
            static_revision: geometry.static_revision,
            dynamic_revision: self.dynamic_revision,
            mesh_uri: &geometry.mesh_uri,
            skin: &skin,
        })
        .map_err(|error| error.to_string())
    }

    /// Peças da exportação: geometria em cache + pesos + skin do snapshot +
    /// malha já deformada pelo núcleo.
    ///
    /// A paleta vem do **payload estático do snapshot** (não de uma cópia
    /// local): o que o viewport recebeu e o que foi exportado não podem
    /// divergir. O `CachedGeometry` é clonado de propósito — devolver uma
    /// referência obrigaria a mantê-lo emprestado enquanto o manifesto é
    /// montado, e a malha base é reconstruída com pouca frequência.
    #[allow(clippy::type_complexity)]
    fn export_parts(
        &mut self,
    ) -> Result<
        (
            CachedGeometry,
            Vec<f32>,
            anigo_core::snapshot::SkinPayload,
            Mesh,
        ),
        String,
    > {
        let snapshot = self.snapshot(true, None)?;
        let skin = snapshot
            .static_payload
            .ok_or_else(|| "static payload unavailable".to_string())?
            .skin;
        let weights = self.channel_weights();
        let geometry = self
            .geometry
            .as_ref()
            .ok_or_else(|| "canonical geometry unavailable".to_string())?
            .clone();
        let mut deformed = geometry.base_mesh.clone();
        let mut vertices = geometry.base_mesh.vertices.clone();
        geometry
            .morph_set
            .apply_cpu(&weights, &geometry.base_mesh.vertices, &mut vertices);
        deformed.vertices = vertices;
        Ok((geometry, weights, skin, deformed))
    }

    /// Grava o GLB + o manifesto ao lado dele (`<path>.manifest.json`).
    ///
    /// Devolve o manifesto serializado e o checksum do arquivo gravado para que
    /// o frontend possa comparar com a geometria que está desenhando.
    pub fn export_glb_to_file(
        &mut self,
        path: &str,
    ) -> Result<(String, String, u64, String), String> {
        let bundle = self.export_bundle()?;
        let manifest_json = bundle
            .manifest_json()
            .map_err(|error| format!("manifesto não serializou: {error}"))?;
        let glb_path = std::path::PathBuf::from(path);
        if let Some(parent) = glb_path.parent() {
            if !parent.as_os_str().is_empty() {
                std::fs::create_dir_all(parent)
                    .map_err(|error| format!("Erro ao criar pasta de export: {error}"))?;
            }
        }
        std::fs::write(&glb_path, &bundle.glb)
            .map_err(|error| format!("Erro ao gravar o GLB: {error}"))?;

        let manifest_path = manifest_path_for(&glb_path);
        std::fs::write(&manifest_path, &manifest_json)
            .map_err(|error| format!("Erro ao gravar o manifesto: {error}"))?;

        Ok((
            glb_path.to_string_lossy().to_string(),
            manifest_path.to_string_lossy().to_string(),
            bundle.glb.len() as u64,
            bundle.glb_checksum(),
        ))
    }

    /// Manifesto da exportação de **frame** (geometria + bloco de render).
    ///
    /// O bloco de render é preenchido por quem renderiza (o app shell conhece o
    /// renderer canônico); aqui só a parte de geometria/skin é montada.
    pub fn export_frame_manifest(
        &mut self,
        render: ExportRenderInfo,
    ) -> Result<ExportManifest, String> {
        let (geometry, weights, skin, deformed) = self.export_parts()?;
        let request = ExportRequest {
            project: &self.project,
            base_mesh: &geometry.base_mesh,
            deformed_mesh: &deformed,
            morph_set: &geometry.morph_set,
            weights: &weights,
            static_revision: geometry.static_revision,
            dynamic_revision: self.dynamic_revision,
            mesh_uri: &geometry.mesh_uri,
            skin: &skin,
        };
        build_export_manifest(&request)
            .map(|manifest| manifest.with_render(render))
            .map_err(|error| error.to_string())
    }

    /// Resumo do histórico + revisões para a UI.
    ///
    /// A UI usa isso para *renderizar* o estado de undo/redo; as decisões
    /// continuam no núcleo (`can_undo`/`can_redo` são autorais).
    pub fn history_report(&self) -> serde_json::Value {
        serde_json::json!({
            "revision": self.history.revision(),
            "base_geometry_revision": self.history.base_geometry_revision(),
            "dynamic_revision": self.dynamic_revision,
            "static_revision": self.static_revision,
            "can_undo": self.history.can_undo(),
            "can_redo": self.history.can_redo(),
            "undo_depth": self.history.undo_depth(),
            "redo_depth": self.history.redo_depth(),
            "last_undo_description": self.history.last_undo_description(),
            "last_redo_description": self.history.last_redo_description(),
            "project_id": self.project.project_id.as_str(),
            "project_name": self.project.name.clone(),
            "project_fingerprint": self.project.content_fingerprint(),
            "project_schema_version": self.project.schema_version,
        })
    }
}

/// Caminho do manifesto que acompanha um artefato (`<arquivo>.manifest.json`).
pub fn manifest_path_for(path: &std::path::Path) -> std::path::PathBuf {
    let mut name = path
        .file_name()
        .map(|value| value.to_string_lossy().to_string())
        .unwrap_or_else(|| "export".to_string());
    name.push_str(".manifest.json");
    path.with_file_name(name)
}

#[cfg(test)]
mod tests {
    use super::*;
    use anigo_core::command::Command;
    use anigo_core::snapshot::SNAPSHOT_FORMAT_VERSION;

    fn session_with_slider(id: &str, value: f32) -> CoreSession {
        let mut session = CoreSession::new();
        let target = if id.starts_with("mrf_") {
            id.to_string()
        } else {
            format!("mrf_{id}")
        };
        let command: Command = serde_json::from_value(serde_json::json!({
            "kind": "set_morph_value",
            "target": target,
            "value": value,
        }))
        .expect("command deserializes");
        session.apply(command).expect("command applies");
        session
    }

    #[test]
    fn snapshot_exposes_the_canonical_geometry_and_authority() {
        let mut session = CoreSession::new();
        let snapshot = session.snapshot(true, None).expect("snapshot");
        assert_eq!(snapshot.snapshot_version, SNAPSHOT_FORMAT_VERSION);
        let payload = snapshot.static_payload.as_ref().expect("static payload");
        assert_eq!(payload.vertex_count, 4070);
        assert_eq!(
            payload.morph_channels.len(),
            anigo_core::morph_catalog::ALL_MORPH_SLIDERS.len()
        );
        assert_eq!(
            payload.morph_channels.len(),
            snapshot.dynamic.deformation_coverage.morph_targets as usize
        );
        assert_eq!(
            payload.catalog_fingerprint,
            anigo_core::snapshot::catalog_fingerprint()
        );
        // The core, not the client, decides who deforms.
        assert_eq!(snapshot.dynamic.deformation_authority, anigo_core::snapshot::DeformationAuthority::Core);
        assert!(snapshot.dynamic.deformation_coverage.is_complete());
    }

    #[test]
    fn static_payload_is_skipped_when_the_client_is_current() {
        let mut session = CoreSession::new();
        let first = session.snapshot(true, None).expect("snapshot");
        let revision = first.dynamic.static_revision;
        // Cliente atual + sem pedido explícito ⇒ só a parte dinâmica viaja.
        let fast = session.snapshot(false, Some(revision)).expect("snapshot");
        assert!(fast.static_payload.is_none());
        // Revisão velha ⇒ a geometria volta, mesmo sem pedido explícito (é o que
        // faz um undo de proporções chegar ao viewport).
        let stale = session.snapshot(false, Some(revision + 99)).expect("snapshot");
        assert!(stale.static_payload.is_some());
        // Pedido explícito (recuperação de contexto GPU) sempre reenvia.
        let forced = session.snapshot(true, Some(revision)).expect("snapshot");
        assert!(forced.static_payload.is_some());
    }

    #[test]
    fn morph_edits_do_not_rebuild_the_base_geometry() {
        let mut session = CoreSession::new();
        let before = session.snapshot(true, None).expect("snapshot");
        let revision = before.dynamic.static_revision;
        let command = Command::from_json(
            r#"{"kind":"set_morph_value","target":"mrf_height_overall","value":1.7}"#,
        )
        .expect("parses");
        session.apply(command).expect("applies");
        let after = session
            .snapshot(false, Some(revision))
            .expect("snapshot");
        // Same base geometry, new dynamic weights: no static payload, no new revision.
        assert!(after.static_payload.is_none());
        assert_eq!(after.dynamic.static_revision, revision);
        assert_ne!(after.dynamic.dynamic_revision, before.dynamic.dynamic_revision);
        assert!(!after.dynamic.morph_weights.is_empty());
    }

    #[test]
    fn proportions_change_the_base_geometry_revision() {
        let mut session = CoreSession::new();
        let before = session.snapshot(true, None).expect("snapshot");
        let revision = before.dynamic.static_revision;
        let command = Command::from_json(
            r#"{"kind":"set_proportions","head_scale":1.2,"shoulder_width":1.1}"#,
        )
        .expect("command parses");
        session.apply(command).expect("command applies");
        let after = session.snapshot(true, Some(revision)).expect("snapshot");
        assert!(after.static_payload.is_some());
        assert!(after.dynamic.static_revision > revision);
    }

    #[test]
    fn undo_restores_the_previous_document_and_reports_core_depths() {
        let mut session = CoreSession::new();
        let before = session.history_report();
        session
            .apply(
                Command::from_json(
                    r#"{"kind":"set_morph_value","target":"mrf_height_overall","value":1.5}"#,
                )
                .expect("parses"),
            )
            .expect("applies");
        assert!(session.history_report()["can_undo"].as_bool().unwrap_or(false));
        session.undo().expect("undo");
        let report = session.history_report();
        assert_eq!(report["can_undo"].as_bool(), Some(false));
        assert_eq!(report["project_fingerprint"], before["project_fingerprint"]);
        assert_eq!(session.project().character.morph_values.len(), 0);
    }

    #[test]
    fn invalid_documents_are_rejected_without_touching_the_session() {
        let mut session = CoreSession::new();
        let fingerprint = session.project().content_fingerprint();
        let error = session
            .load_document(r#"{"project_id": "prj_invalid", "schema_version": 999}"#)
            .expect_err("must fail");
        assert!(!error.is_empty());
        assert_eq!(session.project().content_fingerprint(), fingerprint);
    }

    #[test]
    fn deformed_mesh_applies_the_core_weights() {
        let mut session = CoreSession::new();
        let base = session.deformed_mesh().expect("mesh");
        let mut morphed = session_with_slider("height_overall", 2.0);
        let after = morphed.deformed_mesh().expect("mesh");
        assert_eq!(base.vertices.len(), after.vertices.len());
        let moved = base
            .vertices
            .iter()
            .zip(after.vertices.iter())
            .filter(|(a, b)| (a.position[1] - b.position[1]).abs() > 1e-5)
            .count();
        assert!(moved > 0, "slider must move the canonical mesh");
        // Weight of a slider is (value - catalog default) — the contract the
        // TypeScript viewport relies on for live channel updates.
        let weights = morphed.channel_weights();
        let index = anigo_core::morph_catalog::ALL_MORPH_SLIDERS
            .iter()
            .position(|def| def.id == "height_overall")
            .expect("slider exists");
        let expected = 2.0 - anigo_core::morph_catalog::ALL_MORPH_SLIDERS[index].default_value;
        assert!((weights[index] - expected).abs() < 1e-6);
    }

    #[test]
    fn export_bundle_matches_the_snapshot_geometry() {
        // A paridade viewport ⇄ exportação começa aqui: o manifesto carrega os
        // checksums dos **mesmos bytes** que o snapshot entrega ao viewport.
        let mut session = session_with_slider("height_overall", 1.75);
        let bundle = session.export_bundle().expect("export bundle");
        assert_eq!(bundle.manifest.geometry.vertex_count, 4070);
        assert_eq!(bundle.manifest.skin.bone_count, 24);
        assert!(bundle.manifest.deformation.active_channels >= 1);
        assert!(bundle.manifest.skin.unskinned_vertices == 0);
        assert_eq!(bundle.glb_checksum(), anigo_core::export::buffer_checksum(&bundle.glb));
        assert!(!bundle.glb.is_empty());

        let snapshot = session.snapshot(true, None).expect("snapshot");
        let payload = snapshot.static_payload.expect("static payload");
        assert_eq!(bundle.manifest.geometry.vertex_count, payload.vertex_count);
        assert_eq!(bundle.manifest.geometry.topology_hash, payload.topology_hash);
        let vertex_bytes =
            anigo_core::snapshot::decode_buffer(&payload.vertex_buffer_base64, 72).expect("vertices");
        assert_eq!(
            bundle.manifest.geometry.base_vertex_checksum,
            anigo_core::export::buffer_checksum(&vertex_bytes)
        );
        assert_eq!(
            bundle.manifest.deformation.morph_weights.len(),
            snapshot.dynamic.morph_weights.len()
        );
    }

    #[test]
    fn exported_glb_reopens_with_the_same_deformed_mesh() {
        let mut session = session_with_slider("head_width", 1.25);
        let deformed = session.deformed_mesh().expect("deformed mesh");
        let bundle = session.export_bundle().expect("export bundle");
        let reopened = Mesh::from_glb_bytes(&bundle.glb).expect("GLB reabre");
        assert_eq!(reopened.vertices.len(), deformed.vertices.len());
        assert_eq!(reopened.indices.len(), deformed.indices.len());
        for (exported, original) in reopened.vertices.iter().zip(deformed.vertices.iter()) {
            assert!((exported.position[0] - original.position[0]).abs() < 1e-6);
            assert_eq!(exported.joints, original.joints);
        }
    }

    #[test]
    fn export_writes_glb_and_manifest_next_to_each_other() {
        let mut session = CoreSession::new();
        let dir = std::env::temp_dir().join(format!(
            "anigo-export-test-{}",
            std::process::id()
        ));
        let path = dir.join("canonical.glb");
        let (glb_path, manifest_path, bytes, checksum) = session
            .export_glb_to_file(&path.to_string_lossy())
            .expect("export to file");

        assert!(std::path::Path::new(&glb_path).exists());
        assert!(std::path::Path::new(&manifest_path).exists());
        assert!(bytes > 0);
        assert_eq!(checksum.len(), 16);
        assert!(manifest_path.ends_with("canonical.glb.manifest.json"));

        let manifest_json = std::fs::read_to_string(&manifest_path).expect("manifest file");
        let manifest: anigo_core::export::ExportManifest =
            anigo_core::export::manifest_from_json(&manifest_json).expect("manifest parses");
        assert_eq!(manifest.format_version, 1);
        assert_eq!(manifest.project.project_id.as_str(), "prj_untitled");

        let written = std::fs::read(&glb_path).expect("glb file");
        assert_eq!(
            anigo_core::export::buffer_checksum(&written),
            checksum,
            "o checksum devolvido precisa ser o do arquivo gravado"
        );

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn frame_manifest_carries_the_render_block() {
        let mut session = CoreSession::new();
        let render = anigo_core::export::ExportRenderInfo {
            width: 1920,
            height: 1080,
            color_format: "Rgba8Unorm".to_string(),
            depth_format: "Depth24Plus".to_string(),
            msaa_samples: 4,
            render_passes: vec!["outline".to_string(), "cel".to_string()],
            clear_source: "scene.background_color".to_string(),
            clear_color: [0.08, 0.09, 0.13, 1.0],
            adapter_name: "test-adapter".to_string(),
            backend: "Vulkan".to_string(),
            render_time_ms: 1.5,
        };
        let manifest = session.export_frame_manifest(render.clone()).expect("manifest");
        assert_eq!(manifest.render.as_ref(), Some(&render));
        assert_eq!(manifest.geometry.vertex_count, 4070);
    }

    #[test]
    fn document_round_trips_through_the_session() {
        let session = session_with_slider("height_overall", 1.8);
        let json = session.document().expect("document");
        let mut restored = CoreSession::new();
        restored.load_document(&json).expect("load");
        assert_eq!(
            restored.project().content_fingerprint(),
            session.project().content_fingerprint()
        );
        let snapshot = restored.snapshot(true, None).expect("snapshot");
        assert!(snapshot.static_payload.is_some());
    }
}
